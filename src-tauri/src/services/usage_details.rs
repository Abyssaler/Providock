//! Read-only model/Agent breakdown, multi-series trend, and bounded event pagination.
use crate::{
    domain::{Quality, TokenSet},
    services::{
        aggregate::{push_agents_clause, UsageFilter},
        model_prices::{self, ModelPrice},
    },
    storage::Database,
};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Amount {
    pub currency: String,
    pub amount: String,
}
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub input_fresh: Option<String>,
    pub input_cache_read: Option<String>,
    pub input_cache_write: Option<String>,
    pub input_total: Option<String>,
    pub output_total: Option<String>,
    pub total: Option<String>,
}
impl Tokens {
    fn from_sums(v: &[Option<u128>; 6]) -> Self {
        Self {
            input_fresh: v[0].map(|n| n.to_string()),
            input_cache_read: v[1].map(|n| n.to_string()),
            input_cache_write: v[2].map(|n| n.to_string()),
            input_total: v[3].map(|n| n.to_string()),
            output_total: v[4].map(|n| n.to_string()),
            total: v[5].map(|n| n.to_string()),
        }
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    pub amount: String,
    pub currency: String,
    pub complete: bool,
    pub version: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub event_id: String,
    pub occurred_at: Option<String>,
    pub agent: String,
    pub model: Option<String>,
    pub source_id: String,
    pub granularity: String,
    pub tokens: Tokens,
    pub cost_calculated: Option<Estimate>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPage {
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
    pub rows: Vec<Event>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub name: String,
    pub agent: Option<String>,
    pub events: u64,
    pub tokens: Tokens,
    pub current_cost: Vec<Amount>,
    pub priced_events: u64,
    pub complete_cost_events: u64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub at: String,
    pub tokens: Tokens,
    pub current_cost: Vec<Amount>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub models: Vec<Group>,
    pub agents: Vec<Group>,
    pub series: Vec<Point>,
    pub current_cost: Vec<Amount>,
    pub priced_events: u64,
    pub complete_cost_events: u64,
}
#[derive(Default)]
struct Acc {
    sums: [Option<u128>; 6],
    costs: BTreeMap<String, String>,
    events: u64,
    priced: u64,
    complete: u64,
}
impl Acc {
    fn add(&mut self, tokens: &[Option<u64>; 6], cost: Option<&Estimate>) -> Result<(), String> {
        self.events += 1;
        for (sum, count) in self.sums.iter_mut().zip(tokens) {
            if let Some(n) = count {
                *sum = Some(
                    sum.unwrap_or(0)
                        .checked_add(u128::from(*n))
                        .ok_or("Token 汇总溢出")?,
                );
            }
        }
        if let Some(cost) = cost {
            self.priced += 1;
            self.complete += u64::from(cost.complete);
            let sum = self
                .costs
                .entry(cost.currency.clone())
                .or_insert_with(|| "0".into());
            *sum = crate::domain::decimal::decimal_add(sum, &cost.amount).ok_or("费用汇总溢出")?;
        }
        Ok(())
    }
    fn amounts(&self) -> Vec<Amount> {
        self.costs
            .iter()
            .map(|(c, a)| Amount {
                currency: c.clone(),
                amount: a.clone(),
            })
            .collect()
    }
    fn group(&self, name: String, agent: Option<String>) -> Group {
        Group {
            name,
            agent,
            events: self.events,
            tokens: Tokens::from_sums(&self.sums),
            current_cost: self.amounts(),
            priced_events: self.priced,
            complete_cost_events: self.complete,
        }
    }
}
/// A single shared predicate keeps summaries, model groups, trends and pagination aligned.
fn predicate(f: &UsageFilter) -> (String, Vec<rusqlite::types::Value>) {
    let mut sql = String::from(" WHERE stream_domain = ?");
    let mut params = vec![f
        .stream_domain
        .clone()
        .unwrap_or_else(|| "local_log".into())
        .into()];
    for (column, value, op) in [
        ("agent", &f.agent, "="),
        ("source_id", &f.source_id, "="),
        ("COALESCE(model_normalized,model_raw)", &f.model, "="),
        ("occurred_at", &f.from, ">="),
        ("occurred_at", &f.to, "<"),
    ] {
        if let Some(v) = value {
            sql.push_str(&format!(" AND {column} {op} ?"));
            params.push(v.clone().into());
        }
    }
    push_agents_clause(f, &mut sql, &mut params);
    (sql, params)
}
const COLUMNS:&str="event_id, occurred_at, agent, COALESCE(model_normalized,model_raw), source_id, granularity, input_fresh, input_cache_read, input_cache_write, input_total, output_total, total";
fn row(row: &rusqlite::Row<'_>, rules: &[ModelPrice]) -> Result<(Event, [Option<u64>; 6]), String> {
    let get_string = |i| row.get::<_, Option<String>>(i).map_err(|e| e.to_string());
    let agent: String = row.get(2).map_err(|e| e.to_string())?;
    let model = get_string(3)?;
    let source: String = row.get(4).map_err(|e| e.to_string())?;
    let mut counts = [None; 6];
    for (i, count) in counts.iter_mut().enumerate() {
        *count = row
            .get::<_, Option<i64>>(6 + i)
            .map_err(|e| e.to_string())?
            .map(|n| u64::try_from(n).map_err(|_| "负 Token 无法计量".to_string()))
            .transpose()?;
    }
    let tokens = TokenSet {
        input_fresh: counts[0],
        input_cache_read: counts[1],
        input_cache_write: counts[2],
        input_total: counts[3],
        output_total: counts[4],
        total: counts[5],
        output_reasoning: None,
        quality: Quality::Reported,
    };
    let occurred_at = get_string(1)?;
    let estimate = model_prices::matching(rules, &agent, &source, model.as_deref())
        .map(|p| {
            let granularity = get_string(5)?;
            if p.context_tiers.is_some()
                && !matches!(granularity.as_deref(), Some("request" | "message"))
            {
                return Ok(None);
            }
            model_prices::quote(p, &tokens, occurred_at.as_deref(), tokens.input_total)
        })
        .transpose()?
        .flatten();
    let cost_calculated = estimate.map(|c| {
        let complete = model_prices::is_complete(&c);
        Estimate {
            amount: c.money.amount,
            currency: c.money.currency,
            complete,
            version: c.price_version.unwrap_or_default(),
        }
    });
    Ok((
        Event {
            event_id: row.get(0).map_err(|e| e.to_string())?,
            occurred_at,
            agent,
            model,
            source_id: source,
            granularity: row.get(5).map_err(|e| e.to_string())?,
            tokens: Tokens::from_sums(&counts.map(|n| n.map(u128::from))),
            cost_calculated,
        },
        counts,
    ))
}
pub fn dashboard(db: &Database, filter: &UsageFilter, hourly: bool) -> Result<Dashboard, String> {
    let rules = model_prices::list(db)?;
    let (where_sql, params) = predicate(filter);
    let mut stmt = db
        .conn()
        .prepare(&format!("SELECT {COLUMNS} FROM usage_events{where_sql}"))
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query(rusqlite::params_from_iter(params.iter()))
        .map_err(|e| e.to_string())?;
    let mut all = Acc::default();
    let mut models: BTreeMap<(Option<String>, String), Acc> = BTreeMap::new();
    let mut agents: BTreeMap<String, Acc> = BTreeMap::new();
    let mut buckets: BTreeMap<String, Acc> = BTreeMap::new();
    while let Some(r) = rows.next().map_err(|e| e.to_string())? {
        let (e, counts) = row(r, &rules)?;
        all.add(&counts, e.cost_calculated.as_ref())?;
        models
            .entry((e.model.clone(), e.agent.clone()))
            .or_default()
            .add(&counts, e.cost_calculated.as_ref())?;
        agents
            .entry(e.agent.clone())
            .or_default()
            .add(&counts, e.cost_calculated.as_ref())?;
        if let Some(at) = e.occurred_at {
            // Parse offsets instead of slicing a source timestamp into a falsely UTC bucket.
            if let Ok(t) =
                time::OffsetDateTime::parse(&at, &time::format_description::well_known::Rfc3339)
            {
                let t = t.to_offset(time::UtcOffset::UTC);
                let key = if hourly {
                    format!("{}T{:02}:00:00Z", t.date(), t.hour())
                } else {
                    format!("{}T00:00:00Z", t.date())
                };
                buckets
                    .entry(key)
                    .or_default()
                    .add(&counts, e.cost_calculated.as_ref())?;
            }
        }
    }
    let sort = |items: &mut Vec<Group>| {
        items.sort_by(|a, b| {
            let v = |g: &Group| {
                g.tokens
                    .total
                    .as_deref()
                    .and_then(|s| s.parse::<u128>().ok())
                    .unwrap_or(0)
            };
            v(b).cmp(&v(a))
                .then(a.name.cmp(&b.name))
                .then(a.agent.cmp(&b.agent))
        })
    };
    let mut model_groups: Vec<_> = models
        .into_iter()
        .map(|((model, agent), a)| {
            a.group(model.unwrap_or_else(|| "未提供模型".into()), Some(agent))
        })
        .collect();
    let mut agent_groups: Vec<_> = agents
        .into_iter()
        .map(|(name, a)| a.group(name, None))
        .collect();
    sort(&mut model_groups);
    sort(&mut agent_groups);
    Ok(Dashboard {
        models: model_groups,
        agents: agent_groups,
        series: buckets
            .into_iter()
            .map(|(at, a)| Point {
                at,
                tokens: Tokens::from_sums(&a.sums),
                current_cost: a.amounts(),
            })
            .collect(),
        current_cost: all.amounts(),
        priced_events: all.priced,
        complete_cost_events: all.complete,
    })
}
pub fn events(
    db: &Database,
    filter: &UsageFilter,
    page: u32,
    page_size: u32,
) -> Result<EventPage, String> {
    let page = page.max(1);
    let page_size = page_size.clamp(1, 100);
    let (where_sql, mut params) = predicate(filter);
    let total: i64 = db
        .conn()
        .query_row(
            &format!("SELECT COUNT(*) FROM usage_events{where_sql}"),
            rusqlite::params_from_iter(params.iter()),
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let page = page.min(
        (total as u64)
            .div_ceil(u64::from(page_size))
            .max(1)
            .min(u64::from(u32::MAX)) as u32,
    );
    params.push(i64::from(page_size).into());
    params.push((i64::from(page - 1) * i64::from(page_size)).into());
    let mut stmt=db.conn().prepare(&format!("SELECT {COLUMNS} FROM usage_events{where_sql} ORDER BY occurred_at DESC,event_id DESC LIMIT ? OFFSET ?")).map_err(|e|e.to_string())?;
    let mut rows = stmt
        .query(rusqlite::params_from_iter(params.iter()))
        .map_err(|e| e.to_string())?;
    let rules = model_prices::list(db)?;
    let mut result = Vec::new();
    while let Some(r) = rows.next().map_err(|e| e.to_string())? {
        result.push(row(r, &rules)?.0);
    }
    Ok(EventPage {
        total: total as u64,
        page,
        page_size,
        rows: result,
    })
}
