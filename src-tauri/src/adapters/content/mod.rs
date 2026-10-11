//! 可选正文读取器：按源文件按需读取，不复制进数据库（SECURITY_PRIVACY 第 1/5 节）。
//!
//! 本层是**纯读取层**：`list_messages(bytes, cursor, limit)` 只接收文件字节、字节游标与
//! 每页上限，不读文件系统、不访问网络与凭据、不产生任何写操作，也不解析 usage 做统计
//! （统计是 `adapters::agents::*` 解析器的职责；本层与它们记录形状一致但彼此独立）。
//!
//! 分页（SECURITY_PRIVACY 第 1/5 节 / UI_UX 第 6 节）：
//! - `cursor` 是**字节偏移**，指向下一页第一条未消费记录的起始位置（`None` 表示从头读）。
//! - 页按**行原子**切分：一条记录产出的所有块要么全在本页，要么整条留给下一页，不拆行。
//!   因此 `next_cursor` 始终落在行首（=下一条完整记录的起点），且严格大于传入游标。
//! - `limit` 会被钳制到 `[1, MAX_PAGE_LIMIT]`。为保证前进，若某条记录本身产出的块数超过
//!   `limit` 且本页尚未产出任何块，仍会整条输出该记录（唯一的越界情形，见测试）。
//! - 未写完的尾行（无结尾换行）不消费；空输入与越界游标都给诊断而非崩溃。
//!
//! 不可信输入（SECURITY_PRIVACY 第 1/5 节）：正文只当数据，长度截断到 `MAX_TEXT_CHARS`
//! 并置 `truncated`；控制字符（保留 `\n`/`\t`）剔除；诊断只写偏移与短码，绝不写入整条
//! 原始 JSON、正文或本机路径。
//!
//! TODO(T13+): 跨文件活跃链重建（DATA_CONTRACTS 第 4 节的 fork 活跃对话链）不在本层。
//! 本层只提供「文件内顺序 + forked/undo 标记」这一层可见性；把 fork 的继承段归并到父
//! 会话、或重建可恢复链，需要跨文件上下文，由后续任务实现。

pub mod claude_code_content;
pub mod codex_content;
pub mod kimi_content;

use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// 单块正文最大字符数（Unicode 标量值计数）；超出即截断并置 `truncated = true`。
/// 这是可调策略常量：长度限制用于防止超长/恶意正文拖垮查看，不代表正文就该这么短。
pub const MAX_TEXT_CHARS: usize = 4000;

/// 单页最大块数；`list_messages` 的 `limit` 会被钳制到 `[1, MAX_PAGE_LIMIT]`。
pub const MAX_PAGE_LIMIT: usize = 200;

/// 正文块角色。未知角色/块类型归入 `Other(name)`，既不丢失也不伪造。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentRole {
    User,
    Assistant,
    ToolUse,
    ToolResult,
    System,
    Other(String),
}

impl ContentRole {
    /// 稳定标签，用于诊断与测试断言（`Other` 形如 `other:<name>`）。
    pub fn label(&self) -> String {
        match self {
            ContentRole::User => "user".into(),
            ContentRole::Assistant => "assistant".into(),
            ContentRole::ToolUse => "tool_use".into(),
            ContentRole::ToolResult => "tool_result".into(),
            ContentRole::System => "system".into(),
            ContentRole::Other(name) => format!("other:{name}"),
        }
    }
}

/// 统一内容块。`text` 已经过控制字符剔除与长度截断（见 [`ContentBlock::new`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentBlock {
    pub role: ContentRole,
    pub text: Option<String>,
    pub tool_name: Option<String>,
    /// 工具块（tool_use / tool_result）建议 UI 折叠展示。
    pub is_collapsible_tool: bool,
    /// `text` 是否因超长被截断（上限见 `MAX_TEXT_CHARS`）。
    pub truncated: bool,
    /// 记录里的原始时间；缺失或无法解析时为 `None`（不伪造扫描时间）。
    pub time: Option<String>,
}

