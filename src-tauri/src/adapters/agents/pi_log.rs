//! A06 Pi 会话 JSONL 纯解析器。
//!
//! 证据：docs/research/evidence/a05-a07-ccswitch-importers.md「A06 Pi」节
//! （cc-switch `df40d53c`，`session_usage_pi.rs`），并对照 Pi 官方
//! `packages/coding-agent/docs/session-format.md`（earendil-works/pi）核实字段形状。
//!
//! 文件形状：
//! - 会话一个 JSONL 文件，首行 `{"type":"session","version":3,"id":...,"timestamp":...}`。
//! - 之后每条 entry 有 `type`/`id`/`parentId`/`timestamp`（entry 时间戳为 ISO 8601 字符串）。
//! - 计费记录（本项目采集范围，见下）：
//!   `message` + `message.role=="assistant"` → usage 在 `message.usage`；
//!   `compaction` / `branch_summary` → usage 在 entry 顶层 `usage`。
//!
//! Usage 映射（Pi 归一化字段，`input` 为新鲜输入、cache 分列相加）：
//! - input_fresh = usage.input
//! - input_cache_read = usage.cacheRead
//! - input_cache_write = usage.cacheWrite
//! - input_total = input + cacheRead + cacheWrite（三者都已知时才算，缺一分量不补零）
//! - output_total = usage.output
//! - output_reasoning = usage.reasoning（附属细分；是否已含在 output 未证实，不另加）
//! - total = input_total + output_total
//!
//! 计数语义：
//! - `message` → granularity=message，request_count=1。
//! - `compaction` / `branch_summary` → granularity=request，request_count=1（各是一次真实模型调用）。
//! - `details.retainedTail` 等嵌套旧消息对象是复制历史，不递归采集（上游夹具已证实）。
//! - 响应 ID（`message.responseId`）可能复用，不用于身份；身份 = 文件身份+代数+字节偏移。
//! - 同一文件内重复出现的同一摘要 entry（compaction / branch_summary 同 `id`）只计一次。
//!   跨文件复制/分叉不在本层归并，由扫描层按可证实祖先处理（契约第 4 节）。
//!
//! 费用：`usage.cost{input,output,cacheRead,cacheWrite,total}`，金额单位为十进制字符串。
//! - 分项与总额都有时核对；矛盾记诊断并保留源给的总额（不挑较大值），completeness=partial。
//! - `total==0` 是合法零，保留（quality=reported）。
//! - 无 `cost` 为 None；未知价格不估算（估算属后续层）。
//!
//! 边界：尾行未换行不消费；畸形行记诊断；纯函数，不读文件系统、不访问网络与凭据。

use crate::domain::decimal::decimal_add;
use crate::domain::*;
use serde_json::Value;
use std::collections::HashSet;

pub const PARSER_VERSION: &str = "pi-session-1";

/// Pi 归一化 cost 的金额单位。上游列名 `total_cost_usd` 指向美元；官方单位待核实。
pub const COST_CURRENCY: &str = "USD";

/// 上游 `MAX_TREE_ID_BYTES`。
const MAX_SESSION_ID_BYTES: usize = 256;
/// 官方 session-format 当前的最高版本（v1 线性 / v2 树 / v3 扩展统一）。
const SUPPORTED_MAX_SESSION_VERSION: u64 = 3;

pub struct PiParseConfig {
    pub source_id: String,
    pub file_identity: String,
    pub generation: u64,
    /// 本机观察时间（RFC3339）
    pub observed_at: String,
}

#[derive(Debug, Default)]
pub struct PiParseOutcome {
    pub events: Vec<UsageEvent>,
    /// 已消费字节数（最后一个完整换行之后）；未写完的尾行不计入。
    pub consumed_bytes: u64,
    pub diagnostics: Vec<String>,
    /// 会话头 `id`（校验通过时）；缺失/不合法为 None。
    pub session_id: Option<String>,
    /// 会话头 `version`（缺省按 v1）；无会话头为 None。
    pub session_version: Option<u64>,
}

