//! User-triggered AIHOT catalogue import. No credentials or local model names leave the device.
//! Only exact normalized identifiers match; manual rules (including disabled ones) win.
use super::{
    accounts::now_rfc3339,
    model_prices::{self, ModelPrice},
    pricing::PriceSource,
};
use crate::storage::Database;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    time::Duration,
};

pub const SOURCE: &str = "https://aihot.news/leaderboard";
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MARKER: &str = "window.__reactRouterContext.streamController.enqueue(";

#[derive(Debug, Clone)]
pub struct CataloguePrice {
    pub slug: String,
    pub name: String,
    pub input: Option<String>,
    pub output: Option<String>,
    pub cached: Option<String>,
    pub official_url: String,
}
#[derive(Debug)]
pub struct Catalogue {
    pub models: Vec<CataloguePrice>,
    pub source_updated_at: String,
    pub fx_rate: String,
    pub fx_date: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportTarget {
    pub agent: String,
    pub model: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported: usize,
    pub preserved: usize,
    pub unmatched: Vec<String>,
    pub catalogue_models: usize,
    pub source_updated_at: String,
    pub fx_rate: String,
    pub fx_date: String,
}

/// A single fixed HTTPS GET. Redirects, cookies and account authentication are disabled.
pub fn fetch() -> Result<Catalogue, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("Mozilla/5.0 Providock/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "无法创建价格请求".to_string())?;
    let response = client
        .get(SOURCE)
        .send()
        .map_err(|_| "价格下载失败，请检查网络后重试；已有价格未改动".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "AIHOT 返回 HTTP {}，已有价格未改动",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BYTES as u64)
    {
        return Err("价格页面过大，已停止导入".into());
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "价格下载中断；已有价格未改动".to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("价格页面过大，已停止导入".into());
    }
    parse(std::str::from_utf8(&bytes).map_err(|_| "价格页面编码不兼容".to_string())?)
}

fn reference<'a>(table: &'a [Value], value: &Value) -> Option<&'a Value> {
    value.as_u64().and_then(|i| table.get(i as usize))
}
fn field<'a>(table: &'a [Value], object: &Value, name: &str) -> Option<&'a Value> {
    object.as_object()?.iter().find_map(|(key, value)| {
        let index: usize = key.strip_prefix('_')?.parse().ok()?;
        (table.get(index)?.as_str()? == name)
            .then(|| reference(table, value))
            .flatten()
    })
}
fn text(table: &[Value], object: &Value, name: &str) -> Option<String> {
    field(table, object, name)?
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 1000)
        .map(str::to_string)
}
/// AIHOT's CNY values may carry JS floating-point tails. Normalize to at most 9 decimal places.
fn rate(value: Option<&Value>) -> Option<String> {
    let n = value?.as_f64()?;
    if !n.is_finite() || !(0.0..=999_999_999.0).contains(&n) {
        return None;
    }
    let formatted = format!("{n:.9}");
    let normalized = formatted.trim_end_matches('0').trim_end_matches('.');
    // A positive rate below our supported precision must stay unknown, never become free.
    if n > 0.0 && normalized == "0" {
        return None;
    }
    Some(normalized.to_string())
}

/// Decode JSON strings and reference tables only. Never execute scripts from the remote page.
pub fn parse(html: &str) -> Result<Catalogue, String> {
    let incompatible = || "AIHOT 价格格式已变化或数据不完整；已有价格未改动".to_string();
    if html.len() > MAX_BYTES {
        return Err(incompatible());
    }
    let encoded = html.split_once(MARKER).ok_or_else(incompatible)?.1;
    let decoded = serde_json::Deserializer::from_str(encoded)
        .into_iter::<String>()
        .next()
        .ok_or_else(incompatible)?
        .map_err(|_| incompatible())?;
    let values: Vec<Value> = serde_json::from_str(&decoded).map_err(|_| incompatible())?;
    if values.is_empty() || values.len() > 20_000 {
        return Err(incompatible());
    }
    let loader = field(&values, &values[0], "loaderData").ok_or_else(incompatible)?;
    let board = field(&values, loader, "leaderboard-boards").ok_or_else(incompatible)?;
    let run = field(&values, board, "run").ok_or_else(incompatible)?;
    let source_updated_at = text(&values, run, "generatedAt").ok_or_else(incompatible)?;
    time::OffsetDateTime::parse(
        &source_updated_at,
        &time::format_description::well_known::Rfc3339,
    )
    .map_err(|_| incompatible())?;
    let fx = field(&values, run, "fx").ok_or_else(incompatible)?;
    let fx_rate = rate(field(&values, fx, "rate"))
        .filter(|s| s != "0")
        .ok_or_else(incompatible)?;
    let fx_date = text(&values, fx, "asOf")
        .filter(|s| {
            s.len() == 10
                && s.bytes().enumerate().all(|(i, b)| {
                    if i == 4 || i == 7 {
                        b == b'-'
                    } else {
                        b.is_ascii_digit()
                    }
                })
        })
        .ok_or_else(incompatible)?;
    let models = field(&values, board, "models")
        .and_then(Value::as_object)
        .ok_or_else(incompatible)?;
    let mut quotes = BTreeMap::new();
    for entry in models.values().filter_map(|v| reference(&values, v)) {
        let Some(model) = field(&values, entry, "model") else {
            continue;
        };
        let Some(price) = field(&values, entry, "price").filter(|p| p.is_object()) else {
            continue;
        };
        let (Some(slug), Some(name), Some(official_url)) = (
            text(&values, model, "slug"),
            text(&values, model, "name"),
            text(&values, price, "officialUrl"),
        ) else {
            continue;
        };
        if !official_url.starts_with("https://") || slug.len() > 200 || name.len() > 200 {
            continue;
        }
        let input = rate(field(&values, price, "inputCny"));
        let output = rate(field(&values, price, "outputCny"));
        let cached = rate(field(&values, price, "cachedCny"));
        // Missing components remain unknown; a page with no usable input/output fails safely.
        if input.is_none() && output.is_none() {
            continue;
        }
        if quotes
            .insert(
                slug.clone(),
                CataloguePrice {
                    slug,
                    name,
                    input,
                    output,
                    cached,
                    official_url,
                },
            )
            .is_some()
        {
            return Err(incompatible());
        }
    }
    if quotes.is_empty() {
        return Err(incompatible());
    }
    Ok(Catalogue {
        models: quotes.into_values().collect(),
        source_updated_at,
        fx_rate,
        fx_date,
    })
}

