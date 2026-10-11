//! 会话列表与可选正文查看（T13）。
//!
//! 规范依据：
//! - docs/specs/UI_UX.md 第 6 节（列表只查索引元数据、详情抽屉、正文默认关闭、分页与工具块折叠、
//!   缺失原因分类）、第 8 节（字段未提供的显示口径）。
//! - docs/specs/SECURITY_PRIVACY.md 第 1 节（默认不存正文、按需读源文件、不整条 JSON 落诊断）、
//!   第 3 节（`session_ref` 由后端查索引解析、路径约束在已选根目录内、拒绝任意路径）、
//!   第 5 节（不可信正文：长度上限、分页、控制字符剔除）。
//! - docs/specs/DATA_CONTRACTS.md 第 4 节（物理观察与逻辑去重：会话身份取自索引）、
//!   第 9 节（Token 用 Rust 整数运算与十进制字符串，禁止 REAL）。
//!
//! 边界：
//! - 本模块**不写**任何正文数据；正文只在一次调用内存在于内存，随返回的页释放。
//! - 会话列表的 Token 小计在 Rust 侧用 u128 检查算术求和（不依赖 SQLite `SUM`，防溢出）。
//! - 源文件以**只读**方式打开，路径先经 [`crate::safepath::resolve_within_root`] 约束在源根目录内。
//! - 未实现正文读取器的源（gemini/opencode/pi/grok/mcode/claude_desktop）返回
//!   [`ContentError::UnsupportedSource`]，不伪造正文。

use crate::adapters::content::{
    claude_code_content, codex_content, kimi_content, ContentBlock, MessagePage,
};
use crate::safepath::{self, SafePathError};
use crate::storage::Database;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};

/// `settings` 表中的正文查看开关键；默认关闭（UI_UX 第 6 节）。
pub const CONTENT_VIEW_SETTING_KEY: &str = "content.view.enabled";
/// 会话列表默认页大小与上限。
pub const DEFAULT_SESSION_PAGE_LIMIT: usize = 50;
pub const MAX_SESSION_PAGE_LIMIT: usize = 200;
/// 正文页默认页大小与上限（与 `adapters::content::MAX_PAGE_LIMIT` 对齐）。
pub const DEFAULT_CONTENT_PAGE_LIMIT: usize = 50;
pub const MAX_CONTENT_PAGE_LIMIT: usize = 200;
/// 单个会话文件读取上限；超过即拒绝，避免超大/恶意文件拖垮查看。
pub const MAX_CONTENT_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// 会话身份/actor 标识的最大字符数（索引里的值不应更长）。
const REF_PART_MAX_CHARS: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum SessionQueryError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("invalid cursor")]
    InvalidCursor,
}

// ---------------------------------------------------------------------------
// 会话列表
// ---------------------------------------------------------------------------

/// 会话列表筛选。时间区间为半开 `[from, to)`，与用量页口径一致。
#[derive(Debug, Default, Clone)]
pub struct SessionFilter {
    pub agent: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    /// 只匹配索引元数据（session_id / 标题）；不扫正文。
    pub query: Option<String>,
}

