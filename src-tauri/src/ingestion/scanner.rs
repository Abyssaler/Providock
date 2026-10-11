//! 源发现与增量扫描（A02 Codex、A08 Kimi 新版）。
//!
//! 路径规则（AGENT_ADAPTERS 第 2 节 + 证据文档）：
//! - Kimi：`KIMI_CODE_HOME` > `~/.kimi-code`；会话在 `sessions/<wdKey>/<sessionId>/`，
//!   wire 在 `agents/<agentId>/wire.jsonl`；`state.json` 的 forkedFrom 判定继承。
//! - Codex：`CODEX_HOME` > `~/.codex`；`sessions/YYYY/MM/DD/*.jsonl[.zst]` 与
//!   `archived_sessions/*.jsonl[.zst]`；同名 `.zst` 与 `.jsonl` 并存时只读普通形态。
//!
//! 游标与代：
//! - cursor_value = 已消费字节数，tail_checksum = "{size}:{mtime_secs}"，一致则跳过。
//! - 文件变小（size < 已消费字节）视为截断/重写：代 +1 全量重扫，
//!   旧代数事件同事务退休（墓碑保留身份，防止三度计入）。
//! - 图形界面可能不继承终端环境变量：找不到默认目录时返回空发现列表，由 UI 引导添加。

use super::{
    commit_batch, finish_parse_run, start_parse_run, CursorUpdate, EventInput, ImportBatch,
    IngestionError, SessionUpsert,
};
use crate::adapters::agents::codex_rollout::{self, CodexParseConfig, CodexStatus, EventSignature};
use crate::adapters::agents::kimi_wire::{self, WireParseConfig};
use crate::domain::AgentKind;
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

#[derive(Debug, Serialize)]
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
    out
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
        _ => unreachable!("scanner not implemented for this agent"),
    }
}

struct FileCursor {
    generation: u64,
    consumed_bytes: u64,
    tail_checksum: Option<String>,
}

fn read_cursor(
    db: &Database,
    source_id: &str,
    file_identity: &str,
) -> Result<Option<FileCursor>, IngestionError> {
    let mut stmt = db.conn().prepare(
        "SELECT generation, cursor_value, tail_checksum FROM ingestion_cursors
         WHERE source_id = ?1 AND file_identity = ?2 AND cursor_kind = 'byte_offset'
         ORDER BY generation DESC LIMIT 1",
    )?;
    let mut rows = stmt.query(rusqlite::params![source_id, file_identity])?;
    Ok(if let Some(r) = rows.next()? {
        let value: String = r.get(1)?;
        Some(FileCursor {
            generation: r.get::<_, i64>(0)? as u64,
            consumed_bytes: value.parse().unwrap_or(0),
            tail_checksum: r.get(2)?,
        })
    } else {
        None
    })
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
            let cursor = read_cursor(db, &src.source_id, &file_identity)?;
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
                                "truncation_regen:{file_identity}: 退休旧代数事件 {retired} 条"
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
                        .push(format!("read_error:{file_identity}: {e}"));
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
        let cursor = read_cursor(db, &src.source_id, &file_identity)?;
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
                            "truncation_regen:{file_identity}: 退休旧代数事件 {retired} 条"
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
                    .push(format!("read_error:{file_identity}: {e}"));
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
        let timeline = parent_id.as_ref().and_then(|p| timelines.get(p)).cloned();
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
                    pending.file_identity,
                    parent_id.unwrap_or_default()
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

    finish_parse_run(db, run_id, true, None)?;
    Ok(report)
}

fn collect_rollouts(dir: &Path, max_depth: usize, out: &mut Vec<PathBuf>) {
    if max_depth == 0 || !dir.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rollouts(&path, max_depth - 1, out);
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
