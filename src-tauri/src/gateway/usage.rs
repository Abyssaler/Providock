//! 各 wire 协议的 usage 纯解析与流式合并（网关本体不在此）。
//!
//! 语义依据：
//! - docs/research/evidence/a09-desktop-gateway.md（cc-switch `df40d53c` 的
//!   `proxy/usage/parser.rs` 与 `services/sql_helpers.rs`）
//! - docs/specs/DATA_CONTRACTS.md 第 3 节（互斥输入分量 / cached 子集不重复加）、
//!   第 4 节（物理观察与逻辑去重）、第 5 节（累计/快照不求和）
//! - docs/specs/AGENT_ADAPTERS.md A09（非流式与流式都验证，多次状态更新归为一条）
//! - docs/ARCHITECTURE.md 第 4 节（每协议 usage 解析器）与第 7 节（断流标不完整）
//!
//! 纯函数：不读文件系统、不访问网络、不注入凭据。

use crate::domain::*;
use serde_json::Value;

/// 解析器版本（写入 EvidenceRef.parser_version）。
pub const PARSER_VERSION: &str = "gateway-usage-1";

/// 已实现的 wire 协议（架构第 4 节：按已实现协议注册，不宣称覆盖全部）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GatewayProtocol {
    AnthropicMessages,
    OpenAiChatCompletions,
    OpenAiResponses,
    GeminiNative,
}

impl GatewayProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AnthropicMessages => "anthropic_messages",
            Self::OpenAiChatCompletions => "openai_chat_completions",
            Self::OpenAiResponses => "openai_responses",
            Self::GeminiNative => "gemini_native",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "anthropic_messages" => Some(Self::AnthropicMessages),
            "openai_chat_completions" => Some(Self::OpenAiChatCompletions),
            "openai_responses" => Some(Self::OpenAiResponses),
            "gemini_native" => Some(Self::GeminiNative),
            _ => None,
        }
    }
}

/// 一个 SSE 事件（`event:` 名可选 + 合并后的 `data:` 内容）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// 把原始 SSE 字节切成事件序列（只做 framing，不解释 usage）。
///
/// - `data:` 多行按 SSE 规范用 `\n` 连接；`event:`/`id:`/注释行按需处理。
/// - 块之间以空行分隔。末尾未以空行收尾的块也会被冲出（断流时最后一段
///   可能不完整，交给各协议解析函数按 JSON 解析失败优雅跳过）。
pub fn parse_sse_events(bytes: &[u8]) -> Vec<SseEvent> {
    let text = String::from_utf8_lossy(bytes);
    let mut events = Vec::new();
    let mut cur_event: Option<String> = None;
    let mut cur_data: Vec<String> = Vec::new();

    let flush = |events: &mut Vec<SseEvent>, ev: &mut Option<String>, data: &mut Vec<String>| {
        if ev.is_some() || !data.is_empty() {
            events.push(SseEvent {
                event: ev.take(),
                data: std::mem::take(data).join("\n"),
            });
        }
    };

    for raw_line in text.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() {
            flush(&mut events, &mut cur_event, &mut cur_data);
            continue;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            cur_event = Some(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            cur_data.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }
        // id:/retry:/注释行对 usage 无用，忽略。
    }
    flush(&mut events, &mut cur_event, &mut cur_data);
    events
}

/// 单个逻辑请求的合并后用量（尚未转标准事件）。
///
/// 这是中间结构，表达「相同 request 的多次状态更新归为一条」的单例语义：
/// 流式多帧合并进同一个 UsageSet，而不是逐帧产生事件或累加。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageSet {
    /// 未命中/未写缓存的输入（新鲜输入）。
    pub input_fresh: Option<u64>,
    /// 读缓存输入。
    pub input_cache_read: Option<u64>,
    /// 创建缓存输入（Anthropic 才有；其余协议为 None，不补零）。
    pub input_cache_write: Option<u64>,
    /// 输入总量。Anthropic 由三分量求和；含 cached 子集的协议取源报的 prompt/input 总量。
    pub input_total: Option<u64>,
    /// 输出总量（含附属 reasoning，若源如此）。
    pub output_total: Option<u64>,
    /// 输出中的 reasoning 细分（已含在 output_total 中，不另加）。
    pub output_reasoning: Option<u64>,
    /// 上游返回的模型名。
    pub model: Option<String>,
    /// 上游返回的响应/message id，用于与本地日志关联（关联逻辑不在本任务）。
    pub provider_request_id: Option<String>,
    pub quality: Quality,
    pub completeness: Completeness,
    pub diagnostics: Vec<String>,
}