pub fn parse_session(bytes: &[u8], cfg: &PiParseConfig) -> PiParseOutcome {
    let mut out = PiParseOutcome::default();

    // 只消费以 \n 结尾的行，并保留每行起始字节偏移。
    let mut lines: Vec<(u64, &[u8])> = Vec::new();
    let mut start = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            lines.push((start as u64, &bytes[start..i]));
            start = i + 1;
        }
    }
    out.consumed_bytes = start as u64;

    // 会话头：首个非空行必须是 type=="session"。
    match lines.iter().find(|(_, l)| !l.is_empty()) {
        Some((_, line)) => match serde_json::from_slice::<Value>(line) {
            Ok(v) if v.get("type").and_then(|t| t.as_str()) == Some("session") => {
                match v.get("id").and_then(|x| x.as_str()) {
                    Some(id) if is_valid_session_id(id) => {
                        out.session_id = Some(id.to_string());
                    }
                    Some(_) => out
                        .diagnostics
                        .push("session_id_invalid: 会话头 id 不合法，会话身份保持未知".into()),
                    None => out.diagnostics.push("session_id_missing".into()),
                }
                let version = v.get("version").and_then(|x| x.as_u64()).unwrap_or(1);
                out.session_version = Some(version);
                if version > SUPPORTED_MAX_SESSION_VERSION {
                    out.diagnostics
                        .push(format!("session_version_unsupported:{version}"));
                }
            }
            _ => out
                .diagnostics
                .push("session_header_missing_or_invalid".into()),
        },
        None => out
            .diagnostics
            .push("session_header_missing_or_invalid".into()),
    }

    let mut seen_summaries: HashSet<String> = HashSet::new();

    for (offset, line) in &lines {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            out.diagnostics
                .push(format!("malformed_line_at_offset:{offset}"));
            continue;
        };
        let record_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        let (usage, model, is_summary) = match record_type {
            "message" => {
                let message = v.get("message");
                let role = message
                    .and_then(|m| m.get("role"))
                    .and_then(|r| r.as_str())
                    .unwrap_or("");
                if role != "assistant" {
                    continue;
                }
                let usage = message.and_then(|m| m.get("usage"));
                let model = message
                    .and_then(|m| m.get("model"))
                    .and_then(|x| x.as_str())
                    .or_else(|| {
                        message
                            .and_then(|m| m.get("responseModel"))
                            .and_then(|x| x.as_str())
                    });
                (usage, model, false)
            }
            "compaction" | "branch_summary" => (v.get("usage"), None, true),
            _ => continue,
        };

        let Some(usage) = usage.filter(|u| u.is_object()) else {
            continue;
        };

        // 摘要型 entry 文件内去重：同一对象重复出现不重复计（响应 ID 复用不可作身份）。
        if is_summary {
            let key = v
                .get("id")
                .and_then(|x| x.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| serde_json::to_string(&v).unwrap_or_default());
            if !seen_summaries.insert(key.clone()) {
                out.diagnostics
                    .push(format!("duplicate_summary_entry:{key}@{offset}"));
                continue;
            }
        }

        let Some(tokens) = build_tokens(&usage, *offset, &mut out.diagnostics) else {
            continue;
        };
        let cost_reported = build_cost(&usage, *offset, &mut out.diagnostics);
        let occurred_at = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .map(str::to_string);
        let completeness = if tokens.input_total.is_some() && tokens.output_total.is_some() {
            Completeness::Complete
        } else {
            Completeness::Partial
        };

        out.events.push(UsageEvent {
            schema_version: USAGE_EVENT_SCHEMA_VERSION,
            // 偏移身份：append-only 文件内稳定，重扫/重启幂等；
            // 相同数字的独立请求偏移不同，不会被误判重复。
            event_id: format!(
                "pi:{}:g{}:o{}",
                cfg.file_identity, cfg.generation, offset
            ),
            agent: AgentKind::Pi,
            source_id: cfg.source_id.clone(),
            stream_domain: StreamDomain::LocalLog,
            session_id: out.session_id.clone(),
            parent_session_id: None,
            actor_id: None,
            turn_id: None,
            project_ref: None,
            occurred_at,
            observed_at: cfg.observed_at.clone(),
            model_raw: model.map(str::to_string),
            model_normalized: None,
            granularity: if is_summary {
                Granularity::Request
            } else {
                Granularity::Message
            },
            request_count: Some(1),
            tokens,
            cost_reported,
            cost_estimated: None,
            completeness,
            account_link: None,
            evidence_refs: vec![EvidenceRef {
                source_id: cfg.source_id.clone(),
                file_identity: cfg.file_identity.clone(),
                generation: cfg.generation,
                locator: offset.to_string(),
                parser_version: PARSER_VERSION.into(),
            }],
        });
    }

    out
}

