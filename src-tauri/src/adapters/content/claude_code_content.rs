//! Claude Code `projects/**/*.jsonl` 正文读取器（T13）。
//!
//! 证据：docs/research/evidence/a01-a04-ccswitch-importers.md（cc-switch `df40d53c`，A01 节）
//! 与 docs/specs/AGENT_ADAPTERS.md 第 3 节 A01。
//! 每行一个对象；会话消息行为 `type == "user" | "assistant"`，正文在 `message.content`。
//!
//! 映射（UI_UX 第 6 节：工具块折叠）：
//! - `content` 是字符串 → 按消息角色出一个块。
//! - `content` 是数组：逐块映射
//!   - `{type:"text", text}` → 按消息角色（user→User / assistant→Assistant）；
//!   - `{type:"tool_use", name, input}` → `ToolUse`，`tool_name = name`，`text = input` 的
//!     JSON 序列化（仅供查看，工具内容不可点击执行），`is_collapsible_tool = true`；
//!   - `{type:"tool_result", content, tool_name?}` → `ToolResult`，`is_collapsible_tool = true`；
//!   - 其它块类型（如 `thinking`）→ `Other(<type>)`，文本尽力提取，并记诊断，不丢弃。
//! - 其它行（system / summary 等）跳过；空 content 或无块数组记诊断。
//! - 时间取行级 `timestamp`（RFC3339 字符串，原样保留）。
//!
//! 统计解析器（`agents::claude_code`）只读 assistant 行的 usage；本层只读正文，两者独立。

use super::{diag_token, json_text, paginate, ContentBlock, ContentRole, MessagePage};
use serde_json::Value;

/// 读取一页 Claude Code 会话正文（纯函数，不读文件系统、不访问网络与凭据）。
pub fn list_messages(bytes: &[u8], cursor: Option<u64>, limit: usize) -> MessagePage {
    let mut recognized = 0usize;
    let mut page = paginate(bytes, cursor, limit, Vec::new(), |line, offset, diags| {
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            diags.push(format!("malformed_line_at_offset:{offset}"));
            return Vec::new();
        };
        let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if ty != "user" && ty != "assistant" {
            return Vec::new();
        }
        recognized += 1;

        let Some(message) = v.get("message") else {
            diags.push(format!("message_field_missing_at_offset:{offset}"));
            return Vec::new();
        };
        let role_str = message.get("role").and_then(|r| r.as_str()).unwrap_or(ty);
        let role = claude_role(role_str);
        let time = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .map(str::to_string);
        let Some(content) = message.get("content") else {
            diags.push(format!("content_missing_at_offset:{offset}"));
            return Vec::new();
        };
        content_blocks(content, &role, offset, &time, diags)
    });

    if cursor.is_none() && recognized == 0 {
        page.diagnostics
            .push("no_conversation_records: 没有 type=user/assistant 的消息行".into());
    }
    page
}

fn content_blocks(
    content: &Value,
    role: &ContentRole,
    offset: u64,
    time: &Option<String>,
    diags: &mut Vec<String>,
) -> Vec<ContentBlock> {
    match content {
        Value::String(s) => vec![ContentBlock::new(
            role.clone(),
            Some(s.clone()),
            None,
            false,
            time.clone(),
        )],
        Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                match item {
                    Value::String(s) => out.push(ContentBlock::new(
                        role.clone(),
                        Some(s.clone()),
                        None,
                        false,
                        time.clone(),
                    )),
                    Value::Object(_) => {
                        let btype = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match btype {
                            "text" => {
                                if let Some(text) = item.get("text").and_then(json_text) {
                                    out.push(ContentBlock::new(
                                        role.clone(),
                                        Some(text),
                                        None,
                                        false,
                                        time.clone(),
                                    ));
                                }
                            }
                            "tool_use" => {
                                let name = item
                                    .get("name")
                                    .and_then(|x| x.as_str())
                                    .map(str::to_string);
                                let text = item
                                    .get("input")
                                    .filter(|x| !x.is_null())
                                    .map(|x| x.to_string());
                                out.push(ContentBlock::new(
                                    ContentRole::ToolUse,
                                    text,
                                    name,
                                    true,
                                    time.clone(),
                                ));
                            }
                            "tool_result" => {
                                let text = item.get("content").and_then(json_text);
                                let name = item
                                    .get("tool_name")
                                    .and_then(|x| x.as_str())
                                    .map(str::to_string);
                                out.push(ContentBlock::new(
                                    ContentRole::ToolResult,
                                    text,
                                    name,
                                    true,
                                    time.clone(),
                                ));
                            }
                            other => {
                                let text = json_text(item);
                                diags.push(format!(
                                    "other_content_block_at_offset:{offset}:{}",
                                    diag_token(other)
                                ));
                                out.push(ContentBlock::new(
                                    ContentRole::Other(if other.is_empty() {
                                        "unknown".into()
                                    } else {
                                        diag_token(other)
                                    }),
                                    text,
                                    None,
                                    false,
                                    time.clone(),
                                ));
                            }
                        }
                    }
                    _ => {}
                }
            }
            if out.is_empty() {
                diags.push(format!("content_without_blocks_at_offset:{offset}"));
            }
            out
        }
        _ => {
            diags.push(format!("unsupported_content_at_offset:{offset}"));
            Vec::new()
        }
    }
}

fn claude_role(s: &str) -> ContentRole {
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
    fn content_claude_maps_text_tool_use_and_tool_result() {
        let data = concat!(
            "{\"type\":\"user\",\"timestamp\":\"T1\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"T2\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"sure\"},{\"type\":\"tool_use\",\"id\":\"tu\",\"name\":\"Read\",\"input\":{\"path\":\"a.rs\"}}]}}\n",
            "{\"type\":\"user\",\"timestamp\":\"T3\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"tu\",\"content\":\"file body\"}]}}\n",
        );
        let page = list_messages(data.as_bytes(), None, 200);
        let roles: Vec<String> = page.blocks.iter().map(|b| b.role.label()).collect();
        assert_eq!(roles, ["user", "assistant", "tool_use", "tool_result"]);
        assert_eq!(page.blocks[2].tool_name.as_deref(), Some("Read"));
        assert!(page.blocks[2].is_collapsible_tool);
        assert!(page.blocks[3].is_collapsible_tool);
        assert!(page.blocks[2].text.as_deref().unwrap().contains("a.rs"));
        assert_eq!(page.blocks[0].time.as_deref(), Some("T1"));
        assert!(!page.has_more);
    }

    #[test]
    fn content_claude_unknown_block_becomes_other_with_diagnostic() {
        let data = "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"thinking\",\"thinking\":\"hmm\"}]}}\n";
        let page = list_messages(data.as_bytes(), None, 200);
        assert_eq!(page.blocks.len(), 1);
        assert_eq!(page.blocks[0].role.label(), "other:thinking");
        assert_eq!(page.blocks[0].text.as_deref(), Some("hmm"));
        assert!(page
            .diagnostics
            .iter()
            .any(|d| d.contains("other_content_block_at_offset")));
    }

    #[test]
    fn content_claude_reports_wrong_shape_and_malformed() {
        let data = "{\"type\":\"summary\",\"summary\":\"x\"}\nnot-json\n";
        let page = list_messages(data.as_bytes(), None, 200);
        assert!(page.blocks.is_empty());
        assert!(page
            .diagnostics
            .iter()
            .any(|d| d.starts_with("no_conversation_records")));
        assert!(page
            .diagnostics
            .iter()
            .any(|d| d.starts_with("malformed_line_at_offset:")));
    }
}
