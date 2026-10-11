//! 可唤醒、可停止的后台任务。关闭功能时等待通知，不轮询设置。
use std::{future::Future, time::Duration};
use tokio::sync::watch;

pub struct TaskHandle {
    change: watch::Sender<bool>,
    join: Option<tauri::async_runtime::JoinHandle<()>>,
}

impl TaskHandle {
    pub fn spawn<F, Fut>(run: F) -> Self
    where
        F: FnOnce(watch::Receiver<bool>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let (change, receiver) = watch::channel(false);
        Self {
            change,
            join: Some(tauri::async_runtime::spawn(run(receiver))),
        }
    }

    pub fn wake(&self) {
        let _ = self.change.send(false);
    }

    pub fn shutdown(&mut self) {
        let _ = self.change.send(true);
        if let Some(join) = self.join.take() {
            join.abort();
        }
    }
}

impl Drop for TaskHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 返回 true 表示退出；None 表示只等待设置/账户变化，不启动计时器。
pub async fn wait(duration: Option<Duration>, change: &mut watch::Receiver<bool>) -> bool {
    if *change.borrow() {
        return true;
    }
    match duration {
        Some(duration) => tokio::select! {
            _ = tokio::time::sleep(duration) => false,
            result = change.changed() => result.is_err() || *change.borrow(),
        },
        None => change.changed().await.is_err() || *change.borrow(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn disabled_wait_wakes_on_change_and_stops_on_shutdown() {
        let (sender, mut receiver) = watch::channel(false);
        sender.send(false).unwrap();
        assert!(!wait(None, &mut receiver).await);
        sender.send(true).unwrap();
        assert!(wait(None, &mut receiver).await);
    }
}