/// 读取 usage 的四项核心分量为标准 TokenSet；四项全缺则不是计费记录（返回 None）。
fn build_tokens(usage: &Value, offset: u64, diags: &mut Vec<String>) -> Option<TokenSet> {
    let input = token_field(usage, "input", offset, diags);
    let output = token_field(usage, "output", offset, diags);
    let cache_read = token_field(usage, "cacheRead", offset, diags);
    let cache_write = token_field(usage, "cacheWrite", offset, diags);
    let reasoning = token_field(usage, "reasoning", offset, diags);

    let has_core = input.is_some() || output.is_some() || cache_read.is_some() || cache_write.is_some();
    if !has_core {
        diags.push(format!("usage_without_consumption_fields@{offset}"));
        return None;
    }

    let input_total = match (input, cache_read, cache_write) {
        (Some(a), Some(b), Some(c)) => match a.checked_add(b).and_then(|s| s.checked_add(c)) {
            Some(total) => Some(total),
            None => {
                diags.push(format!("input_total_overflow@{offset}"));
                None
            }
        },
        // 缺一分量：不补零，输入总量保持未知（契约第 3 节）。
        _ => None,
    };
    let total = match (input_total, output) {
        (Some(a), Some(b)) => match a.checked_add(b) {
            Some(total) => Some(total),
            None => {
                diags.push(format!("total_overflow@{offset}"));
                None
            }
        },
        _ => None,
    };
    let all_core = input.is_some() && cache_read.is_some() && cache_write.is_some() && output.is_some();

    Some(TokenSet {
        input_fresh: input,
        input_cache_read: cache_read,
        input_cache_write: cache_write,
        input_total,
        output_total: output,
        output_reasoning: reasoning,
        total,
        quality: if all_core {
            Quality::Reported
        } else {
            Quality::Derived
        },
    })
}

fn token_field(usage: &Value, key: &str, offset: u64, diags: &mut Vec<String>) -> Option<u64> {
    match usage.get(key) {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_u64() {
            Some(n) => Some(n),
            None => {
                diags.push(format!("usage_field_invalid:{key}@{offset}"));
                None
            }
        },
    }
}

/// 解析 `usage.cost`。分项与总额都有时核对；矛盾记诊断并保留源总额（不挑大值）。
fn build_cost(usage: &Value, offset: u64, diags: &mut Vec<String>) -> Option<CostInfo> {
    let cost = usage.get("cost")?.as_object()?;

    let mut components: Vec<String> = Vec::new();
    for key in ["input", "output", "cacheRead", "cacheWrite"] {
        if let Some(v) = cost_decimal(cost.get(key), key, offset, diags) {
            components.push(v);
        }
    }
    let total = cost_decimal(cost.get("total"), "total", offset, diags);

    if total.is_none() && components.is_empty() {
        diags.push(format!("cost_without_recognized_fields@{offset}"));
        return None;
    }

    let (amount, quality, completeness) = match &total {
        Some(total) => {
            let mut completeness = Completeness::Complete;
            if components.len() == 4 {
                if let Some(sum) = sum_decimals(&components) {
                    if !decimal_eq(&sum, total) {
                        diags.push(format!(
                            "cost_component_total_mismatch:components={sum},total={total}@{offset}"
                        ));
                        completeness = Completeness::Partial;
                    }
                }
            }
            (total.clone(), Quality::Reported, completeness)
        }
        None => {
            let sum = sum_decimals(&components)?;
            let completeness = if components.len() == 4 {
                Completeness::Complete
            } else {
                Completeness::Partial
            };
            (sum, Quality::Derived, completeness)
        }
    };

    let money = match Money::new(amount.clone(), COST_CURRENCY) {
        Ok(money) => money,
        Err(_) => {
            diags.push(format!("cost_amount_invalid:{amount}@{offset}"));
            return None;
        }
    };
    Some(CostInfo {
        money,
        quality,
        completeness,
        price_version: None,
        scope: None,
    })
}