impl Default for UsageSet {
    fn default() -> Self {
        Self {
            input_fresh: None,
            input_cache_read: None,
            input_cache_write: None,
            input_total: None,
            output_total: None,
            output_reasoning: None,
            model: None,
            provider_request_id: None,
            quality: Quality::Unknown,
            completeness: Completeness::Unknown,
            diagnostics: Vec::new(),
        }
    }
}

impl UsageSet {
    /// 是否有任何被解析出的 Token 分量。全 None 表示上游未上报 usage。
    pub fn has_any_token(&self) -> bool {
        self.input_fresh.is_some()
            || self.input_cache_read.is_some()
            || self.input_cache_write.is_some()
            || self.input_total.is_some()
            || self.output_total.is_some()
    }

    /// 转标准 UsageEvent：stream_domain=gateway，granularity=request，
    /// request_count=Some(1)。同一 UsageSet → 一条事件（不累加快照）。
    ///
    /// 上游未返回 usage 时全 Token 字段保持 null、completeness=partial——
    /// 不落「全 0 行」（上游透传落全 0 是缺陷，见证据第 5/6 节）。
    pub fn to_usage_event(&self, cfg: &GatewayUsageConfig) -> UsageEvent {
        let tokens = TokenSet {
            input_fresh: self.input_fresh,
            input_cache_read: self.input_cache_read,
            input_cache_write: self.input_cache_write,
            input_total: self.input_total,
            output_total: self.output_total,
            output_reasoning: self.output_reasoning,
            total: self
                .input_total
                .and_then(|i| self.output_total.and_then(|o| i.checked_add(o))),
            quality: self.quality,
        };

        let id_key = self
            .provider_request_id
            .clone()
            .unwrap_or_else(|| format!("{}:g{}", cfg.file_identity, cfg.generation));
        let locator = self
            .provider_request_id
            .clone()
            .unwrap_or_else(|| format!("{}:stream", cfg.protocol.as_str()));

        UsageEvent {
            schema_version: USAGE_EVENT_SCHEMA_VERSION,
            event_id: format!("gateway:{}:{}", cfg.provider, id_key),
            agent: cfg.agent,
            source_id: cfg.source_id.clone(),
            stream_domain: StreamDomain::Gateway,
            session_id: cfg.session_id.clone(),
            parent_session_id: None,
            actor_id: None,
            turn_id: None,
            project_ref: None,
            occurred_at: cfg.occurred_at.clone(),
            observed_at: cfg.observed_at.clone(),
            model_raw: self.model.clone(),
            model_normalized: None,
            granularity: Granularity::Request,
            request_count: Some(1),
            tokens,
            cost_estimated: None,
            completeness: self.completeness,
            account_link: None,
            evidence_refs: vec![EvidenceRef {
                source_id: cfg.source_id.clone(),
                file_identity: cfg.file_identity.clone(),
                generation: cfg.generation,
                locator,
                parser_version: PARSER_VERSION.into(),
            }],
        }
    }
}

/// 转事件所需的外部上下文（采集器/网关调用方提供）。
pub struct GatewayUsageConfig {
    pub source_id: String,
    /// 上游 provider 标识，用于 event_id 命名空间。
    pub provider: String,
    pub protocol: GatewayProtocol,
    /// 连接/流身份（不是可逆路径）。
    pub file_identity: String,
    pub generation: u64,
    /// 本机观察时间（RFC3339）。
    pub observed_at: String,
    /// 该网关源归属的 Agent（模型映射场景为 claude_desktop）。
    pub agent: AgentKind,
    pub session_id: Option<String>,
    /// 事件发生时间；上游响应无时间戳时保持 None（不伪造为观察时间）。
    pub occurred_at: Option<String>,
}

// ─────────────────────────── 顶层派发 ───────────────────────────