/// 列表项。Token 小计为十进制字符串，`None` = 该会话没有已知总量（不显示 0）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionItem {
    pub session_key: String,
    pub source_id: String,
    pub agent: String,
    pub session_id: String,
    pub parent_session_key: Option<String>,
    /// 从 `parent_session_key` 剥离源前缀后的原生父会话 ID（可空）。
    pub parent_session_id: Option<String>,
    pub title: Option<String>,
    pub project_ref: Option<String>,
    pub archived: bool,
    pub has_parent: bool,
    pub has_subagents: bool,
    /// 最后活跃时间（事件 `occurred_at`/`observed_at` 与 `sessions.last_seen_at` 取较晚者）。
    pub last_active_at: Option<String>,
    pub event_count: u64,
    pub token_total: Option<String>,
    pub token_known_events: u64,
    /// 该会话出现过的模型名（去重、排序）；空表示源未提供。
    pub models: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsPage {
    pub items: Vec<SessionItem>,
    /// 键集游标 `(last_active, session_key)` 的不透明编码；`None` = 已到末尾。
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct SessionsCursor {
    last: String,
    key: String,
}

struct RawRow {
    session_key: String,
    source_id: String,
    agent: String,
    session_id: String,
    parent_session_key: Option<String>,
    title: Option<String>,
    project_ref: Option<String>,
    archived: bool,
    has_subagents: bool,
    last_active: String,
    event_count: u64,
    child_count: u64,
}

/// 会话索引分页查询。只读 `sessions` / `usage_events`，不读源文件。
pub fn sessions_page(
    db: &Database,
    filter: &SessionFilter,
    cursor: Option<&str>,
    limit: usize,
) -> Result<SessionsPage, SessionQueryError> {
    let limit = normalize_limit(limit, DEFAULT_SESSION_PAGE_LIMIT, MAX_SESSION_PAGE_LIMIT);
    let (cur_last, cur_key) = match cursor {
        Some(raw) => {
            let parsed: SessionsCursor =
                serde_json::from_str(raw).map_err(|_| SessionQueryError::InvalidCursor)?;
            (parsed.last, parsed.key)
        }
        None => (String::new(), String::new()),
    };

    // 活跃时间表达式可空：用 '' 兜底参与排序与游标比较（SQLite 中 NULL 最小，'' 次之）。
    let last_active = "COALESCE((SELECT MAX(COALESCE(e.occurred_at, e.observed_at)) FROM usage_events e \
                      WHERE e.source_id = s.source_id AND e.session_id = s.session_id), s.last_seen_at, '')";

    let mut sql = format!(
        "SELECT s.session_key, s.source_id, s.agent, s.session_id, s.parent_session_key,
                s.title, s.project_ref, s.archived, s.has_subagents,
                {last_active} AS last_active,
                (SELECT COUNT(*) FROM usage_events e
                  WHERE e.source_id = s.source_id AND e.session_id = s.session_id),
                (SELECT COUNT(*) FROM sessions c WHERE c.parent_session_key = s.session_key)
         FROM sessions s WHERE 1 = 1"
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(agent) = filter.agent.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(" AND s.agent = ?");
        params.push(Box::new(agent.to_string()));
    }
    if let Some(from) = filter.from.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(&format!(" AND {last_active} >= ?"));
        params.push(Box::new(from.to_string()));
    }
    if let Some(to) = filter.to.as_deref().filter(|s| !s.is_empty()) {
        sql.push_str(&format!(" AND {last_active} < ?"));
        params.push(Box::new(to.to_string()));
    }
    if let Some(q) = filter
        .query
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let pattern = format!("%{}%", escape_like(q));
        sql.push_str(" AND (s.session_id LIKE ? ESCAPE '\\' OR s.title LIKE ? ESCAPE '\\')");
        params.push(Box::new(pattern.clone()));
        params.push(Box::new(pattern));
    }
    if cursor.is_some() {
        sql.push_str(&format!(
            " AND ({last_active} < ? OR ({last_active} = ? AND s.session_key > ?))"
        ));
        params.push(Box::new(cur_last.clone()));
        params.push(Box::new(cur_last));
        params.push(Box::new(cur_key));
    }
    sql.push_str(" ORDER BY last_active DESC, s.session_key ASC LIMIT ?");
    params.push(Box::new(limit as i64 + 1));

    let mut raw: Vec<RawRow> = {
        let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| &**p).collect();
        let mut stmt = db.conn().prepare(&sql)?;
        let mut rows = stmt.query(refs.as_slice())?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(RawRow {
                session_key: row.get(0)?,
                source_id: row.get(1)?,
                agent: row.get(2)?,
                session_id: row.get(3)?,
                parent_session_key: row.get(4)?,
                title: row.get(5)?,
                project_ref: row.get(6)?,
                archived: row.get::<_, i64>(7)? != 0,
                has_subagents: row.get::<_, i64>(8)? != 0,
                last_active: row.get(9)?,
                event_count: row.get::<_, i64>(10)?.max(0) as u64,
                child_count: row.get::<_, i64>(11)?.max(0) as u64,
            });
        }
        out
    };

    let has_more = raw.len() > limit;
    raw.truncate(limit);
    let next_cursor = if has_more {
        raw.last().and_then(|r| {
            serde_json::to_string(&SessionsCursor {
                last: r.last_active.clone(),
                key: r.session_key.clone(),
            })
            .ok()
        })
    } else {
        None
    };

    let usage = session_usage(db, &raw)?;
    let items = raw
        .into_iter()
        .map(|r| {
            let summary = usage
                .get(&(r.source_id.clone(), r.session_id.clone()))
                .cloned()
                .unwrap_or_default();
            let parent_session_id = r.parent_session_key.as_ref().and_then(|key| {
                key.strip_prefix(&format!("{}:", r.source_id))
                    .map(str::to_string)
            });
            SessionItem {
                session_key: r.session_key,
                source_id: r.source_id,
                agent: r.agent,
                session_id: r.session_id,
                has_parent: r.parent_session_key.is_some(),
                parent_session_key: r.parent_session_key,
                parent_session_id,
                title: r.title,
                project_ref: r.project_ref,
                archived: r.archived,
                has_subagents: r.has_subagents || r.child_count > 0,
                last_active_at: (!r.last_active.is_empty()).then_some(r.last_active),
                event_count: r.event_count,
                token_total: summary.total,
                token_known_events: summary.known_events,
                models: summary.models,
            }
        })
        .collect();

    Ok(SessionsPage {
        items,
        next_cursor,
        has_more,
    })
}

/// 单个会话在本页聚合出的用量摘要（Token 小计、已知总量事件数、出现过的模型）。
#[derive(Debug, Default, Clone)]
struct SessionUsage {
    total: Option<String>,
    known_events: u64,
    models: Vec<String>,
}

/// 聚合累加器：`Σtotal`（u128 检查算术）、已知总量事件数、去重模型集合。
#[derive(Default)]
struct UsageAcc {
    total: Option<u128>,
    known_events: u64,
    models: BTreeSet<String>,
}

/// 按 `(source_id, session_id)` 聚合 Token 小计、已知事件数与模型名。
///
/// 用 Rust u128 检查算术（DATA_CONTRACTS 第 1/9 节），不做 SQLite `SUM`。
/// Token 小计为 `None` 表示该会话没有已知总量（不显示 0）。
fn session_usage(
    db: &Database,
    rows: &[RawRow],
) -> Result<HashMap<(String, String), SessionUsage>, SessionQueryError> {
    let mut acc: HashMap<(String, String), UsageAcc> = HashMap::new();
    if rows.is_empty() {
        return Ok(HashMap::new());
    }

    let clauses = vec!["(e.source_id = ? AND e.session_id = ?)"; rows.len()].join(" OR ");
    let sql = format!(
        "SELECT e.source_id, e.session_id, e.total, e.model_normalized FROM usage_events e
         WHERE e.session_id IS NOT NULL AND ({clauses})"
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    for r in rows {
        params.push(Box::new(r.source_id.clone()));
        params.push(Box::new(r.session_id.clone()));
    }

    {
        let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| &**p).collect();
        let mut stmt = db.conn().prepare(&sql)?;
        let mut result = stmt.query(refs.as_slice())?;
        while let Some(row) = result.next()? {
            let key: (String, String) = (row.get(0)?, row.get(1)?);
            let total: Option<i64> = row.get(2)?;
            let model: Option<String> = row.get(3)?;
            let entry = acc.entry(key).or_default();
            if let Some(total) = total {
                if let Ok(value) = u128::try_from(total) {
                    entry.total = Some(entry.total.unwrap_or(0) + value);
                }
                entry.known_events += 1;
            }
            // 模型名也可能敏感/超长：只保留有界的短标签。
            if let Some(model) = model.filter(|m| !m.is_empty() && m.chars().count() <= 128) {
                entry.models.insert(model);
            }
        }
    }

    Ok(acc
        .into_iter()
        .map(|(key, acc)| {
            (
                key,
                SessionUsage {
                    total: acc.total.map(|v| v.to_string()),
                    known_events: acc.known_events,
                    models: acc.models.into_iter().collect(),
                },
            )
        })
        .collect())
}

