//! A01 Claude Code `projects/**/*.jsonl` 会话日志纯解析器。
//!
//! 证据：docs/research/evidence/a01-a04-ccswitch-importers.md（cc-switch `df40d53c`，A01 节）
//! 与 docs/specs/AGENT_ADAPTERS.md 第 3 节 A01。
//!
//! 布局（由采集器负责发现与规范化，这里只拿文件字节 + 配置）：
//! - `projects/<项目目录>/*.jsonl`（主会话）
//! - `projects/<项目目录>/<SESSION_ID>/subagents/*.jsonl`（子 Agent，配置 is_subagent=true）
//!
//! 格式（JSONL，每行一个对象；只读，不写回）：
//! - 只对 `type == "assistant"` 且带 `message.usage` 对象的行提取用量；其余行（user/system 等）
//!   忽略。
//! - `sessionId` 取文件中第一次出现的非空值（仅作展示/身份命名空间，不参与上游身份）。
//! - 行级 `timestamp` 是 RFC3339 字符串，原样保留；缺失时 occurred_at = null，不伪造。
//!
//! 字段映射（Anthropic 口径，input 不含 cached）：
//! - `message.usage.input_tokens`            → input_fresh（fresh，不含缓存）
//! - `message.usage.cache_read_input_tokens` → input_cache_read
//! - `message.usage.cache_creation_input_tokens` → input_cache_write
//! - `message.usage.output_tokens`           → output_total
//! - `message.usage.cache_creation.ephemeral_1h_input_tokens` 是 cache_write 的 **TTL 子集**，
//!   只做一致性校验，**不再次加到输入**（契约第 3 节：TTL 分量不能重复加）。
//! - input_total = fresh + read + write；total = input_total + output。
//!
//! 计数与身份（契约第 4 节）：
//! - granularity = request，request_count = 1。
//! - event_id = `claude_code:{原生 session_id}:{message.id}`（原生 message ID 身份）。
//!   同一 message.id 的流式/补全写入**只产生一条事件**，命中「有 stop_reason 优先，
//!   否则 output_tokens 更大」的版本；输出 `replace_existing = true`，由存储层做替换。
//! - 不同 message.id 即使数值完全相同也分别计入——**不做按数值判重**（不复制上游
//!   ±600s + token 四元组指纹去重，见证据第 8.1 点的契约冲突）。
//! - 子 Agent 文件：actor_id 取子 Agent 标识（配置给出，来自目录/文件名）；
//!   session_id/parent_session_id 取文件内容里的 `sessionId`。Claude Code 的子 Agent
//!   记录沿用父会话的 `sessionId`，因此两者同源（证据未提供独立的子会话 id 字段）。
//!
//! 其他：
//! - 无 `message.usage` 的消息不产生事件；usage 四项全零也不产生事件（不制造零消耗事件）。
//! - 日志无上游自报费用，日志内也不做定价：cost_reported / cost_estimated 均为 None
//!   （没有价格不等于免费）。
//! - 尾行未以换行结尾视为未写完，不消费；畸形行记诊断、继续，不崩溃。
//!
//! 纯函数：不读文件系统、不访问网络与凭据。

use crate::domain::*;
use std::collections::HashMap;

pub const PARSER_VERSION: &str = "claude-code-projects-1";

pub struct ClaudeParseConfig {
    pub source_id: String,
    pub file_identity: String,
    pub generation: u64,
    /// 本机观察时间（RFC3339）
    pub observed_at: String,
    /// 是否来自 `<session>/subagents/` 的子 Agent 文件
    pub is_subagent: bool,
    /// 子 Agent 标识（来自目录/文件名，例如 `agent-7f3a`）；主会话文件为 None
    pub subagent_actor_id: Option<String>,
}

/// 一个逻辑事件 + 写入语义。原生 message ID 身份下，同 id 的新版本替换旧版本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEvent {
    pub event: UsageEvent,
    pub replace_existing: bool,
}

#[derive(Debug, Default)]
pub struct ClaudeParseOutcome {
    pub events: Vec<ParsedEvent>,
    /// 已消费字节数（最后一个完整换行之后）。截断尾行不计入。
    pub consumed_bytes: u64,
    pub diagnostics: Vec<String>,
}

