//! A04 Grok Build / Grok CLI `updates.jsonl` 纯解析器。
//!
//! 证据：docs/research/evidence/a01-a04-ccswitch-importers.md「A04 Grok Build」节
//! （cc-switch `df40d53c`）。路径布局 `~/.grok/{sessions,archived_sessions}/<enc-cwd>/<session-id>/updates.jsonl`。
//!
//! 记录形态（JSON-RPC 风格，每行一条）：
//! - 只认 `method == "_x.ai/session/update"` 且 `params.update.sessionUpdate == "turn_completed"`
//!   且 `params.update.usage` 为对象、顶层 `timestamp` 为数字 epoch 秒的事件。
//!   其它类型即使带 usage 也跳过并记诊断（中途快照与轮末事件并存时双导会双算）。
//! - `usage.{inputTokens, outputTokens, cachedReadTokens, reasoningTokens, modelCalls,
//!   costUsdTicks, costIsPartial}`；`usage.modelUsage` 存在时优先取（model → 计数器），
//!   缺字段逐字段退回顶层 usage。modelUsage 单模型时取该模型名与计数器；多模型时退回
//!   顶层 usage（已是回合聚合值，模型名未知）并记诊断——本适配器按回合产出一条事件，
//!   身份不含模型。
//!
//! 计数语义（契约第 5 节）：
//! - granularity = turn_aggregate：`turn_completed` 的 usage 是**单个用户回合内多次模型
//!   推理的聚合**，不是会话累计，**不做差分**；两个连续回合数字完全相同也必须各计一次
//!   （禁止按数值判重）。
//! - inputTokens 记为 fresh 分量、cachedReadTokens 记为读缓存分量（两者分列），
//!   input_cache_write 源不提供保持 null；input_total = fresh + read（分量齐时）。
//!   注意：上游同时声明「inputTokens 含 cache read」（TOTAL 语义）与分列处置，此处按
//!   分列口径（fresh/read 不相交）映射，语义待官方版本核实；若上游实为 TOTAL，则应改为
//!   fresh = inputTokens − cachedReadTokens、input_total = inputTokens。
//! - reasoningTokens ⊂ outputTokens：output_reasoning = reasoningTokens，
//!   output_total = outputTokens（**不另加** reasoning）。
//! - request_count = modelCalls（源给出的本轮模型调用次数；缺失为 null，不强赋 1）。
//! - costUsdTicks 精确换算 1 tick = 10^-10 USD → 十进制字符串（整数运算，无浮点）；
//!   costIsPartial=true → 费用为部分计价下界（CostInfo.completeness=partial；事件 Token 仍为 complete）；无 costUsdTicks → cost 为
//!   None（无价不等于免费，禁止写 0）。
//!
//! 身份（契约第 4 节）：`grok_build:{file_identity}:g{generation}:{会话目录名}:{turn_key}`，
//! turn_key 优先取事件 `prompt_id`（上游 rewind 截断改写前缀导致行序前移时不双算），
//! 缺失退回文件内字节偏移 `o{offset}`。会话目录名是唯一性锚（UUIDv7），归档/活跃副本收敛。
//!
//! 纯函数：不读文件系统、不访问网络与凭据。

use crate::domain::*;
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub const PARSER_VERSION: &str = "grok-build-updates-1";

/// 1 tick = 10^-10 USD。
const TICKS_PER_USD: u128 = 10_000_000_000;

pub struct GrokParseConfig {
    pub source_id: String,
    pub file_identity: String,
    pub generation: u64,
    /// `updates.jsonl` 所在会话目录名（与 `summary.json` 的 `info.id` 一致）。
    pub session_dir_name: String,
    /// 来自 `archived_sessions`；身份不变，仅标记来源（归档/活跃副本收敛）。
    pub archived: bool,
    /// 本机观察时间（RFC3339）。
    pub observed_at: String,
}

#[derive(Debug, Default)]
pub struct GrokParseOutcome {
    pub events: Vec<UsageEvent>,
    /// 已消费字节数（最后一个完整换行之后）；尾部未换行的行不计入。
    pub consumed_bytes: u64,
    pub diagnostics: Vec<String>,
}

