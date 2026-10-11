//! A03 Gemini CLI 纯解析器（会话 JSON 与 JSONL 回放两种形态）。
//!
//! 证据：docs/research/evidence/a01-a04-ccswitch-importers.md A03 节（cc-switch `df40d53c`，
//! `services/session_usage_gemini.rs`、`session_manager/providers/gemini.rs#L75-L122`）。
//! 上游路径为 `<gemini>/tmp/<project_hash>/chats/session-*.json(l)`；`.jsonl` 与同名 `.json`
//! 并存时的优先级属于路径发现层，**不在解析器内推断**：调用方决定形态，调用对应入口。
//!
//! 两种形态：
//! - 完整 JSON 文档：`{sessionId, messages[]}`。一次快照，整体覆盖；解析失败不推进游标。
//! - JSONL 回放：首行/若干行是元数据（含 `sessionId`），带字符串 `id` 的行是消息（同 id
//!   原位覆盖、保留首次出现顺序），`{"$set":{...}}` 合并元数据并在含 `messages` 时整体替换
//!   （本解析器同时支持 `$set` 带 `id` 的按 id 更新），`{"$rewindTo": id}` 删掉该条及之后的
//!   消息、找不到 id 时清空。回放得到「当前消息集合」后再取用量。
//!
//! 只处理 `type == "gemini"` 的消息。无 `tokens` 字段、`tokens: null`、以及四项计数全 0
//! 的消息都不产生事件（不制造零消耗；上游同口径注释「跳过全零的空 token 消息」）。
//!
//! usage 字段映射（`tokens{input, output, thoughts, cached}`；`tool`/`total` 忽略、不参与校验）：
//! - `input_total = input`（reported）
//! - `input_cache_read = cached`（`cached > input` 的异常行钳制为 input 并记诊断，保证互斥分量和不越界）
//! - `input_fresh = input − cached`（钳制不取负）
//! - `input_cache_write = null`：源不提供该分量（Gemini 无写入分量上报）。未知 ≠ 0，
//!   **禁止照抄上游恒写 0 的做法**。
//! - `output_total = output + thoughts`，`output_reasoning = thoughts`
//! - `total = input_total + output_total`（不读源 `total`）
//! - `quality = derived`（input_fresh 为减法、output_total 为加法，不冒称 reported）；
//!   `completeness = complete`（输入总量与输出总量都已知，缺失的写缓存分量不影响任何总量）
//!
//! 包含关系的证据级别（input 含 cached；thoughts 与 candidates 分离）：
//! - E2：官方 API 字段口径 + 上游源码与其测试算术。
//!   * Gemini API `usageMetadata`：`promptTokenCount`（输入）、`cachedContentTokenCount`
//!     （缓存命中，是 `promptTokenCount` 的子集）、`candidatesTokenCount`（可见输出）、
//!     `thoughtsTokenCount`（思考，与 `candidatesTokenCount` 分离）。
//!   * 上游夹具 `{input:8522, output:29, cached:3138, thoughts:405, total:8956}` 满足
//!     `8522+29+405 = 8956`：total 不含 cached（⇒ cached ⊂ input，不能再加一次），
//!     total 含 thoughts（⇒ thoughts 在输出侧）。
//! - **待官方样本确认**：尚未用真实官方 Gemini CLI 会话文件与 `usageMetadata` 逐字段对照。
//!   待确认项：`tokens` 四元组与 usageMetadata 的一一对应、`thoughtsTokenCount` 的计价口径、
//!   是否存在 cache 写入分量（在确认前保持 null，不补 0）。
//!
//! 身份与更新：`event_id = "gemini_cli:{sessionId}:{message.id}"`。`message.id` 缺失或为空
//! 时跳过该条并记诊断——不复制上游兜底 `"unknown"` 导致同会话多条消息撞同一主键的缺陷。
//! `sessionId` 缺失时用调用方给出的文件身份回退建立身份（同样不兜 `"unknown"`）。
//! `$set` 同 id 更新 / 消息行重复出现 / 完整 JSON 快照一律 `replace_existing = true`
//! （消息的部分 usage 更新替换旧版本，不逐次累加）；JSONL 中只出现一次的消息为 false。
//!
//! `$rewindTo` 删掉的历史消息在回放层即不再产生事件；但**已入账的历史事件不由此层删除**
//! （契约第 5 节：对话回退不撤销已发生的消耗，账本保留旧事件是调用方那一层的事）。
//! 解析器只管「当前集合」。
//!
//! 数字相同的独立消息各自计入，不做任何数值判重。尾行未以 `\n` 结尾视为未写完，不消费。
//! 纯函数：不读文件系统、不访问网络与凭据。

