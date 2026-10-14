//! User-entered prices, scoped to an Agent and optional source. Append-only versions;
//! current-price comparisons never rewrite source usage or saved historical estimates.
//!
//! Optional strategies on a rule (stored inside `rates_json.strategy`, see
//! DATA_CONTRACTS.md「费用」与「2026-10-12 模型定价与 IPC 增量」):
//! - `peak` + `peakWindows`: 分时段价格（如 DeepSeek）。Asia/Shanghai 固定 UTC+8，
//!   工作日 + 本地时间落入任一半开时段 [start, end) 时按高峰费率，否则按基础（低谷）费率。
//!   法定节假日日历未实现，周一至周五一律视为工作日。
//! - `longContext`: 长上下文整请求升档。事件 input_total 超过阈值时整个请求按档位费率。
//!
//! 两者同配时 longContext 优先；长上下文档位没有高峰变体，peak 只在未配置 longContext 时参与。
//! 判定所需的 occurred_at / input_total 未知时估算为未知（Ok(None)），绝不按 0 或基础费率臆造。
use crate::{
    domain::{Completeness, CostInfo, Quality, TokenSet},
    services::{
        accounts::now_rfc3339,
        pricing::{self, LongContext, PeakWindows, PriceVersion, RateSet, RateStrategy, Rates},
    },
    storage::Database,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use time::{format_description::well_known::Rfc3339, OffsetDateTime, UtcOffset, Weekday};

const KIND: &str = "providock_manual_per_million_v1";
const DISABLED: &str = "providock_manual_disabled_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPrice {
    #[serde(default)]
    pub version: String,
    pub agent: String,
    pub source_id: Option<String>,
    pub model: String,
    pub currency: String,
    pub input: Option<String>,
    pub output: Option<String>,
    pub cache_read: Option<String>,
    pub cache_write: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak: Option<RateSet>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_windows: Option<PeakWindows>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_context: Option<LongContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tiers: Option<Vec<pricing::ContextTier>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<pricing::PriceSource>,
}
fn yes() -> bool {
    true
}
type Key = (String, Option<String>, String);

pub fn list(db: &Database) -> Result<Vec<ModelPrice>, String> {
    Ok(list_with_disabled(db)?
        .into_iter()
        .filter(|p| p.enabled)
        .collect())
}

pub(crate) fn list_with_disabled(db: &Database) -> Result<Vec<ModelPrice>, String> {
    let mut latest: BTreeMap<Key, ModelPrice> = BTreeMap::new();
    // Existing service orders by effective_from then version within each price key.
    for p in pricing::price_versions_list(db).map_err(|e| e.to_string())? {
        if !matches!(p.tier.as_deref(), Some(KIND | DISABLED)) {
            continue;
        }
        let Some(agent) = p.provider.strip_prefix("manual:") else {
            continue;
        };
        let strategy = p.rates.strategy;
        let source = p.rates.source;
        let rule = ModelPrice {
            version: p.price_version,
            source,
            agent: agent.into(),
            source_id: p.region,
            model: p.model_version,
            currency: p.currency,
            input: p.rates.input,
            output: p.rates.output,
            cache_read: p.rates.cache_read,
            cache_write: p.rates.cache_write,
            enabled: p.tier.as_deref() == Some(KIND),
            peak: strategy.as_ref().and_then(|s| s.peak.clone()),
            peak_windows: strategy.as_ref().and_then(|s| s.peak_windows.clone()),
            long_context: strategy.as_ref().and_then(|s| s.long_context.clone()),
            context_tiers: strategy.and_then(|s| s.context_tiers),
        };
        latest.insert(
            (
                rule.agent.clone(),
                rule.source_id.clone(),
                rule.model.clone(),
            ),
            rule,
        );
    }
    Ok(latest.into_values().collect())
}