/// 非流式响应体 → UsageSet。
pub fn usage_from_response(protocol: GatewayProtocol, body: &[u8]) -> UsageSet {
    match protocol {
        GatewayProtocol::AnthropicMessages => anthropic_messages_response(body),
        GatewayProtocol::OpenAiChatCompletions => openai_chat_completions_response(body),
        GatewayProtocol::OpenAiResponses => openai_responses_response(body),
        GatewayProtocol::GeminiNative => gemini_native_response(body),
    }
}

/// SSE 字节流 → UsageSet（先切事件再按协议合并）。
pub fn usage_from_stream(protocol: GatewayProtocol, body: &[u8]) -> UsageSet {
    let events = parse_sse_events(body);
    usage_from_stream_events(protocol, &events)
}

/// SSE 事件序列 → UsageSet。
pub fn usage_from_stream_events(protocol: GatewayProtocol, events: &[SseEvent]) -> UsageSet {
    match protocol {
        GatewayProtocol::AnthropicMessages => anthropic_messages_stream(events),
        GatewayProtocol::OpenAiChatCompletions => openai_chat_completions_stream(events),
        GatewayProtocol::OpenAiResponses => openai_responses_stream(events),
        GatewayProtocol::GeminiNative => gemini_native_stream(events),
    }
}

// ─────────────────────────── Anthropic Messages ───────────────────────────

/// 非流式：`{id, model, usage:{input_tokens, output_tokens,
/// cache_read_input_tokens, cache_creation_input_tokens}}`。
///
/// Anthropic 口径：`input_tokens` 为 **fresh**（不含缓存），缓存读写单列（证据第 6 节，
/// 记 FRESH）。`input_total = fresh + read + write`。
pub fn anthropic_messages_response(body: &[u8]) -> UsageSet {
    let mut set = UsageSet::default();
    match serde_json::from_slice::<Value>(body) {
        Ok(v) => {
            set.provider_request_id = str_field(&v, "id");
            set.model = str_field(&v, "model");
            if let Some(u) = v.get("usage") {
                apply_anthropic_input(&mut set, u);
                set.output_total = u64_field(u, "output_tokens");
                set.quality = Quality::Reported;
            }
        }
        Err(_) => set
            .diagnostics
            .push("malformed_json:anthropic_messages".into()),
    }
    finalize(&mut set, GatewayProtocol::AnthropicMessages, true);
    set
}

/// 流式合并（**完成语义，不是累加**）：
/// - `message_start` → 取 `message.usage` 的 input/cache（及 model、id）。
/// - `message_delta` → 取 `usage.output_tokens` 的**最后值**（上游 delta 里是累计
///   output 快照，不是增量；证据第 2 节）。
/// - `message_stop` 为终止帧；缺失即断流 → partial。
///
/// 与上游 cc-switch 的差异：其 `from_claude_stream_events` 额外做「delta 的
/// input_tokens 更小且 > 0 时覆盖 start 的 input/缓存」的中转修正；本实现按契约
/// 只以 message_start 定 input/cache，不引入该启发式（见差异说明）。
pub fn anthropic_messages_stream(events: &[SseEvent]) -> UsageSet {
    let mut set = UsageSet::default();
    let mut terminal = false;
    for ev in events {
        let Ok(v) = serde_json::from_str::<Value>(&ev.data) else {
            continue;
        };
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
            "message_start" => {
                if let Some(msg) = v.get("message") {
                    set.provider_request_id = str_field(msg, "id").or(set.provider_request_id);
                    set.model = str_field(msg, "model").or(set.model);
                    if let Some(u) = msg.get("usage") {
                        apply_anthropic_input(&mut set, u);
                        set.quality = Quality::Reported;
                    }
                }
            }
            "message_delta" => {
                if let Some(u) = v.get("usage") {
                    if let Some(o) = u64_field(u, "output_tokens") {
                        set.output_total = Some(o);
                        set.quality = Quality::Reported;
                    }
                }
            }
            "message_stop" => terminal = true,
            _ => {}
        }
    }
    finalize(&mut set, GatewayProtocol::AnthropicMessages, terminal);
    set
}

