//! Import Kimi Code token usage from session wire logs without modifying them.
//!
//! Each session keeps per-agent JSONL record streams at
//! `$KIMI_CODE_HOME/sessions/<workDirKey>/<sessionId>/agents/<agentId>/wire.jsonl`.
//! `usage.record` lines carry `{model, usage: {inputOther, output,
//! inputCacheRead, inputCacheCreation}, time}` per turn. `inputOther` is the
//! fresh (non-cached) input, so rows are written with
//! `INPUT_TOKEN_SEMANTICS_FRESH`.
use crate::database::{lock_conn, Database};
use crate::error::AppError;
use crate::proxy::usage::calculator::CostCalculator;
use crate::proxy::usage::parser::TokenUsage;
use crate::services::session_usage::{
    load_sync_cursors, metadata_modified_nanos, update_sync_state_on_conn, SessionSyncResult,
};
use crate::services::sql_helpers::INPUT_TOKEN_SEMANTICS_FRESH;
use crate::services::usage_stats::find_model_pricing;
use rusqlite::params;
use rust_decimal::Decimal;
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub fn sync_kimicode_usage(db: &Database) -> Result<SessionSyncResult, AppError> {
    let root = crate::session_manager::providers::kimicode::sessions_dir();
    if !root.exists() {
        return Ok(SessionSyncResult::default());
    }
    let mut result = SessionSyncResult::default();
    for file in collect_wire_logs(&root) {
        match sync_wire_log(db, &file) {
            Ok(part) => result.merge(part),
            Err(error) => result.errors.push(format!("{}: {error}", file.display())),
        }
    }
    Ok(result)
}

fn collect_wire_logs(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()) == Some("wire.jsonl") {
                files.push(path);
            }
        }
    }
    files
}

fn session_and_agent(path: &Path) -> (String, String) {
    // .../sessions/<workDirKey>/<sessionId>/agents/<agentId>/wire.jsonl
    let agent = path
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "main".into());
    let session = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".into());
    (session, agent)
}

