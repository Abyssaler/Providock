//! 实机冒烟（默认不运行）：对本机真实 ~/.kimi-code 与 ~/.codex 做只读扫描。
//! 只用 aggregate 计数验证解析器对真实文件的兼容性；不输出任何会话内容、
//! 路径细节或密钥；不产生公共样本。
//! 运行：cargo test --test live_smoke -- --ignored --nocapture

use providock_lib::ingestion::scanner::{discover_sources, scan_source};
use providock_lib::services::aggregate::{usage_summary, UsageFilter};
use providock_lib::storage::Database;

#[test]
#[ignore]
fn live_scan_smoke() {
    let sources = discover_sources();
    if sources.is_empty() {
        eprintln!("live smoke: 本机无 Kimi/Codex 默认目录，跳过");
        return;
    }
    let mut db = Database::open_in_memory().unwrap();
    for src in &sources {
        let report = scan_source(&mut db, src).expect("scan failed");
        eprintln!(
            "[{}] files: seen={} scanned={} unchanged={} deferred={}; events: +{} replace={} dup={}; diagnostics={}",
            report.agent,
            report.files_seen,
            report.files_scanned,
            report.files_unchanged,
            report.files_deferred,
            report.events_inserted,
            report.events_replaced,
            report.events_already_present,
            report.diagnostics.len(),
        );
        for d in report.diagnostics.iter().take(10) {
            eprintln!("  diag: {d}");
        }
    }
    // 重扫必须幂等（C03）
    let before = usage_summary(&db, &UsageFilter::default()).unwrap();
    for src in &sources {
        let r = scan_source(&mut db, src).unwrap();
        assert_eq!(r.events_inserted, 0, "{} 重扫不应新增", src.agent.as_str());
    }
    let after = usage_summary(&db, &UsageFilter::default()).unwrap();
    assert_eq!(before.events, after.events);
    assert_eq!(before.total.value, after.total.value);
    eprintln!(
        "summary: events={} total={:?} known_total_events={}",
        after.events, after.total.value, after.total.known_events
    );
}
