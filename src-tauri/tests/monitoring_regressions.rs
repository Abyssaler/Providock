use providock_lib::{
    adapters::agents::{codex_rollout, kimi_wire},
    domain::AgentKind,
    ingestion::scanner::{scan_source, DiscoveredSource},
    services::aggregate::{usage_series_daily, usage_summary, UsageFilter},
    storage::Database,
};
use std::path::PathBuf;

fn temp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("providock-monitor-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&p).unwrap();
    p
}
fn fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/agents")
        .join(path)
}

#[test]
fn codex_standard_date_depth_and_parser_upgrade_remove_only_proven_duplicates() {
    let root = temp("codex");
    let dir = root.join("sessions/2026/10/11");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rollout-2026-10-11-11111111-1111-4111-8111-111111111111.jsonl");
    std::fs::copy(
        fixture("codex/rollout-jsonl/replayed-cumulative/input/rollout.jsonl"),
        &path,
    )
    .unwrap();
    let source = DiscoveredSource {
        source_id: "codex:test".into(),
        agent: AgentKind::Codex,
        root: root.clone(),
        parser_version: codex_rollout::PARSER_VERSION,
        resolution: "test".into(),
    };
    let mut db = Database::open_in_memory().unwrap();
    let first = scan_source(&mut db, &source).unwrap();
    assert_eq!(first.files_seen, 1);
    assert_eq!(first.events_inserted, 2);
    assert_eq!(
        usage_summary(&db, &UsageFilter::default())
            .unwrap()
            .total
            .value
            .as_deref(),
        Some("240")
    );
    // Simulate an older parser having billed one known replay with its original offset identity.
    let parsed = codex_rollout::parse_rollout(
        &std::fs::read(&path).unwrap(),
        &codex_rollout::CodexParseConfig {
            source_id: source.source_id.clone(),
            file_identity: path.to_string_lossy().into_owned(),
            generation: 0,
            observed_at: "2026-10-11T08:00:00Z".into(),
            filename_thread_id: Some("11111111-1111-4111-8111-111111111111".into()),
            parent_signatures: None,
        },
    );
    let old_id = &parsed.duplicate_event_ids[0];
    let columns = db
        .conn()
        .prepare("PRAGMA table_info(usage_events)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let projection = columns
        .iter()
        .map(|column| {
            if column == "event_id" {
                "?1".to_owned()
            } else {
                column.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    db.conn().execute(&format!("INSERT INTO usage_events SELECT {projection} FROM usage_events ORDER BY rowid LIMIT 1"), [old_id]).unwrap();

    db.conn()
        .execute(
            "UPDATE ingestion_cursors SET parser_version = 'codex-rollout-1'",
            [],
        )
        .unwrap();
    scan_source(&mut db, &source).unwrap();
    let rebuilt = usage_summary(&db, &UsageFilter::default()).unwrap();
    assert_eq!(rebuilt.events, 2);
    assert_eq!(rebuilt.total.value.as_deref(), Some("240"));
    assert_eq!(scan_source(&mut db, &source).unwrap().files_unchanged, 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn model_and_source_filters_apply_to_summary_and_trend_and_unknown_total_stays_unknown() {
    let root = temp("filters");
    let path = root.join("sessions/project/session/agents/main/wire.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::copy(fixture("kimi_code/wire-1.5/basic/input/wire.jsonl"), &path).unwrap();
    let source = DiscoveredSource {
        source_id: "kimi:test".into(),
        agent: AgentKind::KimiCode,
        root: root.clone(),
        parser_version: kimi_wire::PARSER_VERSION,
        resolution: "test".into(),
    };
    let mut db = Database::open_in_memory().unwrap();
    scan_source(&mut db, &source).unwrap();
    db.conn().execute("UPDATE usage_events SET model_normalized = 'chosen' WHERE rowid = (SELECT MIN(rowid) FROM usage_events)", []).unwrap();
    let filter = UsageFilter {
        source_id: Some(source.source_id),
        model: Some("chosen".into()),
        ..Default::default()
    };
    assert_eq!(
        usage_summary(&db, &filter).unwrap().total.value.as_deref(),
        Some("175")
    );
    assert_eq!(
        usage_series_daily(&db, &filter).unwrap()[0].1.as_deref(),
        Some("175")
    );
    db.conn().execute("UPDATE usage_events SET total = NULL, output_total = NULL WHERE model_normalized = 'chosen'", []).unwrap();
    assert!(usage_summary(&db, &filter).unwrap().total.value.is_none());
    assert!(usage_series_daily(&db, &filter).unwrap()[0].1.is_none());
    assert_eq!(
        usage_summary(
            &db,
            &UsageFilter {
                source_id: Some("absent".into()),
                ..Default::default()
            }
        )
        .unwrap()
        .events,
        0
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn codex_total_only_continues_from_last_usage_cumulative_baseline() {
    let mut lines = std::fs::read_to_string(fixture(
        "codex/rollout-jsonl/replayed-cumulative/input/rollout.jsonl",
    ))
    .unwrap()
    .lines()
    .take(3)
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let mut next: serde_json::Value = serde_json::from_str(&lines[2]).unwrap();
    next["timestamp"] = "2026-10-11T07:01:00Z".into();
    let info = next["payload"]["info"].as_object_mut().unwrap();
    info.remove("last_token_usage");
    info.insert("total_token_usage".into(), serde_json::json!({"input_tokens":150,"cached_input_tokens":30,"cache_write_input_tokens":0,"output_tokens":30,"reasoning_output_tokens":0,"total_tokens":180}));
    lines.push(next.to_string());
    let parsed = codex_rollout::parse_rollout(
        (lines.join("\n") + "\n").as_bytes(),
        &codex_rollout::CodexParseConfig {
            source_id: "codex:baseline".into(),
            file_identity: "synthetic".into(),
            generation: 0,
            observed_at: "2026-10-11T08:00:00Z".into(),
            filename_thread_id: Some("11111111-1111-4111-8111-111111111111".into()),
            parent_signatures: None,
        },
    );
    assert_eq!(parsed.events.len(), 2);
    assert_eq!(
        parsed
            .events
            .iter()
            .map(|event| event.tokens.total.unwrap())
            .sum::<u64>(),
        180
    );
    assert_eq!(parsed.events[1].tokens.input_fresh, Some(40));
    assert_eq!(parsed.events[1].request_count, None);
}

#[test]
fn codex_child_append_can_align_with_unchanged_parent() {
    use std::io::Write;
    let root = temp("codex-parent");
    let dir = root.join("sessions/2026/10/11");
    std::fs::create_dir_all(&dir).unwrap();
    let parent_id = "22222222-2222-4222-8222-222222222222";
    let child_id = "33333333-3333-4333-8333-333333333333";
    let child_path = dir.join(format!("rollout-{child_id}.jsonl"));
    let child = std::fs::read_to_string(fixture("codex/rollout-jsonl/fork-replay/input/rollout-2026-10-11T04-00-00-000Z-44444444-4444-4444-4444-444444444444.jsonl")).unwrap().replace("thr_parent", parent_id).replace("thr_child", child_id);
    let mut parent = child.lines().take(4).map(str::to_owned).collect::<Vec<_>>();
    parent[0] = serde_json::json!({"timestamp":"2026-10-11T04:00:00Z","type":"session_meta","payload":{"id":parent_id}}).to_string();
    std::fs::write(
        dir.join(format!("rollout-{parent_id}.jsonl")),
        parent.join("\n") + "\n",
    )
    .unwrap();
    std::fs::write(&child_path, &child).unwrap();
    let source = DiscoveredSource {
        source_id: "codex:fork".into(),
        agent: AgentKind::Codex,
        root: root.clone(),
        parser_version: codex_rollout::PARSER_VERSION,
        resolution: "test".into(),
    };
    let mut db = Database::open_in_memory().unwrap();
    assert_eq!(scan_source(&mut db, &source).unwrap().files_deferred, 0);
    assert_eq!(
        usage_summary(&db, &UsageFilter::default())
            .unwrap()
            .total
            .value
            .as_deref(),
        Some("385")
    );
    // Only the child's file changes. The parent must still supply its inherited prefix.
    let mut next: serde_json::Value = serde_json::from_str(child.lines().last().unwrap()).unwrap();
    next["timestamp"] = "2026-10-11T04:03:10Z".into();
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(&child_path)
            .unwrap(),
        "{next}"
    )
    .unwrap();
    let report = scan_source(&mut db, &source).unwrap();
    assert_eq!(report.files_unchanged, 1);
    assert_eq!(report.files_deferred, 0);
    assert_eq!(report.events_inserted, 1);
    let summary = usage_summary(&db, &UsageFilter::default()).unwrap();
    assert_eq!(summary.events, 4);
    assert_eq!(summary.total.value.as_deref(), Some("440"));
    std::fs::remove_dir_all(root).unwrap();
}
