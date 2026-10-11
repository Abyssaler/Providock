//! Kimi Code `wire.jsonl` 正文读取器（T13）。
//!
//! 证据：docs/research/evidence/a08-kimi-official.md（kimi-code `c9f01f07`）。
//! 记录信封 `{"type", ...payload, "time"}`（payload 展开在顶层，`time` 为 epoch 毫秒）。
//! 本层只取对话/人类记录；统计记录（usage.record、token_counting.* 等）跳过，不解析 usage
//! （统计是 `agents::kimi_wire` 的职责，两者记录形状一致但彼此独立）。
//!
//! 映射（记录名取自证据 §2.2 的 durable 清单与 §2.2 的 human 记录说明）：
//! - `turn.prompt`：用户输入 → `User`。文本取 `prompt` / `text` / `content` 首个可用字段。
//!   迁移产物里的 turn.prompt 可能只有 `turnId`（无正文）→ 记诊断 `turn_prompt_without_text_at_offset`，
//!   不伪造空正文，也不当作正文块。
//! - `context.append_message` / `agent.message.appended`，以及任意 `human.` 前缀记录：
//!   按 `role` 映射（user/assistant/tool/... ；未知角色 → `Other`）。证据未给出这两个记录
//!   正文与角色字段的完整 schema，这里按「payload 展开在顶层」取 `role` 与
//!   `content`/`text`/`message`；无文本且无工具名时跳过并记诊断，不产出空块。
//! - **回合边界**（`turn.ended` / `turn.cancel` / `agent.turn.*`）只作为分页/分组标记记入
//!   diagnostics（`turn_boundary_at_offset:*`），不是正文，不产出块。分页游标本就落在行首，
//!   调用方可据此在回合边界附近对齐展示；跨文件回合重建见 mod.rs 的 TODO。
//! - **undo / fork 标记**不是正文：`agent.switched` / `context.undo` 给一个 `System` 说明块
//!   「此处撤销」；`forked` 给 `System`「此处分叉」。undo 的一次写盘是三条记录
//!   （agent.switched + context.undo + context.undone，证据 §5.2），因此同组只出一个块，
//!   其余记诊断。跨页时该「同组」判断会重置（每页独立调用），见 mod.rs 说明。
//! - 其余记录（metadata / usage.record / token_counting.* / tools.* / llm.request 等）跳过。

use super::{diag_token, json_text, ms_to_rfc3339, paginate, ContentBlock, ContentRole, MessagePage};
use serde_json::Value;

/// 读取一页 Kimi wire 正文（纯函数，不读文件系统、不访问网络与凭据）。
pub fn list_messages(bytes: &[u8], cursor: Option<u64>, limit: usize) -> MessagePage {
    let mut recognized = 0usize;
    let mut undo_open = false;
    let mut page = paginate(bytes, cursor, limit, Vec::new(), |line, offset, diags| {
        if offset == 0 {
            let is_meta = serde_json::from_slice::<Value>(line)
                .ok()
                .and_then(|v| {
                    v.get("type")
                        .and_then(|t| t.as_str())
                        .map(|t| t == "metadata")
                })
                .unwrap_or(false);
            if !is_meta {
                diags.push("not_kimi_wire: 首行不是 metadata 记录".into());
            }
        }

        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            diags.push(format!("malformed_line_at_offset:{offset}"));
            return Vec::new();
        };
        let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match ty {
            "turn.prompt" => match first_text(&v, &["prompt", "text", "content"]) {
                Some(text) => {
                    recognized += 1;
                    vec![ContentBlock::new(
                        ContentRole::User,
                        Some(text),
                        None,
                        false,
                        record_time(&v),
                    )]
                }
                None => {
                    diags.push(format!("turn_prompt_without_text_at_offset:{offset}"));
                    Vec::new()
                }
            },
            "context.append_message" | "agent.message.appended" => {
                let blocks = message_block(&v, offset, diags);
                if !blocks.is_empty() {
                    recognized += 1;
                }
                blocks
            }
            "agent.switched" => {
                undo_open = true;
                vec![ContentBlock::new(
                    ContentRole::System,
                    Some("此处撤销（分支切换）".into()),
                    None,
                    false,
                    record_time(&v),
                )]
            }
            "context.undo" => {
                if undo_open {
                    diags.push(format!("undo_switch_duplicate_at_offset:{offset}"));
                    Vec::new()
                } else {
                    vec![ContentBlock::new(
                        ContentRole::System,
                        Some("此处撤销".into()),
                        None,
                        false,
                        record_time(&v),
                    )]
                }
            }
            "context.undone" => {
                undo_open = false;
                diags.push(format!("undo_complete_at_offset:{offset}"));
                Vec::new()
            }
            "forked" => {
                undo_open = false;
                vec![ContentBlock::new(
                    ContentRole::System,
                    Some("此处分叉：以上为从父会话复制的继承历史，不是本会话新增正文".into()),
                    None,
                    false,
                    record_time(&v),
                )]
            }
            "turn.ended" | "turn.cancel" | "agent.turn.started" | "agent.turn.ended" => {
                diags.push(format!("turn_boundary_at_offset:{offset}"));
                Vec::new()
            }
            other if other.starts_with("human.") => {
                let blocks = message_block(&v, offset, diags);
                if !blocks.is_empty() {
                    recognized += 1;
                }
                blocks
            }
            _ => Vec::new(),
        }
    });

    if cursor.is_none() && recognized == 0 {
        page.diagnostics.push(
            "no_conversation_records: wire 中没有 turn.prompt / context.append_message / human 记录"
                .into(),
        );
    }
    page
}

