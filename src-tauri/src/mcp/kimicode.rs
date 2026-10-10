//! Kimi Code MCP sync (`$KIMI_CODE_HOME/mcp.json`).
//!
//! Native entries: stdio `{command, args, env, cwd}`, HTTP `{url, headers}`,
//! SSE `{transport: "sse", url, headers}`, plus `enabled`,
//! `startupTimeoutMs` / `toolTimeoutMs` and other per-server options which are
//! preserved across syncs.
use crate::app_config::{McpApps, McpServer};
use crate::config::atomic_write_private;
use crate::error::AppError;
use crate::store::AppState;
use serde_json::{json, Value};
use std::{fs, path::Path, sync::Mutex};

static WRITE_LOCK: Mutex<()> = Mutex::new(());
const TRANSPORT_FIELDS: [&str; 8] = [
    "command",
    "args",
    "env",
    "cwd",
    "url",
    "headers",
    "type",
    "transport",
];

fn mcp_path() -> std::path::PathBuf {
    crate::kimicode_config::data_dir().join("mcp.json")
}

fn read(path: &Path) -> Result<Value, AppError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(json!({"mcpServers": {}})),
        Err(e) => return Err(AppError::io(path, e)),
    };
    let value: Value = serde_json::from_str(&text)
        .map_err(|_| AppError::Config("Invalid Kimi Code MCP JSON".into()))?;
    if !value.is_object() || value.get("mcpServers").is_some_and(|v| !v.is_object()) {
        return Err(AppError::Config(
            "Invalid Kimi Code mcpServers object".into(),
        ));
    }
    Ok(value)
}

pub fn sync(id: &str, spec: Option<&Value>) -> Result<(), AppError> {
    let _guard = WRITE_LOCK
        .lock()
        .map_err(|e| AppError::Message(e.to_string()))?;
    sync_file(&mcp_path(), id, spec)
}

pub(crate) fn sync_and_commit<T>(
    id: &str,
    spec: Option<&Value>,
    commit: impl FnOnce() -> Result<T, AppError>,
) -> Result<T, AppError> {
    let _guard = WRITE_LOCK
        .lock()
        .map_err(|e| AppError::Message(e.to_string()))?;
    let path = mcp_path();
    crate::mcode_config::write_and_commit(&path, || sync_file(&path, id, spec), commit)
}

fn sync_file(path: &Path, id: &str, spec: Option<&Value>) -> Result<(), AppError> {
    let mut document = read(path)?;
    if document.get("mcpServers").is_none() {
        document["mcpServers"] = json!({});
    }
    let servers = document["mcpServers"].as_object_mut().unwrap();
    if let Some(spec) = spec {
        super::validation::validate_server_spec(&unified_spec(spec))?;
        let mut merged = servers.get(id).cloned().unwrap_or_else(|| json!({}));
        let object = merged
            .as_object_mut()
            .ok_or_else(|| AppError::Config("Invalid Kimi Code MCP entry".into()))?;
        for field in TRANSPORT_FIELDS {
            object.remove(field);
        }
        object.extend(native_spec(spec).as_object().unwrap().clone());
        object.insert("enabled".into(), json!(true));
        servers.insert(id.into(), merged);
    } else {
        servers.remove(id);
    }
    atomic_write_private(
        path,
        serde_json::to_string_pretty(&document)
            .map_err(|e| AppError::Config(e.to_string()))?
            .as_bytes(),
    )
}

pub fn import(state: &AppState) -> Result<usize, AppError> {
    let document = read(&mcp_path())?;
    let mut existing = state.db.get_all_mcp_servers()?;
    let mut count = 0;
    let mut skipped = Vec::new();
    for (id, native) in document["mcpServers"].as_object().into_iter().flatten() {
        let mut spec = unified_spec(native);
        if super::validation::validate_server_spec(&spec).is_err() {
            skipped.push(format!("'{id}': invalid transport configuration"));
            continue;
        }
        let enabled = native
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        spec.as_object_mut().unwrap().remove("enabled");
        let server = if let Some(mut server) = existing.shift_remove(id) {
            if transport_spec(&server.server) != transport_spec(&spec) {
                skipped.push(format!("'{id}': conflicts with an existing server"));
                continue;
            }
            server.apps.kimicode = enabled;
            server
        } else {
            count += 1;
            McpServer {
                id: id.clone(),
                name: id.clone(),
                server: spec,
                apps: McpApps {
                    kimicode: enabled,
                    ..Default::default()
                },
                description: None,
                homepage: None,
                docs: None,
                tags: vec![],
            }
        };
        state.db.save_mcp_server(&server)?;
    }
    if skipped.is_empty() {
        Ok(count)
    } else {
        Err(AppError::InvalidInput(format!(
            "Imported {count} Kimi Code MCP servers; skipped {}. Native configurations were preserved.",
            skipped.join("; ")
        )))
    }
}

