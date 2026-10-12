//! Official App Server stdio connection, limited to initialization and read-only account methods.
//! Authentication stays with the official Codex process; Providock never reads or copies tokens.
use crate::adapters::accounts::codex_appserver::{self as protocol, CodexError, CodexRateLimits};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

const MAX_OUTPUT: u64 = 2 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(15);
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn candidates() -> Vec<PathBuf> {
    if let Some(path) = std::env::var_os("CODEX_CLI_PATH").filter(|v| !v.is_empty()) {
        return vec![path.into()];
    }
    let mut paths = vec![PathBuf::from("codex")];
    #[cfg(target_os = "macos")]
    paths.extend([
        "/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
        "/Applications/Codex.app/Contents/Resources/codex",
        "/opt/homebrew/bin/codex",
        "/usr/local/bin/codex",
    ].map(PathBuf::from));
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        paths.push(PathBuf::from(home).join(".local/bin/codex"));
    }
    paths
}

pub fn read_limits() -> Result<CodexRateLimits, CodexError> {
    for path in candidates() {
        let mut command = Command::new(path);
        command
            .arg("app-server")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        match command.spawn() {
            Ok(child) => return read_from_child(child, TIMEOUT),
            Err(_) => continue,
        }
    }
    Err(CodexError::Transport(
        "未找到 Codex CLI，请安装官方 CLI 或设置 CODEX_CLI_PATH".into(),
    ))
}

fn read_from_child(child: Child, timeout: Duration) -> Result<CodexRateLimits, CodexError> {
    let mut process = Process(child);
    let stdout = process
        .0
        .stdout
        .take()
        .ok_or_else(|| CodexError::Transport("Codex 输出不可用".into()))?;
    let mut stdin = process
        .0
        .stdin
        .take()
        .ok_or_else(|| CodexError::Transport("Codex 输入不可用".into()))?;
    let (sender, receiver) = mpsc::sync_channel(32);
    let reader = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout.take(MAX_OUTPUT + 1));
        loop {
            let mut line = Vec::new();
            let read = (&mut reader)
                .take(MAX_OUTPUT + 1)
                .read_until(b'\n', &mut line);
            if !matches!(read, Ok(n) if n > 0) || line.len() as u64 > MAX_OUTPUT {
                break;
            }
            if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                if sender.send(value).is_err() {
                    break;
                }
            }
        }
    });
    let deadline = Instant::now() + timeout;
    let send = |stdin: &mut std::process::ChildStdin, value: Value| -> Result<(), CodexError> {
        writeln!(stdin, "{value}")
            .and_then(|_| stdin.flush())
            .map_err(|_| CodexError::Transport("Codex 连接已关闭".into()))
    };
    let response = |id: i64| -> Result<Value, CodexError> {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let value = receiver.recv_timeout(remaining).map_err(|_| {
                CodexError::Transport("Codex 额度查询超时或连接已关闭，请重试".into())
            })?;
            if value.get("id").and_then(Value::as_i64) == Some(id) {
                return Ok(value);
            }
            // No notification or server request can initiate a turn, tool, or approval.
        }
    };
    let result = (|| {
        send(
            &mut stdin,
            protocol::initialize(
                1,
                "providock",
                Some("Providock"),
                env!("CARGO_PKG_VERSION"),
                false,
            )
            .to_wire(),
        )?;
        protocol::parse_initialize(&serde_json::to_vec(&response(1)?).unwrap_or_default())?;
        send(&mut stdin, json!({"method":"initialized"}))?;
        send(
            &mut stdin,
            json!({"method":"account/read","id":2,"params":{"refreshToken":false}}),
        )?;
        let account = response(2)?;
        match account
            .pointer("/result/account/type")
            .and_then(Value::as_str)
        {
            Some("chatgpt") => {}
            Some(_) => {
                return Err(CodexError::Unsupported(
                    "Codex 订阅额度需要 ChatGPT 登录，API Key 登录不支持".into(),
                ))
            }
            None => {
                return Err(CodexError::AuthRequired(
                    "请先在官方 Codex 中使用 ChatGPT 账户登录".into(),
                ))
            }
        }
        send(
            &mut stdin,
            protocol::rate_limits_read(3, false, true).to_wire(),
        )?;
        protocol::parse_rate_limits(&serde_json::to_vec(&response(3)?).unwrap_or_default())
    })();
    drop(stdin);
    drop(receiver);
    drop(process); // Kill before joining: a silent or stalled child cannot keep the reader alive.
    let _ = reader.join();
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn official_handshake_uses_only_read_only_account_methods() {
        let script = r#"
read -r init
case "$init" in *'"method":"initialize"'*) ;; *) exit 2 ;; esac
printf '%s\n' '{"id":1,"result":{"userAgent":"synthetic/1.0","platformFamily":"unix","platformOs":"macos"}}'
read -r initialized
case "$initialized" in *'"method":"initialized"'*) ;; *) exit 3 ;; esac
read -r account
case "$account" in *'"method":"account/read"'*) ;; *) exit 4 ;; esac
printf '%s\n' '{"id":2,"result":{"account":{"type":"chatgpt"}}}'
read -r limits
case "$limits" in *'"method":"account/rateLimits/read"'*) ;; *) exit 5 ;; esac
printf '%s\n' '{"id":3,"result":{"rateLimits":{"primary":{"usedPercent":25,"windowDurationMins":300,"resetsAt":1791853200}}}}'
"#;
        let child = Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let limits = read_from_child(child, Duration::from_secs(2)).unwrap();
        assert_eq!(limits.windows.len(), 1);
        assert_eq!(limits.windows[0].remaining_ratio.as_deref(), Some("0.75"));
        assert_eq!(limits.windows[0].duration_seconds, Some(18000));
    }
    #[cfg(unix)]
    #[test]
    fn stalled_child_is_terminated_at_deadline() {
        let child = Command::new("/bin/cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let started = Instant::now();
        assert!(read_from_child(child, Duration::from_millis(100)).is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
