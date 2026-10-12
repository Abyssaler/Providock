//! 费用版本与成本估算（T14）。
//!
//! 依据 `docs/specs/DATA_CONTRACTS.md` 第 6 节：
//! - 价格键含 provider/channel、model_version、currency、effective_from/to、区域、长上下文档位、
//!   以及 input / output / cache read / cache write / TTL 等分量差异；不可只凭模型名选一个全球价格。
//! - 未知价格**不计算**费用（未知 ≠ 免费）；源给出的合法 reported cost 不被估算覆盖。
//! - 历史价格更新**不悄悄覆盖**历史估值；重算保存新估值版本（事件上带 `price_version`）。
//!
//! 本模块只读写 `price_versions` 表，并显式重算某一事件（[`recompute_event_estimate`]）。
//! [`estimate_cost`] 是**只读**的：它只查当前有效价格，绝不改动任何事件的历史估算。

use crate::domain::{Completeness, CostInfo, Money, Quality, TokenSet};
use crate::services::accounts::now_rfc3339;
use crate::storage::{Database, StorageError};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum PricingError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("price version already exists (价格版本不可变，改动请新建版本): {0}")]
    PriceVersionExists(String),
    #[error("invalid decimal value for {field}: {value}")]
    InvalidDecimal { field: &'static str, value: String },
    #[error("cost arithmetic overflow: {0}")]
    Overflow(String),
    #[error("event not found: {0}")]
    EventNotFound(String),
}

pub type Result<T> = std::result::Result<T, PricingError>;

/// 某一档的费率集合。数值为十进制字符串，单位跟随所属 [`Rates::unit`]；
/// 分量为 `None` 表示该分量费率未知（未知 ≠ 0）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateSet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<String>,
}

/// 高峰时段定义：固定时区 + 是否仅工作日 + 每日若干分钟段。
/// `ranges` 元素形如 "09:00-12:00"（24 小时制，半开区间 [start, end)，不跨午夜）。
/// 法定节假日日历未实现：weekdays_only 把所有周一至周五都当工作日。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeakWindows {
    pub timezone: String,
    pub weekdays_only: bool,
    pub ranges: Vec<String>,
}

/// 长上下文整请求升档：事件 `input_total` 超过 `input_over_tokens` 时，
/// 整个请求（含输出与缓存分量）按该档费率计价，不与高峰档叠加。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LongContext {
    pub input_over_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<String>,
}

/// 手动价格规则的计费策略（序列化在 `rates_json` 的 `strategy` 键下）。
/// 全部字段可选；旧版本行没有该键，反序列化 `serde(default)` 得到 `None`。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateStrategy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak: Option<RateSet>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_windows: Option<PeakWindows>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_context: Option<LongContext>,
}

/// 各分量费率。数值为十进制字符串（每 1 个 token 的金额，单位见 [`Rates::unit`]）。
///
/// TTL 分量（`cache_write_ttl_5m` / `cache_write_ttl_1h`）是 `cache_write` 的细分，
/// 记录证据用；估算时只用 `cache_write` 基础费率，不再相加，避免重复计费。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rates {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_ttl_5m: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_ttl_1h: Option<String>,
    /// 计价单位说明（默认按单个 token 计价）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// 手动规则的计费策略（分时段 / 长上下文升档）。无策略的旧行此键不存在。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<RateStrategy>,
}

impl Rates {
    fn all(&self) -> [(&'static str, Option<&String>); 6] {
        [
            ("input", self.input.as_ref()),
            ("output", self.output.as_ref()),
            ("cache_read", self.cache_read.as_ref()),
            ("cache_write", self.cache_write.as_ref()),
            ("cache_write_ttl_5m", self.cache_write_ttl_5m.as_ref()),
            ("cache_write_ttl_1h", self.cache_write_ttl_1h.as_ref()),
        ]
    }

    fn has_any(&self) -> bool {
        self.all().iter().any(|(_, v)| v.is_some())
    }
}

/// 一条价格版本记录（对应 `price_versions` 表一行）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceVersion {
    pub price_version: String,
    pub provider: String,
    pub model_version: String,
    pub currency: String,
    /// 生效起点（含），RFC3339；`None` = 无下界。
    pub effective_from: Option<String>,
    /// 生效终点（不含），RFC3339；`None` = 无上界。
    pub effective_to: Option<String>,
    pub region: Option<String>,
    /// 长上下文档位等。
    pub tier: Option<String>,
    pub rates: Rates,
}