use crate::domain::*;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub const PARSER_VERSION: &str = "gemini-cli-1";

pub struct GeminiParseConfig {
    pub source_id: String,
    /// 规范化文件身份（不是可逆路径）。
    pub file_identity: String,
    /// 文件代数：轮转/替换递增。
    pub generation: u64,
    /// 本机观察时间（RFC3339）。
    pub observed_at: String,
}

/// 一条解析结果：事件 + 是否为对既有事件身份的替换（upsert 语义）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEvent {
    pub event: UsageEvent,
    pub replace_existing: bool,
}

#[derive(Debug, Default, Clone)]
pub struct GeminiParseOutcome {
    /// 会话身份（顶层 `sessionId`）。即使没有任何用量事件也返回，
    /// 供调用方保留会话索引（无 tokens 的消息不制造零消耗事件）。
    pub session_id: Option<String>,
    pub events: Vec<ParsedEvent>,
    /// 已消费字节数。JSONL：最后一个 `\n` 之后（截断尾行不计入）；
    /// JSON：文档成功解析时为整个文件长度，解析失败/形态不认识时为 0（不推进游标）。
    pub consumed_bytes: u64,
    pub diagnostics: Vec<String>,
}

/// 解析完整 JSON 会话文件 `{sessionId, messages[]}`。
pub fn parse_session_json(bytes: &[u8], cfg: &GeminiParseConfig) -> GeminiParseOutcome {
    let mut out = GeminiParseOutcome::default();

    let doc: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => {
            out.diagnostics.push(format!("json_parse_error:{e}"));
            return out;
        }
    };
    let Some(messages) = doc.get("messages").and_then(Value::as_array) else {
        out.diagnostics
            .push("missing_messages_array: 形态不认识，不猜字段，不推进游标".into());
        return out;
    };

    let raw_session = doc.get("sessionId").and_then(non_empty_str);
    let (session_key, session_id) = session_identity(raw_session, cfg, &mut out.diagnostics);
    out.session_id = session_id.clone();
    out.consumed_bytes = bytes.len() as u64;

    for (i, msg) in messages.iter().enumerate() {
        if msg.get("type").and_then(Value::as_str) != Some("gemini") {
            continue;
        }
        let locator = format!("messages[{i}]");
        match build_event(msg, &session_key, &session_id, &locator, cfg, &mut out.diagnostics) {
            Ok(event) => out.events.push(ParsedEvent {
                event,
                // 完整 JSON 是一次快照，看不出本轮哪些消息被更新：按 upsert 语义全部标记替换
                // （上游同口径 UPSERT）。身份稳定，替换不会重复累计。
                replace_existing: true,
            }),
            Err(reason) => out.diagnostics.push(format!("{locator}: {reason}")),
        }
    }

    out
}

