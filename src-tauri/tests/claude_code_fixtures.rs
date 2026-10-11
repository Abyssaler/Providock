//! 用合成夹具驱动 A01 Claude Code projects JSONL 解析器
//! （fixtures/agents/claude_code/projects-jsonl/）。
//! 预期值由人工按 Anthropic 口径独立计算（见夹具 README），不由 parser 生成。

use providock_lib::adapters::agents::claude_code::{parse_projects, ClaudeParseConfig, ParsedEvent};
use providock_lib::domain::TokenSet;
use serde_json::Value;
use std::path::PathBuf;

fn cases_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/agents/claude_code/projects-jsonl")
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

fn sum_field(events: &[ParsedEvent], field: &str) -> Option<u128> {
    let mut acc: Option<u128> = None;
    for e in events {
        if let Some(v) = token_field(&e.event.tokens, field) {
            acc = Some(acc.unwrap_or(0) + u128::from(v));
        }
    }
    acc
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
    assert!(cases.len() >= 7, "fixture cases missing: {cases:?}");

    for case in cases {
        let dir = root.join(&case);
        let input_file = std::fs::read_dir(dir.join("input"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .unwrap_or_else(|| panic!("{case}: no .jsonl input"));
        let input = std::fs::read(&input_file).unwrap();
        let expected: Value =
            serde_json::from_slice(&std::fs::read(dir.join("expected.json")).unwrap()).unwrap();

        let cfgd = &expected["config"];
        let cfg = ClaudeParseConfig {
            source_id: format!("fixture:{case}"),
            file_identity: input_file
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            generation: 0,
            observed_at: "2026-10-11T08:00:00Z".into(),
            is_subagent: cfgd["is_subagent"].as_bool().unwrap(),
            subagent_actor_id: cfgd["subagent_actor_id"].as_str().map(str::to_string),
        };
        let out = parse_projects(&input, &cfg);

        assert_eq!(
            out.events.len(),
            expected["events"].as_u64().unwrap() as usize,
            "{case} event count"
        );
        // 原生 message ID 身份下所有事件都携带替换语义
        assert!(
            out.events.iter().all(|e| e.replace_existing),
            "{case} replace_existing"
        );

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
                    sum_field(&out.events, field),
                    want,
                    "{case} total {field}"
                );
            }
            let want_req: Option<u128> =
                totals["request_count"].as_str().map(|s| s.parse().unwrap());
            let got_req: Option<u128> = {
                let mut acc = None;
                for e in &out.events {
                    if let Some(v) = e.event.request_count {
                        acc = Some(acc.unwrap_or(0) + u128::from(v));
                    }
                }
                acc
            };
            assert_eq!(got_req, want_req, "{case} request_count");
        }

        if let Some(id) = expected.get("identity") {
            let want_session = id["session_id"].as_str();
            let want_parent = id["parent_session_id"].as_str();
            let want_actor = id["actor_id"].as_str();
            for e in &out.events {
                assert_eq!(
                    e.event.session_id.as_deref(),
                    want_session,
                    "{case} session_id"
                );
                assert_eq!(
                    e.event.parent_session_id.as_deref(),
                    want_parent,
                    "{case} parent_session_id"
                );
                assert_eq!(e.event.actor_id.as_deref(), want_actor, "{case} actor_id");
                assert_eq!(e.event.granularity.as_str(), "request", "{case} granularity");
                assert_eq!(e.event.request_count, Some(1), "{case} request_count per event");
            }
        }

        match expected["consumed"].as_str().unwrap() {
            "all" => assert_eq!(out.consumed_bytes, input.len() as u64, "{case} consumed"),
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
    }
}
