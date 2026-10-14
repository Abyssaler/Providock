//! 账户直连通道实机验证（默认不运行）：用环境变量里的真实凭据走一遍
//! 「适配器构造请求 → ReqwestTransport 真实 HTTPS 发送 → 适配器解析」完整链路。
//!
//! 运行：
//! ```sh
//! PROVIDOCK_LIVE_KIMI_KEY=sk-... PROVIDOCK_LIVE_GLM_KEY=... \
//!   cargo test --test live_accounts -- --ignored --nocapture
//! ```
//!
//! 纪律：密钥只从环境变量读取，绝不打印、不写库、不落任何文件；未设置时跳过。
//! 可选 base 覆盖：`PROVIDOCK_LIVE_KIMI_BASE_URL` / `PROVIDOCK_LIVE_GLM_BASE_URL`。

use providock_lib::adapters::accounts::{common::RequestSpec, glm_plan, kimi_code};
use providock_lib::services::account_query::{
    HttpRequest, HttpTransport, ReqwestTransport, MAX_BODY_BYTES, TOTAL_TIMEOUT_SECONDS,
};
use providock_lib::services::accounts::SnapshotStatus;

fn env_key(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// 与服务层一致的注入方式：秘密只在构造请求头的瞬间使用，随后 drop。
fn http_request(spec: &RequestSpec, secret: &str) -> HttpRequest {
    let auth_header = spec
        .auth
        .as_ref()
        .map(|auth| (auth.header.clone(), auth.header_value(secret)));
    HttpRequest {
        follow_redirects: true,
        method: spec.method.clone(),
        url: spec.url(),
        headers: spec.headers.clone(),
        auth_header,
        timeout_seconds: TOTAL_TIMEOUT_SECONDS,
        max_body_bytes: MAX_BODY_BYTES,
    }
}

#[test]
#[ignore]
fn live_kimi_code_direct_usages() {
    let Some(key) = env_key("PROVIDOCK_LIVE_KIMI_KEY") else {
        eprintln!("live kimi: PROVIDOCK_LIVE_KIMI_KEY 未设置，跳过");
        return;
    };
    let base = env_key("PROVIDOCK_LIVE_KIMI_BASE_URL")
        .unwrap_or_else(|| kimi_code::DEFAULT_BASE.to_owned());
    let spec = kimi_code::build_request(&base).expect("kimi_code build_request");
    let response = ReqwestTransport::new()
        .send(&http_request(&spec, &key))
        .expect("kimi_code send");
    drop(key);

    let report =
        kimi_code::parse_usages(response.status, &response.body).expect("kimi_code parse_usages");
    let ids: Vec<&str> = report
        .windows
        .iter()
        .map(|window| window.window_id.as_str())
        .collect();
    assert!(
        ids.contains(&"kimi:coding_plan:account:5h"),
        "应含 5h 窗口，实际 {ids:?}"
    );
    assert!(
        ids.contains(&"kimi:coding_plan:account:7d"),
        "应含 7d 窗口，实际 {ids:?}"
    );
    for window in &report.windows {
        assert!(
            window.used_ratio.is_some(),
            "{} 缺 used_ratio",
            window.window_id
        );
    }
    eprintln!(
        "live kimi: host={} windows={:?} diagnostics={:?}",
        spec.host, ids, report.diagnostics
    );
}

#[test]
#[ignore]
fn live_glm_no_plan_classification() {
    let Some(key) = env_key("PROVIDOCK_LIVE_GLM_KEY") else {
        eprintln!("live glm: PROVIDOCK_LIVE_GLM_KEY 未设置，跳过");
        return;
    };
    let base = env_key("PROVIDOCK_LIVE_GLM_BASE_URL")
        .unwrap_or_else(|| "https://open.bigmodel.cn".to_owned());
    let spec = glm_plan::build_request(&base, glm_plan::GlmEndpoint::QuotaLimit, None)
        .expect("glm build_request");
    let response = ReqwestTransport::new()
        .send(&http_request(&spec, &key))
        .expect("glm send");
    drop(key);

    // 前提：该凭据是无 Coding Plan 的按量账户（2026-10-12 双站实测形态）。
    // 期望分类为 no_plan；若账户后来购买了套餐，此断言应随之更新而不是放松。
    match glm_plan::parse_response(
        glm_plan::GlmEndpoint::QuotaLimit,
        response.status,
        &response.body,
    ) {
        Err(error @ glm_plan::GlmError::NoPlan { .. }) => {
            assert_eq!(error.status(), SnapshotStatus::NoPlan);
            assert_eq!(
                error.failure_signal().status_hint,
                Some(SnapshotStatus::NoPlan)
            );
            eprintln!(
                "live glm: host={} → no_plan（符合无套餐账户预期）",
                spec.host
            );
        }
        other => panic!("live glm: 期望 no_plan 分类，实际 {other:?}"),
    }
}
