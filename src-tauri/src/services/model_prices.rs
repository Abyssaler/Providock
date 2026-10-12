//! User-entered prices, scoped to an Agent and optional source. Append-only versions;
//! current-price comparisons never rewrite source usage or saved historical estimates.
use crate::{
    domain::{Completeness, CostInfo, Quality, TokenSet},
    services::{
        accounts::now_rfc3339,
        pricing::{self, PriceVersion, Rates},
    },
    storage::Database,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const KIND: &str = "providock_manual_per_million_v1";
const DISABLED: &str = "providock_manual_disabled_v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}
fn yes() -> bool {
    true
}
type Key = (String, Option<String>, String);

pub fn list(db: &Database) -> Result<Vec<ModelPrice>, String> {
    let mut latest: BTreeMap<Key, ModelPrice> = BTreeMap::new();
    // Existing service orders by effective_from then version within each price key.
    for p in pricing::price_versions_list(db).map_err(|e| e.to_string())? {
        if !matches!(p.tier.as_deref(), Some(KIND | DISABLED)) {
            continue;
        }
        let Some(agent) = p.provider.strip_prefix("manual:") else {
            continue;
        };
        let rule = ModelPrice {
            version: p.price_version,
            agent: agent.into(),
            source_id: p.region,
            model: p.model_version,
            currency: p.currency,
            input: p.rates.input,
            output: p.rates.output,
            cache_read: p.rates.cache_read,
            cache_write: p.rates.cache_write,
            enabled: p.tier.as_deref() == Some(KIND),
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
    Ok(latest.into_values().filter(|p| p.enabled).collect())
}

pub fn save(db: &Database, mut rule: ModelPrice) -> Result<ModelPrice, String> {
    if ![
        "claude_code",
        "codex",
        "gemini_cli",
        "grok_build",
        "opencode",
        "pi",
        "mcode",
        "kimi_code",
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
    rule.version = format!(
        "manual-{}-{:08x}",
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
                ..Default::default()
            },
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(rule)
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

pub fn quote(rule: &ModelPrice, tokens: &TokenSet) -> Result<Option<CostInfo>, String> {
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
            input: convert(&rule.input)?,
            output: convert(&rule.output)?,
            cache_read: convert(&rule.cache_read)?,
            cache_write: convert(&rule.cache_write)?,
            unit: Some("per_token".into()),
            ..Default::default()
        },
    };
    pricing::estimate_with_version(&price, tokens).map_err(|e| e.to_string())
}

pub fn is_complete(cost: &CostInfo) -> bool {
    cost.completeness == Completeness::Complete && cost.quality == Quality::Estimated
}
