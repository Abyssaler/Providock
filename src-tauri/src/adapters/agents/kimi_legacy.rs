//! A08 附属：旧版 Python `kimi-cli`（`MoonshotAI/kimi-cli`，已归档）`wire.jsonl` 纯解析器。
//!
//! 证据：docs/research/evidence/a08-kimi-official.md 第 8 节与「补充核实（2026-10-11）」小节，
//! 全部来自 `kimi-cli@9ab1286`（`main`）实读源码。
//!
//! 布局与文件形状：
//! - `<KIMI_SHARE_DIR 或 ~/.kimi>/sessions/<md5(workdir)>/<session uuid>/`，内含
//!   `context.jsonl`、`wire.jsonl`、`state.json`、`subagents/<agent_id>/`（`session.py`）。
//!   session_id 取目录 uuid；旧版没有 `session_index.jsonl`。
//! - 首行 `{"type":"metadata","protocol_version":"<v>"}`，其后每行
//!   `{timestamp: <浮点秒>, message: {type, payload}}`。payload 嵌在 `message` 下，
//!   时间单位是**秒**（浮点）——都与新版 1.5 的「payload 展开在顶层 + time 毫秒」不同。
//! - 版本常量 `WIRE_PROTOCOL_VERSION = "1.10"`、`WIRE_PROTOCOL_LEGACY_VERSION = "1.1"`
//!   （`wire/protocol.py`）。文件存在但缺首行 metadata 时，上游 reader 把版本退化为 1.1
//!   （`wire/file.py`）；本解析器改为「无法确认版本即不解析」（`MissingMetadata`），
//!   不沿用上游的降级默认，也不猜。
//!
//! 用量形状（本任务要求确认的部分）：
//! - 用量只出现在 `StatusUpdate.token_usage`，且**每条记录是该 step 的绝对值**，不是累计：
//!   `soul/kimisoul.py:1261-1273` 在每次成功的 LLM step 之后发一条
//!   `StatusUpdate(token_usage=usage, message_id=result.id, plan_mode=...)`；
//!   字段语义见 `wire/types.py:176-195` 的 "The token usage statistics of the current step."
//!   与 `packages/kosong/src/kosong/chat_provider/__init__.py` 的 `TokenUsage`。
//!   失败/重试的 step 在抛错前不会发这条记录 → 那些尝试的消耗在 wire 里**不存在**（不是 0）。
//! - 因此粒度是：**一个成功 step 一条记录 = 一条 request 事件**（granularity=request，
//!   request_count=1）。同一 step 只有一条，不需要也不能按调用再聚合；会话总量由读取方
//!   求和——旧版自己的可视化就是这么做的（`vis/api/statistics.py:134-144` 逐条累加
//!   `input_other + input_cache_read + input_cache_creation` 与 `output`）。
//! - 四分量映射（snake_case）：`input_other`→input_fresh、`input_cache_read`→
//!   input_cache_read、`input_cache_creation`→input_cache_write、`output`→output_total；
//!   input_total = 三者之和，total = input_total + output。`input_other` 是**不含缓存**的
//!   输入（与新版 inputOther 同名同义）。
//! - `token_usage` 为 null 的 StatusUpdate **不产生事件**：旧版语义里 None 是「与上一状态
//!   相同/无更新」（`wire/types.py` 类注释），而 plan_mode、mcp_status、/compact、/clear
//!   的上下文测量都发同形状记录；单看记录无法与「step 完成但上游未报 usage」区分，
//!   不猜（诊断计数 `status_without_token_usage`）。四项全零的 token_usage 同样不制造
//!   零消耗事件（与 A01 Claude Code 适配器一致）。
//! - `context_usage`/`context_tokens`/`max_context_tokens` 是上下文测量，不是消费，
//!   永不计入（迁移工具写出的 `token_counting.*` 同理，本解析器根本不认那些类型）。
//! - 旧版 wire **不记录模型名**：`wire/types.py` 的全部消息类型都没有 model 字段 →
//!   model_raw 保持 null，不猜。turn_id 同理：TurnBegin 只有 user_input，StepBegin 只有
//!   步号 n，没有轮次 id → turn_id 保持 null，不伪造身份。
//!
//! 子 Agent（同一消耗的两处物理观察）：
//! - `subagents/<agent_id>/wire.jsonl` 是独立的同格式文件（`subagents/store.py`、
//!   `runner.py` 把 WireFile 指向它），本 Agent 自己的 StatusUpdate 就在其中。
//! - 同一份子 Agent 消息还会被包成
//!   `SubagentEvent{parent_tool_call_id, agent_id, subagent_type, event:<envelope>}`
//!   镜像写进父 wire（`subagents/runner.py`），且可多层嵌套（`vis/api/sessions.py` 递归解包）。
//!   父 tool call id 为空时不镜像，子 Agent 自有 wire 是唯一留存证据。
//! - 默认以「各 Agent 自有 wire」为准：`count_mirrored_subagent_usage=false` 跳过镜像，
//!   调用方枚举全部 `agents/*/wire.jsonl` 即每个 Agent 恰好计入一次；
//!   `mirrored_in_parent=true` 用于子 Agent 自有文件（其消耗已由父镜像计入，本文件不重复）。
//!   计入镜像时 actor_id 取**最内层** agent_id（嵌套镜像的归属）。
//!
//! fork / undo：
//! - 旧版**有** fork：`/fork` 复制整份 wire，`/undo` 复制到目标轮次为止的前缀，都写进新
//!   会话目录（`session_fork.py`）。但 **wire 内没有任何 fork 标记**，唯一线索是
//!   `state.json.custom_title` 的前缀约定（`"Fork: "` / `"Undo: "`）与 `wire_mtime`。
//! - 因此单调解析无法自行识别继承前缀。调用方做前缀指纹比对后给出边界
//!   （`ForkContext::Inherited`）；边界不可确认、或复制源不在采集范围时，全部保留并记诊断，
//!   不静默丢弃（契约第 4 节：只有可证实的共享祖先才归并）。
//!
//! 其他：
//! - 身份 = 文件身份 + 代数 + 字节偏移（append-only 文件内稳定，重扫幂等；相同数值的独立
//!   step 偏移不同，不会被误判重复）。
//! - 尾行未以换行结尾视为未写完，不消费；畸形行记诊断、继续，不崩溃。
//!
//! 纯函数：不读文件系统、不访问网络与凭据。