pub fn save(db: &Database, rule: ModelPrice) -> Result<ModelPrice, String> {
    persist(db, rule, None)
}
pub(crate) fn save_imported(
    db: &Database,
    rule: ModelPrice,
    source: pricing::PriceSource,
) -> Result<ModelPrice, String> {
    persist(db, rule, Some(source))
}
fn persist(
    db: &Database,
    mut rule: ModelPrice,
    source: Option<pricing::PriceSource>,
) -> Result<ModelPrice, String> {
    if source.is_none() && rule.source.is_some() {
        let prior = list_with_disabled(db)?.into_iter().find(|p| {
            p.agent == rule.agent
                && p.model == rule.model
                && p.source_id == rule.source_id
                && p.source == rule.source
        });
        if prior.is_some_and(|p| {
            p.input != rule.input
                || p.output != rule.output
                || p.cache_read != rule.cache_read
                || p.cache_write != rule.cache_write
                || p.currency != rule.currency
                || p.peak != rule.peak
                || p.peak_windows != rule.peak_windows
                || p.long_context != rule.long_context
                || p.context_tiers != rule.context_tiers
        }) {
            rule.source = None;
        }
    }
    rule.source = source.or(rule.source);
    if let Some(source) = &rule.source {
        for raw in [&source.source_url, &source.official_url] {
            let url = reqwest::Url::parse(raw).map_err(|_| "价格来源链接无效")?;
            if raw.len() > 2048
                || url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err("价格来源须为公开 HTTPS 链接".into());
            }
        }
        OffsetDateTime::parse(&source.retrieved_at, &Rfc3339).map_err(|_| "价格抓取时间无效")?;
    }

    if ![
        "claude_code",
        "codex",
        "gemini_cli",
        "grok_build",
        "opencode",
        "pi",
        "mcode",
        "kimi_code",
        "deepseek_harness",
    ]
    .contains(&rule.agent.as_str())
    {
        return Err("请选择已支持的 Agent".into());
    }
    rule.model = rule.model.trim().into();
    if rule.model.is_empty() || rule.model.len() > 200 {
        return Err("请输入不超过 200 字节的准确模型名".into());
    }
    if !["USD", "CNY"].contains(&rule.currency.as_str()) {
        return Err("请选择 USD 或 CNY；不同币种分别统计".into());
    }
    if let Some(source) = &rule.source_id {
        let matches: bool = db
            .conn()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sources WHERE source_id=?1 AND agent=?2)",
                rusqlite::params![source, rule.agent],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !matches {
            return Err("数据源与 Agent 不匹配".into());
        }
    }
    for rate in [
        &rule.input,
        &rule.output,
        &rule.cache_read,
        &rule.cache_write,
    ]
    .into_iter()
    .flatten()
    {
        per_token(rate)?;
    }
    if rule.enabled
        && [
            &rule.input,
            &rule.output,
            &rule.cache_read,
            &rule.cache_write,
        ]
        .iter()
        .all(|r| r.is_none())
    {
        return Err("至少填写一项价格；留空表示未知，0 表示明确免费".into());
    }
    validate_strategy(&rule)?;
    let strategy = (rule.peak.is_some()
        || rule.peak_windows.is_some()
        || rule.long_context.is_some()
        || rule.context_tiers.is_some())
    .then(|| RateStrategy {
        peak: rule.peak.clone(),
        peak_windows: rule.peak_windows.clone(),
        long_context: rule.long_context.clone(),
        context_tiers: rule.context_tiers.clone(),
    });
    rule.version = format!(
        "{}-{}-{:08x}",
        if rule.source.is_some() {
            "aihot"
        } else {
            "manual"
        },
        time::OffsetDateTime::now_utc().unix_timestamp_nanos(),
        rand::random::<u32>()
    );
    pricing::insert_price_version(
        db,
        &PriceVersion {
            price_version: rule.version.clone(),
            provider: format!("manual:{}", rule.agent),
            model_version: rule.model.clone(),
            currency: rule.currency.clone(),
            effective_from: Some(now_rfc3339()),
            effective_to: None,
            region: rule.source_id.clone(),
            tier: Some(if rule.enabled { KIND } else { DISABLED }.into()),
            rates: Rates {
                input: rule.input.clone(),
                output: rule.output.clone(),
                cache_read: rule.cache_read.clone(),
                cache_write: rule.cache_write.clone(),
                unit: Some("per_million_tokens".into()),
                strategy,
                source: rule.source.clone(),
                ..Default::default()
            },
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(rule)
}

/// 策略校验：分时段与长上下文档的费率与基础费率同一十进制规则；
/// peak 与 peakWindows 必须成对出现；阈值与时段形状在此把关。
pub(crate) fn validate_strategy(rule: &ModelPrice) -> Result<(), String> {
    if let Some(tiers) = &rule.context_tiers {
        if tiers.is_empty()
            || tiers.len() > 12
            || rule.long_context.is_some()
            || rule.peak.is_some()
        {
            return Err("组合分档需为 1–12 档，不能与旧长上下文/高峰规则混合".into());
        }
        for tier in tiers {
            if tier.label.len() > 200
                || tier
                    .input_min
                    .zip(tier.input_max)
                    .is_some_and(|(a, b)| a >= b)
                || tier
                    .output_min
                    .zip(tier.output_max)
                    .is_some_and(|(a, b)| a >= b)
            {
                return Err("组合分档范围无效".into());
            }
            let r = &tier.rates;
            validate_rate_set(
                [&r.input, &r.output, &r.cache_read, &r.cache_write],
                "分档费率",
            )?;
        }
        for (i, a) in tiers.iter().enumerate() {
            for b in &tiers[i + 1..] {
                let overlap =
                    |amin: Option<u64>, amax: Option<u64>, bmin: Option<u64>, bmax: Option<u64>| {
                        amin.unwrap_or(0) < bmax.unwrap_or(u64::MAX)
                            && bmin.unwrap_or(0) < amax.unwrap_or(u64::MAX)
                    };
                if overlap(a.input_min, a.input_max, b.input_min, b.input_max)
                    && overlap(a.output_min, a.output_max, b.output_min, b.output_max)
                {
                    return Err("组合分档不可重叠".into());
                }
            }
        }
    }
    match (&rule.peak, &rule.peak_windows) {
        (Some(_), None) => return Err("分时段价格需要同时配置高峰时段".into()),
        (None, Some(_)) => return Err("已配置高峰时段，请同时填写高峰费率".into()),
        _ => {}
    }
    if let Some(peak) = &rule.peak {
        validate_rate_set(
            [
                &peak.input,
                &peak.output,
                &peak.cache_read,
                &peak.cache_write,
            ],
            "高峰费率",
        )?;
    }
    if let Some(windows) = &rule.peak_windows {
        validate_peak_windows(windows)?;
    }
    if let Some(long) = &rule.long_context {
        if long.input_over_tokens == 0 || long.input_over_tokens > i64::MAX as u64 {
            return Err("长上下文阈值必须大于 0 且不超过 i64 上限".into());
        }
        validate_rate_set(
            [
                &long.input,
                &long.output,
                &long.cache_read,
                &long.cache_write,
            ],
            "长上下文费率",
        )?;
    }
    Ok(())
}

/// 一档费率至少一项，且每项满足与基础费率相同的十进制约束。
fn validate_rate_set(rates: [&Option<String>; 4], label: &str) -> Result<(), String> {
    if rates.iter().all(|r| r.is_none()) {
        return Err(format!(
            "{label}至少填写一项价格；留空表示未知，0 表示明确免费"
        ));
    }
    for rate in rates.into_iter().flatten() {
        per_token(rate)?;
    }
    Ok(())
}

fn validate_peak_windows(windows: &PeakWindows) -> Result<(), String> {
    if windows.timezone != "Asia/Shanghai" {
        return Err("高峰时段目前仅支持 Asia/Shanghai 时区（固定 UTC+8）".into());
    }
    if windows.ranges.is_empty() || windows.ranges.len() > 4 {
        return Err("高峰时段需配置 1–4 个时间段".into());
    }
    let mut ranges = Vec::with_capacity(windows.ranges.len());
    for raw in &windows.ranges {
        let Some(range) = parse_range(raw) else {
            return Err("高峰时段格式为 HH:MM-HH:MM，开始须早于结束且不跨午夜".into());
        };
        ranges.push(range);
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|w| w[1].0 < w[0].1) {
        return Err("高峰时间段不可重叠".into());
    }
    Ok(())
}

/// "HH:MM-HH:MM" → (起始分钟, 结束分钟)。24 小时制，start < end（不跨午夜）；非法返回 None。
fn parse_range(range: &str) -> Option<(u32, u32)> {
    fn hm(s: &str) -> Option<u32> {
        let (h, m) = s.split_once(':')?;
        if h.len() != 2 || m.len() != 2 {
            return None;
        }
        let h: u32 = h.parse().ok()?;
        let m: u32 = m.parse().ok()?;
        (h < 24 && m < 60).then_some(h * 60 + m)
    }
    let (start, end) = range.split_once('-')?;
    let (start, end) = (hm(start)?, hm(end)?);
    (start < end).then_some((start, end))
}

/// Decimal point shift, with a bounded precision to keep i128 arithmetic checked.
fn per_token(value: &str) -> Result<String, String> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() > 2
        || parts[0].is_empty()
        || parts[0].len() > 9
        || !parts[0].bytes().all(|c| c.is_ascii_digit())
        || parts
            .get(1)
            .is_some_and(|p| p.is_empty() || p.len() > 9 || !p.bytes().all(|c| c.is_ascii_digit()))
    {
        return Err("价格必须为非负十进制数，最多 9 位整数和 9 位小数".into());
    }
    let digits = format!("{}{}", parts[0], parts.get(1).unwrap_or(&""));
    let scale = 6 + parts.get(1).map_or(0, |p| p.len());
    let padded = format!("{:0>width$}", digits, width = scale + 1);
    let split = padded.len() - scale;
    Ok(format!("{}.{}", &padded[..split], &padded[split..]))
}

