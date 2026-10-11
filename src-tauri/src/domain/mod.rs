//! 标准契约类型（docs/specs/DATA_CONTRACTS.md）。
//!
//! 关键口径：
//! - Token 为非负整数；IPC/JSON 一律十进制字符串，避免超过 2^53 失真。
//! - `None` 表示源未提供/不适用；0 只表示明确的零。
//! - 金额不使用二进制浮点。

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

pub mod decimal;

pub mod serde_u64_str {
    use super::*;

    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = u64;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("decimal string or non-negative integer")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<u64, E> {
                v.parse::<u64>().map_err(E::custom)
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<u64, E> {
                Ok(v)
            }
        }
        d.deserialize_any(V)
    }
}

pub mod serde_opt_u64_str {
    use super::*;

    pub fn serialize<S: Serializer>(v: &Option<u64>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) => s.serialize_some(&v.to_string()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
        let opt = Option::<serde_json::Value>::deserialize(d)?;
        match opt {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::String(s)) => {
                s.parse::<u64>().map(Some).map_err(serde::de::Error::custom)
            }
            Some(serde_json::Value::Number(n)) => n
                .as_u64()
                .ok_or_else(|| serde::de::Error::custom("negative or non-integer"))
                .map(Some),
            _ => Err(serde::de::Error::custom("expected decimal string or null")),
        }
    }
}

macro_rules! string_enum {
    ($name:ident { $($variant:ident = $wire:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $($variant),+ }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(match self { $(Self::$variant => $wire),+ })
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                match s.as_str() {
                    $($wire => Ok(Self::$variant),)+
                    _ => Err(serde::de::Error::custom(format!("unknown {} value: {}", stringify!($name), s))),
                }
            }
        }

        impl $name {
            pub fn as_str(&self) -> &'static str {
                match self { $(Self::$variant => $wire),+ }
            }
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($wire => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

string_enum!(AgentKind {
    ClaudeCode = "claude_code",
    Codex = "codex",
    GeminiCli = "gemini_cli",
    GrokBuild = "grok_build",
    Opencode = "opencode",
    Pi = "pi",
    Mcode = "mcode",
    KimiCode = "kimi_code",
    ClaudeDesktop = "claude_desktop",
});

string_enum!(StreamDomain {
    LocalLog = "local_log",
    Gateway = "gateway",
    ImportedCcSwitch = "imported_cc_switch",
});

string_enum!(Granularity {
    Request = "request",
    TurnAggregate = "turn_aggregate",
    Message = "message",
    SessionDelta = "session_delta",
    Unknown = "unknown",
});

string_enum!(CounterKind {
    Delta = "delta",
    Cumulative = "cumulative",
    MessageSnapshot = "message_snapshot",
    TurnAggregate = "turn_aggregate",
});

string_enum!(Quality {
    Reported = "reported",
    Derived = "derived",
    Estimated = "estimated",
    Unknown = "unknown",
});

impl Default for Quality {
    /// 空 TokenSet 的总体质量：未知，不冒称 reported。
    fn default() -> Self {
        Quality::Unknown
    }
}

string_enum!(Completeness {
    Complete = "complete",
    Partial = "partial",
    Unknown = "unknown",
});

string_enum!(UnknownReason {
    Absent = "absent",
    Unsupported = "unsupported",
    FormatUnknown = "format_unknown",
    Truncated = "truncated",
    Disconnected = "disconnected",
    PermissionDenied = "permission_denied",
});

/// 标准 Token 分量。输入三分量互斥（契约第 3 节）：
/// input_total = input_fresh + input_cache_read + input_cache_write。
/// output_reasoning 是附属细分，是否已含在 output_total 由源的 output_semantics 声明。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenSet {
    #[serde(default, with = "serde_opt_u64_str")]
    pub input_fresh: Option<u64>,
    #[serde(default, with = "serde_opt_u64_str")]
    pub input_cache_read: Option<u64>,
    #[serde(default, with = "serde_opt_u64_str")]
    pub input_cache_write: Option<u64>,
    #[serde(default, with = "serde_opt_u64_str")]
    pub input_total: Option<u64>,
    #[serde(default, with = "serde_opt_u64_str")]
    pub output_total: Option<u64>,
    #[serde(default, with = "serde_opt_u64_str")]
    pub output_reasoning: Option<u64>,
    #[serde(default, with = "serde_opt_u64_str")]
    pub total: Option<u64>,
    /// 整个 usage 块的总体质量；逐字段未知用 null 表示。缺省为 unknown。
    #[serde(default)]
    pub quality: Quality,
}

impl TokenSet {
    /// 三分量齐全时求输入总量（检查溢出）。任一分量未知返回 None，不补零。
    pub fn derive_input_total(&self) -> Option<u64> {
        let fresh = self.input_fresh?;
        let read = self.input_cache_read?;
        let write = self.input_cache_write?;
        fresh.checked_add(read)?.checked_add(write)
    }

    /// 输入与输出总量齐全时求总 Token。
    pub fn derive_total(&self) -> Option<u64> {
        self.input_total?.checked_add(self.output_total?)
    }

    /// 已知分量小计：把已知的输入分量加起来，用于“已知 Token 小计”展示。
    /// 返回值是 Option 分量各自的和；全部未知时返回 None。
    pub fn known_input_subtotal(&self) -> Option<u64> {
        let mut acc: Option<u64> = None;
        for v in [
            self.input_fresh,
            self.input_cache_read,
            self.input_cache_write,
        ]
        .into_iter()
        .flatten()
        {
            acc = Some(acc.unwrap_or(0).checked_add(v)?);
        }
        acc
    }
}

/// 金额：十进制字符串 + 币种/厂商单位。不做浮点运算；不同币种不合并。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    pub amount: String,
    /// ISO 货币或厂商单位（如 USD、CNY、credits）。
    pub currency: String,
}