impl ContentBlock {
    /// 构造块并统一做控制字符剔除与长度截断。
    pub fn new(
        role: ContentRole,
        raw_text: Option<String>,
        tool_name: Option<String>,
        is_collapsible_tool: bool,
        time: Option<String>,
    ) -> Self {
        let (text, truncated) = match raw_text {
            Some(raw) => {
                let (sanitized, truncated) = sanitize_and_truncate(&raw);
                (Some(sanitized), truncated)
            }
            None => (None, false),
        };
        ContentBlock {
            role,
            text,
            tool_name,
            is_collapsible_tool,
            truncated,
            time,
        }
    }
}

/// 一页正文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagePage {
    pub blocks: Vec<ContentBlock>,
    /// 下一页游标（行首字节偏移）；已到末尾为 `None`。
    pub next_cursor: Option<u64>,
    /// 是否还有未消费的行。为真时下一页**可能**为空页（剩余行都不含正文）。
    pub has_more: bool,
    /// 只含偏移与短码的原因说明，不含正文与原始 JSON。
    pub diagnostics: Vec<String>,
}

fn sanitize_and_truncate(raw: &str) -> (String, bool) {
    let mut out = String::new();
    let mut count = 0usize;
    let mut truncated = false;
    for ch in raw.chars() {
        // 剔除控制字符（保留换行与制表符），防止不可信正文里的控制序列影响展示。
        if ch.is_control() && ch != '\n' && ch != '\t' {
            continue;
        }
        if count >= MAX_TEXT_CHARS {
            truncated = true;
            break;
        }
        out.push(ch);
        count += 1;
    }
    (out, truncated)
}

/// epoch 毫秒 → RFC3339；越界或非法返回 `None`。
pub(crate) fn ms_to_rfc3339(ms: i64) -> Option<String> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
        .ok()
        .and_then(|dt| dt.format(&Rfc3339).ok())
}

