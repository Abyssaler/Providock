//! 源发现与增量扫描（A01–A08；本轮接入 A01/A03/A04/A05/A06/A07，A02/A08 既有）。
//!
//! 路径规则（AGENT_ADAPTERS 第 2 节 + 证据文档）：
//! - Kimi：`KIMI_CODE_HOME` > `~/.kimi-code`；会话在 `sessions/<wdKey>/<sessionId>/`，
//!   wire 在 `agents/<agentId>/wire.jsonl`；`state.json` 的 forkedFrom 判定继承。
//! - Codex：`CODEX_HOME` > `~/.codex`；`sessions/YYYY/MM/DD/*.jsonl[.zst]` 与
//!   `archived_sessions/*.jsonl[.zst]`；同名 `.zst` 与 `.jsonl` 并存时只读普通形态。
//! - Claude Code：`CLAUDE_CONFIG_DIR` > `~/.claude`；主会话
//!   `projects/<project>/*.jsonl`，子 Agent `<session>/subagents/*.jsonl` 与固定一层
//!   `<session>/subagents/workflows/wf_*/*.jsonl`（不无限递归）；子 Agent 的 actor
//!   取自文件相对会话目录的路径段。
//! - Gemini CLI：`~/.gemini`（证据未发现环境变量覆盖）；`tmp/<hash>/chats/session-*`，
//!   同名 `.json` 与 `.jsonl` 并存只读 `.jsonl`。
//! - Grok Build：`~/.grok/{sessions,archived_sessions}` 下 `**/updates.jsonl`
//!   （深度 ≤16，跳过符号链接，单文件 >50 MiB 跳过并记诊断）。
//! - OpenCode：`OPENCODE_DB`（绝对原样 / 相对拼数据目录）> `XDG_DATA_HOME/opencode`
//!   > `~/.local/share/opencode`；库为 `opencode.db`（`root` 即解析后的库文件路径）。
//! - Pi：`PI_CODING_AGENT_DIR` > `~/.pi/agent`；会话根 `PI_CODING_AGENT_SESSION_DIR`
//!   > `settings.json:sessionDir` > `<agent>/sessions`（`root` 即会话根）。
//! - MCode：`MINIMAX_DATA_DIR` > `MAVIS_DATA_DIR` > `~/.minimax`；
//!   库 `<root>/v2/sqlite/runtime-state.sqlite`，source_id 用规范化库路径哈希
//!   （CLI/TUI/桌面同库只采一次）。
//!
//! 游标与代：
//! - cursor_value = 已消费字节数，tail_checksum = "{size}:{mtime_secs}"，一致则跳过。
//! - 文件变小（size < 已消费字节）视为截断/重写：代 +1 全量重扫，
//!   旧代数事件同事务退休（墓碑保留身份，防止三度计入）。事件身份含代数的适配器
//!   （Kimi/Codex/Grok/Pi）才可用此规则。
//! - 原生消息 ID 身份的适配器（Claude/Gemini/OpenCode）事件 ID 不含代数：
//!   截断/重写只重扫，靠 `replace_existing` 覆盖同一 ID 校正，**不做代数换代**
//!   （换代墓碑会让同一 ID 永久无法重入）。
//! - OpenCode 用 `cursor_kind = "mtime"`：db 与 `-wal` 的 size:mtime 指纹不变则跳过，
//!   变化则整库重读（`replace_existing` 使重扫幂等且能校正被更新的行）。
//! - MCode 用 `cursor_kind = "row_id"`，cursor_value = max_id；重叠回看命中已存在 ID
//!   时 `replace_existing` 覆盖而不新增，游标只前进不回退。
//! - 图形界面可能不继承终端环境变量：找不到默认目录时返回空发现列表，由 UI 引导添加。

