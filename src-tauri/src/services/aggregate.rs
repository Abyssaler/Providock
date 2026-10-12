//! 聚合查询。口径见 DATA_CONTRACTS：
//! - 汇总在 Rust 侧用 u128 检查算术完成，不依赖 SQLite SUM（防溢出、不转 REAL）。
//! - 未知字段不参与小计；每个总量附覆盖率（已知事件数/筛选事件数）。
//! - 读缓存占比 = Σinput_cache_read / Σinput_total，只在两者均已知的事件上计算（加权）。
//! - 费用分 reported / estimated，按币种分别相加；occurred_at 未知的事件进 time_unknown。

use crate::domain::decimal::{decimal_add, ratio_decimal};
use crate::storage::Database;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Default, Clone)]
pub struct UsageFilter {
    /// RFC3339 半开区间 [from, to)；None = 不限
    pub from: Option<String>,
    pub to: Option<String>,
    pub agent: Option<String>,
    /// 用量页可见 Agent 列表；None = 不限，Some(空列表) 不匹配任何行
    pub agents: Option<Vec<String>>,
    /// 默认只统计本地账本；网关域需显式选择（R12）
    pub stream_domain: Option<String>,
    pub source_id: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricTotal {
    /// 十进制字符串；无已知值 = null（不显示 0）
    pub value: Option<String>,
    /// 该字段已知的事件数
    pub known_events: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoneyTotal {
    pub currency: String,
    pub amount: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBreakdown {
    pub agent: String,
    pub total: MetricTotal,
    pub events: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub events: u64,
    /// 缺发生时间、未计入任何日期桶的事件数
    pub time_unknown_events: u64,
    /// 已知 request_count 之和（null = 完全没有已知值）
    pub request_count: Option<String>,
    pub request_count_known_events: u64,
    pub input_fresh: MetricTotal,
    pub input_cache_read: MetricTotal,
    pub input_cache_write: MetricTotal,
    pub input_total: MetricTotal,
    pub output_total: MetricTotal,
    pub total: MetricTotal,
    /// 读缓存 Token 占比（0..1 四位小数）与覆盖率
    pub cache_read_ratio: Option<String>,
    pub cache_ratio_eligible_events: u64,
    pub cost_reported: Vec<MoneyTotal>,
    pub cost_estimated: Vec<MoneyTotal>,
    pub by_agent: Vec<AgentBreakdown>,
}

#[derive(Default)]
struct Acc {
    sums: [Option<u128>; 6], // fresh, read, write, input_total, output, total
    known: [u64; 6],
    events: u64,
    time_unknown: u64,
    request_count: Option<u128>,
    request_count_known: u64,
    cache_num: u128,
    cache_den: u128,
    cache_eligible: u64,
    cost_reported: BTreeMap<String, String>,
    cost_estimated: BTreeMap<String, String>,
}

const FIELDS: [&str; 6] = [
    "input_fresh",
    "input_cache_read",
    "input_cache_write",
    "input_total",
    "output_total",
    "total",
];

fn col_index(field: &str) -> usize {
    FIELDS.iter().position(|f| *f == field).unwrap()
}

/// `agents` 列表谓词：值走参数绑定，不拼接字面量；Some(空) 语义上匹配不到任何行。
pub(crate) fn push_agents_clause(
    filter: &UsageFilter,
    sql: &mut String,
    params: &mut Vec<rusqlite::types::Value>,
) {
    if let Some(agents) = &filter.agents {
        if agents.is_empty() {
            sql.push_str(" AND 1 = 0");
        } else {
            let placeholders = vec!["?"; agents.len()].join(", ");
            sql.push_str(&format!(" AND agent IN ({placeholders})"));
            for agent in agents {
                params.push(agent.clone().into());
            }
        }
    }
}

pub fn usage_summary(db: &Database, filter: &UsageFilter) -> crate::storage::Result<UsageSummary> {
    let mut sql = String::from(
        "SELECT input_fresh, input_cache_read, input_cache_write, input_total,
                output_total, total, request_count, occurred_at,
                cost_reported_amount, cost_reported_currency,
                cost_estimated_amount, cost_estimated_currency, agent
         FROM usage_events WHERE 1=1",
    );
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(d) = &filter.stream_domain {
        sql.push_str(" AND stream_domain = ?");
        params.push(d.clone().into());
    } else {
        // 默认只看本地账本，网关/导入域显式选择才混入（R12）
        sql.push_str(" AND stream_domain = 'local_log'");
    }
    if let Some(a) = &filter.agent {
        sql.push_str(" AND agent = ?");
        params.push(a.clone().into());
    }
    push_agents_clause(filter, &mut sql, &mut params);
    if let Some(source_id) = &filter.source_id {
        sql.push_str(" AND source_id = ?");
        params.push(source_id.clone().into());
    }
    if let Some(model) = &filter.model {
        sql.push_str(" AND COALESCE(model_normalized, model_raw) = ?");
        params.push(model.clone().into());
    }
    if let Some(f) = &filter.from {
        sql.push_str(" AND occurred_at >= ?");
        params.push(f.clone().into());
    }
    if let Some(t) = &filter.to {
        sql.push_str(" AND occurred_at < ?");
        params.push(t.clone().into());
    }

    let mut overall = Acc::default();
    let mut per_agent: BTreeMap<String, Acc> = BTreeMap::new();

    let mut stmt = db.conn().prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(params.iter()))?;
    while let Some(row) = rows.next()? {
        let agent: String = row.get(12)?;
        let accs = [&mut overall, per_agent.entry(agent).or_default()];
        for acc in accs {
            acc.events += 1;
            let mut vals = [None; 6];
            for (i, v) in vals.iter_mut().enumerate() {
                *v = row.get::<_, Option<i64>>(i)?.map(|x| x as u64);
                if let Some(x) = *v {
                    acc.sums[i] = Some(acc.sums[i].unwrap_or(0) + u128::from(x));
                    acc.known[i] += 1;
                }
            }
            if row.get::<_, Option<String>>(7)?.is_none() {
                acc.time_unknown += 1;
            }
            if let Some(rc) = row.get::<_, Option<i64>>(6)? {
                acc.request_count = Some(acc.request_count.unwrap_or(0) + rc as u128);
                acc.request_count_known += 1;
            }
            if let (Some(read), Some(input_total)) = (vals[1], vals[3]) {
                acc.cache_num += u128::from(read);
                acc.cache_den += u128::from(input_total);
                acc.cache_eligible += 1;
            }
            for (amount_idx, currency_idx, map) in [
                (8usize, 9usize, &mut acc.cost_reported),
                (10usize, 11usize, &mut acc.cost_estimated),
            ] {
                if let (Some(amount), Some(currency)) = (
                    row.get::<_, Option<String>>(amount_idx)?,
                    row.get::<_, Option<String>>(currency_idx)?,
                ) {
                    let entry = map.entry(currency).or_insert_with(|| "0".to_string());
                    if let Some(sum) = decimal_add(entry, &amount) {
                        *entry = sum;
                    }
                }
            }
        }
    }

    fn metric(acc: &Acc, field: &str) -> MetricTotal {
        let i = col_index(field);
        MetricTotal {
            value: acc.sums[i].map(|v| v.to_string()),
            known_events: acc.known[i],
        }
    }
    fn finish(acc: &Acc) -> UsageSummary {
        UsageSummary {
            events: acc.events,
            time_unknown_events: acc.time_unknown,
            request_count: acc.request_count.map(|v| v.to_string()),
            request_count_known_events: acc.request_count_known,
            input_fresh: metric(acc, "input_fresh"),
            input_cache_read: metric(acc, "input_cache_read"),
            input_cache_write: metric(acc, "input_cache_write"),
            input_total: metric(acc, "input_total"),
            output_total: metric(acc, "output_total"),
            total: metric(acc, "total"),
            cache_read_ratio: ratio_decimal(acc.cache_num, acc.cache_den),
            cache_ratio_eligible_events: acc.cache_eligible,
            cost_reported: acc
                .cost_reported
                .iter()
                .map(|(currency, amount)| MoneyTotal {
                    currency: currency.clone(),
                    amount: amount.clone(),
                })
                .collect(),
            cost_estimated: acc
                .cost_estimated
                .iter()
                .map(|(currency, amount)| MoneyTotal {
                    currency: currency.clone(),
                    amount: amount.clone(),
                })
                .collect(),
            by_agent: vec![],
        }
    }

    let mut summary = finish(&overall);
    summary.by_agent = per_agent
        .iter()
        .map(|(agent, acc)| AgentBreakdown {
            agent: agent.clone(),
            total: metric(acc, "total"),
            events: acc.events,
        })
        .collect();
    Ok(summary)
}

/// 按 UTC 日聚合的趋势序列（本机时区分日在 T12 完善，当前契约：UTC 日）。
pub fn usage_series_daily(
    db: &Database,
    filter: &UsageFilter,
) -> crate::storage::Result<Vec<(String, Option<String>)>> {
    let mut sql = String::from(
        "SELECT substr(occurred_at, 1, 10) AS day, total
         FROM usage_events WHERE occurred_at IS NOT NULL",
    );
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if filter.stream_domain.is_none() {
        sql.push_str(" AND stream_domain = 'local_log'");
    } else if let Some(d) = &filter.stream_domain {
        sql.push_str(" AND stream_domain = ?");
        params.push(d.clone().into());
    }
    if let Some(a) = &filter.agent {
        sql.push_str(" AND agent = ?");
        params.push(a.clone().into());
    }
    push_agents_clause(filter, &mut sql, &mut params);
    if let Some(source_id) = &filter.source_id {
        sql.push_str(" AND source_id = ?");
        params.push(source_id.clone().into());
    }
    if let Some(model) = &filter.model {
        sql.push_str(" AND COALESCE(model_normalized, model_raw) = ?");
        params.push(model.clone().into());
    }
    if let Some(f) = &filter.from {
        sql.push_str(" AND occurred_at >= ?");
        params.push(f.clone().into());
    }
    if let Some(t) = &filter.to {
        sql.push_str(" AND occurred_at < ?");
        params.push(t.clone().into());
    }
    let mut stmt = db.conn().prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(params.iter()))?;
    let mut days: BTreeMap<String, u128> = BTreeMap::new();
    let mut day_has_known: BTreeMap<String, bool> = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let day: String = row.get(0)?;
        let total: Option<i64> = row.get(1)?;
        let Some(total) = total else {
            day_has_known.entry(day).or_insert(false);
            continue;
        };
        *days.entry(day.clone()).or_insert(0) += total as u128;
        day_has_known.insert(day, true);
    }
    Ok(day_has_known
        .into_keys()
        .map(|d| {
            let v = days.get(&d).map(|x| x.to_string());
            (d, v)
        })
        .collect())
}