pub fn matching<'a>(
    rules: &'a [ModelPrice],
    agent: &str,
    source: &str,
    model: Option<&str>,
) -> Option<&'a ModelPrice> {
    let model = model?;
    rules
        .iter()
        .find(|p| p.agent == agent && p.model == model && p.source_id.as_deref() == Some(source))
        .or_else(|| {
            rules
                .iter()
                .find(|p| p.agent == agent && p.model == model && p.source_id.is_none())
        })
}

/// 估算某事件在当前规则下的费用。`occurred_at` 为事件发生的 RFC3339 时间（可空），
/// `input_total` 为事件输入总量（可空）——两者是策略判定所需的唯一事件上下文。
///
/// 解析顺序（与 DATA_CONTRACTS.md「2026-10-12 模型定价与 IPC 增量」一致）：
/// 1. 配置了 longContext：input_total 未知 → Ok(None)（未知 ≠ 0，不按基础费率臆造）；
///    input_total > 阈值 → 整个请求按档位费率；否则按基础费率（长上下文档位无高峰变体，
///    配置了升档的规则不再叠加 peak）。
/// 2. 否则配置了 peak：occurred_at 缺失或不可解析 → Ok(None)；否则换算到 Asia/Shanghai
///    （固定 UTC+8，无夏令时），工作日（周一至周五，法定节假日未实现）且本地时间落入任一
///    半开时段 [start, end) → 高峰费率，其余 → 基础费率。
/// 3. 否则按基础费率。
///
/// 选中的档位中某分量费率为 None 时，该分量按未知处理（沿用 estimate_with_version 的
/// 部分估算语义：只累计“数量与费率都已知”的分量，完整性相应降级）。
pub fn quote(
    rule: &ModelPrice,
    tokens: &TokenSet,
    occurred_at: Option<&str>,
    input_total: Option<u64>,
) -> Result<Option<CostInfo>, String> {
    let Some([input, output, cache_read, cache_write]) =
        select_rates(rule, occurred_at, input_total, tokens.output_total)
    else {
        return Ok(None);
    };
    let convert = |s: &Option<String>| s.as_deref().map(per_token).transpose();
    let price = PriceVersion {
        price_version: rule.version.clone(),
        provider: format!("manual:{}", rule.agent),
        model_version: rule.model.clone(),
        currency: rule.currency.clone(),
        effective_from: None,
        effective_to: None,
        region: rule.source_id.clone(),
        tier: Some("current_user_price_comparison".into()),
        rates: Rates {
            input: convert(input)?,
            output: convert(output)?,
            cache_read: convert(cache_read)?,
            cache_write: convert(cache_write)?,
            unit: Some("per_token".into()),
            ..Default::default()
        },
    };
    pricing::estimate_with_version(&price, tokens).map_err(|e| e.to_string())
}

