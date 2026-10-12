//! T10 B05–B07 账户适配器集成测试：GLM Coding Plan、Kimi 本机 Server、Codex App Server。
//!
//! 覆盖：请求构造（host 判定、查询串、auth scheme、**不装秘密**）、脱敏响应夹具解析、
//! 错误映射（401/403、429、HTTP200 业务码、schema 变化、能力缺失）。全部为纯函数 +
//! 本地夹具，不触网、不发送推理请求。
//!
//! 预期值取自 `fixtures/accounts/**/expected.json`（人工按证据语义独立计算，见各 README），
//! 断言用「子集比较」：只校验 expected 里出现的字段，避免与无关字段耦合并防止漂移。

use providock_lib::adapters::accounts::{
    codex_appserver, common, glm_plan, kimi_code, kimi_server,
};
use providock_lib::services::accounts::SnapshotStatus;
use serde_json::Value;
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/accounts")
}

fn read_bytes(rel: &str) -> Vec<u8> {
    let path = fixtures().join(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

fn read_json(rel: &str) -> Value {
    serde_json::from_slice(&read_bytes(rel)).unwrap_or_else(|e| panic!("json {rel}: {e}"))
}

/// `expected` 出现的字段/数组元素必须与 `actual` 一致（expected 是 actual 的子集）。
fn assert_contains(actual: &Value, expected: &Value, path: &str) {
    match expected {
        Value::Object(fields) => {
            let actual = actual
                .as_object()
                .unwrap_or_else(|| panic!("{path}: expected object, got {actual}"));
            for (key, want) in fields {
                let got = actual
                    .get(key)
                    .unwrap_or_else(|| panic!("{path}.{key}: missing in actual"));
                assert_contains(got, want, &format!("{path}.{key}"));
            }
        }
        Value::Array(items) => {
            let actual = actual
                .as_array()
                .unwrap_or_else(|| panic!("{path}: expected array, got {actual}"));
            assert_eq!(actual.len(), items.len(), "{path}: array length");
            for (index, want) in items.iter().enumerate() {
                assert_contains(&actual[index], want, &format!("{path}[{index}]"));
            }
        }
        other => assert_eq!(actual, other, "{path}"),
    }
}

// ---------------------------------------------------------------------------
// B05 GLM Coding Plan
// ---------------------------------------------------------------------------

#[test]
fn glm_region_and_request_shape() {
    use glm_plan::{build_request, GlmEndpoint, GlmError, GlmRegion, TimeWindow};

    // 三个 host 精确判定区域；URL 用配置 base 的实际 origin。
    for (base, region, host) in [
        (
            "https://api.z.ai/api/anthropic",
            GlmRegion::Global,
            "api.z.ai",
        ),
        (
            "https://open.bigmodel.cn/api/anthropic",
            GlmRegion::China,
            "open.bigmodel.cn",
        ),
        (
            "https://dev.bigmodel.cn/api/anthropic",
            GlmRegion::China,
            "dev.bigmodel.cn",
        ),
    ] {
        assert_eq!(glm_plan::region_for_base(base).unwrap(), region);
        let spec = build_request(base, GlmEndpoint::QuotaLimit, None).unwrap();
        assert_eq!(spec.method, "GET");
        assert_eq!(spec.host, host);
        assert_eq!(spec.path, "/api/monitor/usage/quota/limit");
        // GLM 是原 token，不加 Bearer（认证位置统一为 common::AuthPlacement）。
        assert_eq!(spec.auth, Some(common::AuthPlacement::raw_token()));
        assert_eq!(
            spec.url(),
            format!("https://{host}/api/monitor/usage/quota/limit")
        );
        assert!(spec
            .headers
            .iter()
            .any(|(k, v)| k == "Accept-Language" && v == "en-US,en"));
        assert!(spec
            .headers
            .iter()
            .any(|(k, v)| k == "Content-Type" && v == "application/json"));
        // 不装秘密：没有任何 Authorization 头（凭据由服务层注入）。
        assert!(spec
            .headers
            .iter()
            .all(|(k, _)| !k.eq_ignore_ascii_case("authorization")));
    }

    // 未知 host 精确拒绝；含相似子串的域名也不是 GLM。
    assert!(matches!(
        build_request("https://api.deepseek.com/x", GlmEndpoint::QuotaLimit, None),
        Err(GlmError::UnrecognizedHost(_))
    ));
    assert!(matches!(
        build_request("https://evil-z.ai/api", GlmEndpoint::QuotaLimit, None),
        Err(GlmError::UnrecognizedHost(_))
    ));

    // quota/limit 无查询串；传时间窗是调用方错误。
    assert!(matches!(
        build_request(
            "https://api.z.ai/api/anthropic",
            GlmEndpoint::QuotaLimit,
            Some(TimeWindow {
                start_ms: 0,
                end_ms: 0
            })
        ),
        Err(GlmError::InvalidBaseUrl(_))
    ));

    // model-usage 带 startTime/endTime（`yyyy-MM-dd HH:mm:ss`，encodeURIComponent 编码）。
    let spec = build_request(
        "https://api.z.ai/api/anthropic",
        GlmEndpoint::ModelUsage,
        Some(TimeWindow {
            start_ms: 1_791_622_800_000, // 2026-10-10T09:00:00Z
            end_ms: 1_791_709_199_999,   // 2026-10-11T08:59:59Z
        }),
    )
    .unwrap();
    assert_eq!(
        spec.url(),
        "https://api.z.ai/api/monitor/usage/model-usage\
         ?startTime=2026-10-10%2009%3A00%3A00&endTime=2026-10-11%2008%3A59%3A59"
    );

    // model-usage 缺时间窗是错误。
    assert!(matches!(
        build_request(
            "https://open.bigmodel.cn/api/anthropic",
            GlmEndpoint::ToolUsage,
            None
        ),
        Err(GlmError::InvalidBaseUrl(_))
    ));
}

#[test]
fn glm_quota_limit_fixture_wrapped_and_unwrapped() {
    use glm_plan::{parse_response, GlmEndpoint};

    let report = parse_response(
        GlmEndpoint::QuotaLimit,
        200,
        &read_bytes("glm_plan/quota-limit/input/quota_limit.json"),
    )
    .unwrap();
    assert_eq!(report.windows.len(), 2);
    assert_contains(
        &serde_json::to_value(&report).unwrap(),
        &read_json("glm_plan/quota-limit/expected.json"),
        "glm.quota",
    );

    // 无 `data` 包装（`json.data || json`）；percentage 字符串与整数都接受。
    let report = parse_response(
        GlmEndpoint::QuotaLimit,
        200,
        &read_bytes("glm_plan/quota-limit-unwrapped/input/quota_limit.json"),
    )
    .unwrap();
    assert_contains(
        &serde_json::to_value(&report).unwrap(),
        &read_json("glm_plan/quota-limit-unwrapped/expected.json"),
        "glm.unwrapped",
    );
}

#[test]
fn glm_duplicate_window_and_over_limit_are_disambiguated() {
    use glm_plan::{parse_response, GlmEndpoint};

    // 新套餐可能出现两条 TOKENS_LIMIT：窗口 id 唯一化，超 100% 记 raw/over。
    let body = br#"{"data":{"limits":[
        {"type":"TOKENS_LIMIT","percentage":120},
        {"type":"TOKENS_LIMIT","percentage":5}
    ]}}"#;
    let report = parse_response(GlmEndpoint::QuotaLimit, 200, body).unwrap();
    assert_eq!(report.windows.len(), 2);
    assert_eq!(
        report.windows[0].window_id,
        "glm:coding_plan:account:5h:tokens"
    );
    assert_eq!(
        report.windows[1].window_id,
        "glm:coding_plan:account:5h:tokens:2"
    );
    // 120%：封顶 1，原比例进 raw_ratio，标 over_limit。
    assert_eq!(report.windows[0].used_ratio.as_deref(), Some("1"));
    assert_eq!(report.windows[0].remaining_ratio.as_deref(), Some("0"));
    assert_eq!(report.windows[0].raw_ratio.as_deref(), Some("1.2"));
    assert_eq!(report.windows[0].over_limit, Some(true));
    assert_eq!(report.windows[1].used_ratio.as_deref(), Some("0.05"));
    assert_eq!(report.windows[1].over_limit, None);

    // 未知 type 只记诊断，不报错。
    let body = br#"{"data":{"limits":[{"type":"CREDIT_LIMIT","percentage":50}]}}"#;
    let report = parse_response(GlmEndpoint::QuotaLimit, 200, body).unwrap();
    assert!(report.windows.is_empty());
    assert!(report
        .diagnostics
        .iter()
        .any(|d| d.contains("unknown_limit_type:CREDIT_LIMIT")));
}

