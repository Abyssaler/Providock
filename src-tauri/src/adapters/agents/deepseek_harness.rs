//! A10. Official v3/v4 durable settlement records. See evidence/a10-deepseek-harness.md.
use crate::domain::*;
use serde_json::Value;
use std::collections::HashMap;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
pub const PARSER_VERSION: &str = "deepseek-harness-1";
#[derive(Default)]
pub struct Outcome {
    pub events: Vec<UsageEvent>,
    pub session_id: Option<String>,
    pub parent_id: Option<String>,
    pub seeded: bool,
    pub subagent: bool,
    pub consumed: u64,
    pub compatible: bool,
    pub diagnostics: Vec<String>,
}
pub struct Config<'a> {
    pub source_id: &'a str,
    pub file_identity: &'a str,
    pub observed_at: &'a str,
    pub parent_available: bool,
}
fn timestamp(value: Option<&Value>) -> Option<String> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(value?.as_i64()?) * 1_000_000)
        .ok()?
        .format(&Rfc3339)
        .ok()
}
fn usage_tokens(v: &Value) -> Option<TokenSet> {
    let count = |key| {
        v.get(key)
            .and_then(Value::as_u64)
            .filter(|n| *n <= 9_007_199_254_740_991)
    };
    let fresh = count("inputTokens")?;
    let output = count("outputTokens")?;
    let read = count("cacheReadTokens");
    let write = count("cacheWriteTokens");
    let known = fresh
        .checked_add(read.unwrap_or(0))?
        .checked_add(write.unwrap_or(0))?;
    let total = count("totalTokens");
    let input_total = match total {
        Some(total) => {
            let prompt = total.checked_sub(output)?;
            if prompt < known || (read.is_some() && write.is_some() && prompt != known) {
                return None;
            }
            Some(prompt)
        }
        None => match (read, write) {
            (Some(_), Some(_)) => Some(known),
            _ => None,
        },
    };
    Some(TokenSet {
        input_fresh: Some(fresh),
        input_cache_read: read,
        input_cache_write: write,
        input_total,
        output_total: Some(output),
        output_reasoning: count("reasoningTokens").filter(|r| *r <= output),
        total: total.or_else(|| input_total?.checked_add(output)),
        quality: Quality::Reported,
    })
}
pub fn parse(bytes: &[u8], cfg: &Config<'_>) -> Outcome {
    let mut out = Outcome::default();
    let consumed = bytes
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|p| p + 1)
        .unwrap_or(0);
    let rows: Result<Vec<Value>, _> = bytes[..consumed]
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .map(serde_json::from_slice)
        .collect();
    let Ok(rows) = rows else {
        out.diagnostics.push("deepseek_harness:invalid_json".into());
        return out;
    };
    let Some(header) = rows.first() else {
        return out;
    };
    if header.get("type").and_then(Value::as_str) != Some("session")
        || !matches!(header.get("version").and_then(Value::as_u64), Some(3 | 4))
    {
        out.diagnostics
            .push("deepseek_harness:unsupported_format".into());
        return out;
    }
    let Some(sid) = header
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 256)
    else {
        return out;
    };
    out.session_id = Some(sid.into());
    out.parent_id = header
        .get("parentSession")
        .and_then(Value::as_str)
        .map(str::to_string);
    out.seeded = header.get("isSeeded").and_then(Value::as_bool) == Some(true);
    out.subagent = header.get("origin").and_then(Value::as_str) == Some("subagent");
    let events = &rows[1..];
    if events
        .iter()
        .enumerate()
        .any(|(i, e)| e.get("seq").and_then(Value::as_u64) != Some(i as u64))
    {
        out.diagnostics
            .push("deepseek_harness:noncontiguous_seq".into());
        return out;
    }
    let cut = events.iter().rposition(|e| {
        e.get("type").and_then(Value::as_str) == Some("session/end-seed")
            && e.pointer("/data/inherited").and_then(Value::as_bool) == Some(true)
    });
    if out.seeded && (cut.is_none() || !cfg.parent_available) {
        out.diagnostics
            .push("deepseek_harness:fork_parent_or_cut_unavailable".into());
        return out;
    }
    if !out.seeded && cut.is_some() {
        out.diagnostics
            .push("deepseek_harness:unexpected_seed_cut".into());
        return out;
    }
    let mut attempts: HashMap<(u64, u64), u64> = HashMap::new();
    let mut route: Option<String> = None;
    for (index, e) in events.iter().enumerate() {
        let ty = e.get("type").and_then(Value::as_str).unwrap_or("");
        let data = &e["data"];
        if ty == "request/context" {
            route = data
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if ![
            "assistant/message",
            "assistant/attempt",
            "compaction/summary",
        ]
        .contains(&ty)
        {
            continue;
        }
        let identity = if ty == "compaction/summary" {
            let Some(id) = data.get("compactionId").and_then(Value::as_str) else {
                continue;
            };
            format!("compact:{id}")
        } else {
            let (Some(turn), Some(step)) = (
                data.get("turn").and_then(Value::as_u64),
                data.get("step").and_then(Value::as_u64),
            ) else {
                out.diagnostics
                    .push("deepseek_harness:missing_attempt_identity".into());
                continue;
            };
            let ordinal = attempts.entry((turn, step)).or_default();
            let id = format!("t{turn}:s{step}:a{ordinal}");
            *ordinal += 1;
            id
        };
        if out.seeded && cut.is_some_and(|cut| index < cut) {
            continue;
        }
        let stream_usage = data
            .get("stream")
            .and_then(Value::as_array)
            .and_then(|records| {
                records.iter().rev().find_map(|record| {
                    let chunk = record.get("chunk")?;
                    (chunk.get("type").and_then(Value::as_str) == Some("usage"))
                        .then(|| chunk.get("usage"))
                        .flatten()
                })
            });
        let usage = data.get("usage").or(stream_usage);
        let tokens = usage.and_then(usage_tokens).unwrap_or_default();
        let model = data
            .pointer("/message/source/model")
            .or_else(|| data.get("model"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| route.clone());
        let complete = tokens.input_fresh.is_some()
            && tokens.input_cache_read.is_some()
            && tokens.input_cache_write.is_some()
            && tokens.output_total.is_some();
        out.events.push(UsageEvent {
            schema_version: USAGE_EVENT_SCHEMA_VERSION,
            event_id: format!("deepseek_harness:{sid}:{identity}"),
            agent: AgentKind::DeepseekHarness,
            source_id: cfg.source_id.into(),
            stream_domain: StreamDomain::LocalLog,
            session_id: Some(sid.into()),
            parent_session_id: out.parent_id.clone(),
            actor_id: None,
            turn_id: data
                .get("turn")
                .and_then(Value::as_u64)
                .map(|v| v.to_string()),
            project_ref: None,
            occurred_at: timestamp(e.get("time")),
            observed_at: cfg.observed_at.into(),
            model_raw: model,
            model_normalized: None,
            granularity: Granularity::Request,
            request_count: Some(1),
            tokens,
            cost_estimated: None,
            completeness: if complete {
                Completeness::Complete
            } else {
                Completeness::Partial
            },
            account_link: None,
            evidence_refs: vec![EvidenceRef {
                source_id: cfg.source_id.into(),
                file_identity: cfg.file_identity.into(),
                generation: 0,
                locator: index.to_string(),
                parser_version: PARSER_VERSION.into(),
            }],
        });
    }
    out.compatible = true;
    out.consumed = consumed as u64;
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config<'static> {
        Config {
            source_id: "synthetic",
            file_identity: "synthetic.jsonl",
            observed_at: "2026-10-13T00:00:00Z",
            parent_available: true,
        }
    }
    #[test]
    fn independent_expected_totals_and_retry_accounting() {
        let data =
            include_bytes!("../../../../fixtures/agents/deepseek_harness/session-v4/input.jsonl");
        let out = parse(data, &config());
        assert!(out.compatible);
        assert_eq!(out.events.len(), 4);
        assert_eq!(
            out.events
                .iter()
                .filter_map(|e| e.tokens.total)
                .sum::<u64>(),
            612
        );
        assert_eq!(out.events[0].tokens.input_total, Some(500));
        assert_eq!(out.events[0].tokens.input_cache_write, None);
        assert_ne!(out.events[1].event_id, out.events[2].event_id);
    }
    #[test]
    fn unknown_version_and_broken_sequences_do_not_advance() {
        let original = String::from_utf8(
            include_bytes!("../../../../fixtures/agents/deepseek_harness/session-v4/input.jsonl")
                .to_vec(),
        )
        .unwrap();
        for data in [
            original.replace("\"version\":4", "\"version\":5"),
            original.replace("\"seq\":2", "\"seq\":9"),
        ] {
            let out = parse(data.as_bytes(), &config());
            assert!(!out.compatible);
            assert!(out.events.is_empty());
            assert_eq!(out.consumed, 0);
        }
    }
    #[test]
    fn source_copy_and_v3_v4_keep_identities() {
        let original = String::from_utf8(
            include_bytes!("../../../../fixtures/agents/deepseek_harness/session-v4/input.jsonl")
                .to_vec(),
        )
        .unwrap();
        let older = parse(
            original
                .replace("\"version\":4", "\"version\":3")
                .as_bytes(),
            &config(),
        );
        let mut cfg = config();
        cfg.file_identity = "copy.jsonl";
        let copied = parse(original.as_bytes(), &cfg);
        assert_eq!(
            older.events.iter().map(|e| &e.event_id).collect::<Vec<_>>(),
            copied
                .events
                .iter()
                .map(|e| &e.event_id)
                .collect::<Vec<_>>()
        );
    }
    #[test]
    fn incomplete_tail_is_not_consumed() {
        let original =
            include_bytes!("../../../../fixtures/agents/deepseek_harness/session-v4/input.jsonl");
        let mut appended = original.to_vec();
        appended.extend_from_slice(b"{\"type\":");
        let out = parse(&appended, &config());
        assert_eq!(out.events.len(), 4);
        assert_eq!(out.consumed, original.len() as u64);
    }
}