// ---------------------------------------------------------------------------
// 读写
// ---------------------------------------------------------------------------

const PRICE_COLUMNS: &str = "price_version, provider, model_version, currency, effective_from, \
                             effective_to, region, tier, rates_json";

fn row_to_price(row: &rusqlite::Row<'_>) -> Result<PriceVersion> {
    let rates_json: String = row.get(8)?;
    let rates: Rates = serde_json::from_str(&rates_json)?;
    Ok(PriceVersion {
        price_version: row.get(0)?,
        provider: row.get(1)?,
        model_version: row.get(2)?,
        currency: row.get(3)?,
        effective_from: row.get(4)?,
        effective_to: row.get(5)?,
        region: row.get(6)?,
        tier: row.get(7)?,
        rates,
    })
}

fn validate_rate(field: &'static str, value: &str) -> Result<()> {
    if parse_decimal(value).is_none() {
        return Err(PricingError::InvalidDecimal {
            field,
            value: value.to_string(),
        });
    }
    Ok(())
}

/// 写入一条价格版本。价格版本**不可变**（PK 已存在时报 [`PricingError::PriceVersionExists`]）：
/// 价格变动请新建版本，从而不悄悄改动任何历史估值采用的版本。
pub fn insert_price_version(db: &Database, price: &PriceVersion) -> Result<()> {
    for (field, value) in [
        ("input", price.rates.input.as_deref()),
        ("output", price.rates.output.as_deref()),
        ("cache_read", price.rates.cache_read.as_deref()),
        ("cache_write", price.rates.cache_write.as_deref()),
        (
            "cache_write_ttl_5m",
            price.rates.cache_write_ttl_5m.as_deref(),
        ),
        (
            "cache_write_ttl_1h",
            price.rates.cache_write_ttl_1h.as_deref(),
        ),
    ] {
        if let Some(value) = value {
            validate_rate(field, value)?;
        }
    }
    let rates_json = serde_json::to_string(&price.rates)?;
    let changed = db.conn().execute(
        "INSERT OR IGNORE INTO price_versions
            (price_version, provider, model_version, currency, effective_from, effective_to,
             region, tier, rates_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            price.price_version,
            price.provider,
            price.model_version,
            price.currency,
            price.effective_from,
            price.effective_to,
            price.region,
            price.tier,
            rates_json,
        ],
    )?;
    if changed == 0 {
        return Err(PricingError::PriceVersionExists(
            price.price_version.clone(),
        ));
    }
    Ok(())
}

pub fn price_version_get(
    db: &Database,
    price_version: &str,
    provider: &str,
    model_version: &str,
) -> Result<Option<PriceVersion>> {
    let sql = format!(
        "SELECT {PRICE_COLUMNS} FROM price_versions
         WHERE price_version = ?1 AND provider = ?2 AND model_version = ?3"
    );
    let mut stmt = db.conn().prepare(&sql)?;
    let mut rows = stmt.query(params![price_version, provider, model_version])?;
    match rows.next()? {
        Some(row) => Ok(Some(row_to_price(row)?)),
        None => Ok(None),
    }
}