use super::{
    commit_batch, finish_parse_run, start_parse_run, CursorUpdate, EventInput, ImportBatch,
    IngestionError, SessionUpsert,
};
use crate::adapters::agents::claude_code::{self, ClaudeParseConfig};
use crate::adapters::agents::codex_rollout::{self, CodexParseConfig, CodexStatus, EventSignature};
use crate::adapters::agents::gemini_cli::{self, GeminiParseConfig};
use crate::adapters::agents::grok_build::{self, GrokParseConfig};
use crate::adapters::agents::kimi_wire::{self, WireParseConfig};
use crate::adapters::agents::mcode_db::{self, McodeConfig, McodeStatus};
use crate::adapters::agents::opencode_db::{self, OpenCodeConfig};
use crate::adapters::agents::pi_log::{self, PiParseConfig};
use crate::domain::AgentKind;
use crate::services::redact;
use crate::storage::Database;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct DiscoveredSource {
    pub source_id: String,
    pub agent: AgentKind,
    pub root: PathBuf,
    pub parser_version: &'static str,
    /// 路径解析依据（UI“为什么选中此路径”）
    pub resolution: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceScanReport {
    pub source_id: String,
    pub agent: String,
    pub files_seen: u64,
    pub files_scanned: u64,
    pub files_unchanged: u64,
    pub files_deferred: u64,
    pub events_inserted: u64,
    pub events_replaced: u64,
    pub events_already_present: u64,
    pub diagnostics: Vec<String>,
}

/// 发现已知 Agent 目录（显式环境变量优先，其次系统默认）。
pub fn discover_sources() -> Vec<DiscoveredSource> {
    let mut out = Vec::new();
    let home = home_dir();

    let kimi = std::env::var_os("KIMI_CODE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .map(|p| (p, "环境变量 KIMI_CODE_HOME".to_string()))
        .or_else(|| {
            home.as_ref()
                .map(|h| (h.join(".kimi-code"), "默认主目录 ~/.kimi-code".to_string()))
        });
    if let Some((root, how)) = kimi {
        if root.join("sessions").is_dir() {
            out.push(DiscoveredSource {
                source_id: "kimi_code:default".into(),
                agent: AgentKind::KimiCode,
                root,
                parser_version: kimi_wire::PARSER_VERSION,
                resolution: how,
            });
        }
    }

    let codex = std::env::var_os("CODEX_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .map(|p| (p, "环境变量 CODEX_HOME".to_string()))
        .or_else(|| {
            home.as_ref()
                .map(|h| (h.join(".codex"), "默认主目录 ~/.codex".to_string()))
        });
    if let Some((root, how)) = codex {
        if root.join("sessions").is_dir() || root.join("archived_sessions").is_dir() {
            out.push(DiscoveredSource {
                source_id: "codex:default".into(),
                agent: AgentKind::Codex,
                root,
                parser_version: codex_rollout::PARSER_VERSION,
                resolution: how,
            });
        }
    }

    // A01 Claude Code：CLAUDE_CONFIG_DIR > ~/.claude
    let claude = env_path("CLAUDE_CONFIG_DIR")
        .map(|p| (p, "环境变量 CLAUDE_CONFIG_DIR".to_string()))
        .or_else(|| {
            home.as_ref()
                .map(|h| (h.join(".claude"), "默认主目录 ~/.claude".to_string()))
        });
    if let Some((root, how)) = claude {
        if root.join("projects").is_dir() {
            out.push(DiscoveredSource {
                source_id: "claude_code:default".into(),
                agent: AgentKind::ClaudeCode,
                root,
                parser_version: claude_code::PARSER_VERSION,
                resolution: how,
            });
        }
    }

    // A03 Gemini CLI：~/.gemini（证据未发现环境变量覆盖，不发明变量）
    if let Some(root) = home.as_ref().map(|h| h.join(".gemini")) {
        if root.join("tmp").is_dir() {
            out.push(DiscoveredSource {
                source_id: "gemini_cli:default".into(),
                agent: AgentKind::GeminiCli,
                root,
                parser_version: gemini_cli::PARSER_VERSION,
                resolution: "默认主目录 ~/.gemini".into(),
            });
        }
    }

    // A04 Grok Build：~/.grok（证据未发现环境变量覆盖）
    if let Some(root) = home.as_ref().map(|h| h.join(".grok")) {
        if root.join("sessions").is_dir() || root.join("archived_sessions").is_dir() {
            out.push(DiscoveredSource {
                source_id: "grok_build:default".into(),
                agent: AgentKind::GrokBuild,
                root,
                parser_version: grok_build::PARSER_VERSION,
                resolution: "默认主目录 ~/.grok".into(),
            });
        }
    }

    // A05 OpenCode：OPENCODE_DB > XDG_DATA_HOME/opencode > ~/.local/share/opencode
    if let Some((db_path, how)) = opencode_db_path(home.as_deref()) {
        if db_path.is_file() {
            out.push(DiscoveredSource {
                source_id: format!(
                    "opencode:{}",
                    redact::anon_file_id(&normalize_path(&db_path).to_string_lossy())
                ),
                agent: AgentKind::Opencode,
                root: db_path,
                parser_version: opencode_db::PARSER_VERSION,
                resolution: how,
            });
        }
    }

    // A06 Pi：PI_CODING_AGENT_DIR > ~/.pi/agent；会话根再由四级规则解析
    let pi_agent = env_path("PI_CODING_AGENT_DIR")
        .filter(|p| p.is_absolute())
        .map(|p| (p, "环境变量 PI_CODING_AGENT_DIR".to_string()))
        .or_else(|| {
            home.as_ref().map(|h| {
                (
                    h.join(".pi").join("agent"),
                    "默认目录 ~/.pi/agent".to_string(),
                )
            })
        });
    if let Some((agent_dir, agent_how)) = pi_agent {
        let (session_root, how) = pi_session_root(&agent_dir, home.as_deref(), &agent_how);
        if session_root.is_dir() {
            out.push(DiscoveredSource {
                source_id: "pi:default".into(),
                agent: AgentKind::Pi,
                root: session_root,
                parser_version: pi_log::PARSER_VERSION,
                resolution: how,
            });
        }
    }

    // A07 MCode：MINIMAX_DATA_DIR > MAVIS_DATA_DIR > ~/.minimax
    let mcode = env_path("MINIMAX_DATA_DIR")
        .map(|p| (p, "环境变量 MINIMAX_DATA_DIR".to_string()))
        .or_else(|| env_path("MAVIS_DATA_DIR").map(|p| (p, "环境变量 MAVIS_DATA_DIR".to_string())))
        .or_else(|| {
            home.as_ref()
                .map(|h| (h.join(".minimax"), "默认目录 ~/.minimax".to_string()))
        });
    if let Some((root, how)) = mcode {
        let db_path = root.join("v2/sqlite/runtime-state.sqlite");
        if db_path.is_file() {
            out.push(DiscoveredSource {
                source_id: format!(
                    "mcode:{}",
                    redact::anon_file_id(&normalize_path(&db_path).to_string_lossy())
                ),
                agent: AgentKind::Mcode,
                root,
                parser_version: mcode_db::PARSER_VERSION,
                resolution: how,
            });
        }
    }

    out
}

/// 非空环境变量 → 路径（空白值视为未设置，与证据 A07 的口径一致）。
fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// 展开 Pi 允许的 `~` / `~/` / `~\` 前缀（证据 A06 `resolve_global_session_dir`）。
/// 未识别的前缀按字面路径处理（`~user` 不展开，保持非绝对并落入默认分支）。
fn expand_home(raw: &str, home: &Path) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        return home.join(rest);
    }
    if raw == "~" {
        return home.to_path_buf();
    }
    PathBuf::from(raw)
}

/// 规范化路径（解析符号链接/相对段）；不存在时退回原路径。
fn normalize_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// A05 OpenCode 库路径（证据 `opencode_config.rs:73-105`）。
fn opencode_db_path(home: Option<&Path>) -> Option<(PathBuf, String)> {
    let data_dir = || -> Option<PathBuf> {
        env_path("XDG_DATA_HOME")
            .map(|x| x.join("opencode"))
            .or_else(|| home.map(|h| h.join(".local/share/opencode")))
    };
    if let Some(raw) = env_path("OPENCODE_DB") {
        if raw.is_absolute() {
            return Some((raw, "环境变量 OPENCODE_DB（绝对路径）".into()));
        }
        // 相对路径拼到数据目录下（证据 `:84`）。
        return data_dir().map(|d| (d.join(raw), "环境变量 OPENCODE_DB（相对数据目录）".into()));
    }
    if let Some(xdg) = env_path("XDG_DATA_HOME") {
        return Some((
            xdg.join("opencode").join("opencode.db"),
            "环境变量 XDG_DATA_HOME".into(),
        ));
    }
    home.map(|h| {
        (
            h.join(".local/share/opencode/opencode.db"),
            "默认目录 ~/.local/share/opencode".into(),
        )
    })
}

/// A06 Pi 会话根：`PI_CODING_AGENT_SESSION_DIR` > `settings.json:sessionDir` > `<agent>/sessions`。
/// 环境变量与设置项都必须是绝对路径且存在的目录（证据 `pi.rs:160-196`），否则落回默认。
fn pi_session_root(agent_dir: &Path, home: Option<&Path>, agent_how: &str) -> (PathBuf, String) {
    if let Some(raw) = env_path("PI_CODING_AGENT_SESSION_DIR") {
        if raw.is_absolute() && raw.is_dir() {
            return (
                raw,
                format!("环境变量 PI_CODING_AGENT_SESSION_DIR（agent 目录：{agent_how}）"),
            );
        }
    }
    if let Some(home) = home {
        if let Some(configured) = std::fs::read_to_string(agent_dir.join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| {
                v.get("sessionDir")
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
            })
        {
            let path = expand_home(&configured, home);
            if path.is_absolute() && path.is_dir() {
                return (
                    path,
                    format!("settings.json sessionDir（agent 目录：{agent_how}）"),
                );
            }
        }
    }
    (
        agent_dir.join("sessions"),
        format!("默认会话根 <agent>/sessions（agent 目录：{agent_how}）"),
    )
}

/// GUI 环境下读主目录：unix $HOME / Windows %USERPROFILE%。
fn home_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

pub fn scan_source(
    db: &mut Database,
    src: &DiscoveredSource,
) -> Result<SourceScanReport, IngestionError> {
    db.conn().execute(
        "INSERT INTO sources (source_id, agent, stream_domain, root_path, display_label, parser_version, last_sync_at)
         VALUES (?1, ?2, 'local_log', ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ','now'))
         ON CONFLICT(source_id) DO UPDATE SET parser_version = excluded.parser_version,
            last_sync_at = excluded.last_sync_at, root_path = excluded.root_path",
        rusqlite::params![
            src.source_id,
            src.agent.as_str(),
            src.root.to_string_lossy(),
            format!("{}（{}）", src.agent.as_str(), src.resolution),
            src.parser_version,
        ],
    )?;

    match src.agent {
        AgentKind::KimiCode => scan_kimi(db, src),
        AgentKind::Codex => scan_codex(db, src),
        AgentKind::ClaudeCode => scan_claude_code(db, src),
        AgentKind::GeminiCli => scan_gemini(db, src),
        AgentKind::GrokBuild => scan_grok(db, src),
        AgentKind::Opencode => scan_opencode(db, src),
        AgentKind::Pi => scan_pi(db, src),
        AgentKind::Mcode => scan_mcode(db, src),
        AgentKind::ClaudeDesktop => unreachable!("scanner not implemented for this agent"),
    }
}

/// 空报告骨架（各扫描函数的计数与诊断累加起点）。
fn base_report(src: &DiscoveredSource) -> SourceScanReport {
    SourceScanReport {
        source_id: src.source_id.clone(),
        agent: src.agent.as_str().into(),
        files_seen: 0,
        files_scanned: 0,
        files_unchanged: 0,
        files_deferred: 0,
        events_inserted: 0,
        events_replaced: 0,
        events_already_present: 0,
        diagnostics: vec![],
    }
}

/// 诊断里的文件/库路径一律匿名化（SECURITY_PRIVACY：日志不含本机绝对路径）。
fn anon_id(path: &str) -> String {
    redact::redact_path(path)
}

struct FileCursor {
    generation: u64,
    consumed_bytes: u64,
    tail_checksum: Option<String>,
    parser_version: String,
}

fn read_cursor(
    db: &Database,
    source_id: &str,
    file_identity: &str,
    cursor_kind: &str,
) -> Result<Option<FileCursor>, IngestionError> {
    let mut stmt = db.conn().prepare(
        "SELECT generation, cursor_value, tail_checksum, parser_version FROM ingestion_cursors
         WHERE source_id = ?1 AND file_identity = ?2 AND cursor_kind = ?3
         ORDER BY generation DESC LIMIT 1",
    )?;
    let mut rows = stmt.query(rusqlite::params![source_id, file_identity, cursor_kind])?;
    Ok(if let Some(r) = rows.next()? {
        let value: String = r.get(1)?;
        Some(FileCursor {
            generation: r.get::<_, i64>(0)? as u64,
            consumed_bytes: value.parse().unwrap_or(0),
            tail_checksum: r.get(2)?,
            parser_version: r.get(3)?,
        })
    } else {
        None
    })
}

/// 依据 tail_checksum 判断是否需要（重）扫描；从未记录过一律需要。
fn checksum_changed(cursor: Option<&FileCursor>, checksum: &str) -> bool {
    cursor.and_then(|c| c.tail_checksum.as_deref()) != Some(checksum)
}

enum FileAction {
    Skip,
    Scan {
        generation: u64,
        retire_before: Option<u64>,
    },
}

fn decide_action(cursor: Option<&FileCursor>, size: u64, mtime_secs: u64) -> FileAction {
    let checksum = format!("{size}:{mtime_secs}");
    match cursor {
        Some(c) if c.tail_checksum.as_deref() == Some(&checksum) => FileAction::Skip,
        Some(c) if size < c.consumed_bytes => FileAction::Scan {
            generation: c.generation + 1,
            retire_before: Some(c.generation + 1),
        },
        Some(c) => FileAction::Scan {
            generation: c.generation,
            retire_before: None,
        },
        None => FileAction::Scan {
            generation: 0,
            retire_before: None,
        },
    }
}

fn observed_now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

fn file_meta(path: &Path) -> Option<(u64, u64)> {
    let meta = path.metadata().ok()?;
    let size = meta.len();
    let mtime_secs = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Some((size, mtime_secs))
}

fn scan_kimi(
    db: &mut Database,
    src: &DiscoveredSource,
) -> Result<SourceScanReport, IngestionError> {
    let mut report = SourceScanReport {
        source_id: src.source_id.clone(),
        agent: src.agent.as_str().into(),
        files_seen: 0,
        files_scanned: 0,
        files_unchanged: 0,
        files_deferred: 0,
        events_inserted: 0,
        events_replaced: 0,
        events_already_present: 0,
        diagnostics: vec![],
    };
    let sessions_root = src.root.join("sessions");

    // 第一遍：收集会话目录与 forkedFrom（父可用性判定）
    struct KimiSession {
        session_id: String,
        forked_from: Option<String>,
        /// (agent_id, wire_path)
        wires: Vec<(String, PathBuf)>,
    }
    let mut sessions: Vec<KimiSession> = Vec::new();
    let mut stack = vec![sessions_root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let depth = path
                .strip_prefix(&sessions_root)
                .unwrap()
                .components()
                .count();
            if depth < 2 {
                stack.push(path);
                continue;
            }
            // depth == 2：<wdKey>/<sessionId>
            let session_id = path.file_name().unwrap().to_string_lossy().into_owned();
            let forked_from = std::fs::read_to_string(path.join("state.json"))
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .and_then(|v| {
                    v.get("forkedFrom")
                        .and_then(|f| f.as_str())
                        .map(str::to_string)
                });
            let mut wires = Vec::new();
            if let Ok(agent_entries) = std::fs::read_dir(path.join("agents")) {
                for agent_entry in agent_entries.flatten() {
                    let wire_path = agent_entry.path().join("wire.jsonl");
                    if wire_path.is_file() {
                        wires.push((
                            agent_entry.file_name().to_string_lossy().into_owned(),
                            wire_path,
                        ));
                    }
                }
            }
            sessions.push(KimiSession {
                session_id,
                forked_from,
                wires,
            });
        }
    }
    let known_ids: HashSet<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();

    let run_id = start_parse_run(db, &src.source_id, src.parser_version)?;

    for session in &sessions {
        let has_subagents = session.wires.iter().any(|(name, _)| name != "main");
        for (agent_id, wire_path) in &session.wires {
            report.files_seen += 1;
            let file_identity = wire_path.to_string_lossy().into_owned();
            let Some((size, mtime_secs)) = file_meta(wire_path) else {
                continue;
            };
            let cursor = read_cursor(db, &src.source_id, &file_identity, "byte_offset")?;
            let generation = match decide_action(cursor.as_ref(), size, mtime_secs) {
                FileAction::Skip => {
                    report.files_unchanged += 1;
                    continue;
                }
                FileAction::Scan {
                    generation,
                    retire_before,
                } => {
                    if let Some(before) = retire_before {
                        let retired = super::retire_file_generations(
                            db,
                            &src.source_id,
                            &file_identity,
                            before,
                        )?;
                        if retired > 0 {
                            report.diagnostics.push(format!(
                                "truncation_regen:{}: 退休旧代数事件 {retired} 条",
                                anon_id(&file_identity)
                            ));
                        }
                    }
                    generation
                }
            };

            let bytes = match std::fs::read(wire_path) {
                Ok(b) => b,
                Err(e) => {
                    report
                        .diagnostics
                        .push(format!("read_error:{}: {e}", anon_id(&file_identity)));
                    continue;
                }
            };
            let cfg = WireParseConfig {
                source_id: src.source_id.clone(),
                file_identity: file_identity.clone(),
                generation,
                session_id: session.session_id.clone(),
                dir_agent_id: Some(agent_id.clone()),
                forked_from_present: session.forked_from.is_some(),
                parent_available: session
                    .forked_from
                    .as_ref()
                    .is_some_and(|p| known_ids.contains(p.as_str())),
                observed_at: observed_now(),
            };
            let out = kimi_wire::parse_wire(&bytes, &cfg);
            report.diagnostics.extend(out.diagnostics.iter().cloned());
            report.files_scanned += 1;

            let batch = ImportBatch {
                source_id: src.source_id.clone(),
                events: out
                    .events
                    .into_iter()
                    .map(|event| EventInput {
                        event,
                        replace_existing: false,
                    })
                    .collect(),
                sessions: vec![SessionUpsert {
                    session_id: session.session_id.clone(),
                    parent_session_key: session
                        .forked_from
                        .as_ref()
                        .map(|p| format!("{}:{}", src.source_id, p)),
                    title: None,
                    project_ref: None,
                    last_seen_at: None,
                    archived: false,
                    has_subagents,
                }],
                cursor: Some(CursorUpdate {
                    file_identity: file_identity.clone(),
                    generation,
                    parser_version: src.parser_version.into(),
                    cursor_kind: "byte_offset".into(),
                    cursor_value: out.consumed_bytes.to_string(),
                    tail_checksum: Some(format!("{size}:{mtime_secs}")),
                }),
            };
            match commit_batch(db, run_id, &batch) {
                Ok(stats) => {
                    report.events_inserted += stats.inserted;
                    report.events_replaced += stats.replaced;
                    report.events_already_present += stats.already_present;
                }
                Err(e) => {
                    finish_parse_run(db, run_id, false, Some(&e.to_string()))?;
                    return Err(e);
                }
            }
        }
    }

    finish_parse_run(db, run_id, true, None)?;
    Ok(report)
}

/// 从 rollout 文件名提取 UUID（8-4-4-4-12 hex）。双段名取前段（逻辑线程）。
fn uuid_from_filename(name: &str) -> Option<String> {
    let bytes = name.as_bytes();
    if bytes.len() < 36 {
        return None;
    }
    let dash_positions = [8usize, 13, 18, 23];
    for start in 0..=bytes.len() - 36 {
        let seg = &bytes[start..start + 36];
        let ok = seg.iter().enumerate().all(|(i, &c)| {
            if dash_positions.contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        });
        if ok {
            return Some(name[start..start + 36].to_string());
        }
    }
    None
}

struct PendingFile {
    bytes: Vec<u8>,
    generation: u64,
    file_identity: String,
    size: u64,
    mtime_secs: u64,
    filename_thread_id: Option<String>,
    archived: bool,
}

fn scan_codex(
    db: &mut Database,
    src: &DiscoveredSource,
) -> Result<SourceScanReport, IngestionError> {
    let mut report = SourceScanReport {
        source_id: src.source_id.clone(),
        agent: src.agent.as_str().into(),
        files_seen: 0,
        files_scanned: 0,
        files_unchanged: 0,
        files_deferred: 0,
        events_inserted: 0,
        events_replaced: 0,
        events_already_present: 0,
        diagnostics: vec![],
    };

    // 收集 rollout 文件；同名 .zst 与 .jsonl 并存只留普通形态
    let mut raw_files: Vec<PathBuf> = Vec::new();
    collect_rollouts(&src.root.join("sessions"), 3, &mut raw_files);
    let archived_root = src.root.join("archived_sessions");
    collect_rollouts(&archived_root, 1, &mut raw_files);
    let mut by_logical: HashMap<String, PathBuf> = HashMap::new();
    for p in raw_files {
        let logical = p.to_string_lossy().trim_end_matches(".zst").to_string();
        by_logical
            .entry(logical)
            .and_modify(|e| {
                if p.extension().is_some_and(|x| x == "jsonl") {
                    *e = p.clone();
                }
            })
            .or_insert(p);
    }
    let files: Vec<PathBuf> = by_logical.into_values().collect();
    report.files_seen = files.len() as u64;

    let run_id = start_parse_run(db, &src.source_id, src.parser_version)?;

    // 准备阶段：读文件、判游标/代
    let mut pendings: Vec<PendingFile> = Vec::new();
    for path in &files {
        let file_identity = path.to_string_lossy().trim_end_matches(".zst").to_string();
        let Some((size, mtime_secs)) = file_meta(path) else {
            continue;
        };
        let mut cursor = read_cursor(db, &src.source_id, &file_identity, "byte_offset")?;
        if let Some(old) = &mut cursor {
            if old.parser_version != src.parser_version {
                old.tail_checksum = None;
            }
        }
        let generation = match decide_action(cursor.as_ref(), size, mtime_secs) {
            FileAction::Skip => {
                report.files_unchanged += 1;
                continue;
            }
            FileAction::Scan {
                generation,
                retire_before,
            } => {
                if let Some(before) = retire_before {
                    let retired =
                        super::retire_file_generations(db, &src.source_id, &file_identity, before)?;
                    if retired > 0 {
                        report.diagnostics.push(format!(
                            "truncation_regen:{}: 退休旧代数事件 {retired} 条",
                            anon_id(&file_identity)
                        ));
                    }
                }
                generation
            }
        };
        let bytes = match read_rollout_bytes(path) {
            Ok(b) => b,
            Err(e) => {
                report
                    .diagnostics
                    .push(format!("read_error:{}: {e}", anon_id(&file_identity)));
                continue;
            }
        };
        pendings.push(PendingFile {
            bytes,
            generation,
            filename_thread_id: path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(uuid_from_filename),
            file_identity,
            size,
            mtime_secs,
            archived: path.starts_with(&archived_root),
        });
    }

    // 两遍解析：先非 fork 文件收集签名时间线，再处理 fork 文件
    let mut timelines: HashMap<String, Vec<EventSignature>> = HashMap::new();
    let mut ready: Vec<(PendingFile, codex_rollout::CodexParseOutcome)> = Vec::new();
    let mut deferred_files: Vec<PendingFile> = Vec::new();

    for pending in pendings {
        let cfg = CodexParseConfig {
            source_id: src.source_id.clone(),
            file_identity: pending.file_identity.clone(),
            generation: pending.generation,
            observed_at: observed_now(),
            filename_thread_id: pending.filename_thread_id.clone(),
            parent_signatures: None,
        };
        let out = codex_rollout::parse_rollout(&pending.bytes, &cfg);
        match &out.status {
            Some(CodexStatus::Deferred(_)) => deferred_files.push(pending),
            _ => {
                if let Some(s) = &out.session {
                    if let Some(tid) = &s.thread_id {
                        timelines.insert(tid.clone(), out.signatures.clone());
                    }
                }
                ready.push((pending, out));
            }
        }
    }

    let parent_files: HashMap<String, &PathBuf> = files
        .iter()
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?;
            Some((uuid_from_filename(name)?, path))
        })
        .collect();
    for pending in deferred_files {
        // probe 只为取 session 信息（parent_signatures=None 时 fork 文件必 Deferred）
        let probe_cfg = CodexParseConfig {
            source_id: src.source_id.clone(),
            file_identity: pending.file_identity.clone(),
            generation: pending.generation,
            observed_at: observed_now(),
            filename_thread_id: pending.filename_thread_id.clone(),
            parent_signatures: None,
        };
        let probe = codex_rollout::parse_rollout(&pending.bytes, &probe_cfg);
        let parent_id = probe.session.and_then(|s| s.parent_thread_id);
        let timeline = parent_id
            .as_ref()
            .and_then(|p| timelines.get(p))
            .cloned()
            .or_else(|| {
                let path = parent_files.get(parent_id.as_ref()?)?;
                let bytes = read_rollout_bytes(path).ok()?;
                // 只构建签名，不导入祖先；父未变化也能供活跃子会话对齐。
                let config = CodexParseConfig {
                    source_id: src.source_id.clone(),
                    file_identity: path.to_string_lossy().into_owned(),
                    generation: 0,
                    observed_at: observed_now(),
                    filename_thread_id: parent_id.clone(),
                    parent_signatures: Some(vec![]),
                };
                let parsed = codex_rollout::parse_rollout(&bytes, &config);
                (parsed.status == Some(CodexStatus::Ok)).then_some(parsed.signatures)
            });
        match timeline {
            Some(tl) => {
                let cfg = CodexParseConfig {
                    source_id: src.source_id.clone(),
                    file_identity: pending.file_identity.clone(),
                    generation: pending.generation,
                    observed_at: observed_now(),
                    filename_thread_id: pending.filename_thread_id.clone(),
                    parent_signatures: Some(tl),
                };
                let out = codex_rollout::parse_rollout(&pending.bytes, &cfg);
                if let Some(s) = &out.session {
                    if let Some(tid) = &s.thread_id {
                        timelines.insert(tid.clone(), out.signatures.clone());
                    }
                }
                ready.push((pending, out));
            }
            None => {
                report.files_deferred += 1;
                report.diagnostics.push(format!(
                    "deferred: 父时间线不可用（{} ← {}），下轮重试",
                    anon_id(&pending.file_identity),
                    "unavailable"
                ));
            }
        }
    }

    for (pending, out) in ready {
        report.files_scanned += 1;
        report.diagnostics.extend(out.diagnostics.iter().cloned());
        let session_upserts = out
            .session
            .as_ref()
            .and_then(|s| {
                s.thread_id.clone().map(|sid| SessionUpsert {
                    session_id: sid,
                    parent_session_key: s
                        .parent_thread_id
                        .clone()
                        .map(|p| format!("{}:{}", src.source_id, p)),
                    title: None,
                    project_ref: None,
                    last_seen_at: s.first_timestamp.clone(),
                    archived: pending.archived,
                    has_subagents: s.parent_thread_id.is_some(),
                })
            })
            .into_iter()
            .collect();
        let batch = ImportBatch {
            source_id: src.source_id.clone(),
            events: out
                .events
                .into_iter()
                .map(|event| EventInput {
                    event,
                    replace_existing: false,
                })
                .collect(),
            sessions: session_upserts,
            cursor: Some(CursorUpdate {
                file_identity: pending.file_identity.clone(),
                generation: pending.generation,
                parser_version: src.parser_version.into(),
                cursor_kind: "byte_offset".into(),
                cursor_value: out.consumed_bytes.to_string(),
                tail_checksum: Some(format!("{}:{}", pending.size, pending.mtime_secs)),
            }),
        };
        match super::commit_batch_retiring(db, run_id, &batch, &out.duplicate_event_ids) {
            Ok(stats) => {
                if stats.tombstoned > 0 {
                    report
                        .diagnostics
                        .push(format!("duplicate_snapshot_retired:{}", stats.tombstoned));
                }
                report.events_inserted += stats.inserted;
                report.events_replaced += stats.replaced;
                report.events_already_present += stats.already_present;
            }
            Err(e) => {
                finish_parse_run(db, run_id, false, Some(&e.to_string()))?;
                return Err(e);
            }
        }
    }

    finish_parse_run(db, run_id, true, None)?;
    Ok(report)
}