/// Anthropic input 三分量（fresh/read/write 分列，互斥求和）。
fn apply_anthropic_input(set: &mut UsageSet, u: &Value) {
    set.input_fresh = u64_field(u, "input_tokens");
    set.input_cache_read = u64_field(u, "cache_read_input_tokens");
    set.input_cache_write = u64_field(u, "cache_creation_input_tokens");
    // 注意：cache_creation 的 TTL 细分（ephemeral_1h 等）是 write 的子集，不另加。
    set.input_total = match (set.input_fresh, set.input_cache_read, set.input_cache_write) {
        (Some(f), Some(r), Some(w)) => f.checked_add(r).and_then(|s| s.checked_add(w)),
        _ => None,
    };
    if set.input_total.is_none() && set.input_fresh.is_some() {
        set.diagnostics
            .push("anthropic_input_components_incomplete:anthropic_messages".into());
    }
}

// ─────────────────────── OpenAI Chat Completions ───────────────────────

/// 非流式：`{id, model, usage:{prompt_tokens, completion_tokens,
/// prompt_tokens_details.cached_tokens, completion_tokens_details.reasoning_tokens}}`。
///
/// 口径：prompt 含 cached 子集 → input_total=prompt_tokens、cache_read=cached、
/// fresh=prompt−cached（不补成 prompt+cached）；completion 含 reasoning 子集 →
/// output_total=completion_tokens、output_reasoning=reasoning 不另加。
pub fn openai_chat_completions_response(body: &[u8]) -> UsageSet {
    let mut set = UsageSet::default();
    match serde_json::from_slice::<Value>(body) {
        Ok(v) => {
            set.provider_request_id = str_field(&v, "id");
            set.model = str_field(&v, "model");
            if let Some(u) = v.get("usage") {
                apply_openai_usage(&mut set, u);
            }
        }
        Err(_) => set
            .diagnostics
            .push("malformed_json:openai_chat_completions".into()),
    }
    finalize(&mut set, GatewayProtocol::OpenAiChatCompletions, true);
    set
}

/// 流式：取**最后一个带非空 `usage` 的 chunk**（`stream_options.include_usage`
/// 的末快照，不累加）；`data: [DONE]` 为终止帧。
pub fn openai_chat_completions_stream(events: &[SseEvent]) -> UsageSet {
    let mut set = UsageSet::default();
    let mut terminal = false;
    for ev in events {
        if ev.data.trim() == "[DONE]" {
            terminal = true;
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&ev.data) else {
            continue;
        };
        set.provider_request_id = str_field(&v, "id").or(set.provider_request_id);
        set.model = str_field(&v, "model").or(set.model);
        if let Some(u) = v.get("usage") {
            if !u.is_null() {
                apply_openai_usage(&mut set, u);
            }
        }
    }
    finalize(&mut set, GatewayProtocol::OpenAiChatCompletions, terminal);
    set
}

/// OpenAI 口径（prompt 含 cached 子集）的通用输入映射。
fn apply_openai_usage(set: &mut UsageSet, u: &Value) {
    let prompt = u64_field(u, "prompt_tokens");
    let cached = u
        .get("prompt_tokens_details")
        .and_then(|d| u64_field(d, "cached_tokens"));
    apply_subset_input(set, prompt, cached, "openai_chat_completions");

    set.output_total = u64_field(u, "completion_tokens");
    set.output_reasoning = u
        .get("completion_tokens_details")
        .and_then(|d| u64_field(d, "reasoning_tokens"));
    if set.input_total.is_some() || set.output_total.is_some() {
        set.quality = Quality::Reported;
    }
}

// ─────────────────────────── OpenAI Responses ───────────────────────────

/// 非流式 `response` 对象：`usage{input_tokens, input_tokens_details.cached_tokens,
/// output_tokens, output_tokens_details.reasoning_tokens}`。子集语义同 Chat。
pub fn openai_responses_response(body: &[u8]) -> UsageSet {
    let mut set = UsageSet::default();
    match serde_json::from_slice::<Value>(body) {
        Ok(v) => {
            set.provider_request_id = str_field(&v, "id");
            set.model = str_field(&v, "model");
            if let Some(u) = v.get("usage") {
                apply_responses_usage(&mut set, u);
            }
        }
        Err(_) => set
            .diagnostics
            .push("malformed_json:openai_responses".into()),
    }
    finalize(&mut set, GatewayProtocol::OpenAiResponses, true);
    set
}

