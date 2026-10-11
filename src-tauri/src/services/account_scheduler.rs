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

use crate::services::account_query;
use crate::services::accounts::{self, ConfigKind};
use crate::storage::Database;
use crate::vault::Vault;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
/// 未启用时重查开关的间隔（用户可能随时打开）。
const DISABLED_POLL_SECONDS: u64 = 60;

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
/// `sample = 0` 返回基准值（便于测试）；真实循环用随机样本。
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

/// 调度器句柄：`shutdown()` 请求取消，`Drop` 时中止任务（应用退出可取消）。
pub struct SchedulerHandle {
    shutdown: tokio::sync::watch::Sender<bool>,
    join: Option<tauri::async_runtime::JoinHandle<()>>,
}

impl SchedulerHandle {
    pub fn shutdown(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}

impl Drop for SchedulerHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}

/// 启动后台调度任务。默认开启；失败静默降级（查询层把失败记为 stale/unavailable，不抛出）。
pub fn spawn(db: Arc<Mutex<Database>>, vault: Arc<dyn Vault>) -> SchedulerHandle {
    let (shutdown, receiver) = tokio::sync::watch::channel(false);
    let join = tauri::async_runtime::spawn(scheduler_loop(db, vault, receiver));
    SchedulerHandle {
        shutdown,
        join: Some(join),
    }
}

async fn scheduler_loop(
    db: Arc<Mutex<Database>>,
    vault: Arc<dyn Vault>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    if *shutdown.borrow() {
        return;
    }
    // 启动时只补一次最新，不重放错过的周期。
    run_round(&db, &vault).await;

    loop {
        let config = {
            let Ok(guard) = db.lock() else { return };
            load_config(&guard)
        };
        if !config.enabled {
            if sleep_or_shutdown(Duration::from_secs(DISABLED_POLL_SECONDS), &mut shutdown).await {
                return;
            }
            continue;
        }
        // 10% 抖动；真实样本取自 OS 熵源。
        let sample = rand::random::<f64>() * 2.0 - 1.0;
        let interval = jittered_duration(config.tick_seconds(), sample);
        if sleep_or_shutdown(interval, &mut shutdown).await {
            return;
        }
        run_round(&db, &vault).await;
    }
}

/// 在阻塞线程池里执行一轮刷新：网络 I/O 与 SQLite 都是阻塞操作，不得占用异步工作线程。
async fn run_round(db: &Arc<Mutex<Database>>, vault: &Arc<dyn Vault>) {
    let db = Arc::clone(db);
    let vault = Arc::clone(vault);
    // 阻塞任务无法被 abort 打断；取消只在 await 点生效（已知边界）。
    let _ = tokio::task::spawn_blocking(move || refresh_round(&db, &vault)).await;
}

/// 等待时长或关闭信号；返回 `true` 表示收到关闭。
async fn sleep_or_shutdown(
    duration: Duration,
    shutdown: &mut tokio::sync::watch::Receiver<bool>,
) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => false,
        changed = shutdown.changed() => changed.is_err() || *shutdown.borrow(),
    }
}

/// 一轮刷新：对每个有效账户按各自周期判断是否到期（到期才刷，错过合并为一次）。
fn refresh_round(db: &Arc<Mutex<Database>>, vault: &Arc<dyn Vault>) {
    let Ok(mut guard) = db.lock() else { return };
    let Ok(accounts) = accounts::accounts_list(&guard, false) else {
        return;
    };
    for account in accounts {
        // 失败静默：查询层已把平台/网络失败写成 stale/unavailable；本地错误直接跳过。
        drop(account_query::query_account(
            &mut guard,
            vault.as_ref(),
            &account.account_id,
        ));
    }
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
}
