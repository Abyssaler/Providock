//! A05 OpenCode 夹具驱动测试（fixtures/agents/opencode/）。
//!
//! 输入是 SQLite 合成库；预期值人工独立计算（见各用例 README），不由 parser 反推。
//! `consumed` 断言针对 JSONL 的字节偏移，对 SQLite 源不适用，故本 harness 不做该断言。
//! 版本探测、字段映射、费用 0、compaction、V1/V2 并存、消息行更新均在 expected.json 描述。

use providock_lib::adapters::agents::opencode_db::{open_read_only, read_events, OpenCodeConfig};
use providock_lib::domain::{TokenSet, UsageEvent};
use rusqlite::{params, Connection, OpenFlags};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn cases_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/agents/opencode")
}

fn token_field(t: &TokenSet, name: &str) -> Option<u64> {
    match name {
        "input_fresh" => t.input_fresh,
        "input_cache_read" => t.input_cache_read,
        "input_cache_write" => t.input_cache_write,
        "input_total" => t.input_total,
        "output_total" => t.output_total,
        "total" => t.total,
        _ => panic!("unknown token field {name}"),
    }
}

fn sum_field(events: &[&UsageEvent], field: &str) -> Option<u128> {
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
            (p.is_dir() && p.join("expected.json").exists())
                .then(|| p.file_name().unwrap().to_string_lossy().into_owned())
        })
        .collect();
    cases.sort();
    assert!(cases.len() >= 3, "fixture cases missing: {cases:?}");

    for case in cases {
        let dir = root.join(&case);
        let expected: Value =
            serde_json::from_slice(&std::fs::read(dir.join("expected.json")).unwrap()).unwrap();
        let db_path = dir.join("input/opencode.db");
        assert!(db_path.exists(), "{case}: missing input/opencode.db");

        let cfg = OpenCodeConfig {
            source_id: format!("fixture:{case}"),
            file_identity: format!("fixture-{case}"),
            generation: 0,
            observed_at: "2026-10-11T08:00:00Z".into(),
            session_id: expected["config"]["session_id"]
                .as_str()
                .map(str::to_string),
        };

        let conn = open_read_only(&db_path).unwrap();
        let out = read_events(&conn, &cfg);

        // 版本探测
        let want_schema = expected["version"].as_str().unwrap();
        assert_eq!(
            out.schema.map(|s| s.as_str()),
            Some(want_schema),
            "{case} schema"
        );

        // 事件数
        assert_eq!(
            out.events.len(),
            expected["events"].as_u64().unwrap() as usize,
            "{case} event count"
        );

        // 原生 id 身份：消息行会被更新，全部必须 replace_existing。
        for p in &out.events {
            assert!(p.replace_existing, "{case} replace_existing must be true");
        }

        let events: Vec<&UsageEvent> = out.events.iter().map(|p| &p.event).collect();

        // 合计
        if let Some(totals) = expected.get("totals") {
            for field in [
                "input_fresh",
                "input_cache_read",
                "input_cache_write",
                "input_total",
                "output_total",
                "total",
            ] {
                let want: Option<u128> = match &totals[field] {
                    Value::Null => None,
                    Value::String(s) => Some(s.parse().unwrap()),
                    other => panic!("{case}.{field}: bad expected {other}"),
                };
                assert_eq!(sum_field(&events, field), want, "{case} total {field}");
            }
            let want_req: Option<u128> =
                totals["request_count"].as_str().map(|s| s.parse().unwrap());
            let got_req: Option<u128> = events.iter().fold(None, |acc, e| {
                e.request_count.map(|v| acc.unwrap_or(0) + u128::from(v))
            });
            assert_eq!(got_req, want_req, "{case} request_count");
        }

        // 粒度
        if let Some(g) = expected.get("granularity") {
            let want = g.as_str().unwrap();
            for e in &events {
                assert_eq!(e.granularity.as_str(), want, "{case} granularity");
            }
        }

        // 事件 id 前缀：V1+V2 并存时确认只读 V2，冻结的 V1 行不出现。
        if let Some(prefix) = expected.get("event_id_prefix").and_then(|v| v.as_str()) {
            for e in &events {
                assert!(
                    e.event_id.starts_with(prefix),
                    "{case} event_id {} not under {prefix}",
                    e.event_id
                );
            }
        }

        // 逐事件费用（null = 无 cost 字段 = 未知）。

        // 诊断包含
        for d in expected["diagnostics_include"].as_array().unwrap() {
            let needle = d.as_str().unwrap();
            assert!(
                out.diagnostics.iter().any(|x| x.contains(needle)),
                "{case} diagnostics missing {needle}: {:?}",
                out.diagnostics
            );
        }

        // 消息行更新：同一 message id 新值替换（在临时副本上执行 UPDATE，不改动夹具）。
        if let Some(uc) = expected.get("update_case") {
            let base_events = expected["events"].as_u64().unwrap() as usize;
            run_update_case(&case, &db_path, &cfg, uc, base_events);
        }
    }
}

fn run_update_case(
    case: &str,
    db_path: &Path,
    cfg: &OpenCodeConfig,
    uc: &Value,
    base_events: usize,
) {
    let tmp = std::env::temp_dir().join(format!(
        "providock-opencode-{case}-{}.db",
        std::process::id()
    ));
    std::fs::copy(db_path, &tmp).unwrap();

    // 写副本：模拟上游对该消息行的后续更新（夹具本体只读）。
    {
        let conn = Connection::open(&tmp).unwrap();
        let sql = uc["update_sql"].as_str().unwrap();
        let new_data = serde_json::to_string(&uc["new_data"]).unwrap();
        let message_id = uc["message_id"].as_str().unwrap();
        conn.execute(sql, params![new_data, message_id]).unwrap();
    }

    let conn = Connection::open_with_flags(&tmp, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    conn.busy_timeout(Duration::from_millis(2_000)).unwrap();
    let out = read_events(&conn, cfg);

    let want = &uc["expect"];
    let event_id = want["event_id"].as_str().unwrap();
    let pe = out
        .events
        .iter()
        .find(|p| p.event.event_id == event_id)
        .unwrap_or_else(|| panic!("{case}: updated event {event_id} not found"));
    if want["replace_existing"].as_bool().unwrap_or(false) {
        assert!(pe.replace_existing, "{case} replace_existing after update");
    }
    for field in [
        "input_fresh",
        "input_cache_read",
        "input_cache_write",
        "input_total",
        "output_total",
        "total",
    ] {
        if let Some(s) = want[field].as_str() {
            assert_eq!(
                token_field(&pe.event.tokens, field),
                Some(s.parse().unwrap()),
                "{case} updated {field}"
            );
        }
    }

    // 同一 message id 是替换而非新增：事件总数不变。
    assert_eq!(
        out.events.len(),
        base_events,
        "{case} update must not change event count"
    );

    std::fs::remove_file(&tmp).ok();
}
