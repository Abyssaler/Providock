//! 统计导出（T14）。
//!
//! 依据：
//! - `docs/specs/SECURITY_PRIVACY.md` 第 5 节：导出采用用户选择的文件位置，预览时间、路径脱敏、
//!   字段与范围；CSV 对以 `=`/`+`/`-`/`@`/Tab/CR/LF 开头的文本防公式注入；JSON 带契约版本、
//!   质量、币种、scope 与覆盖信息；正文导出是单独明确操作，默认不随统计导出。
//! - `docs/specs/DATA_CONTRACTS.md` 第 1/6/9 节：Token/金额一律十进制字符串；不同币种不合并；
//!   导出字段集合不得含 `project_ref` 明文。
//! - `docs/ROADMAP_AND_ACCEPTANCE.md` C15。
//!
//! 设计：
//! - [`export_statistics`] 只做**预览**（范围 / 字段 / 记录数 / 脱敏说明 / 是否含费用），
//!   绝不创建文件；调用方拿预览确认后再调 [`write_export`] 真正写盘。
//! - `project_ref` 默认导出为匿名标签 `<file:…>`（`services::redact::anon_file_id` 的稳定哈希），
//!   仅当 [`ExportPathPolicy::keep_project_paths`] 显式置真才保留原始路径。
//! - 本库结构上不保存凭据与正文，导出字段集合里也不含这些列（见模块测试）。

use crate::services::aggregate::UsageFilter;
use crate::services::redact::anon_file_id;
use crate::storage::{Database, StorageError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// 导出契约版本（JSON 顶层字段 `contract_version`）。
pub const CONTRACT_VERSION: &str = "providock.statistics.v2";

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ExportError>;

// ---------------------------------------------------------------------------
// 输入
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Csv,
    Json,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Json => "json",
        }
    }
}

/// 导出路径策略。
///
/// - `target_path`：用户选择的写出位置；预览阶段只做展示，不创建/覆盖文件。
/// - `keep_project_paths`：默认 `false`，`project_ref` 导出为匿名标签；
///   用户显式设为 `true` 才在导出里保留原始路径（展示/取证需要时）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportPathPolicy {
    pub target_path: PathBuf,
    pub keep_project_paths: bool,
}

impl ExportPathPolicy {
    pub fn new(target_path: impl Into<PathBuf>) -> Self {
        Self {
            target_path: target_path.into(),
            keep_project_paths: false,
        }
    }

    pub fn keep_project_paths(mut self, keep: bool) -> Self {
        self.keep_project_paths = keep;
        self
    }
}

// ---------------------------------------------------------------------------
// 字段与预览
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColKind {
    Text,
    /// 十进制字符串：token 数 / 金额 / 计数。原样写出，**不加**防公式注入前缀
    /// （负金额 `-3.5` 是合法数值，不是公式）。
    Decimal,
}

struct Column {
    name: &'static str,
    kind: ColKind,
    /// 单位/语义：tokens / currency / count / timestamp / id / path / model / quality ...
    unit: &'static str,
    /// 该列的质量语义说明。
    quality: &'static str,
}

