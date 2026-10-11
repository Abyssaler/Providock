//! A02 Codex rollout JSONL 纯解析器。
//!
//! 证据：docs/research/evidence/a01-a04-ccswitch-importers.md（cc-switch `df40d53c`，A02 节）。
//! 格式（`~/.codex/sessions/YYYY/MM/DD/*.jsonl` 与 `archived_sessions/`，`.zst` 由调用方解压）：
//! - `session_meta`（首条）：payload.id/thread_id/threadId、forked_from_id、
//!   source.subagent.thread_spawn.parent_thread_id；行级 timestamp。
//! - `turn_context`：payload.model 跟踪模型变化。
//! - `event_msg` + `payload.type=="token_count"`：info.total_token_usage（会话累计）/
//!   last_token_usage（单次精确值）。
//!
//! 字段语义（官方口径，经上游源码核实）：
//! - input_tokens 为 TOTAL：含 cached read 与 cache write（两者是 input 的互斥子集）。
//!   映射：cache_read=cached_input_tokens（回退 cache_read_input_tokens），
//!   cache_write=cache_write_input_tokens，fresh=input−read−write（钳制不取负）。
//! - output_tokens 为输出；reasoning_output_tokens 是输出的附属细分（不另加）。
//!
//! 计数规则（契约第 5 节）：
//! - 有 last_token_usage 优先直接采用（granularity=request，request_count=1）。
//! - 否则对 total_token_usage 做高水位差分（granularity=session_delta，request_count=None）。
//!   首个累计量的基线为零（完整会话起点）；重复快照产生零增量不发事件；
//!   字段倒退视为计数器重置：记诊断缺口、基线重定为当前快照，本次不发事件。
//! - fork：子 rollout 复制父的 token 事件前缀；用父时间线签名做前缀对齐跳过继承段
//!   （明确共享祖先的归并，契约第 4 节）。父时间线不可用 → Deferred，不推进游标。
//! - 相同数字的独立请求是文件内不同偏移的不同事件，不按数字判重。
//! - 尾行未换行不消费；重扫幂等由事件身份（文件身份+代数+字节偏移）保证。
//!
//! 纯函数：不读文件系统、不访问网络与凭据。

use crate::domain::*;

pub const PARSER_VERSION: &str = "codex-rollout-1";

/// token 事件签名：用于父子 rollout 前缀对齐（不是去重依据）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventSignature {
    pub input: u64,
    pub cached: u64,
    pub cache_write: u64,
    pub output: u64,
    pub reasoning: u64,
    pub total: Option<u64>,
}

pub struct CodexParseConfig {
    pub source_id: String,
    pub file_identity: String,
    pub generation: u64,
    pub observed_at: String,
    /// 文件名末尾 UUID（逻辑线程 id）；缺失时无法建立稳定身份 → Deferred
    pub filename_thread_id: Option<String>,
    /// 父线程 token 事件签名时间线（fork 对齐用）；父文件暂不可读为 None
    pub parent_signatures: Option<Vec<EventSignature>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexStatus {
    Ok,
    /// 父时间线暂不可用等原因；调用方下一轮重试，不推进游标
    Deferred(String),
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SessionMeta {
    pub thread_id: Option<String>,
    pub parent_thread_id: Option<String>,
    pub first_timestamp: Option<String>,
}

#[derive(Default)]
pub struct CodexParseOutcome {
    pub events: Vec<UsageEvent>,
    pub consumed_bytes: u64,
    pub diagnostics: Vec<String>,
    pub session: Option<SessionMeta>,
    /// 本文件 token 事件签名时间线（供子 rollout 对齐）
    pub signatures: Vec<EventSignature>,
    pub status: Option<CodexStatus>,
}

pub fn parse_rollout(bytes: &[u8], cfg: &CodexParseConfig) -> CodexParseOutcome {
    let mut out = CodexParseOutcome::default();

    // 只消费以 \n 结尾的行
    let mut lines: Vec<(u64, &[u8])> = Vec::new();
    let mut start = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            lines.push((start as u64, &bytes[start..i]));
            start = i + 1;
        }
    }
    out.consumed_bytes = start as u64;

