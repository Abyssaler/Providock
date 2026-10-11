//! 用合成夹具驱动网关 usage 解析器（fixtures/gateway/<protocol>/<case>/）。
//! 预期值由人工按官方语义独立计算（见夹具 README），不由 parser 生成。

use providock_lib::domain::{
    AgentKind, Completeness, Granularity, Quality, StreamDomain, TokenSet,
};
use providock_lib::gateway::{
    usage_from_response, usage_from_stream, GatewayProtocol, GatewayUsageConfig, UsageSet,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn cases_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/gateway")
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

fn parse_expected_u64(v: &Value, case: &str, field: &str) -> Option<u64> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.parse().unwrap_or_else(|e| panic!("{case}.{field}: {e}"))),
        other => panic!("{case}.{field}: expected string/null, got {other}"),
    }
}

fn cfg_for(protocol: GatewayProtocol, case: &str) -> GatewayUsageConfig {
    GatewayUsageConfig {
        source_id: format!("fixture:gateway:{case}"),
        provider: "fixture-provider".into(),
        protocol,
        file_identity: format!("fixture-{case}-conn"),
        generation: 0,
        observed_at: "2026-10-11T08:00:00Z".into(),
        agent: AgentKind::ClaudeDesktop,
        session_id: Some("fixture-session".into()),
        occurred_at: None,
    }
}

fn run_case(protocol_dir: &str, case_dir: &Path, expected: &Value) {
    let case_name = case_dir.file_name().unwrap().to_string_lossy();
    let case = format!("{protocol_dir}/{case_name}");
    let input = std::fs::read(case_dir.join(expected["input"].as_str().unwrap())).unwrap();
    let protocol = GatewayProtocol::parse(expected["protocol"].as_str().unwrap()).unwrap();

    let set: UsageSet = match expected["kind"].as_str().unwrap() {
        "non_stream" => usage_from_response(protocol, &input),
        "stream" => usage_from_stream(protocol, &input),
        other => panic!("{case}: unknown kind {other}"),
    };

    // Token 逐字段（None 表示未知）
    let want_tokens = &expected["tokens"];
    for field in [
        "input_fresh",
        "input_cache_read",
        "input_cache_write",
        "input_total",
        "output_total",
        "output_reasoning",
    ] {
        let want = parse_expected_u64(&want_tokens[field], &case, field);
        let got = match field {
            "input_fresh" => set.input_fresh,
            "input_cache_read" => set.input_cache_read,
            "input_cache_write" => set.input_cache_write,
            "input_total" => set.input_total,
            "output_total" => set.output_total,
            "output_reasoning" => set.output_reasoning,
            _ => unreachable!(),
        };
        assert_eq!(got, want, "{case} token {field}");
    }

    // model / provider_request_id
    let want_id = expected["request_id"].as_str();
    assert_eq!(
        set.provider_request_id.as_deref(),
        want_id,
        "{case} request_id"
    );
    let want_model = expected["model"].as_str();
    assert_eq!(set.model.as_deref(), want_model, "{case} model");

    // quality / completeness
    assert_eq!(
        set.quality.as_str(),
        expected["quality"].as_str().unwrap(),
        "{case} quality"
    );
    assert_eq!(
        set.completeness.as_str(),
        expected["completeness"].as_str().unwrap(),
        "{case} completeness"
    );

    // 诊断包含
    for d in expected["diagnostics_include"].as_array().unwrap() {
        let needle = d.as_str().unwrap();
        assert!(
            set.diagnostics.iter().any(|x| x.contains(needle)),
            "{case} diagnostics missing {needle}: {:?}",
            set.diagnostics
        );
    }

    // 转标准事件：网关域、request 粒度、请求数 1，且不写全 0 行
    let event = set.to_usage_event(&cfg_for(protocol, &case));
    assert_eq!(event.stream_domain, StreamDomain::Gateway, "{case}");
    assert_eq!(event.granularity, Granularity::Request, "{case}");
    assert_eq!(event.request_count, Some(1), "{case}");
    assert_eq!(event.agent, AgentKind::ClaudeDesktop, "{case}");
    assert_eq!(
        event.completeness,
        Completeness::parse(expected["completeness"].as_str().unwrap()).unwrap(),
        "{case} event completeness"
    );

    // total 只在 input_total 与 output_total 都已知时给出
    let want_total = parse_expected_u64(&want_tokens["total"], &case, "total");
    assert_eq!(
        token_field(&event.tokens, "total"),
        want_total,
        "{case} event total"
    );
    assert_eq!(
        event.tokens.quality,
        Quality::parse(expected["quality"].as_str().unwrap()).unwrap(),
        "{case} event token quality"
    );

    // provider_request_id 参与 event_id（无则回落到连接身份，本夹具均有 id）
    if let Some(id) = want_id {
        assert!(event.event_id.contains(id), "{case} event_id {id}");
    }

    // 无 usage 的用例：确认是全 null 且请求数仍为 1（不是全 0 行）
    if expected["tokens"]["input_total"].is_null() && expected["tokens"]["output_total"].is_null() {
        assert!(!set.has_any_token(), "{case} should have no token");
        for field in [
            "input_fresh",
            "input_cache_read",
            "input_cache_write",
            "input_total",
            "output_total",
            "output_reasoning",
            "total",
        ] {
            assert_eq!(
                token_field(&event.tokens, field),
                None,
                "{case} no-usage {field} must be null"
            );
        }
    }
}

#[test]
fn gateway_usage_fixture_cases() {
    let root = cases_root();
    let protocols: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            p.is_dir().then_some(p)
        })
        .collect();
    assert_eq!(
        protocols.len(),
        4,
        "expected 4 protocol dirs: {protocols:?}"
    );

    let mut total_cases = 0usize;
    for protocol_dir in protocols {
        let protocol_name = protocol_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let cases: Vec<PathBuf> = std::fs::read_dir(&protocol_dir)
            .unwrap()
            .filter_map(|e| {
                let p = e.unwrap().path();
                p.is_dir().then_some(p)
            })
            .collect();
        assert!(
            cases.len() >= 5,
            "{protocol_name} needs >=5 cases (non-stream/stream/truncated/no-usage/tool-call), got {}",
            cases.len()
        );
        for case_dir in cases {
            let expected: Value =
                serde_json::from_slice(&std::fs::read(case_dir.join("expected.json")).unwrap())
                    .unwrap();
            assert_eq!(
                expected["protocol"].as_str().unwrap(),
                protocol_name,
                "protocol dir/expected mismatch"
            );
            run_case(&protocol_name, &case_dir, &expected);
            total_cases += 1;
        }
    }
    assert_eq!(total_cases, 20, "expected 4 protocols x 5 cases");
}