/// 选出本事件适用的费率档 [input, output, cache_read, cache_write]。
/// 返回 None = 策略判定所需的上下文未知，估算整体未知。
fn select_rates<'a>(
    rule: &'a ModelPrice,
    occurred_at: Option<&str>,
    input_total: Option<u64>,
    output_total: Option<u64>,
) -> Option<[&'a Option<String>; 4]> {
    let base = [
        &rule.input,
        &rule.output,
        &rule.cache_read,
        &rule.cache_write,
    ];
    if let Some(tiers) = &rule.context_tiers {
        let matches = |value: Option<u64>, min: Option<u64>, max: Option<u64>| -> Option<bool> {
            if min.is_none() && max.is_none() {
                return Some(true);
            }
            let v = value?;
            Some(v >= min.unwrap_or(0) && max.is_none_or(|m| v < m))
        };
        let mut selected = None;
        for t in tiers {
            let i = matches(input_total, t.input_min, t.input_max)?;
            if !i {
                continue;
            }
            let o = matches(output_total, t.output_min, t.output_max)?;
            if i && o {
                if selected.is_some() {
                    return None;
                }
                let r = &t.rates;
                selected = Some([&r.input, &r.output, &r.cache_read, &r.cache_write]);
            }
        }
        return selected;
    }
    if let Some(long) = &rule.long_context {
        let input = input_total?;
        if input > long.input_over_tokens {
            return Some([
                &long.input,
                &long.output,
                &long.cache_read,
                &long.cache_write,
            ]);
        }
        return Some(base);
    }
    if let Some(peak) = &rule.peak {
        let windows = rule.peak_windows.as_ref()?;
        if is_peak_time(windows, occurred_at?)? {
            return Some([
                &peak.input,
                &peak.output,
                &peak.cache_read,
                &peak.cache_write,
            ]);
        }
    }
    Some(base)
}

