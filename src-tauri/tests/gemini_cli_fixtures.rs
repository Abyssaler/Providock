//! 用合成夹具驱动 Gemini CLI 解析器（fixtures/agents/gemini_cli/chats/）。
//! 预期值由人工按官方语义独立计算（见夹具 README 的算术过程），不由 parser 反推。

use providock_lib::adapters::agents::gemini_cli::{
    parse_session_json, parse_session_jsonl, GeminiParseConfig,
};
use providock_lib::domain::{AgentKind, Completeness, Granularity, Quality, TokenSet, UsageEvent};
use serde_json::Value;
use std::path::PathBuf;

fn cases_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/agents/gemini_cli/chats")
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
        other => panic!("unknown token field {other}"),
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
    assert!(cases.len() >= 10, "fixture cases missing: {cases:?}");

    for case in cases {
        let dir = root.join(&case);
        let expected: Value =
            serde_json::from_slice(&std::fs::read(dir.join("expected.json")).unwrap()).unwrap();

        // 形态由用例声明：json = 完整会话文件，jsonl = 回放格式
        let form = expected["form"].as_str().unwrap();
        let input_dir = dir.join("input");
        let input_file = std::fs::read_dir(&input_dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|x| x == form))
            .unwrap_or_else(|| panic!("{case}: no .{form} input"));
        let input = std::fs::read(&input_file).unwrap();

        let cfg = GeminiParseConfig {
            source_id: format!("fixture:{case}"),
            file_identity: input_file
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            generation: 0,
            observed_at: "2026-10-11T08:00:00Z".into(),
        };
        let out = match form {
            "json" => parse_session_json(&input, &cfg),
            "jsonl" => parse_session_jsonl(&input, &cfg),
            other => panic!("{case}: unknown form {other}"),
        };

        // 会话身份：即使没有用量事件也要保留（供调用方建会话索引）
        assert_eq!(
            out.session_id.as_deref(),
            expected["session_id"].as_str(),
            "{case} session_id"
        );

        // 事件数
        assert_eq!(
            out.events.len(),
            expected["events"].as_u64().unwrap() as usize,
            "{case} event count: {:?}",
            out.diagnostics
        );

        // 事件身份（gemini_cli:{sessionId}:{message.id}）
        let want_ids: Vec<Option<&str>> = expected["event_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str())
            .collect();
        let got_ids: Vec<Option<&str>> = out
            .events
            .iter()
            .map(|e| Some(e.event.event_id.as_str()))
            .collect();
        assert_eq!(got_ids, want_ids, "{case} event_ids");

        // 发生时间（源消息时间字段；无则 null）
        let want_occurred: Vec<Option<&str>> = expected["occurred_at"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str())
            .collect();
        let got_occurred: Vec<Option<&str>> = out
            .events
            .iter()
            .map(|e| e.event.occurred_at.as_deref())
            .collect();
        assert_eq!(got_occurred, want_occurred, "{case} occurred_at");

        // 替换语义：$set 同 id 更新 / 重复写入 / 完整 JSON 快照 → true
        let want_replace: Vec<bool> = expected["replace_existing"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_bool().unwrap())
            .collect();
        let got_replace: Vec<bool> = out.events.iter().map(|e| e.replace_existing).collect();
        assert_eq!(got_replace, want_replace, "{case} replace_existing");

        // 逐事件不变量 + 契约算术自洽
        for e in &out.events {
            assert_eq!(e.event.agent, AgentKind::GeminiCli, "{case} agent");
            assert_eq!(
                e.event.granularity,
                Granularity::Message,
                "{case} granularity"
            );
            assert_eq!(e.event.request_count, Some(1), "{case} request_count");
            assert_eq!(e.event.tokens.quality, Quality::Derived, "{case} quality");
            assert_eq!(
                e.event.completeness,
                Completeness::Complete,
                "{case} completeness"
            );
            assert_eq!(
                e.event.tokens.input_cache_write, None,
                "{case} cache write 必须是未知，不能写 0"
            );
            let t = &e.event.tokens;
            assert_eq!(
                t.input_total,
                t.input_fresh
                    .unwrap()
                    .checked_add(t.input_cache_read.unwrap()),
                "{case} input_total = fresh + read（cached 不重复加）"
            );
            assert_eq!(
                t.total,
                t.input_total.unwrap().checked_add(t.output_total.unwrap()),
                "{case} total = input_total + output_total"
            );
        }

        // Token 合计。字段全未知时期望为 null（例如源不提供的 input_cache_write）
        let events: Vec<UsageEvent> = out.events.iter().map(|e| e.event.clone()).collect();
        let totals = &expected["totals"];
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
            assert_eq!(sum_field(&events, field), want, "{case} total {field}");
        }
        let want_req: Option<u128> = totals["request_count"].as_str().map(|s| s.parse().unwrap());
        let got_req: Option<u128> = {
            let mut acc = None;
            for e in &events {
                if let Some(v) = e.request_count {
                    acc = Some(acc.unwrap_or(0) + u128::from(v));
                }
            }
            acc
        };
        assert_eq!(got_req, want_req, "{case} request_count");

        // 消费边界
        match expected["consumed"].as_str().unwrap() {
            "all" => assert_eq!(out.consumed_bytes, input.len() as u64, "{case} consumed"),
            "none" => assert_eq!(out.consumed_bytes, 0, "{case} consumed"),
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
