//! B08 explicit New API / One API instances. Only documented read-only endpoints;
//! token budgets are not account cash, and raw vendor quota has no assumed currency.
use super::common::{self, AuthPlacement, ParsedAccountResponse, RequestSpec};
use crate::domain::Quality;
use crate::services::accounts::{
    BalanceEntry, BalanceKind, CredentialType, FailureSignal, QuotaUnit, QuotaWindow,
    SnapshotStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelayConfig {
    pub base_url: String,
    pub user_id: Option<u64>,
    pub currency: Option<String>,
    pub quota_per_unit: Option<u64>,
}
impl RelayConfig {
    pub fn validate(&self, platform: &str, credential: CredentialType) -> Result<(), String> {
        let url = reqwest::Url::parse(&self.base_url).map_err(|_| "请输入中转站 HTTPS 根地址")?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port_or_known_default() != Some(443)
            || !url.path().trim_matches('/').is_empty()
        {
            return Err("中转站须使用 HTTPS 根地址，不含 /v1、登录信息、参数或自定义端口".into());
        }
        if !["new_api", "one_api"].contains(&platform)
            || ![CredentialType::ApiKey, CredentialType::ManagementKey].contains(&credential)
        {
            return Err("中转站协议或凭据类型不支持".into());
        }
        if platform == "one_api" && credential != CredentialType::ManagementKey {
            return Err("One API 账户余额需要个人访问令牌".into());
        }
        if credential == CredentialType::ManagementKey && self.user_id.is_none_or(|id| id == 0) {
            return Err("个人访问令牌查询需要用户 ID".into());
        }
        if self.currency.is_some() != self.quota_per_unit.is_some()
            || self
                .currency
                .as_deref()
                .is_some_and(|c| !["CNY", "USD"].contains(&c))
            || self
                .quota_per_unit
                .is_some_and(|q| q == 0 || q > 1_000_000_000_000)
        {
            return Err("货币换算需同时填写 CNY/USD 与每货币单位额度（1–1000000000000）".into());
        }
        Ok(())
    }
}
pub fn build_request(
    config: &RelayConfig,
    platform: &str,
    credential: CredentialType,
) -> Result<RequestSpec, String> {
    config.validate(platform, credential)?;
    let url = reqwest::Url::parse(&config.base_url).map_err(|_| "站点地址无效")?;
    let path = if credential == CredentialType::ManagementKey {
        "/api/user/self"
    } else {
        "/api/usage/token"
    };
    let mut request = RequestSpec::https(url.host_str().ok_or("站点地址无效")?, path)
        .with_auth(AuthPlacement::bearer());
    if credential == CredentialType::ManagementKey && platform == "new_api" {
        request.headers.push((
            "New-Api-User".into(),
            config.user_id.ok_or("需要用户 ID")?.to_string(),
        ));
    }
    Ok(request)
}
fn fail(status: u16, schema: bool, business: bool) -> ParsedAccountResponse {
    ParsedAccountResponse::Failure(FailureSignal {
        http_status: Some(status),
        schema_changed: schema,
        business_code: business.then(|| "relay_business_error".into()),
        status_hint: business.then_some(SnapshotStatus::AuthRequired),
        ..Default::default()
    })
}
fn amount(v: &Value, divisor: Option<u64>) -> Option<String> {
    let raw = common::decimal_field(v)?;
    let q = raw.parse::<i128>().ok()?;
    let Some(divisor) = divisor else {
        return Some(q.to_string());
    };
    let scaled = q
        .checked_mul(1_000_000_000)?
        .checked_div(i128::from(divisor))?;
    if q != 0 && scaled == 0 {
        return None;
    }
    let sign = if scaled < 0 { "-" } else { "" };
    let v = scaled.unsigned_abs();
    let s = format!("{sign}{}.{:09}", v / 1_000_000_000, v % 1_000_000_000);
    Some(s.trim_end_matches('0').trim_end_matches('.').into())
}
pub fn parse_response(
    config: &RelayConfig,
    credential: CredentialType,
    status: u16,
    body: &str,
) -> ParsedAccountResponse {
    if status != 200 {
        return fail(status, false, false);
    }
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return fail(status, true, false);
    };
    if v.get("success") == Some(&Value::Bool(false))
        || v.get("code") == Some(&Value::Bool(false))
        || v.get("error").is_some()
    {
        return fail(status, false, true);
    }
    let Some(data) = v.get("data").filter(|d| d.is_object()) else {
        return fail(status, true, false);
    };
    let management = credential == CredentialType::ManagementKey;
    if management
        && (v.get("success") != Some(&Value::Bool(true))
            || data.get("id").and_then(Value::as_u64) != config.user_id)
    {
        return fail(status, true, false);
    }
    if !management && data.get("object").and_then(Value::as_str) != Some("token_usage") {
        return fail(status, true, false);
    }
    let unlimited = if management {
        false
    } else {
        let Some(u) = data.get("unlimited_quota").and_then(Value::as_bool) else {
            return fail(status, true, false);
        };
        u
    };
    let (remain, used) = if management {
        ("quota", "used_quota")
    } else {
        ("total_available", "total_used")
    };
    let Some(used) = data
        .get(used)
        .and_then(|v| amount(v, config.quota_per_unit))
    else {
        return fail(status, true, false);
    };
    let remaining = if unlimited {
        None
    } else {
        let Some(r) = data
            .get(remain)
            .and_then(|v| amount(v, config.quota_per_unit))
        else {
            return fail(status, true, false);
        };
        Some(r)
    };
    let scope = if management { "account" } else { "key" };
    let mut window = QuotaWindow::new(
        format!("relay:{scope}:quota"),
        if config.currency.is_some() {
            QuotaUnit::Currency
        } else {
            QuotaUnit::VendorUnit
        },
    );
    window.label = Some(format!(
        "{}额度{}{}",
        if management { "账户" } else { "Key" },
        if unlimited { "（无限额）" } else { "" },
        config
            .currency
            .as_ref()
            .map(|c| format!(" ({c})"))
            .unwrap_or_default()
    ));
    window.scope = Some(scope.into());
    window.used = Some(used);
    window.remaining = remaining.clone();
    window.quality = if config.currency.is_some() {
        Quality::Derived
    } else {
        Quality::Reported
    };
    if !unlimited {
        window.limit = window
            .used
            .as_deref()
            .zip(remaining.as_deref())
            .and_then(|(u, r)| crate::domain::decimal::decimal_add(u, r));
    }
    window.evidence_ref=Some("relay-v1:quota;currency conversion explicitly configured by user;not physical model tokens".into());
    let mut balances = vec![];
    if management {
        if let (Some(cur), Some(remain)) = (&config.currency, remaining) {
            let Ok(mut b) = BalanceEntry::new(BalanceKind::Available, remain, cur) else {
                return fail(status, true, false);
            };
            b.wallet_scope = Some(format!(
                "{}:account:{}",
                config.base_url.trim_end_matches('/'),
                config.user_id.unwrap_or(0)
            ));
            b.quality = Quality::Derived;
            b.formula = Some(format!(
                "quota / {} (user configured)",
                config.quota_per_unit.unwrap_or(1)
            ));
            balances.push(b);
        }
    }
    let mut windows = vec![window];
    // New API only exposes owner quota when the Key explicitly allows it.
    if !management {
        if let Some(owner) = data.get("user_quota") {
            let Some(remaining) = owner
                .get("total_available")
                .and_then(|v| amount(v, config.quota_per_unit))
            else {
                return fail(status, true, false);
            };
            let Some(used) = owner
                .get("total_used")
                .and_then(|v| amount(v, config.quota_per_unit))
            else {
                return fail(status, true, false);
            };
            let mut w = QuotaWindow::new(
                "relay:account:quota",
                if config.currency.is_some() {
                    QuotaUnit::Currency
                } else {
                    QuotaUnit::VendorUnit
                },
            );
            w.label = Some(format!(
                "账户额度（Key 已授权查询）{}",
                config
                    .currency
                    .as_ref()
                    .map(|c| format!(" ({c})"))
                    .unwrap_or_default()
            ));
            w.scope = Some("account".into());
            w.remaining = Some(remaining.clone());
            w.used = Some(used.clone());
            w.limit = crate::domain::decimal::decimal_add(&remaining, &used);
            w.quality = if config.currency.is_some() {
                Quality::Derived
            } else {
                Quality::Reported
            };
            windows.push(w);
            if let Some(cur) = &config.currency {
                let Ok(mut b) = BalanceEntry::new(BalanceKind::Available, remaining, cur) else {
                    return fail(status, true, false);
                };
                b.wallet_scope = Some(format!(
                    "{}:account:identity_unknown",
                    config.base_url.trim_end_matches('/')
                ));
                b.quality = Quality::Derived;
                b.formula = Some(format!(
                    "user_quota.total_available / {} (user configured)",
                    config.quota_per_unit.unwrap_or(1)
                ));
                balances.push(b);
            }
        }
    }
    ParsedAccountResponse::success(balances, windows)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> RelayConfig {
        RelayConfig {
            base_url: "https://relay.example".into(),
            user_id: Some(42),
            currency: Some("USD".into()),
            quota_per_unit: Some(500000),
        }
    }
    #[test]
    fn requests_and_units() {
        let c = config();
        let s = build_request(&c, "new_api", CredentialType::ManagementKey).unwrap();
        assert_eq!(s.path, "/api/user/self");
        assert_eq!(s.headers, vec![("New-Api-User".into(), "42".into())]);
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../../fixtures/accounts/relay-v1/input.json"
        ))
        .unwrap();
        let ParsedAccountResponse::Success {
            balances,
            quota_windows,
            ..
        } = parse_response(
            &c,
            CredentialType::ManagementKey,
            200,
            &fixture["account"].to_string(),
        )
        else {
            panic!()
        };
        assert_eq!(balances[0].amount, "1.97531");
        assert_eq!(quota_windows[0].used.as_deref(), Some("0.02469"));
        assert_eq!(quota_windows[0].limit.as_deref(), Some("2"));
        let ParsedAccountResponse::Success {
            balances,
            quota_windows,
            ..
        } = parse_response(
            &c,
            CredentialType::ApiKey,
            200,
            &fixture["token"].to_string(),
        )
        else {
            panic!()
        };
        assert!(balances.is_empty());
        assert_eq!(quota_windows[0].scope.as_deref(), Some("key"));
        let ParsedAccountResponse::Success {
            balances,
            quota_windows,
            ..
        } = parse_response(
            &c,
            CredentialType::ApiKey,
            200,
            &fixture["owner_authorized"].to_string(),
        )
        else {
            panic!()
        };
        assert_eq!(balances[0].amount, "5");
        assert_eq!(quota_windows.len(), 2);
        assert_eq!(quota_windows[1].scope.as_deref(), Some("account"));
        let mut raw = c.clone();
        raw.currency = None;
        raw.quota_per_unit = None;
        let ParsedAccountResponse::Success { quota_windows, .. } = parse_response(
            &raw,
            CredentialType::ApiKey,
            200,
            &fixture["unlimited"].to_string(),
        ) else {
            panic!()
        };
        assert_eq!(quota_windows[0].used.as_deref(), Some("12345"));
        assert_eq!(quota_windows[0].remaining, None);
        assert_eq!(quota_windows[0].limit, None);
    }
    #[test]
    fn failure_and_url_guards() {
        let c = config();
        for (status, body) in [
            (200, "{}"),
            (
                200,
                r#"{"success":false,"message":"synthetic secret must not escape"}"#,
            ),
            (401, "{}"),
            (429, "{}"),
        ] {
            assert!(!parse_response(&c, CredentialType::ApiKey, status, body).is_success());
        }
        for url in [
            "http://relay.example",
            "https://key@relay.example",
            "https://relay.example/v1",
            "https://relay.example?key=secret",
            "https://relay.example:8443",
        ] {
            let mut c = c.clone();
            c.base_url = url.into();
            assert!(c.validate("new_api", CredentialType::ApiKey).is_err());
        }
        assert_eq!(
            amount(&serde_json::json!(-500000), Some(500000)).as_deref(),
            Some("-1")
        );
    }
}
