//! Codex rollout 夹具驱动测试（fixtures/agents/codex/rollout-jsonl/）。
//! 预期值人工独立计算（见夹具 README），不由 parser 生成。

use providock_lib::adapters::agents::codex_rollout::{
    parse_rollout, CodexParseConfig, CodexStatus, EventSignature,
};
use providock_lib::domain::{TokenSet, UsageEvent};
use serde_json::Value;
use std::path::PathBuf;

fn cases_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/agents/codex/rollout-jsonl")
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

fn sum_field(events: &[UsageEvent], field: &str) -> Option<u128> {
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
        let input_dir = dir.join("input");
        let input_file = std::fs::read_dir(&input_dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .unwrap_or_else(|| panic!("{case}: no .jsonl input"));
        let input = std::fs::read(&input_file).unwrap();
        let expected: Value =
            serde_json::from_slice(&std::fs::read(dir.join("expected.json")).unwrap()).unwrap();

        let cfgd = &expected["config"];
        let parent_signatures: Option<Vec<EventSignature>> =
            cfgd["parent_signatures"].as_array().map(|arr| {
                arr.iter()
                    .map(|s| {
                        let a = s.as_array().unwrap();
                        EventSignature {
                            input: a[0].as_u64().unwrap(),
                            cached: a[1].as_u64().unwrap(),
                            cache_write: a[2].as_u64().unwrap(),
                            output: a[3].as_u64().unwrap(),
                            reasoning: a[4].as_u64().unwrap(),
                            total: a[5].as_u64(),
                        }
                    })
                    .collect()
            });
        let cfg = CodexParseConfig {
            source_id: format!("fixture:{case}"),
            file_identity: input_file
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            generation: 0,
            observed_at: "2026-10-11T08:00:00Z".into(),
            filename_thread_id: cfgd["filename_thread_id"].as_str().map(str::to_string),
            parent_signatures,
        };
        let out = parse_rollout(&input, &cfg);

        // 状态
        let status_expect = expected["status"].as_str().unwrap();
        match status_expect {
            "ok" => assert_eq!(out.status, Some(CodexStatus::Ok), "{case}"),
            s if s.starts_with("deferred:") => {
                let reason = s.trim_start_matches("deferred:");
                match &out.status {
                    Some(CodexStatus::Deferred(r)) => {
                        assert!(r.contains(reason), "{case}: {r} vs {reason}")
                    }
                    other => panic!("{case}: expected deferred, got {other:?}"),
                }
            }
            other => panic!("{case}: bad status expectation {other}"),
        }

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
                "total",
            ] {
                let want: Option<u128> = match &totals[field] {
                    Value::Null => None,
                    Value::String(s) => Some(s.parse().unwrap()),
                    other => panic!("{case}.{field}: bad expected {other}"),
                };
                assert_eq!(sum_field(&out.events, field), want, "{case} total {field}");
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

        if let Some(g) = expected.get("granularity") {
            let want = g.as_str().unwrap();
            for e in &out.events {
                assert_eq!(e.granularity.as_str(), want, "{case} granularity");
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
    }
}