/// 单个 cost 字段 → 规范化十进制字符串。接受 JSON 数字或字符串；负值按上游钳制为 0。
fn cost_decimal(
    value: Option<&Value>,
    key: &str,
    offset: u64,
    diags: &mut Vec<String>,
) -> Option<String> {
    let raw = match value? {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Null => return None,
        _ => {
            diags.push(format!("cost_field_invalid:{key}@{offset}"));
            return None;
        }
    };
    let plain = match plain_decimal(&raw) {
        Some(p) => p,
        None => {
            diags.push(format!("cost_field_invalid:{key}@{offset}"));
            return None;
        }
    };
    if plain.starts_with('-') {
        diags.push(format!("cost_negative_clamped:{key}@{offset}"));
        return Some("0".into());
    }
    Some(plain)
}

fn sum_decimals(parts: &[String]) -> Option<String> {
    let mut acc = "0".to_string();
    for part in parts {
        acc = decimal_add(&acc, part)?;
    }
    Some(acc)
}

/// 十进制相等（按值，忽略尾零与写法差异），不引入浮点。
fn decimal_eq(a: &str, b: &str) -> bool {
    let neg_b = match b.strip_prefix('-') {
        Some(rest) => rest.to_string(),
        None => format!("-{b}"),
    };
    decimal_add(a, &neg_b).as_deref() == Some("0")
}

