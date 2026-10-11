//! Codex rollout `*.jsonl` 正文读取器（T13）。
//!
//! 证据：docs/research/evidence/a01-a04-ccswitch-importers.md（cc-switch `df40d53c`，A02 节）
//! 与 docs/specs/AGENT_ADAPTERS.md 第 3 节 A02。每行 `{"timestamp","type","payload"}`。
//! 本层只取对话记录，忽略统计记录（`session_meta` / `turn_context` / `event_msg:token_count`）。
//!
//! 映射：
//! - `type == "response_item"`（payload.type 枚举）：
//!   - `message` → 按 `payload.role`（user→User / assistant→Assistant / system→System /
//!     其它→`Other`）；文本取 `payload.content`（数组里的 `input_text`/`output_text`/`text`）。
//!   - `function_call` / `custom_tool_call` → `ToolUse`，`tool_name = payload.name`，
//!     `text = payload.arguments`（或 `input`），`is_collapsible_tool = true`。
//!   - `function_call_output` / `custom_tool_call_output` → `ToolResult`，`is_collapsible_tool = true`。
//!   - `reasoning` → `Other("reasoning")`，文本取 `payload.summary` / `payload.content`。
//!   - 未知 `payload.type` → 跳过并记诊断 `unknown_response_item_type_at_offset`（分组诊断，
//!     不猜字段、不伪造正文）。
//! - `type == "event_msg"` 且 `payload.type == "user_message"` → `User`，文本取
//!   `payload.message` / `payload.text`；`agent_message` → `Assistant`。其余 event_msg 跳过。
//! - 时间取行级 `timestamp`（RFC3339 字符串，原样保留）。
//!
//! 统计解析器（`agents::codex_rollout`）只读 token_count；本层只读正文，两者独立。

use super::{diag_token, json_text, paginate, ContentBlock, ContentRole, MessagePage};
use serde_json::Value;

/// 读取一页 Codex rollout 正文（纯函数，不读文件系统、不访问网络与凭据）。
pub fn list_messages(bytes: &[u8], cursor: Option<u64>, limit: usize) -> MessagePage {
    let mut recognized = 0usize;
    let mut page = paginate(bytes, cursor, limit, Vec::new(), |line, offset, diags| {
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            diags.push(format!("malformed_line_at_offset:{offset}"));
            return Vec::new();
        };
        let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let time = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .map(str::to_string);
        let Some(payload) = v.get("payload") else {
            return Vec::new();
        };

        let blocks = match ty {
            "response_item" => response_item(payload, offset, &time, diags),
            "event_msg" => event_message(payload, offset, &time, diags),
            _ => Vec::new(),
        };
        if !blocks.is_empty() {
            recognized += 1;
        }
        blocks
    });

    if cursor.is_none() && recognized == 0 {
        page.diagnostics.push(
            "no_conversation_records: 没有 response_item 或用户输入记录".into(),
        );
    }
    page
}

fn response_item(
    payload: &Value,
    offset: u64,
    time: &Option<String>,
    diags: &mut Vec<String>,
) -> Vec<ContentBlock> {
    let ptype = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match ptype {
        "message" => {
            let role = codex_role(payload.get("role").and_then(|r| r.as_str()).unwrap_or(""));
            match payload.get("content").and_then(json_text) {
                Some(text) => vec![ContentBlock::new(role, Some(text), None, false, time.clone())],
                None => {
                    diags.push(format!("message_without_content_at_offset:{offset}"));
                    Vec::new()
                }
            }
        }
        "function_call" | "custom_tool_call" => {
            let name = payload
                .get("name")
                .and_then(|x| x.as_str())
                .map(str::to_string);
            let text = payload
                .get("arguments")
                .or_else(|| payload.get("input"))
                .and_then(json_text);
            if name.is_none() && text.is_none() {
                diags.push(format!("tool_call_without_body_at_offset:{offset}"));
                return Vec::new();
            }
            vec![ContentBlock::new(
                ContentRole::ToolUse,
                text,
                name,
                true,
                time.clone(),
            )]
        }
        "function_call_output" | "custom_tool_call_output" => {
            match payload.get("output").and_then(json_text) {
                Some(text) => vec![ContentBlock::new(
                    ContentRole::ToolResult,
                    Some(text),
                    None,
                    true,
                    time.clone(),
                )],
                None => {
                    diags.push(format!("tool_output_without_text_at_offset:{offset}"));
                    Vec::new()
                }
            }
        }
        "reasoning" => {
            let text = payload
                .get("summary")
                .or_else(|| payload.get("content"))
                .and_then(json_text);
            vec![ContentBlock::new(
                ContentRole::Other("reasoning".into()),
                text,
                None,
                false,
                time.clone(),
            )]
        }
        other => {
            diags.push(format!(
                "unknown_response_item_type_at_offset:{offset}:{}",
                diag_token(other)
            ));
            Vec::new()
        }
    }
}