    // 第一遍：session_meta / 模型跟踪所需信息
    let mut session = SessionMeta::default();
    for (_, line) in &lines {
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("session_meta") {
            continue;
        }
        let payload = v.get("payload").cloned().unwrap_or(serde_json::Value::Null);
        session.thread_id = ["id", "thread_id", "threadId"]
            .iter()
            .find_map(|k| payload.get(*k).and_then(|x| x.as_str()))
            .map(str::to_string);
        let forked = payload.get("forked_from_id").and_then(|x| x.as_str());
        let subagent_parent = payload
            .get("source")
            .and_then(|s| s.get("subagent"))
            .and_then(|s| s.get("thread_spawn"))
            .and_then(|s| s.get("parent_thread_id"))
            .and_then(|x| x.as_str());
        session.parent_thread_id = match (forked, subagent_parent) {
            (Some(a), Some(b)) if a != b => {
                out.diagnostics
                    .push("parent_conflict: forked_from_id 与 subagent 父不一致".into());
                None
            }
            (a, b) => a.or(b).map(str::to_string),
        };
        if session.parent_thread_id.as_deref() == session.thread_id.as_deref() {
            out.diagnostics.push("parent_is_self: 忽略父引用".into());
            session.parent_thread_id = None;
        }
        session.first_timestamp = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .map(str::to_string);
        break; // 仅首条 session_meta 生效
    }

    let logical_thread = match (&cfg.filename_thread_id, &session.thread_id) {
        (Some(f), _) => Some(f.clone()),
        (None, Some(s)) => {
            out.diagnostics
                .push("filename_without_uuid: 退化用 session_meta 线程 id 建立身份".into());
            Some(s.clone())
        }
        (None, None) => None,
    };
    let Some(logical_thread) = logical_thread else {
        out.status = Some(CodexStatus::Deferred(
            "no_thread_identity: 文件名无 UUID 且缺 session_meta".into(),
        ));
        return out;
    };
    out.session = Some(session.clone());

    // fork：父已声明但时间线不可用 → deferred（下一轮重试）
    if session.parent_thread_id.is_some() && cfg.parent_signatures.is_none() {
        out.status = Some(CodexStatus::Deferred("parent_timeline_unavailable".into()));
        return out;
    }

    // 第二遍：token 事件
    struct RawEvent {
        offset: u64,
        timestamp: Option<String>,
        signature: EventSignature,
        effective: EventSignature, // 差分后实际计入的用量
        from_last_usage: bool,
    }
    let mut raw_events: Vec<RawEvent> = Vec::new();
    let mut model: Option<String> = None;
    let mut model_at: Vec<Option<String>> = Vec::new();

    #[derive(Default, Clone, Copy)]
    struct Counters {
        input: u64,
        cached: u64,
        cache_write: u64,
        output: u64,
    }
    let mut high_water: Option<Counters> = None;
    let mut reset_reported = false;

    for (offset, line) in &lines {
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if ty == "turn_context" {
            let m = v
                .get("payload")
                .and_then(|p| p.get("model"))
                .or_else(|| {
                    v.get("payload")
                        .and_then(|p| p.get("info"))
                        .and_then(|i| i.get("model"))
                })
                .and_then(|x| x.as_str());
            if let Some(m) = m {
                model = Some(m.to_string());
            }
            continue;
        }
        if ty != "event_msg" {
            continue;
        }
        let payload = match v.get("payload") {
            Some(p) => p,
            None => continue,
        };
        if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
            continue;
        }
        let info = payload
            .get("info")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let timestamp = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .map(str::to_string);

