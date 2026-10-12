//! Small read-only menu-bar summary. Never scans files or queries provider accounts.
use crate::storage::Database;
use std::collections::BTreeMap;

pub const AGENTS: [(&str, &str); 9] = [
    ("claude_code", "Claude Code"),
    ("codex", "Codex"),
    ("gemini_cli", "Gemini CLI"),
    ("grok_build", "Grok Build"),
    ("opencode", "OpenCode"),
    ("pi", "Pi"),
    ("mcode", "MCode"),
    ("kimi_code", "Kimi Code"),
    ("deepseek_harness", "DeepSeek Harness"),
];
#[derive(Debug, Default)]
pub struct RecentUsage {
    pub events: u64,
    /// total, fresh input, output, cache read; None means no known quantity.
    pub tokens: [Option<u128>; 4],
    pub agents: BTreeMap<String, Option<u128>>,
}
pub fn recent(db: &Database, now: time::OffsetDateTime) -> Result<RecentUsage, String> {
    let fmt = time::format_description::well_known::Rfc3339;
    let from = (now - time::Duration::hours(24))
        .format(&fmt)
        .map_err(|e| e.to_string())?;
    let to = now.format(&fmt).map_err(|e| e.to_string())?;
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT agent,total,input_fresh,output_total,input_cache_read FROM usage_events
         WHERE stream_domain='local_log' AND julianday(occurred_at)>=julianday(?1)
         AND julianday(occurred_at)<julianday(?2)",
        )
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query([from, to]).map_err(|e| e.to_string())?;
    let mut out = RecentUsage::default();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        out.events += 1;
        let agent: String = row.get(0).map_err(|e| e.to_string())?;
        let agent_sum = out.agents.entry(agent).or_default();
        for (i, sum) in out.tokens.iter_mut().enumerate() {
            if let Some(n) = row
                .get::<_, Option<i64>>(i + 1)
                .map_err(|e| e.to_string())?
            {
                let n = u128::try_from(n).map_err(|_| "负 Token 无法计量")?;
                *sum = Some(sum.unwrap_or(0).checked_add(n).ok_or("Token 汇总溢出")?);
                if i == 0 {
                    *agent_sum = Some(
                        agent_sum
                            .unwrap_or(0)
                            .checked_add(n)
                            .ok_or("Token 汇总溢出")?,
                    );
                }
            }
        }
    }
    Ok(out)
}
pub fn compact(n: Option<u128>) -> String {
    let Some(n) = n else { return "—".into() };
    let (divisor, unit) = if n >= 100_000_000 {
        (100_000_000, "亿")
    } else if n >= 10_000 {
        (10_000, "万")
    } else {
        return n.to_string();
    };
    let integer = n / divisor;
    let hundredths = (n % divisor) * 100 / divisor;
    format!("{integer}.{hundredths:02}{unit}")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rolling_window_handles_offsets_and_excludes_unknown_time_and_gateway() {
        let db = Database::open_in_memory().unwrap();
        db.conn().execute("INSERT INTO sources(source_id,agent,stream_domain,root_path,parser_version) VALUES ('s','codex','local_log','/synthetic','test')", []).unwrap();
        db.conn().execute("INSERT INTO parse_runs(run_id,source_id,parser_version,status) VALUES (1,'s','test','committed')", []).unwrap();
        for (id, at, domain, total) in [
            (
                "start",
                Some("2026-10-11T08:00:00+08:00"),
                "local_log",
                Some(12),
            ),
            (
                "inside",
                Some("2026-10-12T07:59:59+08:00"),
                "local_log",
                Some(34),
            ),
            (
                "unknown-count",
                Some("2026-10-11T12:00:00Z"),
                "local_log",
                None,
            ),
            ("end", Some("2026-10-12T00:00:00Z"), "local_log", Some(100)),
            ("old", Some("2026-10-10T23:59:59Z"), "local_log", Some(100)),
            (
                "gateway",
                Some("2026-10-11T12:00:00Z"),
                "gateway",
                Some(100),
            ),
            ("unknown-time", None, "local_log", Some(100)),
        ] {
            db.conn().execute("INSERT INTO usage_events(event_id,schema_version,parse_run_id,source_id,agent,stream_domain,occurred_at,observed_at,granularity,total,tokens_quality) VALUES (?1,1,1,'s','codex',?3,?2,'2026-10-12T00:00:00Z','request',?4,'reported')", rusqlite::params![id,at,domain,total]).unwrap();
        }
        let now = time::OffsetDateTime::parse(
            "2026-10-12T00:00:00Z",
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap();
        let out = recent(&db, now).unwrap();
        assert_eq!(out.events, 3);
        assert_eq!(out.tokens, [Some(46), None, None, None]);
        assert_eq!(out.agents["codex"], Some(46));
        assert_eq!(compact(None), "—");
        assert_eq!(compact(Some(0)), "0");
        assert_eq!(
            compact(Some(u128::MAX)),
            "3402823669209384634633746074317.68亿"
        );
    }
}