use crate::domain::*;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub const PARSER_VERSION: &str = "kimi-legacy-1.10";
/// 支持的最高协议版本（`1.10` 族：主版本 1、次版本 ≤ 10；含历史遗留 `1.1`）。
/// 更高版本标记不兼容，不猜测。
pub const SUPPORTED_MAX_PROTOCOL: &str = "1.10";
/// 嵌套 SubagentEvent 的解包深度上限（防止畸形文件造成无限递归）。
const MAX_SUBAGENT_NESTING: usize = 8;

/// 跨会话完整前缀复制（旧版 `/fork`、`/undo`）的继承上下文，由调用方按证据给出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForkContext {
    /// 未识别为分叉复制（默认）。
    None,
    /// 已确认继承前缀的结束位置：`prefix_end` 是**第一个非继承行的起始字节偏移**，
    /// 之前（不含）的记录视为从复制源继承，跳过并记诊断。
    Inherited { prefix_end: u64 },
    /// 识别为分叉复制、复制源也在采集范围内，但无法确认前缀边界
    /// （旧版 wire 无标记）→ 全部保留并记诊断。
    Unresolved,
    /// 识别为分叉复制，但复制源不在采集范围 → 全部计入并记诊断
    /// （这些记录是唯一的留存证据）。
    SourceUnavailable,
}

