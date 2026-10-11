//! A08 Kimi Code 新版（TypeScript）wire.jsonl 纯解析器。
//!
//! 证据：docs/research/evidence/a08-kimi-official.md（kimi-code `c9f01f07`）。要点：
//! - 首行 `{"type":"metadata","protocol_version":"1.5","created_at"}`；
//!   之后每行 `{"type", ...payload, "time"}`（payload 展开在顶层，time 为 epoch 毫秒）。
//! - 每次 LLM 请求一条 durable `usage.record`：`{agentId, model, usage{
//!   inputOther, output, inputCacheRead, inputCacheCreation, raw?}, usageScope?}`。
//!   映射：inputOther→input_fresh，inputCacheRead→input_cache_read，
//!   inputCacheCreation→input_cache_write，output→output_total。
//! - usageScope=turn（source.type=='turn'）与 session（当前唯一来源是全量压缩）
//!   都是独立调用记录，都计入；`agent.status.updated` 非 durable 快照，本就不在文件里。
//! - `token_counting.*` 是上下文测量，不是消费，永不计入。
//! - 上游在 usage 缺失时写全零记录（emptyUsage 兜底）。一次真实调用输入必然 >0，
//!   因此四项全零映射为“用量未知”（字段 null、completeness=partial），不当作 0 消耗。
//! - fork：state.json 有 forkedFrom 且 wire 中有 `forked` 记录时，标记之前的行为
//!   从父会话复制的继承历史；父会话在采集范围内时跳过这些行（父处已计），
//!   父会话不可达时保留并记诊断（唯一留存证据，见 DATA_CONTRACTS 第 4 节）。
//! - undo（agent.switched/context.undo/context.undone）不撤销真实消耗，不特殊处理。
//! - 尾行未以换行结尾视为未写完，不推进已消费偏移；下轮重扫由事件身份去重。
//!
//! 纯函数：不读文件系统、不访问网络与凭据。

use crate::domain::*;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub const PARSER_VERSION: &str = "kimi-wire-1.5";
/// 支持的最高协议版本（迁移链 1.0→1.5 形状一致；更高版本标记不兼容，不猜测）。
pub const SUPPORTED_MAX_PROTOCOL: &str = "1.5";

pub struct WireParseConfig {
    pub source_id: String,
    pub file_identity: String,
    pub generation: u64,
    pub session_id: String,
    /// 目录名给出的 agentId（main / agent-N）；记录内 agentId 优先
    pub dir_agent_id: Option<String>,
    /// state.json 含 forkedFrom
    pub forked_from_present: bool,
    /// 父会话是否在当前采集范围内
    pub parent_available: bool,
    /// 本机观察时间（RFC3339）
    pub observed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compatibility {
    Ok,
    UnsupportedProtocol(String),
    MissingMetadata,
}

pub struct WireParseOutcome {
    pub compatibility: Compatibility,
    pub events: Vec<UsageEvent>,
    /// 已消费字节数（最后一个完整换行之后）。崩溃/截断尾行不计入。
    pub consumed_bytes: u64,
    pub diagnostics: Vec<String>,
}

pub fn parse_wire(bytes: &[u8], cfg: &WireParseConfig) -> WireParseOutcome {
    let mut diagnostics = Vec::new();
    let mut events = Vec::new();

    // 按行迭代并记录每行起始字节偏移；只消费以 \n 结尾的行。
    let mut lines: Vec<(u64, &[u8])> = Vec::new();
    let mut start = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            lines.push((start as u64, &bytes[start..i]));
            start = i + 1;
        }
    }
    let consumed_bytes = start as u64;

    if lines.is_empty() {
        return WireParseOutcome {
            compatibility: Compatibility::MissingMetadata,
            events,
            consumed_bytes,
            diagnostics: vec!["empty_or_unterminated_wire".into()],
        };
    }

    // 协议头
    let compatibility = match parse_metadata(lines[0].1) {
        Some(v) if version_supported(&v) => Compatibility::Ok,
        Some(v) => Compatibility::UnsupportedProtocol(v),
        None => Compatibility::MissingMetadata,
    };
    if compatibility != Compatibility::Ok {
        diagnostics.push(format!("wire_compatibility:{:?}", compatibility));
        return WireParseOutcome {
            compatibility,
            events,
            consumed_bytes: lines[0].0 + lines[0].1.len() as u64 + 1,
            diagnostics,
        };
    }

    // 先找最后一个 forked 标记的位置（继承边界）
    let mut fork_boundary_end: Option<u64> = None;
    for (offset, line) in &lines[1..] {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(line) {
            if v.get("type").and_then(|t| t.as_str()) == Some("forked") {
                fork_boundary_end = Some(offset + line.len() as u64 + 1);
            }
        }
    }

    let skip_inherited = match (
        cfg.forked_from_present,
        cfg.parent_available,
        fork_boundary_end,
    ) {
        (true, true, Some(_)) => true,
        (true, true, None) => {
            diagnostics.push("forked_from_without_marker: 无法定位继承边界，全部保留并标记".into());
            false
        }
        (true, false, _) => {
            diagnostics.push(
                "fork_parent_missing: 父会话不在采集范围，继承记录作为唯一留存证据计入".into(),
            );
            false
        }
        _ => false,
    };
    let mut inherited_skipped: u64 = 0;

    for (offset, line) in &lines[1..] {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(line) else {
            diagnostics.push(format!("malformed_line_at_offset:{offset}"));
            continue;
        };
        let record_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if record_type != "usage.record" {
            continue;
        }
        if skip_inherited
            && fork_boundary_end.is_some_and(|end| (*offset + line.len() as u64 + 1) <= end)
        {
            inherited_skipped += 1;
            continue;
        }
        match build_event(&v, *offset, cfg) {
            Ok(e) => events.push(e),
            Err(msg) => diagnostics.push(format!("usage_record_at_offset:{offset}: {msg}")),
        }
    }
    if inherited_skipped > 0 {
        diagnostics.push(format!(
            "fork_inherited_skipped:{inherited_skipped}（父会话已计，不重复）"
        ));
    }

    WireParseOutcome {
        compatibility,
        events,
        consumed_bytes,
        diagnostics,
    }
}

