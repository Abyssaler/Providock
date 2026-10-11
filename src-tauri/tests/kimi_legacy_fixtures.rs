//! 用合成夹具驱动旧版 Python kimi-cli wire 解析器（fixtures/agents/kimi_legacy/wire-python/）。
//! 预期值由人工按官方语义独立计算（见夹具 README），不由 parser 生成。

use providock_lib::adapters::agents::kimi_legacy::{
    parse_wire, Compatibility, ForkContext, WireParseConfig,
};
use providock_lib::domain::{TokenSet, UsageEvent};
use serde_json::Value;
use std::path::PathBuf;

fn cases_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/agents/kimi_legacy/wire-python")
}

fn token_field(t: &TokenSet, name: &str) -> Option<u64> {
    match name {
        "input_fresh" => t.input_fresh,
        "input_cache_read" => t.input_cache_read,
        "input_cache_write" => t.input_cache_write,
        "input_total" => t.input_total,
        "output_total" => t.output_total,
        "total" => t.total,
        _ => panic!("unknown token field {name}"),
    }
}

fn expected_total(events: &[UsageEvent], field: &str) -> Option<u128> {
    let mut acc: Option<u128> = None;
    for e in events {
        if let Some(v) = token_field(&e.tokens, field) {
            acc = Some(acc.unwrap_or(0) + u128::from(v));
        }
    }
    acc
}

/// 第 `line_no` 行（1 起，含首行）的起始字节偏移。
fn line_start(input: &[u8], line_no: usize) -> u64 {
    assert!(line_no >= 1);
    let mut idx = 0usize;
    for _ in 1..line_no {
        let pos = input[idx..]
            .iter()
            .position(|b| *b == b'\n')
            .unwrap_or_else(|| panic!("第 {line_no} 行不存在"));
        idx += pos + 1;
    }
    idx as u64
}