pub struct WireParseConfig {
    pub source_id: String,
    pub file_identity: String,
    pub generation: u64,
    /// 会话身份 = `<bucket>/<session_uuid>` 的目录 uuid。
    pub session_id: String,
    /// 目录给出的 Agent 身份：主 Agent 为 `main`，子 Agent 为 `subagents/<id>` 的 `<id>`。
    /// 记录内的 `SubagentEvent.agent_id`（若有）优先。
    pub agent_id: String,
    /// 本文件是子 Agent 自有 wire，且其消耗已由父 wire 的 SubagentEvent 镜像计入
    /// （调用方判定）→ 本文件的消耗事件跳过，只记诊断，避免同一消耗计两遍。
    pub mirrored_in_parent: bool,
    /// 是否计入本文件内 SubagentEvent 镜像的子 Agent 用量。默认 false：以各 Agent 自有
    /// wire 为准；仅当某个子 Agent 文件不可达、只能从父镜像恢复时才置 true。
    pub count_mirrored_subagent_usage: bool,
    /// 完整前缀复制的继承上下文。
    pub fork: ForkContext,
    /// 本机观察时间（RFC3339）。
    pub observed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compatibility {
    Ok,
    UnsupportedProtocol(String),
    MissingMetadata,
}

pub struct WireParseOutcome {
    pub compatibility: Compatibility,
    pub events: Vec<UsageEvent>,
    /// 已消费字节数（最后一个完整换行之后）。崩溃/截断尾行不计入。
    pub consumed_bytes: u64,
    pub diagnostics: Vec<String>,
}

pub fn parse_wire(bytes: &[u8], cfg: &WireParseConfig) -> WireParseOutcome {
    let mut diagnostics = Vec::new();
    let mut events = Vec::new();

    // 按行迭代并记录每行起始字节偏移；只消费以 \n 结尾的行。
    let mut lines: Vec<(u64, &[u8])> = Vec::new();
    let mut start = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            lines.push((start as u64, &bytes[start..i]));
            start = i + 1;
        }
    }
    let consumed_bytes = start as u64;

    if lines.is_empty() {
        return WireParseOutcome {
            compatibility: Compatibility::MissingMetadata,
            events,
            consumed_bytes,
            diagnostics: vec!["empty_or_unterminated_wire".into()],
        };
    }

    // 协议头：必须是首行的 metadata。
    let compatibility = match parse_metadata(lines[0].1) {
        Some(v) if version_supported(&v) => Compatibility::Ok,
        Some(v) => Compatibility::UnsupportedProtocol(v),
        None => Compatibility::MissingMetadata,
    };
    if compatibility != Compatibility::Ok {
        diagnostics.push(format!("wire_compatibility:{compatibility:?}"));
        return WireParseOutcome {
            compatibility,
            events,
            consumed_bytes: lines[0].0 + lines[0].1.len() as u64 + 1,
            diagnostics,
        };
    }

    let fork_prefix_end = match &cfg.fork {
        ForkContext::Inherited { prefix_end } => Some(*prefix_end),
        ForkContext::Unresolved => {
            diagnostics.push(
                "fork_boundary_unknown: 旧版 wire 无 fork 标记，继承前缀边界无法确认，全部保留并标记"
                    .into(),
            );
            None
        }
        ForkContext::SourceUnavailable => {
            diagnostics.push(
                "fork_source_missing: 复制源（父会话）不在采集范围，完整前缀复制的记录作为唯一留存证据计入"
                    .into(),
            );
            None
        }
        ForkContext::None => None,
    };

    let mut inherited_skipped: u64 = 0;
    let mut own_skipped_mirrored: u64 = 0;
    let mut mirrors_skipped: u64 = 0;
    let mut mirrored_counted: u64 = 0;
    let mut status_without_usage: u64 = 0;
    let mut zero_usage_skipped: u64 = 0;

    for (offset, line) in &lines[1..] {
        let offset = *offset;
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(line) else {
            diagnostics.push(format!("malformed_line_at_offset:{offset}"));
            continue;
        };
        if fork_prefix_end.is_some_and(|end| offset < end) {
            inherited_skipped += 1;
            continue;
        }
        let Some(message) = v.get("message") else {
            diagnostics.push(format!("malformed_record_at_offset:{offset}: 缺 message"));
            continue;
        };
        let Some(msg_type) = message.get("type").and_then(|t| t.as_str()) else {
            diagnostics.push(format!(
                "malformed_record_at_offset:{offset}: 缺 message.type"
            ));
            continue;
        };
        let empty = serde_json::Value::Null;
        let payload = message.get("payload").unwrap_or(&empty);

        match msg_type {
            "StatusUpdate" => match status_usage(payload) {
                StatusUsage::Absent => status_without_usage += 1,
                StatusUsage::Zero => zero_usage_skipped += 1,
                StatusUsage::Usage(u) => {
                    if cfg.mirrored_in_parent {
                        own_skipped_mirrored += 1;
                    } else {
                        push_event(
                            &mut events,
                            &mut diagnostics,
                            offset,
                            u,
                            cfg,
                            cfg.agent_id.as_str(),
                            v.get("timestamp").and_then(|t| t.as_f64()),
                        );
                    }
                }
                StatusUsage::Invalid(msg) => {
                    diagnostics.push(format!("token_usage_invalid_at_offset:{offset}: {msg}"))
                }
            },
            "SubagentEvent" => {
                if cfg.count_mirrored_subagent_usage {
                    let mut found = Vec::new();
                    collect_mirrored(
                        payload,
                        cfg.agent_id.as_str(),
                        0,
                        &mut found,
                        &mut diagnostics,
                    );
                    for (actor, inner_payload) in found {
                        match status_usage(inner_payload) {
                            StatusUsage::Absent => status_without_usage += 1,
                            StatusUsage::Zero => zero_usage_skipped += 1,
                            StatusUsage::Usage(u) => {
                                push_event(
                                    &mut events,
                                    &mut diagnostics,
                                    offset,
                                    u,
                                    cfg,
                                    actor.as_deref().unwrap_or(cfg.agent_id.as_str()),
                                    v.get("timestamp").and_then(|t| t.as_f64()),
                                );
                                mirrored_counted += 1;
                            }
                            StatusUsage::Invalid(msg) => diagnostics
                                .push(format!("token_usage_invalid_at_offset:{offset}: {msg}")),
                        }
                    }
                } else {
                    mirrors_skipped += 1;
                }
            }
            _ => {}
        }
    }

    if inherited_skipped > 0 {
        diagnostics.push(format!(
            "fork_inherited_skipped:{inherited_skipped}（复制源已计，不重复）"
        ));
    }
    if own_skipped_mirrored > 0 {
        diagnostics.push(format!(
            "subagent_mirrored_in_parent:{own_skipped_mirrored}（父 wire 的 SubagentEvent 镜像已计，本文件不重复）"
        ));
    }
    if mirrors_skipped > 0 {
        diagnostics.push(format!(
            "mirrored_subagent_records_skipped:{mirrors_skipped}（以各子 Agent 自有 wire.jsonl 为准）"
        ));
    }
    if mirrored_counted > 0 {
        diagnostics.push(format!(
            "mirrored_subagent_records_counted:{mirrored_counted}（子 Agent 自有 wire 不可达，从父镜像恢复）"
        ));
    }
    if status_without_usage > 0 {
        diagnostics.push(format!(
            "status_without_token_usage:{status_without_usage}（计划模式/上下文测量等状态更新，不含用量）"
        ));
    }
    if zero_usage_skipped > 0 {
        diagnostics.push(format!(
            "zero_token_usage_skipped:{zero_usage_skipped}（四分量全零，不制造零消耗事件）"
        ));
    }

    WireParseOutcome {
        compatibility,
        events,
        consumed_bytes,
        diagnostics,
    }
}