#[test]
fn glm_error_mapping() {
    use glm_plan::{parse_response, GlmEndpoint, GlmError};

    // HTTP200 + 业务码 401 → auth_required。
    let error = parse_response(
        GlmEndpoint::QuotaLimit,
        200,
        &read_bytes("glm_plan/business-error/input/quota_limit.json"),
    )
    .unwrap_err();
    match &error {
        GlmError::Business { code, .. } => assert_eq!(code, "401"),
        other => panic!("expected business error, got {other:?}"),
    }
    assert_eq!(error.status(), SnapshotStatus::AuthRequired);
    assert!(error.failure_signal().is_failure());

    // 真实 401 信封同时带 `code:401` 与 `success:false`：数值 code 优先，
    // 不得误分类为 success_false（unavailable）。2026-10-12 误分类修复的回归。
    let error = parse_response(
        GlmEndpoint::QuotaLimit,
        200,
        &read_bytes("glm_plan/business-error-auth/input/quota_limit.json"),
    )
    .unwrap_err();
    match &error {
        GlmError::Business { code, message } => {
            assert_eq!(code, "401");
            assert_eq!(message, "token expired or incorrect");
        }
        other => panic!("expected business 401, got {other:?}"),
    }
    assert_eq!(error.status(), SnapshotStatus::AuthRequired);
    let signal = error.failure_signal();
    assert_eq!(signal.business_code.as_deref(), Some("401"));
    assert_eq!(signal.status(), SnapshotStatus::AuthRequired);

    // 无 Coding Plan（2026-10-12 双站实测原文）：code 500 + “不存在coding plan”
    // → 独立状态 no_plan，不是通用失败。
    let error = parse_response(
        GlmEndpoint::QuotaLimit,
        200,
        &read_bytes("glm_plan/no-plan/input/quota_limit.json"),
    )
    .unwrap_err();
    match &error {
        GlmError::NoPlan { code, message } => {
            assert_eq!(code, "500");
            assert!(message.contains("不存在coding plan"));
        }
        other => panic!("expected no-plan, got {other:?}"),
    }
    assert_eq!(error.status(), SnapshotStatus::NoPlan);
    let signal = error.failure_signal();
    assert_eq!(signal.status_hint, Some(SnapshotStatus::NoPlan));
    assert_eq!(signal.business_code.as_deref(), Some("500"));
    assert!(signal.is_failure());
    // success:false 且无业务码 → 仍是 success_false 通用失败（既有行为不变）。
    let error = parse_response(
        GlmEndpoint::QuotaLimit,
        200,
        br#"{"success":false,"msg":"unknown failure"}"#,
    )
    .unwrap_err();
    match &error {
        GlmError::Business { code, .. } => assert_eq!(code, "success_false"),
        other => panic!("expected success_false, got {other:?}"),
    }
    assert_eq!(error.status(), SnapshotStatus::Unavailable);

    // 传输层 401/429/5xx。
    for (status, want) in [
        (401u16, SnapshotStatus::AuthRequired),
        (403, SnapshotStatus::AuthRequired),
        (429, SnapshotStatus::RateLimited),
        (503, SnapshotStatus::Unavailable),
    ] {
        let error = parse_response(GlmEndpoint::QuotaLimit, status, b"{}").unwrap_err();
        assert_eq!(error.status(), want, "HTTP {status}");
        assert_eq!(error.failure_signal().status(), want);
    }

    // schema 变化：TOKENS_LIMIT 缺 percentage → incompatible，不补零。
    let error = parse_response(
        GlmEndpoint::QuotaLimit,
        200,
        br#"{"data":{"limits":[{"type":"TOKENS_LIMIT"}]}}"#,
    )
    .unwrap_err();
    assert!(matches!(error, GlmError::SchemaChanged(_)));
    assert_eq!(error.status(), SnapshotStatus::Incompatible);

    // 缺 limits 也是 schema 变化。
    let error = parse_response(GlmEndpoint::QuotaLimit, 200, br#"{"data":{}}"#).unwrap_err();
    assert_eq!(error.status(), SnapshotStatus::Incompatible);
}

#[test]
fn glm_detail_endpoints_build_but_do_not_fabricate() {
    use glm_plan::{parse_response, GlmEndpoint};

    // model/tool usage 明细暂无 schema 依据：请求能构造，解析只记诊断不回填数据。
    for endpoint in [GlmEndpoint::ModelUsage, GlmEndpoint::ToolUsage] {
        let report = parse_response(endpoint, 200, br#"{"data":{"anything":1}}"#).unwrap();
        assert!(report.windows.is_empty());
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.contains("detail_stats_parse_not_in_scope")));
    }
}

