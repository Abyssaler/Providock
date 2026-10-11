//! 手动与自动采集共用一个入口；后台只每 60 秒扫描，不启用文件监听。
use super::background::{self, TaskHandle};
use crate::{
    ingestion::scanner::{discover_sources, scan_source, DiscoveredSource, SourceScanReport},
    storage::Database,
};
use rusqlite::OptionalExtension;
use serde::Serialize;
use std::{
    sync::{Arc, Mutex, TryLockError},
    time::Duration,
};
use tauri::Emitter;

pub const AUTO_SYNC_KEY: &str = "usage.auto_sync_enabled";
pub const SYNC_INTERVAL_SECONDS: u64 = 60;
pub const SYNC_EVENT: &str = "usage-sync-updated";

pub fn enabled(db: &Database) -> bool {
    db.conn()
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [AUTO_SYNC_KEY],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()
        .is_none_or(|value| value == "true")
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub running: bool,
    pub last_completed_at: Option<String>,
    pub last_error: Option<String>,
    pub reports: Vec<SourceScanReport>,
}

pub struct UsageSync {
    db: Arc<Mutex<Database>>,
    gate: Mutex<()>,
    status: Mutex<SyncStatus>,
}

impl UsageSync {
    pub fn new(db: Arc<Mutex<Database>>) -> Self {
        Self {
            db,
            gate: Mutex::new(()),
            status: Mutex::new(SyncStatus::default()),
        }
    }

    pub fn status(&self) -> SyncStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// 已有同步时直接合并，不排队再扫一遍。每个源之间释放 DB 锁。
    pub fn run(
        &self,
        automatic: bool,
        stopped: impl Fn() -> bool,
        notify: impl Fn(&SyncStatus),
    ) -> Result<Vec<SourceScanReport>, String> {
        self.run_sources(automatic, discover_sources(), stopped, notify)
    }

    fn run_sources(
        &self,
        automatic: bool,
        sources: Vec<DiscoveredSource>,
        stopped: impl Fn() -> bool,
        notify: impl Fn(&SyncStatus),
    ) -> Result<Vec<SourceScanReport>, String> {
        let _gate = match self.gate.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return Ok(self.status().reports),
            Err(TryLockError::Poisoned(_)) => return Err("同步锁不可用".into()),
        };
        if stopped() {
            return Ok(vec![]);
        }
        if automatic && !enabled(&*self.db.lock().map_err(|e| e.to_string())?) {
            return Ok(vec![]);
        }
        self.status.lock().map_err(|e| e.to_string())?.running = true;
        notify(&self.status());
        let result = (|| {
            let mut reports = Vec::new();
            for source in sources {
                if stopped() {
                    break;
                }
                let mut db = self.db.lock().map_err(|e| e.to_string())?;
                if automatic && !enabled(&db) {
                    break;
                }
                let active = db
                    .conn()
                    .query_row(
                        "SELECT enabled FROM sources WHERE source_id = ?1",
                        [&source.source_id],
                        |r| r.get::<_, bool>(0),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?
                    .unwrap_or(true);
                if !active {
                    continue;
                }
                match scan_source(&mut db, &source) {
                    Ok(report) => reports.push(report),
                    Err(_) => reports.push(SourceScanReport {
                        source_id: source.source_id,
                        agent: source.agent.as_str().into(),
                        diagnostics: vec!["scan_failed".into()],
                        ..Default::default()
                    }),
                }
            }
            Ok::<_, String>(reports)
        })();
        {
            let mut status = self.status.lock().map_err(|e| e.to_string())?;
            status.running = false;
            status.last_completed_at = Some(super::accounts::now_rfc3339());
            status.last_error = result.as_ref().err().cloned();
            if let Ok(reports) = &result {
                status.reports = reports.clone();
            }
        }
        notify(&self.status());
        result
    }
}

pub fn spawn(sync: Arc<UsageSync>, app: tauri::AppHandle) -> TaskHandle {
    TaskHandle::spawn(move |mut change| async move {
        loop {
            if *change.borrow() {
                return;
            }
            let active = sync.db.lock().map(|db| enabled(&db)).unwrap_or(false);
            if active {
                let worker = Arc::clone(&sync);
                let stopped = change.clone();
                let app = app.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    worker.run(
                        true,
                        || *stopped.borrow(),
                        |status| {
                            let _ = app.emit(SYNC_EVENT, status);
                        },
                    )
                })
                .await;
            }
            if background::wait(
                active.then_some(Duration::from_secs(SYNC_INTERVAL_SECONDS)),
                &mut change,
            )
            .await
            {
                return;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn service() -> UsageSync {
        UsageSync::new(Arc::new(Mutex::new(Database::open_in_memory().unwrap())))
    }
    #[test]
    fn disabled_start_does_not_scan_but_manual_still_works() {
        let sync = service();
        sync.db
            .lock()
            .unwrap()
            .conn()
            .execute("INSERT INTO settings VALUES (?1, 'false')", [AUTO_SYNC_KEY])
            .unwrap();
        sync.run_sources(
            true,
            vec![],
            || false,
            |_| panic!("disabled automatic scan ran"),
        )
        .unwrap();
        assert!(sync.status().last_completed_at.is_none());
        sync.run_sources(false, vec![], || false, |_| {}).unwrap();
        assert!(sync.status().last_completed_at.is_some());
    }
    #[test]
    fn concurrent_triggers_are_coalesced_and_stop_prevents_scan() {
        let sync = service();
        let gate = sync.gate.lock().unwrap();
        sync.run_sources(false, vec![], || false, |_| panic!("duplicate scan ran"))
            .unwrap();
        assert!(sync.status().last_completed_at.is_none());
        drop(gate);
        sync.run_sources(false, vec![], || true, |_| panic!("stopped scan ran"))
            .unwrap();
        assert!(sync.status().last_completed_at.is_none());
    }
}