/// 解析 JSONL 回放格式，得到当前消息集合后取用量。
pub fn parse_session_jsonl(bytes: &[u8], cfg: &GeminiParseConfig) -> GeminiParseOutcome {
    let mut out = GeminiParseOutcome::default();

    // 只消费以 \n 结尾的行；尾行未写完不消费
    let mut lines: Vec<(u64, &[u8])> = Vec::new();
    let mut start = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            lines.push((start as u64, &bytes[start..i]));
            start = i + 1;
        }
    }
    out.consumed_bytes = start as u64;

    let mut replay = Replay::default();
    for (offset, line) in &lines {
        if line.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            out.diagnostics.push(format!("malformed_line_at_offset:{offset}"));
            continue;
        };
        let locator = offset.to_string();

        if let Some(target) = record.get("$rewindTo").and_then(non_empty_str) {
            match replay.remove_from(target) {
                Some(removed) => out
                    .diagnostics
                    .push(format!("rewind_to:{target}:removed:{removed}")),
                None => {
                    replay.clear_current();
                    out.diagnostics
                        .push(format!("rewind_to_unknown_id:{target}:cleared_all"));
                }
            }
            continue;
        }

        // 元数据行：sessionId 在消息行、$set、纯元数据行上都可能出现
        if let Some(sid) = record.get("sessionId").and_then(non_empty_str) {
            replay.session_id = Some(sid.to_string());
        }

        if let Some(patch) = record.get("$set").and_then(Value::as_object) {
            if let Some(sid) = patch.get("sessionId").and_then(non_empty_str) {
                replay.session_id = Some(sid.to_string());
            }
            if let Some(arr) = patch.get("messages").and_then(Value::as_array) {
                replace_messages(&mut replay, arr, &locator, &mut out.diagnostics);
                continue;
            }
            if let Some(id) = patch.get("id").and_then(non_empty_str) {
                // $set 按 message id 更新的形态：浅合并进既有消息（嵌套对象整体替换）
                let merged = merge_message(replay.messages.get(id), patch);
                replay.upsert(id, merged, &locator);
                continue;
            }
            continue; // 纯元数据合并
        }

        if let Some(arr) = record.get("messages").and_then(Value::as_array) {
            // 单行即完整文档的兼容形态：整体替换
            replace_messages(&mut replay, arr, &locator, &mut out.diagnostics);
            continue;
        }

        match record.get("id") {
            Some(v) => match non_empty_str(v) {
                Some(id) => replay.upsert(id, record.clone(), &locator),
                None => out
                    .diagnostics
                    .push(format!("record_with_invalid_id_at_offset:{offset}")),
            },
            None => {
                if record.get("sessionId").and_then(non_empty_str).is_none() {
                    out.diagnostics
                        .push(format!("unrecognized_record_at_offset:{offset}"));
                }
            }
        }
    }

    let (session_key, session_id) =
        session_identity(replay.session_id.as_deref(), cfg, &mut out.diagnostics);
    out.session_id = session_id.clone();

    for id in &replay.order {
        let Some(msg) = replay.messages.get(id) else {
            continue;
        };
        if msg.get("type").and_then(Value::as_str) != Some("gemini") {
            continue;
        }
        let locator = replay.locators.get(id).cloned().unwrap_or_default();
        match build_event(msg, &session_key, &session_id, &locator, cfg, &mut out.diagnostics) {
            Ok(event) => out.events.push(ParsedEvent {
                event,
                replace_existing: replay.replace_flags.get(id).copied().unwrap_or(false),
            }),
            Err(reason) => out.diagnostics.push(format!("{locator}: {reason}")),
        }
    }

    out
}

/// JSONL 回放的当前消息集合：首次出现顺序 + 同 id 原位覆盖。
#[derive(Default)]
struct Replay {
    session_id: Option<String>,
    order: Vec<String>,
    messages: HashMap<String, Value>,
    locators: HashMap<String, String>,
    /// 该 id 的当前版本是否替换了先前版本（= 写入前该 id 是否出现过）
    replace_flags: HashMap<String, bool>,
    /// 本文件内出现过的所有 id（rewind 不移除：rewind 掉的消息其已入账事件仍在账本里，
    /// 同 id 再次出现时按替换处理）
    ever_seen: HashSet<String>,
}

impl Replay {
    fn upsert(&mut self, id: &str, value: Value, locator: &str) {
        let replace = self.ever_seen.contains(id);
        self.ever_seen.insert(id.to_string());
        if !self.messages.contains_key(id) {
            self.order.push(id.to_string());
        }
        self.messages.insert(id.to_string(), value);
        self.locators.insert(id.to_string(), locator.to_string());
        self.replace_flags.insert(id.to_string(), replace);
    }

    /// 删掉该 id 及之后的消息；找不到该 id 返回 None（调用方清空）。
    fn remove_from(&mut self, id: &str) -> Option<usize> {
        let idx = self.order.iter().position(|x| x == id)?;
        let removed: Vec<String> = self.order.drain(idx..).collect();
        for m in &removed {
            self.messages.remove(m);
            self.locators.remove(m);
            self.replace_flags.remove(m);
        }
        Some(removed.len())
    }

    fn clear_current(&mut self) {
        self.order.clear();
        self.messages.clear();
        self.locators.clear();
        self.replace_flags.clear();
    }
}