// ---------------------------------------------------------------------------
// B06 Kimi 本机 Server
// ---------------------------------------------------------------------------

#[test]
fn kimi_loopback_request_shape() {
    use kimi_server::{build_request, KimiEndpoint, KimiError, DEFAULT_BASE};

    let spec = build_request(DEFAULT_BASE, KimiEndpoint::OauthUsage).unwrap();
    assert_eq!(spec.method, "GET");
    assert_eq!(spec.scheme, "http");
    assert_eq!(spec.host, "127.0.0.1:58627");
    assert_eq!(spec.url(), "http://127.0.0.1:58627/api/v1/oauth/usage");
    assert_eq!(spec.auth, Some(common::AuthPlacement::bearer()));
    // 不装秘密。
    assert!(spec
        .headers
        .iter()
        .all(|(k, _)| !k.eq_ignore_ascii_case("authorization")));

    // localhost / ::1 / 127.x 接受；端口可换。
    assert!(build_request("http://localhost:1234", KimiEndpoint::Meta).is_ok());
    assert_eq!(
        build_request("http://[::1]:58627", KimiEndpoint::Meta)
            .unwrap()
            .url(),
        "http://[::1]:58627/api/v1/meta"
    );
    // 非 loopback host 拒绝（绝不把本机令牌发往别处）。
    assert!(matches!(
        build_request("http://evil.example.com:58627", KimiEndpoint::Meta),
        Err(KimiError::NonLoopbackHost(_))
    ));
    // 非 http 也拒绝。
    assert!(matches!(
        build_request("https://127.0.0.1:58627", KimiEndpoint::Meta),
        Err(KimiError::InvalidBase(_))
    ));
}

