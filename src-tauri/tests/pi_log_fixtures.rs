//! 用合成夹具驱动 Pi 会话解析器（fixtures/agents/pi/sessions-jsonl/）。
//! 预期值由人工按官方语义独立计算（见夹具 README），不由 parser 生成。

use providock_lib::adapters::agents::pi_log::{parse_session, PiParseConfig};
use providock_lib::domain::{AgentKind, TokenSet, UsageEvent};
use serde_json::Value;
use std::path::PathBuf;

fn cases_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/agents/pi/sessions-jsonl")
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

fn sum_field(events: &[UsageEvent], field: &str) -> Option<u128> {
    let mut acc: Option<u128> = None;
    for e in events {
        if let Some(v) = token_field(&e.tokens, field) {
            acc = Some(acc.unwrap_or(0) + u128::from(v));
        }
    }
    acc
}

/// 每个用例的 input/ 目录下恰好一个 .jsonl（真实 Pi 文件名为 `<timestamp>_<session-id>.jsonl`）。
fn case_input(case: &str, dir: &std::path::Path) -> (PathBuf, Vec<u8>) {
    let input_dir = dir.join("input");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&input_dir)
        .unwrap_or_else(|e| panic!("{case}: input dir unreadable: {e}"))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    assert_eq!(files.len(), 1, "{case}: expected exactly one .jsonl input");
    let path = files.pop().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    (path, bytes)
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
        let (input_file, input) = case_input(&case, &dir);
        let expected: Value =
            serde_json::from_slice(&std::fs::read(dir.join("expected.json")).unwrap()).unwrap();

        let cfg = PiParseConfig {
            source_id: format!("fixture:{case}"),
            file_identity: input_file
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            generation: 0,
            observed_at: "2026-10-11T08:00:00Z".into(),
        };
        let out = parse_session(&input, &cfg);

        // 会话身份
        match &expected["session_id"] {
            Value::Null => assert_eq!(out.session_id, None, "{case} session_id"),
            Value::String(s) => assert_eq!(
                out.session_id.as_deref(),
                Some(s.as_str()),
                "{case} session_id"
            ),
            other => panic!("{case}: bad session_id expectation {other}"),
        }

        // 事件数与基本不变量
        assert_eq!(
            out.events.len(),
            expected["events"].as_u64().unwrap() as usize,
            "{case} event count"
        );
        for (i, e) in out.events.iter().enumerate() {
            assert_eq!(e.agent, AgentKind::Pi, "{case} event {i} agent");
            assert_eq!(e.session_id, out.session_id, "{case} event {i} session");
            assert_eq!(e.request_count, Some(1), "{case} event {i} request_count");
            assert!(
                e.event_id
                    .starts_with(&format!("pi:{}:g0:o", cfg.file_identity)),
                "{case} event {i} event_id {}",
                e.event_id
            );
        }

        // Token 合计
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

        // 粒度分布
        if let Some(counts) = expected.get("granularity_counts") {
            for (name, want) in counts.as_object().unwrap() {
                let got = out
                    .events
                    .iter()
                    .filter(|e| e.granularity.as_str() == name)
                    .count();
                assert_eq!(
                    got as u64,
                    want.as_u64().unwrap(),
                    "{case} granularity {name}"
                );
            }
        }

        // 费用（与事件一一对应）

        // 消费边界
        match expected["consumed"].as_str().unwrap() {
            "all" => assert_eq!(out.consumed_bytes, input.len() as u64, "{case} consumed"),
            "last_newline" => {
                let pos = input.iter().rposition(|b| *b == b'\n').unwrap() as u64 + 1;
                assert_eq!(
                    out.consumed_bytes, pos,
                    "{case} consumed after last newline"
                );
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