/// 一个 StatusUpdate 的用量解析结果。
enum StatusUsage {
    /// token_usage 缺失或为 null：按旧版语义是「无更新」，不产生事件。
    Absent,
    /// 四项全零：不制造零消耗事件。
    Zero,
    /// 非零的四分量。
    Usage(TokenUsage),
    /// 字段非法（缺必填项、负数、类型错、溢出）。
    Invalid(String),
}

struct TokenUsage {
    input_fresh: u64,
    input_cache_read: u64,
    input_cache_write: u64,
    output: u64,
    input_total: u64,
    total: u64,
    /// 缓存分量缺失、按上游模型默认 0 补齐（证据说正常 dump 四项齐全，这是防御性路径）。
    cache_field_defaulted: bool,
}

fn status_usage(payload: &serde_json::Value) -> StatusUsage {
    let Some(usage) = payload.get("token_usage") else {
        return StatusUsage::Absent;
    };
    if usage.is_null() {
        return StatusUsage::Absent;
    }
    let Some(obj) = usage.as_object() else {
        return StatusUsage::Invalid("token_usage 不是对象".into());
    };

    // input_other / output 是上游 TokenUsage 的必填字段，缺失即视为畸形，不补值。
    let input_fresh = match number_as_u64(obj, "input_other") {
        Err(e) => return StatusUsage::Invalid(e),
        Ok(None) => return StatusUsage::Invalid("token_usage.input_other 缺失".into()),
        Ok(Some(v)) => v,
    };
    let output = match number_as_u64(obj, "output") {
        Err(e) => return StatusUsage::Invalid(e),
        Ok(None) => return StatusUsage::Invalid("token_usage.output 缺失".into()),
        Ok(Some(v)) => v,
    };
    // 缓存分量在上游模型里带默认值 0（"For now, only Anthropic API supports this."），
    // 缺失/null 时按该声明补 0，并留下诊断标记；负数或类型错仍然报错。
    let mut cache_field_defaulted = false;
    let mut cache_field = |key: &str| -> Result<u64, String> {
        match number_as_u64(obj, key) {
            Err(e) => Err(e),
            Ok(None) => {
                cache_field_defaulted = true;
                Ok(0)
            }
            Ok(Some(v)) => Ok(v),
        }
    };
    let input_cache_read = match cache_field("input_cache_read") {
        Ok(v) => v,
        Err(e) => return StatusUsage::Invalid(e),
    };
    let input_cache_write = match cache_field("input_cache_creation") {
        Ok(v) => v,
        Err(e) => return StatusUsage::Invalid(e),
    };
    drop(cache_field);
    if [input_fresh, input_cache_read, input_cache_write, output]
        .iter()
        .all(|v| *v == 0)
    {
        return StatusUsage::Zero;
    }

    let Some(input_total) = input_fresh
        .checked_add(input_cache_read)
        .and_then(|s| s.checked_add(input_cache_write))
    else {
        return StatusUsage::Invalid("输入分量求和溢出".into());
    };
    let Some(total) = input_total.checked_add(output) else {
        return StatusUsage::Invalid("总 token 求和溢出".into());
    };
    StatusUsage::Usage(TokenUsage {
        input_fresh,
        input_cache_read,
        input_cache_write,
        output,
        input_total,
        total,
        cache_field_defaulted,
    })
}