fn escape_like(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn normalize_limit(limit: usize, default: usize, max: usize) -> usize {
    if limit == 0 {
        default.min(max).max(1)
    } else {
        limit.min(max)
    }
}

// ---------------------------------------------------------------------------
// 正文查看开关（settings 表；正文不落任何表）
// ---------------------------------------------------------------------------

/// 读取正文查看开关；默认关闭（无记录 = 关）。
pub fn content_view_enabled(db: &Database) -> Result<bool, SessionQueryError> {
    let value: Option<String> = db
        .conn()
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [CONTENT_VIEW_SETTING_KEY],
            |row| row.get(0),
        )
        .optional()?;
    Ok(matches!(value.as_deref(), Some("true") | Some("1")))
}

/// 写入正文查看开关，返回写入后的值。
pub fn content_view_set(db: &Database, enabled: bool) -> Result<bool, SessionQueryError> {
    db.conn().execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![
            CONTENT_VIEW_SETTING_KEY,
            if enabled { "true" } else { "false" }
        ],
    )?;
    Ok(enabled)
}

// ---------------------------------------------------------------------------
// 正文页
// ---------------------------------------------------------------------------

/// 正文读取请求；`session_ref` 只含索引身份，不含任何系统路径。
#[derive(Debug, Clone)]
pub struct SessionRef {
    pub source_id: String,
    pub session_id: String,
    pub actor_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBlockDto {
    pub role: String,
    pub text: Option<String>,
    pub tool_name: Option<String>,
    pub is_collapsible_tool: bool,
    pub truncated: bool,
    pub time: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentPage {
    pub source_id: String,
    pub session_id: String,
    pub actor_id: Option<String>,
    pub agent: String,
    /// 源根目录内的相对定位（不含本机绝对路径）；供「来源定位」展示。
    pub located_path: Option<String>,
    pub blocks: Vec<ContentBlockDto>,
    /// 下一页字节游标（十进制字符串）；`None` = 已到末尾。
    pub next_cursor: Option<String>,
    pub has_more: bool,
    /// 只含偏移与短码，不含正文与原始 JSON。
    pub diagnostics: Vec<String>,
}

/// 正文查看的明确失败原因（UI_UX 第 6 节：源文件已删除 / 权限不足 / 版本不兼容 / 该源暂不支持）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentError {
    /// 正文查看开关关闭。
    ContentDisabled,
    /// `session_ref` 不在索引中。
    SessionNotFound,
    /// 源根目录或会话文件已不存在。
    SourceGone,
    /// 读取被系统拒绝。
    PermissionDenied,
    /// 文件存在但形状与读取器不匹配（版本/格式差异）。
    Incompatible(String),
    /// 该源的正文读取器尚未实现。
    UnsupportedSource(String),
    /// 身份或路径未通过根目录约束校验。
    PathRejected,
    /// 文件超过读取上限。
    TooLarge,
    /// 其他内部错误（只含短码级信息）。
    Internal(String),
}

impl ContentError {
    /// 稳定错误码（前端据此分类展示原因）。
    pub fn code(&self) -> &'static str {
        match self {
            ContentError::ContentDisabled => "ContentDisabled",
            ContentError::SessionNotFound => "SessionNotFound",
            ContentError::SourceGone => "SourceGone",
            ContentError::PermissionDenied => "PermissionDenied",
            ContentError::Incompatible(_) => "Incompatible",
            ContentError::UnsupportedSource(_) => "UnsupportedSource",
            ContentError::PathRejected => "PathRejected",
            ContentError::TooLarge => "TooLarge",
            ContentError::Internal(_) => "Internal",
        }
    }

    /// 面向用户的中文说明（不含本机路径与正文）。
    pub fn message(&self) -> String {
        match self {
            ContentError::ContentDisabled => "对话查看未开启，未读取任何源文件".into(),
            ContentError::SessionNotFound => "该会话不在本机索引中，拒绝读取".into(),
            ContentError::SourceGone => "源文件已删除或移动，统计保留但正文不可读".into(),
            ContentError::PermissionDenied => "权限不足，无法读取该源文件".into(),
            ContentError::Incompatible(detail) => format!("会话文件与读取器不兼容：{detail}"),
            ContentError::UnsupportedSource(agent) => {
                format!("{agent} 的正文读取器尚未实现，未读取也未伪造正文")
            }
            ContentError::PathRejected => "路径未通过根目录约束校验，已拒绝".into(),
            ContentError::TooLarge => format!(
                "会话文件超过 {} MiB 上限，未读取",
                MAX_CONTENT_FILE_BYTES / (1024 * 1024)
            ),
            ContentError::Internal(detail) => format!("读取失败：{detail}"),
        }
    }
}