fn event_message(
    payload: &Value,
    offset: u64,
    time: &Option<String>,
    diags: &mut Vec<String>,
) -> Vec<ContentBlock> {
    let ptype = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match ptype {
        "user_message" | "agent_message" => {
            let role = if ptype == "user_message" {
                ContentRole::User
            } else {
                ContentRole::Assistant
            };
            match payload
                .get("message")
                .or_else(|| payload.get("text"))
                .and_then(json_text)
            {
                Some(text) => vec![ContentBlock::new(role, Some(text), None, false, time.clone())],
                None => {
                    diags.push(format!(
                        "event_message_without_text_at_offset:{offset}:{}",
                        diag_token(ptype)
                    ));
                    Vec::new()
                }
            }
        }
        _ => Vec::new(),
    }
}

fn codex_role(s: &str) -> ContentRole {
    match s {
        "user" => ContentRole::User,
        "assistant" => ContentRole::Assistant,
        "system" => ContentRole::System,
        "" => ContentRole::Other("unknown".into()),
        other => ContentRole::Other(diag_token(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_codex_maps_response_items_and_user_message() {
        let data = concat!(
            "{\"timestamp\":\"T0\",\"type\":\"session_meta\",\"payload\":{\"id\":\"thr\"}}\n",
            "{\"timestamp\":\"T1\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"add a test\"}]}}\n",
            "{\"timestamp\":\"T2\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"sure\"}]}}\n",
            "{\"timestamp\":\"T3\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"shell\",\"arguments\":\"{\\\"cmd\\\":\\\"ls\\\"}\"}}\n",
            "{\"timestamp\":\"T4\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"output\":\"ok\"}}\n",
            "{\"timestamp\":\"T5\",\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"next\"}}\n",
        );
        let page = list_messages(data.as_bytes(), None, 200);
        let roles: Vec<String> = page.blocks.iter().map(|b| b.role.label()).collect();
        assert_eq!(
            roles,
            ["user", "assistant", "tool_use", "tool_result", "user"]
        );
        assert_eq!(page.blocks[2].tool_name.as_deref(), Some("shell"));
        assert!(page.blocks[2].is_collapsible_tool);
        assert!(page.blocks[3].is_collapsible_tool);
        assert_eq!(page.blocks[0].time.as_deref(), Some("T1"));
        assert!(!page.has_more);
    }

    #[test]
    fn content_codex_reasoning_and_unknown_type() {
        let data = concat!(
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"reasoning\",\"summary\":[{\"type\":\"summary_text\",\"text\":\"think\"}]}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"web_search_call\"}}\n",
        );
        let page = list_messages(data.as_bytes(), None, 200);
        assert_eq!(page.blocks.len(), 1);
        assert_eq!(page.blocks[0].role.label(), "other:reasoning");
        assert_eq!(page.blocks[0].text.as_deref(), Some("think"));
        assert!(page
            .diagnostics
            .iter()
            .any(|d| d.contains("unknown_response_item_type_at_offset")));
    }

    #[test]
    fn content_codex_reports_wrong_shape() {
        let data = "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thr\"}}\n";
        let page = list_messages(data.as_bytes(), None, 200);
        assert!(page.blocks.is_empty());
        assert!(page
            .diagnostics
            .iter()
            .any(|d| d.starts_with("no_conversation_records")));
    }
}