fn push_event(
    events: &mut Vec<UsageEvent>,
    diagnostics: &mut Vec<String>,
    offset: u64,
    u: TokenUsage,
    cfg: &WireParseConfig,
    actor: &str,
    timestamp: Option<f64>,
) {
    if u.cache_field_defaulted {
        diagnostics.push(format!(
            "token_usage_cache_field_defaulted_at_offset:{offset}（上游模型默认 0）"
        ));
    }
    let occurred_at = timestamp.and_then(ts_to_rfc3339);
    if occurred_at.is_none() {
        diagnostics.push(format!("occurred_at_unavailable_at_offset:{offset}"));
    }
    events.push(UsageEvent {
        schema_version: USAGE_EVENT_SCHEMA_VERSION,
        // 身份 = 文件身份 + 代数 + 字节偏移：append-only 文件内稳定，重扫/重启幂等；
        // 数值相同的独立 step 偏移不同，不会被误判重复。
        event_id: format!(
            "kimi_legacy:{}:g{}:o{}",
            cfg.file_identity, cfg.generation, offset
        ),
        agent: AgentKind::KimiCode,
        source_id: cfg.source_id.clone(),
        stream_domain: StreamDomain::LocalLog,
        session_id: Some(cfg.session_id.clone()),
        // 旧版 wire 不记录复制源会话 id（fork 也无标记）→ 不猜。
        parent_session_id: None,
        actor_id: Some(actor.to_string()),
        // 旧版没有轮次 id（TurnBegin 只有 user_input，StepBegin 只有步号）→ 不伪造。
        turn_id: None,
        project_ref: None,
        occurred_at,
        observed_at: cfg.observed_at.clone(),
        // 旧版 wire 的任何消息类型都没有 model 字段 → 不猜。
        model_raw: None,
        model_normalized: None,
        granularity: Granularity::Request,
        request_count: Some(1),
        tokens: TokenSet {
            input_fresh: Some(u.input_fresh),
            input_cache_read: Some(u.input_cache_read),
            input_cache_write: Some(u.input_cache_write),
            input_total: Some(u.input_total),
            output_total: Some(u.output),
            output_reasoning: None,
            total: Some(u.total),
            quality: Quality::Reported,
        },
        cost_estimated: None,
        // 旧版没有会话级累计快照，逐 step 绝对值本身即完整记录。
        completeness: Completeness::Complete,
        account_link: None,
        evidence_refs: vec![EvidenceRef {
            source_id: cfg.source_id.clone(),
            file_identity: cfg.file_identity.clone(),
            generation: cfg.generation,
            locator: offset.to_string(),
            parser_version: PARSER_VERSION.into(),
        }],
    });
}

/// 递归解包 SubagentEvent，收集其中的 StatusUpdate payload 及其最内层 agent_id。
/// 返回 (最内层 actor_id, StatusUpdate 的 payload)。
///
/// 记录形状（`wire/types.py` 的 `SubagentEvent` + `WireMessageEnvelope`）：
/// `message.payload = {parent_tool_call_id, agent_id, subagent_type, event: {type, payload}}`，
/// 即 agent_id 与 event 同级；嵌套时内层 `event` 自身又是一个 SubagentEvent 信封。
fn collect_mirrored<'a>(
    payload: &'a serde_json::Value,
    outer_actor: &str,
    depth: usize,
    out: &mut Vec<(Option<String>, &'a serde_json::Value)>,
    diagnostics: &mut Vec<String>,
) {
    if depth >= MAX_SUBAGENT_NESTING {
        diagnostics.push(format!("subagent_nesting_too_deep:{depth}"));
        return;
    }
    let Some(inner) = payload.get("event") else {
        diagnostics.push("subagent_event_without_event".into());
        return;
    };
    let Some(inner_type) = inner.get("type").and_then(|t| t.as_str()) else {
        diagnostics.push("subagent_event_without_type".into());
        return;
    };
    let inner_payload = inner.get("payload").unwrap_or(&serde_json::Value::Null);
    let actor = payload
        .get("agent_id")
        .and_then(|a| a.as_str())
        .unwrap_or(outer_actor)
        .to_string();
    match inner_type {
        "StatusUpdate" => out.push((Some(actor), inner_payload)),
        "SubagentEvent" => collect_mirrored(inner_payload, &actor, depth + 1, out, diagnostics),
        // 其他类型（ToolCall/ContentPart/...）与用量无关。
        _ => {}
    }
}