/// 从 JSON 值抽文本：字符串直接返回；数字/布尔转字符串；数组拼接各元素；对象取首个
/// 文本字段（顺序 text / content / message / output / thinking / summary）。无文本返回 `None`。
pub(crate) fn json_text(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(_) | serde_json::Value::Bool(_) => Some(v.to_string()),
        serde_json::Value::Array(items) => {
            let parts: Vec<String> = items.iter().filter_map(json_text).collect();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        serde_json::Value::Object(map) => {
            for key in [
                "text", "content", "message", "output", "thinking", "summary",
            ] {
                if let Some(inner) = map.get(key) {
                    if let Some(text) = json_text(inner) {
                        return Some(text);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// 诊断里安全嵌入的短标识：只保留 ASCII 字母数字与 `_`/`-`/`.`，最长 48 字符。
/// 防止不可信的类型名把控制字符或超长文本带进诊断。
pub(crate) fn diag_token(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .take(48)
        .collect()
}

/// 行驱动的分页驱动：从 `cursor` 起的**完整行**（以 `\n` 结尾）逐行交给 `parse_line`，
/// 按行原子地装入 `blocks` 直到 `limit`。`parse_line` 返回该行的块与可追加的诊断。
pub(crate) fn paginate<F>(
    bytes: &[u8],
    cursor: Option<u64>,
    limit: usize,
    mut diagnostics: Vec<String>,
    mut parse_line: F,
) -> MessagePage
where
    F: FnMut(&[u8], u64, &mut Vec<String>) -> Vec<ContentBlock>,
{
    if bytes.is_empty() {
        diagnostics.push("empty_input".into());
        return MessagePage {
            blocks: Vec::new(),
            next_cursor: None,
            has_more: false,
            diagnostics,
        };
    }

    let len = bytes.len() as u64;
    let eff_limit = limit.clamp(1, MAX_PAGE_LIMIT);
    let start = cursor.unwrap_or(0);
    if start > len {
        diagnostics.push(format!("cursor_out_of_range:{start}>{len}"));
        return MessagePage {
            blocks: Vec::new(),
            next_cursor: None,
            has_more: false,
            diagnostics,
        };
    }

    let mut blocks: Vec<ContentBlock> = Vec::new();
    let mut next_cursor = None;
    let mut has_more = false;
    let mut line_start = start as usize;

    while line_start < bytes.len() {
        let Some(nl) = bytes[line_start..].iter().position(|b| *b == b'\n') else {
            break; // 未写完的尾行不消费
        };
        let line_end = line_start + nl;
        let offset = line_start as u64;
        let line = &bytes[line_start..line_end];
        line_start = line_end + 1;

        if blocks.len() >= eff_limit {
            next_cursor = Some(offset);
            has_more = true;
            break;
        }
        let mut produced = parse_line(line, offset, &mut diagnostics);
        if !produced.is_empty() && !blocks.is_empty() && blocks.len() + produced.len() > eff_limit {
            // 该行整条留给下一页，保证页为行原子的、游标落在行首。
            next_cursor = Some(offset);
            has_more = true;
            break;
        }
        blocks.append(&mut produced);
    }

    MessagePage {
        blocks,
        next_cursor,
        has_more,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(text: &str) -> ContentBlock {
        ContentBlock::new(ContentRole::User, Some(text.to_string()), None, false, None)
    }

    #[test]
    fn content_truncates_long_text_and_flags_it() {
        let long = "x".repeat(MAX_TEXT_CHARS + 500);
        let b = block(&long);
        assert!(b.truncated);
        assert_eq!(b.text.as_deref().unwrap().chars().count(), MAX_TEXT_CHARS);
    }

    #[test]
    fn content_short_text_is_not_truncated() {
        let b = block("hello");
        assert!(!b.truncated);
        assert_eq!(b.text.as_deref(), Some("hello"));
    }

    #[test]
    fn content_strips_control_chars_keeps_newline_tab() {
        let b = block("a\u{0}b\u{7}c\nd\te");
        assert_eq!(b.text.as_deref(), Some("abc\nd\te"));
    }

    #[test]
    fn content_pagination_is_line_atomic_and_cursor_strictly_advances() {
        // 每行产出 1 个块；limit=2 → 第一页 2 块、游标指向第 3 行。
        let data = b"a\nb\nc\nd\n";
        let page = paginate(data, None, 2, Vec::new(), |line, _o, _d| {
            vec![block(std::str::from_utf8(line).unwrap())]
        });
        assert_eq!(page.blocks.len(), 2);
        assert!(page.has_more);
        assert_eq!(page.next_cursor, Some(4)); // 第 3 行 "c" 的行首
        let page2 = paginate(data, page.next_cursor, 2, Vec::new(), |line, _o, _d| {
            vec![block(std::str::from_utf8(line).unwrap())]
        });
        assert_eq!(page2.blocks.len(), 2);
        assert!(!page2.has_more);
        assert_eq!(page2.next_cursor, None);
    }

    #[test]
    fn content_pagination_defers_oversized_line_but_guarantees_progress() {
        // 第一行产出 3 块，limit=2：本页为空时仍整行输出（越界但不死循环）。
        let data = b"a\n";
        let page = paginate(data, None, 2, Vec::new(), |_line, _o, _d| {
            vec![block("1"), block("2"), block("3")]
        });
        assert_eq!(page.blocks.len(), 3);
        assert!(!page.has_more);
    }

    #[test]
    fn content_limit_is_clamped_and_empty_input_diagnosed() {
        let page = paginate(b"", None, 0, Vec::new(), |_l, _o, _d| Vec::new());
        assert!(page.diagnostics.iter().any(|d| d == "empty_input"));

        let data = b"a\nb\n";
        let page = paginate(data, None, 10_000, Vec::new(), |line, _o, _d| {
            vec![block(std::str::from_utf8(line).unwrap())]
        });
        assert_eq!(page.blocks.len(), 2);
        assert!(!page.has_more);

        let page = paginate(data, Some(999), 10, Vec::new(), |_l, _o, _d| Vec::new());
        assert!(page
            .diagnostics
            .iter()
            .any(|d| d.starts_with("cursor_out_of_range:")));
    }
}