/// 流式：取 `response.completed` 事件内 `response.usage` 的最终快照；该事件即终止帧。
pub fn openai_responses_stream(events: &[SseEvent]) -> UsageSet {
    let mut set = UsageSet::default();
    let mut terminal = false;
    for ev in events {
        let Ok(v) = serde_json::from_str::<Value>(&ev.data) else {
            continue;
        };
        let typ = v.get("type").and_then(Value::as_str).unwrap_or("");
        if let Some(resp) = v.get("response") {
            set.provider_request_id = str_field(resp, "id").or(set.provider_request_id);
            set.model = str_field(resp, "model").or(set.model);
            if typ == "response.completed" {
                if let Some(u) = resp.get("usage") {
                    apply_responses_usage(&mut set, u);
                }
                terminal = true;
            }
        }
    }
    finalize(&mut set, GatewayProtocol::OpenAiResponses, terminal);
    set
}

fn apply_responses_usage(set: &mut UsageSet, u: &Value) {
    let input = u64_field(u, "input_tokens");
    let cached = u
        .get("input_tokens_details")
        .and_then(|d| u64_field(d, "cached_tokens"));
    apply_subset_input(set, input, cached, "openai_responses");

    set.output_total = u64_field(u, "output_tokens");
    set.output_reasoning = u
        .get("output_tokens_details")
        .and_then(|d| u64_field(d, "reasoning_tokens"));
    if set.input_total.is_some() || set.output_total.is_some() {
        set.quality = Quality::Reported;
    }
}

// ─────────────────────────── Gemini Native ───────────────────────────

/// 非流式：`usageMetadata{promptTokenCount, candidatesTokenCount,
/// cachedContentTokenCount, thoughtsTokenCount}` + `modelVersion` + `responseId`。
///
/// 口径：cached ⊂ prompt；`output_total = candidatesTokenCount + thoughtsTokenCount`
/// （**derived**：Gemini 把 thoughts 与 candidates 分列，本实现按证据累加；
/// output_reasoning=thoughts 已含在内，不另加）。
pub fn gemini_native_response(body: &[u8]) -> UsageSet {
    let mut set = UsageSet::default();
    match serde_json::from_slice::<Value>(body) {
        Ok(v) => {
            set.provider_request_id = str_field(&v, "responseId").or_else(|| str_field(&v, "id"));
            set.model = str_field(&v, "modelVersion");
            if let Some(u) = v.get("usageMetadata") {
                apply_gemini_usage(&mut set, u);
            }
        }
        Err(_) => set.diagnostics.push("malformed_json:gemini_native".into()),
    }
    finalize(&mut set, GatewayProtocol::GeminiNative, true);
    set
}

/// 流式：取最后一个 chunk 的 `usageMetadata`（累计快照，不求和）；
/// 终止帧为出现 `candidates[].finishReason`。
pub fn gemini_native_stream(events: &[SseEvent]) -> UsageSet {
    let mut set = UsageSet::default();
    let mut terminal = false;
    for ev in events {
        let Ok(v) = serde_json::from_str::<Value>(&ev.data) else {
            continue;
        };
        set.provider_request_id = str_field(&v, "responseId")
            .or_else(|| str_field(&v, "id"))
            .or(set.provider_request_id);
        set.model = str_field(&v, "modelVersion").or(set.model);
        if let Some(u) = v.get("usageMetadata") {
            apply_gemini_usage(&mut set, u);
        }
        if let Some(cands) = v.get("candidates").and_then(Value::as_array) {
            if cands
                .iter()
                .any(|c| c.get("finishReason").and_then(Value::as_str).is_some())
            {
                terminal = true;
            }
        }
    }
    finalize(&mut set, GatewayProtocol::GeminiNative, terminal);
    set
}

fn apply_gemini_usage(set: &mut UsageSet, u: &Value) {
    let prompt = u64_field(u, "promptTokenCount");
    let cached = u64_field(u, "cachedContentTokenCount");
    apply_subset_input(set, prompt, cached, "gemini_native");

    let candidates = u64_field(u, "candidatesTokenCount");
    let thoughts = u64_field(u, "thoughtsTokenCount");
    set.output_reasoning = thoughts;
    // thoughts 分列时与 candidates 相加得到输出总量；缺 thoughts 视为该协议未分列
    // （Gemini 为 0 时通常省略该字段），但也缺 candidates 则输出未知。
    set.output_total = match candidate_sum(candidates, thoughts) {
        Ok(v) => v,
        Err(_) => {
            set.diagnostics
                .push("gemini_output_overflow:gemini_native".into());
            None
        }
    };
    if set.input_total.is_some() || set.output_total.is_some() {
        // output_total 是 candidates+thoughts 的推导，标 derived（证据级别说明）。
        set.quality = Quality::Derived;
    }
}

