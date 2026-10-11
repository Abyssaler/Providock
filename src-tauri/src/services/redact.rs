//! 诊断脱敏（T03）。
//!
//! 诊断 / 日志中不得出现本机绝对路径、疑似密钥串或超长正文
//! （docs/specs/SECURITY_PRIVACY.md 第 2、5、7 节；DATA_CONTRACTS 第 9 节 `diagnostic_events`）。
//! 提供三层处理，可单独调用，也可用 [`redact`] 组合：
//! 1. [`strip_secrets`]：剥离 Bearer / `sk-` 等疑似密钥、`key=value` 赋值、PEM 私钥块；
//! 2. [`anonymize_paths`]：把绝对路径替换为稳定匿名文件 ID `<file:xxxxxxxxxxxxxxxx>`；
//! 3. [`truncate`]：截断超长消息。
//!
//! 说明：匿名 ID 是路径字符串的稳定哈希，便于同一台机器、跨运行识别同一文件；
//! 需要降低离线枚举风险时可用 [`anon_file_id_salted`] 配合本机盐。

use sha2::{Digest, Sha256};

/// 诊断消息保留的最大字符数，超出部分截断。
pub const MAX_MESSAGE_CHARS: usize = 2048;

const FILE_ID_BYTES: usize = 8;
const FILE_ID_DOMAIN: &[u8] = b"providock.file.v1\0";
const REDACTED: &str = "<REDACTED>";
const REDACTED_PEM: &str = "<REDACTED-PRIVATE-KEY>";

/// 完整脱敏管线：剥离密钥 → 匿名化路径 → 截断。
pub fn redact(text: &str) -> String {
    truncate(&anonymize_paths(&strip_secrets(text)), MAX_MESSAGE_CHARS)
}

/// 稳定匿名文件 ID：同一路径字符串在所有运行中映射到同一 ID（SHA-256 前 8 字节十六进制）。
pub fn anon_file_id(path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(FILE_ID_DOMAIN);
    hasher.update(path.as_bytes());
    hex_of(&hasher.finalize()[..FILE_ID_BYTES])
}

/// 带盐匿名文件 ID：提供本机盐时使用，降低对路径的离线枚举风险。
pub fn anon_file_id_salted(salt: &[u8], path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt);
    hasher.update(FILE_ID_DOMAIN);
    hasher.update(path.as_bytes());
    hex_of(&hasher.finalize()[..FILE_ID_BYTES])
}

/// 用匿名 ID 替换单条已知路径（例如 DB 中 `sources.root_path`）。
pub fn redact_path(path: &str) -> String {
    format!("<file:{}>", anon_file_id(path))
}

/// 剥离疑似密钥串：PEM 私钥块、`Bearer <token>`、已知前缀密钥、`key=value` 敏感赋值。
pub fn strip_secrets(input: &str) -> String {
    let without_pem = redact_pem_blocks(input);
    let without_bearer = redact_bearer(&without_pem);
    let without_prefixed = redact_prefixed_tokens(&without_bearer);
    redact_sensitive_assignments(&without_prefixed)
}