fn number_as_u64(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<u64>, String> {
    match obj.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(n)) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("{key} 不是非负整数: {n}")),
        Some(other) => Err(format!("{key} 类型非法: {other}")),
    }
}

/// `timestamp` 是浮点秒（旧版 `wire/file.py` 写 `time.time()`），转成 UTC RFC3339；
/// 非法/超范围返回 None，不伪造。
///
/// 精度取微秒：f64 秒在 ~1.7e9 附近的绝对分辨率约 2e-7 s，纳秒级小数位是浮点噪声
/// （如 1778563951.827712 会被算成 …827712059ns），按微秒四舍五入后再格式化。
fn ts_to_rfc3339(t: f64) -> Option<String> {
    if !t.is_finite() || t < 0.0 || t > i64::MAX as f64 {
        return None;
    }
    let secs = t.floor();
    let frac = t - secs;
    let micros = (frac * 1_000_000.0).round() as i64;
    let mut secs_i = secs as i64;
    let micros = if micros >= 1_000_000 {
        secs_i = secs_i.checked_add(1)?;
        0
    } else {
        micros
    };
    let nanos = u32::try_from(micros).ok()?.checked_mul(1_000)?;
    let dt = OffsetDateTime::from_unix_timestamp(secs_i).ok()?;
    dt.replace_nanosecond(nanos).ok()?.format(&Rfc3339).ok()
}

fn parse_metadata(line: &[u8]) -> Option<String> {
    let v = serde_json::from_slice::<serde_json::Value>(line).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("metadata") {
        return None;
    }
    v.get("protocol_version")
        .and_then(|p| p.as_str())
        .map(str::to_string)
}