// ---------------------------------------------------------------------------
// A01 / A03 / A04 / A05 / A06 / A07 共用工具
// ---------------------------------------------------------------------------

/// 提交一批并累计统计；失败时结束 parse_run 并上抛（沿用既有纪律）。
fn commit_and_count(
    db: &mut Database,
    run_id: i64,
    batch: &ImportBatch,
    report: &mut SourceScanReport,
) -> Result<(), IngestionError> {
    match commit_batch(db, run_id, batch) {
        Ok(stats) => {
            report.events_inserted += stats.inserted;
            report.events_replaced += stats.replaced;
            report.events_already_present += stats.already_present;
            Ok(())
        }
        Err(e) => {
            finish_parse_run(db, run_id, false, Some(&e.to_string()))?;
            Err(e)
        }
    }
}

/// 会话 upsert 去重：同一批里同一 session_id 只建一条（事件可能来自多个文件）。
fn session_upserts(
    session_ids: impl IntoIterator<Item = Option<String>>,
    archived: bool,
    has_subagents: bool,
) -> Vec<SessionUpsert> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for sid in session_ids.into_iter().flatten() {
        if seen.insert(sid.clone()) {
            out.push(SessionUpsert {
                session_id: sid,
                parent_session_key: None,
                title: None,
                project_ref: None,
                last_seen_at: None,
                archived,
                has_subagents,
            });
        }
    }
    out
}