fn build_event(
    v: &serde_json::Value,
    offset: u64,
    cfg: &WireParseConfig,
) -> std::result::Result<UsageEvent, String> {
    let usage = v.get("usage").ok_or("missing usage object")?;
    let agent_id = v
        .get("agentId")
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .or_else(|| cfg.dir_agent_id.clone());
    let model = v.get("model").and_then(|x| x.as_str()).map(str::to_string);
    let occurred_at = v
        .get("time")
        .and_then(|t| t.as_i64())
        .and_then(|ms| OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).ok())
        .and_then(|dt| dt.format(&Rfc3339).ok());

    let input_fresh = u64_field(usage, "inputOther")?;
    let input_cache_read = u64_field(usage, "inputCacheRead")?;
    let input_cache_write = u64_field(usage, "inputCacheCreation")?;
    let output = u64_field(usage, "output")?;

    // 全零 = 上游 emptyUsage 兜底（usage 缺失），映射为未知而非零消耗
    let all_zero = [input_fresh, input_cache_read, input_cache_write, output]
        .iter()
        .all(|x| *x == 0);
    let (tokens, completeness) = if all_zero {
        (TokenSet::default(), Completeness::Partial)
    } else {
        let input_total = input_fresh
            .checked_add(input_cache_read)
            .and_then(|s| s.checked_add(input_cache_write))
            .ok_or("input total overflow")?;
        let total = input_total.checked_add(output).ok_or("total overflow")?;
        (
            TokenSet {
                input_fresh: Some(input_fresh),
                input_cache_read: Some(input_cache_read),
                input_cache_write: Some(input_cache_write),
                input_total: Some(input_total),
                output_total: Some(output),
                output_reasoning: None,
                total: Some(total),
                quality: Quality::Reported,
            },
            Completeness::Complete,
        )
    };

    Ok(UsageEvent {
        schema_version: USAGE_EVENT_SCHEMA_VERSION,
        // 身份 = 文件身份 + 代数 + 字节偏移：append-only 文件内稳定，
        // 重扫/重启幂等；相同数字的独立请求偏移不同，不会被误判重复。
        event_id: format!(
            "kimi_code:{}:g{}:o{}",
            cfg.file_identity, cfg.generation, offset
        ),
        agent: AgentKind::KimiCode,
        source_id: cfg.source_id.clone(),
        stream_domain: StreamDomain::LocalLog,
        session_id: Some(cfg.session_id.clone()),
        parent_session_id: None,
        actor_id: agent_id,
        turn_id: None,
        project_ref: None,
        occurred_at,
        observed_at: cfg.observed_at.clone(),
        model_raw: model,
        model_normalized: None,
        granularity: Granularity::Request,
        request_count: Some(1),
        tokens,
        cost_reported: None,
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
    })
}