/// 从一条消息记录（context.append_message / human.*）构造块。
fn message_block(v: &Value, offset: u64, diags: &mut Vec<String>) -> Vec<ContentBlock> {
    let role_str = v
        .get("role")
        .and_then(|r| r.as_str())
        .or_else(|| {
            v.get("message")
                .and_then(|m| m.get("role"))
                .and_then(|r| r.as_str())
        })
        .unwrap_or("");
    let role = kimi_role(role_str);

    let tool_name = ["toolName", "tool_name", "name"]
        .iter()
        .find_map(|k| v.get(*k).and_then(|x| x.as_str()))
        .or_else(|| {
            v.get("message")
                .and_then(|m| m.get("name"))
                .and_then(|x| x.as_str())
        })
        .map(str::to_string);

    let text = first_text(v, &["content", "text", "message"]);
    if text.is_none() && tool_name.is_none() {
        diags.push(format!("message_without_text_at_offset:{offset}"));
        return Vec::new();
    }

    let collapsible = matches!(role, ContentRole::ToolUse | ContentRole::ToolResult);
    vec![ContentBlock::new(role, text, tool_name, collapsible, record_time(v))]
}

fn first_text(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| v.get(*k).and_then(json_text))
}

fn kimi_role(s: &str) -> ContentRole {
    match s {
        "user" => ContentRole::User,
        "assistant" => ContentRole::Assistant,
        "tool" | "tool_result" => ContentRole::ToolResult,
        "tool_use" => ContentRole::ToolUse,
        "system" => ContentRole::System,
        "" => ContentRole::Other("unknown".into()),
        other => ContentRole::Other(diag_token(other)),
    }
}

fn record_time(v: &Value) -> Option<String> {
    if let Some(ms) = v.get("time").and_then(|t| t.as_i64()) {
        return ms_to_rfc3339(ms);
    }
    v.get("timestamp")
        .and_then(|t| t.as_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_kimi_maps_turn_prompt_and_human_records() {
        let data = concat!(
            "{\"type\":\"metadata\",\"protocol_version\":\"1.5\"}\n",
            "{\"type\":\"turn.prompt\",\"agentId\":\"main\",\"turnId\":\"t1\",\"prompt\":\"hi\",\"time\":1760000000000}\n",
            "{\"type\":\"context.append_message\",\"role\":\"assistant\",\"content\":\"hello\",\"time\":1760000001000}\n",
            "{\"type\":\"context.append_message\",\"role\":\"tool\",\"toolName\":\"Bash\",\"content\":\"ok\",\"time\":1760000002000}\n",
            "{\"type\":\"agent.message.appended\",\"role\":\"assistant\",\"content\":\"human-mode\",\"time\":1760000003000}\n",
        );
        let page = list_messages(data.as_bytes(), None, 200);
        let roles: Vec<String> = page.blocks.iter().map(|b| b.role.label()).collect();
        assert_eq!(roles, ["user", "assistant", "tool_result", "assistant"]);
        assert_eq!(page.blocks[2].tool_name.as_deref(), Some("Bash"));
        assert!(page.blocks[2].is_collapsible_tool);
        assert!(page.blocks[0]
            .time
            .as_deref()
            .is_some_and(|t| t.starts_with("2025-10-09T")));
        assert!(!page.has_more);
    }

    #[test]
    fn content_kimi_marks_undo_and_fork_as_system_once() {
        let data = concat!(
            "{\"type\":\"metadata\",\"protocol_version\":\"1.5\"}\n",
            "{\"type\":\"turn.prompt\",\"agentId\":\"main\",\"turnId\":\"t1\",\"prompt\":\"u\",\"time\":1}\n",
            "{\"type\":\"forked\",\"agentId\":\"main\",\"time\":2}\n",
            "{\"type\":\"agent.switched\",\"agentId\":\"main\",\"time\":3}\n",
            "{\"type\":\"context.undo\",\"agentId\":\"main\",\"time\":4}\n",
            "{\"type\":\"context.undone\",\"agentId\":\"main\",\"time\":5}\n",
            "{\"type\":\"turn.ended\",\"agentId\":\"main\",\"turnId\":\"t1\",\"time\":6}\n",
        );
        let page = list_messages(data.as_bytes(), None, 200);
        let roles: Vec<String> = page.blocks.iter().map(|b| b.role.label()).collect();
        assert_eq!(roles, ["user", "system", "system"]);
        assert!(page.blocks[1].text.as_deref().unwrap().contains("分叉"));
        assert!(page.blocks[2].text.as_deref().unwrap().contains("撤销"));
        assert!(page
            .diagnostics
            .iter()
            .any(|d| d.starts_with("turn_boundary_at_offset:")));
    }

    #[test]
    fn content_kimi_reports_wrong_shape() {
        let data = "{\"type\":\"usage.record\",\"agentId\":\"main\",\"usage\":{}}\n";
        let page = list_messages(data.as_bytes(), None, 200);
        assert!(page.blocks.is_empty());
        assert!(page
            .diagnostics
            .iter()
            .any(|d| d.starts_with("no_conversation_records")));
    }
}