/// 目录的直接子目录（排序，保证扫描顺序稳定）。
fn dirs_of(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

/// 目录的直接子 `.jsonl` 文件（排序）。
fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    out.sort();
    out
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 文件相对 `base` 的路径段（`/` 连接、去掉 `.jsonl` 扩展名），用作子 Agent actor 标识。
fn relative_jsonl_id(base: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(base).unwrap_or(path);
    let mut parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if let Some(last) = parts.last_mut() {
        if let Some(stem) = last.strip_suffix(".jsonl") {
            *last = stem.to_string();
        }
    }
    parts.join("/")
}

/// Grok `updates.jsonl` 单文件上限（证据：>50 MiB 视为异常/恶意文件，跳过）。
const GROK_MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;
/// Grok 递归收集深度上限（证据 `session_usage_grokbuild.rs#L150-L162`）。
const GROK_MAX_DEPTH: usize = 16;
/// Pi 会话 JSONL 递归收集深度上限（官方 ProjectDirectories 布局）。
const PI_MAX_DEPTH: usize = 8;

// ---------------------------------------------------------------------------
// A01 Claude Code
// ---------------------------------------------------------------------------

fn scan_claude_code(
    db: &mut Database,
    src: &DiscoveredSource,
) -> Result<SourceScanReport, IngestionError> {
    let mut report = base_report(src);
    let projects_root = src.root.join("projects");

    struct Target {
        path: PathBuf,
        is_subagent: bool,
        actor_id: Option<String>,
        /// 主会话 = 文件 stem；子 Agent = 会话目录名（用于 has_subagents 归组）
        session_hint: String,
    }
    let mut targets: Vec<Target> = Vec::new();
    let mut subagent_groups: HashSet<String> = HashSet::new();

    for project_dir in dirs_of(&projects_root) {
        // 子 Agent：固定深度 <session>/subagents/*.jsonl 与
        // <session>/subagents/workflows/wf_*/*.jsonl（不无限递归）。
        for session_dir in dirs_of(&project_dir) {
            let subagents = session_dir.join("subagents");
            if !subagents.is_dir() {
                continue;
            }
            let session_name = file_name(&session_dir);
            subagent_groups.insert(session_name.clone());
            for file in jsonl_files(&subagents) {
                targets.push(Target {
                    actor_id: Some(relative_jsonl_id(&session_dir, &file)),
                    is_subagent: true,
                    session_hint: session_name.clone(),
                    path: file,
                });
            }
            for wf_dir in dirs_of(&subagents.join("workflows")) {
                if !file_name(&wf_dir).starts_with("wf_") {
                    continue;
                }
                for file in jsonl_files(&wf_dir) {
                    targets.push(Target {
                        actor_id: Some(relative_jsonl_id(&session_dir, &file)),
                        is_subagent: true,
                        session_hint: session_name.clone(),
                        path: file,
                    });
                }
            }
        }
        // 主会话：<project>/*.jsonl（不递归进会话目录）
        for file in jsonl_files(&project_dir) {
            targets.push(Target {
                is_subagent: false,
                actor_id: None,
                session_hint: file
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                path: file,
            });
        }
    }

    let run_id = start_parse_run(db, &src.source_id, src.parser_version)?;
    for target in &targets {
        report.files_seen += 1;
        let file_identity = target.path.to_string_lossy().into_owned();
        let Some((size, mtime_secs)) = file_meta(&target.path) else {
            continue;
        };
        let checksum = format!("{size}:{mtime_secs}");
        let cursor = read_cursor(db, &src.source_id, &file_identity, "byte_offset")?;
        if !checksum_changed(cursor.as_ref(), &checksum) {
            report.files_unchanged += 1;
            continue;
        }
        let bytes = match std::fs::read(&target.path) {
            Ok(b) => b,
            Err(e) => {
                report
                    .diagnostics
                    .push(format!("read_error:{}: {e}", anon_id(&file_identity)));
                continue;
            }
        };
        let cfg = ClaudeParseConfig {
            source_id: src.source_id.clone(),
            file_identity: file_identity.clone(),
            // 原生 message ID 身份：不换代（见模块文档）。
            generation: 0,
            observed_at: observed_now(),
            is_subagent: target.is_subagent,
            subagent_actor_id: target.actor_id.clone(),
        };
        let out = claude_code::parse_projects(&bytes, &cfg);
        report.diagnostics.extend(out.diagnostics.iter().cloned());
        report.files_scanned += 1;

        let session_id = out
            .events
            .iter()
            .find_map(|p| p.event.session_id.clone())
            .or_else(|| (!target.session_hint.is_empty()).then(|| target.session_hint.clone()));
        let has_subagents = subagent_groups.contains(&target.session_hint)
            || session_id
                .as_deref()
                .is_some_and(|s| subagent_groups.contains(s));
        let sessions = session_upserts([session_id], false, has_subagents);
        let batch = ImportBatch {
            source_id: src.source_id.clone(),
            events: out
                .events
                .into_iter()
                .map(|p| EventInput {
                    event: p.event,
                    replace_existing: p.replace_existing,
                })
                .collect(),
            sessions,
            cursor: Some(CursorUpdate {
                file_identity,
                generation: 0,
                parser_version: src.parser_version.into(),
                cursor_kind: "byte_offset".into(),
                cursor_value: out.consumed_bytes.to_string(),
                tail_checksum: Some(checksum),
            }),
        };
        commit_and_count(db, run_id, &batch, &mut report)?;
    }

    finish_parse_run(db, run_id, true, None)?;
    Ok(report)
}

// ---------------------------------------------------------------------------
// A03 Gemini CLI
// ---------------------------------------------------------------------------

fn scan_gemini(
    db: &mut Database,
    src: &DiscoveredSource,
) -> Result<SourceScanReport, IngestionError> {
    let mut report = base_report(src);
    let tmp_root = src.root.join("tmp");

    // tmp/<hash>/chats/session-*：同名 .json 与 .jsonl 并存只读 .jsonl。
    let mut selected: Vec<(PathBuf, bool)> = Vec::new();
    for hash_dir in dirs_of(&tmp_root) {
        let chats = hash_dir.join("chats");
        let Ok(entries) = std::fs::read_dir(&chats) else {
            continue;
        };
        let mut by_stem: HashMap<String, (Option<PathBuf>, Option<PathBuf>)> = HashMap::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if !file_name(&path).starts_with("session-") {
                continue;
            }
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if stem.is_empty() {
                continue;
            }
            match path.extension().and_then(|e| e.to_str()) {
                Some("jsonl") => by_stem.entry(stem).or_insert((None, None)).1 = Some(path),
                Some("json") => by_stem.entry(stem).or_insert((None, None)).0 = Some(path),
                _ => {}
            }
        }
        let mut stems: Vec<String> = by_stem.keys().cloned().collect();
        stems.sort();
        for stem in stems {
            let (json, jsonl) = by_stem.remove(&stem).expect("collected stem");
            if let Some(p) = jsonl {
                selected.push((p, true));
            } else if let Some(p) = json {
                selected.push((p, false));
            }
        }
    }

    let run_id = start_parse_run(db, &src.source_id, src.parser_version)?;
    for (path, is_jsonl) in selected {
        report.files_seen += 1;
        let file_identity = path.to_string_lossy().into_owned();
        let Some((size, mtime_secs)) = file_meta(&path) else {
            continue;
        };
        let checksum = format!("{size}:{mtime_secs}");
        let cursor = read_cursor(db, &src.source_id, &file_identity, "byte_offset")?;
        if !checksum_changed(cursor.as_ref(), &checksum) {
            report.files_unchanged += 1;
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                report
                    .diagnostics
                    .push(format!("read_error:{}: {e}", anon_id(&file_identity)));
                continue;
            }
        };
        let cfg = GeminiParseConfig {
            source_id: src.source_id.clone(),
            file_identity: file_identity.clone(),
            generation: 0,
            observed_at: observed_now(),
        };
        let out = if is_jsonl {
            gemini_cli::parse_session_jsonl(&bytes, &cfg)
        } else {
            gemini_cli::parse_session_json(&bytes, &cfg)
        };
        report.diagnostics.extend(out.diagnostics.iter().cloned());
        report.files_scanned += 1;

        let sessions = session_upserts([out.session_id.clone()], false, false);
        // JSON 整文件解析：解析失败（consumed == 0）不推进游标，下轮重试；
        // JSONL 追加/重写都需要从 0 回放，游标只用于记录完整消费位置与变化检测。
        let cursor_update = (is_jsonl || out.consumed_bytes > 0).then(|| CursorUpdate {
            file_identity,
            generation: 0,
            parser_version: src.parser_version.into(),
            cursor_kind: "byte_offset".into(),
            cursor_value: out.consumed_bytes.to_string(),
            tail_checksum: Some(checksum),
        });
        let batch = ImportBatch {
            source_id: src.source_id.clone(),
            events: out
                .events
                .into_iter()
                .map(|p| EventInput {
                    event: p.event,
                    replace_existing: p.replace_existing,
                })
                .collect(),
            sessions,
            cursor: cursor_update,
        };
        commit_and_count(db, run_id, &batch, &mut report)?;
    }

    finish_parse_run(db, run_id, true, None)?;
    Ok(report)
}