fn normalized(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}
pub fn match_price<'a>(catalogue: &'a Catalogue, model: &str) -> Option<&'a CataloguePrice> {
    let name = if let Some((prefix, tail)) = model.split_once('/') {
        match prefix.to_ascii_lowercase().as_str() {
            "openai" | "anthropic" | "google" | "deepseek" | "zhipuai" | "z-ai" | "moonshotai" => {
                tail.to_string()
            }
            "kimi-code" => format!("kimi-{tail}"),
            _ => model.to_string(),
        }
    } else {
        model.to_string()
    };
    let key = normalized(&name);
    if key.is_empty() {
        return None;
    }
    let mut matched = catalogue
        .models
        .iter()
        .filter(|p| normalized(&p.slug) == key || normalized(&p.name) == key);
    let first = matched.next()?;
    matched.next().is_none().then_some(first)
}

/// Apply one batch atomically. Append new imported versions only; preserve manual, source-specific, complex and disabled rules.
pub fn apply(
    db: &Database,
    catalogue: &Catalogue,
    target: Option<ImportTarget>,
) -> Result<ImportResult, String> {
    let existing = model_prices::list_with_disabled(db)?;
    let mut pairs = BTreeSet::<(String, String)>::new();
    {
        let mut stmt = db.conn().prepare("SELECT DISTINCT agent, COALESCE(model_normalized,model_raw) FROM usage_events WHERE stream_domain='local_log' AND COALESCE(model_normalized,model_raw) IS NOT NULL").map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        for pair in rows {
            pairs.insert(pair.map_err(|e| e.to_string())?);
        }
    }
    if let Some(target) = target {
        if !target.model.trim().is_empty() {
            pairs.insert((target.agent, target.model.trim().into()));
        }
    }
    let mut result = ImportResult {
        imported: 0,
        preserved: 0,
        unmatched: vec![],
        catalogue_models: catalogue.models.len(),
        source_updated_at: catalogue.source_updated_at.clone(),
        fx_rate: catalogue.fx_rate.clone(),
        fx_date: catalogue.fx_date.clone(),
    };
    let mut planned = vec![];
    for (agent, model) in pairs {
        let current: Vec<_> = existing
            .iter()
            .filter(|p| p.agent == agent && p.model == model)
            .collect();
        if current.iter().any(|p| {
            !p.enabled
                || p.source.is_none()
                || p.source_id.is_some()
                || p.peak.is_some()
                || p.long_context.is_some()
        }) {
            result.preserved += 1;
            continue;
        }
        let Some(price) = match_price(catalogue, &model) else {
            result.unmatched.push(model);
            continue;
        };
        if current.iter().any(|p| {
            p.currency == "CNY"
                && p.input == price.input
                && p.output == price.output
                && p.cache_read == price.cached
                && p.cache_write.is_none()
        }) {
            result.preserved += 1;
            continue;
        }
        planned.push((agent, model, price));
    }
    db.conn()
        .execute_batch("SAVEPOINT aihot_import")
        .map_err(|e| e.to_string())?;
    let write = (|| {
        for (agent, model, price) in planned {
            let source = PriceSource {
                source_url: SOURCE.into(),
                official_url: price.official_url.clone(),
                retrieved_at: now_rfc3339(),
                source_updated_at: catalogue.source_updated_at.clone(),
                fx_rate: catalogue.fx_rate.clone(),
                fx_date: catalogue.fx_date.clone(),
            };
            model_prices::save_imported(
                db,
                ModelPrice {
                    version: String::new(),
                    agent,
                    source_id: None,
                    model,
                    currency: "CNY".into(),
                    input: price.input.clone(),
                    output: price.output.clone(),
                    cache_read: price.cached.clone(),
                    cache_write: None,
                    enabled: true,
                    peak: None,
                    peak_windows: None,
                    long_context: None,
                    source: None,
                },
                source,
            )?;
            result.imported += 1;
        }
        Ok::<(), String>(())
    })();
    if let Err(error) = write {
        let _ = db
            .conn()
            .execute_batch("ROLLBACK TO aihot_import; RELEASE aihot_import");
        return Err(error);
    }
    db.conn()
        .execute_batch("RELEASE aihot_import")
        .map_err(|e| e.to_string())?;
    result.unmatched.sort();
    result.unmatched.dedup();
    Ok(result)
}