pub fn parse_updates(bytes: &[u8], cfg: &GrokParseConfig) -> GrokParseOutcome {
    let mut out = GrokParseOutcome::default();

    // 只消费以 \n 结尾的行；尾行未换行视为未写完，不消费。
    let mut lines: Vec<(u64, &[u8])> = Vec::new();
    let mut start = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            lines.push((start as u64, &bytes[start..i]));
            start = i + 1;
        }
    }
    out.consumed_bytes = start as u64;

    if cfg.archived {
        out.diagnostics.push(
            "archived_sessions:归档目录事件按会话目录名建立身份，与活跃副本收敛不重复计".into(),
        );
    }

    for (offset, line) in &lines {
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_slice(line) {
            Ok(v) => v,
            Err(_) => {
                out.diagnostics.push(format!("malformed_line_at_offset:{offset}"));
                continue;
            }
        };

        let update = v.pointer("/params/update");
        let usage = update
            .and_then(|u| u.get("usage"))
            .filter(|u| u.is_object());
        let is_target_method =
            v.get("method").and_then(Value::as_str) == Some("_x.ai/session/update");
        let session_update = update.and_then(|u| u.get("sessionUpdate")).and_then(Value::as_str);

        if !(is_target_method && session_update == Some("turn_completed")) {
            if usage.is_some() {
                out.diagnostics.push(format!(
                    "usage_bearing_non_turn_event_skipped:{offset}:{}:{}",
                    v.get("method").and_then(Value::as_str).unwrap_or("none"),
                    session_update.unwrap_or("none")
                ));
            }
            continue;
        }
        let Some(usage) = usage else {
            out.diagnostics.push(format!("turn_completed_without_usage:{offset}"));
            continue;
        };
        let Some(ts) = v.get("timestamp").and_then(Value::as_i64) else {
            out.diagnostics.push(format!("turn_completed_without_numeric_timestamp:{offset}"));
            continue;
        };

        // epoch 秒；> 1e11 视为毫秒（防御性，上游实测为秒）。
        let (nanos, as_millis) = if ts.unsigned_abs() > 100_000_000_000 {
            (i128::from(ts) * 1_000_000, true)
        } else {
            (i128::from(ts) * 1_000_000_000, false)
        };
        let Some(occurred_at) = OffsetDateTime::from_unix_timestamp_nanos(nanos)
            .ok()
            .and_then(|dt| dt.format(&Rfc3339).ok())
        else {
            out.diagnostics.push(format!("timestamp_out_of_range:{offset}"));
            continue;
        };
        if as_millis {
            out.diagnostics.push(format!("timestamp_treated_as_millis:{offset}"));
        }

        // 身份：prompt_id 优先，缺失退回字节偏移。
        let prompt_id = find_prompt_id(&v);
        let key = match &prompt_id {
            Some(p) => p.clone(),
            None => format!("o{offset}"),
        };
        let event_id = format!(
            "grok_build:{}:g{}:{}:{}",
            cfg.file_identity, cfg.generation, cfg.session_dir_name, key
        );

        // modelUsage 优先（单模型取该模型计数器）；多模型时退回顶层 usage（回合聚合值）。
        let model_usage = usage.get("modelUsage").and_then(Value::as_object);
        let (src, model_raw) = match model_usage {
            Some(m) if m.len() == 1 => {
                let (name, entry) = m.iter().next().expect("len==1");
                let model = if name.is_empty() {
                    None
                } else {
                    Some(name.clone())
                };
                (entry, model)
            }
            Some(m) if m.len() > 1 => {
                out.diagnostics.push(format!(
                    "model_usage_multi_model:{offset}:{}:顶层 usage 作为回合聚合，模型名未知",
                    m.len()
                ));
                (usage, None)
            }
            _ => (usage, None),
        };
        let read = |k: &str| {
            src.get(k)
                .and_then(Value::as_u64)
                .or_else(|| usage.get(k).and_then(Value::as_u64))
        };

        let input_fresh = read("inputTokens");
        let input_cache_read = read("cachedReadTokens");
        let output_total = read("outputTokens");
        let output_reasoning = read("reasoningTokens");
        let request_count = read("modelCalls");

        // 输入互斥分量：fresh(inputTokens) + read(cachedReadTokens)，源不提供 cache write。
        let input_total = match (input_fresh, input_cache_read) {
            (Some(f), Some(r)) => f.checked_add(r),
            _ => None,
        };
        let total = match (input_total, output_total) {
            (Some(i), Some(o)) => i.checked_add(o),
            _ => None,
        };

        let cost_partial = src
            .get("costIsPartial")
            .and_then(Value::as_bool)
            .or_else(|| usage.get("costIsPartial").and_then(Value::as_bool))
            .unwrap_or(false);
        let cost_reported = read("costUsdTicks").and_then(|ticks| {
            Money::new(ticks_to_usd(ticks), "USD")
                .ok()
                .map(|money| CostInfo {
                    money,
                    quality: Quality::Reported,
                    completeness: if cost_partial {
                        Completeness::Partial
                    } else {
                        Completeness::Complete
                    },
                    price_version: None,
                    scope: None,
                })
        });
        // costIsPartial 只影响费用覆盖范围（CostInfo.completeness=partial）；
        // 事件级 completeness 描述 Token 计量完整性，此处 Token 齐全 → complete。
        let completeness = Completeness::Complete;

        out.events.push(UsageEvent {
            schema_version: USAGE_EVENT_SCHEMA_VERSION,
            event_id,
            agent: AgentKind::GrokBuild,
            source_id: cfg.source_id.clone(),
            stream_domain: StreamDomain::LocalLog,
            session_id: Some(cfg.session_dir_name.clone()),
            parent_session_id: None,
            actor_id: None,
            turn_id: prompt_id,
            project_ref: None,
            occurred_at: Some(occurred_at),
            observed_at: cfg.observed_at.clone(),
            model_raw,
            model_normalized: None,
            granularity: Granularity::TurnAggregate,
            request_count,
            tokens: TokenSet {
                input_fresh,
                input_cache_read,
                input_cache_write: None,
                input_total,
                output_total,
                output_reasoning,
                total,
                quality: Quality::Reported,
            },
            cost_reported,
            cost_estimated: None,
            completeness,
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

    out
}

/// prompt_id 的可能位置（上游字段名未完全固定，按证据取每轮唯一 UUID）。
fn find_prompt_id(v: &Value) -> Option<String> {
    const PATHS: [&str; 6] = [
        "/params/update/prompt_id",
        "/params/update/promptId",
        "/params/prompt_id",
        "/params/promptId",
        "/prompt_id",
        "/promptId",
    ];
    PATHS.iter().find_map(|p| {
        v.pointer(p)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    })
}

/// 整数 tick → USD 十进制字符串，精确无浮点。1 tick = 10^-10 USD。
pub fn ticks_to_usd(ticks: u64) -> String {
    let v = u128::from(ticks);
    let int = v / TICKS_PER_USD;
    let frac = v % TICKS_PER_USD;
    if frac == 0 {
        return int.to_string();
    }
    let frac = format!("{frac:010}");
    format!("{int}.{}", frac.trim_end_matches('0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> GrokParseConfig {
        GrokParseConfig {
            source_id: "test-src".into(),
            file_identity: "updatesA".into(),
            generation: 0,
            session_dir_name: "0199aaaa-0000-7000-8000-000000000001".into(),
            archived: false,
            observed_at: "2026-10-11T08:00:00Z".into(),
        }
    }

    #[test]
    fn ticks_convert_without_float() {
        assert_eq!(ticks_to_usd(338_880_000), "0.033888");
        assert_eq!(ticks_to_usd(44_288_000), "0.0044288");
        assert_eq!(ticks_to_usd(10_000_000_000), "1");
        assert_eq!(ticks_to_usd(5_000_000_000), "0.5");
        assert_eq!(ticks_to_usd(25_000_000), "0.0025");
        assert_eq!(ticks_to_usd(1), "0.0000000001");
        assert_eq!(ticks_to_usd(0), "0");
        assert_eq!(ticks_to_usd(12_345_678_901), "1.2345678901");
    }

    fn line(prompt: &str, usage: &str, ts: i64) -> String {
        format!(
            "{{\"method\":\"_x.ai/session/update\",\"params\":{{\"update\":{{\"sessionUpdate\":\"turn_completed\",\"prompt_id\":\"{prompt}\",\"usage\":{usage}}}}},\"timestamp\":{ts}}}\n"
        )
    }

    #[test]
    fn identical_consecutive_turns_are_both_counted() {
        let data = format!(
            "{}{}",
            line(
                "p-1",
                r#"{"inputTokens":1000,"outputTokens":200,"cachedReadTokens":400}"#,
                1_791_626_400
            ),
            line(
                "p-2",
                r#"{"inputTokens":1000,"outputTokens":200,"cachedReadTokens":400}"#,
                1_791_626_460
            ),
        );
        let out = parse_updates(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 2, "相同数字也必须各计一次");
        assert_eq!(out.events[0].tokens.input_total, Some(1400));
        assert_eq!(out.events[1].tokens.input_total, Some(1400));
        assert_ne!(out.events[0].event_id, out.events[1].event_id);
    }

    #[test]
    fn unterminated_tail_not_consumed() {
        let data = format!(
            "{}{}",
            line("p-1", r#"{"inputTokens":5,"outputTokens":2}"#, 1_791_626_400),
            "{\"method\":\"_x.ai/session/update\",\"params\":{\"update\":{\"sessionUpdate\":\"turn_completed\""
        );
        let out = parse_updates(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1);
        // 只在第一个换行之后消费；尾行起点不被计入
        let first_nl = data.find('\n').unwrap() + 1;
        assert_eq!(out.consumed_bytes, first_nl as u64);
    }

    #[test]
    fn non_turn_event_with_usage_is_skipped_with_diagnostic() {
        let data = concat!(
            "{\"method\":\"_x.ai/session/update\",\"params\":{\"update\":{\"sessionUpdate\":\"turn_in_progress\",\"usage\":{\"inputTokens\":9}}},\"timestamp\":1791626400}\n",
            "{\"method\":\"_x.ai/session/update\",\"params\":{\"update\":{\"sessionUpdate\":\"turn_completed\",\"prompt_id\":\"p\",\"usage\":{\"inputTokens\":1000,\"cachedReadTokens\":0,\"outputTokens\":10}}},\"timestamp\":1791626460}\n",
        );
        let out = parse_updates(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1);
        assert!(out
            .diagnostics
            .iter()
            .any(|d| d.contains("usage_bearing_non_turn_event_skipped")));
    }
}