// ---------------------------------------------------------------------------
// A04 Grok Build
// ---------------------------------------------------------------------------

/// 递归收集 `updates.jsonl`（深度上限、跳过符号链接、超大文件跳过并记诊断）。
fn collect_updates(
    dir: &Path,
    depth: usize,
    archived: bool,
    out: &mut Vec<(PathBuf, bool)>,
    report: &mut SourceScanReport,
) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_symlink() {
            report.diagnostics.push(format!(
                "symlink_skipped:{}",
                anon_id(&path.to_string_lossy())
            ));
            continue;
        }
        if file_type.is_dir() {
            collect_updates(&path, depth - 1, archived, out, report);
        } else if file_type.is_file() && file_name(&path) == "updates.jsonl" {
            match std::fs::metadata(&path) {
                Ok(meta) if meta.len() > GROK_MAX_FILE_BYTES => {
                    report.files_deferred += 1;
                    report.diagnostics.push(format!(
                        "oversize_skipped:{}: {} bytes > 50MiB",
                        anon_id(&path.to_string_lossy()),
                        meta.len()
                    ));
                }
                Ok(_) => out.push((path, archived)),
                Err(_) => {}
            }
        }
    }
}

fn scan_grok(
    db: &mut Database,
    src: &DiscoveredSource,
) -> Result<SourceScanReport, IngestionError> {
    let mut report = base_report(src);
    let mut files: Vec<(PathBuf, bool)> = Vec::new();
    collect_updates(
        &src.root.join("sessions"),
        GROK_MAX_DEPTH,
        false,
        &mut files,
        &mut report,
    );
    collect_updates(
        &src.root.join("archived_sessions"),
        GROK_MAX_DEPTH,
        true,
        &mut files,
        &mut report,
    );
    files.sort();

    let run_id = start_parse_run(db, &src.source_id, src.parser_version)?;
    for (path, archived) in files {
        report.files_seen += 1;
        let file_identity = path.to_string_lossy().into_owned();
        let Some((size, mtime_secs)) = file_meta(&path) else {
            continue;
        };
        let cursor = read_cursor(db, &src.source_id, &file_identity, "byte_offset")?;
        // 事件 ID 含代数：截断/重写换代 + 退休旧代（墓碑防三度计入）。
        let generation = match decide_action(cursor.as_ref(), size, mtime_secs) {
            FileAction::Skip => {
                report.files_unchanged += 1;
                continue;
            }
            FileAction::Scan {
                generation,
                retire_before,
            } => {
                if let Some(before) = retire_before {
                    let retired =
                        super::retire_file_generations(db, &src.source_id, &file_identity, before)?;
                    if retired > 0 {
                        report.diagnostics.push(format!(
                            "truncation_regen:{}: 退休旧代数事件 {retired} 条",
                            anon_id(&file_identity)
                        ));
                    }
                }
                generation
            }
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                report
                    .diagnostics
                    .push(format!("read_error:{}: {e}", anon_id(&file_identity)));
                continue;
            }
        };
        let session_dir_name = file_name(path.parent().unwrap_or(&path));
        let cfg = GrokParseConfig {
            source_id: src.source_id.clone(),
            file_identity: file_identity.clone(),
            generation,
            session_dir_name: session_dir_name.clone(),
            archived,
            observed_at: observed_now(),
        };
        let out = grok_build::parse_updates(&bytes, &cfg);
        report.diagnostics.extend(out.diagnostics.iter().cloned());
        report.files_scanned += 1;

        let batch = ImportBatch {
            source_id: src.source_id.clone(),
            events: out
                .events
                .into_iter()
                .map(|event| EventInput {
                    event,
                    replace_existing: false,
                })
                .collect(),
            sessions: session_upserts([Some(session_dir_name)], archived, false),
            cursor: Some(CursorUpdate {
                file_identity,
                generation,
                parser_version: src.parser_version.into(),
                cursor_kind: "byte_offset".into(),
                cursor_value: out.consumed_bytes.to_string(),
                tail_checksum: Some(format!("{size}:{mtime_secs}")),
            }),
        };
        commit_and_count(db, run_id, &batch, &mut report)?;
    }

    finish_parse_run(db, run_id, true, None)?;
    Ok(report)
}