/// 单个 assistant 行解析出的候选用量（尚未做 message.id 归并、尚未加总）。
struct Candidate {
    message_id: String,
    offset: u64,
    occurred_at: Option<String>,
    model: Option<String>,
    has_stop_reason: bool,
    input_fresh: u64,
    output_total: u64,
    cache_read: u64,
    cache_write: u64,
}

pub fn parse_projects(bytes: &[u8], cfg: &ClaudeParseConfig) -> ClaudeParseOutcome {
    let mut out = ClaudeParseOutcome::default();

    // 按行拆分并记录每行起始偏移；只消费以 \n 结尾的行。
    let mut lines: Vec<(u64, &[u8])> = Vec::new();
    let mut start = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            lines.push((start as u64, &bytes[start..i]));
            start = i + 1;
        }
    }
    out.consumed_bytes = start as u64;

    let mut session_id: Option<String> = None;
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut merged_updates: u64 = 0;

    for (offset, line) in &lines {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(line) else {
            out.diagnostics
                .push(format!("malformed_line_at_offset:{offset}"));
            continue;
        };

        // sessionId 取文件中第一次出现的值（任何类型的行都可能有）
        if session_id.is_none() {
            if let Some(s) = v.get("sessionId").and_then(|x| x.as_str()) {
                if !s.is_empty() {
                    session_id = Some(s.to_string());
                }
            }
        }

        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(message) = v.get("message") else {
            continue;
        };
        // 无 usage 的消息不产生事件（也不诊断：流式非最终块常常没有 usage）
        let Some(usage) = message.get("usage") else {
            continue;
        };
        let Some(message_id) = message.get("id").and_then(|x| x.as_str()) else {
            out.diagnostics
                .push(format!("assistant_without_message_id_at_offset:{offset}"));
            continue;
        };
        let Some(input_fresh) = usage.get("input_tokens").and_then(|x| x.as_u64()) else {
            out.diagnostics
                .push(format!("usage_input_tokens_missing_at_offset:{offset}"));
            continue;
        };
        let Some(output_total) = usage.get("output_tokens").and_then(|x| x.as_u64()) else {
            out.diagnostics
                .push(format!("usage_output_tokens_missing_at_offset:{offset}"));
            continue;
        };
        // 缓存字段缺失按 0（未使用缓存），不制造未知分量
        let cache_read = usage
            .get("cache_read_input_tokens")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let cache_write = usage
            .get("cache_creation_input_tokens")
            .and_then(|x| x.as_u64())
            .unwrap_or(0);

        // TTL 子集：ephemeral_1h 只是 cache_write 的细分，不重复加；仅在越界时记诊断
        if let Some(ttl_1h) = usage
            .get("cache_creation")
            .and_then(|c| c.get("ephemeral_1h_input_tokens"))
            .and_then(|x| x.as_u64())
        {
            if ttl_1h > cache_write {
                out.diagnostics.push(format!(
                    "cache_creation_1h_exceeds_total_at_offset:{offset}: {ttl_1h}>{cache_write}（未重复加）"
                ));
            }
        }

        // 四项全零 = 上游零用量快照，不制造零消耗事件
        if input_fresh == 0 && output_total == 0 && cache_read == 0 && cache_write == 0 {
            out.diagnostics
                .push(format!("zero_usage_message_at_offset:{offset}"));
            continue;
        }

        let has_stop_reason = message.get("stop_reason").is_some_and(|x| !x.is_null());
        let candidate = Candidate {
            message_id: message_id.to_string(),
            offset: *offset,
            occurred_at: v
                .get("timestamp")
                .and_then(|t| t.as_str())
                .map(str::to_string),
            model: message
                .get("model")
                .and_then(|m| m.as_str())
                .map(str::to_string),
            has_stop_reason,
            input_fresh,
            output_total,
            cache_read,
            cache_write,
        };

        match index.get(message_id).copied() {
            Some(idx) => {
                merged_updates += 1;
                let existing = &candidates[idx];
                let new_wins = match (existing.has_stop_reason, candidate.has_stop_reason) {
                    (false, true) => true,
                    (true, false) => false,
                    // 有无 stop_reason 一致时取 output_tokens 更大者；相等保留先出现的
                    _ => candidate.output_total > existing.output_total,
                };
                if new_wins {
                    candidates[idx] = candidate;
                }
            }
            None => {
                index.insert(message_id.to_string(), candidates.len());
                candidates.push(candidate);
            }
        }
    }

    if merged_updates > 0 {
        out.diagnostics
            .push(format!("streaming_updates_merged:{merged_updates}"));
    }

    if session_id.is_none() {
        out.diagnostics
            .push("session_id_missing: 事件身份退化为文件身份命名空间".into());
    }
    if cfg.is_subagent && cfg.subagent_actor_id.is_none() {
        out.diagnostics
            .push("subagent_actor_id_missing: 子 Agent 文件缺标识，actor_id 留空".into());
    }

    let session_ns = session_id
        .clone()
        .unwrap_or_else(|| cfg.file_identity.clone());

    for c in candidates {
        let Some(input_total) = c
            .input_fresh
            .checked_add(c.cache_read)
            .and_then(|s| s.checked_add(c.cache_write))
        else {
            out.diagnostics
                .push(format!("token_overflow_at_offset:{}", c.offset));
            continue;
        };
        let Some(total) = input_total.checked_add(c.output_total) else {
            out.diagnostics
                .push(format!("token_overflow_at_offset:{}", c.offset));
            continue;
        };

        // 子 Agent 沿用父会话 sessionId，故 parent_session_id 与 session_id 同源；
        // actor_id 才是区分子 Agent 的身份。
        let (ev_session, ev_parent, ev_actor) = if cfg.is_subagent {
            (
                session_id.clone(),
                session_id.clone(),
                cfg.subagent_actor_id.clone(),
            )
        } else {
            (session_id.clone(), None, None)
        };

        out.events.push(ParsedEvent {
            replace_existing: true,
            event: UsageEvent {
                schema_version: USAGE_EVENT_SCHEMA_VERSION,
                event_id: format!("claude_code:{session_ns}:{}", c.message_id),
                agent: AgentKind::ClaudeCode,
                source_id: cfg.source_id.clone(),
                stream_domain: StreamDomain::LocalLog,
                session_id: ev_session,
                parent_session_id: ev_parent,
                actor_id: ev_actor,
                turn_id: None,
                project_ref: None,
                occurred_at: c.occurred_at,
                observed_at: cfg.observed_at.clone(),
                model_raw: c.model,
                model_normalized: None,
                granularity: Granularity::Request,
                request_count: Some(1),
                tokens: TokenSet {
                    input_fresh: Some(c.input_fresh),
                    input_cache_read: Some(c.cache_read),
                    input_cache_write: Some(c.cache_write),
                    input_total: Some(input_total),
                    output_total: Some(c.output_total),
                    output_reasoning: None,
                    total: Some(total),
                    quality: Quality::Reported,
                },
                cost_reported: None,
                cost_estimated: None,
                completeness: Completeness::Complete,
                account_link: None,
                evidence_refs: vec![EvidenceRef {
                    source_id: cfg.source_id.clone(),
                    file_identity: cfg.file_identity.clone(),
                    generation: cfg.generation,
                    locator: c.offset.to_string(),
                    parser_version: PARSER_VERSION.into(),
                }],
            },
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ClaudeParseConfig {
        ClaudeParseConfig {
            source_id: "test-src".into(),
            file_identity: "fileA".into(),
            generation: 0,
            observed_at: "2026-10-11T08:00:00Z".into(),
            is_subagent: false,
            subagent_actor_id: None,
        }
    }

    fn assistant(id: &str, stop: &str, usage: &str) -> String {
        format!(
            "{{\"type\":\"assistant\",\"sessionId\":\"sess-1\",\"timestamp\":\"2026-10-11T01:00:00.000Z\",\"message\":{{\"id\":\"{id}\",\"model\":\"claude-sonnet-4-20250514\",\"stop_reason\":{stop},\"usage\":{usage}}}}}\n"
        )
    }

    #[test]
    fn maps_anthropic_fields_and_ttl_subset_not_added() {
        let data = assistant(
            "msg_1",
            "\"end_turn\"",
            "{\"input_tokens\":100,\"output_tokens\":50,\"cache_read_input_tokens\":20,\"cache_creation_input_tokens\":1000,\"cache_creation\":{\"ephemeral_1h_input_tokens\":600}}",
        );
        let out = parse_projects(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1);
        let t = &out.events[0].event.tokens;
        assert_eq!(t.input_fresh, Some(100));
        assert_eq!(t.input_cache_read, Some(20));
        // 1h 细分子集不重复加：write 仍是 1000，不是 1600
        assert_eq!(t.input_cache_write, Some(1000));
        assert_eq!(t.input_total, Some(1120));
        assert_eq!(t.output_total, Some(50));
        assert_eq!(t.total, Some(1170));
        assert!(out.events[0].replace_existing);
    }

    #[test]
    fn same_message_id_merges_stop_reason_priority() {
        let data = format!(
            "{}{}",
            assistant(
                "msg_s",
                "null",
                "{\"input_tokens\":200,\"output_tokens\":40}"
            ),
            assistant(
                "msg_s",
                "\"end_turn\"",
                "{\"input_tokens\":200,\"output_tokens\":25}"
            ),
        );
        let out = parse_projects(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1, "同 id 只产出一条");
        let e = &out.events[0].event;
        assert_eq!(
            e.tokens.output_total,
            Some(25),
            "stop_reason 优先，即使更小"
        );
        assert_eq!(e.tokens.total, Some(225));
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("streaming_updates_merged:1")));
    }

    #[test]
    fn larger_output_wins_when_stop_reason_presence_equal() {
        let data = format!(
            "{}{}",
            assistant("msg_o", "null", "{\"input_tokens\":80,\"output_tokens\":7}"),
            assistant(
                "msg_o",
                "null",
                "{\"input_tokens\":80,\"output_tokens\":12}"
            ),
        );
        let out = parse_projects(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].event.tokens.output_total, Some(12));
    }

    #[test]
    fn identical_numbers_different_ids_are_both_counted() {
        let data = format!(
            "{}{}",
            assistant(
                "msg_a",
                "null",
                "{\"input_tokens\":100,\"output_tokens\":20}"
            ),
            assistant(
                "msg_b",
                "null",
                "{\"input_tokens\":100,\"output_tokens\":20}"
            ),
        );
        let out = parse_projects(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 2, "相同数字不判重");
    }

    #[test]
    fn no_usage_and_all_zero_make_no_event() {
        let data = concat!(
            "{\"type\":\"assistant\",\"sessionId\":\"sess-1\",\"message\":{\"id\":\"msg_no_usage\",\"content\":[]}}\n",
            "{\"type\":\"assistant\",\"sessionId\":\"sess-1\",\"message\":{\"id\":\"msg_zero\",\"usage\":{\"input_tokens\":0,\"output_tokens\":0,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n",
            "{\"type\":\"user\",\"message\":{\"content\":\"hi\"}}\n",
        );
        let out = parse_projects(data.as_bytes(), &cfg());
        assert!(out.events.is_empty());
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("zero_usage_message_at_offset")));
    }

    #[test]
    fn unterminated_tail_not_consumed() {
        let data = format!(
            "{}{}",
            assistant(
                "msg_t",
                "null",
                "{\"input_tokens\":64,\"output_tokens\":16}"
            ),
            "{\"type\":\"assistant\",\"message\":{\"id\":\"msg_x\",\"usa"
        );
        let out = parse_projects(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1);
        let nl = data.match_indices('\n').next().unwrap().0;
        assert_eq!(out.consumed_bytes, nl as u64 + 1);
    }

    #[test]
    fn subagent_identity_from_content_and_config() {
        let data = assistant(
            "msg_sub",
            "\"end_turn\"",
            "{\"input_tokens\":300,\"output_tokens\":60}",
        );
        let cfg = ClaudeParseConfig {
            is_subagent: true,
            subagent_actor_id: Some("agent-7f3a".into()),
            ..cfg()
        };
        let out = parse_projects(data.as_bytes(), &cfg);
        let e = &out.events[0].event;
        assert_eq!(e.session_id.as_deref(), Some("sess-1"));
        assert_eq!(e.parent_session_id.as_deref(), Some("sess-1"));
        assert_eq!(e.actor_id.as_deref(), Some("agent-7f3a"));
        assert_eq!(e.event_id, "claude_code:sess-1:msg_sub");
    }

    #[test]
    fn malformed_line_diagnosed_without_panic() {
        let data = concat!(
            "{not json\n",
            "{\"type\":\"assistant\",\"sessionId\":\"s\",\"message\":{\"id\":\"m\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n",
        );
        let out = parse_projects(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1);
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.starts_with("malformed_line_at_offset:0")));
    }
}