pub fn price_versions_list(db: &Database) -> Result<Vec<PriceVersion>> {
    let sql = format!(
        "SELECT {PRICE_COLUMNS} FROM price_versions
         ORDER BY provider, model_version, effective_from, price_version"
    );
    let mut stmt = db.conn().prepare(&sql)?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(row_to_price(row)?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 估算
// ---------------------------------------------------------------------------

fn version_covers(version: &PriceVersion, at: &str) -> bool {
    let from_ok = version
        .effective_from
        .as_deref()
        .is_none_or(|from| from <= at);
    let to_ok = version.effective_to.as_deref().is_none_or(|to| at < to);
    from_ok && to_ok
}

/// 命中多版本时选**有效区间最新**的：`effective_from` 较大者优先，再按 `price_version` 决定
/// 确定性次序（不同币种同区间并存时选其一，绝不跨币种相加）。
fn select_effective(
    db: &Database,
    provider: &str,
    model: &str,
    at: &str,
) -> Result<Option<PriceVersion>> {
    let sql = format!(
        "SELECT {PRICE_COLUMNS} FROM price_versions
         WHERE provider = ?1 AND model_version = ?2"
    );
    let mut stmt = db.conn().prepare(&sql)?;
    let mut rows = stmt.query(params![provider, model])?;
    let mut best: Option<PriceVersion> = None;
    while let Some(row) = rows.next()? {
        let candidate = row_to_price(row)?;
        if !version_covers(&candidate, at) {
            continue;
        }
        let better = match &best {
            None => true,
            Some(current) => {
                let (c_from, c_id) = (
                    candidate.effective_from.clone().unwrap_or_default(),
                    candidate.price_version.clone(),
                );
                let (b_from, b_id) = (
                    current.effective_from.clone().unwrap_or_default(),
                    current.price_version.clone(),
                );
                (c_from, c_id) > (b_from, b_id)
            }
        };
        if better {
            best = Some(candidate);
        }
    }
    Ok(best)
}

/// 在“当前时间”估算：见 [`estimate_cost_at`]。
///
/// 未知价格返回 `None`（未知 ≠ 免费）。**只读**价格表，不改动事件上的历史估算。
pub fn estimate_cost(
    db: &Database,
    provider: &str,
    model: &str,
    tokens: &TokenSet,
) -> Result<Option<CostInfo>> {
    estimate_cost_at(db, provider, model, tokens, &now_rfc3339())
}

/// 在指定时刻 `at`（RFC3339）估算费用。抽出来便于测试用固定时间复现版本选择。
pub fn estimate_cost_at(
    db: &Database,
    provider: &str,
    model: &str,
    tokens: &TokenSet,
    at: &str,
) -> Result<Option<CostInfo>> {
    let Some(version) = select_effective(db, provider, model, at)? else {
        return Ok(None);
    };
    estimate_with_version(&version, tokens)
}

/// Pure calculation for a selected immutable price version.
pub fn estimate_with_version(
    version: &PriceVersion,
    tokens: &TokenSet,
) -> Result<Option<CostInfo>> {
    if !version.rates.has_any() {
        return Ok(None);
    }

    let mut total: Option<String> = None;
    apply(
        &mut total,
        tokens.input_fresh,
        version.rates.input.as_deref(),
        "input",
    )?;
    apply(
        &mut total,
        tokens.output_total,
        version.rates.output.as_deref(),
        "output",
    )?;
    apply(
        &mut total,
        tokens.input_cache_read,
        version.rates.cache_read.as_deref(),
        "cache_read",
    )?;
    apply(
        &mut total,
        tokens.input_cache_write,
        version.rates.cache_write.as_deref(),
        "cache_write",
    )?;

    let Some(amount) = total else {
        // 没有任何“token 与费率都已知”的分量——价格未知，不臆造 0。
        return Ok(None);
    };

    let complete = [
        (tokens.input_fresh, &version.rates.input),
        (tokens.input_cache_read, &version.rates.cache_read),
        (tokens.input_cache_write, &version.rates.cache_write),
        (tokens.output_total, &version.rates.output),
    ]
    .iter()
    .all(|(count, rate)| count.is_some_and(|n| n == 0 || rate.is_some()));
    Ok(Some(CostInfo {
        money: Money::new(amount, version.currency.clone()).map_err(|_| {
            PricingError::InvalidDecimal {
                field: "cost",
                value: version.currency.clone(),
            }
        })?,
        quality: Quality::Estimated,
        completeness: if complete {
            Completeness::Complete
        } else {
            Completeness::Partial
        },
        price_version: Some(version.price_version.clone()),
        scope: version
            .tier
            .clone()
            .or_else(|| version.region.clone())
            .or_else(|| Some("api_price_equivalent".to_string())),
    }))
}

fn apply(
    total: &mut Option<String>,
    count: Option<u64>,
    rate: Option<&str>,
    field: &'static str,
) -> Result<()> {
    let (Some(count), Some(rate)) = (count, rate) else {
        return Ok(());
    };
    let amount = mul_u64_decimal(count, rate, field)?;
    *total = Some(match total.take() {
        None => amount,
        Some(prev) => crate::domain::decimal::decimal_add(&prev, &amount)
            .ok_or_else(|| PricingError::Overflow("cost sum".to_string()))?,
    });
    Ok(())
}

/// 显式重算某事件的估算，写入**新的估值版本**。
///
/// 规则：价格变动只新增 `price_version`；只有调用本函数才改动事件上的
/// `cost_estimated_*`。未重算的事件保留其原 `price_version`（历史估值不被悄悄覆盖）。
/// 无有效价格返回 `None`，不改动事件。
pub fn recompute_event_estimate(
    db: &Database,
    event_id: &str,
    provider: &str,
    model: &str,
) -> Result<Option<CostInfo>> {
    let tokens = load_event_tokens(db, event_id)?
        .ok_or_else(|| PricingError::EventNotFound(event_id.to_string()))?;
    let Some(cost) = estimate_cost(db, provider, model, &tokens)? else {
        return Ok(None);
    };
    db.conn().execute(
        "UPDATE usage_events SET
            cost_estimated_amount = ?2,
            cost_estimated_currency = ?3,
            cost_estimated_price_version = ?4
         WHERE event_id = ?1",
        params![
            event_id,
            cost.money.amount,
            cost.money.currency,
            cost.price_version,
        ],
    )?;
    Ok(Some(cost))
}

fn load_event_tokens(db: &Database, event_id: &str) -> Result<Option<TokenSet>> {
    let row = db
        .conn()
        .query_row(
            "SELECT input_fresh, input_cache_read, input_cache_write, input_total,
                    output_total, output_reasoning, total, tokens_quality
             FROM usage_events WHERE event_id = ?1",
            params![event_id],
            |r| {
                Ok((
                    r.get::<_, Option<i64>>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                    r.get::<_, Option<i64>>(6)?,
                    r.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?;
    let Some((fresh, read, write, input_total, output, reasoning, total, quality_raw)) = row else {
        return Ok(None);
    };
    let u = |v: Option<i64>| v.map(|x| x as u64);
    Ok(Some(TokenSet {
        input_fresh: u(fresh),
        input_cache_read: u(read),
        input_cache_write: u(write),
        input_total: u(input_total),
        output_total: u(output),
        output_reasoning: u(reasoning),
        total: u(total),
        quality: Quality::parse(&quality_raw).unwrap_or(Quality::Unknown),
    }))
}

// ---------------------------------------------------------------------------
// 精确十进制乘法（不引入浮点，不新增 crate）
// ---------------------------------------------------------------------------

/// `(负号, 整数段, 小数段)`。非法输入返回 `None`。
fn parse_decimal(s: &str) -> Option<(bool, &str, &str)> {
    let (neg, t) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let mut parts = t.splitn(2, '.');
    let int_part = parts.next().unwrap_or("");
    let frac = parts.next().unwrap_or("");
    if int_part.is_empty() && frac.is_empty() {
        return None;
    }
    if !int_part.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((neg, if int_part.is_empty() { "0" } else { int_part }, frac))
}

fn scaled_to_decimal(v: i128, scale: usize) -> String {
    let neg = v < 0;
    let digits = v.unsigned_abs().to_string();
    let padded = if digits.len() <= scale {
        format!("{}{}", "0".repeat(scale + 1 - digits.len()), digits)
    } else {
        digits
    };
    let (int_part, frac) = padded.split_at(padded.len() - scale);
    let frac = frac.trim_end_matches('0');
    let int_part = int_part.trim_start_matches('0');
    let int_part = if int_part.is_empty() { "0" } else { int_part };
    let body = if frac.is_empty() {
        int_part.to_string()
    } else {
        format!("{int_part}.{frac}")
    };
    if neg && body != "0" {
        format!("-{body}")
    } else {
        body
    }
}

/// `count * rate`（精确十进制，无浮点）。非法费率或溢出返回错误。
fn mul_u64_decimal(count: u64, rate: &str, field: &'static str) -> Result<String> {
    let (neg, int_part, frac) =
        parse_decimal(rate).ok_or_else(|| PricingError::InvalidDecimal {
            field,
            value: rate.to_string(),
        })?;
    let scale = frac.len();
    let digits = format!("{int_part}{frac}");
    let value: i128 = digits.parse().map_err(|_| PricingError::InvalidDecimal {
        field,
        value: rate.to_string(),
    })?;
    let value = if neg { -value } else { value };
    let product = (count as i128)
        .checked_mul(value)
        .ok_or_else(|| PricingError::Overflow(format!("{count} * {rate}")))?;
    Ok(scaled_to_decimal(product, scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: &str = "2026-10-11T00:00:00Z";

    fn db_with_source() -> Database {
        let db = Database::open_in_memory().unwrap();
        db.conn()
            .execute(
                "INSERT INTO sources (source_id, agent, stream_domain, root_path, parser_version)
                 VALUES ('src1', 'kimi_code', 'local_log', '/anon', 'kimi-wire-1.5')",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO parse_runs (run_id, source_id, parser_version, status)
                 VALUES (1, 'src1', 'kimi-wire-1.5', 'committed')",
                [],
            )
            .unwrap();
        db
    }

    fn price(
        id: &str,
        from: Option<&str>,
        to: Option<&str>,
        currency: &str,
        input: &str,
        output: &str,
    ) -> PriceVersion {
        PriceVersion {
            price_version: id.into(),
            provider: "kimi".into(),
            model_version: "kimi-k2".into(),
            currency: currency.into(),
            effective_from: from.map(str::to_string),
            effective_to: to.map(str::to_string),
            region: None,
            tier: None,
            rates: Rates {
                input: Some(input.into()),
                output: Some(output.into()),
                cache_read: Some("0".into()),
                cache_write: Some("0".into()),
                ..Default::default()
            },
        }
    }

    fn tokens() -> TokenSet {
        TokenSet {
            input_fresh: Some(1000),
            input_cache_read: Some(500),
            input_cache_write: Some(0),
            input_total: Some(1500),
            output_total: Some(200),
            output_reasoning: None,
            total: Some(1700),
            quality: Quality::Reported,
        }
    }

    #[test]
    fn unknown_price_is_none_not_free() {
        let db = db_with_source();
        let cost = estimate_cost_at(&db, "kimi", "unknown-model", &tokens(), T0).unwrap();
        assert!(cost.is_none(), "未知价格必须返回 None");
    }

    #[test]
    fn explicit_zero_rate_is_zero_not_unknown() {
        let db = db_with_source();
        let mut p = price("v0", Some("2026-01-01T00:00:00Z"), None, "USD", "0", "0");
        p.rates.cache_read = Some("0".into());
        insert_price_version(&db, &p).unwrap();
        let cost = estimate_cost_at(&db, "kimi", "kimi-k2", &tokens(), T0)
            .unwrap()
            .expect("费率明确为 0 应得到 0，而不是 None");
        assert_eq!(cost.money.amount, "0");
        assert_eq!(cost.money.currency, "USD");
        assert_eq!(cost.quality, Quality::Estimated);
    }

    #[test]
    fn selects_latest_effective_interval() {
        let db = db_with_source();
        // 已过期
        insert_price_version(
            &db,
            &price(
                "v-old",
                Some("2026-01-01T00:00:00Z"),
                Some("2026-06-01T00:00:00Z"),
                "USD",
                "0.1",
                "0.2",
            ),
        )
        .unwrap();
        // 命中：较早
        insert_price_version(
            &db,
            &price("v-mid", Some("2026-06-01T00:00:00Z"), None, "USD", "1", "2"),
        )
        .unwrap();
        // 命中：最新
        insert_price_version(
            &db,
            &price(
                "v-new",
                Some("2026-09-01T00:00:00Z"),
                None,
                "USD",
                "10",
                "20",
            ),
        )
        .unwrap();
        let cost = estimate_cost_at(&db, "kimi", "kimi-k2", &tokens(), T0)
            .unwrap()
            .unwrap();
        assert_eq!(cost.price_version.as_deref(), Some("v-new"));
        // 1000*10 + 200*20 + 500*0 + 0*0 = 14000
        assert_eq!(cost.money.amount, "14000");
        assert_eq!(cost.completeness, Completeness::Complete);
    }

    #[test]
    fn multi_currency_is_not_merged() {
        let db = db_with_source();
        // 同区间两个币种：只选其一（确定性），绝不相加
        insert_price_version(
            &db,
            &price(
                "cny-v1",
                Some("2026-01-01T00:00:00Z"),
                None,
                "CNY",
                "7",
                "14",
            ),
        )
        .unwrap();
        insert_price_version(
            &db,
            &price(
                "usd-v1",
                Some("2026-01-01T00:00:00Z"),
                None,
                "USD",
                "1",
                "2",
            ),
        )
        .unwrap();
        let cost = estimate_cost_at(&db, "kimi", "kimi-k2", &tokens(), T0)
            .unwrap()
            .unwrap();
        // usd-v1 > cny-v1（与 effective_from 相同，按 price_version 决定次序）
        assert_eq!(cost.money.currency, "USD");
        assert_eq!(cost.money.amount, "1400"); // 1000*1 + 200*2
    }

    #[test]
    fn price_versions_are_immutable_and_history_is_not_silently_overwritten() {
        let db = db_with_source();
        insert_price_version(
            &db,
            &price("v1", Some("2026-01-01T00:00:00Z"), None, "USD", "1", "2"),
        )
        .unwrap();
        // 同版本 id 重复写入被拒绝（改动请新建版本）
        assert!(matches!(
            insert_price_version(
                &db,
                &price("v1", Some("2026-01-01T00:00:00Z"), None, "USD", "9", "9")
            ),
            Err(PricingError::PriceVersionExists(_))
        ));
        // 取回仍是原费率
        let got = price_version_get(&db, "v1", "kimi", "kimi-k2")
            .unwrap()
            .unwrap();
        assert_eq!(got.rates.input.as_deref(), Some("1"));

        // 事件写入 v1 估算
        db.conn().execute(
            "INSERT INTO usage_events (event_id, schema_version, parse_run_id, source_id, agent,
                stream_domain, observed_at, granularity, input_fresh, input_cache_read,
                input_cache_write, input_total, output_total, total, tokens_quality)
             VALUES ('e1', 1, 1, 'src1', 'kimi_code', 'local_log',
                '2026-10-11T06:00:00Z', 'request', 1000, 500, 0, 1500, 200, 1700, 'reported')",
            [],
        ).unwrap();
        recompute_event_estimate(&db, "e1", "kimi", "kimi-k2")
            .unwrap()
            .unwrap();

        // 价格上调：新增版本，不覆盖 v1；未重算的事件仍用 v1
        insert_price_version(
            &db,
            &price(
                "v2",
                Some("2026-10-01T00:00:00Z"),
                None,
                "USD",
                "100",
                "200",
            ),
        )
        .unwrap();
        let stored: String = db
            .conn()
            .query_row(
                "SELECT cost_estimated_price_version FROM usage_events WHERE event_id = 'e1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, "v1", "价格更新不得悄悄改动历史估值");

        // 显式重算 → 采用新版本 v2
        let recomputed = recompute_event_estimate(&db, "e1", "kimi", "kimi-k2")
            .unwrap()
            .unwrap();
        assert_eq!(recomputed.price_version.as_deref(), Some("v2"));
        let stored: String = db
            .conn()
            .query_row(
                "SELECT cost_estimated_price_version FROM usage_events WHERE event_id = 'e1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, "v2");
    }

    #[test]
    fn decimal_multiplication_is_exact() {
        assert_eq!(mul_u64_decimal(1000, "0.000003", "input").unwrap(), "0.003");
        assert_eq!(mul_u64_decimal(3, "0.1", "input").unwrap(), "0.3");
        assert_eq!(mul_u64_decimal(0, "10", "input").unwrap(), "0");
        assert_eq!(mul_u64_decimal(2, "-1.5", "input").unwrap(), "-3");
        assert!(mul_u64_decimal(1000, "1e5", "input").is_err());
    }

    #[test]
    fn rates_json_roundtrip() {
        let db = db_with_source();
        let mut p = price("v1", Some("2026-01-01T00:00:00Z"), None, "USD", "1", "2");
        p.rates.cache_write_ttl_5m = Some("1.25".into());
        p.rates.unit = Some("per_token".into());
        insert_price_version(&db, &p).unwrap();
        let got = price_version_get(&db, "v1", "kimi", "kimi-k2")
            .unwrap()
            .unwrap();
        assert_eq!(got, p);
        assert_eq!(price_versions_list(&db).unwrap().len(), 1);
    }

    #[test]
    fn strategy_json_is_version_tolerant_both_directions() {
        // 新行（含 strategy）：旧结构（无该字段）读取时忽略未知键，不报错
        let new_json = r#"{"input":"2","unit":"per_million_tokens","strategy":{"peak":{"input":"4"},"peakWindows":{"timezone":"Asia/Shanghai","weekdaysOnly":true,"ranges":["09:00-12:00"]},"longContext":{"inputOverTokens":128000,"input":"5"}}}"#;
        let rates: Rates = serde_json::from_str(new_json).unwrap();
        let strategy = rates.strategy.unwrap();
        assert_eq!(strategy.peak.unwrap().input.as_deref(), Some("4"));
        assert_eq!(
            strategy.peak_windows.unwrap().ranges,
            vec!["09:00-12:00".to_string()]
        );
        assert_eq!(strategy.long_context.unwrap().input_over_tokens, 128000);
        #[derive(Deserialize)]
        struct OldRates {
            #[allow(dead_code)]
            input: Option<String>,
        }
        let old: OldRates = serde_json::from_str(new_json).unwrap();
        assert_eq!(old.input.as_deref(), Some("2"));
        // 旧行（无 strategy 键）：新结构 serde(default) 得到 None
        let flat: Rates =
            serde_json::from_str(r#"{"input":"2","unit":"per_million_tokens"}"#).unwrap();
        assert!(flat.strategy.is_none());
        assert!(!serde_json::to_string(&flat).unwrap().contains("strategy"));
    }
}