// ---------------------------------------------------------------------------
// A05 OpenCode（只读 SQLite，mtime 游标）
// ---------------------------------------------------------------------------

/// SQLite 主库与 `-wal` 的组合指纹（含纳秒 mtime），用于 mtime 游标。
struct SqliteStamp {
    signature: String,
    mtime_nanos: u128,
}

fn wal_path(db_path: &Path) -> PathBuf {
    let mut name = db_path.as_os_str().to_os_string();
    name.push("-wal");
    PathBuf::from(name)
}

fn stat_stamp(path: &Path) -> Option<(u64, u128)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime_nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Some((meta.len(), mtime_nanos))
}

fn sqlite_stamp(db_path: &Path) -> SqliteStamp {
    let mut parts: Vec<String> = Vec::new();
    let mut max_mtime = 0u128;
    for (label, path) in [("db", db_path.to_path_buf()), ("wal", wal_path(db_path))] {
        match stat_stamp(&path) {
            Some((size, mtime)) => {
                max_mtime = max_mtime.max(mtime);
                parts.push(format!("{label}={size}:{mtime}"));
            }
            None => parts.push(format!("{label}=absent")),
        }
    }
    SqliteStamp {
        signature: parts.join(","),
        mtime_nanos: max_mtime,
    }
}