#[test]
fn kimi_meta_and_oauth_usage_fixtures() {
    use kimi_server::{parse_meta, parse_oauth_usage};

    let meta = parse_meta(200, &read_bytes("kimi_server/meta/input/meta.json")).unwrap();
    assert_contains(
        &serde_json::to_value(&meta).unwrap(),
        &read_json("kimi_server/meta/expected.json"),
        "kimi.meta",
    );

    let report = parse_oauth_usage(
        200,
        &read_bytes("kimi_server/oauth-usage/input/oauth_usage.json"),
    )
    .unwrap();
    assert_eq!(report.windows.len(), 4);
    assert_eq!(report.balances.len(), 2);
    assert_contains(
        &serde_json::to_value(&report).unwrap(),
        &read_json("kimi_server/oauth-usage/expected.json"),
        "kimi.usage",
    );

    // 缺窗口 + 空钱包：只回存在的窗口，无余额行。
    let report = parse_oauth_usage(
        200,
        &read_bytes("kimi_server/oauth-usage-null-extra/input/oauth_usage.json"),
    )
    .unwrap();
    assert_contains(
        &serde_json::to_value(&report).unwrap(),
        &read_json("kimi_server/oauth-usage-null-extra/expected.json"),
        "kimi.null_extra",
    );
}

