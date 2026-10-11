//! 账户查询调度器（T10 服务侧）：按周期在后台只读刷新账户余额与计划额度。
//!
//! 依据 `docs/specs/ACCOUNT_ADAPTERS.md` 第 5 节（调度与失败）：
//! - 拟定默认轮询：余额 15 分钟、计划额度 5 分钟；带 10% 抖动；
//! - 机器休眠后**合并补刷**，不瞬间补发所有错过周期；启动时只补一次最新；
//! - 系统凭据库锁定时只显示已保存快照（不弹授权），失败静默降级（stale）；
//! - 删账户 / 关窗口 / 应用退出能取消任务；默认开启。
//!
//! 边界：本模块只做只读查询调度；网关推理不采用这套重试策略。
//! 纯计算函数（配置、间隔、抖动、合并）单独可测，不在测试里起真实循环。

use super::background;
pub use super::background::TaskHandle as SchedulerHandle;
use crate::services::account_query;
use crate::services::accounts::{self, ConfigKind};
use crate::storage::Database;
use crate::vault::Vault;
use std::collections::{hash_map::DefaultHasher, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::Emitter;

/// settings 键：调度开关（"true"/"false"，默认开启）。
pub const SCHEDULER_ENABLED_KEY: &str = "accounts.scheduler.enabled";
/// settings 键：余额刷新周期（秒）。
pub const BALANCE_INTERVAL_KEY: &str = "accounts.scheduler.balance_interval_seconds";
/// settings 键：计划额度刷新周期（秒）。
pub const PLAN_INTERVAL_KEY: &str = "accounts.scheduler.plan_interval_seconds";
/// 默认余额周期：15 分钟。
pub const DEFAULT_BALANCE_INTERVAL_SECONDS: u64 = 15 * 60;
/// 默认计划额度周期：5 分钟。
pub const DEFAULT_PLAN_INTERVAL_SECONDS: u64 = 5 * 60;
/// 抖动比例：±10%。
pub const JITTER_RATIO: f64 = 0.1;

/// 调度配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerConfig {
    pub enabled: bool,
    pub balance_interval_seconds: u64,
    pub plan_interval_seconds: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            balance_interval_seconds: DEFAULT_BALANCE_INTERVAL_SECONDS,
            plan_interval_seconds: DEFAULT_PLAN_INTERVAL_SECONDS,
        }
    }
}

impl SchedulerConfig {
    /// 该账户配置类型对应的刷新周期。
    pub fn interval_for(&self, config_kind: ConfigKind) -> u64 {
        match config_kind {
            // 计划额度为主：用较短周期。
            ConfigKind::CodingPlan | ConfigKind::Both => self.plan_interval_seconds,
            // 纯 API 余额：用较长周期。
            ConfigKind::ApiPayg => self.balance_interval_seconds,
        }
    }

    /// 后台循环的最小节拍（两种周期的较小者）。
    pub fn tick_seconds(&self) -> u64 {
        self.plan_interval_seconds
            .min(self.balance_interval_seconds)
            .max(1)
    }
}