/// 把文本中的绝对路径替换为匿名文件 ID。URL（`https://...`）不会被误判。
pub fn anonymize_paths(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        if path_start_at(&chars, i) {
            let mut end = scan_path_end(&chars, i);
            // 去掉紧随路径的冒号（如 "path: error"），使 ID 与裸路径一致
            while end > i && chars[end - 1] == ':' {
                end -= 1;
            }
            if end - i >= 2 {
                let path: String = chars[i..end].iter().collect();
                out.push_str("<file:");
                out.push_str(&anon_file_id(&path));
                out.push('>');
                i = end;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 截断到 `max_chars` 个字符（按 Unicode 标量，不切断多字节序列），并追加剧断标记。
pub fn truncate(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut out = String::with_capacity(max_chars + 16);
    for (count, ch) in text.chars().enumerate() {
        if count == max_chars {
            out.push('…');
            out.push_str("[truncated]");
            return out;
        }
        out.push(ch);
    }
    out
}

// ---------------------------------------------------------------------------
// 密钥剥离
// ---------------------------------------------------------------------------

fn is_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '+' | '/' | '~')
}

fn is_boundary(chars: &[char], i: usize) -> bool {
    i == 0 || !is_token_char(chars[i - 1])
}

fn matches_word_ci(chars: &[char], i: usize, word_lower: &[char]) -> bool {
    if i + word_lower.len() > chars.len() {
        return false;
    }
    chars[i..i + word_lower.len()]
        .iter()
        .zip(word_lower)
        .all(|(actual, expected)| actual.to_ascii_lowercase() == *expected)
}

/// `-----BEGIN ... -----` 到对应 `-----END ... -----` 的整块（含私钥/证书）替换为占位符。
fn redact_pem_blocks(input: &str) -> String {
    if !input.contains("-----BEGIN") {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(begin) = rest.find("-----BEGIN") {
        out.push_str(&rest[..begin]);
        let after = &rest[begin..];
        match after.find("-----END") {
            Some(end_rel) => {
                let tail = &after[end_rel..]; // "-----END..."
                let close = tail[5..] // 跳过 "-----"，再找 END 标签后的收尾 "-----"
                    .find("-----")
                    .map(|pos| end_rel + 5 + pos + 5)
                    .unwrap_or(after.len());
                out.push_str(REDACTED_PEM);
                rest = &after[close.min(after.len())..];
            }
            None => {
                out.push_str(REDACTED_PEM);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// `Bearer <token>`（大小写不敏感，须有空白分隔）中的 token 替换为占位符。
fn redact_bearer(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let word: Vec<char> = "bearer".chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        if is_boundary(&chars, i) && matches_word_ci(&chars, i, &word) {
            let mut j = i + word.len();
            let ws_start = j;
            while j < chars.len() && (chars[j] == ' ' || chars[j] == '\t') {
                j += 1;
            }
            if j > ws_start && j < chars.len() && is_token_char(chars[j]) {
                out.extend(chars[i..j].iter());
                while j < chars.len() && is_token_char(chars[j]) {
                    j += 1;
                }
                out.push_str(REDACTED);
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 已知密钥前缀后的 token 替换为占位符（保留前缀便于识别类型）。
const SECRET_PREFIXES: &[&str] = &[
    "sk-",
    "sk_",
    "rk-",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxr-",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "github_pat_",
    "glpat-",
    "AIza",
    "AKIA",
    "ASIA",
    "ya29.",
    "eyJ",
];

fn redact_prefixed_tokens(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        if is_boundary(&chars, i) {
            if let Some(prefix) = SECRET_PREFIXES.iter().find(|prefix| {
                let pc: Vec<char> = prefix.chars().collect();
                i + pc.len() <= chars.len() && chars[i..i + pc.len()] == pc[..]
            }) {
                let plen = prefix.chars().count();
                let mut j = i + plen;
                let value_start = j;
                while j < chars.len() && is_token_char(chars[j]) {
                    j += 1;
                }
                if j > value_start {
                    out.extend(chars[i..i + plen].iter());
                    out.push_str(REDACTED);
                    i = j;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 敏感键名到值的赋值（`key=value` / `"key": "value"`）。
const SENSITIVE_KEYS: &[&str] = &[
    "authorization",
    "authorisation",
    "api_key",
    "apikey",
    "api-key",
    "access_token",
    "refresh_token",
    "id_token",
    "auth_token",
    "token",
    "client_secret",
    "app_secret",
    "secret",
    "password",
    "passwd",
    "pwd",
    "private_key",
    "privatekey",
    "x-api-key",
    "x_api_key",
];

fn redact_sensitive_assignments(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    'outer: while i < chars.len() {
        if is_boundary(&chars, i) {
            for key in SENSITIVE_KEYS {
                let kc: Vec<char> = key.chars().collect();
                if i + kc.len() > chars.len() || !matches_word_ci(&chars, i, &kc) {
                    continue;
                }
                let after_key = i + kc.len();
                // 键名后必须是分隔符，避免匹配更长的标识符
                if after_key < chars.len() && is_token_char(chars[after_key]) {
                    continue;
                }
                if let Some((end, value_start)) = parse_assignment_value(&chars, after_key) {
                    out.extend(chars[i..value_start].iter());
                    out.push_str(REDACTED);
                    i = end;
                    continue 'outer;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 从分隔符起解析赋值右侧的值。返回 `(结束索引, 值起始索引)`；不是赋值则 `None`。
fn parse_assignment_value(chars: &[char], mut j: usize) -> Option<(usize, usize)> {
    if j < chars.len() && matches!(chars[j], '"' | '\'') {
        j += 1;
    }
    while j < chars.len() && chars[j] == ' ' {
        j += 1;
    }
    if j >= chars.len() || (chars[j] != ':' && chars[j] != '=') {
        return None;
    }
    j += 1;
    while j < chars.len() && chars[j] == ' ' {
        j += 1;
    }

    let quote = if j < chars.len() && matches!(chars[j], '"' | '\'') {
        let quote = chars[j];
        j += 1;
        Some(quote)
    } else {
        None
    };
    let value_start = j;

    match quote {
        Some(quote) => {
            while j < chars.len() && chars[j] != quote {
                j += 1;
            }
            (j > value_start).then_some((j, value_start))
        }
        None => {
            while j < chars.len() && is_token_char(chars[j]) {
                j += 1;
            }
            (j > value_start).then_some((j, value_start))
        }
    }
}

// ---------------------------------------------------------------------------
// 路径匿名化
// ---------------------------------------------------------------------------

fn is_path_terminator(ch: char) -> bool {
    matches!(
        ch,
        '"' | '\'' | '`' | '<' | '>' | '|' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '='
    )
}

fn path_boundary(chars: &[char], i: usize) -> bool {
    i == 0 || chars[i - 1].is_whitespace() || is_path_terminator(chars[i - 1])
}

/// 判断 `i` 处是否为绝对路径起点（POSIX `/`、Windows 盘符 `C:\`、UNC `\\`）。
fn path_start_at(chars: &[char], i: usize) -> bool {
    if !path_boundary(chars, i) {
        return false;
    }
    let ch = chars[i];
    if ch == '/' {
        return true;
    }
    if ch == '\\' && i + 1 < chars.len() && chars[i + 1] == '\\' {
        return true;
    }
    ch.is_ascii_alphabetic()
        && i + 2 < chars.len()
        && chars[i + 1] == ':'
        && (chars[i + 2] == '\\' || chars[i + 2] == '/')
}

/// 路径结束位置。遇到空白时，仅当后续片段仍像路径延续（含分隔符）才跨越空格，
/// 以便处理 macOS 常见的带空格路径（如 `Application Support`），同时不吞并后续散文。
fn scan_path_end(chars: &[char], start: usize) -> usize {
    let mut j = start;
    while j < chars.len() {
        let ch = chars[j];
        if ch.is_whitespace() {
            let mut k = j;
            while k < chars.len() && chars[k].is_whitespace() {
                k += 1;
            }
            if k < chars.len() && !is_path_terminator(chars[k]) && run_continues_path(&chars[k..]) {
                j = k;
                continue;
            }
            break;
        }
        if is_path_terminator(ch) {
            break;
        }
        j += 1;
    }
    j
}

fn run_continues_path(run: &[char]) -> bool {
    for &ch in run {
        if ch.is_whitespace() || is_path_terminator(ch) {
            break;
        }
        if ch == '/' || ch == '\\' {
            return true;
        }
    }
    false
}

fn hex_of(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_anonymization_is_stable() {
        let path = "/Users/alice/Library/Application Support/agent/session.jsonl";
        let a = redact(&format!("failed to read {path}"));
        let b = redact(&format!("failed to read {path}"));
        assert_eq!(a, b);
        assert!(a.contains("<file:"));
        assert!(!a.contains("alice"));
        assert!(!a.contains("session.jsonl"));
        assert_eq!(anon_file_id(path), anon_file_id(path));
        assert_ne!(anon_file_id(path), anon_file_id("/Users/bob/other"));
    }

    #[test]
    fn windows_path_is_anonymized() {
        let result = redact("open C:\\Users\\alice\\AppData\\Roaming\\x.jsonl failed");
        assert!(result.contains("<file:"));
        assert!(!result.contains("alice"));
        assert!(!result.contains("x.jsonl"));
    }

    #[test]
    fn url_is_not_mistaken_for_path() {
        let result = redact("GET https://api.example.com/v1/usage returned 401");
        assert!(!result.contains("<file:"), "URL 不应被当作绝对路径: {result}");
    }

    #[test]
    fn bearer_token_is_stripped() {
        let result = strip_secrets("Authorization: Bearer sk-abcdef1234567890");
        assert!(!result.contains("abcdef1234567890"));
        assert!(!result.contains("sk-abc"));
        assert!(result.contains(REDACTED));
    }

    #[test]
    fn prefixed_key_is_stripped() {
        let result = strip_secrets("using key sk-proj-0123456789abcdef and done");
        assert!(!result.contains("0123456789abcdef"));
        assert!(result.contains(REDACTED));
    }

    #[test]
    fn assignment_secrets_are_stripped() {
        let bare = strip_secrets("api_key=AKIAIOSFODNN7EXAMPLE");
        assert!(!bare.contains("AKIAIOSFODNN7EXAMPLE"));

        let quoted = strip_secrets(r#""password": "hunter2-secret""#);
        assert!(!quoted.contains("hunter2-secret"));

        let token = strip_secrets("access_token=abcDEF123456");
        assert!(!token.contains("abcDEF123456"));
    }

    #[test]
    fn pem_private_key_is_removed() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIabc123\n-----END RSA PRIVATE KEY-----";
        let result = strip_secrets(pem);
        assert!(!result.contains("MIIabc123"));
        assert!(result.contains(REDACTED_PEM));
    }

    #[test]
    fn long_message_is_truncated() {
        let long = "x".repeat(5000);
        let result = redact(&long);
        assert!(result.contains("[truncated]"));
        assert!(result.chars().count() < 5000);
        assert!(result.chars().count() <= MAX_MESSAGE_CHARS + "…[truncated]".chars().count());
    }

    #[test]
    fn short_message_is_untouched() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("héllo", 3), "hél…[truncated]");
    }

    #[test]
    fn salted_and_plain_ids_differ() {
        let path = "/tmp/file.jsonl";
        assert_ne!(anon_file_id(path), anon_file_id_salted(b"salt", path));
        assert_eq!(
            anon_file_id_salted(b"salt", path),
            anon_file_id_salted(b"salt", path)
        );
    }
}