fn transport_spec(spec: &Value) -> Value {
    let mut spec = unified_spec(spec);
    if let Some(object) = spec.as_object_mut() {
        object.retain(|key, _| {
            matches!(
                key.as_str(),
                "command" | "args" | "env" | "cwd" | "url" | "headers" | "type"
            )
        });
    }
    spec
}

/// Normalize any accepted shape to the unified `{type: stdio|http|sse, ...}` spec.
fn unified_spec(native: &Value) -> Value {
    let mut spec = native.clone();
    if !spec.is_object() {
        return spec;
    }
    let is_sse = spec.get("transport").and_then(Value::as_str) == Some("sse")
        || spec.get("type").and_then(Value::as_str) == Some("sse");
    let has_url = spec.get("url").is_some();
    let has_command = spec.get("command").is_some();
    if is_sse && has_url {
        spec["type"] = json!("sse");
    } else if spec.get("type").and_then(Value::as_str) == Some("streamable-http")
        || (spec.get("type").is_none() && has_url)
    {
        spec["type"] = json!("http");
    } else if spec.get("type").is_none() && has_command {
        spec["type"] = json!("stdio");
    }
    spec.as_object_mut().unwrap().remove("transport");
    spec
}

/// Map the unified spec to Kimi Code's native mcp.json shape.
fn native_spec(spec: &Value) -> Value {
    let unified = unified_spec(spec);
    let mut native = unified.clone();
    let object = native.as_object_mut().unwrap();
    match unified.get("type").and_then(Value::as_str) {
        Some("sse") => {
            object.remove("type");
            object.insert("transport".into(), json!("sse"));
        }
        // HTTP is the default transport for url entries; stdio carries no type.
        _ => {
            object.remove("type");
        }
    }
    native
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_spec_maps_transports() {
        assert_eq!(
            native_spec(&json!({"type": "stdio", "command": "node", "args": ["s.js"]})),
            json!({"command": "node", "args": ["s.js"]})
        );
        assert_eq!(
            native_spec(
                &json!({"type": "http", "url": "https://example.com/mcp", "headers": {"A": "b"}})
            ),
            json!({"url": "https://example.com/mcp", "headers": {"A": "b"}})
        );
        assert_eq!(
            native_spec(&json!({"type": "sse", "url": "https://example.com/sse"})),
            json!({"url": "https://example.com/sse", "transport": "sse"})
        );
        // Native shapes round-trip through the unified form.
        assert_eq!(
            unified_spec(&json!({"url": "https://example.com/mcp"}))["type"],
            json!("http")
        );
        assert_eq!(
            unified_spec(&json!({"transport": "sse", "url": "https://example.com/sse"}))["type"],
            json!("sse")
        );
        assert_eq!(
            unified_spec(&json!({"command": "node"}))["type"],
            json!("stdio")
        );
    }

    #[test]
    fn sync_preserves_unmanaged_entries_and_timeouts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        let original = json!({"mcpServers": {"other": {"url": "https://example.com/mcp"}, "edit": {"command": "old", "startupTimeoutMs": 5000}}});
        fs::write(&path, original.to_string()).unwrap();

        sync_file(
            &path,
            "edit",
            Some(&json!({"type": "sse", "url": "https://example.com/sse", "headers": {"X": "y"}})),
        )
        .unwrap();
        let written = read(&path).unwrap();
        assert_eq!(
            written["mcpServers"]["other"],
            original["mcpServers"]["other"]
        );
        assert_eq!(written["mcpServers"]["edit"]["startupTimeoutMs"], 5000);
        assert_eq!(written["mcpServers"]["edit"]["transport"], "sse");
        assert_eq!(written["mcpServers"]["edit"]["enabled"], true);
        assert!(written["mcpServers"]["edit"].get("command").is_none());

        sync_file(&path, "edit", None).unwrap();
        assert!(read(&path).unwrap()["mcpServers"].get("edit").is_none());

        fs::write(&path, "broken json").unwrap();
        assert!(sync_file(&path, "new", Some(&json!({"command": "node"}))).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "broken json");
    }
}