/// 读取一页会话正文。
///
/// 流程：开关校验 → `session_ref` 查索引 → 源根目录 → 按源规则定位文件 →
/// [`safepath::resolve_within_root`] 约束 → 只读打开 → 对应读取器分页。
pub fn session_content_page(
    db: &Database,
    session_ref: &SessionRef,
    cursor: Option<u64>,
    limit: usize,
) -> Result<ContentPage, ContentError> {
    if !content_view_enabled(db).map_err(|e| ContentError::Internal(e.to_string()))? {
        return Err(ContentError::ContentDisabled);
    }
    let limit = normalize_limit(limit, DEFAULT_CONTENT_PAGE_LIMIT, MAX_CONTENT_PAGE_LIMIT);

    validate_session_id(&session_ref.session_id)?;
    if let Some(actor) = session_ref.actor_id.as_deref() {
        validate_actor_id(actor)?;
    }

    // 1) session_ref 必须由后端索引解析（SECURITY_PRIVACY 第 3 节）。
    let agent: Option<String> = db
        .conn()
        .query_row(
            "SELECT agent FROM sessions WHERE source_id = ?1 AND session_id = ?2",
            rusqlite::params![session_ref.source_id, session_ref.session_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| ContentError::Internal(e.to_string()))?;
    let Some(agent) = agent else {
        return Err(ContentError::SessionNotFound);
    };

    // 2) 正文读取器未实现的源：明确不支持，不猜文件位置。
    if !supports_content(&agent) {
        return Err(ContentError::UnsupportedSource(agent));
    }

    // 3) 源根目录（用户/发现规则选择的根，不是前端传入的路径）。
    let root: Option<String> = db
        .conn()
        .query_row(
            "SELECT root_path FROM sources WHERE source_id = ?1",
            [&session_ref.source_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| ContentError::Internal(e.to_string()))?;
    let root = PathBuf::from(root.ok_or(ContentError::SourceGone)?);
    if !root.is_dir() {
        return Err(ContentError::SourceGone);
    }

    // 4) 按源目录规则定位（相对路径），再约束在根内。
    let relative = locate(
        &agent,
        &root,
        &session_ref.session_id,
        session_ref.actor_id.as_deref(),
    )?;
    let resolved = match safepath::resolve_within_root(&root, &relative) {
        Ok(path) => path,
        Err(SafePathError::RootUnavailable) => return Err(ContentError::SourceGone),
        Err(SafePathError::EscapesRoot)
        | Err(SafePathError::OutsideRoot)
        | Err(SafePathError::InvalidPath) => return Err(ContentError::PathRejected),
        Err(SafePathError::Io(e)) => return Err(ContentError::Internal(e.to_string())),
    };
    if !resolved.is_file() {
        return Err(ContentError::SourceGone);
    }

    let meta = std::fs::metadata(&resolved).map_err(map_io)?;
    if meta.len() > MAX_CONTENT_FILE_BYTES {
        return Err(ContentError::TooLarge);
    }
    let bytes = read_file(&resolved)?;

    let page = match agent.as_str() {
        "kimi_code" => kimi_content::list_messages(&bytes, cursor, limit),
        "claude_code" => claude_code_content::list_messages(&bytes, cursor, limit),
        "codex" => codex_content::list_messages(&bytes, cursor, limit),
        other => return Err(ContentError::UnsupportedSource(other.to_string())),
    };
    if cursor.is_none() && is_incompatible(&agent, &page, &bytes) {
        return Err(ContentError::Incompatible(
            "文件形状与读取器预期不符".into(),
        ));
    }

    Ok(ContentPage {
        source_id: session_ref.source_id.clone(),
        session_id: session_ref.session_id.clone(),
        actor_id: session_ref.actor_id.clone(),
        agent,
        located_path: Some(path_label(&relative)),
        blocks: page.blocks.into_iter().map(block_dto).collect(),
        next_cursor: page.next_cursor.map(|c| c.to_string()),
        has_more: page.has_more,
        diagnostics: page.diagnostics,
    })
}

/// 支持的正文源（读取器已实现）。其余源明确返回「暂不支持」。
pub fn supports_content(agent: &str) -> bool {
    matches!(agent, "kimi_code" | "claude_code" | "codex")
}

fn block_dto(block: ContentBlock) -> ContentBlockDto {
    ContentBlockDto {
        role: block.role.label(),
        text: block.text,
        tool_name: block.tool_name,
        is_collapsible_tool: block.is_collapsible_tool,
        truncated: block.truncated,
        time: block.time,
    }
}

/// 会话 ID 校验：非空、长度有界、不含路径分隔符（会话 ID 本身不含分隔符）。
fn validate_session_id(value: &str) -> Result<(), ContentError> {
    if value.is_empty()
        || value.chars().count() > REF_PART_MAX_CHARS
        || value.contains('/')
        || value.contains('\\')
        || value == "."
        || value == ".."
    {
        return Err(ContentError::PathRejected);
    }
    Ok(())
}

/// actor 标识校验：允许 `/`（Claude 子 Agent 的 actor 是相对会话目录的路径段），
/// 但拒绝绝对路径、反斜杠与任何 `.` / `..` / 空段，防止定位注入。
fn validate_actor_id(value: &str) -> Result<(), ContentError> {
    if value.is_empty()
        || value.chars().count() > REF_PART_MAX_CHARS
        || value.contains('\\')
        || value.starts_with('/')
    {
        return Err(ContentError::PathRejected);
    }
    if value
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(ContentError::PathRejected);
    }
    Ok(())
}

fn map_io(err: std::io::Error) -> ContentError {
    match err.kind() {
        std::io::ErrorKind::PermissionDenied => ContentError::PermissionDenied,
        std::io::ErrorKind::NotFound => ContentError::SourceGone,
        _ => ContentError::Internal(err.to_string()),
    }
}

/// 以只读方式打开并读取文件（SECURITY_PRIVACY 第 3 节：源文件只读）。
fn read_file(path: &Path) -> Result<Vec<u8>, ContentError> {
    let mut file = std::fs::File::open(path).map_err(map_io)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(map_io)?;
    Ok(bytes)
}

fn path_label(relative: &Path) -> String {
    relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// 文件整体不像该源的会话形状（全部行都解析失败，或 Kimi 首行不是 metadata）。
fn is_incompatible(agent: &str, page: &MessagePage, bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    if agent == "kimi_code"
        && page
            .diagnostics
            .iter()
            .any(|d| d.starts_with("not_kimi_wire"))
    {
        return true;
    }
    let malformed = page
        .diagnostics
        .iter()
        .filter(|d| d.starts_with("malformed_line_at_offset:"))
        .count();
    let lines = bytes
        .split(|b| *b == b'\n')
        .filter(|line| line.iter().any(|b| !b.is_ascii_whitespace()))
        .count();
    malformed > 0 && malformed >= lines
}

// ---------------------------------------------------------------------------
// 源目录规则定位（只枚举目录，不读正文）
// ---------------------------------------------------------------------------

fn locate(
    agent: &str,
    root: &Path,
    session_id: &str,
    actor_id: Option<&str>,
) -> Result<PathBuf, ContentError> {
    match agent {
        "kimi_code" => locate_kimi(root, session_id, actor_id),
        "claude_code" => locate_claude(root, session_id, actor_id),
        "codex" => locate_codex(root, session_id),
        other => Err(ContentError::UnsupportedSource(other.to_string())),
    }
}

/// Kimi：`sessions/<wdKey>/<sessionId>/agents/<actor>/wire.jsonl`。
fn locate_kimi(
    root: &Path,
    session_id: &str,
    actor_id: Option<&str>,
) -> Result<PathBuf, ContentError> {
    let sessions = root.join("sessions");
    let mut wd_keys = dir_names(&sessions);
    wd_keys.sort();
    for wd in wd_keys {
        let agents_dir = sessions.join(&wd).join(session_id).join("agents");
        if !agents_dir.is_dir() {
            continue;
        }
        let actor = match actor_id {
            Some(actor) => actor.to_string(),
            None => {
                if agents_dir.join("main").is_dir() {
                    "main".to_string()
                } else {
                    let mut names = dir_names(&agents_dir);
                    names.sort();
                    names.into_iter().next().ok_or(ContentError::SourceGone)?
                }
            }
        };
        let rel = Path::new("sessions")
            .join(&wd)
            .join(session_id)
            .join("agents")
            .join(&actor)
            .join("wire.jsonl");
        if root.join(&rel).is_file() {
            return Ok(rel);
        }
    }
    Err(ContentError::SourceGone)
}

/// Claude Code：主会话 `projects/<project>/<sessionId>.jsonl`；子 Agent 在
/// `<sessionId>/subagents/**` 下，actor 为相对会话目录的路径段（由扫描器给出）。
fn locate_claude(
    root: &Path,
    session_id: &str,
    actor_id: Option<&str>,
) -> Result<PathBuf, ContentError> {
    let projects = root.join("projects");
    let mut project_dirs = dir_names(&projects);
    project_dirs.sort();
    if project_dirs.is_empty() {
        return Err(ContentError::SourceGone);
    }

    let wants_main = actor_id.is_none() || actor_id == Some("main");
    if wants_main {
        for project in &project_dirs {
            let rel = Path::new("projects")
                .join(project)
                .join(format!("{session_id}.jsonl"));
            if root.join(&rel).is_file() {
                return Ok(rel);
            }
        }
    }

    for project in &project_dirs {
        let base = Path::new("projects").join(project).join(session_id);
        if !root.join(&base).is_dir() {
            continue;
        }
        if let Some(actor) = actor_id {
            let direct = base.join(format!("{actor}.jsonl"));
            let nested = base.join("subagents").join(format!("{actor}.jsonl"));
            for candidate in [direct, nested] {
                if root.join(&candidate).is_file() {
                    return Ok(candidate);
                }
            }
        }
        // 主文件缺失或无 actor 指定：取排序后的第一个子 Agent 文件。
        let mut files = Vec::new();
        collect_jsonl_rel(
            &root.join(&base).join("subagents"),
            &base.join("subagents"),
            3,
            &mut files,
        );
        files.sort();
        if let Some(file) = files.into_iter().next() {
            return Ok(file);
        }
    }
    Err(ContentError::SourceGone)
}

/// Codex：`sessions/**` 与 `archived_sessions/*` 下文件名含 thread uuid 的 rollout。
fn locate_codex(root: &Path, session_id: &str) -> Result<PathBuf, ContentError> {
    let mut plain: Vec<PathBuf> = Vec::new();
    let mut compressed: Vec<PathBuf> = Vec::new();
    for (base, depth) in [
        (Path::new("sessions"), 5usize),
        (Path::new("archived_sessions"), 2),
    ] {
        let mut found = Vec::new();
        collect_rollout_rel(&root.join(base), base, depth, session_id, &mut found);
        for file in found {
            if file.to_string_lossy().ends_with(".jsonl") {
                plain.push(file);
            } else {
                compressed.push(file);
            }
        }
    }
    plain.sort();
    if let Some(file) = plain.into_iter().next() {
        return Ok(file);
    }
    compressed.sort();
    if !compressed.is_empty() {
        return Err(ContentError::Incompatible(
            "Codex 压缩归档 (.jsonl.zst) 暂不支持正文查看".into(),
        ));
    }
    Err(ContentError::SourceGone)
}

fn dir_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

/// 递归收集 `<base>` 相对根下的 `.jsonl` 文件（深度上限，跳过符号链接）。
fn collect_jsonl_rel(dir: &Path, prefix: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<_> = entries.flatten().collect();
    children.sort_by_key(|e| e.file_name());
    for entry in children {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let rel = prefix.join(name);
        if file_type.is_dir() {
            collect_jsonl_rel(&entry.path(), &rel, depth - 1, out);
        } else if file_type.is_file() && name.ends_with(".jsonl") {
            out.push(rel);
        }
    }
}

/// 递归收集文件名含 `session_id` 的 rollout（`.jsonl` / `.jsonl.zst`）。
fn collect_rollout_rel(
    dir: &Path,
    prefix: &Path,
    depth: usize,
    session_id: &str,
    out: &mut Vec<PathBuf>,
) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<_> = entries.flatten().collect();
    children.sort_by_key(|e| e.file_name());
    for entry in children {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let rel = prefix.join(name);
        if file_type.is_dir() {
            collect_rollout_rel(&entry.path(), &rel, depth - 1, session_id, out);
        } else if name.starts_with("rollout-")
            && name.contains(session_id)
            && (name.ends_with(".jsonl") || name.ends_with(".jsonl.zst"))
        {
            out.push(rel);
        }
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 自建临时目录（不引入 tempfile）：与 safepath 测试同风格。
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(tag: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let name = format!(
                "providock-sessions-{tag}-{}-{nanos}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let path = std::env::temp_dir().join(name);
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn write(&self, rel: &str, content: &str) -> PathBuf {
            let full = self.path.join(rel);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(&full, content).unwrap();
            full
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn insert_source(db: &Database, source_id: &str, agent: &str, root: &Path) {
        db.conn()
            .execute(
                "INSERT INTO sources (source_id, agent, stream_domain, root_path, parser_version)
                 VALUES (?1, ?2, 'local_log', ?3, 'test')",
                rusqlite::params![source_id, agent, root.to_string_lossy()],
            )
            .unwrap();
    }

    fn insert_session(
        db: &Database,
        source_id: &str,
        agent: &str,
        session_id: &str,
        parent: Option<&str>,
        last_seen: Option<&str>,
    ) {
        db.conn()
            .execute(
                "INSERT INTO sessions (session_key, source_id, agent, session_id, parent_session_key, last_seen_at, has_subagents)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
                rusqlite::params![
                    format!("{source_id}:{session_id}"),
                    source_id,
                    agent,
                    session_id,
                    parent,
                    last_seen
                ],
            )
            .unwrap();
    }

    fn insert_run(db: &Database, source_id: &str) -> i64 {
        db.conn()
            .execute(
                "INSERT INTO parse_runs (source_id, parser_version, status, is_active)
                 VALUES (?1, 'test', 'committed', 1)",
                [source_id],
            )
            .unwrap();
        db.conn().last_insert_rowid()
    }

    fn insert_event(
        db: &Database,
        run_id: i64,
        source_id: &str,
        session_id: &str,
        occurred_at: &str,
        total: Option<i64>,
    ) {
        insert_event_with_model(db, run_id, source_id, session_id, occurred_at, total, None);
    }

    fn insert_event_with_model(
        db: &Database,
        run_id: i64,
        source_id: &str,
        session_id: &str,
        occurred_at: &str,
        total: Option<i64>,
        model: Option<&str>,
    ) {
        db.conn()
            .execute(
                "INSERT INTO usage_events (event_id, schema_version, parse_run_id, source_id, agent,
                    stream_domain, session_id, observed_at, granularity, total, model_normalized,
                    tokens_quality, completeness)
                 VALUES (?1, 1, ?2, ?3, 'kimi_code', 'local_log', ?4, ?5, 'request', ?6, ?7, 'reported', 'complete')",
                rusqlite::params![
                    format!("evt:{source_id}:{session_id}:{occurred_at}"),
                    run_id,
                    source_id,
                    session_id,
                    occurred_at,
                    total,
                    model
                ],
            )
            .unwrap();
    }

    const WIRE: &str = concat!(
        r#"{"type":"metadata","protocol_version":"1.5"}"#,
        "\n",
        r#"{"type":"turn.prompt","agentId":"main","turnId":"t1","prompt":"Run the suite","time":1760000003000}"#,
        "\n",
        r#"{"type":"context.append_message","agentId":"main","role":"assistant","content":"Done.","time":1760000004000}"#,
        "\n"
    );

    fn kimi_db(dir: &TempDir) -> Database {
        let db = Database::open_in_memory().unwrap();
        insert_source(&db, "kimi_code:default", "kimi_code", dir.path());
        insert_session(&db, "kimi_code:default", "kimi_code", "sess-1", None, None);
        db
    }

    #[test]
    fn sessions_page_sums_tokens_and_pages_by_keyset() {
        let dir = TempDir::new("list");
        let db = Database::open_in_memory().unwrap();
        insert_source(&db, "kimi_code:default", "kimi_code", dir.path());
        insert_session(&db, "kimi_code:default", "kimi_code", "b", None, None);
        insert_session(&db, "kimi_code:default", "kimi_code", "a", None, None);
        insert_session(&db, "kimi_code:default", "kimi_code", "c", None, None);
        let run = insert_run(&db, "kimi_code:default");
        // a 最活跃、b 次之、c 无事件（时间未知 → 排最后）
        insert_event_with_model(
            &db,
            run,
            "kimi_code:default",
            "a",
            "2026-10-10T10:00:00Z",
            Some(i64::MAX),
            Some("kimi-k2"),
        );
        insert_event_with_model(
            &db,
            run,
            "kimi_code:default",
            "a",
            "2026-10-10T11:00:00Z",
            Some(1),
            Some("kimi-k2-turbo"),
        );
        insert_event(
            &db,
            run,
            "kimi_code:default",
            "b",
            "2026-10-09T10:00:00Z",
            None,
        );
        insert_event(
            &db,
            run,
            "kimi_code:default",
            "b",
            "2026-10-09T11:00:00Z",
            Some(5),
        );

        let page = sessions_page(&db, &SessionFilter::default(), None, 2).unwrap();
        assert_eq!(page.items.len(), 2);
        assert!(page.has_more);
        let keys: Vec<&str> = page.items.iter().map(|i| i.session_id.as_str()).collect();
        assert_eq!(keys, vec!["a", "b"]);
        // u128 求和：i64::MAX + 1 不溢出，且是十进制字符串
        let a = &page.items[0];
        assert_eq!(a.token_total.as_deref(), Some("9223372036854775808"));
        assert_eq!(a.token_known_events, 2);
        assert_eq!(a.event_count, 2);
        assert_eq!(a.last_active_at.as_deref(), Some("2026-10-10T11:00:00Z"));
        assert_eq!(a.models, vec!["kimi-k2", "kimi-k2-turbo"]);
        // b 只有一条已知总量：小计 5，已知 1 / 事件 2
        let b = &page.items[1];
        assert_eq!(b.token_total.as_deref(), Some("5"));
        assert_eq!(b.token_known_events, 1);
        assert_eq!(b.event_count, 2);

        let page2 = sessions_page(
            &db,
            &SessionFilter::default(),
            page.next_cursor.as_deref(),
            2,
        )
        .unwrap();
        assert_eq!(page2.items.len(), 1);
        assert!(!page2.has_more);
        assert_eq!(page2.items[0].session_id, "c");
        assert_eq!(page2.items[0].token_total, None);
        assert_eq!(page2.items[0].last_active_at, None);

        // 无效游标被拒绝，不做越权读取
        assert!(matches!(
            sessions_page(&db, &SessionFilter::default(), Some("not-json"), 2),
            Err(SessionQueryError::InvalidCursor)
        ));
    }

    #[test]
    fn sessions_page_filters_by_agent_time_and_metadata_query() {
        let dir = TempDir::new("filter");
        let db = Database::open_in_memory().unwrap();
        insert_source(&db, "kimi_code:default", "kimi_code", dir.path());
        insert_source(&db, "codex:default", "codex", dir.path());
        insert_session(&db, "kimi_code:default", "kimi_code", "alpha", None, None);
        insert_session(&db, "codex:default", "codex", "beta", None, None);
        let run = insert_run(&db, "kimi_code:default");
        insert_event(
            &db,
            run,
            "kimi_code:default",
            "alpha",
            "2026-10-01T00:00:00Z",
            Some(1),
        );
        let run2 = insert_run(&db, "codex:default");
        insert_event(
            &db,
            run2,
            "codex:default",
            "beta",
            "2026-10-05T00:00:00Z",
            Some(1),
        );

        let all = sessions_page(&db, &SessionFilter::default(), None, 10).unwrap();
        assert_eq!(all.items.len(), 2);

        let agent = sessions_page(
            &db,
            &SessionFilter {
                agent: Some("codex".into()),
                ..Default::default()
            },
            None,
            10,
        )
        .unwrap();
        assert_eq!(agent.items.len(), 1);
        assert_eq!(agent.items[0].session_id, "beta");

        let range = sessions_page(
            &db,
            &SessionFilter {
                from: Some("2026-10-04T00:00:00Z".into()),
                to: Some("2026-10-06T00:00:00Z".into()),
                ..Default::default()
            },
            None,
            10,
        )
        .unwrap();
        assert_eq!(range.items.len(), 1);
        assert_eq!(range.items[0].session_id, "beta");

        // 搜索命中元数据（session_id）；未命中标题为 NULL 的会话
        let by_id = sessions_page(
            &db,
            &SessionFilter {
                query: Some("alph".into()),
                ..Default::default()
            },
            None,
            10,
        )
        .unwrap();
        assert_eq!(by_id.items.len(), 1);
        assert_eq!(by_id.items[0].session_id, "alpha");
        assert_eq!(by_id.items[0].title, None);

        // LIKE 通配符被转义，不当作模式
        let literal = sessions_page(
            &db,
            &SessionFilter {
                query: Some("%".into()),
                ..Default::default()
            },
            None,
            10,
        )
        .unwrap();
        assert!(literal.items.is_empty());
    }

    #[test]
    fn sessions_page_pages_identically_for_equal_timestamps() {
        let dir = TempDir::new("tie");
        let db = Database::open_in_memory().unwrap();
        insert_source(&db, "kimi_code:default", "kimi_code", dir.path());
        for id in ["s1", "s2", "s3"] {
            insert_session(&db, "kimi_code:default", "kimi_code", id, None, None);
        }
        let run = insert_run(&db, "kimi_code:default");
        for id in ["s1", "s2", "s3"] {
            insert_event(
                &db,
                run,
                "kimi_code:default",
                id,
                "2026-10-07T00:00:00Z",
                Some(1),
            );
        }
        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            let page = sessions_page(&db, &SessionFilter::default(), cursor.as_deref(), 2).unwrap();
            seen.extend(page.items.iter().map(|i| i.session_id.clone()));
            if !page.has_more {
                break;
            }
            cursor = page.next_cursor;
        }
        assert_eq!(seen, vec!["s1", "s2", "s3"]);
    }

    #[test]
    fn content_view_toggle_defaults_off_and_persists() {
        let db = Database::open_in_memory().unwrap();
        assert!(!content_view_enabled(&db).unwrap());
        assert!(content_view_set(&db, true).unwrap());
        assert!(content_view_enabled(&db).unwrap());
        assert!(!content_view_set(&db, false).unwrap());
        assert!(!content_view_enabled(&db).unwrap());
    }

    #[test]
    fn content_requires_enabled_toggle() {
        let dir = TempDir::new("disabled");
        dir.write("sessions/wd1/sess-1/agents/main/wire.jsonl", WIRE);
        let db = kimi_db(&dir);
        let session_ref = SessionRef {
            source_id: "kimi_code:default".into(),
            session_id: "sess-1".into(),
            actor_id: None,
        };
        assert_eq!(
            session_content_page(&db, &session_ref, None, 10).unwrap_err(),
            ContentError::ContentDisabled
        );
    }

    #[test]
    fn content_reads_kimi_wire_and_pages() {
        let dir = TempDir::new("ok");
        dir.write("sessions/wd1/sess-1/agents/main/wire.jsonl", WIRE);
        let db = kimi_db(&dir);
        content_view_set(&db, true).unwrap();
        let session_ref = SessionRef {
            source_id: "kimi_code:default".into(),
            session_id: "sess-1".into(),
            actor_id: None,
        };
        let page = session_content_page(&db, &session_ref, None, 1).unwrap();
        assert_eq!(page.agent, "kimi_code");
        assert_eq!(page.blocks.len(), 1);
        assert_eq!(page.blocks[0].role, "user");
        assert!(page.has_more);
        assert_eq!(
            page.located_path.as_deref(),
            Some("sessions/wd1/sess-1/agents/main/wire.jsonl")
        );

        let next: u64 = page.next_cursor.as_deref().unwrap().parse().unwrap();
        let page2 = session_content_page(&db, &session_ref, Some(next), 1).unwrap();
        assert_eq!(page2.blocks.len(), 1);
        assert_eq!(page2.blocks[0].role, "assistant");
        assert!(!page2.has_more);
        assert_eq!(page2.next_cursor, None);
    }

    #[test]
    fn content_rejects_unknown_session_ref() {
        let dir = TempDir::new("unknown");
        let db = kimi_db(&dir);
        content_view_set(&db, true).unwrap();
        assert_eq!(
            session_content_page(
                &db,
                &SessionRef {
                    source_id: "kimi_code:default".into(),
                    session_id: "not-indexed".into(),
                    actor_id: None,
                },
                None,
                10,
            )
            .unwrap_err(),
            ContentError::SessionNotFound
        );
    }

    #[test]
    fn content_rejects_path_separators_in_ref() {
        let dir = TempDir::new("evil-ref");
        let db = kimi_db(&dir);
        content_view_set(&db, true).unwrap();
        for evil in ["../outside", "a/b", "..", "evil\\path"] {
            let err = session_content_page(
                &db,
                &SessionRef {
                    source_id: "kimi_code:default".into(),
                    session_id: evil.into(),
                    actor_id: None,
                },
                None,
                10,
            )
            .unwrap_err();
            assert_eq!(err, ContentError::PathRejected, "session_id={evil}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn content_rejects_symlink_escape_from_source_root() {
        let root = TempDir::new("symroot");
        let outside = TempDir::new("symout");
        outside.write("sess-1/agents/main/wire.jsonl", WIRE);
        std::fs::create_dir_all(root.path().join("sessions")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("sessions").join("wd1"))
            .unwrap();

        let db = kimi_db(&root);
        content_view_set(&db, true).unwrap();
        let err = session_content_page(
            &db,
            &SessionRef {
                source_id: "kimi_code:default".into(),
                session_id: "sess-1".into(),
                actor_id: None,
            },
            None,
            10,
        )
        .unwrap_err();
        assert_eq!(err, ContentError::PathRejected);
    }

    #[test]
    fn content_reports_source_gone_and_unsupported_source() {
        let dir = TempDir::new("gone");
        let db = kimi_db(&dir);
        content_view_set(&db, true).unwrap();
        // 会话在索引中，但源树里没有对应文件
        assert_eq!(
            session_content_page(
                &db,
                &SessionRef {
                    source_id: "kimi_code:default".into(),
                    session_id: "sess-1".into(),
                    actor_id: None,
                },
                None,
                10,
            )
            .unwrap_err(),
            ContentError::SourceGone
        );

        // 未实现读取器的源：明确 UnsupportedSource，不猜路径
        insert_source(&db, "pi:default", "pi", dir.path());
        insert_session(&db, "pi:default", "pi", "p1", None, None);
        let err = session_content_page(
            &db,
            &SessionRef {
                source_id: "pi:default".into(),
                session_id: "p1".into(),
                actor_id: None,
            },
            None,
            10,
        )
        .unwrap_err();
        assert_eq!(err.code(), "UnsupportedSource");
        assert!(err.message().contains("pi"));
    }

    #[test]
    fn content_reports_incompatible_shape() {
        let dir = TempDir::new("incompat");
        dir.write(
            "sessions/wd1/sess-1/agents/main/wire.jsonl",
            "{\"type\":\"llm.request\",\"agentId\":\"main\"}\n",
        );
        let db = kimi_db(&dir);
        content_view_set(&db, true).unwrap();
        let err = session_content_page(
            &db,
            &SessionRef {
                source_id: "kimi_code:default".into(),
                session_id: "sess-1".into(),
                actor_id: None,
            },
            None,
            10,
        )
        .unwrap_err();
        assert_eq!(err.code(), "Incompatible");
    }

    #[test]
    fn content_reads_codex_rollout_by_thread_id() {
        let dir = TempDir::new("codex");
        let sid = "11111111-2222-3333-4444-555555555555";
        dir.write(
            &format!("sessions/2026/10/11/rollout-2026-10-11T01-00-00-000Z-{sid}.jsonl"),
            concat!(
                r#"{"timestamp":"2026-10-11T01:00:00Z","type":"event_msg","payload":{"type":"user_message","message":"hi"}}"#,
                "\n"
            ),
        );
        let db = Database::open_in_memory().unwrap();
        insert_source(&db, "codex:default", "codex", dir.path());
        insert_session(&db, "codex:default", "codex", sid, None, None);
        content_view_set(&db, true).unwrap();
        let page = session_content_page(
            &db,
            &SessionRef {
                source_id: "codex:default".into(),
                session_id: sid.into(),
                actor_id: None,
            },
            None,
            10,
        )
        .unwrap();
        assert_eq!(page.blocks.len(), 1);
        assert_eq!(page.blocks[0].role, "user");
    }

    #[test]
    fn content_reads_claude_main_and_subagent() {
        let dir = TempDir::new("claude");
        dir.write(
            "projects/demo/sess-9.jsonl",
            concat!(
                r#"{"type":"user","timestamp":"2026-10-11T01:00:00Z","message":{"role":"user","content":"main"}}"#,
                "\n"
            ),
        );
        dir.write(
            "projects/demo/sess-9/subagents/agent-x.jsonl",
            concat!(
                r#"{"type":"assistant","timestamp":"2026-10-11T01:00:01Z","message":{"role":"assistant","content":"sub"}}"#,
                "\n"
            ),
        );
        let db = Database::open_in_memory().unwrap();
        insert_source(&db, "claude_code:default", "claude_code", dir.path());
        insert_session(
            &db,
            "claude_code:default",
            "claude_code",
            "sess-9",
            None,
            None,
        );
        content_view_set(&db, true).unwrap();

        let main = session_content_page(
            &db,
            &SessionRef {
                source_id: "claude_code:default".into(),
                session_id: "sess-9".into(),
                actor_id: None,
            },
            None,
            10,
        )
        .unwrap();
        assert_eq!(main.blocks.len(), 1);
        assert_eq!(main.blocks[0].text.as_deref(), Some("main"));

        let sub = session_content_page(
            &db,
            &SessionRef {
                source_id: "claude_code:default".into(),
                session_id: "sess-9".into(),
                actor_id: Some("subagents/agent-x".into()),
            },
            None,
            10,
        )
        .unwrap();
        assert_eq!(sub.blocks.len(), 1);
        assert_eq!(sub.blocks[0].text.as_deref(), Some("sub"));
    }

    #[test]
    fn error_codes_are_stable() {
        assert_eq!(ContentError::ContentDisabled.code(), "ContentDisabled");
        assert_eq!(ContentError::SourceGone.code(), "SourceGone");
        assert_eq!(ContentError::PermissionDenied.code(), "PermissionDenied");
        assert_eq!(
            ContentError::Incompatible("x".into()).code(),
            "Incompatible"
        );
        assert_eq!(
            ContentError::UnsupportedSource("pi".into()).code(),
            "UnsupportedSource"
        );
    }
}