fn scan_opencode(
    db: &mut Database,
    src: &DiscoveredSource,
) -> Result<SourceScanReport, IngestionError> {
    let mut report = base_report(src);
    // root 即解析后的库文件路径（见模块文档）。
    let db_path = src.root.clone();
    let file_identity = db_path.to_string_lossy().into_owned();
    if !db_path.is_file() {
        report
            .diagnostics
            .push(format!("db_missing:{}", anon_id(&file_identity)));
        return Ok(report);
    }
    report.files_seen = 1;

    let stamp = sqlite_stamp(&db_path);
    let cursor = read_cursor(db, &src.source_id, &file_identity, "mtime")?;
    if !checksum_changed(cursor.as_ref(), &stamp.signature) {
        report.files_unchanged += 1;
        return Ok(report);
    }
    let conn = match opencode_db::open_read_only(&db_path) {
        Ok(c) => c,
        Err(e) => {
            report
                .diagnostics
                .push(format!("open_failed:{}: {e}", anon_id(&file_identity)));
            return Ok(report);
        }
    };

    let run_id = start_parse_run(db, &src.source_id, src.parser_version)?;
    let cfg = OpenCodeConfig {
        source_id: src.source_id.clone(),
        file_identity: file_identity.clone(),
        generation: 0,
        observed_at: observed_now(),
        session_id: None,
    };
    // 整库重读：消息行可能被更新，replace_existing 覆盖而不新增（重扫幂等）。
    let out = opencode_db::read_events(&conn, &cfg);
    report.diagnostics.extend(out.diagnostics.iter().cloned());
    report.files_scanned += 1;

    let sessions = session_upserts(
        out.events.iter().map(|p| p.event.session_id.clone()),
        false,
        false,
    );
    let batch = ImportBatch {
        source_id: src.source_id.clone(),
        events: out
            .events
            .into_iter()
            .map(|p| EventInput {
                event: p.event,
                replace_existing: p.replace_existing,
            })
            .collect(),
        sessions,
        cursor: Some(CursorUpdate {
            file_identity,
            generation: 0,
            parser_version: src.parser_version.into(),
            cursor_kind: "mtime".into(),
            cursor_value: stamp.mtime_nanos.to_string(),
            tail_checksum: Some(stamp.signature),
        }),
    };
    commit_and_count(db, run_id, &batch, &mut report)?;

    finish_parse_run(db, run_id, true, None)?;
    Ok(report)
}

// ---------------------------------------------------------------------------
// A06 Pi（会话 JSONL）
// ---------------------------------------------------------------------------

/// 递归收集会话 `.jsonl`（跳过符号链接；Pi 只接受普通文件）。
fn collect_pi_sessions(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_pi_sessions(&path, depth - 1, out);
        } else if file_type.is_file() && path.extension().is_some_and(|x| x == "jsonl") {
            out.push(path);
        }
    }
}

