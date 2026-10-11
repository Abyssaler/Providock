//! 用合成夹具驱动 Kimi wire 解析器（fixtures/agents/kimi_code/wire-1.5/）。
//! 预期值由人工按官方语义独立计算（见夹具 README），不由 parser 生成。

use providock_lib::adapters::agents::kimi_wire::{parse_wire, Compatibility, WireParseConfig};
use serde_json::Value;
use std::path::PathBuf;

fn cases_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/agents/kimi_code/wire-1.5")
}

fn token_field(t: &providock_lib::domain::TokenSet, name: &str) -> Option<u64> {
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

fn expected_total(events: &[providock_lib::domain::UsageEvent], field: &str) -> Option<u128> {
    let mut acc: Option<u128> = None;
    for e in events {
        if let Some(v) = token_field(&e.tokens, field) {
            acc = Some(acc.unwrap_or(0) + u128::from(v));
        }
    }
    acc
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
        case_names.len() >= 7,
        "fixture cases missing: {case_names:?}"
    );

    for case in case_names {
        let dir = root.join(&case);
        let input = std::fs::read(dir.join("input/wire.jsonl")).unwrap();
        let expected: Value =
            serde_json::from_slice(&std::fs::read(dir.join("expected.json")).unwrap()).unwrap();

        let cfgd = &expected["config"];
        let cfg = WireParseConfig {
            source_id: format!("fixture:{case}"),
            file_identity: format!("fixture-{case}-wire"),
            generation: 0,
            session_id: "session_fixture".into(),
            dir_agent_id: Some("main".into()),
            forked_from_present: cfgd["forked_from_present"].as_bool().unwrap(),
            parent_available: cfgd["parent_available"].as_bool().unwrap(),
            observed_at: "2026-10-11T08:00:00Z".into(),
        };
        let out = parse_wire(&input, &cfg);

        // 兼容性
        let compat_expect = expected["compatibility"].as_str().unwrap();
        match compat_expect {
            "ok" => assert_eq!(out.compatibility, Compatibility::Ok, "{case}"),
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
            s if s.starts_with("before_marker:") => {
                let marker = s.trim_start_matches("before_marker:");
                let pos = input
                    .windows(marker.len())
                    .position(|w| w == marker.as_bytes())
                    .unwrap() as u64;
                assert_eq!(out.consumed_bytes, pos, "{case} consumed before marker");
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