/// 导出列清单（顺序即 CSV 列顺序）。契约第 9 节：普通表不得有凭据/正文字段，
/// 这里也据表结构生成，结构上不含 `credential_*` / `prompt_body` / `response_body`。
const COLUMNS: &[Column] = &[
    Column {
        name: "event_id",
        kind: ColKind::Text,
        unit: "id",
        quality: "identity",
    },
    Column {
        name: "agent",
        kind: ColKind::Text,
        unit: "agent",
        quality: "reported",
    },
    Column {
        name: "source_id",
        kind: ColKind::Text,
        unit: "id",
        quality: "identity",
    },
    Column {
        name: "stream_domain",
        kind: ColKind::Text,
        unit: "domain",
        quality: "reported",
    },
    Column {
        name: "session_id",
        kind: ColKind::Text,
        unit: "id",
        quality: "reported",
    },
    Column {
        name: "actor_id",
        kind: ColKind::Text,
        unit: "id",
        quality: "reported",
    },
    Column {
        name: "granularity",
        kind: ColKind::Text,
        unit: "granularity",
        quality: "reported",
    },
    Column {
        name: "occurred_at",
        kind: ColKind::Text,
        unit: "timestamp",
        quality: "source_time_or_null",
    },
    Column {
        name: "observed_at",
        kind: ColKind::Text,
        unit: "timestamp",
        quality: "local_scan_time",
    },
    Column {
        name: "project_ref",
        kind: ColKind::Text,
        unit: "path",
        quality: "anonymized_label",
    },
    Column {
        name: "model_raw",
        kind: ColKind::Text,
        unit: "model",
        quality: "reported",
    },
    Column {
        name: "model_normalized",
        kind: ColKind::Text,
        unit: "model",
        quality: "derived",
    },
    Column {
        name: "request_count",
        kind: ColKind::Decimal,
        unit: "count",
        quality: "reported_or_null",
    },
    Column {
        name: "input_fresh",
        kind: ColKind::Decimal,
        unit: "tokens",
        quality: "per_event",
    },
    Column {
        name: "input_cache_read",
        kind: ColKind::Decimal,
        unit: "tokens",
        quality: "per_event",
    },
    Column {
        name: "input_cache_write",
        kind: ColKind::Decimal,
        unit: "tokens",
        quality: "per_event",
    },
    Column {
        name: "input_total",
        kind: ColKind::Decimal,
        unit: "tokens",
        quality: "per_event",
    },
    Column {
        name: "output_total",
        kind: ColKind::Decimal,
        unit: "tokens",
        quality: "per_event",
    },
    Column {
        name: "output_reasoning",
        kind: ColKind::Decimal,
        unit: "tokens",
        quality: "per_event",
    },
    Column {
        name: "total",
        kind: ColKind::Decimal,
        unit: "tokens",
        quality: "per_event",
    },
    Column {
        name: "tokens_quality",
        kind: ColKind::Text,
        unit: "quality",
        quality: "reported_derived_estimated_unknown",
    },
    Column {
        name: "completeness",
        kind: ColKind::Text,
        unit: "completeness",
        quality: "complete_partial_unknown",
    },
    Column {
        name: "cost_estimated_amount",
        kind: ColKind::Decimal,
        unit: "currency",
        quality: "estimated",
    },
    Column {
        name: "cost_estimated_currency",
        kind: ColKind::Text,
        unit: "currency",
        quality: "reported",
    },
    Column {
        name: "cost_estimated_price_version",
        kind: ColKind::Text,
        unit: "price_version",
        quality: "estimated",
    },
    Column {
        name: "account_link",
        kind: ColKind::Text,
        unit: "json",
        quality: "confirmed_explicit_or_unknown",
    },
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportField {
    pub name: &'static str,
    pub unit: &'static str,
    pub quality: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RangeInfo {
    /// 半开区间 `[from, to)`；`None` = 不限。
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeInfo {
    pub agent: Option<String>,
    /// 未显式选择时默认只导出本地账本（`local_log`），与聚合口径一致。
    pub stream_domain: String,
}

/// 导出预览：确认前的全部信息，不产生任何文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreview {
    pub contract_version: &'static str,
    pub format: ExportFormat,
    pub target_path: String,
    pub range: RangeInfo,
    pub scope: ScopeInfo,
    pub fields: Vec<ExportField>,
    pub record_count: u64,
    /// `project_ref` 是否保留原始路径（默认脱敏为匿名标签）。
    pub keep_project_paths: bool,
    /// 是否含费用（任一事件有 reported / estimated 金额）。
    pub contains_cost: bool,
    /// 出现的币种（不同币种分开列出，绝不合并）。
    pub currencies: Vec<String>,
    /// 人类可读脱敏说明（与 `note_keys` 对应，供前端 i18n）。
    pub redaction_notes: Vec<String>,
    pub note_keys: Vec<&'static str>,
    pub generated_at: String,
}

/// 写盘结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOutcome {
    pub contract_version: &'static str,
    pub format: ExportFormat,
    pub target_path: String,
    pub record_count: u64,
    pub bytes_written: usize,
    pub generated_at: String,
}

// ---------------------------------------------------------------------------
// 行数据
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRow {
    pub event_id: String,
    pub agent: String,
    pub source_id: String,
    pub stream_domain: String,
    pub session_id: Option<String>,
    pub actor_id: Option<String>,
    pub granularity: String,
    pub occurred_at: Option<String>,
    pub observed_at: String,
    /// 已按策略脱敏后的导出值（匿名标签或原始路径）。
    pub project_ref: Option<String>,
    pub model_raw: Option<String>,
    pub model_normalized: Option<String>,
    pub request_count: Option<String>,
    pub input_fresh: Option<String>,
    pub input_cache_read: Option<String>,
    pub input_cache_write: Option<String>,
    pub input_total: Option<String>,
    pub output_total: Option<String>,
    pub output_reasoning: Option<String>,
    pub total: Option<String>,
    pub tokens_quality: String,
    pub completeness: String,
    pub cost_estimated_amount: Option<String>,
    pub cost_estimated_currency: Option<String>,
    pub cost_estimated_price_version: Option<String>,
    pub account_link: Option<String>,
}

impl ExportRow {
    /// 按 [`COLUMNS`] 顺序取列值。`None` 表示源未提供（导出为空单元格，不补零）。
    fn cells(&self) -> Vec<(&'static Column, Option<String>)> {
        let v: Vec<Option<String>> = vec![
            Some(self.event_id.clone()),
            Some(self.agent.clone()),
            Some(self.source_id.clone()),
            Some(self.stream_domain.clone()),
            self.session_id.clone(),
            self.actor_id.clone(),
            Some(self.granularity.clone()),
            self.occurred_at.clone(),
            Some(self.observed_at.clone()),
            self.project_ref.clone(),
            self.model_raw.clone(),
            self.model_normalized.clone(),
            self.request_count.clone(),
            self.input_fresh.clone(),
            self.input_cache_read.clone(),
            self.input_cache_write.clone(),
            self.input_total.clone(),
            self.output_total.clone(),
            self.output_reasoning.clone(),
            self.total.clone(),
            Some(self.tokens_quality.clone()),
            Some(self.completeness.clone()),
            self.cost_estimated_amount.clone(),
            self.cost_estimated_currency.clone(),
            self.cost_estimated_price_version.clone(),
            self.account_link.clone(),
        ];
        COLUMNS.iter().zip(v).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Coverage {
    events_total: u64,
    events_with_known_time: u64,
    events_with_unknown_time: u64,
    events_with_known_tokens: u64,
    events_with_unknown_tokens: u64,
    events_with_estimated_cost: u64,
    events_without_cost: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct QualityBlock {
    /// 各 `tokens_quality` 取值的事件数（reported / derived / estimated / unknown）。
    tokens_quality_counts: BTreeMap<String, u64>,
    /// 未知一律表示源未提供，绝不等于 0。
    unknown_is_not_zero: bool,
}

struct ExportData {
    rows: Vec<ExportRow>,
    contains_cost: bool,
    currencies: Vec<String>,
    coverage: Coverage,
    quality: QualityBlock,
}

// ---------------------------------------------------------------------------
// CSV 防公式注入
// ---------------------------------------------------------------------------

/// 文本字段的 CSV 编码：以 `=`/`+`/`-`/`@`/Tab/CR/LF 开头时前缀 `'`，
/// 随后按 CSV 规则引用（含 `,`/`"`/CR/LF 时用双引号包裹，内部 `"` 翻倍）。
///
/// 十进制字段不走这里（见 [`ColKind::Decimal`]）：`-3.5` 是数值，原样写出。
pub fn csv_text_field(value: &str) -> String {
    let needs_prefix = matches!(
        value.chars().next(),
        Some('=') | Some('+') | Some('-' | '@' | '\t' | '\r' | '\n')
    );
    let prefixed = if needs_prefix {
        format!("'{value}")
    } else {
        value.to_string()
    };
    csv_quote(&prefixed)
}

fn csv_quote(value: &str) -> String {
    if value.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

// ---------------------------------------------------------------------------
// 采集行
// ---------------------------------------------------------------------------

const SELECT_COLUMNS: &str = "event_id, agent, source_id, stream_domain, session_id, actor_id,
     granularity, occurred_at, observed_at, project_ref, model_raw, model_normalized,
     request_count, input_fresh, input_cache_read, input_cache_write, input_total,
     output_total, output_reasoning, total, tokens_quality, completeness,
     cost_estimated_amount,
     cost_estimated_currency, cost_estimated_price_version, account_link";

fn effective_domain(filter: &UsageFilter) -> String {
    filter
        .stream_domain
        .clone()
        .unwrap_or_else(|| "local_log".to_string())
}

fn collect(db: &Database, filter: &UsageFilter, keep_project_paths: bool) -> Result<ExportData> {
    let mut sql = format!("SELECT {SELECT_COLUMNS} FROM usage_events WHERE 1=1");
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(d) = &filter.stream_domain {
        sql.push_str(" AND stream_domain = ?");
        params.push(d.clone().into());
    } else {
        sql.push_str(" AND stream_domain = 'local_log'");
    }
    if let Some(a) = &filter.agent {
        sql.push_str(" AND agent = ?");
        params.push(a.clone().into());
    }
    if let Some(f) = &filter.from {
        sql.push_str(" AND occurred_at >= ?");
        params.push(f.clone().into());
    }
    if let Some(t) = &filter.to {
        sql.push_str(" AND occurred_at < ?");
        params.push(t.clone().into());
    }
    sql.push_str(" ORDER BY occurred_at, event_id");

    let mut currencies: BTreeSet<String> = BTreeSet::new();
    let mut quality_counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut coverage = Coverage {
        events_total: 0,
        events_with_known_time: 0,
        events_with_unknown_time: 0,
        events_with_known_tokens: 0,
        events_with_unknown_tokens: 0,
        events_with_estimated_cost: 0,
        events_without_cost: 0,
    };
    let mut contains_cost = false;
    let mut rows = Vec::new();

    let mut stmt = db.conn().prepare(&sql)?;
    let mut query = stmt.query(rusqlite::params_from_iter(params.iter()))?;
    while let Some(r) = query.next()? {
        let int_str = |i: usize| -> std::result::Result<Option<String>, rusqlite::Error> {
            Ok(r.get::<_, Option<i64>>(i)?.map(|x| x.to_string()))
        };
        let project_raw: Option<String> = r.get(9)?;
        let project_ref = match project_raw {
            Some(path) if !keep_project_paths => Some(format!("<file:{}>", anon_file_id(&path))),
            other => other,
        };
        let cost_estimated_amount: Option<String> = r.get(22)?;
        let cost_estimated_currency: Option<String> = r.get(23)?;

        coverage.events_total += 1;
        let occurred_at: Option<String> = r.get(7)?;
        if occurred_at.is_some() {
            coverage.events_with_known_time += 1;
        } else {
            coverage.events_with_unknown_time += 1;
        }
        let total = int_str(19)?;
        if total.is_some() {
            coverage.events_with_known_tokens += 1;
        } else {
            coverage.events_with_unknown_tokens += 1;
        }
        let estimated = cost_estimated_amount.is_some();

        if estimated {
            coverage.events_with_estimated_cost += 1;
        }
        if !estimated {
            coverage.events_without_cost += 1;
        }
        if estimated {
            contains_cost = true;
        }

        if let Some(c) = &cost_estimated_currency {
            currencies.insert(c.clone());
        }
        let tokens_quality: String = r.get(20)?;
        *quality_counts.entry(tokens_quality.clone()).or_insert(0) += 1;

        rows.push(ExportRow {
            event_id: r.get(0)?,
            agent: r.get(1)?,
            source_id: r.get(2)?,
            stream_domain: r.get(3)?,
            session_id: r.get(4)?,
            actor_id: r.get(5)?,
            granularity: r.get(6)?,
            occurred_at,
            observed_at: r.get(8)?,
            project_ref,
            model_raw: r.get(10)?,
            model_normalized: r.get(11)?,
            request_count: int_str(12)?,
            input_fresh: int_str(13)?,
            input_cache_read: int_str(14)?,
            input_cache_write: int_str(15)?,
            input_total: int_str(16)?,
            output_total: int_str(17)?,
            output_reasoning: int_str(18)?,
            total,
            tokens_quality,
            completeness: r.get(21)?,
            cost_estimated_amount,
            cost_estimated_currency,
            cost_estimated_price_version: r.get(24)?,
            account_link: r.get(25)?,
        });
    }

    Ok(ExportData {
        rows,
        contains_cost,
        currencies: currencies.into_iter().collect(),
        coverage,
        quality: QualityBlock {
            tokens_quality_counts: quality_counts,
            unknown_is_not_zero: true,
        },
    })
}

fn field_list() -> Vec<ExportField> {
    COLUMNS
        .iter()
        .map(|c| ExportField {
            name: c.name,
            unit: c.unit,
            quality: c.quality,
        })
        .collect()
}

fn redaction_notes(
    format: ExportFormat,
    keep_project_paths: bool,
) -> (Vec<String>, Vec<&'static str>) {
    let mut notes = Vec::new();
    let mut keys = Vec::new();
    if keep_project_paths {
        notes.push("project_ref 以原始路径导出（用户显式选择）".to_string());
        keys.push("export.redaction.project_paths_kept");
    } else {
        notes.push("project_ref 已脱敏为匿名标签 <file:…>（稳定哈希）".to_string());
        keys.push("export.redaction.project_paths_anonymized");
    }
    notes.push("不含凭据、Cookie 或任何明文秘密".to_string());
    keys.push("export.redaction.no_credentials");
    notes.push("不含会话正文（统计库本就不保存正文）".to_string());
    keys.push("export.redaction.no_body");
    notes.push("未知值导出为空，绝不补零".to_string());
    keys.push("export.redaction.unknown_not_zero");
    if format == ExportFormat::Csv {
        notes.push("CSV 文本字段以 = + - @ Tab CR LF 开头时前缀单引号防止公式注入".to_string());
        keys.push("export.redaction.csv_formula_prefix");
    }
    (notes, keys)
}

// ---------------------------------------------------------------------------
// 预览 / 写盘
// ---------------------------------------------------------------------------

/// 生成导出预览：范围、字段清单、记录数、脱敏说明、是否含费用。
///
/// 只读数据库，不创建/覆盖任何文件。调用方确认后调 [`write_export`]。
pub fn export_statistics(
    db: &Database,
    filter: &UsageFilter,
    format: ExportFormat,
    path_policy: &ExportPathPolicy,
) -> Result<ExportPreview> {
    let data = collect(db, filter, path_policy.keep_project_paths)?;
    let (redaction_notes, note_keys) = redaction_notes(format, path_policy.keep_project_paths);
    let record_count = data.rows.len() as u64;
    Ok(ExportPreview {
        contract_version: CONTRACT_VERSION,
        format,
        target_path: path_policy.target_path.to_string_lossy().into_owned(),
        range: RangeInfo {
            from: filter.from.clone(),
            to: filter.to.clone(),
        },
        scope: ScopeInfo {
            agent: filter.agent.clone(),
            stream_domain: effective_domain(filter),
        },
        fields: field_list(),
        record_count,
        keep_project_paths: path_policy.keep_project_paths,
        contains_cost: data.contains_cost,
        currencies: data.currencies,
        redaction_notes,
        note_keys,
        generated_at: crate::services::accounts::now_rfc3339(),
    })
}

/// 真正写盘（调用方已用 [`export_statistics`] 预览并确认）。
///
/// 父目录不存在时创建；同名文件会被覆盖。文件删除/覆盖不保证底层介质不可恢复。
pub fn write_export(
    db: &Database,
    filter: &UsageFilter,
    format: ExportFormat,
    path_policy: &ExportPathPolicy,
) -> Result<ExportOutcome> {
    let data = collect(db, filter, path_policy.keep_project_paths)?;
    let generated_at = crate::services::accounts::now_rfc3339();
    let content = match format {
        ExportFormat::Csv => render_csv(&data.rows),
        ExportFormat::Json => render_json(&data, filter, generated_at.clone())?,
    };
    let path = &path_policy.target_path;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, content.as_bytes())?;
    Ok(ExportOutcome {
        contract_version: CONTRACT_VERSION,
        format,
        target_path: path.to_string_lossy().into_owned(),
        record_count: data.rows.len() as u64,
        bytes_written: content.len(),
        generated_at,
    })
}

fn render_csv(rows: &[ExportRow]) -> String {
    let mut out = String::new();
    out.push_str(&COLUMNS.iter().map(|c| c.name).collect::<Vec<_>>().join(","));
    out.push('\n');
    for row in rows {
        let line: Vec<String> = row
            .cells()
            .into_iter()
            .map(|(col, value)| match value {
                None => String::new(),
                // 十进制字符串原样；文本字段做防注入前缀 + CSV 引用
                Some(v) => match col.kind {
                    ColKind::Decimal => v,
                    ColKind::Text => csv_text_field(&v),
                },
            })
            .collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

fn render_json(data: &ExportData, filter: &UsageFilter, generated_at: String) -> Result<String> {
    let records: Vec<serde_json::Value> = data
        .rows
        .iter()
        .map(|row| {
            let mut obj = serde_json::Map::new();
            for (col, value) in row.cells() {
                obj.insert(
                    col.name.to_string(),
                    match value {
                        Some(v) => serde_json::Value::String(v),
                        None => serde_json::Value::Null,
                    },
                );
            }
            serde_json::Value::Object(obj)
        })
        .collect();

    let doc = serde_json::json!({
        "contract_version": CONTRACT_VERSION,
        "generated_at": generated_at,
        "range": { "from": filter.from.clone(), "to": filter.to.clone() },
        "scope": {
            "agent": filter.agent.clone(),
            "stream_domain": effective_domain(filter),
        },
        "currencies": data.currencies.clone(),
        "quality": data.quality.clone(),
        "coverage": data.coverage.clone(),
        "contains_cost": data.contains_cost,
        "records": records,
    });
    Ok(serde_json::to_string_pretty(&doc)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Database {
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

    fn insert_event(
        db: &Database,
        event_id: &str,
        project_ref: Option<&str>,
        model_raw: Option<&str>,
    ) {
        db.conn()
            .execute(
                "INSERT INTO usage_events (
                    event_id, schema_version, parse_run_id, source_id, agent, stream_domain,
                    session_id, occurred_at, observed_at, project_ref, model_raw,
                    granularity, request_count, input_fresh, input_cache_read, input_cache_write,
                    input_total, output_total, total, tokens_quality, completeness,
                    cost_estimated_amount, cost_estimated_currency
                 ) VALUES (
                    ?1, 1, 1, 'src1', 'kimi_code', 'local_log',
                    'sess', '2026-10-11T06:00:00Z', '2026-10-11T06:00:01Z', ?2, ?3,
                    'request', 1, 100, 40, 0, 140, 30, 170, 'reported', 'complete',
                    '0.42', 'USD'
                 )",
                rusqlite::params![event_id, project_ref, model_raw],
            )
            .unwrap();
    }

    #[test]
    fn csv_text_field_prefixes_formula_starters() {
        assert_eq!(csv_text_field("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(csv_text_field("+1"), "'+1");
        assert_eq!(csv_text_field("-cmd"), "'-cmd");
        assert_eq!(csv_text_field("@user"), "'@user");
        assert_eq!(csv_text_field("\tx"), "'\tx");
        assert_eq!(csv_text_field("\rx"), "\"'\rx\"");
        assert_eq!(csv_text_field("\nx"), "\"'\nx\"");
        // 普通文本不动
        assert_eq!(csv_text_field("hello"), "hello");
        // 含逗号/引号走标准 CSV 引用
        assert_eq!(csv_text_field("a,b"), "\"a,b\"");
        assert_eq!(csv_text_field("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn decimal_fields_are_not_prefixed() {
        // 负金额是合法数值，原样写出，不加注入前缀
        let rows = vec![ExportRow {
            event_id: "e1".into(),
            agent: "kimi_code".into(),
            source_id: "src1".into(),
            stream_domain: "local_log".into(),
            session_id: None,
            actor_id: None,
            granularity: "request".into(),
            occurred_at: Some("2026-10-11T06:00:00Z".into()),
            observed_at: "2026-10-11T06:00:01Z".into(),
            project_ref: None,
            model_raw: None,
            model_normalized: None,
            request_count: Some("1".into()),
            input_fresh: Some("100".into()),
            input_cache_read: None,
            input_cache_write: None,
            input_total: Some("100".into()),
            output_total: None,
            output_reasoning: None,
            total: Some("100".into()),
            tokens_quality: "reported".into(),
            completeness: "complete".into(),
            cost_estimated_amount: Some("-0.05".into()),
            cost_estimated_currency: Some("USD".into()),
            cost_estimated_price_version: None,
            account_link: None,
        }];
        let csv = render_csv(&rows);
        assert!(csv.contains("-0.05"), "负金额应原样: {csv}");
        assert!(!csv.contains("'-0.05"), "负金额不得加注入前缀: {csv}");
    }

    #[test]
    fn preview_reports_scope_fields_and_cost() {
        let db = setup_db();
        insert_event(
            &db,
            "e1",
            Some("/Users/alice/SecretProject"),
            Some("kimi-k2"),
        );
        let policy = ExportPathPolicy::new("/tmp/out.csv");
        let preview =
            export_statistics(&db, &UsageFilter::default(), ExportFormat::Csv, &policy).unwrap();
        assert_eq!(preview.record_count, 1);
        assert!(preview.contains_cost);
        assert_eq!(preview.currencies, vec!["USD".to_string()]);
        assert_eq!(preview.scope.stream_domain, "local_log");
        assert!(preview
            .fields
            .iter()
            .any(|f| f.name == "total" && f.unit == "tokens"));
        assert!(preview
            .note_keys
            .contains(&"export.redaction.project_paths_anonymized"));
    }

    #[test]
    fn project_path_is_anonymized_by_default_and_kept_on_request() {
        let db = setup_db();
        insert_event(
            &db,
            "e1",
            Some("/Users/alice/SecretProject"),
            Some("kimi-k2"),
        );

        // 默认：匿名标签，不含明文
        let out = std::env::temp_dir().join(format!("providock-exp-{}.csv", std::process::id()));
        let policy = ExportPathPolicy::new(&out);
        write_export(&db, &UsageFilter::default(), ExportFormat::Csv, &policy).unwrap();
        let csv = std::fs::read_to_string(&out).unwrap();
        assert!(!csv.contains("SecretProject"), "不得含项目明文: {csv}");
        assert!(!csv.contains("alice"), "不得含用户名: {csv}");
        assert!(csv.contains("<file:"), "应为匿名标签: {csv}");
        assert_eq!(csv.matches("<file:").count(), 1);

        // 显式保留
        let policy = ExportPathPolicy::new(&out).keep_project_paths(true);
        write_export(&db, &UsageFilter::default(), ExportFormat::Csv, &policy).unwrap();
        let csv = std::fs::read_to_string(&out).unwrap();
        assert!(csv.contains("/Users/alice/SecretProject"));
        std::fs::remove_file(&out).ok();
    }

    #[test]
    fn export_field_set_has_no_secret_or_body_columns() {
        // 结构断言：导出字段集合不含凭据/正文字段，也不含 project_ref 明文含义之外的秘密列
        for col in COLUMNS {
            let name = col.name;
            assert!(!name.contains("credential"), "{name}");
            assert!(!name.contains("vault"), "{name}");
            assert!(!name.contains("prompt_body"), "{name}");
            assert!(!name.contains("response_body"), "{name}");
        }
        // project_ref 列存在，但导出值是脱敏标签（见上一个测试）
        assert!(COLUMNS.iter().any(|c| c.name == "project_ref"));
    }

    #[test]
    fn json_has_contract_quality_currency_scope_coverage() {
        let db = setup_db();
        insert_event(
            &db,
            "e1",
            Some("/Users/alice/SecretProject"),
            Some("kimi-k2"),
        );
        let out = std::env::temp_dir().join(format!("providock-exp-{}.json", std::process::id()));
        let policy = ExportPathPolicy::new(&out);
        write_export(&db, &UsageFilter::default(), ExportFormat::Json, &policy).unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["contract_version"], CONTRACT_VERSION);
        assert!(doc["generated_at"].is_string());
        assert_eq!(doc["scope"]["stream_domain"], "local_log");
        assert_eq!(doc["currencies"][0], "USD");
        assert!(doc["quality"]["tokens_quality_counts"]["reported"].is_number());
        assert_eq!(doc["coverage"]["events_total"], 1);
        assert_eq!(doc["coverage"]["events_with_known_time"], 1);
        assert_eq!(doc["coverage"]["events_with_estimated_cost"], 1);
        // 记录里项目路径已脱敏
        assert!(!text.contains("SecretProject"));
        assert!(doc["records"][0]["project_ref"]
            .as_str()
            .unwrap()
            .starts_with("<file:"));
        std::fs::remove_file(&out).ok();
    }
}
