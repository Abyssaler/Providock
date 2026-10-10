//! Kimi Code sessions live under `$KIMI_CODE_HOME/sessions/<workDirKey>/<sessionId>/`.
//!
//! `state.json` carries metadata (`title`, `lastPrompt`, `createdAt`,
//! `updatedAt`, `archived`, `forkedFrom`); the main agent's message stream is
//! `agents/main/wire.jsonl`, a JSONL record log whose
//! `agent.message.appended` entries wrap `{message: {role, content}}`.
//!
//! Deletion is intentionally unsupported here (mirrors the MCode provider):
//! a session is a directory with several indexes, not a single file.
use crate::session_manager::{SessionMessage, SessionMeta};
use serde_json::Value;
use std::path::{Path, PathBuf};

use super::utils::{extract_text, truncate_summary, TITLE_MAX_CHARS};

const PROVIDER_ID: &str = "kimicode";

pub(crate) fn sessions_dir() -> PathBuf {
    crate::kimicode_config::data_dir().join("sessions")
}

pub fn scan_sessions() -> Vec<SessionMeta> {
    let root = sessions_dir();
    if !root.exists() {
        return vec![];
    }
    let mut sessions = Vec::new();
    for workdir in read_dirs(&root) {
        for session_dir in read_dirs(&workdir) {
            if let Some(meta) = parse_session(&session_dir) {
                sessions.push(meta);
            }
        }
    }
    sessions
}

fn read_dirs(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

fn parse_session(dir: &Path) -> Option<SessionMeta> {
    let state_path = dir.join("state.json");
    let state: Value = serde_json::from_str(&std::fs::read_to_string(&state_path).ok()?).ok()?;

    if state.get("archived").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let session_id = state
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))?;

    // Title priority: auto/custom title > last prompt > nothing.
    let title = state
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .or_else(|| {
            state
                .get("lastPrompt")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|t| !t.is_empty())
        })
        .map(|t| truncate_summary(t, TITLE_MAX_CHARS));

    let wire = dir.join("agents").join("main").join("wire.jsonl");
    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id: session_id.clone(),
        title,
        summary: None,
        project_dir: state.get("cwd").and_then(Value::as_str).map(str::to_string),
        created_at: state.get("createdAt").and_then(Value::as_i64),
        last_active_at: state.get("updatedAt").and_then(Value::as_i64),
        source_path: Some(wire.to_string_lossy().to_string()),
        // Session ids are simple identifiers; only offer a resume command when
        // the id cannot inject extra flags into the command line.
        resume_command: session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
            .then(|| format!("kimi --session {session_id}")),
    })
}

pub fn load_messages(source_path: &str) -> Result<Vec<SessionMessage>, String> {
    read_messages(Path::new(source_path))
}

fn read_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "Failed to read Kimi Code session log {}: {e}",
            path.display()
        )
    })?;
    let mut messages = Vec::new();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("agent.message.appended") {
            continue;
        }
        let message = &value["message"];
        let content = &message["message"];
        let role = content
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if !matches!(role, "user" | "assistant") {
            continue;
        }
        let text_content = extract_text(&content["content"]);
        if text_content.trim().is_empty() {
            continue;
        }
        messages.push(SessionMessage {
            role: role.to_string(),
            content: text_content,
            ts: value.get("time").and_then(Value::as_i64),
        });
    }
    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_session(root: &Path, workdir: &str, id: &str, state: &str, wire: &str) -> PathBuf {
        let dir = root.join(workdir).join(id);
        std::fs::create_dir_all(dir.join("agents").join("main")).unwrap();
        std::fs::write(dir.join("state.json"), state).unwrap();
        std::fs::write(dir.join("agents").join("main").join("wire.jsonl"), wire).unwrap();
        dir
    }

    #[test]
    fn scan_reads_title_times_and_resume_command() {
        let temp = tempfile::tempdir().unwrap();
        let dir = write_session(
            temp.path(),
            "wd_code_abc",
            "session-1",
            r#"{"id":"session-1","cwd":"/work/code","createdAt":100,"updatedAt":200,"archived":false,"title":"Fix the bug","lastPrompt":"ignored"}"#,
            "",
        );
        let meta = parse_session(&dir).unwrap();
        assert_eq!(meta.provider_id, "kimicode");
        assert_eq!(meta.session_id, "session-1");
        assert_eq!(meta.title.as_deref(), Some("Fix the bug"));
        assert_eq!(meta.project_dir.as_deref(), Some("/work/code"));
        assert_eq!(meta.created_at, Some(100));
        assert_eq!(meta.last_active_at, Some(200));
        assert_eq!(
            meta.resume_command.as_deref(),
            Some("kimi --session session-1")
        );

        // Archived sessions are hidden; missing title falls back to lastPrompt.
        let dir = write_session(
            temp.path(),
            "wd_x",
            "session-2",
            r#"{"id":"session-2","archived":true,"title":"Hidden"}"#,
            "",
        );
        assert!(parse_session(&dir).is_none());
        let dir = write_session(
            temp.path(),
            "wd_x",
            "session-3",
            r#"{"id":"session-3","title":"","lastPrompt":"  hello world  "}"#,
            "",
        );
        assert_eq!(
            parse_session(&dir).unwrap().title.as_deref(),
            Some("hello world")
        );
    }

    #[test]
    fn load_messages_reads_user_and_assistant_text() {
        let temp = tempfile::tempdir().unwrap();
        let wire = temp.path().join("wire.jsonl");
        std::fs::write(
            &wire,
            concat!(
                "{\"type\":\"metadata\"}\n",
                "{\"type\":\"agent.message.appended\",\"message\":{\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"Fix it\"}]},\"meta\":{}},\"time\":100}\n",
                "{\"type\":\"agent.message.appended\",\"message\":{\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"think\",\"text\":\"...\"},{\"type\":\"text\",\"text\":\"Done\"}]},\"meta\":{}},\"time\":200}\n",
                "{\"type\":\"agent.message.appended\",\"message\":{\"message\":{\"role\":\"tool\",\"content\":[{\"type\":\"text\",\"text\":\"internal\"}]},\"meta\":{}},\"time\":300}\n",
            ),
        )
        .unwrap();

        let messages = read_messages(&wire).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Fix it");
        assert_eq!(messages[1].role, "assistant");
        assert!(messages[1].content.contains("Done"));
        assert_eq!(messages[1].ts, Some(200));
    }
}
