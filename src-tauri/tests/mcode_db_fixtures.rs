//! MCode runtime-state.sqlite 夹具驱动测试（fixtures/agents/mcode/runtime-state-sqlite/）。
//! 预期值按证据语义人工独立计算（见夹具 README），不由 parser 反推。
//! 库用只读规则打开（`open_read_only`）。

use providock_lib::adapters::agents::mcode_db::{
    open_read_only, read_events, McodeConfig, McodeStatus, DEFAULT_OVERLAP,
};
use providock_lib::domain::{TokenSet, UsageEvent};
use serde_json::Value;
use std::path::PathBuf;

fn cases_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/agents/mcode/runtime-state-sqlite")
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

fn sum_request_count(events: &[UsageEvent]) -> Option<u128> {
    let mut acc: Option<u128> = None;
    for e in events {
        if let Some(v) = e.request_count {
            acc = Some(acc.unwrap_or(0) + u128::from(v));
        }
    }
    acc
}

fn parse_opt_u128(v: &Value, ctx: &str) -> Option<u128> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(
            s.parse()
                .unwrap_or_else(|_| panic!("{ctx}: bad number {s}")),
        ),
        other => panic!("{ctx}: bad expected {other}"),
    }
}

fn row_id(event_id: &str) -> u64 {
    event_id
        .strip_prefix("mcode:")
        .unwrap_or_else(|| panic!("unexpected event_id {event_id}"))
        .parse()
        .unwrap_or_else(|_| panic!("non-numeric event_id {event_id}"))
}

fn check_totals(case: &str, label: &str, events: &[UsageEvent], totals: &Value) {
    for field in [
        "input_fresh",
        "input_cache_read",
        "input_cache_write",
        "input_total",
        "output_total",
        "output_reasoning",
        "total",
    ] {
        if let Some(v) = totals.get(field) {
            assert_eq!(
                sum_field(events, field),
                parse_opt_u128(v, &format!("{case}.{label}.{field}")),
                "{case} {label} total {field}"
            );
        }
    }
    if let Some(v) = totals.get("request_count") {
        assert_eq!(
            sum_request_count(events),
            parse_opt_u128(v, &format!("{case}.{label}.request_count")),
            "{case} {label} request_count"
        );
    }
}

#[test]
fn fixture_cases() {
    let root = cases_root();
    let mut cases: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            (p.is_dir() && p.join("expected.json").exists())
                .then(|| p.file_name().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    cases.sort();
    assert!(cases.len() >= 6, "fixture cases missing: {cases:?}");

    for case in cases {
        let dir = root.join(&case);
        let sqlite = dir.join("runtime-state.sqlite");
        let expected: Value =
            serde_json::from_slice(&std::fs::read(dir.join("expected.json")).unwrap()).unwrap();

        let conn = open_read_only(&sqlite).unwrap_or_else(|e| panic!("{case}: open failed: {e}"));

        let cfgd = &expected["config"];
        let after_id = cfgd["after_id"].as_u64();
        let overlap = cfgd["overlap"].as_u64().unwrap_or(DEFAULT_OVERLAP);
        let cfg = McodeConfig {
            source_id: format!("fixture:{case}"),
            file_identity: format!("fixture-{case}"),
            generation: 0,
            observed_at: "2026-10-11T08:00:00Z".into(),
            after_id,
            overlap,
        };
        let out = read_events(&conn, &cfg);

        // 状态
        let status_expect = expected["status"].as_str().unwrap();
        match status_expect {
            "ok" => assert_eq!(out.status, McodeStatus::Ok, "{case}"),
            s if s.starts_with("unsupported:") => {
                let needle = s.trim_start_matches("unsupported:");
                match &out.status {
                    McodeStatus::Unsupported(r) => assert!(r.contains(needle), "{case}: {r}"),
                    other => panic!("{case}: expected unsupported, got {other:?}"),
                }
            }
            s if s.starts_with("deferred:") => {
                let needle = s.trim_start_matches("deferred:");
                match &out.status {
                    McodeStatus::Deferred(r) => assert!(r.contains(needle), "{case}: {r}"),
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

        // 输出事件 id 集合（顺序即 ORDER BY id）
        if let Some(ids) = expected.get("event_ids") {
            let want: Vec<&str> = ids
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            let got: Vec<&str> = out.events.iter().map(|e| e.event_id.as_str()).collect();
            assert_eq!(got, want, "{case} event_ids");
        }

        // 增量新增 id 集合：直接断言 id > after_id 的输出（模拟入库层去重后的新事件）
        if let Some(ids) = expected.get("new_event_ids") {
            let want: Vec<&str> = ids
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            let got: Vec<String> = out
                .events
                .iter()
                .filter(|e| after_id.is_none_or(|a| row_id(&e.event_id) > a))
                .map(|e| e.event_id.clone())
                .collect();
            let want: Vec<String> = want.into_iter().map(str::to_string).collect();
            assert_eq!(got, want, "{case} new_event_ids");
        }

        // max_id（游标推进）
        if let Some(m) = expected.get("max_id") {
            let want = m.as_str().map(|s| s.parse::<u64>().unwrap());
            assert_eq!(out.max_id, want, "{case} max_id");
        }

        // 逐事件 model_raw（原样）
        if let Some(models) = expected.get("models") {
            let want: Vec<Option<&str>> = models
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str())
                .collect();
            let got: Vec<Option<&str>> =
                out.events.iter().map(|e| e.model_raw.as_deref()).collect();
            assert_eq!(got, want, "{case} models");
        }

        // 逐事件 occurred_at
        if let Some(at) = expected.get("occurred_at") {
            let want: Vec<Option<&str>> =
                at.as_array().unwrap().iter().map(|v| v.as_str()).collect();
            let got: Vec<Option<&str>> = out
                .events
                .iter()
                .map(|e| e.occurred_at.as_deref())
                .collect();
            assert_eq!(got, want, "{case} occurred_at");
        }

        // 逐事件 cost_reported（金额+币种，或 null）

        if let Some(totals) = expected.get("totals") {
            check_totals(&case, "all", &out.events, totals);
        }

        if let Some(totals) = expected.get("new_totals") {
            let new_events: Vec<UsageEvent> = out
                .events
                .iter()
                .filter(|e| after_id.is_none_or(|a| row_id(&e.event_id) > a))
                .cloned()
                .collect();
            check_totals(&case, "new", &new_events, totals);
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
