//! 用量页 Agent 可见性：settings 表单键存 JSON 数组（`usage.visible_agents`）。
//! 键缺失或值损坏一律回退"全部可见"（读取永不失败）；写入前做白名单校验。

use crate::storage::Database;
use serde::{Deserialize, Serialize};

/// 用量页可选的 9 个 Agent，与采集适配器一一对应；顺序即默认展示顺序。
pub const AGENT_IDS: [&str; 9] = [
    "claude_code",
    "codex",
    "gemini_cli",
    "grok_build",
    "opencode",
    "pi",
    "mcode",
    "kimi_code",
    "deepseek_harness",
];

pub const SETTING_KEY: &str = "usage.visible_agents";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentVisibilityDto {
    pub visible_agents: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentVisibilityUpdate {
    pub visible_agents: Vec<String>,
}

pub fn all_visible() -> AgentVisibilityDto {
    AgentVisibilityDto {
        visible_agents: AGENT_IDS.iter().map(|id| (*id).to_owned()).collect(),
    }
}

/// 写入前校验：至少 1 个，且每个 ID 与白名单精确匹配。
pub fn validate(visible_agents: &[String]) -> Result<(), String> {
    if visible_agents.is_empty() {
        return Err("至少需要选择一个 Agent".into());
    }
    for id in visible_agents {
        if !AGENT_IDS.contains(&id.as_str()) {
            return Err(format!("未知的 Agent：{id}"));
        }
    }
    Ok(())
}

/// 存储值必须满足与写入相同的校验；否则视为损坏，回退全部可见。
fn parse_stored(raw: Option<String>) -> AgentVisibilityDto {
    let Some(raw) = raw else {
        return all_visible();
    };
    match serde_json::from_str::<Vec<String>>(&raw) {
        Ok(agents) if validate(&agents).is_ok() => AgentVisibilityDto {
            visible_agents: agents,
        },
        _ => all_visible(),
    }
}

pub fn read(db: &Database) -> AgentVisibilityDto {
    parse_stored(
        db.conn()
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [SETTING_KEY],
                |r| r.get::<_, String>(0),
            )
            .ok(),
    )
}

/// 通过 settings upsert 持久化；返回保存后的值。
pub fn write(db: &Database, visible_agents: &[String]) -> Result<AgentVisibilityDto, String> {
    validate(visible_agents)?;
    let value = serde_json::to_string(visible_agents).map_err(|e| e.to_string())?;
    db.conn()
        .execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![SETTING_KEY, value],
        )
        .map_err(|e| e.to_string())?;
    Ok(AgentVisibilityDto {
        visible_agents: visible_agents.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Database;

    fn set_raw(db: &Database, raw: &str) {
        db.conn()
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)",
                rusqlite::params![SETTING_KEY, raw],
            )
            .unwrap();
    }

    #[test]
    fn absent_setting_returns_all_eight_agents() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(read(&db), all_visible());
        assert_eq!(all_visible().visible_agents.len(), AGENT_IDS.len());
        assert_eq!(read(&db).visible_agents.len(), 9);
    }

    #[test]
    fn corrupt_or_invalid_stored_values_fall_back_to_all_agents() {
        let corrupt = [
            "",
            "not json",
            "123",
            "\"codex\"",
            "{\"agents\":[\"codex\"]}",
            "[]",
            "[1,2]",
            "[\"codex\",\"bogus\"]",
            "[\"CODEX\"]",
            "[\"codex\",\"\"]",
        ];
        for raw in corrupt {
            let db = Database::open_in_memory().unwrap();
            set_raw(&db, raw);
            assert_eq!(read(&db), all_visible(), "raw = {raw}");
        }
    }

    #[test]
    fn validation_rejects_empty_list_and_unknown_ids_with_clear_errors() {
        assert_eq!(validate(&[]).unwrap_err(), "至少需要选择一个 Agent");
        assert_eq!(
            validate(&["bogus".into()]).unwrap_err(),
            "未知的 Agent：bogus"
        );
        // 大小写与空串都必须精确匹配白名单
        assert_eq!(
            validate(&["codex".into(), "CODEX".into()]).unwrap_err(),
            "未知的 Agent：CODEX"
        );
        assert_eq!(validate(&["".into()]).unwrap_err(), "未知的 Agent：");
        assert!(validate(&["claude_code".into(), "mcode".into()]).is_ok());
    }

    #[test]
    fn saved_selection_roundtrips_and_upsert_replaces_previous_value() {
        let db = Database::open_in_memory().unwrap();
        let saved = write(&db, &["kimi_code".into(), "codex".into()]).unwrap();
        assert_eq!(saved.visible_agents, ["kimi_code", "codex"]);
        assert_eq!(read(&db).visible_agents, ["kimi_code", "codex"]);
        let stored: String = db
            .conn()
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [SETTING_KEY],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored, r#"["kimi_code","codex"]"#);
        write(&db, &["pi".into()]).unwrap();
        assert_eq!(read(&db).visible_agents, ["pi"]);
    }
}