#[test]
fn fixture_cases() {
    let root = cases_root();
    let mut case_names: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            p.is_dir()
                .then(|| p.file_name().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    case_names.sort();
    assert!(
        case_names.len() >= 4,
        "fixture cases missing: {case_names:?}"
    );

    for case in case_names {
        let dir = root.join(&case);
        let input = std::fs::read(dir.join("input/wire.jsonl")).unwrap();
        let expected: Value =
            serde_json::from_slice(&std::fs::read(dir.join("expected.json")).unwrap()).unwrap();

        let cfgd = &expected["config"];
        let fork = match cfgd["fork"].as_str().unwrap() {
            "none" => ForkContext::None,
            "unresolved" => ForkContext::Unresolved,
            "source_unavailable" => ForkContext::SourceUnavailable,
            s if s.starts_with("inherited_before_line:") => {
                let n: usize = s
                    .trim_start_matches("inherited_before_line:")
                    .parse()
                    .unwrap();
                ForkContext::Inherited {
                    prefix_end: line_start(&input, n),
                }
            }
            other => panic!("{case}: unknown fork config {other}"),
        };
        let cfg = WireParseConfig {
            source_id: format!("fixture:{case}"),
            file_identity: format!("fixture-{case}-wire"),
            generation: 0,
            session_id: "11111111-aaaa-4bbb-8ccc-111111111111".into(),
            agent_id: cfgd["agent_id"].as_str().unwrap().to_string(),
            mirrored_in_parent: cfgd["mirrored_in_parent"].as_bool().unwrap(),
            count_mirrored_subagent_usage: cfgd["count_mirrored_subagent_usage"].as_bool().unwrap(),
            fork,
            observed_at: "2026-10-11T08:00:00Z".into(),
        };
        let out = parse_wire(&input, &cfg);

        // 兼容性
        match expected["compatibility"].as_str().unwrap() {
            "ok" => assert_eq!(out.compatibility, Compatibility::Ok, "{case}"),
            "missing_metadata" => {
                assert_eq!(out.compatibility, Compatibility::MissingMetadata, "{case}")
            }
            s if s.starts_with("unsupported_protocol:") => {
                let v = s.trim_start_matches("unsupported_protocol:");
                assert_eq!(
                    out.compatibility,
                    Compatibility::UnsupportedProtocol(v.to_string()),
                    "{case}"
                );
            }
            other => panic!("{case}: unknown expected compatibility {other}"),
        }

        // 事件数
        assert_eq!(
            out.events.len(),
            expected["events"].as_u64().unwrap() as usize,
            "{case} event count"
        );

        // Token 合计（None 期望 = 全部事件该字段未知）
        if let Some(totals) = expected.get("totals") {
            for field in [
                "input_fresh",
                "input_cache_read",
                "input_cache_write",
                "input_total",
                "output_total",
                "total",
            ] {
                let want: Option<u128> = match &totals[field] {
                    Value::Null => None,
                    Value::String(s) => Some(s.parse().unwrap()),
                    other => panic!("{case}.{field}: bad expected {other}"),
                };
                assert_eq!(
                    expected_total(&out.events, field),
                    want,
                    "{case} total {field}"
                );
            }
            let want_req: Option<u128> =
                totals["request_count"].as_str().map(|s| s.parse().unwrap());
            let got_req: Option<u128> = {
                let mut acc = None;
                for e in &out.events {
                    if let Some(v) = e.request_count {
                        acc = Some(acc.unwrap_or(0) + u128::from(v));
                    }
                }
                acc
            };
            assert_eq!(got_req, want_req, "{case} request_count");
        }

        // actor 归属（子 Agent / 镜像用例）
        if let Some(actors) = expected.get("actors") {
            for (actor, spec) in actors.as_object().unwrap() {
                let mine: Vec<&UsageEvent> = out
                    .events
                    .iter()
                    .filter(|e| e.actor_id.as_deref() == Some(actor.as_str()))
                    .collect();
                assert_eq!(
                    mine.len(),
                    spec["events"].as_u64().unwrap() as usize,
                    "{case}: actor {actor} event count"
                );
                let want_total: Option<u128> = spec["total"].as_str().map(|s| s.parse().unwrap());
                let got_total: Option<u128> = {
                    let mut acc = None;
                    for e in &mine {
                        if let Some(v) = e.tokens.total {
                            acc = Some(acc.unwrap_or(0) + u128::from(v));
                        }
                    }
                    acc
                };
                assert_eq!(got_total, want_total, "{case}: actor {actor} total");
            }
            // 期望的 actor 集合必须恰好覆盖全部事件
            let expected_actors: u64 = actors
                .as_object()
                .unwrap()
                .values()
                .map(|s| s["events"].as_u64().unwrap())
                .sum();
            assert_eq!(
                expected_actors,
                out.events.len() as u64,
                "{case}: actors 覆盖全部事件"
            );
        }

        // completeness 断言（可选）
        if let Some(c) = expected.get("completeness") {
            let want = c.as_str().unwrap();
            for e in &out.events {
                assert_eq!(e.completeness.as_str(), want, "{case} completeness");
            }
        }

        // 消费边界
        match expected["consumed"].as_str().unwrap() {
            "all" => assert_eq!(out.consumed_bytes, input.len() as u64, "{case} consumed"),
            "first_line_only" => {
                let first_nl = input.iter().position(|b| *b == b'\n').unwrap() as u64;
                assert_eq!(out.consumed_bytes, first_nl + 1, "{case} consumed");
            }
            "before_trailing_partial" => {
                let last_nl = input.iter().rposition(|b| *b == b'\n').unwrap() as u64;
                assert_eq!(
                    out.consumed_bytes,
                    last_nl + 1,
                    "{case} consumed（未写完的尾行不消费）"
                );
            }
            other => panic!("{case}: unknown consumed expectation {other}"),
        }

        // 诊断包含
        for d in expected["diagnostics_include"].as_array().unwrap() {
            let needle = d.as_str().unwrap();
            assert!(
                out.diagnostics.iter().any(|x| x.contains(needle)),
                "{case} diagnostics missing {needle}: {:?}",
                out.diagnostics
            );
        }
    }
}