        let read_counters = |usage: &serde_json::Value| -> Option<(Counters, u64, Option<u64>)> {
            let input = usage.get("input_tokens")?.as_u64()?;
            let cached = usage
                .get("cached_input_tokens")
                .or_else(|| usage.get("cache_read_input_tokens"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let cache_write = usage
                .get("cache_write_input_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let output = usage.get("output_tokens")?.as_u64()?;
            let reasoning = usage
                .get("reasoning_output_tokens")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let total = usage.get("total_tokens").and_then(|x| x.as_u64());
            Some((
                Counters {
                    input,
                    cached,
                    cache_write,
                    output,
                },
                reasoning,
                total,
            ))
        };

        let last = info.get("last_token_usage").and_then(read_counters);
        let total = info.get("total_token_usage").and_then(read_counters);

        // 签名优先取累计快照字段（与父时间线可比），否则取单次值
        let signature = match (total, last) {
            (Some((t, r, tt)), _) => EventSignature {
                input: t.input,
                cached: t.cached,
                cache_write: t.cache_write,
                output: t.output,
                reasoning: r,
                total: tt,
            },
            (None, Some((l, r, tt))) => EventSignature {
                input: l.input,
                cached: l.cached,
                cache_write: l.cache_write,
                output: l.output,
                reasoning: r,
                total: tt,
            },
            (None, None) => continue,
        };

        let (effective, from_last) = if let Some((l, r, tt)) = last {
            (
                EventSignature {
                    input: l.input,
                    cached: l.cached,
                    cache_write: l.cache_write,
                    output: l.output,
                    reasoning: r,
                    total: tt,
                },
                true,
            )
        } else if let Some((t, r, tt)) = total {
            let hw = high_water.unwrap_or_default();
            let mut reset = false;
            let diff = |cur: u64, hi: u64, reset: &mut bool| {
                if cur < hi {
                    *reset = true;
                    0
                } else {
                    cur - hi
                }
            };
            let d = Counters {
                input: diff(t.input, hw.input, &mut reset),
                cached: diff(t.cached, hw.cached, &mut reset),
                cache_write: diff(t.cache_write, hw.cache_write, &mut reset),
                output: diff(t.output, hw.output, &mut reset),
            };
            if reset {
                if !reset_reported {
                    out.diagnostics
                        .push("counter_reset: 累计量倒退，重置前区间用量未知（缺口不补零）".into());
                    reset_reported = true;
                }
                high_water = Some(t); // 基线重定为当前快照
            } else {
                high_water = Some(Counters {
                    input: hw.input.max(t.input),
                    cached: hw.cached.max(t.cached),
                    cache_write: hw.cache_write.max(t.cache_write),
                    output: hw.output.max(t.output),
                });
            }
            if reset || (d.input == 0 && d.cached == 0 && d.cache_write == 0 && d.output == 0) {
                // 重复快照或重置事件：零增量，不产生计量事件，但签名入时间线供对齐
                out.signatures.push(signature);
                continue;
            }
            (
                EventSignature {
                    input: d.input,
                    cached: d.cached,
                    cache_write: d.cache_write,
                    output: d.output,
                    reasoning: r,
                    total: tt,
                },
                false,
            )
        } else {
            continue;
        };

        out.signatures.push(signature);
        raw_events.push(RawEvent {
            offset: *offset,
            timestamp,
            signature,
            effective,
            from_last_usage: from_last,
        });
        model_at.push(model.clone());
    }

    // fork 前缀对齐：跳过与父时间线前缀一致的继承段
    let skip = match (&session.parent_thread_id, &cfg.parent_signatures) {
        (Some(_), Some(parent)) => {
            let child_sigs: Vec<&EventSignature> =
                raw_events.iter().map(|e| &e.signature).collect();
            let mut n = 0;
            while n < child_sigs.len() && n < parent.len() && *child_sigs[n] == parent[n] {
                n += 1;
            }
            if n > 0 {
                out.diagnostics
                    .push(format!("fork_replay_skipped:{n}（父 rollout 已计）"));
            }
            n
        }
        _ => 0,
    };

    for (idx, e) in raw_events.into_iter().enumerate().skip(skip) {
        let model_raw = model_at[idx].clone();
        let eff = &e.effective;
        // input 为 TOTAL：拆分互斥分量，钳制不取负
        let cache_read = eff.cached.min(eff.input);
        let cache_write = eff.cache_write.min(eff.input - cache_read);
        let fresh = eff.input - cache_read - cache_write;
        let input_total = eff.input;
        let output_total = eff.output;
        let total = input_total.saturating_add(output_total);

        out.events.push(UsageEvent {
            schema_version: USAGE_EVENT_SCHEMA_VERSION,
            event_id: format!(
                "codex:{}:{}:g{}:o{}",
                logical_thread, cfg.file_identity, cfg.generation, e.offset
            ),
            agent: AgentKind::Codex,
            source_id: cfg.source_id.clone(),
            stream_domain: StreamDomain::LocalLog,
            session_id: session.thread_id.clone().or(Some(logical_thread.clone())),
            parent_session_id: session.parent_thread_id.clone(),
            actor_id: None,
            turn_id: None,
            project_ref: None,
            occurred_at: e.timestamp.clone(),
            observed_at: cfg.observed_at.clone(),
            model_raw,
            model_normalized: None,
            granularity: if e.from_last_usage {
                Granularity::Request
            } else {
                Granularity::SessionDelta
            },
            request_count: if e.from_last_usage { Some(1) } else { None },
            tokens: TokenSet {
                input_fresh: Some(fresh),
                input_cache_read: Some(cache_read),
                input_cache_write: Some(cache_write),
                input_total: Some(input_total),
                output_total: Some(output_total),
                output_reasoning: Some(eff.reasoning),
                total: Some(total),
                quality: if e.from_last_usage {
                    Quality::Reported
                } else {
                    Quality::Derived
                },
            },
            cost_reported: None,
            cost_estimated: None,
            completeness: Completeness::Complete,
            account_link: None,
            evidence_refs: vec![EvidenceRef {
                source_id: cfg.source_id.clone(),
                file_identity: cfg.file_identity.clone(),
                generation: cfg.generation,
                locator: e.offset.to_string(),
                parser_version: PARSER_VERSION.into(),
            }],
        });
    }

    out.status = Some(CodexStatus::Ok);
    out
}