impl Money {
    pub fn new(
        amount: impl Into<String>,
        currency: impl Into<String>,
    ) -> Result<Self, ContractError> {
        let amount = amount.into();
        validate_decimal(&amount).map_err(|_| ContractError::InvalidDecimal(amount.clone()))?;
        Ok(Self {
            amount,
            currency: currency.into(),
        })
    }
}

/// 合法性检查：可选负号 + 数字 + 可选小数部分。不做数值解析。
fn validate_decimal(s: &str) -> Result<(), ()> {
    let t = s.strip_prefix('-').unwrap_or(s);
    let mut parts = t.splitn(2, '.');
    let int_part = parts.next().unwrap_or("");
    let frac_part = parts.next();
    if int_part.is_empty() && frac_part.is_none_or(str::is_empty) {
        return Err(());
    }
    if !int_part.chars().all(|c| c.is_ascii_digit()) {
        return Err(());
    }
    if let Some(f) = frac_part {
        if f.is_empty() || !f.chars().all(|c| c.is_ascii_digit()) {
            return Err(());
        }
    }
    Ok(())
}

/// 费用条目。reported 与 estimated 在事件上分开存放，不混合命名。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostInfo {
    pub money: Money,
    pub quality: Quality,
    /// complete = 覆盖该事件全部计费范围；partial = 部分计价（下界）。
    #[serde(default = "default_complete")]
    pub completeness: Completeness,
    /// 估算采用的价格版本；reported 费用为 null。
    #[serde(default)]
    pub price_version: Option<String>,
    /// 该费用覆盖的范围说明（例如仅部分模型调用）。
    #[serde(default)]
    pub scope: Option<String>,
}

fn default_complete() -> Completeness {
    Completeness::Complete
}

/// 账户关联：只能为用户确认 / 明确标识 / 未知，不能从模型名猜。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AccountLink {
    Confirmed { account_id: String },
    ExplicitIdentifier { identifier: String },
    Unknown,
}

/// 源定位证据：一个逻辑事件可有多条物理观察。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub source_id: String,
    /// 规范化文件身份（不是可逆路径）。
    pub file_identity: String,
    /// 文件代数：轮转/替换递增。
    #[serde(with = "serde_u64_str")]
    pub generation: u64,
    /// 字节偏移或行号/行 ID，由适配器决定。
    pub locator: String,
    pub parser_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageEvent {
    pub schema_version: u32,
    /// 逻辑事件稳定身份；不是 Token 数组的哈希。
    pub event_id: String,
    pub agent: AgentKind,
    pub source_id: String,
    pub stream_domain: StreamDomain,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub parent_session_id: Option<String>,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    /// 本机项目引用；敏感路径在存储层单独处理，展示/导出可脱敏。
    #[serde(default)]
    pub project_ref: Option<String>,
    /// UTC RFC3339；缺发生时间保持 None，不伪造为扫描时间。
    #[serde(default)]
    pub occurred_at: Option<String>,
    pub observed_at: String,
    #[serde(default)]
    pub model_raw: Option<String>,
    #[serde(default)]
    pub model_normalized: Option<String>,
    pub granularity: Granularity,
    /// 确认的模型调用次数；聚合源未提供时保持 None。
    #[serde(default, with = "serde_opt_u64_str")]
    pub request_count: Option<u64>,
    pub tokens: TokenSet,
    #[serde(default)]
    pub cost_reported: Option<CostInfo>,
    #[serde(default)]
    pub cost_estimated: Option<CostInfo>,
    #[serde(default = "default_unknown_completeness")]
    pub completeness: Completeness,
    #[serde(default)]
    pub account_link: Option<AccountLink>,
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceRef>,
}