/// 把 JSON 数字/字符串的十进制写法规范化为无指数的普通十进制字符串。
/// 例：`1.55e-5` → `0.0000155`；`100` → `100`；`-0` → `0`。非法输入返回 None。
fn plain_decimal(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (neg, raw) = match raw.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    let raw = raw.strip_prefix('+').unwrap_or(raw);

    let (mantissa, exp) = match raw.find(|c: char| c == 'e' || c == 'E') {
        Some(i) => (&raw[..i], raw[i + 1..].parse::<i64>().ok()?),
        None => (raw, 0),
    };
    let (int_part, frac_part) = match mantissa.find('.') {
        Some(i) => (&mantissa[..i], &mantissa[i + 1..]),
        None => (mantissa, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    if !int_part.chars().all(|c| c.is_ascii_digit())
        || !frac_part.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }

    let digits = format!("{int_part}{frac_part}");
    let point = int_part.len() as i64 + exp;
    let n_digits = digits.len() as i64;
    let (int_out, frac_out) = if point <= 0 {
        ("0".to_string(), format!("{}{digits}", "0".repeat((-point) as usize)))
    } else if point >= n_digits {
        (format!("{digits}{}", "0".repeat((point - n_digits) as usize)), String::new())
    } else {
        let p = point as usize;
        (digits[..p].to_string(), digits[p..].to_string())
    };

    let int_out = int_out.trim_start_matches('0');
    let int_out = if int_out.is_empty() { "0" } else { int_out };
    // 不去尾零：保留源给的小数位（契约「保留源精度」）；等值比较由 decimal_eq 处理。
    let body = if frac_out.is_empty() {
        int_out.to_string()
    } else {
        format!("{int_out}.{frac_out}")
    };
    if body.trim_end_matches('0').trim_end_matches('.').is_empty() {
        return Some("0".into());
    }
    Some(if neg { format!("-{body}") } else { body })
}

/// 上游 `is_valid_tree_id`：非空、≤256 字节、首尾 ASCII 字母数字、字符集 [A-Za-z0-9._-]。
fn is_valid_session_id(s: &str) -> bool {
    let alpha_num = |c: char| c.is_ascii_alphanumeric();
    !s.is_empty()
        && s.len() <= MAX_SESSION_ID_BYTES
        && s.chars().all(|c| alpha_num(c) || matches!(c, '.' | '_' | '-'))
        && s.starts_with(alpha_num)
        && s.ends_with(alpha_num)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> PiParseConfig {
        PiParseConfig {
            source_id: "test-src".into(),
            file_identity: "sessionA".into(),
            generation: 0,
            observed_at: "2026-10-11T08:00:00Z".into(),
        }
    }

    #[test]
    fn plain_decimal_expands_exponent() {
        assert_eq!(plain_decimal("1.55e-5").as_deref(), Some("0.0000155"));
        assert_eq!(plain_decimal("100").as_deref(), Some("100"));
        // 无指数时保留源小数位（契约「保留源精度」）
        assert_eq!(plain_decimal("0.10").as_deref(), Some("0.10"));
        assert_eq!(plain_decimal("0.000").as_deref(), Some("0.000"));
        assert_eq!(plain_decimal("-0").as_deref(), Some("0"));
        assert_eq!(plain_decimal("1e3").as_deref(), Some("1000"));
        assert_eq!(plain_decimal("1.5e1").as_deref(), Some("15"));
        assert_eq!(plain_decimal("nope"), None);
        assert_eq!(plain_decimal(""), None);
    }

    #[test]
    fn session_id_validation() {
        assert!(is_valid_session_id("a1b2c3d4"));
        assert!(is_valid_session_id("a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d"));
        assert!(!is_valid_session_id(""));
        assert!(!is_valid_session_id("-lead"));
        assert!(!is_valid_session_id("has space"));
        assert!(!is_valid_session_id(&"x".repeat(257)));
    }

    #[test]
    fn assistant_usage_maps_and_summary_counts_as_request() {
        let data = concat!(
            "{\"type\":\"session\",\"version\":3,\"id\":\"sess0001\",\"timestamp\":\"2026-10-11T01:00:00.000Z\"}\n",
            "{\"type\":\"message\",\"id\":\"00000001\",\"parentId\":null,\"timestamp\":\"2026-10-11T01:00:01.000Z\",\"message\":{\"role\":\"assistant\",\"model\":\"m\",\"usage\":{\"input\":100,\"output\":40,\"cacheRead\":30,\"cacheWrite\":5}}}\n",
            "{\"type\":\"compaction\",\"id\":\"00000002\",\"parentId\":\"00000001\",\"timestamp\":\"2026-10-11T01:00:02.000Z\",\"usage\":{\"input\":11,\"output\":4,\"cacheRead\":0,\"cacheWrite\":0}}\n"
        );
        let out = parse_session(data.as_bytes(), &cfg());
        assert_eq!(out.session_id.as_deref(), Some("sess0001"));
        assert_eq!(out.events.len(), 2);
        let a = &out.events[0];
        assert_eq!(a.tokens.input_fresh, Some(100));
        assert_eq!(a.tokens.input_cache_read, Some(30));
        assert_eq!(a.tokens.input_cache_write, Some(5));
        assert_eq!(a.tokens.input_total, Some(135));
        assert_eq!(a.tokens.total, Some(175));
        assert_eq!(a.granularity, Granularity::Message);
        let first_entry_offset = data.find('\n').unwrap() as u64 + 1;
        assert_eq!(a.event_id, format!("pi:sessionA:g0:o{first_entry_offset}"));
        let c = &out.events[1];
        assert_eq!(c.granularity, Granularity::Request);
        assert_eq!(c.request_count, Some(1));
        assert!(c.model_raw.is_none());
    }

    #[test]
    fn unterminated_tail_not_consumed() {
        let data = concat!(
            "{\"type\":\"session\",\"version\":3,\"id\":\"sess0002\",\"timestamp\":\"2026-10-11T01:00:00.000Z\"}\n",
            "{\"type\":\"message\",\"id\":\"00000001\",\"parentId\":null,\"timestamp\":\"2026-10-11T01:00:01.000Z\",\"message\":{\"role\":\"assistant\",\"model\":\"m\",\"usage\":{\"input\":5,\"output\":2,\"cacheRead\":0,\"cacheWrite\":0}}}\n",
            "{\"type\":\"message\",\"id\":\"00000002\",\"parentId\":\"00000001\",\"message\":{\"role\":\"assistant\",\"usage\":{\"input\":9"
        );
        let out = parse_session(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1);
        let second_nl = data.match_indices('\n').nth(1).unwrap().0;
        assert_eq!(out.consumed_bytes, second_nl as u64 + 1);
    }
}