fn u64_field(usage: &serde_json::Value, key: &str) -> std::result::Result<u64, String> {
    usage
        .get(key)
        .and_then(|x| x.as_u64())
        .ok_or_else(|| format!("usage.{key} missing or negative"))
}

fn parse_metadata(line: &[u8]) -> Option<String> {
    let v = serde_json::from_slice::<serde_json::Value>(line).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("metadata") {
        return None;
    }
    v.get("protocol_version")
        .and_then(|p| p.as_str())
        .map(str::to_string)
}

/// 主版本 1 且次版本 ≤ 5 视为同族可解析；更高版本标记不兼容。
fn version_supported(v: &str) -> bool {
    let mut parts = v.split('.');
    match (parts.next(), parts.next()) {
        (Some(major), Some(minor)) => major == "1" && minor.parse::<u32>().is_ok_and(|m| m <= 5),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> WireParseConfig {
        WireParseConfig {
            source_id: "test-src".into(),
            file_identity: "wireA".into(),
            generation: 0,
            session_id: "session_001".into(),
            dir_agent_id: Some("main".into()),
            forked_from_present: false,
            parent_available: false,
            observed_at: "2026-10-11T08:00:00Z".into(),
        }
    }

    #[test]
    fn version_gate() {
        assert!(version_supported("1.0"));
        assert!(version_supported("1.5"));
        assert!(!version_supported("1.6"));
        assert!(!version_supported("2.0"));
        assert!(!version_supported("x"));
    }

    #[test]
    fn all_zero_usage_maps_to_unknown_not_zero() {
        let data = concat!(
            "{\"type\":\"metadata\",\"protocol_version\":\"1.5\",\"created_at\":1760000000000}\n",
            "{\"type\":\"usage.record\",\"agentId\":\"main\",\"model\":\"kimi-k2\",\"usage\":{\"inputOther\":0,\"output\":0,\"inputCacheRead\":0,\"inputCacheCreation\":0},\"usageScope\":\"turn\",\"time\":1760000001000}\n"
        );
        let out = parse_wire(data.as_bytes(), &cfg());
        assert_eq!(out.compatibility, Compatibility::Ok);
        assert_eq!(out.events.len(), 1);
        let e = &out.events[0];
        assert_eq!(e.tokens.input_fresh, None, "全零兜底不是零消耗");
        assert_eq!(e.tokens.total, None);
        assert_eq!(e.completeness, Completeness::Partial);
        assert_eq!(e.request_count, Some(1), "请求真实发生过");
    }

    #[test]
    fn unterminated_tail_not_consumed() {
        let data = concat!(
            "{\"type\":\"metadata\",\"protocol_version\":\"1.5\",\"created_at\":1}\n",
            "{\"type\":\"usage.record\",\"agentId\":\"main\",\"model\":\"m\",\"usage\":{\"inputOther\":5,\"output\":2,\"inputCacheRead\":0,\"inputCacheCreation\":0},\"time\":1}\n",
            "{\"type\":\"usage.record\",\"agentId\":\"main\",\"model\":\"m\",\"usage\":{\"inputOth"
        );
        let out = parse_wire(data.as_bytes(), &cfg());
        assert_eq!(out.events.len(), 1);
        // 第二个换行之后（未写完的尾行起点）不消费
        let second_nl = data.match_indices('\n').nth(1).unwrap().0;
        assert_eq!(out.consumed_bytes, second_nl as u64 + 1);
    }

    #[test]
    fn higher_protocol_is_incompatible_not_guessed() {
        let data = "{\"type\":\"metadata\",\"protocol_version\":\"1.9\",\"created_at\":1}\n{\"type\":\"usage.record\",\"agentId\":\"main\",\"model\":\"m\",\"usage\":{\"inputOther\":5,\"output\":2,\"inputCacheRead\":0,\"inputCacheCreation\":0}}\n";
        let out = parse_wire(data.as_bytes(), &cfg());
        assert_eq!(
            out.compatibility,
            Compatibility::UnsupportedProtocol("1.9".into())
        );
        assert!(out.events.is_empty());
    }
}