fn scan_pi(db: &mut Database, src: &DiscoveredSource) -> Result<SourceScanReport, IngestionError> {
    let mut report = base_report(src);
    let mut files: Vec<PathBuf> = Vec::new();
    collect_pi_sessions(&src.root, PI_MAX_DEPTH, &mut files);
    files.sort();

    let run_id = start_parse_run(db, &src.source_id, src.parser_version)?;
    for path in files {
        report.files_seen += 1;
        let file_identity = path.to_string_lossy().into_owned();
        let Some((size, mtime_secs)) = file_meta(&path) else {
            continue;
        };
        let cursor = read_cursor(db, &src.source_id, &file_identity, "byte_offset")?;
        // 事件 ID 含代数：截断/重写换代 + 退休旧代。
        let generation = match decide_action(cursor.as_ref(), size, mtime_secs) {
            FileAction::Skip => {
                report.files_unchanged += 1;
                continue;
            }
            FileAction::Scan {
                generation,
                retire_before,
            } => {
                if let Some(before) = retire_before {
                    let retired =
                        super::retire_file_generations(db, &src.source_id, &file_identity, before)?;
                    if retired > 0 {
                        report.diagnostics.push(format!(
                            "truncation_regen:{}: 退休旧代数事件 {retired} 条",
                            anon_id(&file_identity)
                        ));
                    }
                }
                generation
            }
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                report
                    .diagnostics
                    .push(format!("read_error:{}: {e}", anon_id(&file_identity)));
                continue;
            }
        };
        let cfg = PiParseConfig {
            source_id: src.source_id.clone(),
            file_identity: file_identity.clone(),
            generation,
            observed_at: observed_now(),
        };
        let out = pi_log::parse_session(&bytes, &cfg);
        report.diagnostics.extend(out.diagnostics.iter().cloned());
        report.files_scanned += 1;

        let batch = ImportBatch {
            source_id: src.source_id.clone(),
            events: out
                .events
                .into_iter()
                .map(|event| EventInput {
                    event,
                    replace_existing: false,
                })
                .collect(),
            sessions: session_upserts([out.session_id.clone()], false, false),
            cursor: Some(CursorUpdate {
                file_identity,
                generation,
                parser_version: src.parser_version.into(),
                cursor_kind: "byte_offset".into(),
                cursor_value: out.consumed_bytes.to_string(),
                tail_checksum: Some(format!("{size}:{mtime_secs}")),
            }),
        };
        commit_and_count(db, run_id, &batch, &mut report)?;
    }

    finish_parse_run(db, run_id, true, None)?;
    Ok(report)
}

// ---------------------------------------------------------------------------
// A07 MCode（只读 SQLite，row_id 游标）
// ---------------------------------------------------------------------------

fn scan_mcode(
    db: &mut Database,
    src: &DiscoveredSource,
) -> Result<SourceScanReport, IngestionError> {
    let mut report = base_report(src);
    let db_path = src.root.join("v2/sqlite/runtime-state.sqlite");
    let file_identity = db_path.to_string_lossy().into_owned();
    if !db_path.is_file() {
        report
            .diagnostics
            .push(format!("db_missing:{}", anon_id(&file_identity)));
        return Ok(report);
    }
    report.files_seen = 1;

    let conn = match mcode_db::open_read_only(&db_path) {
        Ok(c) => c,
        Err(e) => {
            report
                .diagnostics
                .push(format!("open_failed:{}: {e}", anon_id(&file_identity)));
            return Ok(report);
        }
    };
    let run_id = start_parse_run(db, &src.source_id, src.parser_version)?;

    // cursor_value = max_id（重叠回看由解析器的 overlap 处理，命中已存在 ID 不重复新增）。
    let cursor = read_cursor(db, &src.source_id, &file_identity, "row_id")?;
    let after_id = cursor.as_ref().map(|c| c.consumed_bytes);
    let cfg = McodeConfig::new(
        src.source_id.clone(),
        file_identity.clone(),
        0,
        observed_now(),
        after_id,
    );
    let out = mcode_db::read_events(&conn, &cfg);
    report.diagnostics.extend(out.diagnostics.iter().cloned());

    match out.status {
        McodeStatus::Unsupported(msg) => {
            // 表缺失/列结构不符：保留既有数据，不推进游标，下轮可重试。
            report.files_deferred += 1;
            report.diagnostics.push(format!(
                "mcode_unsupported:{}: {msg}",
                anon_id(&file_identity)
            ));
            finish_parse_run(db, run_id, true, None)?;
            return Ok(report);
        }
        McodeStatus::Deferred(msg) => {
            report.files_deferred += 1;
            report
                .diagnostics
                .push(format!("mcode_deferred:{}: {msg}", anon_id(&file_identity)));
            finish_parse_run(db, run_id, true, None)?;
            return Ok(report);
        }
        McodeStatus::Ok => {}
    }
    report.files_scanned += 1;

    let sessions = session_upserts(
        out.events.iter().map(|e| e.session_id.clone()),
        false,
        false,
    );
    let cursor_update = out.max_id.map(|max_id| CursorUpdate {
        file_identity,
        generation: 0,
        parser_version: src.parser_version.into(),
        cursor_kind: "row_id".into(),
        cursor_value: max_id.to_string(),
        tail_checksum: None,
    });
    let batch = ImportBatch {
        source_id: src.source_id.clone(),
        events: out
            .events
            .into_iter()
            .map(|event| EventInput {
                event,
                // 重叠回看可能重新读到已更新的行：覆盖校正，不新增。
                replace_existing: true,
            })
            .collect(),
        sessions,
        cursor: cursor_update,
    };
    commit_and_count(db, run_id, &batch, &mut report)?;

    finish_parse_run(db, run_id, true, None)?;
    Ok(report)
}

fn collect_rollouts(dir: &Path, max_depth: usize, out: &mut Vec<PathBuf>) {
    if !dir.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|ty| ty.is_symlink()) {
            continue;
        }
        if path.is_dir() {
            if max_depth > 0 {
                collect_rollouts(&path, max_depth - 1, out);
            }
        } else {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_rollout = name.starts_with("rollout-")
                && (name.ends_with(".jsonl") || name.ends_with(".jsonl.zst"));
            if is_rollout {
                out.push(path);
            }
        }
    }
}

fn read_rollout_bytes(path: &Path) -> std::io::Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    if path.extension().is_some_and(|e| e == "zst") {
        zstd::decode_all(&bytes[..])
    } else {
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_extraction() {
        assert_eq!(
            uuid_from_filename(
                "rollout-2026-10-11T01-00-00-000Z-11111111-2222-3333-4444-555555555555.jsonl"
            ),
            Some("11111111-2222-3333-4444-555555555555".to_string())
        );
        // 双段名取前段
        assert_eq!(
            uuid_from_filename("rollout-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee_ffffffff-0000-1111-2222-333333333333.jsonl"),
            Some("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string())
        );
        assert_eq!(uuid_from_filename("rollout-no-uuid.jsonl"), None);
    }

    #[test]
    fn action_decision() {
        // 未变化跳过
        let c = FileCursor {
            parser_version: "test".into(),
            generation: 0,
            consumed_bytes: 100,
            tail_checksum: Some("100:55".into()),
        };
        assert!(matches!(decide_action(Some(&c), 100, 55), FileAction::Skip));
        // 追加 → 同代继续
        assert!(matches!(
            decide_action(Some(&c), 200, 56),
            FileAction::Scan {
                generation: 0,
                retire_before: None
            }
        ));
        // 变小 → 截断，代+1 并退休旧代
        assert!(matches!(
            decide_action(Some(&c), 50, 57),
            FileAction::Scan {
                generation: 1,
                retire_before: Some(1)
            }
        ));
        // 无游标 → 全量
        assert!(matches!(
            decide_action(None, 100, 55),
            FileAction::Scan {
                generation: 0,
                retire_before: None
            }
        ));
    }
}