fn default_unknown_completeness() -> Completeness {
    Completeness::Unknown
}

pub const USAGE_EVENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ContractError {
    #[error("invalid decimal string: {0}")]
    InvalidDecimal(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_serialize_as_decimal_strings() {
        let t = TokenSet {
            input_fresh: Some(1200),
            input_cache_read: Some(800),
            ..Default::default()
        };
        let json = serde_json::to_value(&t).unwrap();
        assert_eq!(json["input_fresh"], "1200");
        assert_eq!(json["input_cache_read"], "800");
        assert_eq!(json["output_reasoning"], serde_json::Value::Null);
    }

    #[test]
    fn tokens_beyond_2pow53_roundtrip_without_loss() {
        let t = TokenSet {
            total: Some(9_007_199_254_740_993_123),
            ..Default::default()
        };
        let s = serde_json::to_string(&t).unwrap();
        let back: TokenSet = serde_json::from_str(&s).unwrap();
        assert_eq!(back.total, Some(9_007_199_254_740_993_123));
    }

    #[test]
    fn unknown_is_not_zero() {
        let t = TokenSet::default();
        assert_eq!(t.derive_input_total(), None);
        assert_eq!(t.known_input_subtotal(), None);
    }

    #[test]
    fn input_components_are_exclusive_sum() {
        // 契约场景：互斥 fresh 600 / read 400 / write 100 → 输入总 1100
        let t = TokenSet {
            input_fresh: Some(600),
            input_cache_read: Some(400),
            input_cache_write: Some(100),
            ..Default::default()
        };
        assert_eq!(t.derive_input_total(), Some(1100));
    }

    #[test]
    fn cached_subset_does_not_double_count() {
        // 契约场景：总输入 1000 含 cached 400 → 不能得 1400；
        // fresh/write 划分未知时 input_total 保持源给的 1000。
        let t = TokenSet {
            input_total: Some(1000),
            input_cache_read: Some(400),
            ..Default::default()
        };
        assert_eq!(t.derive_input_total(), None); // 分量不齐，不算
        assert_eq!(t.input_total, Some(1000));
    }

    #[test]
    fn derive_operations_checked_overflow() {
        let t = TokenSet {
            input_fresh: Some(u64::MAX),
            input_cache_read: Some(1),
            input_cache_write: Some(0),
            ..Default::default()
        };
        assert_eq!(t.derive_input_total(), None);
    }

    #[test]
    fn money_rejects_floaty_garbage() {
        assert!(Money::new("10.50", "CNY").is_ok());
        assert!(Money::new("-3", "USD").is_ok());
        assert!(Money::new("1e5", "USD").is_err());
        assert!(Money::new("", "USD").is_err());
        assert!(Money::new("1.2.3", "USD").is_err());
    }

    #[test]
    fn enums_use_contract_wire_names() {
        assert_eq!(
            serde_json::to_string(&AgentKind::KimiCode).unwrap(),
            "\"kimi_code\""
        );
        assert_eq!(
            serde_json::to_string(&Granularity::TurnAggregate).unwrap(),
            "\"turn_aggregate\""
        );
        assert!(AgentKind::parse("claude_desktop") == Some(AgentKind::ClaudeDesktop));
    }

    #[test]
    fn usage_event_example_matches_contract_shape() {
        // 与 DATA_CONTRACTS 第 2 节示例同形
        let event = UsageEvent {
            schema_version: USAGE_EVENT_SCHEMA_VERSION,
            event_id: "synthetic:kimi:request-001".into(),
            agent: AgentKind::KimiCode,
            source_id: "synthetic-source".into(),
            stream_domain: StreamDomain::LocalLog,
            session_id: Some("session-001".into()),
            parent_session_id: None,
            actor_id: Some("main".into()),
            turn_id: None,
            project_ref: None,
            occurred_at: None,
            observed_at: "2026-10-11T06:00:00Z".into(),
            model_raw: None,
            model_normalized: None,
            granularity: Granularity::Request,
            request_count: Some(1),
            tokens: TokenSet {
                input_fresh: Some(1200),
                input_cache_read: Some(800),
                input_cache_write: Some(0),
                input_total: Some(2000),
                output_total: Some(300),
                output_reasoning: None,
                total: Some(2300),
                quality: Quality::Reported,
            },
            cost_reported: None,
            cost_estimated: None,
            completeness: Completeness::Complete,
            account_link: None,
            evidence_refs: vec![],
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["request_count"], "1");
        assert_eq!(json["tokens"]["input_total"], "2000");
        assert_eq!(json["tokens"]["output_reasoning"], serde_json::Value::Null);
        let back: UsageEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
    }
}