fn sync_wire_log(db: &Database, path: &Path) -> Result<SessionSyncResult, AppError> {
    let mut result = SessionSyncResult::default();
    let key = path.to_string_lossy().into_owned();
    let metadata = std::fs::metadata(path).map_err(|e| AppError::io(path, e))?;
    let modified = metadata_modified_nanos(&metadata);
    let cursors = load_sync_cursors(db)?;
    let cursor = cursors.get(&key).copied().unwrap_or_default();
    if cursor.last_modified == modified {
        return Ok(result);
    }

    let file = std::fs::File::open(path).map_err(|e| AppError::io(path, e))?;
    let reader = BufReader::new(file);
    let (session, agent) = session_and_agent(path);

    let mut rows: Vec<(String, String, TokenUsage, i64)> = Vec::new();
    let mut line_no = 0i64;
    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        line_no += 1;
        if line_no <= cursor.last_line_offset {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("usage.record") {
            continue;
        }
        let Some(usage) = value.get("usage").filter(|u| u.is_object()) else {
            continue;
        };
        let get = |name: &str| {
            usage
                .get(name)
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(u32::MAX as u64) as u32
        };
        let tokens = TokenUsage {
            input_tokens: get("inputOther"),
            output_tokens: get("output"),
            cache_read_tokens: get("inputCacheRead"),
            cache_creation_tokens: get("inputCacheCreation"),
            model: value
                .get("model")
                .and_then(Value::as_str)
                .filter(|m| !m.is_empty())
                .map(str::to_string),
            message_id: None,
        };
        let created_at = value.get("time").and_then(Value::as_i64).unwrap_or(0) / 1000;
        let request_id = format!("kimicode:{session}:{agent}:{line_no}");
        rows.push((request_id, session.clone(), tokens, created_at));
    }

    let mut conn = lock_conn!(db.conn);
    let tx = conn.transaction()?;
    result.files_scanned = 1;
    for (request_id, session, tokens, created_at) in rows {
        let model = tokens.model.clone().unwrap_or_else(|| "unknown".into());
        let cost = find_model_pricing(&tx, &model)
            .map(|pricing| {
                CostCalculator::calculate_for_app("kimicode", &tokens, &pricing, Decimal::ONE)
                    .total_cost
                    .to_string()
            })
            .unwrap_or_else(|| "0".into());
        let changed = tx.execute(
            "INSERT OR IGNORE INTO proxy_request_logs (
                request_id, provider_id, app_type, model, request_model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd,
                total_cost_usd, latency_ms, status_code, session_id, provider_type,
                is_streaming, cost_multiplier, created_at, data_source, input_token_semantics
             ) VALUES (?1, '_kimicode_session', 'kimicode', ?2, ?2, ?3, ?4, ?5, ?6,
                       '0', '0', '0', '0', ?7, 0, 200, ?8, 'kimicode_session', 1, '1', ?9,
                       'kimicode_session', ?10)",
            params![
                request_id,
                model,
                tokens.input_tokens,
                tokens.output_tokens,
                tokens.cache_read_tokens,
                tokens.cache_creation_tokens,
                cost,
                session,
                created_at,
                INPUT_TOKEN_SEMANTICS_FRESH
            ],
        )?;
        result.imported += changed as u32;
        result.skipped += u32::from(changed == 0);
    }
    // Commit the cursor with the rows; truncated files must not be reimported.
    update_sync_state_on_conn(&tx, &key, modified, line_no)?;
    tx.commit()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_log_usage_import_is_incremental() -> Result<(), AppError> {
        let dir = tempfile::tempdir().unwrap();
        let wire = dir
            .path()
            .join("wd_x")
            .join("session-1")
            .join("agents")
            .join("main");
        std::fs::create_dir_all(&wire).unwrap();
        let wire = wire.join("wire.jsonl");
        std::fs::write(
            &wire,
            concat!(
                "{\"type\":\"metadata\"}\n",
                "{\"type\":\"usage.record\",\"agentId\":\"main\",\"model\":\"kimi-code/k3\",\"usage\":{\"inputOther\":100,\"output\":20,\"inputCacheRead\":300,\"inputCacheCreation\":40},\"usageScope\":\"turn\",\"time\":1700000000000}\n",
            ),
        )
        .unwrap();

        let db = Database::memory().unwrap();
        let result = sync_wire_log(&db, &wire).unwrap();
        assert_eq!(result.imported, 1);

        let conn = lock_conn!(db.conn);
        let row: (i64, i64, i64, i64, String, i64) = conn
            .query_row(
                "SELECT input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                         model, input_token_semantics
                 FROM proxy_request_logs WHERE app_type = 'kimicode'",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row.0, 100);
        assert_eq!(row.1, 20);
        assert_eq!(row.2, 300);
        assert_eq!(row.3, 40);
        assert_eq!(row.4, "kimi-code/k3");
        assert_eq!(row.5, INPUT_TOKEN_SEMANTICS_FRESH);
        drop(conn);

        // Second run with an unchanged file imports nothing; a new record is
        // picked up after append.
        assert_eq!(sync_wire_log(&db, &wire).unwrap().imported, 0);
        std::fs::write(
            &wire,
            concat!(
                "{\"type\":\"metadata\"}\n",
                "{\"type\":\"usage.record\",\"agentId\":\"main\",\"model\":\"kimi-code/k3\",\"usage\":{\"inputOther\":100,\"output\":20,\"inputCacheRead\":300,\"inputCacheCreation\":40},\"usageScope\":\"turn\",\"time\":1700000000000}\n",
                "{\"type\":\"usage.record\",\"agentId\":\"main\",\"model\":\"kimi-code/kimi-for-coding\",\"usage\":{\"inputOther\":1,\"output\":2,\"inputCacheRead\":0,\"inputCacheCreation\":0},\"usageScope\":\"turn\",\"time\":1700000001000}\n",
            ),
        )
        .unwrap();
        assert_eq!(sync_wire_log(&db, &wire).unwrap().imported, 1);
        Ok(())
    }
}