/// 主版本 1 且次版本 ≤ 10 视为同族可解析（含历史遗留 1.1）；更高版本标记不兼容。
fn version_supported(v: &str) -> bool {
    let mut parts = v.split('.');
    match (parts.next(), parts.next()) {
        (Some(major), Some(minor)) => major == "1" && minor.parse::<u32>().is_ok_and(|m| m <= 10),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> WireParseConfig {
        WireParseConfig {
            source_id: "test-src".into(),
            file_identity: "wireA".into(),
            generation: 0,
            session_id: "11111111-aaaa-4bbb-8ccc-111111111111".into(),
            agent_id: "main".into(),
            mirrored_in_parent: false,
            count_mirrored_subagent_usage: false,
            fork: ForkContext::None,
            observed_at: "2026-10-11T08:00:00Z".into(),
        }
    }

    fn status(token_usage: &str, ts: f64) -> String {
        format!(
            "{{\"timestamp\": {ts}, \"message\": {{\"type\": \"StatusUpdate\", \"payload\": {{\"context_usage\": null, \"context_tokens\": null, \"max_context_tokens\": null, \"token_usage\": {token_usage}, \"message_id\": \"chatcmpl-x\", \"plan_mode\": false, \"mcp_status\": null}}}}}}\n"
        )
    }

    const HEADER: &str = "{\"type\": \"metadata\", \"protocol_version\": \"1.10\"}\n";

    #[test]
    fn version_gate() {
        assert!(version_supported("1.1"), "历史遗留版本同族");
        assert!(version_supported("1.10"));
        assert!(!version_supported("1.11"));
        assert!(!version_supported("2.0"));
        assert!(!version_supported("x"));
        assert!(!version_supported("1"));
    }

    #[test]
    fn one_step_one_absolute_event() {
        let data = format!(
            "{HEADER}{}{}",
            status(
                "{\"input_other\": 100, \"output\": 30, \"input_cache_read\": 40, \"input_cache_creation\": 5}",
                1778563951.827712
            ),
            status(
                "{\"input_other\": 200, \"output\": 50, \"input_cache_read\": 0, \"input_cache_creation\": 0}",
                1778563952.5
            ),
        );
        let out = parse_wire(data.as_bytes(), &cfg());
        assert_eq!(out.compatibility, Compatibility::Ok);
        assert_eq!(out.events.len(), 2, "每 step 一条绝对值事件，不聚合");
        let e = &out.events[0];
        assert_eq!(e.tokens.input_fresh, Some(100));
        assert_eq!(e.tokens.input_cache_read, Some(40));
        assert_eq!(e.tokens.input_cache_write, Some(5));
        assert_eq!(e.tokens.input_total, Some(145));
        assert_eq!(e.tokens.output_total, Some(30));
        assert_eq!(e.tokens.total, Some(175));
        assert_eq!(e.granularity, Granularity::Request);
        assert_eq!(e.request_count, Some(1));
        assert_eq!(e.completeness, Completeness::Complete);
        assert_eq!(e.model_raw, None, "旧版 wire 不记录模型名");
        assert_eq!(
            e.occurred_at.as_deref(),
            Some("2026-05-12T05:32:31.827712Z"),
            "浮点秒 → RFC3339 UTC"
        );
        // 偏移身份：第一条记录从 metadata 行之后开始
        assert_eq!(
            e.evidence_refs[0].locator,
            HEADER.len().to_string(),
            "locator = 行起始字节偏移"
        );
        assert!(e.event_id.ends_with(&format!(":o{}", HEADER.len())));
        assert_ne!(out.events[0].event_id, out.events[1].event_id);
    }

    #[test]
    fn null_token_usage_is_a_status_update_not_consumption() {
        let data = format!(
            "{HEADER}{}{}",
            status("null", 1778563951.0),
            status(
                "{\"input_other\": 0, \"output\": 40, \"input_cache_read\": 0, \"input_cache_creation\": 0}",
                1778563952.0
            ),
        );
        let out = parse_wire(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1, "token_usage=null 不产生事件");
        assert_eq!(out.events[0].tokens.input_total, Some(0), "明确的 0 保留");
        assert_eq!(out.events[0].tokens.output_total, Some(40));
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("status_without_token_usage:1")));
    }

    #[test]
    fn all_zero_token_usage_does_not_manufacture_zero_event() {
        let data = format!(
            "{HEADER}{}",
            status(
                "{\"input_other\": 0, \"output\": 0, \"input_cache_read\": 0, \"input_cache_creation\": 0}",
                1778563951.0
            )
        );
        let out = parse_wire(data.as_bytes(), &cfg());
        assert!(out.events.is_empty());
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("zero_token_usage_skipped:1")));
    }

    #[test]
    fn missing_required_field_is_invalid_not_zero() {
        let data = format!(
            "{HEADER}{}",
            status(
                "{\"output\": 12, \"input_cache_read\": 0, \"input_cache_creation\": 0}",
                1778563951.0
            )
        );
        let out = parse_wire(data.as_bytes(), &cfg());
        assert!(out.events.is_empty(), "缺 input_other 不补零");
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("token_usage_invalid_at_offset")));
    }

    #[test]
    fn mirrored_subagent_can_be_skipped_or_counted_with_actor() {
        let mirror = "{\"timestamp\": 1778563955.0, \"message\": {\"type\": \"SubagentEvent\", \"payload\": {\"parent_tool_call_id\": \"call-1\", \"agent_id\": \"a1b2c3d4\", \"subagent_type\": \"coder\", \"event\": {\"type\": \"StatusUpdate\", \"payload\": {\"token_usage\": {\"input_other\": 20, \"output\": 5, \"input_cache_read\": 0, \"input_cache_creation\": 0}, \"plan_mode\": null, \"mcp_status\": null}}}}}\n";
        let data = format!("{HEADER}{mirror}");

        let skipped = parse_wire(data.as_bytes(), &cfg());
        assert!(skipped.events.is_empty(), "默认以子 Agent 自有 wire 为准");
        assert!(skipped
            .diagnostics
            .iter()
            .any(|d| d.contains("mirrored_subagent_records_skipped:1")));

        let mut count_cfg = cfg();
        count_cfg.count_mirrored_subagent_usage = true;
        let counted = parse_wire(data.as_bytes(), &count_cfg);
        assert_eq!(counted.events.len(), 1);
        assert_eq!(counted.events[0].actor_id.as_deref(), Some("a1b2c3d4"));
        assert_eq!(counted.events[0].tokens.total, Some(25));
    }

    #[test]
    fn mirrored_in_parent_file_is_not_double_counted() {
        let data = format!(
            "{HEADER}{}",
            status(
                "{\"input_other\": 20, \"output\": 5, \"input_cache_read\": 0, \"input_cache_creation\": 0}",
                1778563955.0
            )
        );
        let mut c = cfg();
        c.agent_id = "a1b2c3d4".into();
        c.mirrored_in_parent = true;
        let out = parse_wire(data.as_bytes(), &c);
        assert!(out.events.is_empty());
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("subagent_mirrored_in_parent:1")));
    }

    #[test]
    fn nested_mirror_uses_innermost_actor() {
        let nested = "{\"timestamp\": 1778563956.0, \"message\": {\"type\": \"SubagentEvent\", \"payload\": {\"agent_id\": \"a1b2c3d4\", \"event\": {\"type\": \"SubagentEvent\", \"payload\": {\"agent_id\": \"a9f9f9f9\", \"event\": {\"type\": \"StatusUpdate\", \"payload\": {\"token_usage\": {\"input_other\": 50, \"output\": 5, \"input_cache_read\": 0, \"input_cache_creation\": 0}, \"plan_mode\": null, \"mcp_status\": null}}}}}}}\n";
        let data = format!("{HEADER}{nested}");
        let mut c = cfg();
        c.count_mirrored_subagent_usage = true;
        let out = parse_wire(data.as_bytes(), &c);
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].actor_id.as_deref(), Some("a9f9f9f9"));
    }

    #[test]
    fn fork_prefix_is_skipped_when_boundary_confirmed() {
        let inherited = status(
            "{\"input_other\": 900, \"output\": 90, \"input_cache_read\": 0, \"input_cache_creation\": 0}",
            1778563950.0,
        );
        let new = status(
            "{\"input_other\": 7, \"output\": 2, \"input_cache_read\": 0, \"input_cache_creation\": 0}",
            1778563960.0,
        );
        let data = format!("{HEADER}{inherited}{new}");
        let prefix_end = HEADER.len() as u64 + inherited.len() as u64;
        let mut c = cfg();
        c.fork = ForkContext::Inherited { prefix_end };
        let out = parse_wire(data.as_bytes(), &c);
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].tokens.input_fresh, Some(7));
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("fork_inherited_skipped:1")));

        // 边界不可确认时不静默丢弃，全部保留并记诊断。
        let mut c2 = cfg();
        c2.fork = ForkContext::Unresolved;
        let out2 = parse_wire(data.as_bytes(), &c2);
        assert_eq!(out2.events.len(), 2);
        assert!(out2
            .diagnostics
            .iter()
            .any(|d| d.contains("fork_boundary_unknown")));
    }

    #[test]
    fn unterminated_tail_not_consumed_and_malformed_line_diagnosed() {
        let mut data = String::from(HEADER);
        data.push_str(&status(
            "{\"input_other\": 5, \"output\": 2, \"input_cache_read\": 0, \"input_cache_creation\": 0}",
            1778563951.0,
        ));
        data.push_str("{not json}\n");
        data.push_str(&status(
            "{\"input_other\": 6, \"output\": 3, \"input_cache_read\": 0, \"input_cache_creation\": 0}",
            1778563952.0,
        ));
        let newline_end = data.len() as u64;
        data.push_str("{\"timestamp\": 1778563953.0, \"message\": {\"type\": \"StatusU");

        let out = parse_wire(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 2);
        assert_eq!(out.consumed_bytes, newline_end, "未写完的尾行不消费");
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("malformed_line_at_offset")));
    }

    #[test]
    fn higher_protocol_and_missing_header_are_incompatible() {
        let data = "{\"type\": \"metadata\", \"protocol_version\": \"1.11\"}\n{\"timestamp\": 1.0, \"message\": {\"type\": \"StatusUpdate\", \"payload\": {\"token_usage\": {\"input_other\": 5, \"output\": 2, \"input_cache_read\": 0, \"input_cache_creation\": 0}}}}\n";
        let out = parse_wire(data.as_bytes(), &cfg());
        assert_eq!(
            out.compatibility,
            Compatibility::UnsupportedProtocol("1.11".into())
        );
        assert!(out.events.is_empty());
        let first_nl = data.find('\n').unwrap() as u64 + 1;
        assert_eq!(out.consumed_bytes, first_nl);

        // 首行不是 metadata：无法确认版本，不沿用上游 1.1 降级默认。
        let headerless = "{\"timestamp\": 1.0, \"message\": {\"type\": \"StatusUpdate\", \"payload\": {\"token_usage\": null}}}\n";
        let out2 = parse_wire(headerless.as_bytes(), &cfg());
        assert_eq!(out2.compatibility, Compatibility::MissingMetadata);
        assert!(out2.events.is_empty());
    }

    #[test]
    fn migrated_context_only_wire_yields_no_events() {
        // 迁移工具写出的 wire 只有 token_counting.measured 之类；旧版 wire 的对应形态是
        // 只带 context_tokens 的 StatusUpdate。两者都不是消费。
        let data = format!(
            "{HEADER}{}",
            "{\"timestamp\": 1778563951.0, \"message\": {\"type\": \"StatusUpdate\", \"payload\": {\"context_usage\": 3.5, \"context_tokens\": 8192, \"max_context_tokens\": 262144, \"token_usage\": null, \"message_id\": null, \"plan_mode\": false, \"mcp_status\": null}}}\n"
        );
        let out = parse_wire(data.as_bytes(), &cfg());
        assert_eq!(out.compatibility, Compatibility::Ok);
        assert!(out.events.is_empty(), "上下文测量不是消费");
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("status_without_token_usage:1")));
    }
}
