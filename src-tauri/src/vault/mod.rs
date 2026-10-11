//! 系统凭据库抽象（T03）。
//!
//! 约束（docs/specs/SECURITY_PRIVACY.md 第 2 节、DATA_CONTRACTS 第 7/9 节）：
//! - 明文秘密只进系统凭据库（macOS Keychain / Windows 凭据管理器）；普通 SQLite 只存
//!   `credential_refs`（credential_id / mask / fingerprint / vault_ref），绝不落盘明文。
//! - 凭据库不可用时返回 [`VaultError::Unavailable`]，**禁止任何明文文件回退**。
//! - 仅需本次进程内短期持有秘密时，用 [`SecretString`] 封装并在用完后尽快 drop（Drop 清零）。
//! - service 名固定 [`SERVICE_NAME`]，entry 名即 `vault_ref`（凭据 UUID）。

pub mod mask;

use crate::storage::StorageError;
use zeroize::Zeroize;

/// 应用在系统凭据库中的 service 标识（与 tauri identifier 一致）。
pub const SERVICE_NAME: &str = "io.providock.app";

/// 进程内存中持有的明文秘密。Drop 时清零底层缓冲；`Debug` 恒为占位符，避免被误写入日志。
///
/// 注意：Rust `String` 在扩容/拷贝时可能留下旧副本，本类型只缩小泄露窗口，
/// 不能宣称消除了进程内存风险（SECURITY_PRIVACY 第 2 节）。
pub struct SecretString(String);

impl SecretString {
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// 仅在真正需要时读取（调用凭据库、构造请求头）；**不得**转发给日志或 IPC 响应。
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretString(<redacted>)")
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// 凭据库错误。
///
/// 刻意**不携带 keyring 底层错误**：`keyring::Error::BadEncoding(Vec<u8>)` 会持有原始密码字节，
/// 一旦经 `Debug`/日志打印即泄露秘密。这里只保留稳定的、无秘密的类别。
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("system credential store is unavailable")]
    Unavailable,
    #[error("credential not found in system credential store")]
    NotFound,
    #[error("system credential store rejected the operation")]
    Backend,
    #[error("invalid credential reference")]
    InvalidRef,
    #[error("credential fingerprint salt is missing or corrupt")]
    CorruptSalt,
    /// 内部存储错误：`source()` 保留 sqlite 细节，Display 只给类别。
    #[error("vault storage error")]
    Storage(#[from] StorageError),
}

pub type VaultResult<T> = std::result::Result<T, VaultError>;

/// 系统凭据库抽象。`vault_ref` 即凭据 UUID（`credential_refs.vault_ref`）。
pub trait Vault: Send + Sync {
    /// 写入/覆盖秘密。secret 由调用方在返回后尽快 drop。
    fn store(&self, vault_ref: &str, secret: &SecretString) -> VaultResult<()>;

    /// 读取秘密。返回的 [`SecretString`] 应尽快使用并 drop。
    fn get(&self, vault_ref: &str) -> VaultResult<SecretString>;

    /// 幂等删除：条目不存在也返回 `Ok`（清理路径不因缺条目失败）。
    fn delete(&self, vault_ref: &str) -> VaultResult<()>;

    fn exists(&self, vault_ref: &str) -> VaultResult<bool>;
}

/// 只接受 UUID 形态的引用（ASCII 字母/数字/`-`/`_`），避免奇异 entry 名与路径注入。
fn validate_ref(vault_ref: &str) -> VaultResult<()> {
    if vault_ref.is_empty() || vault_ref.len() > 128 {
        return Err(VaultError::InvalidRef);
    }
    if !vault_ref
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(VaultError::InvalidRef);
    }
    Ok(())
}

/// keyring 错误 → 本模块错误。丢弃底层错误，杜绝 `BadEncoding` 的字节经 Debug 外泄。
fn map_keyring_error(err: keyring::Error) -> VaultError {
    match err {
        keyring::Error::NoEntry => VaultError::NotFound,
        keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => {
            VaultError::Unavailable
        }
        _ => VaultError::Backend,
    }
}

/// 系统凭据库实现（macOS Keychain / Windows 凭据管理器）。
#[derive(Debug, Default)]
pub struct KeyringVault;

impl KeyringVault {
    pub fn new() -> Self {
        Self
    }

    fn entry(vault_ref: &str) -> VaultResult<keyring::Entry> {
        validate_ref(vault_ref)?;
        keyring::Entry::new(SERVICE_NAME, vault_ref).map_err(map_keyring_error)
    }
}

impl Vault for KeyringVault {
    fn store(&self, vault_ref: &str, secret: &SecretString) -> VaultResult<()> {
        let entry = Self::entry(vault_ref)?;
        entry
            .set_password(secret.expose())
            .map_err(map_keyring_error)
    }

    fn get(&self, vault_ref: &str) -> VaultResult<SecretString> {
        let entry = Self::entry(vault_ref)?;
        let password = entry.get_password().map_err(map_keyring_error)?;
        Ok(SecretString::new(password))
    }

    fn delete(&self, vault_ref: &str) -> VaultResult<()> {
        let entry = Self::entry(vault_ref)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(map_keyring_error(err)),
        }
    }

    fn exists(&self, vault_ref: &str) -> VaultResult<bool> {
        let entry = Self::entry(vault_ref)?;
        match entry.get_password() {
            Ok(secret) => {
                // 立即用 SecretString 包裹并 drop，避免临时明文悬挂
                drop(SecretString::new(secret));
                Ok(true)
            }
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(err) => Err(map_keyring_error(err)),
        }
    }
}