/// `$set.messages`（或单行完整文档）整体替换当前集合；元素缺 id 时跳过并记诊断。
fn replace_messages(
    replay: &mut Replay,
    arr: &[Value],
    locator: &str,
    diagnostics: &mut Vec<String>,
) {
    let mut new_ids: Vec<&str> = Vec::new();
    for entry in arr {
        match entry.get("id").and_then(non_empty_str) {
            Some(id) => new_ids.push(id),
            None => diagnostics.push(format!(
                "{locator}: set_messages_entry_without_id: 跳过（不复制上游 unknown 兜底）"
            )),
        }
    }
    let dropped = replay
        .order
        .iter()
        .filter(|x| !new_ids.contains(&x.as_str()))
        .count();
    replay.clear_current();
    for entry in arr {
        if let Some(id) = entry.get("id").and_then(non_empty_str) {
            replay.upsert(id, entry.clone(), locator);
        }
    }
    diagnostics.push(format!(
        "set_messages_replaced:kept:{}:dropped:{dropped}",
        new_ids.len()
    ));
}

/// 浅合并：patch 的顶层键覆盖既有消息（嵌套对象整体替换，不做深合并）。
fn merge_message(existing: Option<&Value>, patch: &serde_json::Map<String, Value>) -> Value {
    let mut merged = existing
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (k, v) in patch {
        merged.insert(k.clone(), v.clone());
    }
    Value::Object(merged)
}

fn build_event(
    msg: &Value,
    session_key: &str,
    session_id: &Option<String>,
    locator: &str,
    cfg: &GeminiParseConfig,
    diagnostics: &mut Vec<String>,
) -> std::result::Result<UsageEvent, String> {
    let id = msg.get("id").and_then(non_empty_str).ok_or(
        "missing_message_id: 跳过该消息（不复制上游 \"unknown\" 兜底撞主键的缺陷）",
    )?;
    let tokens = msg
        .get("tokens")
        .filter(|t| t.is_object())
        .ok_or("missing_tokens: 无 usage 字段，不制造零消耗事件")?;

    let read = |k: &str| tokens.get(k).and_then(Value::as_u64).unwrap_or(0);
    let input = read("input");
    let output = read("output");
    let thoughts = read("thoughts");
    let cached = read("cached");
    if input == 0 && output == 0 && thoughts == 0 && cached == 0 {
        return Err("all_zero_tokens: 空 usage 兜底，跳过（不记为明确的零消耗）".into());
    }

    // cached 是 input 的子集：不重复加；异常行钳制，保证 input_fresh+input_cache_read == input_total
    let cache_read = cached.min(input);
    if cached > input {
        diagnostics.push(format!(
            "{locator}: cached_exceeds_input: cached={cached} > input={input}，读缓存钳制为 input"
        ));
    }
    let input_fresh = input - cache_read;
    let output_total = output
        .checked_add(thoughts)
        .ok_or("output_total_overflow: 跳过（不静默回绕）")?;
    let total = input
        .checked_add(output_total)
        .ok_or("total_overflow: 跳过（不静默回绕）")?;

    Ok(UsageEvent {
        schema_version: USAGE_EVENT_SCHEMA_VERSION,
        event_id: format!("gemini_cli:{session_key}:{id}"),
        agent: AgentKind::GeminiCli,
        source_id: cfg.source_id.clone(),
        stream_domain: StreamDomain::LocalLog,
        session_id: session_id.clone(),
        parent_session_id: None,
        actor_id: None,
        turn_id: None,
        project_ref: None,
        occurred_at: msg
            .get("timestamp")
            .and_then(non_empty_str)
            .map(str::to_string),
        observed_at: cfg.observed_at.clone(),
        // 上游对缺 model 的消息兜底 "unknown"；本解析器保持 null（未知 ≠ 某个模型名）
        model_raw: msg.get("model").and_then(non_empty_str).map(str::to_string),
        model_normalized: None,
        granularity: Granularity::Message,
        request_count: Some(1),
        tokens: TokenSet {
            input_fresh: Some(input_fresh),
            input_cache_read: Some(cache_read),
            input_cache_write: None,
            input_total: Some(input),
            output_total: Some(output_total),
            output_reasoning: Some(thoughts),
            total: Some(total),
            quality: Quality::Derived,
        },
        cost_reported: None,
        cost_estimated: None,
        completeness: Completeness::Complete,
        account_link: None,
        evidence_refs: vec![EvidenceRef {
            source_id: cfg.source_id.clone(),
            file_identity: cfg.file_identity.clone(),
            generation: cfg.generation,
            locator: locator.to_string(),
            parser_version: PARSER_VERSION.into(),
        }],
    })
}