fn setting_bool(db: &Database, key: &str, default: bool) -> bool {
    match db
        .conn()
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .ok()
    {
        Some(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        None => default,
    }
}

fn setting_u64(db: &Database, key: &str, default: u64) -> u64 {
    db.conn()
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

/// 从 settings 读取调度配置；缺失或非法值回退默认。
pub fn load_config(db: &Database) -> SchedulerConfig {
    SchedulerConfig {
        enabled: setting_bool(db, SCHEDULER_ENABLED_KEY, true),
        balance_interval_seconds: setting_u64(
            db,
            BALANCE_INTERVAL_KEY,
            DEFAULT_BALANCE_INTERVAL_SECONDS,
        ),
        plan_interval_seconds: setting_u64(db, PLAN_INTERVAL_KEY, DEFAULT_PLAN_INTERVAL_SECONDS),
    }
}

/// 写回调度配置（供 UI 开关与周期设置使用）。
pub fn save_config(db: &Database, config: &SchedulerConfig) -> accounts::Result<()> {
    let set = |key: &str, value: String| -> accounts::Result<()> {
        db.conn().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    };
    set(
        SCHEDULER_ENABLED_KEY,
        if config.enabled { "true" } else { "false" }.to_owned(),
    )?;
    set(
        BALANCE_INTERVAL_KEY,
        config.balance_interval_seconds.max(1).to_string(),
    )?;
    set(
        PLAN_INTERVAL_KEY,
        config.plan_interval_seconds.max(1).to_string(),
    )?;
    Ok(())
}

/// 抖动后的周期（秒）：`sample ∈ [-1, 1]` 时区间为 `[0.9·base, 1.1·base]`，至少 1 秒。
///
/// `sample = 0` 返回基准值（便于测试）；真实循环按账户与本次尝试生成稳定样本。
pub fn jitter_seconds(base_seconds: u64, sample: f64) -> u64 {
    let sample = sample.clamp(-1.0, 1.0);
    let scaled = base_seconds as f64 * (1.0 + JITTER_RATIO * sample);
    (scaled.round() as i64).max(1) as u64
}

/// 抖动后的周期时长。
pub fn jittered_duration(base_seconds: u64, sample: f64) -> Duration {
    Duration::from_secs(jitter_seconds(base_seconds, sample))
}

/// 到期的批次数：**合并补刷**——错过多个周期也只补 1 次，不瞬间补发。
///
/// `elapsed < interval` → 0；否则 → 1。
pub fn batches_due(elapsed_seconds: u64, interval_seconds: u64) -> u32 {
    if interval_seconds == 0 {
        return 0;
    }
    u32::from(elapsed_seconds >= interval_seconds)
}

// ---------------------------------------------------------------------------
// 后台循环
// ---------------------------------------------------------------------------

pub fn spawn(
    db: Arc<Mutex<Database>>,
    vault: Arc<dyn Vault>,
    app: tauri::AppHandle,
) -> SchedulerHandle {
    SchedulerHandle::spawn(move |mut change| async move {
        let mut attempts = HashMap::new();
        loop {
            if *change.borrow() {
                return;
            }
            let worker_db = Arc::clone(&db);
            let worker_vault = Arc::clone(&vault);
            let stopped = change.clone();
            let app = app.clone();
            let result = tokio::task::spawn_blocking(move || {
                let next = refresh_round(
                    &worker_db,
                    worker_vault.as_ref(),
                    &mut attempts,
                    || *stopped.borrow(),
                    || {
                        let _ = app.emit("accounts-updated", ());
                    },
                );
                (attempts, next)
            })
            .await;
            let next = match result {
                Ok((updated, next)) => {
                    attempts = updated;
                    next
                }
                Err(_) => return,
            };
            if background::wait(next, &mut change).await {
                return;
            }
        }
    })
}

fn now_seconds() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

/// 每个账户的抖动固定到这次查询及周期，避免每轮重抽导致到期时间漂移。
fn deadline(account_id: &str, last_attempt: i64, interval: u64) -> i64 {
    let mut hash = DefaultHasher::new();
    (account_id, last_attempt, interval).hash(&mut hash);
    let sample = (hash.finish() % 2001) as f64 / 1000.0 - 1.0;
    last_attempt.saturating_add(jitter_seconds(interval, sample) as i64)
}

fn schedule(db: &Database, attempts: &HashMap<String, i64>, now: i64) -> Vec<(String, i64)> {
    let config = load_config(db);
    if !config.enabled {
        return vec![];
    }
    let Ok(accounts) = accounts::accounts_list(db, false) else {
        return vec![];
    };
    accounts.into_iter().map(|account| {
        // 成功和失败均算查询尝试；手动刷新也自然推迟下一次自动查询。
        let saved: Option<i64> = db.conn().query_row(
            "SELECT MAX(CAST(strftime('%s', queried_at) AS INTEGER)) FROM account_snapshots WHERE account_id = ?1",
            [&account.account_id], |row| row.get(0),
        ).ok().flatten();
        let last = saved.into_iter().chain(attempts.get(&account.account_id).copied()).max();
        let due = last.map(|last| deadline(&account.account_id, last, config.interval_for(account.config_kind))).unwrap_or(now);
        (account.account_id, due)
    }).collect()
}

fn refresh_round(
    db: &Arc<Mutex<Database>>,
    vault: &dyn Vault,
    attempts: &mut HashMap<String, i64>,
    stopped: impl Fn() -> bool,
    notify: impl Fn(),
) -> Option<Duration> {
    if stopped() {
        return None;
    }
    let now = now_seconds();
    // 启动首轮也先检查 enabled。无账户/关闭时返回 None，纯等待设置变化。
    let scheduled = {
        let guard = db.lock().ok()?;
        schedule(&guard, attempts, now)
    };
    let mut changed = false;
    for (account_id, due) in scheduled {
        if due > now {
            continue;
        }
        if stopped() {
            break;
        }
        {
            let guard = db.lock().ok()?;
            if !load_config(&guard).enabled {
                break;
            }
        }
        attempts.insert(account_id.clone(), now_seconds());
        // HTTP / 凭据读取在 DB 锁外；删除或换凭据后不提交旧查询结果。
        drop(account_query::query_account_shared(db, vault, &account_id));
        changed = true;
    }
    if changed {
        notify();
    }
    let now = now_seconds();
    let guard = db.lock().ok()?;
    schedule(&guard, attempts, now)
        .into_iter()
        .map(|(_, due)| Duration::from_secs(due.saturating_sub(now).max(1) as u64))
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Database;

    #[test]
    fn defaults_match_contract() {
        let config = SchedulerConfig::default();
        assert!(config.enabled, "默认开启");
        assert_eq!(config.balance_interval_seconds, 900);
        assert_eq!(config.plan_interval_seconds, 300);
        assert_eq!(config.interval_for(ConfigKind::ApiPayg), 900);
        assert_eq!(config.interval_for(ConfigKind::CodingPlan), 300);
        assert_eq!(config.interval_for(ConfigKind::Both), 300);
        assert_eq!(config.tick_seconds(), 300, "节拍取两种周期的较小者");
    }

    #[test]
    fn jitter_stays_within_ten_percent() {
        assert_eq!(jitter_seconds(900, 0.0), 900);
        assert_eq!(jitter_seconds(900, 1.0), 990);
        assert_eq!(jitter_seconds(900, -1.0), 810);
        // 越界样本被夹紧，仍在 ±10% 内。
        for sample in [-2.0, 2.0, 0.5, -0.5] {
            let value = jitter_seconds(900, sample);
            assert!((810..=990).contains(&value), "sample={sample} → {value}");
        }
        // 极小基准不低于 1 秒。
        assert!(jitter_seconds(1, -1.0) >= 1);
    }

    #[test]
    fn missed_periods_merge_into_one_batch() {
        assert_eq!(batches_due(0, 900), 0);
        assert_eq!(batches_due(899, 900), 0);
        assert_eq!(batches_due(900, 900), 1);
        // 休眠错过 4 个周期：只补一次，不补发 4 次。
        assert_eq!(batches_due(3600, 900), 1);
        assert_eq!(batches_due(86_400, 300), 1);
        assert_eq!(batches_due(100, 0), 0);
    }

    #[test]
    fn config_roundtrips_through_settings() {
        let db = Database::open_in_memory().unwrap();
        // 默认值
        assert_eq!(load_config(&db), SchedulerConfig::default());

        let custom = SchedulerConfig {
            enabled: false,
            balance_interval_seconds: 1200,
            plan_interval_seconds: 120,
        };
        save_config(&db, &custom).unwrap();
        assert_eq!(load_config(&db), custom);

        // 非法值回退默认
        db.conn()
            .execute(
                "UPDATE settings SET value = 'not-a-number' WHERE key = ?1",
                [BALANCE_INTERVAL_KEY],
            )
            .unwrap();
        assert_eq!(
            load_config(&db).balance_interval_seconds,
            DEFAULT_BALANCE_INTERVAL_SECONDS
        );
    }
    fn new_account(db: &Database, kind: ConfigKind) -> accounts::Account {
        accounts::account_create(
            db,
            &accounts::NewAccount {
                platform: "deepseek".into(),
                region: None,
                display_name: "test".into(),
                vendor_user_id: None,
                scope: None,
                config_kind: kind,
            },
        )
        .unwrap()
    }

    #[test]
    fn each_account_uses_its_own_deadline_and_latest_manual_or_failed_attempt() {
        let db = Database::open_in_memory().unwrap();
        let balance = new_account(&db, ConfigKind::ApiPayg);
        let plan = new_account(&db, ConfigKind::CodingPlan);
        let both = new_account(&db, ConfigKind::Both);
        for account in [&balance, &plan, &both] {
            db.conn().execute("INSERT INTO account_snapshots (account_id, queried_at, status) VALUES (?1, '2026-10-11T00:00:00Z', 'unavailable')", [&account.account_id]).unwrap();
        }
        let last = 1791676800;
        let due = schedule(&db, &HashMap::new(), last + 400);
        assert!(
            due.iter()
                .find(|(id, _)| id == &balance.account_id)
                .unwrap()
                .1
                > last + 400
        );
        assert!(due.iter().find(|(id, _)| id == &plan.account_id).unwrap().1 <= last + 400);
        assert!(due.iter().find(|(id, _)| id == &both.account_id).unwrap().1 <= last + 400);
        let attempts = HashMap::from([(plan.account_id.clone(), last + 400)]);
        assert!(
            schedule(&db, &attempts, last + 400)
                .iter()
                .find(|(id, _)| id == &plan.account_id)
                .unwrap()
                .1
                > last + 400
        );
        assert_eq!(
            deadline(&plan.account_id, last, 300),
            deadline(&plan.account_id, last, 300)
        );
    }

    #[test]
    fn disabled_first_round_does_not_query_or_notify_and_empty_waits() {
        let db = Arc::new(Mutex::new(Database::open_in_memory().unwrap()));
        let vault = crate::vault::MemoryVault::default();
        assert!(refresh_round(
            &db,
            &vault,
            &mut HashMap::new(),
            || false,
            || panic!("empty round queried")
        )
        .is_none());
        {
            let guard = db.lock().unwrap();
            new_account(&guard, ConfigKind::ApiPayg);
            save_config(
                &guard,
                &SchedulerConfig {
                    enabled: false,
                    ..Default::default()
                },
            )
            .unwrap();
        }
        assert!(refresh_round(
            &db,
            &vault,
            &mut HashMap::new(),
            || false,
            || panic!("disabled round queried")
        )
        .is_none());
        assert_eq!(
            db.lock()
                .unwrap()
                .conn()
                .query_row("SELECT COUNT(*) FROM account_snapshots", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