/// 进程内内存凭据库。
///
/// ⚠ **仅用于测试，绝不可用于生产**：内容不加密、不持久、进程退出即丢失，
/// 不满足产品“秘密只存系统凭据库”的要求。生产装配必须使用 [`KeyringVault`]。
pub struct MemoryVault {
    entries: std::sync::Mutex<std::collections::HashMap<String, SecretString>>,
}

impl MemoryVault {
    pub fn new() -> Self {
        Self {
            entries: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl Default for MemoryVault {
    fn default() -> Self {
        Self::new()
    }
}

impl Vault for MemoryVault {
    fn store(&self, vault_ref: &str, secret: &SecretString) -> VaultResult<()> {
        validate_ref(vault_ref)?;
        let mut entries = self.entries.lock().map_err(|_| VaultError::Backend)?;
        entries.insert(vault_ref.to_owned(), SecretString::new(secret.expose()));
        Ok(())
    }

    fn get(&self, vault_ref: &str) -> VaultResult<SecretString> {
        validate_ref(vault_ref)?;
        let entries = self.entries.lock().map_err(|_| VaultError::Backend)?;
        entries
            .get(vault_ref)
            .map(|s| SecretString::new(s.expose()))
            .ok_or(VaultError::NotFound)
    }

    fn delete(&self, vault_ref: &str) -> VaultResult<()> {
        validate_ref(vault_ref)?;
        let mut entries = self.entries.lock().map_err(|_| VaultError::Backend)?;
        entries.remove(vault_ref);
        Ok(())
    }

    fn exists(&self, vault_ref: &str) -> VaultResult<bool> {
        validate_ref(vault_ref)?;
        let entries = self.entries.lock().map_err(|_| VaultError::Backend)?;
        Ok(entries.contains_key(vault_ref))
    }
}

// ---------------------------------------------------------------------------
// T09 前端 IPC 接口建议（本步不实现 command，仅给出契约，供接入时参考）：
//
// #[tauri::command]
// async fn credential_store(
//     state: tauri::State<'_, AppState>,
//     account_id: String,
//     credential_type: String,   // api_key / management_key / oauth_handle / local_server_token / ak_sk
//     secret: String,            // 前端仅在此刻传入，收到后立即 SecretString::new(secret) 并尽快 drop
// ) -> Result<CredentialRefDto, String>;
//   - 生成 credential_id（UUID）；vault_ref = credential_id；
//   - KeyringVault.store(vault_ref, secret)，失败返回 VaultError::Unavailable（前端提示“仅本次内存使用/未保存”）；
//   - 用 mask::load_or_create_salt(&db) + mask::fingerprint_hex 计算 fingerprint，写 credential_refs；
//   - 响应只含 credential_id / mask / fingerprint / capabilities，**绝不回显 secret**。
//
// #[tauri::command]
// async fn credential_delete(state, credential_id: String) -> Result<(), String>;
//   - 后端由 credential_id 查 credential_refs.vault_ref，再 KeyringVault.delete；随后删除引用行；
//   - 不接受前端直接传 vault_ref 或系统路径。
//
// #[tauri::command]
// async fn credential_status(state, credential_id: String) -> Result<CredentialStatusDto, String>;
//   - 只返回 exists/类型/掩码/指纹，不返回明文；
//   - 真正需要请求时在后端 get，构造请求头后立即 drop SecretString，缩短生命周期。
//
// 另：绝不把 secret 写入 URL query、日志、异常文本或剪贴板；网关/账户请求失败信息须经
// services::redact 处理（见 src/services/redact.rs）。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const REF: &str = "11111111-2222-3333-4444-555555555555";

    #[test]
    fn secret_string_debug_does_not_leak() {
        let secret = SecretString::new("sk-supersecret-value");
        let dbg = format!("{secret:?}");
        assert_eq!(dbg, "SecretString(<redacted>)");
        assert!(!dbg.contains("supersecret"));
    }

    #[test]
    fn invalid_reference_is_rejected() {
        let vault = MemoryVault::new();
        let secret = SecretString::new("sk-x");
        assert!(matches!(
            vault.store("", &secret),
            Err(VaultError::InvalidRef)
        ));
        assert!(matches!(
            vault.store("../../etc/passwd", &secret),
            Err(VaultError::InvalidRef)
        ));
        assert!(matches!(
            vault.store("has space", &secret),
            Err(VaultError::InvalidRef)
        ));
    }

    #[test]
    fn memory_vault_roundtrip() {
        let vault = MemoryVault::new();
        assert!(!vault.exists(REF).unwrap());
        assert!(matches!(vault.get(REF), Err(VaultError::NotFound)));

        {
            let secret = SecretString::new("sk-test-0000");
            vault.store(REF, &secret).unwrap();
        }
        assert!(vault.exists(REF).unwrap());
        {
            let got = vault.get(REF).unwrap();
            assert_eq!(got.expose(), "sk-test-0000");
        }

        vault.delete(REF).unwrap();
        assert!(!vault.exists(REF).unwrap());
        // 幂等删除
        vault.delete(REF).unwrap();
    }

    /// 真实钥匙串往返。CI / 无钥匙串环境不跑：`cargo test -- --ignored`。
    #[test]
    #[ignore = "requires a real OS credential store"]
    fn keyring_roundtrip_real_store() {
        let vault = KeyringVault::new();
        let reference = format!("it-{}", std::process::id());
        drop(vault.delete(&reference));

        let secret = SecretString::new("sk-synthetic-0000");
        vault.store(&reference, &secret).unwrap();
        {
            let got = vault.get(&reference).unwrap();
            assert_eq!(got.expose(), "sk-synthetic-0000");
        }
        assert!(vault.exists(&reference).unwrap());
        vault.delete(&reference).unwrap();
        assert!(!vault.exists(&reference).unwrap());
    }
}
