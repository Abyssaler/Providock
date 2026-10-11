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
    // 重扫必须幂等（C03）。活会话可能在两次扫描之间新增记录：
    // 允许「新增 = 真实追加」，但不允许重复计数——事件数增量必须恰好等于新增数。
    let before = usage_summary(&db, &UsageFilter::default()).unwrap();
    let mut inserted_2nd = 0u64;
    for src in &sources {
        let r = scan_source(&mut db, src).unwrap();
        inserted_2nd += r.events_inserted;
    }
    let after = usage_summary(&db, &UsageFilter::default()).unwrap();
    assert_eq!(
        after.events,
        before.events + inserted_2nd,
        "重扫事件数不守恒（重复计数嫌疑）"
    );
    // 第三轮紧跟第二轮（无追加间隙）：必须完全幂等，总量不变
    let mut inserted_3rd = 0u64;
    for src in &sources {
        inserted_3rd += scan_source(&mut db, src).unwrap().events_inserted;
    }
    let third = usage_summary(&db, &UsageFilter::default()).unwrap();
    assert_eq!(third.events, after.events + inserted_3rd);
    if inserted_3rd == 0 {
        assert_eq!(third.total.value, after.total.value, "无新增时总量必须不变");
    }
    eprintln!(
        "summary: events={} total={:?} known_total_events={}",
        after.events, after.total.value, after.total.known_events
    );
}
