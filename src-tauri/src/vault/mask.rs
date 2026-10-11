//! 凭据掩码与本机指纹（T03）。
//!
//! - 掩码：默认只显示末 4 位，短 Key 整串掩码；掩码绝不泄露末 4 位以外的原文。
//! - 指纹：带盐 HMAC-SHA256。盐存 `settings` 表（键名 [`SALT_SETTING_KEY`]），
//!   首次调用生成 32 字节随机值；指纹用于 UI 标识/去重，不能反推原文。
//!
//! 依据：docs/specs/SECURITY_PRIVACY.md 第 2 节、DATA_CONTRACTS 第 7/9 节
//! （`credential_refs.mask` / `credential_refs.fingerprint`）。

use super::{VaultError, VaultResult};
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// 掩码固定前缀（位数不反映原始长度）。
pub const MASK_PREFIX: &str = "********";
/// 长 Key 允许显示的末尾字符数。
pub const VISIBLE_TAIL: usize = 4;
/// ≤ 该字符数的 Key 视为“短 Key”，整串掩码。
pub const SHORT_KEY_MAX: usize = 8;

/// `settings` 表中保存指纹盐的键名。
pub const SALT_SETTING_KEY: &str = "vault.fingerprint_salt";
/// 指纹盐长度（字节）。
pub const SALT_LEN: usize = 32;

type HmacSha256 = Hmac<Sha256>;

/// 生成掩码：短 Key（≤ [`SHORT_KEY_MAX`] 字符）整串掩码；长 Key 只保留末 [`VISIBLE_TAIL`] 位。
pub fn mask_secret(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= SHORT_KEY_MAX {
        return "*".repeat(chars.len());
    }
    let tail: String = chars[chars.len() - VISIBLE_TAIL..].iter().collect();
    format!("{MASK_PREFIX}{tail}")
}

/// 生成 32 字节随机盐（取自 OS 熵源）。
pub fn generate_salt() -> [u8; SALT_LEN] {
    let mut salt = [0u8; SALT_LEN];
    rand::fill(&mut salt);
    salt
}

/// 带盐 HMAC-SHA256 指纹，返回 64 位小写十六进制。
///
/// HMAC 接受任意长度密钥，`new_from_slice` 对 `Hmac<Sha256>` 不可能失败；panic 信息不含秘密。
pub fn fingerprint_hex(salt: &[u8], secret: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(salt).expect("HMAC-SHA256 accepts keys of any length");
    mac.update(secret.as_bytes());
    encode_hex(&mac.finalize().into_bytes())
}

/// 从 `settings` 表读取盐；不存在时生成随机盐并写入。
///
/// 已存在的盐损坏（非合法十六进制或长度不符）时返回 [`VaultError::CorruptSalt`]，
/// **不静默重建**——重建会让该安装既有的全部指纹失效。
pub fn load_or_create_salt(db: &crate::storage::Database) -> VaultResult<[u8; SALT_LEN]> {
    use rusqlite::{params, OptionalExtension};

    let existing: Option<String> = db
        .conn()
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [SALT_SETTING_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(crate::storage::StorageError::from)?;

    match existing {
        Some(hex) => {
            let bytes = decode_hex(&hex).ok_or(VaultError::CorruptSalt)?;
            <[u8; SALT_LEN]>::try_from(bytes.as_slice()).map_err(|_| VaultError::CorruptSalt)
        }
        None => {
            let salt = generate_salt();
            db.conn()
                .execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2)",
                    params![SALT_SETTING_KEY, encode_hex(&salt)],
                )
                .map_err(crate::storage::StorageError::from)?;
            Ok(salt)
        }
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        out.push((hex_val(bytes[i])? << 4) | hex_val(bytes[i + 1])?);
        i += 2;
    }
    Some(out)
}

fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_key_is_fully_masked() {
        assert_eq!(mask_secret(""), "");
        assert_eq!(mask_secret("abcd"), "****");
        assert_eq!(mask_secret("12345678"), "********");
    }

    #[test]
    fn long_key_shows_only_last_four() {
        let secret = "sk-abcdefghijklmnop";
        let mask = mask_secret(secret);
        assert_eq!(mask, format!("{MASK_PREFIX}mnop"));
        assert!(mask.ends_with("mnop"));
        // 不得泄露末 4 位以外的原文
        assert!(!mask.contains("sk-"));
        assert!(!mask.contains("abcdefghijkl"));
        assert!(!mask.contains(secret));
    }

    #[test]
    fn fingerprint_is_deterministic_and_hex() {
        let salt = [7u8; SALT_LEN];
        let a = fingerprint_hex(&salt, "sk-x");
        assert_eq!(a, fingerprint_hex(&salt, "sk-x"));
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn different_salt_gives_different_fingerprint() {
        let a = fingerprint_hex(&[1u8; SALT_LEN], "sk-same");
        let b = fingerprint_hex(&[2u8; SALT_LEN], "sk-same");
        assert_ne!(a, b);
    }

    #[test]
    fn different_secret_gives_different_fingerprint() {
        let salt = [9u8; SALT_LEN];
        assert_ne!(
            fingerprint_hex(&salt, "sk-a"),
            fingerprint_hex(&salt, "sk-b")
        );
    }

    #[test]
    fn hex_helpers_roundtrip() {
        let bytes = [0u8, 1, 15, 16, 254, 255];
        assert_eq!(decode_hex(&encode_hex(&bytes)).unwrap(), bytes);
        assert!(decode_hex("abc").is_none());
        assert!(decode_hex("zz").is_none());
    }

    #[test]
    fn salt_is_persisted_once_and_stable() {
        let db = crate::storage::Database::open_in_memory().unwrap();
        let first = load_or_create_salt(&db).unwrap();
        let second = load_or_create_salt(&db).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            fingerprint_hex(&first, "sk-x"),
            fingerprint_hex(&second, "sk-x")
        );
    }

    #[test]
    fn separate_installations_have_different_salts() {
        let a = crate::storage::Database::open_in_memory().unwrap();
        let b = crate::storage::Database::open_in_memory().unwrap();
        assert_ne!(
            load_or_create_salt(&a).unwrap(),
            load_or_create_salt(&b).unwrap()
        );
    }

    #[test]
    fn corrupt_salt_is_reported_not_rebuilt() {
        let db = crate::storage::Database::open_in_memory().unwrap();
        db.conn()
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, 'zzzz')",
                [SALT_SETTING_KEY],
            )
            .unwrap();
        assert!(matches!(
            load_or_create_salt(&db),
            Err(VaultError::CorruptSalt)
        ));
    }
}