fn candidate_sum(candidates: Option<u64>, thoughts: Option<u64>) -> Result<Option<u64>, ()> {
    match (candidates, thoughts) {
        (Some(c), Some(t)) => c.checked_add(t).map(Some).ok_or(()),
        (Some(c), None) => Ok(Some(c)),
        (None, _) => Ok(None),
    }
}

// ─────────────────────────── 共用工具 ───────────────────────────

/// 含 cached 子集的输入映射：input_total=源报总量，cache_read=cached，
/// fresh=总量−cached。总量冲突（cached>总量）时保留 read 并记诊断，fresh 留空。
fn apply_subset_input(set: &mut UsageSet, total: Option<u64>, cached: Option<u64>, protocol: &str) {
    set.input_total = total;
    match (total, cached) {
        (Some(t), Some(c)) if c <= t => {
            set.input_cache_read = Some(c);
            set.input_fresh = Some(t - c);
        }
        (Some(t), Some(c)) => {
            set.input_cache_read = Some(c);
            set.diagnostics.push(format!(
                "cached_exceeds_input_total:{protocol}:total={t},cached={c}"
            ));
        }
        (_, Some(c)) => set.input_cache_read = Some(c),
        (_, None) => {}
    }
}

fn u64_field(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// 完成语义收尾：有用量且见到终止帧 → complete；否则 partial。
/// 缺终止帧 / 上游未上报 usage 分别记诊断；全 None 时质量回落到 unknown。
fn finalize(set: &mut UsageSet, protocol: GatewayProtocol, terminal_seen: bool) {
    let has = set.has_any_token();
    if has && terminal_seen {
        set.completeness = Completeness::Complete;
        return;
    }
    set.completeness = Completeness::Partial;
    if !terminal_seen {
        set.diagnostics.push(format!(
            "stream_missing_terminal_frame:{}",
            protocol.as_str()
        ));
    }
    if !has {
        set.diagnostics
            .push(format!("usage_absent:{}", protocol.as_str()));
        set.quality = Quality::Unknown;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> GatewayUsageConfig {
        GatewayUsageConfig {
            source_id: "gw-src".into(),
            provider: "anthropic-relay".into(),
            protocol: GatewayProtocol::AnthropicMessages,
            file_identity: "conn-1".into(),
            generation: 0,
            observed_at: "2026-10-11T08:00:00Z".into(),
            agent: AgentKind::ClaudeDesktop,
            session_id: Some("sess-1".into()),
            occurred_at: None,
        }
    }

    #[test]
    fn anthropic_stream_takes_last_output_delta_not_sum() {
        let data = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"model\":\"m\",\"usage\":{\"input_tokens\":100,\"cache_read_input_tokens\":20,\"cache_creation_input_tokens\":5,\"output_tokens\":1}}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":40}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":77}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        );
        let set = anthropic_messages_stream(&parse_sse_events(data.as_bytes()));
        assert_eq!(set.input_fresh, Some(100));
        assert_eq!(set.input_cache_read, Some(20));
        assert_eq!(set.input_cache_write, Some(5));
        assert_eq!(set.input_total, Some(125));
        assert_eq!(set.output_total, Some(77), "取最后值不是 1+40+77");
        assert_eq!(set.completeness, Completeness::Complete);
    }

    #[test]
    fn anthropic_stream_truncated_is_partial_with_values_kept() {
        let data = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_2\",\"model\":\"m\",\"usage\":{\"input_tokens\":50,\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0}}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":9}}\n\n",
        );
        let set = anthropic_messages_stream(&parse_sse_events(data.as_bytes()));
        assert_eq!(set.completeness, Completeness::Partial);
        assert_eq!(set.output_total, Some(9));
        assert!(set
            .diagnostics
            .iter()
            .any(|d| d.contains("stream_missing_terminal_frame")));
    }

    #[test]
    fn openai_subset_input_does_not_double_count_cached() {
        let body = br#"{"id":"c1","model":"gpt","usage":{"prompt_tokens":1000,"completion_tokens":250,"prompt_tokens_details":{"cached_tokens":400},"completion_tokens_details":{"reasoning_tokens":60}}}"#;
        let set = openai_chat_completions_response(body);
        assert_eq!(set.input_total, Some(1000));
        assert_eq!(set.input_cache_read, Some(400));
        assert_eq!(set.input_fresh, Some(600), "1000-400，不是 1400");
        assert_eq!(set.output_total, Some(250));
        assert_eq!(set.output_reasoning, Some(60), "reasoning 是子集不另加");
        assert_eq!(set.completeness, Completeness::Complete);
    }

    #[test]
    fn openai_stream_uses_last_usage_chunk() {
        let data = concat!(
            "data: {\"id\":\"c2\",\"model\":\"gpt\",\"choices\":[{\"delta\":{\"content\":\"hi\"}}],\"usage\":null}\n\n",
            "data: {\"id\":\"c2\",\"model\":\"gpt\",\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5}}\n\n",
            "data: {\"id\":\"c2\",\"model\":\"gpt\",\"choices\":[],\"usage\":{\"prompt_tokens\":30,\"completion_tokens\":7,\"prompt_tokens_details\":{\"cached_tokens\":10}}}\n\n",
            "data: [DONE]\n\n",
        );
        let set = openai_chat_completions_stream(&parse_sse_events(data.as_bytes()));
        assert_eq!(set.input_total, Some(30), "末 chunk 快照");
        assert_eq!(set.input_fresh, Some(20));
        assert_eq!(set.output_total, Some(7));
        assert_eq!(set.completeness, Completeness::Complete);
    }

    #[test]
    fn gemini_output_is_candidates_plus_thoughts_derived() {
        let body = br#"{"candidates":[{"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":1000,"candidatesTokenCount":150,"thoughtsTokenCount":50,"cachedContentTokenCount":200,"totalTokenCount":1200},"modelVersion":"gemini-2.5-pro","responseId":"g1"}"#;
        let set = gemini_native_response(body);
        assert_eq!(set.input_fresh, Some(800));
        assert_eq!(set.output_total, Some(200), "150+50");
        assert_eq!(set.output_reasoning, Some(50));
        assert_eq!(set.quality, Quality::Derived);
    }

    #[test]
    fn no_usage_is_null_partial_not_zero_row() {
        let body = br#"{"id":"resp_x","model":"gpt-5","status":"completed"}"#;
        let set = openai_responses_response(body);
        assert!(!set.has_any_token());
        assert_eq!(set.completeness, Completeness::Partial);
        assert_eq!(set.quality, Quality::Unknown);
        let event = set.to_usage_event(&cfg());
        assert_eq!(event.tokens.input_fresh, None);
        assert_eq!(event.tokens.output_total, None);
        assert_eq!(event.tokens.total, None);
        assert_eq!(event.request_count, Some(1));
        assert_eq!(event.completeness, Completeness::Partial);
    }

    #[test]
    fn to_usage_event_marks_gateway_and_request_granularity() {
        let body = br#"{"id":"msg_42","model":"claude-x","usage":{"input_tokens":10,"output_tokens":3,"cache_read_input_tokens":1,"cache_creation_input_tokens":2}}"#;
        let event = anthropic_messages_response(body).to_usage_event(&cfg());
        assert_eq!(event.stream_domain, StreamDomain::Gateway);
        assert_eq!(event.granularity, Granularity::Request);
        assert_eq!(event.request_count, Some(1));
        assert_eq!(event.agent, AgentKind::ClaudeDesktop);
        assert_eq!(event.tokens.input_total, Some(13));
        assert_eq!(event.tokens.total, Some(16));
        assert!(event.event_id.contains("msg_42"));
        assert_eq!(event.tokens.quality, Quality::Reported);
    }

    #[test]
    fn malformed_body_does_not_panic() {
        let set = usage_from_response(GatewayProtocol::GeminiNative, b"{not json");
        assert!(!set.has_any_token());
        assert_eq!(set.completeness, Completeness::Partial);
        assert!(set.diagnostics.iter().any(|d| d.contains("malformed_json")));
    }
}
