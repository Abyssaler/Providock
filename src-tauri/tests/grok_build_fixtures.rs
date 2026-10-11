//! Grok Build `updates.jsonl` 夹具驱动测试（fixtures/agents/grok_build/updates-jsonl/）。
//! 预期值由人工按证据语义独立计算（见夹具 README），不由 parser 生成。

use providock_lib::adapters::agents::grok_build::{parse_updates, GrokParseConfig};
use providock_lib::domain::TokenSet;
use serde_json::Value;
use std::path::PathBuf;

fn cases_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/agents/grok_build/updates-jsonl")
}

fn token_field(t: &TokenSet, name: &str) -> Option<u64> {
    match name {
        "input_fresh" => t.input_fresh,
        "input_cache_read" => t.input_cache_read,
        "input_cache_write" => t.input_cache_write,
        "input_total" => t.input_total,
        "output_total" => t.output_total,
        "output_reasoning" => t.output_reasoning,
        "total" => t.total,
        _ => panic!("unknown token field {name}"),
    }
}

fn sum_field(events: &[providock_lib::domain::UsageEvent], field: &str) -> Option<u128> {
    let mut acc: Option<u128> = None;
    for e in events {
        if let Some(v) = token_field(&e.tokens, field) {
            acc = Some(acc.unwrap_or(0) + u128::from(v));
        }
    }
    acc
}

fn expected_u128(v: &Value, case: &str, what: &str) -> Option<u128> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.parse().unwrap_or_else(|_| panic!("{case}.{what}: {s}"))),
        other => panic!("{case}.{what}: bad expected {other}"),
    }
}

#[test]
fn fixture_cases() {
    let root = cases_root();
    let mut cases: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            p.is_dir()
                .then(|| p.file_name().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    cases.sort();
    assert!(cases.len() >= 9, "fixture cases missing: {cases:?}");

    for case in cases {
        let dir = root.join(&case);
        let input = std::fs::read(dir.join("input/updates.jsonl")).unwrap();
        let expected: Value =
            serde_json::from_slice(&std::fs::read(dir.join("expected.json")).unwrap()).unwrap();

        let cfg = GrokParseConfig {
            source_id: format!("fixture:{case}"),
            file_identity: format!("fixture-{case}-updates"),
            generation: 0,
            session_dir_name: format!("session-{case}"),
            archived: expected["config"]["archived"].as_bool().unwrap(),
            observed_at: "2026-10-11T08:00:00Z".into(),
        };
        let out = parse_updates(&input, &cfg);

        assert_eq!(
            out.events.len(),
            expected["events"].as_u64().unwrap() as usize,
            "{case} event count"
        );

        if let Some(totals) = expected.get("totals") {
            for field in [
                "input_fresh",
                "input_cache_read",
                "input_cache_write",
                "input_total",
                "output_total",
                "output_reasoning",
                "total",
            ] {
                let want = expected_u128(&totals[field], &case, field);
                assert_eq!(sum_field(&out.events, field), want, "{case} total {field}");
            }
            let want_req = expected_u128(&totals["request_count"], &case, "request_count");
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

        for (i, exp) in expected["event_expect"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let e = &out.events[i];
            if let Some(t) = exp["turn_id"].as_str() {
                assert_eq!(e.turn_id.as_deref(), Some(t), "{case}[{i}] turn_id");
            }
            match &exp["model_raw"] {
                Value::Null => assert_eq!(e.model_raw, None, "{case}[{i}] model_raw"),
                Value::String(s) => {
                    assert_eq!(
                        e.model_raw.as_deref(),
                        Some(s.as_str()),
                        "{case}[{i}] model_raw"
                    )
                }
                other => panic!("{case}[{i}] bad model_raw {other}"),
            }
            if let Some(t) = exp["occurred_at"].as_str() {
                assert_eq!(e.occurred_at.as_deref(), Some(t), "{case}[{i}] occurred_at");
            }
            if let Some(g) = exp["granularity"].as_str() {
                assert_eq!(e.granularity.as_str(), g, "{case}[{i}] granularity");
            }
            if let Some(c) = exp["completeness"].as_str() {
                assert_eq!(e.completeness.as_str(), c, "{case}[{i}] completeness");
            }
            match &exp["cost"] {
                Value::Null => {
                    assert!(
                        e.cost_reported.is_none(),
                        "{case}[{i}] cost should be null (无价不等于免费)"
                    )
                }
                c => {
                    let cost = e
                        .cost_reported
                        .as_ref()
                        .unwrap_or_else(|| panic!("{case}[{i}] missing cost_reported"));
                    assert_eq!(
                        cost.money.amount,
                        c["amount"].as_str().unwrap(),
                        "{case}[{i}] amount"
                    );
                    assert_eq!(
                        cost.money.currency,
                        c["currency"].as_str().unwrap(),
                        "{case}[{i}] currency"
                    );
                    assert_eq!(
                        cost.quality.as_str(),
                        c["quality"].as_str().unwrap(),
                        "{case}[{i}] quality"
                    );
                    assert_eq!(
                        cost.completeness.as_str(),
                        c["completeness"].as_str().unwrap(),
                        "{case}[{i}] cost completeness"
                    );
                }
            }
        }

        match expected["consumed"].as_str().unwrap() {
            "all" => assert_eq!(out.consumed_bytes, input.len() as u64, "{case} consumed"),
            "any" => {}
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

        for d in expected["diagnostics_include"].as_array().unwrap() {
            let needle = d.as_str().unwrap();
            assert!(
                out.diagnostics.iter().any(|x| x.contains(needle)),
                "{case} diagnostics missing {needle}: {:?}",
                out.diagnostics
            );
        }

        if let Some(counts) = expected.get("diagnostics_count") {
            for (needle, want) in counts.as_object().unwrap() {
                let got = out
                    .diagnostics
                    .iter()
                    .filter(|x| x.contains(needle.as_str()))
                    .count();
                assert_eq!(
                    got,
                    want.as_u64().unwrap() as usize,
                    "{case} diagnostics count {needle}"
                );
            }
        }
    }
}