/// 事件身份里的会话键 + 事件 `session_id` 字段。缺 sessionId 时用文件身份回退，不兜 "unknown"。
fn session_identity(
    raw: Option<&str>,
    cfg: &GeminiParseConfig,
    diagnostics: &mut Vec<String>,
) -> (String, Option<String>) {
    match raw {
        Some(s) => (s.to_string(), Some(s.to_string())),
        None => {
            diagnostics.push(
                "missing_session_id: 用文件身份回退建立事件身份（不复制上游 unknown 兜底）".into(),
            );
            (cfg.file_identity.clone(), None)
        }
    }
}

fn non_empty_str(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> GeminiParseConfig {
        GeminiParseConfig {
            source_id: "test-src".into(),
            file_identity: "sess-fixture.jsonl".into(),
            generation: 0,
            observed_at: "2026-10-11T08:00:00Z".into(),
        }
    }

    fn msg(id: &str, tokens: &str) -> String {
        format!(
            "{{\"id\":\"{id}\",\"type\":\"gemini\",\"timestamp\":\"2026-10-11T01:00:00.000Z\",\"model\":\"gemini-2.5-pro\",\"tokens\":{tokens}}}\n"
        )
    }

    fn events(out: &GeminiParseOutcome) -> Vec<UsageEvent> {
        out.events.iter().map(|e| e.event.clone()).collect()
    }

    #[test]
    fn gemini_cached_is_input_subset_not_added_again() {
        // 契约场景：总输入 1000 含 cached 400 → fresh 600 / read 400 / input_total 1000（不是 1400）
        let data = format!(
            "{{\"sessionId\":\"s1\",\"messages\":[{}\n]}}",
            msg("m1", "{\"input\":1000,\"output\":10,\"thoughts\":0,\"cached\":400,\"total\":1010}")
                .trim_end()
        );
        let out = parse_session_json(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1);
        let t = &out.events[0].event.tokens;
        assert_eq!(t.input_total, Some(1000));
        assert_eq!(t.input_cache_read, Some(400));
        assert_eq!(t.input_fresh, Some(600));
        assert_eq!(t.total, Some(1010));
        assert_eq!(
            t.input_fresh.unwrap() + t.input_cache_read.unwrap(),
            t.input_total.unwrap()
        );
    }

    #[test]
    fn gemini_thoughts_are_counted_on_output_side() {
        // 上游夹具 {8522, 29, 3138, 405, total 8956}: 8522+29+405=8956
        let data = format!(
            "{{\"sessionId\":\"s1\",\"messages\":[{}\n]}}",
            msg(
                "m1",
                "{\"input\":8522,\"output\":29,\"thoughts\":405,\"cached\":3138,\"tool\":0,\"total\":8956}"
            )
            .trim_end()
        );
        let out = parse_session_json(data.as_bytes(), &cfg());
        let t = &out.events[0].event.tokens;
        assert_eq!(t.output_total, Some(434), "thoughts 计入输出");
        assert_eq!(t.output_reasoning, Some(405));
        assert_eq!(t.input_fresh, Some(5384));
        assert_eq!(t.total, Some(8956), "与源 total 一致（源 total 本身不读取）");
    }

    #[test]
    fn gemini_cache_write_stays_unknown_not_zero() {
        let data = format!(
            "{{\"sessionId\":\"s1\",\"messages\":[{}\n]}}",
            msg("m1", "{\"input\":10,\"output\":1,\"thoughts\":0,\"cached\":0}").trim_end()
        );
        let out = parse_session_json(data.as_bytes(), &cfg());
        let t = &out.events[0].event.tokens;
        assert_eq!(t.input_cache_write, None, "源不提供写分量：未知 ≠ 0");
        assert_eq!(t.quality, Quality::Derived);
        assert_eq!(t.output_reasoning, Some(0));
    }

    #[test]
    fn gemini_missing_message_id_is_skipped_with_diagnostic() {
        let data = concat!(
            "{\"sessionId\":\"s1\",\"messages\":[",
            "{\"type\":\"gemini\",\"tokens\":{\"input\":999,\"output\":99,\"thoughts\":0,\"cached\":0}},",
            "{\"id\":\"\",\"type\":\"gemini\",\"tokens\":{\"input\":888,\"output\":88,\"thoughts\":0,\"cached\":0}},",
            "{\"id\":\"ok\",\"type\":\"gemini\",\"tokens\":{\"input\":100,\"output\":10,\"thoughts\":0,\"cached\":0}}",
            "]}"
        );
        let out = parse_session_json(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1, "缺 id 的消息不产生事件");
        assert_eq!(out.events[0].event.event_id, "gemini_cli:s1:ok");
        assert_eq!(
            out.diagnostics
                .iter()
                .filter(|d| d.contains("missing_message_id"))
                .count(),
            2
        );
        assert!(!out.events[0].event.event_id.contains("unknown"));
    }

    #[test]
    fn gemini_message_without_tokens_is_not_zero_consumption() {
        let data = concat!(
            "{\"sessionId\":\"s1\",\"messages\":[",
            "{\"id\":\"a\",\"type\":\"gemini\"},",
            "{\"id\":\"b\",\"type\":\"gemini\",\"tokens\":null},",
            "{\"id\":\"c\",\"type\":\"gemini\",\"tokens\":{\"input\":0,\"output\":0,\"thoughts\":0,\"cached\":0}},",
            "{\"id\":\"d\",\"type\":\"gemini\",\"tokens\":{\"input\":5,\"output\":5,\"thoughts\":0,\"cached\":0}}",
            "]}"
        );
        let out = parse_session_json(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].event.event_id, "gemini_cli:s1:d");
        assert!(out.diagnostics.iter().any(|d| d.contains("missing_tokens")));
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("all_zero_tokens")));
    }

    #[test]
    fn gemini_non_gemini_messages_are_ignored() {
        let data = concat!(
            "{\"sessionId\":\"s1\",\"messages\":[",
            "{\"id\":\"u\",\"type\":\"user\",\"tokens\":{\"input\":9,\"output\":0,\"thoughts\":0,\"cached\":0}},",
            "{\"id\":\"t\",\"type\":\"tool\"},",
            "{\"id\":\"g\",\"type\":\"gemini\",\"tokens\":{\"input\":7,\"output\":3,\"thoughts\":0,\"cached\":0}}",
            "]}"
        );
        let out = parse_session_json(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].event.event_id, "gemini_cli:s1:g");
    }

    #[test]
    fn gemini_json_snapshot_consumes_whole_file_and_marks_replace() {
        let data = format!(
            "{{\"sessionId\":\"s1\",\"messages\":[{}\n]}}",
            msg("m1", "{\"input\":1,\"output\":1,\"thoughts\":0,\"cached\":0}").trim_end()
        );
        let out = parse_session_json(data.as_bytes(), &cfg());
        assert_eq!(out.consumed_bytes, data.len() as u64);
        assert!(out.events[0].replace_existing);
        assert_eq!(out.session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn gemini_unparseable_json_consumes_nothing() {
        let out = parse_session_json(b"{\"sessionId\":\"s1\",\"messages\":[", &cfg());
        assert!(out.events.is_empty());
        assert_eq!(out.consumed_bytes, 0, "不推进游标，下一轮重试");
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("json_parse_error")));
    }

    #[test]
    fn gemini_missing_messages_array_is_unknown_shape() {
        let out = parse_session_json(br#"{"sessionId":"s1"}"#, &cfg());
        assert!(out.events.is_empty());
        assert_eq!(out.consumed_bytes, 0);
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("missing_messages_array")));
    }

    #[test]
    fn gemini_jsonl_set_updates_message_in_place() {
        let data = format!(
            "{}{}{}",
            "{\"sessionId\":\"s1\"}\n",
            msg("m1", "{\"input\":100,\"output\":10,\"thoughts\":0,\"cached\":0}"),
            "{\"$set\":{\"id\":\"m1\",\"tokens\":{\"input\":220,\"output\":22,\"thoughts\":6,\"cached\":60}}}\n"
        );
        let out = parse_session_jsonl(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1, "同 id 原位覆盖，不产生两条事件");
        let e = &out.events[0].event;
        assert_eq!(e.tokens.input_total, Some(220), "取更新后的值");
        assert_eq!(e.tokens.input_fresh, Some(160));
        assert_eq!(e.tokens.output_total, Some(28));
        assert!(out.events[0].replace_existing);
    }

    #[test]
    fn gemini_jsonl_set_messages_replaces_whole_set() {
        let data = concat!(
            "{\"sessionId\":\"s1\"}\n",
            "{\"id\":\"r1\",\"type\":\"gemini\",\"tokens\":{\"input\":100,\"output\":10,\"thoughts\":0,\"cached\":0}}\n",
            "{\"id\":\"r2\",\"type\":\"gemini\",\"tokens\":{\"input\":200,\"output\":20,\"thoughts\":0,\"cached\":0}}\n",
            "{\"$set\":{\"messages\":[{\"id\":\"r2\",\"type\":\"gemini\",\"tokens\":{\"input\":260,\"output\":26,\"thoughts\":6,\"cached\":60}}]}}\n",
            "{\"id\":\"r3\",\"type\":\"gemini\",\"tokens\":{\"input\":50,\"output\":5,\"thoughts\":0,\"cached\":10}}\n"
        );
        let out = parse_session_jsonl(data.as_bytes(), &cfg());
        let ids: Vec<&str> = out
            .events
            .iter()
            .map(|e| e.event.event_id.as_str())
            .collect();
        assert_eq!(ids, vec!["gemini_cli:s1:r2", "gemini_cli:s1:r3"]);
        assert_eq!(out.events[0].event.tokens.input_total, Some(260));
        assert!(out.events[0].replace_existing, "r2 被整体替换");
        assert!(!out.events[1].replace_existing, "r3 首次写入");
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("set_messages_replaced:kept:1:dropped:1")));
    }

    #[test]
    fn gemini_jsonl_rewind_truncates_current_set() {
        let data = concat!(
            "{\"sessionId\":\"s1\"}\n",
            "{\"id\":\"m1\",\"type\":\"gemini\",\"tokens\":{\"input\":100,\"output\":10,\"thoughts\":0,\"cached\":0}}\n",
            "{\"id\":\"m2\",\"type\":\"gemini\",\"tokens\":{\"input\":200,\"output\":20,\"thoughts\":0,\"cached\":0}}\n",
            "{\"id\":\"m3\",\"type\":\"gemini\",\"tokens\":{\"input\":300,\"output\":30,\"thoughts\":0,\"cached\":0}}\n",
            "{\"$rewindTo\":\"m3\"}\n",
            "{\"id\":\"m4\",\"type\":\"gemini\",\"tokens\":{\"input\":400,\"output\":40,\"thoughts\":0,\"cached\":0}}\n"
        );
        let out = parse_session_jsonl(data.as_bytes(), &cfg());
        let ids: Vec<&str> = out
            .events
            .iter()
            .map(|e| e.event.event_id.as_str())
            .collect();
        assert_eq!(
            ids,
            vec!["gemini_cli:s1:m1", "gemini_cli:s1:m2", "gemini_cli:s1:m4"]
        );
        assert!(out.diagnostics.iter().any(|d| d.contains("rewind_to:m3")));
    }

    #[test]
    fn gemini_jsonl_rewind_unknown_id_clears_all() {
        let data = concat!(
            "{\"sessionId\":\"s1\"}\n",
            "{\"id\":\"m1\",\"type\":\"gemini\",\"tokens\":{\"input\":100,\"output\":10,\"thoughts\":0,\"cached\":0}}\n",
            "{\"$rewindTo\":\"nope\"}\n"
        );
        let out = parse_session_jsonl(data.as_bytes(), &cfg());
        assert!(out.events.is_empty());
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("rewind_to_unknown_id:nope:cleared_all")));
    }

    #[test]
    fn gemini_jsonl_rewound_then_rewritten_message_is_replacement() {
        let data = concat!(
            "{\"sessionId\":\"s1\"}\n",
            "{\"id\":\"m1\",\"type\":\"gemini\",\"tokens\":{\"input\":100,\"output\":10,\"thoughts\":0,\"cached\":0}}\n",
            "{\"$rewindTo\":\"m1\"}\n",
            "{\"id\":\"m1\",\"type\":\"gemini\",\"tokens\":{\"input\":150,\"output\":15,\"thoughts\":0,\"cached\":0}}\n"
        );
        let out = parse_session_jsonl(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].event.tokens.input_total, Some(150));
        assert!(
            out.events[0].replace_existing,
            "同 id 的历史事件仍在账本里，重写按替换处理"
        );
    }

    #[test]
    fn gemini_jsonl_truncated_tail_not_consumed() {
        let data = concat!(
            "{\"sessionId\":\"s1\"}\n",
            "{\"id\":\"m1\",\"type\":\"gemini\",\"tokens\":{\"input\":64,\"output\":8,\"thoughts\":0,\"cached\":16}}\n",
            "INCOMPLETE{\"id\":\"m2\",\"type\":\"gemini\",\"tokens\":{\"input\":128"
        );
        let out = parse_session_jsonl(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1);
        let marker = data.find("INCOMPLETE").unwrap() as u64;
        assert_eq!(out.consumed_bytes, marker);
    }

    #[test]
    fn gemini_jsonl_ignores_malformed_and_unrecognized_lines() {
        let data = concat!(
            "{\"sessionId\":\"s1\"}\n",
            "not json\n",
            "{\"someOtherKey\":1}\n",
            "{\"id\":\"m1\",\"type\":\"gemini\",\"tokens\":{\"input\":10,\"output\":1,\"thoughts\":0,\"cached\":0}}\n"
        );
        let out = parse_session_jsonl(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1);
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("malformed_line_at_offset")));
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("unrecognized_record_at_offset")));
        assert_eq!(out.consumed_bytes, data.len() as u64);
    }

    #[test]
    fn gemini_identical_numbers_are_distinct_messages() {
        let tok = "{\"input\":1000,\"output\":100,\"thoughts\":0,\"cached\":0}";
        let data = format!(
            "{{\"sessionId\":\"s1\",\"messages\":[{},{}\n]}}",
            msg("a", tok).trim_end(),
            msg("b", tok).trim_end()
        );
        let out = parse_session_json(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 2, "相同数字的独立消息分别计入");
        assert_eq!(
            out.events[0].event.tokens.total,
            out.events[1].event.tokens.total
        );
        assert_ne!(out.events[0].event.event_id, out.events[1].event.event_id);
    }

    #[test]
    fn gemini_cached_exceeding_input_is_clamped_with_diagnostic() {
        let data = format!(
            "{{\"sessionId\":\"s1\",\"messages\":[{}\n]}}",
            msg("m1", "{\"input\":100,\"output\":10,\"thoughts\":5,\"cached\":400}").trim_end()
        );
        let out = parse_session_json(data.as_bytes(), &cfg());
        let t = &out.events[0].event.tokens;
        assert_eq!(t.input_cache_read, Some(100));
        assert_eq!(t.input_fresh, Some(0));
        assert_eq!(t.input_total, Some(100));
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("cached_exceeds_input")));
    }

    #[test]
    fn gemini_missing_session_id_falls_back_to_file_identity() {
        let data = format!(
            "{{\"messages\":[{}\n]}}",
            msg("m1", "{\"input\":10,\"output\":1,\"thoughts\":0,\"cached\":0}").trim_end()
        );
        let out = parse_session_json(data.as_bytes(), &cfg());
        assert_eq!(out.session_id, None);
        assert_eq!(
            out.events[0].event.event_id,
            "gemini_cli:sess-fixture.jsonl:m1"
        );
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("missing_session_id")));
    }

    #[test]
    fn gemini_event_carries_message_granularity_and_evidence() {
        let data = format!(
            "{{\"sessionId\":\"s1\",\"messages\":[{}\n]}}",
            msg("m1", "{\"input\":10,\"output\":1,\"thoughts\":2,\"cached\":0}").trim_end()
        );
        let out = parse_session_json(data.as_bytes(), &cfg());
        let e = &out.events[0].event;
        assert_eq!(e.agent, AgentKind::GeminiCli);
        assert_eq!(e.granularity, Granularity::Message);
        assert_eq!(e.request_count, Some(1));
        assert_eq!(e.completeness, Completeness::Complete);
        assert_eq!(e.occurred_at.as_deref(), Some("2026-10-11T01:00:00.000Z"));
        assert_eq!(e.evidence_refs.len(), 1);
        assert_eq!(e.evidence_refs[0].locator, "messages[0]");
        assert_eq!(e.evidence_refs[0].parser_version, PARSER_VERSION);
        assert_eq!(events(&out).len(), 1);
    }
}