/// occurred_at(RFC3339，可带任意偏移) 换算到 UTC+8 后判定是否高峰：
/// weekdays_only 时周六/周日恒为低谷；本地分钟落入任一时段 [start, end) 为高峰。
/// 时间不可解析或存储的时段形状损坏时返回 None（调用方给出未知估算）。
fn is_peak_time(windows: &PeakWindows, occurred_at: &str) -> Option<bool> {
    debug_assert_eq!(windows.timezone, "Asia/Shanghai", "save() 已白名单校验");
    let t = OffsetDateTime::parse(occurred_at, &Rfc3339).ok()?;
    let t = t.to_offset(UtcOffset::from_hms(8, 0, 0).ok()?);
    if windows.weekdays_only && matches!(t.weekday(), Weekday::Saturday | Weekday::Sunday) {
        return Some(false);
    }
    if windows.exclude_china_holidays {
        // State Council 2026 schedule; future/older calendars stay unknown.
        if t.year() != 2026 {
            return None;
        }
        let md = u16::from(t.month() as u8) * 100 + u16::from(t.day());
        if [
            (101, 103),
            (215, 223),
            (404, 406),
            (501, 505),
            (619, 621),
            (925, 927),
            (1001, 1007),
        ]
        .iter()
        .any(|&(a, b)| a <= md && md <= b)
        {
            return Some(false);
        }
    }
    let minute = u32::from(t.hour()) * 60 + u32::from(t.minute());
    let in_range = windows
        .ranges
        .iter()
        .map(|r| parse_range(r))
        .collect::<Option<Vec<_>>>()?
        .iter()
        .any(|&(start, end)| start <= minute && minute < end);
    Some(in_range)
}

pub fn is_complete(cost: &CostInfo) -> bool {
    cost.completeness == Completeness::Complete && cost.quality == Quality::Estimated
}