#[test]
fn kimi_error_mapping() {
    use kimi_server::{parse_meta, parse_oauth_usage, KimiError};

    // 上游 in-band 失败（信封 code=0）：unavailable，而不是 fake 数据。
    let error = parse_oauth_usage(
        200,
        &read_bytes("kimi_server/oauth-usage-inband-error/input/oauth_usage.json"),
    )
    .unwrap_err();
    match &error {
        KimiError::UpstreamError { status, .. } => assert_eq!(*status, Some(401)),
        other => panic!("expected upstream error, got {other:?}"),
    }
    assert_eq!(error.status(), SnapshotStatus::Unavailable);

    // HTTP200 + 业务码 40101 → auth_required。
    let error = parse_meta(
        200,
        &read_bytes("kimi_server/business-error/input/meta.json"),
    )
    .unwrap_err();
    match &error {
        KimiError::Business { code, .. } => assert_eq!(code, "40101"),
        other => panic!("expected business error, got {other:?}"),
    }
    assert_eq!(error.status(), SnapshotStatus::AuthRequired);
    assert!(error.failure_signal().is_failure());

    // 传输层 401 / 429。
    assert_eq!(
        parse_meta(401, b"{}").unwrap_err().status(),
        SnapshotStatus::AuthRequired
    );
    assert_eq!(
        parse_meta(429, b"{}").unwrap_err().status(),
        SnapshotStatus::RateLimited
    );

    // 信封缺 code → incompatible。
    let error = parse_meta(200, br#"{"msg":"x","data":{}}"#).unwrap_err();
    assert!(matches!(error, KimiError::SchemaChanged(_)));
    assert_eq!(error.status(), SnapshotStatus::Incompatible);

    // 未知 kind → incompatible。
    let error = parse_oauth_usage(200, br#"{"code":0,"data":{"kind":"mystery"}}"#).unwrap_err();
    assert_eq!(error.status(), SnapshotStatus::Incompatible);
}

// ---------------------------------------------------------------------------
// B-04 Kimi Code Coding Key 直连通道
// ---------------------------------------------------------------------------

#[test]
fn kimi_code_request_shape() {
    use kimi_code::{build_request, KimiCodeError, DEFAULT_BASE, GLOBAL_BASE};

    let spec = build_request(DEFAULT_BASE).unwrap();
    assert_eq!(spec.method, "GET");
    assert_eq!(spec.scheme, "https");
    assert_eq!(spec.host, "api.kimi.com");
    assert_eq!(spec.path, "/coding/v1/usages");
    assert_eq!(spec.url(), "https://api.kimi.com/coding/v1/usages");
    assert_eq!(spec.auth, Some(common::AuthPlacement::bearer()));
    assert!(spec
        .headers
        .iter()
        .any(|(k, v)| k == "Accept" && v == "application/json"));
    // 不装秘密：没有任何 Authorization 头（凭据由服务层注入）。
    assert!(spec
        .headers
        .iter()
        .all(|(k, _)| !k.eq_ignore_ascii_case("authorization")));

    // 官方 CLI 的国际站 base 同样接受；尾斜杠归一。
    assert_eq!(
        build_request(&format!("{GLOBAL_BASE}/")).unwrap().url(),
        "https://api.kimi.ai/coding/v1/usages"
    );

    // host 精确匹配：子串相似的域名不是 Kimi Code。
    assert!(matches!(
        build_request("https://api.kimi.com.evil.com/coding/v1"),
        Err(KimiCodeError::UnrecognizedHost(_))
    ));
    assert!(matches!(
        build_request("https://evil-api.kimi.com/coding/v1"),
        Err(KimiCodeError::UnrecognizedHost(_))
    ));
    // 外网 http 与显式端口拒绝；缺 scheme 拒绝。
    assert!(matches!(
        build_request("http://api.kimi.com/coding/v1"),
        Err(KimiCodeError::InvalidBase(_))
    ));
    assert!(matches!(
        build_request("https://api.kimi.com:8443/coding/v1"),
        Err(KimiCodeError::InvalidBase(_))
    ));
    assert!(matches!(
        build_request("api.kimi.com/coding/v1"),
        Err(KimiCodeError::InvalidBase(_))
    ));
    // userinfo 拒绝。
    assert!(matches!(
        build_request("https://user@api.kimi.com/coding/v1"),
        Err(KimiCodeError::InvalidBase(_))
    ));
}

#[test]
fn kimi_code_usages_fixtures() {
    use kimi_code::parse_usages;

    // 完整形态（4 窗口 + booster_wallet/usage/limits 未解析段）。
    let report = parse_usages(200, &read_bytes("kimi_code/usages/input/usages.json")).unwrap();
    assert_eq!(report.windows.len(), 4);
    assert_contains(
        &serde_json::to_value(&report).unwrap(),
        &read_json("kimi_code/usages/expected.json"),
        "kimi_code.usages",
    );
    // 窗口身份与 B06 本机 Server 通道一致（两通道互为上游，快照窗口连续）。
    assert_eq!(report.windows[0].window_id, "kimi:coding_plan:account:5h");
    assert_eq!(report.windows[1].window_id, "kimi:coding_plan:account:7d");

    // 无月窗 + 字符串比例 + 小数秒 reset_time（对照 2026-10-12 实测账户形态）。
    let report = parse_usages(
        200,
        &read_bytes("kimi_code/usages-minimal/input/usages.json"),
    )
    .unwrap();
    assert_eq!(report.windows.len(), 2);
    assert_contains(
        &serde_json::to_value(&report).unwrap(),
        &read_json("kimi_code/usages-minimal/expected.json"),
        "kimi_code.usages_minimal",
    );
}

#[test]
fn kimi_code_error_mapping() {
    use kimi_code::{parse_usages, KimiCodeError};

    // HTTP 401 + {"error":{"type":"invalid_authentication_error"}} → auth_required，
    // 稳定机器码 type 记入 business_code（2026-10-12 实测形态）。
    let error = parse_usages(
        401,
        &read_bytes("kimi_code/unauthorized-401/input/usages.json"),
    )
    .unwrap_err();
    match &error {
        KimiCodeError::Http { status, code } => {
            assert_eq!(*status, 401);
            assert_eq!(code.as_deref(), Some("invalid_authentication_error"));
        }
        other => panic!("expected http 401, got {other:?}"),
    }
    assert_eq!(error.status(), SnapshotStatus::AuthRequired);
    let signal = error.failure_signal();
    assert_eq!(signal.http_status, Some(401));
    assert_eq!(
        signal.business_code.as_deref(),
        Some("invalid_authentication_error")
    );

    for (status, want) in [
        (403u16, SnapshotStatus::AuthRequired),
        (429, SnapshotStatus::RateLimited),
        (500, SnapshotStatus::Unavailable),
    ] {
        let error = parse_usages(status, b"{}").unwrap_err();
        assert_eq!(error.status(), want, "HTTP {status}");
    }

    // 缺 usages → schema 变化（不补零、不冒充成功空数据）。
    let error = parse_usages(200, br#"{"usage":{"limit":"100"}}"#).unwrap_err();
    assert!(matches!(error, KimiCodeError::SchemaChanged(_)));
    assert_eq!(error.status(), SnapshotStatus::Incompatible);

    // 窗口条目存在但缺 used_ratio → schema 变化。
    let error = parse_usages(200, br#"{"usages":{"limit_5h":{}}}"#).unwrap_err();
    assert!(matches!(error, KimiCodeError::SchemaChanged(_)));

    // 窗口不是对象 → 记诊断并跳过；未知 usages 键 → unknown_usage_key 诊断。
    let report = parse_usages(
        200,
        br#"{"usages":{"limit_5h":"oops","limit_30d":{"used_ratio":0.5}}}"#,
    )
    .unwrap();
    assert!(report.windows.is_empty());
    assert!(report
        .diagnostics
        .iter()
        .any(|d| d == "limit_5h_not_object"));
    assert!(report
        .diagnostics
        .iter()
        .any(|d| d == "unknown_usage_key:limit_30d"));

    // 超 100% 封顶：used=1、remaining=0、原值进 raw_ratio。
    let report = parse_usages(200, br#"{"usages":{"limit_5h":{"used_ratio":1.2}}}"#).unwrap();
    assert_eq!(report.windows[0].used_ratio.as_deref(), Some("1"));
    assert_eq!(report.windows[0].remaining_ratio.as_deref(), Some("0"));
    assert_eq!(report.windows[0].raw_ratio.as_deref(), Some("1.2"));
    assert_eq!(report.windows[0].over_limit, Some(true));

    // 非法 reset_time：记诊断、保持未知（不猜时间）。
    let report = parse_usages(
        200,
        br#"{"usages":{"limit_5h":{"used_ratio":0.1,"reset_time":"next week"}}}"#,
    )
    .unwrap();
    assert_eq!(report.windows[0].resets_at, None);
    assert!(report
        .diagnostics
        .iter()
        .any(|d| d == "limit_5h.reset_time_not_rfc3339"));
}

// ---------------------------------------------------------------------------
// B07 Codex App Server
// ---------------------------------------------------------------------------

#[test]
fn codex_request_frames() {
    use codex_appserver::{
        initialize, rate_limits_read, usage_read, Transport, INITIALIZE_METHOD, RATE_LIMITS_METHOD,
        USAGE_METHOD,
    };

    let spec = initialize(1, "providock", Some("Providock"), "0.1.0", true);
    assert_eq!(spec.method, INITIALIZE_METHOD);
    assert_eq!(spec.transport, Transport::Stdio);
    assert_eq!(spec.auth, common::AuthPlacement::server_held());
    let wire = spec.to_wire();
    assert_eq!(wire["id"], 1);
    assert_eq!(wire["method"], "initialize");
    assert_eq!(wire["params"]["clientInfo"]["name"], "providock");
    assert_eq!(wire["params"]["clientInfo"]["title"], "Providock");
    assert_eq!(wire["params"]["clientInfo"]["version"], "0.1.0");
    assert_eq!(wire["params"]["capabilities"]["experimentalApi"], true);
    let text = wire.to_string().to_ascii_lowercase();
    assert!(!text.contains("authorization"));
    assert!(!text.contains("bearer"));

    let spec = rate_limits_read(2, true, false);
    assert_eq!(spec.method, RATE_LIMITS_METHOD);
    assert_eq!(spec.to_wire()["params"]["supportsLunaReserve"], true);
    assert!(spec.to_wire()["params"]
        .get("excludeResetCreditDetails")
        .is_none());

    let spec = usage_read(3, Some("019fc8ab-1fb2-7000-8000-000000000123"));
    assert_eq!(spec.method, USAGE_METHOD);
    assert_eq!(
        spec.to_wire()["params"]["threadId"],
        "019fc8ab-1fb2-7000-8000-000000000123"
    );
    // 无 threadId：空参数对象（账户级用量）。
    assert_eq!(
        usage_read(4, None).to_wire()["params"],
        serde_json::json!({})
    );
}

#[test]
fn codex_rate_limits_fixture() {
    use codex_appserver::parse_rate_limits;

    let report = parse_rate_limits(&read_bytes(
        "codex_appserver/rate-limits/input/response.json",
    ))
    .unwrap();
    assert_eq!(report.windows.len(), 2);
    assert_contains(
        &serde_json::to_value(&report).unwrap(),
        &read_json("codex_appserver/rate-limits/expected.json"),
        "codex.rate_limits",
    );
}

#[test]
fn codex_usage_fixtures() {
    use codex_appserver::parse_usage;

    let usage = parse_usage(&read_bytes("codex_appserver/usage/input/response.json")).unwrap();
    assert!(usage.thread_usage.is_none());
    assert_contains(
        &serde_json::to_value(&usage).unwrap(),
        &read_json("codex_appserver/usage/expected.json"),
        "codex.usage",
    );

    // summary 全 null：不产出指标（没有就省）；thread usage 透出。
    let usage = parse_usage(&read_bytes(
        "codex_appserver/usage-thread/input/response.json",
    ))
    .unwrap();
    assert!(usage.metrics.is_empty());
    assert!(usage.thread_usage.is_some());
    assert_contains(
        &serde_json::to_value(&usage).unwrap(),
        &read_json("codex_appserver/usage-thread/expected.json"),
        "codex.usage_thread",
    );
}

#[test]
fn codex_initialize_fixture() {
    use codex_appserver::parse_initialize;

    let info = parse_initialize(&read_bytes(
        "codex_appserver/initialize/input/response.json",
    ))
    .unwrap();
    assert_contains(
        &serde_json::to_value(&info).unwrap(),
        &read_json("codex_appserver/initialize/expected.json"),
        "codex.initialize",
    );
    // 协商结果写入 evidence（无数字版本号，以 userAgent 为准）。
    assert!(info.evidence.contains("user_agent=codex_cli_rs/0.50.0"));
}

#[test]
fn codex_error_mapping() {
    use codex_appserver::{parse_rate_limits, parse_usage, CodexError, RpcOutcome};

    // method not found → unsupported（能力缺失）。
    let error = parse_usage(&read_bytes(
        "codex_appserver/error-method-not-found/input/response.json",
    ))
    .unwrap_err();
    assert!(matches!(error, CodexError::Unsupported(_)));
    assert_eq!(error.status(), SnapshotStatus::Unsupported);
    assert!(error.failure_signal().is_failure());

    // chatgpt 认证要求（API-key-only / Bedrock 不支持 usage）→ unsupported。
    let error = parse_rate_limits(&read_bytes(
        "codex_appserver/error-chatgpt-required/input/response.json",
    ))
    .unwrap_err();
    assert!(matches!(error, CodexError::Unsupported(_)));
    assert_eq!(error.status(), SnapshotStatus::Unsupported);

    // 完全未登录 → auth_required。
    let error = parse_rate_limits(&read_bytes(
        "codex_appserver/error-auth-required/input/response.json",
    ))
    .unwrap_err();
    assert!(matches!(error, CodexError::AuthRequired(_)));
    assert_eq!(error.status(), SnapshotStatus::AuthRequired);

    // 解析器把信封拆成 Ok/Error，不误判。
    let outcome = codex_appserver::parse_response(&read_bytes(
        "codex_appserver/error-auth-required/input/response.json",
    ))
    .unwrap();
    assert!(matches!(outcome, RpcOutcome::Error(_)));

    // 非 JSON / 缺 result 与 error → 协议错误 → incompatible。
    let error = parse_usage(b"not json").unwrap_err();
    assert_eq!(error.status(), SnapshotStatus::Incompatible);
    let error = parse_usage(br#"{"id":1}"#).unwrap_err();
    assert!(matches!(error, CodexError::Protocol(_)));
}

// ---------------------------------------------------------------------------
// 结构性守卫
// ---------------------------------------------------------------------------

/// 适配器纯函数：不得引用 HTTP 客户端（凭据注入与网络在服务层）。
#[test]
fn account_adapters_do_not_reference_http_clients() {
    for (name, source) in [
        (
            "glm_plan",
            include_str!("../src/adapters/accounts/glm_plan.rs"),
        ),
        (
            "kimi_server",
            include_str!("../src/adapters/accounts/kimi_server.rs"),
        ),
        (
            "kimi_code",
            include_str!("../src/adapters/accounts/kimi_code.rs"),
        ),
        (
            "codex_appserver",
            include_str!("../src/adapters/accounts/codex_appserver.rs"),
        ),
    ] {
        assert!(
            !source.contains("reqwest"),
            "{name} 适配器不得引用 reqwest（不触网）"
        );
    }
}

/// 请求规格不含秘密：Authorization 只能由服务层按 scheme 注入。
#[test]
fn request_specs_carry_no_credentials() {
    use glm_plan::{build_request as glm_build, GlmEndpoint};
    use kimi_server::{build_request as kimi_build, KimiEndpoint};

    let glm = glm_build(
        "https://api.z.ai/api/anthropic",
        GlmEndpoint::QuotaLimit,
        None,
    )
    .unwrap();
    assert!(glm
        .headers
        .iter()
        .all(|(k, _)| !k.eq_ignore_ascii_case("authorization")));
    assert_eq!(glm.auth, Some(common::AuthPlacement::raw_token()));

    let kimi = kimi_build(kimi_server::DEFAULT_BASE, KimiEndpoint::Meta).unwrap();
    assert!(kimi
        .headers
        .iter()
        .all(|(k, _)| !k.eq_ignore_ascii_case("authorization")));
    assert_eq!(kimi.auth, Some(common::AuthPlacement::bearer()));

    let kimi_code = kimi_code::build_request(kimi_code::DEFAULT_BASE).unwrap();
    assert!(kimi_code
        .headers
        .iter()
        .all(|(k, _)| !k.eq_ignore_ascii_case("authorization")));
    assert_eq!(kimi_code.auth, Some(common::AuthPlacement::bearer()));
}
