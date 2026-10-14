//! 独立账户领域（T09）：账户 / 凭据引用 / 钱包链接 / 额度快照 / 失败与过期模型。
//!
//! 契约依据：
//! - `docs/specs/DATA_CONTRACTS.md` 第 6 节（费用）、第 7 节（账户领域）、
//!   第 8 节（QuotaWindow）、第 9 节（表清单）
//! - `docs/specs/ACCOUNT_ADAPTERS.md` 第 1 节（能力原则）、第 4 节（凭据与账户建模）、
//!   第 5 节（调度与失败）、第 6 节（展示规则）
//! - `docs/specs/SECURITY_PRIVACY.md` 第 2 节（凭据）
//!
//! 边界：
//! - 账户域与用量账本完全分域：本模块只读写账户侧的表，账户可以没有任何 Agent 绑定
//!   （文件末尾有结构性测试守住这一条）。
//! - 明文秘密只在 [`SecretString`] 中短暂停留；本模块任何返回值、错误与日志都不回显秘密。
//! - HTTP 请求、host 白名单与平台解析属 T10；本模块只提供落库、查询、过期与失败建模。
//! - 表结构由 `storage::migrations` 定义，本模块不新增表、不改迁移。
//!
//! 时间口径：`queried_at` 是本机请求时间，`provider_as_of` 是平台数据时间，两者分开存，
//! 不用本机刷新时间冒充平台刚结算（DEVELOPMENT_HANDBOOK 第 7 节）。

use crate::domain::decimal::decimal_add;
use crate::domain::{Money, Quality};
use crate::storage::{Database, StorageError};
use crate::vault::{mask, SecretString, Vault, VaultError};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AccountsError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Vault(#[from] VaultError),
    #[error("account not found: {0}")]
    AccountNotFound(String),
    #[error("account is soft-deleted: {0}")]
    AccountDeleted(String),
    #[error("credential not found: {0}")]
    CredentialNotFound(String),
    #[error("credential {credential_id} does not belong to account {account_id}")]
    CredentialAccountMismatch {
        credential_id: String,
        account_id: String,
    },
    #[error("snapshot not found: {0}")]
    SnapshotNotFound(i64),
    #[error("secret must not be empty")]
    EmptySecret,
    #[error("field must not be empty: {0}")]
    EmptyField(&'static str),
    #[error("invalid money value: {0}")]
    InvalidMoney(String),
    #[error("invalid decimal value for {field}: {value}")]
    InvalidDecimal { field: &'static str, value: String },
    #[error("quota window needs a non-empty window_id")]
    MissingWindowId,
    #[error("quota window appears twice in one snapshot: {0}")]
    DuplicateWindowId(String),
    #[error("usage metric appears twice in one snapshot: {0}")]
    DuplicateUsageMetric(String),
    #[error("HTTP {status} without a business error code is not a failure")]
    NotAFailure { status: u16 },
    #[error("unknown {kind} value in database: {value}")]
    CorruptRow { kind: &'static str, value: String },
}

pub type Result<T> = std::result::Result<T, AccountsError>;

// ---------------------------------------------------------------------------
// 契约枚举
// ---------------------------------------------------------------------------

/// 契约枚举：`as_str` 是唯一 wire 形态，DB 与 JSON 都用它往返。
macro_rules! wire_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $wire:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $($variant),+ }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $wire),+ }
            }
            /// 未知取值返回 `None`，调用方按 [`AccountsError::CorruptRow`] 处理，不猜默认值。
            pub fn parse(s: &str) -> Option<Self> {
                match s { $($wire => Some(Self::$variant),)+ _ => None }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
                let raw = String::deserialize(d)?;
                Self::parse(&raw).ok_or_else(|| {
                    serde::de::Error::custom(format!("unknown {} value: {raw}", stringify!($name)))
                })
            }
        }
    };
}

wire_enum! {
    /// 快照状态（DATA_CONTRACTS 第 8 节，取值固定）。
    ///
    /// 无支持能力（`unsupported`）与暂时失败（`unavailable` / `stale`）是不同状态。
    /// `no_plan`（2026-10-12 新增）：平台确认该账户**没有对应套餐**（如 GLM 返回
    /// “当前用户不存在coding plan”）——不是凭据失效也不是暂时失败，UI 显示
    /// “无 Coding Plan 套餐”而不是通用错误。
    SnapshotStatus {
        Ok = "ok",
        Stale = "stale",
        Refreshing = "refreshing",
        AuthRequired = "auth_required",
        RateLimited = "rate_limited",
        Unsupported = "unsupported",
        Incompatible = "incompatible",
        Unavailable = "unavailable",
        NoPlan = "no_plan",
    }
}

/// 可用型状态，写入成功快照时使用。
const SNAPSHOT_OK: &str = "ok";

wire_enum! {
    /// 凭据类型（ACCOUNT_ADAPTERS 第 4 节）。
    CredentialType {
        ApiKey = "api_key",
        ManagementKey = "management_key",
        OauthHandle = "oauth_handle",
        LocalServerToken = "local_server_token",
        AkSk = "ak_sk",
    }
}

wire_enum! {
    /// 账户配置类型（`accounts.config_kind`）。
    ConfigKind {
        ApiPayg = "api_payg",
        CodingPlan = "coding_plan",
        Both = "both",
    }
}

wire_enum! {
    /// 余额种类（DATA_CONTRACTS 第 7 节）。
    BalanceKind {
        Paid = "paid",
        Grant = "grant",
        Available = "available",
        Credit = "credit",
    }
}

wire_enum! {
    /// 额度窗口单位（DATA_CONTRACTS 第 8 节）。
    QuotaUnit {
        Tokens = "tokens",
        Requests = "requests",
        Credits = "credits",
        Currency = "currency",
        Percent = "percent",
        VendorUnit = "vendor_unit",
    }
}

wire_enum! {
    /// 窗口周期类型（DATA_CONTRACTS 第 8 节）。
    PeriodKind {
        Rolling = "rolling",
        Calendar = "calendar",
        Fixed = "fixed",
        Unknown = "unknown",
    }
}

// ---------------------------------------------------------------------------
// 标识与时间
// ---------------------------------------------------------------------------

/// 时间戳 + 随机数自实现的 UUIDv4 形态标识（不引入 uuid crate）。
///
/// 前 6 字节是本机毫秒时间戳（可读、单调递增），其余 74 位取自 OS 熵源。
/// 形态与 [`crate::vault`] 的 `vault_ref` 校验（十六进制 + `-`）一致。
pub fn new_uuid() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let mut bytes = [0u8; 16];
    bytes[..6].copy_from_slice(&millis.to_be_bytes()[2..8]);
    bytes[6..14].copy_from_slice(&rand::random::<u64>().to_be_bytes());
    bytes[14..].copy_from_slice(&rand::random::<u16>().to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant
    hex_uuid(&bytes)
}

fn hex_uuid(bytes: &[u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(36);
    for (i, byte) in bytes.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// 本机当前时间（UTC RFC3339），用于 `queried_at`。
///
/// 平台数据时间用 [`NewSnapshot::provider_as_of`]，两者不得互相顶替。
pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| String::from("1970-01-01T00:00:00Z"))
}

fn validate_decimal(field: &'static str, value: &str) -> Result<()> {
    if decimal_add(value, "0").is_none() {
        return Err(AccountsError::InvalidDecimal {
            field,
            value: value.to_owned(),
        });
    }
    Ok(())
}

/// `1 - ratio`：比例之间可以换算，绝对量不可以（契约第 8 节）。
fn one_minus(ratio: &str) -> Option<String> {
    let negated = match ratio.strip_prefix('-') {
        Some(rest) => rest.to_owned(),
        None => format!("-{ratio}"),
    };
    decimal_add("1", &negated)
}

// ---------------------------------------------------------------------------
// 账户
// ---------------------------------------------------------------------------

/// 账户（DATA_CONTRACTS 第 7 节）。字段名与契约一致，不做 camelCase 转换。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub account_id: String,
    pub platform: String,
    pub region: Option<String>,
    pub display_name: String,
    pub vendor_user_id: Option<String>,
    /// organization / project 等范围。
    pub scope: Option<String>,
    pub config_kind: ConfigKind,
    pub created_at: String,
    /// 软删除时间；`None` = 有效账户。
    pub deleted_at: Option<String>,
}

impl Account {
    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }
}

/// 新建账户的输入。账户不携带任何 Agent 绑定。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewAccount {
    pub platform: String,
    pub region: Option<String>,
    pub display_name: String,
    pub vendor_user_id: Option<String>,
    pub scope: Option<String>,
    pub config_kind: ConfigKind,
}

impl NewAccount {
    pub fn new(
        platform: impl Into<String>,
        display_name: impl Into<String>,
        config_kind: ConfigKind,
    ) -> Self {
        Self {
            platform: platform.into(),
            region: None,
            display_name: display_name.into(),
            vendor_user_id: None,
            scope: None,
            config_kind,
        }
    }
}

/// 账户字段更新。`Some` = 覆盖；`None` = 保持不变。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountPatch {
    pub region: Option<String>,
    pub display_name: Option<String>,
    pub vendor_user_id: Option<String>,
    pub scope: Option<String>,
    pub config_kind: Option<ConfigKind>,
}

const ACCOUNT_COLUMNS: &str = "account_id, platform, region, display_name, vendor_user_id, scope, \
                               config_kind, created_at, deleted_at";

fn row_to_account(row: &rusqlite::Row<'_>) -> Result<Account> {
    let config_raw: String = row.get(6)?;
    Ok(Account {
        account_id: row.get(0)?,
        platform: row.get(1)?,
        region: row.get(2)?,
        display_name: row.get(3)?,
        vendor_user_id: row.get(4)?,
        scope: row.get(5)?,
        config_kind: ConfigKind::parse(&config_raw).ok_or(AccountsError::CorruptRow {
            kind: "config_kind",
            value: config_raw,
        })?,
        created_at: row.get(7)?,
        deleted_at: row.get(8)?,
    })
}

pub fn account_create(db: &Database, draft: &NewAccount) -> Result<Account> {
    if draft.platform.trim().is_empty() {
        return Err(AccountsError::EmptyField("platform"));
    }
    if draft.display_name.trim().is_empty() {
        return Err(AccountsError::EmptyField("display_name"));
    }
    let account_id = new_uuid();
    db.conn().execute(
        "INSERT INTO accounts (account_id, platform, region, display_name, vendor_user_id, scope, config_kind)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            account_id,
            draft.platform,
            draft.region,
            draft.display_name,
            draft.vendor_user_id,
            draft.scope,
            draft.config_kind.as_str(),
        ],
    )?;
    account_get(db, &account_id)?.ok_or(AccountsError::AccountNotFound(account_id))
}

/// 读取账户（含软删除行，调用方按 `deleted_at` 判断）。
pub fn account_get(db: &Database, account_id: &str) -> Result<Option<Account>> {
    let sql = format!("SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE account_id = ?1");
    let mut stmt = db.conn().prepare(&sql)?;
    let mut rows = stmt.query(params![account_id])?;
    match rows.next()? {
        Some(row) => Ok(Some(row_to_account(row)?)),
        None => Ok(None),
    }
}

/// 账户列表。`include_deleted = false` 时只返回有效账户，软删除行默认隐藏。
pub fn accounts_list(db: &Database, include_deleted: bool) -> Result<Vec<Account>> {
    let sql = format!(
        "SELECT {ACCOUNT_COLUMNS} FROM accounts WHERE (?1 = 1 OR deleted_at IS NULL)
         ORDER BY created_at, account_id"
    );
    let mut stmt = db.conn().prepare(&sql)?;
    let mut rows = stmt.query(params![i64::from(include_deleted)])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(row_to_account(row)?);
    }
    Ok(out)
}

pub fn account_update(db: &Database, account_id: &str, patch: &AccountPatch) -> Result<Account> {
    ensure_active_account(db, account_id)?;
    if let Some(name) = patch.display_name.as_deref() {
        if name.trim().is_empty() {
            return Err(AccountsError::EmptyField("display_name"));
        }
    }
    db.conn().execute(
        "UPDATE accounts SET
            region          = COALESCE(?2, region),
            display_name    = COALESCE(?3, display_name),
            vendor_user_id  = COALESCE(?4, vendor_user_id),
            scope           = COALESCE(?5, scope),
            config_kind     = COALESCE(?6, config_kind)
         WHERE account_id = ?1",
        params![
            account_id,
            patch.region,
            patch.display_name,
            patch.vendor_user_id,
            patch.scope,
            patch.config_kind.map(ConfigKind::as_str),
        ],
    )?;
    account_get(db, account_id)?
        .ok_or_else(|| AccountsError::AccountNotFound(account_id.to_owned()))
}

fn ensure_active_account(db: &Database, account_id: &str) -> Result<()> {
    match account_get(db, account_id)? {
        None => Err(AccountsError::AccountNotFound(account_id.to_owned())),
        Some(account) if account.is_deleted() => {
            Err(AccountsError::AccountDeleted(account_id.to_owned()))
        }
        Some(_) => Ok(()),
    }
}

/// 凭据清理结果（删除账户时用于说明共享引用是否保留）。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CredentialCleanup {
    /// 删除的 `credential_refs` 行数。
    pub refs_removed: usize,
    /// 从配置的凭据库删除的条目数。
    pub vault_entries_removed: usize,
    /// 因仍被其他引用共享而保留的凭据库条目数。
    pub shared_refs_kept: usize,
}

/// 软删除账户：行保留（历史快照与时间信息不丢），`deleted_at` 置位；
/// 归属凭据按引用清理（共享引用先检查）。删除本地条目不撤销厂商 Key。
pub fn account_delete(
    db: &mut Database,
    vault: &dyn Vault,
    account_id: &str,
) -> Result<CredentialCleanup> {
    let account = account_get(db, account_id)?
        .ok_or_else(|| AccountsError::AccountNotFound(account_id.to_owned()))?;
    if !account.is_deleted() {
        db.conn().execute(
            "UPDATE accounts SET deleted_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE account_id = ?1 AND deleted_at IS NULL",
            params![account_id],
        )?;
    }
    cleanup_account_credentials(db, vault, account_id)
}

/// 清理某账户的全部凭据引用与对应的配置的凭据库条目。
///
/// 共享引用检查：`vault_ref` 若仍被其它 `credential_refs` 行引用，则**不删除**凭据库条目
/// （当前模型一个凭据只属一个账户，检查函数仍保留，便于以后多账户共享凭据）。
pub fn cleanup_account_credentials(
    db: &mut Database,
    vault: &dyn Vault,
    account_id: &str,
) -> Result<CredentialCleanup> {
    let mut cleanup = CredentialCleanup::default();
    let refs: Vec<(String, String)> = {
        let tx = db.transaction()?;
        let refs = {
            let mut stmt = tx.prepare(
                "SELECT credential_id, vault_ref FROM credential_refs WHERE account_id = ?1",
            )?;
            let mut rows = stmt.query(params![account_id])?;
            let mut collected = Vec::new();
            while let Some(row) = rows.next()? {
                collected.push((row.get(0)?, row.get(1)?));
            }
            collected
        };
        // 快照的 credential_id 置空后再删引用行：保留快照本身与时间信息，
        // 同时避免外键阻止凭据清理。
        tx.execute(
            "UPDATE account_snapshots SET credential_id = NULL
             WHERE credential_id IN (SELECT credential_id FROM credential_refs WHERE account_id = ?1)",
            params![account_id],
        )?;
        tx.execute(
            "DELETE FROM credential_refs WHERE account_id = ?1",
            params![account_id],
        )?;
        tx.commit()?;
        refs
    };
    cleanup.refs_removed = refs.len();
    for (credential_id, vault_ref) in refs {
        if vault_ref_is_shared(db, &vault_ref, &credential_id)? {
            cleanup.shared_refs_kept += 1;
            continue;
        }
        // 凭据库不在 SQLite 事务内：本地引用已删除后再尽力清理条目；
        // 凭据库锁定不应把“删除账户”变成不可完成的动作。
        if vault.delete(&vault_ref).is_ok() {
            cleanup.vault_entries_removed += 1;
        }
    }
    Ok(cleanup)
}

/// `vault_ref` 是否被除 `exclude_credential_id` 之外的行引用（共享引用检查）。
fn vault_ref_is_shared(
    db: &Database,
    vault_ref: &str,
    exclude_credential_id: &str,
) -> Result<bool> {
    let count: i64 = db.conn().query_row(
        "SELECT COUNT(*) FROM credential_refs WHERE vault_ref = ?1 AND credential_id <> ?2",
        params![vault_ref, exclude_credential_id],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

// ---------------------------------------------------------------------------
// 凭据
// ---------------------------------------------------------------------------

/// 对外的凭据引用视图：只含引用、类型、掩码与指纹，**不含秘密，也不含 vault_ref**。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CredentialRef {
    pub credential_id: String,
    pub account_id: String,
    pub credential_type: CredentialType,
    pub mask: String,
    pub fingerprint: String,
    /// 该凭据声明的能力（JSON）；无声明为 `null`。
    pub capabilities: Option<serde_json::Value>,
    pub created_at: String,
    /// 退休时间：更换 Key 后旧引用保留行与时间信息，但不再是当前凭据。
    pub retired_at: Option<String>,
}

impl CredentialRef {
    pub fn is_retired(&self) -> bool {
        self.retired_at.is_some()
    }
}

/// DB 行 + 仅供内部清理使用的 `vault_ref`（不出现在任何 DTO / IPC 响应里）。
struct StoredCredential {
    credential: CredentialRef,
    vault_ref: String,
}

const CREDENTIAL_COLUMNS: &str =
    "credential_id, account_id, credential_type, mask, fingerprint, vault_ref, capabilities, \
     created_at, retired_at";

fn row_to_stored_credential(row: &rusqlite::Row<'_>) -> Result<StoredCredential> {
    let type_raw: String = row.get(2)?;
    let caps_raw: Option<String> = row.get(6)?;
    let capabilities = match caps_raw {
        None => None,
        Some(text) => Some(
            serde_json::from_str(&text).map_err(|_| AccountsError::CorruptRow {
                kind: "capabilities",
                value: text,
            })?,
        ),
    };
    Ok(StoredCredential {
        vault_ref: row.get(5)?,
        credential: CredentialRef {
            credential_id: row.get(0)?,
            account_id: row.get(1)?,
            credential_type: CredentialType::parse(&type_raw).ok_or(AccountsError::CorruptRow {
                kind: "credential_type",
                value: type_raw,
            })?,
            mask: row.get(3)?,
            fingerprint: row.get(4)?,
            capabilities,
            created_at: row.get(7)?,
            retired_at: row.get(8)?,
        },
    })
}

fn load_stored_credential(db: &Database, credential_id: &str) -> Result<Option<StoredCredential>> {
    let sql = format!("SELECT {CREDENTIAL_COLUMNS} FROM credential_refs WHERE credential_id = ?1");
    let mut stmt = db.conn().prepare(&sql)?;
    let mut rows = stmt.query(params![credential_id])?;
    match rows.next()? {
        Some(row) => Ok(Some(row_to_stored_credential(row)?)),
        None => Ok(None),
    }
}

/// 保存凭据：明文只进配置的凭据库，DB 只写 `credential_refs`（引用 / 类型 / 掩码 / 指纹）。
///
/// 顺序：先生成 `credential_id`（同时作为 `vault_ref`），凭据库写入成功后才写 DB 行；
/// DB 写入失败时回滚刚写入的凭据库条目，避免留下无法解析的秘密。
/// 返回的 [`CredentialRef`] 不含秘密：前端提交后即清空输入，响应不回显。
pub fn credential_save(
    db: &Database,
    vault: &dyn Vault,
    account_id: &str,
    credential_type: CredentialType,
    capabilities: Option<serde_json::Value>,
    secret: &SecretString,
) -> Result<CredentialRef> {
    ensure_active_account(db, account_id)?;
    if secret.is_empty() {
        return Err(AccountsError::EmptySecret);
    }
    let credential_id = new_uuid();
    let vault_ref = credential_id.clone();
    let mask_text = mask::mask_secret(secret.expose());
    let salt = mask::load_or_create_salt(db)?;
    let fingerprint = mask::fingerprint_hex(&salt, secret.expose());
    let capabilities_json = capabilities.as_ref().map(|value| value.to_string());

    vault.store(&vault_ref, secret)?;
    let insert = db.conn().execute(
        "INSERT INTO credential_refs
            (credential_id, account_id, credential_type, mask, fingerprint, vault_ref, capabilities)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            credential_id,
            account_id,
            credential_type.as_str(),
            mask_text,
            fingerprint,
            vault_ref,
            capabilities_json,
        ],
    );
    if let Err(err) = insert {
        let _ = vault.delete(&vault_ref);
        return Err(err.into());
    }
    load_stored_credential(db, &credential_id)?
        .map(|stored| stored.credential)
        .ok_or(AccountsError::CredentialNotFound(credential_id))
}

/// 更换 Key：给同一账户写入新凭据引用，旧引用标记 `retired_at`。
///
/// 旧引用行保留（既有快照仍指向它，时间信息不丢），旧秘密在确认无共享引用后从凭据库删除。
/// `capabilities` 为 `None` 时继承旧凭据的声明。
pub fn credential_rotate(
    db: &mut Database,
    vault: &dyn Vault,
    credential_id: &str,
    capabilities: Option<serde_json::Value>,
    secret: &SecretString,
) -> Result<CredentialRef> {
    let old = load_stored_credential(db, credential_id)?
        .ok_or_else(|| AccountsError::CredentialNotFound(credential_id.to_owned()))?;
    ensure_active_account(db, &old.credential.account_id)?;
    if secret.is_empty() {
        return Err(AccountsError::EmptySecret);
    }
    let new_id = new_uuid();
    let mask_text = mask::mask_secret(secret.expose());
    let salt = mask::load_or_create_salt(db)?;
    let fingerprint = mask::fingerprint_hex(&salt, secret.expose());
    let capabilities_json = capabilities
        .or_else(|| old.credential.capabilities.clone())
        .as_ref()
        .map(|value| value.to_string());

    vault.store(&new_id, secret)?;
    let written = (|| -> Result<()> {
        let tx = db.transaction()?;
        tx.execute(
            "UPDATE credential_refs SET retired_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE credential_id = ?1 AND retired_at IS NULL",
            params![credential_id],
        )?;
        tx.execute(
            "INSERT INTO credential_refs
                (credential_id, account_id, credential_type, mask, fingerprint, vault_ref, capabilities)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                new_id,
                old.credential.account_id,
                old.credential.credential_type.as_str(),
                mask_text,
                fingerprint,
                new_id,
                capabilities_json,
            ],
        )?;
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = written {
        let _ = vault.delete(&new_id);
        return Err(err);
    }
    let shared = vault_ref_is_shared(db, &old.vault_ref, &old.credential.credential_id)?;
    if !shared {
        let _ = vault.delete(&old.vault_ref);
    }
    load_stored_credential(db, &new_id)?
        .map(|stored| stored.credential)
        .ok_or(AccountsError::CredentialNotFound(new_id))
}

/// 删除凭据：先删本地引用（同时把快照的 `credential_id` 置空，保留快照），
/// 再清理配置的凭据库条目（共享引用先检查）。清除本地凭据不等于平台撤销 Key。
pub fn credential_delete(db: &mut Database, vault: &dyn Vault, credential_id: &str) -> Result<()> {
    let stored = load_stored_credential(db, credential_id)?
        .ok_or_else(|| AccountsError::CredentialNotFound(credential_id.to_owned()))?;
    {
        let tx = db.transaction()?;
        tx.execute(
            "UPDATE account_snapshots SET credential_id = NULL WHERE credential_id = ?1",
            params![credential_id],
        )?;
        tx.execute(
            "DELETE FROM credential_refs WHERE credential_id = ?1",
            params![credential_id],
        )?;
        tx.commit()?;
    }
    if !vault_ref_is_shared(db, &stored.vault_ref, credential_id)? {
        let _ = vault.delete(&stored.vault_ref);
    }
    Ok(())
}

/// 账户的凭据列表；`include_retired = false` 时只返回当前凭据。
pub fn credentials_list(
    db: &Database,
    account_id: &str,
    include_retired: bool,
) -> Result<Vec<CredentialRef>> {
    let sql = format!(
        "SELECT {CREDENTIAL_COLUMNS} FROM credential_refs
         WHERE account_id = ?1 AND (?2 = 1 OR retired_at IS NULL)
         ORDER BY created_at, credential_id"
    );
    let mut stmt = db.conn().prepare(&sql)?;
    let mut rows = stmt.query(params![account_id, i64::from(include_retired)])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(row_to_stored_credential(row)?.credential);
    }
    Ok(out)
}

/// 服务层解封凭据所需的引用信息：**含 `vault_ref`，绝不出现在 DTO / IPC 响应里**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveCredential {
    pub capabilities: Option<serde_json::Value>,
    pub credential_id: String,
    pub credential_type: CredentialType,
    /// 配置的凭据库条目名；仅供服务层短期解封。
    pub vault_ref: String,
}

/// 账户当前（未退休）的凭据引用；没有有效凭据时返回 `None`。
///
/// 多个有效凭据时取最新创建的一条。`vault_ref` 只在服务层内部流转。
pub fn active_credential_ref(db: &Database, account_id: &str) -> Result<Option<ActiveCredential>> {
    let sql = format!(
        "SELECT {CREDENTIAL_COLUMNS} FROM credential_refs
         WHERE account_id = ?1 AND retired_at IS NULL
         ORDER BY created_at DESC, credential_id DESC LIMIT 1"
    );
    let mut stmt = db.conn().prepare(&sql)?;
    let mut rows = stmt.query(params![account_id])?;
    match rows.next()? {
        Some(row) => {
            let stored = row_to_stored_credential(row)?;
            Ok(Some(ActiveCredential {
                capabilities: stored.credential.capabilities,
                credential_id: stored.credential.credential_id,
                credential_type: stored.credential.credential_type,
                vault_ref: stored.vault_ref,
            }))
        }
        None => Ok(None),
    }
}

fn ensure_credential_owned(db: &Database, credential_id: &str, account_id: &str) -> Result<()> {
    match load_stored_credential(db, credential_id)? {
        None => Err(AccountsError::CredentialNotFound(credential_id.to_owned())),
        Some(stored) if stored.credential.account_id != account_id => {
            Err(AccountsError::CredentialAccountMismatch {
                credential_id: credential_id.to_owned(),
                account_id: account_id.to_owned(),
            })
        }
        Some(_) => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// 快照：余额
// ---------------------------------------------------------------------------

/// 余额条目（DATA_CONTRACTS 第 7 节）。`amount` 是十进制字符串，不用二进制浮点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceEntry {
    pub kind: BalanceKind,
    pub amount: String,
    /// ISO 货币或厂商单位（如 USD / credits）；不同币种/单位分开成行，绝不合并。
    pub currency: String,
    pub wallet_scope: Option<String>,
    pub expires_at: Option<String>,
    /// 来源公式（例如 `total = granted + topped_up`）；用于避免“已含项”被重复相加。
    pub formula: Option<String>,
    pub quality: Quality,
}

impl BalanceEntry {
    /// 平台明确返回的余额：质量默认 `reported`（源给的合法 0 也是 reported）。
    pub fn new(
        kind: BalanceKind,
        amount: impl Into<String>,
        currency: impl Into<String>,
    ) -> Result<Self> {
        let amount = amount.into();
        let currency = currency.into();
        Money::new(&amount, &currency)
            .map_err(|_| AccountsError::InvalidMoney(format!("{amount} {currency}")))?;
        Ok(Self {
            kind,
            amount,
            currency,
            wallet_scope: None,
            expires_at: None,
            formula: None,
            quality: Quality::Reported,
        })
    }

    pub fn with_formula(mut self, formula: impl Into<String>) -> Self {
        self.formula = Some(formula.into());
        self
    }

    pub fn with_quality(mut self, quality: Quality) -> Self {
        self.quality = quality;
        self
    }
}

fn validate_balance(balance: &BalanceEntry) -> Result<()> {
    Money::new(&balance.amount, &balance.currency).map_err(|_| {
        AccountsError::InvalidMoney(format!("{} {}", balance.amount, balance.currency))
    })?;
    Ok(())
}

/// 同币种/同钱包范围的总值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceTotal {
    pub currency: String,
    pub wallet_scope: Option<String>,
    pub amount: String,
}

/// `formula` 是否声明该行是“总值行”（左边是 total / total_balance / available）。
///
/// 契约第 7 节：`total_balance` 已含 paid / grant 时不能再相加。
fn formula_declares_total(formula: Option<&str>) -> bool {
    let Some(lhs) = formula.and_then(|text| text.split('=').next()) else {
        return false;
    };
    matches!(
        lhs.trim().to_ascii_lowercase().as_str(),
        "total" | "total_balance" | "available"
    )
}

/// 小数位数（`"10.00"` → 2，`"300"` → 0）。
fn frac_len(value: &str) -> usize {
    value.split_once('.').map_or(0, |(_, frac)| frac.len())
}

/// 把合计补回参与行的最大小数位：`decimal_add` 会去掉尾零，但余额要保留源精度
/// （DATA_CONTRACTS 第 1 节：保留源精度及单位转换规则）。
fn pad_scale(value: &str, scale: usize) -> String {
    if scale == 0 {
        return value.to_owned();
    }
    let (int_part, frac_part) = value.split_once('.').unwrap_or((value, ""));
    let mut out = String::with_capacity(value.len() + scale);
    out.push_str(int_part);
    out.push('.');
    out.push_str(frac_part);
    for _ in frac_part.len()..scale {
        out.push('0');
    }
    out
}

/// 汇总余额：按 `(currency, wallet_scope)` 分组，**绝不跨币种/单位相加**。
///
/// 组内若存在总值行（`kind = available`，或 `formula` 明示左边是 total/available），
/// 只取总值行，不再叠加 paid / grant / credit；没有总值行时才相加各组件。
/// 负余额保留真实值（加法的结果不封零）。
pub fn grouped_balances(entries: &[BalanceEntry]) -> Result<Vec<BalanceTotal>> {
    let mut groups: BTreeMap<(String, Option<String>), Vec<&BalanceEntry>> = BTreeMap::new();
    for entry in entries {
        groups
            .entry((entry.currency.clone(), entry.wallet_scope.clone()))
            .or_default()
            .push(entry);
    }
    let mut out = Vec::new();
    for ((currency, wallet_scope), rows) in groups {
        let total_rows: Vec<&BalanceEntry> = rows
            .iter()
            .copied()
            .filter(|entry| {
                entry.kind == BalanceKind::Available
                    || formula_declares_total(entry.formula.as_deref())
            })
            .collect();
        let selected: &[&BalanceEntry] = if total_rows.is_empty() {
            &rows
        } else {
            &total_rows
        };
        let mut amount = String::from("0");
        for entry in selected {
            amount = decimal_add(&amount, &entry.amount).ok_or_else(|| {
                AccountsError::InvalidDecimal {
                    field: "amount",
                    value: entry.amount.clone(),
                }
            })?;
        }
        let scale = selected
            .iter()
            .map(|e| frac_len(&e.amount))
            .max()
            .unwrap_or(0);
        out.push(BalanceTotal {
            currency,
            wallet_scope,
            amount: pad_scale(&amount, scale),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 快照：额度窗口
// ---------------------------------------------------------------------------

/// 额度窗口（DATA_CONTRACTS 第 8 节）。`window_id` 是平台身份，不能只用 “5h”。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaWindow {
    pub window_id: String,
    pub label: Option<String>,
    pub resource: Option<String>,
    /// account / organization / project / key / subscription。
    pub scope: Option<String>,
    pub unit: QuotaUnit,
    pub period_kind: Option<PeriodKind>,
    /// 实际已知才填；没有证据保持 `None`，绝不推断“默认 7 天”。
    pub duration_seconds: Option<u64>,
    pub starts_at: Option<String>,
    pub resets_at: Option<String>,
    pub used: Option<String>,
    pub limit: Option<String>,
    pub remaining: Option<String>,
    pub used_ratio: Option<String>,
    pub remaining_ratio: Option<String>,
    pub raw_ratio: Option<String>,
    pub over_limit: Option<bool>,
    pub quality: Quality,
    pub evidence_ref: Option<String>,
}

impl QuotaWindow {
    pub fn new(window_id: impl Into<String>, unit: QuotaUnit) -> Self {
        Self {
            window_id: window_id.into(),
            label: None,
            resource: None,
            scope: None,
            unit,
            period_kind: None,
            duration_seconds: None,
            starts_at: None,
            resets_at: None,
            used: None,
            limit: None,
            remaining: None,
            used_ratio: None,
            remaining_ratio: None,
            raw_ratio: None,
            over_limit: None,
            quality: Quality::Reported,
            evidence_ref: None,
        }
    }

    /// 只拿到“已用比例”的窗口：`used` / `limit` / `remaining` 保持 `None`，
    /// 只填 `used_ratio`，并由 `1 - used_ratio` 得到 `remaining_ratio`。
    ///
    /// 契约第 8 节：比例可以换算成比例，**绝不能反推绝对 Token/请求数**。
    pub fn from_used_ratio(
        window_id: impl Into<String>,
        unit: QuotaUnit,
        used_ratio: &str,
    ) -> Result<Self> {
        validate_decimal("used_ratio", used_ratio)?;
        let mut window = Self::new(window_id, unit);
        window.used_ratio = Some(used_ratio.to_owned());
        window.remaining_ratio = one_minus(used_ratio);
        Ok(window)
    }
}

fn validate_window(window: &QuotaWindow) -> Result<()> {
    for (field, value) in [
        ("used", &window.used),
        ("limit", &window.limit),
        ("remaining", &window.remaining),
        ("used_ratio", &window.used_ratio),
        ("remaining_ratio", &window.remaining_ratio),
        ("raw_ratio", &window.raw_ratio),
    ] {
        if let Some(value) = value {
            validate_decimal(field, value)?;
        }
    }
    if let Some(seconds) = window.duration_seconds {
        if i64::try_from(seconds).is_err() {
            return Err(AccountsError::InvalidDecimal {
                field: "duration_seconds",
                value: seconds.to_string(),
            });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 快照：账户级用量指标
// ---------------------------------------------------------------------------

/// 账户级用量指标（DATA_CONTRACTS 第 7 节 `AccountSnapshot.usage_metrics[]`）。
///
/// 与余额/窗口同属一次快照的标准化字段：`value` 是十进制字符串（不用二进制浮点），
/// `unit` 复用契约单位枚举，`scope` 说明指标归属（account / key / …）。不是现金余额，
/// 也不参与余额合计。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageMetric {
    pub name: String,
    pub value: String,
    pub unit: QuotaUnit,
    pub scope: Option<String>,
    pub quality: Quality,
}

impl UsageMetric {
    /// 平台明确返回的指标：质量默认 `reported`。
    pub fn new(name: impl Into<String>, value: impl Into<String>, unit: QuotaUnit) -> Result<Self> {
        let name = name.into();
        let value = value.into();
        if name.trim().is_empty() {
            return Err(AccountsError::EmptyField("usage_metric.name"));
        }
        validate_decimal("usage_metric.value", &value)?;
        Ok(Self {
            name,
            value,
            unit,
            scope: None,
            quality: Quality::Reported,
        })
    }

    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    pub fn with_quality(mut self, quality: Quality) -> Self {
        self.quality = quality;
        self
    }
}

fn validate_metric(metric: &UsageMetric) -> Result<()> {
    if metric.name.trim().is_empty() {
        return Err(AccountsError::EmptyField("usage_metric.name"));
    }
    validate_decimal("usage_metric.value", &metric.value)
}

// ---------------------------------------------------------------------------
// 快照：写入与查询
// ---------------------------------------------------------------------------

/// 一次成功查询的标准快照输入。只保存标准化必要字段，不保存完整鉴权响应。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewSnapshot {
    pub account_id: String,
    /// 产出该快照的凭据引用；可为 `None`（例如本机 Server 通道）。
    pub credential_id: Option<String>,
    /// 本机请求时间（RFC3339）。
    pub queried_at: String,
    /// 平台数据时间；未知保持 `None`，不用 `queried_at` 顶替。
    pub provider_as_of: Option<String>,
    /// 接口版本与来源标注。
    pub evidence: Option<String>,
    pub expires_at: Option<String>,
    pub balances: Vec<BalanceEntry>,
    pub quota_windows: Vec<QuotaWindow>,
    /// 账户级用量指标（契约第 7 节）；没有则为空，不造零值占位。
    pub usage_metrics: Vec<UsageMetric>,
}

/// 已存快照（含余额、额度窗口与账户级用量指标）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub snapshot_id: i64,
    pub account_id: String,
    pub credential_id: Option<String>,
    pub queried_at: String,
    pub provider_as_of: Option<String>,
    pub status: SnapshotStatus,
    pub evidence: Option<String>,
    pub error_code: Option<String>,
    pub expires_at: Option<String>,
    pub balances: Vec<BalanceEntry>,
    pub quota_windows: Vec<QuotaWindow>,
    pub usage_metrics: Vec<UsageMetric>,
}

impl Snapshot {
    /// 展示用状态：旧值仍可用但已不是最新成功查询时显示 `stale`（契约第 8 节）。
    pub fn effective_status(&self, is_stale: bool) -> SnapshotStatus {
        if is_stale && self.status == SnapshotStatus::Ok {
            SnapshotStatus::Stale
        } else {
            self.status
        }
    }
}

const INSERT_BALANCE: &str = "INSERT INTO balances
    (snapshot_id, kind, amount, currency, wallet_scope, expires_at, formula, quality)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)";

/// 账户级用量指标：同一快照内 `(name, scope)` 唯一，UPSERT 幂等重试。
const UPSERT_METRIC: &str = "INSERT INTO usage_metrics
    (snapshot_id, name, value, unit, scope, quality)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6)
    ON CONFLICT(snapshot_id, name, scope) DO UPDATE SET
        value = excluded.value,
        unit = excluded.unit,
        quality = excluded.quality";

/// `quota_windows` 以 `(window_id, snapshot_id)` 为主键：同一快照内同窗口 UPSERT（幂等重试），
/// 不同快照各存一行——窗口历史随快照时间线完整保留（契约第 7 节默认保留策略）。
const UPSERT_WINDOW: &str = "INSERT INTO quota_windows
    (window_id, snapshot_id, label, resource, scope, unit, period_kind, duration_seconds,
     starts_at, resets_at, used, limit_value, remaining, used_ratio, remaining_ratio, raw_ratio,
     over_limit, quality, evidence_ref)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
    ON CONFLICT(window_id, snapshot_id) DO UPDATE SET
        label = excluded.label,
        resource = excluded.resource,
        scope = excluded.scope,
        unit = excluded.unit,
        period_kind = excluded.period_kind,
        duration_seconds = excluded.duration_seconds,
        starts_at = excluded.starts_at,
        resets_at = excluded.resets_at,
        used = excluded.used,
        limit_value = excluded.limit_value,
        remaining = excluded.remaining,
        used_ratio = excluded.used_ratio,
        remaining_ratio = excluded.remaining_ratio,
        raw_ratio = excluded.raw_ratio,
        over_limit = excluded.over_limit,
        quality = excluded.quality,
        evidence_ref = excluded.evidence_ref";

/// 写入一次成功快照：快照 + balances + quota_windows **同一事务**，任一步失败整体回滚。
///
/// 只有成功查询走这里（`status = ok`）；失败用 [`failure_record`]。
/// 返回 `snapshot_id`。
pub fn snapshot_write(db: &mut Database, draft: &NewSnapshot) -> Result<i64> {
    ensure_active_account(db, &draft.account_id)?;
    if let Some(credential_id) = draft.credential_id.as_deref() {
        ensure_credential_owned(db, credential_id, &draft.account_id)?;
    }
    for balance in &draft.balances {
        validate_balance(balance)?;
    }
    let mut seen_windows = BTreeSet::new();
    for window in &draft.quota_windows {
        if window.window_id.trim().is_empty() {
            return Err(AccountsError::MissingWindowId);
        }
        if !seen_windows.insert(window.window_id.as_str()) {
            return Err(AccountsError::DuplicateWindowId(window.window_id.clone()));
        }
        validate_window(window)?;
    }
    let mut seen_metrics = BTreeSet::new();
    for metric in &draft.usage_metrics {
        validate_metric(metric)?;
        if !seen_metrics.insert((metric.name.as_str(), metric.scope.as_deref())) {
            return Err(AccountsError::DuplicateUsageMetric(metric.name.clone()));
        }
    }

    let tx = db.transaction()?;
    tx.execute(
        "INSERT INTO account_snapshots
            (account_id, credential_id, queried_at, provider_as_of, status, evidence, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            draft.account_id,
            draft.credential_id,
            draft.queried_at,
            draft.provider_as_of,
            SNAPSHOT_OK,
            draft.evidence,
            draft.expires_at,
        ],
    )?;
    let snapshot_id = tx.last_insert_rowid();
    for balance in &draft.balances {
        tx.execute(
            INSERT_BALANCE,
            params![
                snapshot_id,
                balance.kind.as_str(),
                balance.amount,
                balance.currency,
                balance.wallet_scope,
                balance.expires_at,
                balance.formula,
                balance.quality.as_str(),
            ],
        )?;
    }
    for window in &draft.quota_windows {
        let duration: Option<i64> = match window.duration_seconds {
            Some(seconds) => {
                Some(
                    i64::try_from(seconds).map_err(|_| AccountsError::InvalidDecimal {
                        field: "duration_seconds",
                        value: seconds.to_string(),
                    })?,
                )
            }
            None => None,
        };
        tx.execute(
            UPSERT_WINDOW,
            params![
                window.window_id,
                snapshot_id,
                window.label,
                window.resource,
                window.scope,
                window.unit.as_str(),
                window.period_kind.map(PeriodKind::as_str),
                duration,
                window.starts_at,
                window.resets_at,
                window.used,
                window.limit,
                window.remaining,
                window.used_ratio,
                window.remaining_ratio,
                window.raw_ratio,
                window.over_limit.map(i64::from),
                window.quality.as_str(),
                window.evidence_ref,
            ],
        )?;
    }
    for metric in &draft.usage_metrics {
        tx.execute(
            UPSERT_METRIC,
            params![
                snapshot_id,
                metric.name,
                metric.value,
                metric.unit.as_str(),
                metric.scope,
                metric.quality.as_str(),
            ],
        )?;
    }
    tx.commit()?;
    Ok(snapshot_id)
}

/// 最近一次成功快照 + stale 标记。
///
/// - 返回的是最近的成功（`ok`）快照，网络失败时旧值保留而非丢弃；
/// - `is_stale = 存在比该成功快照更新的失败记录`（即最近成功快照已非最新）；
/// - 从来没有成功快照时返回 `None`（此时用 [`snapshot_latest_status`] 看最新失败状态）。
pub fn snapshot_latest(db: &Database, account_id: &str) -> Result<Option<(Snapshot, bool)>> {
    let latest_ok: Option<(i64, String)> = db
        .conn()
        .query_row(
            "SELECT snapshot_id, queried_at FROM account_snapshots
             WHERE account_id = ?1 AND status = ?2
             ORDER BY queried_at DESC, snapshot_id DESC LIMIT 1",
            params![account_id, SNAPSHOT_OK],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((snapshot_id, queried_at)) = latest_ok else {
        return Ok(None);
    };
    let newer_failures: i64 = db.conn().query_row(
        "SELECT COUNT(*) FROM account_snapshots
         WHERE account_id = ?1 AND status <> ?2
           AND (queried_at > ?3 OR (queried_at = ?3 AND snapshot_id > ?4))",
        params![account_id, SNAPSHOT_OK, queried_at, snapshot_id],
        |row| row.get(0),
    )?;
    Ok(Some((load_snapshot(db, snapshot_id)?, newer_failures > 0)))
}

/// 最新一行快照的状态（不分成功/失败），用于展示“本次刷新失败”的原因。
pub fn snapshot_latest_status(db: &Database, account_id: &str) -> Result<Option<SnapshotStatus>> {
    let raw: Option<String> = db
        .conn()
        .query_row(
            "SELECT status FROM account_snapshots WHERE account_id = ?1
             ORDER BY queried_at DESC, snapshot_id DESC LIMIT 1",
            params![account_id],
            |row| row.get(0),
        )
        .optional()?;
    match raw {
        None => Ok(None),
        Some(value) => Ok(Some(parse_status(&value)?)),
    }
}

fn parse_status(value: &str) -> Result<SnapshotStatus> {
    SnapshotStatus::parse(value).ok_or(AccountsError::CorruptRow {
        kind: "status",
        value: value.to_owned(),
    })
}

fn load_snapshot(db: &Database, snapshot_id: i64) -> Result<Snapshot> {
    let mut snapshot = {
        let mut stmt = db.conn().prepare(
            "SELECT snapshot_id, account_id, credential_id, queried_at, provider_as_of, status,
                    evidence, error_code, expires_at
             FROM account_snapshots WHERE snapshot_id = ?1",
        )?;
        let mut rows = stmt.query(params![snapshot_id])?;
        match rows.next()? {
            Some(row) => row_to_snapshot(row)?,
            None => return Err(AccountsError::SnapshotNotFound(snapshot_id)),
        }
    };
    snapshot.balances = load_balances(db, snapshot_id)?;
    snapshot.quota_windows = load_quota_windows(db, snapshot_id)?;
    snapshot.usage_metrics = load_usage_metrics(db, snapshot_id)?;
    Ok(snapshot)
}

fn row_to_snapshot(row: &rusqlite::Row<'_>) -> Result<Snapshot> {
    let status_raw: String = row.get(5)?;
    Ok(Snapshot {
        snapshot_id: row.get(0)?,
        account_id: row.get(1)?,
        credential_id: row.get(2)?,
        queried_at: row.get(3)?,
        provider_as_of: row.get(4)?,
        status: parse_status(&status_raw)?,
        evidence: row.get(6)?,
        error_code: row.get(7)?,
        expires_at: row.get(8)?,
        balances: Vec::new(),
        quota_windows: Vec::new(),
        usage_metrics: Vec::new(),
    })
}

fn load_balances(db: &Database, snapshot_id: i64) -> Result<Vec<BalanceEntry>> {
    let mut stmt = db.conn().prepare(
        "SELECT kind, amount, currency, wallet_scope, expires_at, formula, quality
         FROM balances WHERE snapshot_id = ?1",
    )?;
    let mut rows = stmt.query(params![snapshot_id])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let kind_raw: String = row.get(0)?;
        let quality_raw: String = row.get(6)?;
        out.push(BalanceEntry {
            kind: BalanceKind::parse(&kind_raw).ok_or(AccountsError::CorruptRow {
                kind: "balance.kind",
                value: kind_raw,
            })?,
            amount: row.get(1)?,
            currency: row.get(2)?,
            wallet_scope: row.get(3)?,
            expires_at: row.get(4)?,
            formula: row.get(5)?,
            quality: Quality::parse(&quality_raw).ok_or(AccountsError::CorruptRow {
                kind: "balance.quality",
                value: quality_raw,
            })?,
        });
    }
    Ok(out)
}

fn load_quota_windows(db: &Database, snapshot_id: i64) -> Result<Vec<QuotaWindow>> {
    let mut stmt = db.conn().prepare(
        "SELECT window_id, label, resource, scope, unit, period_kind, duration_seconds,
                starts_at, resets_at, used, limit_value, remaining, used_ratio, remaining_ratio,
                raw_ratio, over_limit, quality, evidence_ref
         FROM quota_windows WHERE snapshot_id = ?1 ORDER BY window_id",
    )?;
    let mut rows = stmt.query(params![snapshot_id])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let unit_raw: String = row.get(4)?;
        let period_raw: Option<String> = row.get(5)?;
        let duration_raw: Option<i64> = row.get(6)?;
        let over_limit_raw: Option<i64> = row.get(15)?;
        let quality_raw: String = row.get(16)?;
        let period_kind = match period_raw {
            None => None,
            Some(value) => Some(PeriodKind::parse(&value).ok_or(AccountsError::CorruptRow {
                kind: "quota.period_kind",
                value,
            })?),
        };
        let duration_seconds = match duration_raw {
            None => None,
            Some(value) => Some(u64::try_from(value).map_err(|_| AccountsError::CorruptRow {
                kind: "quota.duration_seconds",
                value: value.to_string(),
            })?),
        };
        out.push(QuotaWindow {
            window_id: row.get(0)?,
            label: row.get(1)?,
            resource: row.get(2)?,
            scope: row.get(3)?,
            unit: QuotaUnit::parse(&unit_raw).ok_or(AccountsError::CorruptRow {
                kind: "quota.unit",
                value: unit_raw,
            })?,
            period_kind,
            duration_seconds,
            starts_at: row.get(7)?,
            resets_at: row.get(8)?,
            used: row.get(9)?,
            limit: row.get(10)?,
            remaining: row.get(11)?,
            used_ratio: row.get(12)?,
            remaining_ratio: row.get(13)?,
            raw_ratio: row.get(14)?,
            over_limit: over_limit_raw.map(|value| value != 0),
            quality: Quality::parse(&quality_raw).ok_or(AccountsError::CorruptRow {
                kind: "quota.quality",
                value: quality_raw,
            })?,
            evidence_ref: row.get(17)?,
        });
    }
    Ok(out)
}

fn load_usage_metrics(db: &Database, snapshot_id: i64) -> Result<Vec<UsageMetric>> {
    let mut stmt = db.conn().prepare(
        "SELECT name, value, unit, scope, quality
         FROM usage_metrics WHERE snapshot_id = ?1 ORDER BY name, scope",
    )?;
    let mut rows = stmt.query(params![snapshot_id])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let unit_raw: String = row.get(2)?;
        let quality_raw: String = row.get(4)?;
        out.push(UsageMetric {
            name: row.get(0)?,
            value: row.get(1)?,
            unit: QuotaUnit::parse(&unit_raw).ok_or(AccountsError::CorruptRow {
                kind: "usage_metric.unit",
                value: unit_raw,
            })?,
            scope: row.get(3)?,
            quality: Quality::parse(&quality_raw).ok_or(AccountsError::CorruptRow {
                kind: "usage_metric.quality",
                value: quality_raw,
            })?,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 失败模型
// ---------------------------------------------------------------------------

/// 失败信号：严格区分传输层 HTTP 状态与平台业务码（ACCOUNT_ADAPTERS 第 5 节）。
///
/// **HTTP 200 且没有业务错误码、没有传输/协议错误 ⇒ 不是失败**，
/// 调用 [`failure_record`] 会被拒绝——成功响应不该被记成失败，失败也不该被 HTTP 200 掩盖。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureSignal {
    /// 传输层状态码；无响应（连接失败/超时）为 `None`。
    pub http_status: Option<u16>,
    /// 平台业务错误码：HTTP 200 + 业务失败时同样非空。非空即视为失败。
    pub business_code: Option<String>,
    /// 连接失败 / 超时 / DNS / TLS：完全没有 HTTP 响应。
    pub transport_failed: bool,
    /// 必需字段缺失或类型变化（schema 变化），不补零。
    pub schema_changed: bool,
    /// 平台明确没有该查询能力——与暂时失败不同。
    pub unsupported: bool,
    /// 适配器基于平台语义给出的显式状态，优先于默认推断。
    pub status_hint: Option<SnapshotStatus>,
}

impl FailureSignal {
    /// 该信号是否确为失败。业务码非空时即使 HTTP 2xx 也是失败。
    pub fn is_failure(&self) -> bool {
        if self.transport_failed || self.schema_changed || self.unsupported {
            return true;
        }
        match self.http_status {
            None => true,
            Some(status) if status >= 400 => true,
            Some(_) => self
                .business_code
                .as_deref()
                .is_some_and(|code| !code.trim().is_empty()),
        }
    }

    /// 默认状态映射。适配器知道平台语义时用 `status_hint` 覆盖。
    pub fn status(&self) -> SnapshotStatus {
        if let Some(hint) = self.status_hint {
            return hint;
        }
        if self.unsupported {
            return SnapshotStatus::Unsupported;
        }
        if self.schema_changed {
            return SnapshotStatus::Incompatible;
        }
        match self.http_status {
            Some(401) | Some(403) => SnapshotStatus::AuthRequired,
            Some(429) => SnapshotStatus::RateLimited,
            Some(503) | Some(504) => SnapshotStatus::Unavailable,
            Some(status) if status >= 500 => SnapshotStatus::Unavailable,
            // HTTP 200 + 业务码错误：平台可达但拒绝给出数据，默认 unavailable；
            // 凭据失效、地区限制等语义由适配器用 status_hint 明确。
            Some(_) => SnapshotStatus::Unavailable,
            None => SnapshotStatus::Unavailable,
        }
    }

    /// 落库用的脱敏错误码（无秘密）：业务码优先，其次传输/schema/能力类别，最后 `http_<status>`。
    pub fn error_code(&self) -> Option<String> {
        if let Some(code) = self
            .business_code
            .as_deref()
            .map(str::trim)
            .filter(|code| !code.is_empty())
        {
            return Some(crate::services::redact::truncate(code, 128));
        }
        if self.transport_failed {
            return Some(String::from("transport"));
        }
        if self.schema_changed {
            return Some(String::from("schema_changed"));
        }
        if self.unsupported {
            return Some(String::from("unsupported"));
        }
        self.http_status.map(|status| format!("http_{status}"))
    }
}

/// 记录一次失败查询：写一行失败快照（无 balances / quota_windows），
/// 最近成功快照与其时间保持不变，由 [`snapshot_latest`] 标 `stale`。
///
/// 返回落库的状态。HTTP 200 且无业务码的信号会被拒绝（[`AccountsError::NotAFailure`]）。
pub fn failure_record(
    db: &Database,
    account_id: &str,
    credential_id: Option<&str>,
    queried_at: &str,
    signal: &FailureSignal,
) -> Result<SnapshotStatus> {
    ensure_active_account(db, account_id)?;
    if !signal.is_failure() {
        return Err(AccountsError::NotAFailure {
            status: signal.http_status.unwrap_or(0),
        });
    }
    if let Some(credential_id) = credential_id {
        ensure_credential_owned(db, credential_id, account_id)?;
    }
    let status = signal.status();
    db.conn().execute(
        "INSERT INTO account_snapshots
            (account_id, credential_id, queried_at, provider_as_of, status, error_code)
         VALUES (?1, ?2, ?3, NULL, ?4, ?5)",
        params![
            account_id,
            credential_id,
            queried_at,
            status.as_str(),
            signal.error_code(),
        ],
    )?;
    Ok(status)
}

// ---------------------------------------------------------------------------
// 钱包
// ---------------------------------------------------------------------------

/// 确认/取消“账户共享同一钱包”。
///
/// 钱包分组只由用户确认驱动：`confirmed = 1` 才参与分组，
/// 余额数值相同**绝不**自动合并（契约第 7 节）。
pub fn wallet_link(
    db: &Database,
    wallet_id: &str,
    account_id: &str,
    confirmed: bool,
) -> Result<()> {
    if wallet_id.trim().is_empty() {
        return Err(AccountsError::EmptyField("wallet_id"));
    }
    ensure_active_account(db, account_id)?;
    db.conn().execute(
        "INSERT INTO wallet_links (wallet_id, account_id, confirmed) VALUES (?1, ?2, ?3)
         ON CONFLICT(wallet_id, account_id) DO UPDATE SET confirmed = excluded.confirmed",
        params![wallet_id, account_id, i64::from(confirmed)],
    )?;
    Ok(())
}

/// 取消钱包链接（行删除，不留确认痕迹）。
pub fn wallet_unlink(db: &Database, wallet_id: &str, account_id: &str) -> Result<()> {
    db.conn().execute(
        "DELETE FROM wallet_links WHERE wallet_id = ?1 AND account_id = ?2",
        params![wallet_id, account_id],
    )?;
    Ok(())
}

/// 与 `account_id` 已确认共享钱包的账户集合（含自身，按字典序）。
///
/// 只有双方都 `confirmed = 1` 才算同一钱包；软删除账户排除在外。
/// 判断依据只有 `wallet_links`，余额数值本身不参与。
pub fn confirmed_wallet_group(db: &Database, account_id: &str) -> Result<Vec<String>> {
    let mut ids: BTreeSet<String> = BTreeSet::new();
    ids.insert(account_id.to_owned());
    let mut stmt = db.conn().prepare(
        "SELECT DISTINCT peer.account_id
           FROM wallet_links mine
           JOIN wallet_links peer ON peer.wallet_id = mine.wallet_id
           JOIN accounts a ON a.account_id = peer.account_id
          WHERE mine.account_id = ?1 AND mine.confirmed = 1 AND peer.confirmed = 1
            AND a.deleted_at IS NULL",
    )?;
    let mut rows = stmt.query(params![account_id])?;
    while let Some(row) = rows.next()? {
        ids.insert(row.get(0)?);
    }
    Ok(ids.into_iter().collect())
}

/// 已确认共享钱包的分组余额：取组内各账户最近成功快照的余额，按
/// `(currency, wallet_scope)` 汇总，遵守“总值行不叠加组件”规则。
///
/// 未确认共享时组内只有自己，因此两个余额相同的账户不会被自动相加。
pub fn wallet_group_balances(db: &Database, account_id: &str) -> Result<Vec<BalanceTotal>> {
    let mut entries = Vec::new();
    for id in confirmed_wallet_group(db, account_id)? {
        if let Some((snapshot, _is_stale)) = snapshot_latest(db, &id)? {
            entries.extend(snapshot.balances);
        }
    }
    grouped_balances(&entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::MemoryVault;

    fn db() -> Database {
        Database::open_in_memory().expect("in-memory database")
    }

    fn ts(minute: u32) -> String {
        format!("2026-10-11T10:{minute:02}:00Z")
    }

    fn new_account(db: &Database, name: &str) -> Account {
        account_create(db, &NewAccount::new("deepseek", name, ConfigKind::ApiPayg))
            .expect("account created")
    }

    fn ok_snapshot(account_id: &str, queried_at: &str, amount: &str) -> NewSnapshot {
        NewSnapshot {
            account_id: account_id.to_owned(),
            queried_at: queried_at.to_owned(),
            balances: vec![BalanceEntry::new(BalanceKind::Available, amount, "USD").unwrap()],
            quota_windows: vec![QuotaWindow::from_used_ratio(
                "deepseek:api:account:5h:tokens",
                QuotaUnit::Tokens,
                "0.58",
            )
            .unwrap()],
            ..Default::default()
        }
    }

    // ---- 账户 CRUD ---------------------------------------------------------

    #[test]
    fn account_crud_soft_delete() {
        let mut db = db();
        let vault = MemoryVault::new();
        let account = new_account(&db, "DeepSeek 主账户");
        assert!(account.deleted_at.is_none());
        assert_eq!(account.config_kind, ConfigKind::ApiPayg);
        assert_eq!(
            account_get(&db, &account.account_id).unwrap(),
            Some(account.clone())
        );
        assert_eq!(accounts_list(&db, false).unwrap().len(), 1);

        let updated = account_update(
            &db,
            &account.account_id,
            &AccountPatch {
                display_name: Some("改名".into()),
                region: Some("cn".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.display_name, "改名");
        assert_eq!(updated.region.as_deref(), Some("cn"));
        // 未指定的字段保持原值
        assert_eq!(updated.scope, account.scope);

        // 软删除：行还在、字段保留，默认列表隐藏
        let cleanup = account_delete(&mut db, &vault, &account.account_id).unwrap();
        assert_eq!(cleanup, CredentialCleanup::default());
        let after = account_get(&db, &account.account_id).unwrap().unwrap();
        assert!(after.is_deleted());
        assert_eq!(after.display_name, "改名");
        assert!(accounts_list(&db, false).unwrap().is_empty());
        assert_eq!(accounts_list(&db, true).unwrap().len(), 1);

        // 软删除后不能再更新/写快照
        assert!(matches!(
            account_update(&db, &account.account_id, &AccountPatch::default()),
            Err(AccountsError::AccountDeleted(_))
        ));
        assert!(matches!(
            snapshot_write(&mut db, &ok_snapshot(&account.account_id, &ts(1), "1.00")),
            Err(AccountsError::AccountDeleted(_))
        ));
        assert!(matches!(
            account_delete(&mut db, &vault, "missing"),
            Err(AccountsError::AccountNotFound(_))
        ));
    }

    /// 账户可以没有任何 Agent：账户侧读写完全不依赖采集源。
    #[test]
    fn account_needs_no_agent_binding() {
        let db = db();
        let account = new_account(&db, "无 Agent 账户");
        assert!(account_get(&db, &account.account_id).unwrap().is_some());
        assert!(accounts_list(&db, false).unwrap().len() == 1);
    }

    /// 结构性守卫：账户域代码不得触碰用量账本的表。
    #[test]
    fn accounts_module_never_touches_usage_ledger_tables() {
        let source = include_str!("accounts.rs");
        let forbidden = [
            concat!("usage", "_events"),
            concat!("usage", "_observations"),
            concat!("event_identity", "_tombstones"),
            concat!("ingestion", "_cursors"),
            concat!("parse", "_runs"),
            concat!("source", "_files"),
            concat!("price", "_versions"),
            concat!("source", "s"),
            concat!("sess", "ions"),
        ];
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "账户域不得引用用量账本表 {needle}"
            );
        }
    }

    // ---- 凭据 -------------------------------------------------------------

    #[test]
    fn credential_save_stores_reference_only_and_never_echoes_secret() {
        let db = db();
        let vault = MemoryVault::new();
        let account = new_account(&db, "DeepSeek");
        let plaintext = "sk-synthetic-abcdefghijklmnop";
        let secret = SecretString::new(plaintext);

        let credential = credential_save(
            &db,
            &vault,
            &account.account_id,
            CredentialType::ApiKey,
            Some(serde_json::json!(["balance", "quota"])),
            &secret,
        )
        .unwrap();

        // 响应不回显秘密，掩码只保留末 4 位
        let json = serde_json::to_string(&credential).unwrap();
        assert!(!json.contains(plaintext));
        assert!(!json.contains("abcdefghijkl"));
        assert_eq!(credential.mask, "********mnop");
        assert_eq!(credential.fingerprint.len(), 64);
        assert!(!credential.is_retired());

        // DB 只写引用/类型/掩码/指纹/能力
        let (vault_ref, mask_text, fingerprint, kind): (String, String, String, String) = db
            .conn()
            .query_row(
                "SELECT vault_ref, mask, fingerprint, credential_type FROM credential_refs
                 WHERE credential_id = ?1",
                params![credential.credential_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(vault_ref, credential.credential_id);
        assert_eq!(mask_text, credential.mask);
        assert_eq!(fingerprint, credential.fingerprint);
        assert_eq!(kind, "api_key");

        // 普通 SQLite 里搜不到明文
        let leaked: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM credential_refs
                 WHERE mask = ?1 OR fingerprint = ?1 OR vault_ref = ?1",
                params![plaintext],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(leaked, 0);

        // 明文确实进了配置的凭据库
        let stored = vault.get(&vault_ref).unwrap();
        assert_eq!(stored.expose(), plaintext);
        assert_eq!(vault_ref, credential.credential_id);

        // 保存后列出凭据不含秘密
        let listed = credentials_list(&db, &account.account_id, false).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(!serde_json::to_string(&listed).unwrap().contains(plaintext));
    }

    #[test]
    fn credential_rejects_empty_secret_and_missing_account() {
        let db = db();
        let vault = MemoryVault::new();
        let account = new_account(&db, "DeepSeek");
        let empty = SecretString::new("");
        assert!(matches!(
            credential_save(
                &db,
                &vault,
                &account.account_id,
                CredentialType::ApiKey,
                None,
                &empty
            ),
            Err(AccountsError::EmptySecret)
        ));
        let secret = SecretString::new("sk-synthetic-1");
        assert!(matches!(
            credential_save(
                &db,
                &vault,
                "no-such",
                CredentialType::ApiKey,
                None,
                &secret
            ),
            Err(AccountsError::AccountNotFound(_))
        ));
        // 凭据库没有被写入任何条目
        assert!(!vault.exists("no-such").unwrap());
    }

    #[test]
    fn credential_rotate_retires_old_reference_and_keeps_history() {
        let mut db = db();
        let vault = MemoryVault::new();
        let account = new_account(&db, "DeepSeek");
        let old_secret = SecretString::new("sk-synthetic-old-old-old");
        let old = credential_save(
            &db,
            &vault,
            &account.account_id,
            CredentialType::ApiKey,
            None,
            &old_secret,
        )
        .unwrap();

        // 旧凭据下已有快照，轮换后快照仍指向旧引用
        let mut draft = ok_snapshot(&account.account_id, &ts(1), "10.00");
        draft.credential_id = Some(old.credential_id.clone());
        let snapshot_id = snapshot_write(&mut db, &draft).unwrap();

        let new_secret = SecretString::new("sk-synthetic-new-new-new");
        let rotated =
            credential_rotate(&mut db, &vault, &old.credential_id, None, &new_secret).unwrap();

        assert_ne!(rotated.credential_id, old.credential_id);
        assert_eq!(rotated.credential_type, CredentialType::ApiKey);
        assert_eq!(rotated.mask, "********-new");
        assert_eq!(rotated.fingerprint.len(), 64);

        // 旧引用退休但行保留
        let all = credentials_list(&db, &account.account_id, true).unwrap();
        assert_eq!(all.len(), 2);
        let current = credentials_list(&db, &account.account_id, false).unwrap();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].credential_id, rotated.credential_id);

        // 新秘密在凭据库，旧秘密已清理
        assert_eq!(
            vault.get(&rotated.credential_id).unwrap().expose(),
            "sk-synthetic-new-new-new"
        );
        assert!(!vault.exists(&old.credential_id).unwrap());

        // 旧快照保留（时间信息不丢）
        let (snapshot, _stale) = snapshot_latest(&db, &account.account_id).unwrap().unwrap();
        assert_eq!(snapshot.snapshot_id, snapshot_id);
        assert_eq!(
            snapshot.credential_id.as_deref(),
            Some(old.credential_id.as_str())
        );
    }

    #[test]
    fn credential_delete_and_account_delete_clean_up_by_reference() {
        let mut db = db();
        let vault = MemoryVault::new();
        let first = new_account(&db, "账户一");
        let second = new_account(&db, "账户二");
        let secret = SecretString::new("sk-synthetic-shared-0000");
        let shared = credential_save(
            &db,
            &vault,
            &first.account_id,
            CredentialType::ApiKey,
            None,
            &secret,
        )
        .unwrap();

        // 人为制造共享引用：第二个账户引用同一个 vault_ref
        let second_credential_id = new_uuid();
        db.conn()
            .execute(
                "INSERT INTO credential_refs
                    (credential_id, account_id, credential_type, mask, fingerprint, vault_ref)
                 VALUES (?1, ?2, 'api_key', ?3, ?4, ?5)",
                params![
                    second_credential_id,
                    second.account_id,
                    shared.mask,
                    shared.fingerprint,
                    shared.credential_id,
                ],
            )
            .unwrap();

        // 删除账户一：共享引用检查使凭据库条目保留
        let cleanup = account_delete(&mut db, &vault, &first.account_id).unwrap();
        assert_eq!(cleanup.refs_removed, 1);
        assert_eq!(cleanup.shared_refs_kept, 1);
        assert_eq!(cleanup.vault_entries_removed, 0);
        assert_eq!(
            credentials_list(&db, &first.account_id, true)
                .unwrap()
                .len(),
            0
        );
        assert!(vault.exists(&shared.credential_id).unwrap());

        // 删除最后一个引用：这次凭据库条目被清理
        credential_delete(&mut db, &vault, &second_credential_id).unwrap();
        assert!(!vault.exists(&shared.credential_id).unwrap());
        assert!(matches!(
            credential_delete(&mut db, &vault, &second_credential_id),
            Err(AccountsError::CredentialNotFound(_))
        ));
    }

    #[test]
    fn account_delete_removes_credentials_and_keeps_snapshots() {
        let mut db = db();
        let vault = MemoryVault::new();
        let account = new_account(&db, "DeepSeek");
        let secret = SecretString::new("sk-synthetic-solo");
        let credential = credential_save(
            &db,
            &vault,
            &account.account_id,
            CredentialType::ApiKey,
            None,
            &secret,
        )
        .unwrap();
        let mut draft = ok_snapshot(&account.account_id, &ts(1), "3.00");
        draft.credential_id = Some(credential.credential_id.clone());
        let snapshot_id = snapshot_write(&mut db, &draft).unwrap();

        let cleanup = account_delete(&mut db, &vault, &account.account_id).unwrap();
        assert_eq!(cleanup.refs_removed, 1);
        assert_eq!(cleanup.vault_entries_removed, 1);
        assert!(!vault.exists(&credential.credential_id).unwrap());

        // 快照保留，credential_id 置空
        let (snapshot, _stale) = snapshot_latest(&db, &account.account_id).unwrap().unwrap();
        assert_eq!(snapshot.snapshot_id, snapshot_id);
        assert_eq!(snapshot.credential_id, None);
        assert_eq!(snapshot.balances.len(), 1);
    }

    /// 账户侧表不得出现明文秘密列，也不得保存完整鉴权响应
    /// （契约第 9 节：禁止 credential_plaintext / prompt_body / response_body 字段；
    /// 第 7 节：快照只保存标准化必要字段）。
    #[test]
    fn account_tables_expose_no_plaintext_or_raw_response_columns() {
        let db = db();
        let tables = [
            "accounts",
            "credential_refs",
            "wallet_links",
            "account_snapshots",
            "balances",
            "quota_windows",
            "usage_metrics",
        ];
        for table in tables {
            let sql = format!("PRAGMA table_info({table})");
            let mut stmt = db.conn().prepare(&sql).unwrap();
            let mut rows = stmt.query([]).unwrap();
            let mut seen = 0;
            while let Some(row) = rows.next().unwrap() {
                let column: String = row.get(1).unwrap();
                let lowered = column.to_ascii_lowercase();
                for forbidden in [
                    "plaintext",
                    "secret",
                    "prompt",
                    "response",
                    "body",
                    "raw_json",
                ] {
                    assert!(
                        !lowered.contains(forbidden),
                        "{table}.{column} 不应保存秘密或原始响应"
                    );
                }
                seen += 1;
            }
            assert!(seen > 0, "{table} 应存在");
        }
    }

    // ---- 快照：事务与查询 --------------------------------------------------

    #[test]
    fn snapshot_write_is_transactional() {
        let mut db = db();
        let account = new_account(&db, "DeepSeek");
        let mut draft = ok_snapshot(&account.account_id, &ts(1), "10.00");
        draft
            .balances
            .push(BalanceEntry::new(BalanceKind::Grant, "5.00", "USD").unwrap());
        draft.quota_windows.push(
            QuotaWindow::from_used_ratio("glm:coding_plan:7d:requests", QuotaUnit::Requests, "0.9")
                .unwrap(),
        );

        // 人为让窗口插入失败，验证快照与余额一起回滚
        db.conn()
            .execute_batch(
                "CREATE TEMP TRIGGER quota_fail BEFORE INSERT ON quota_windows
                 BEGIN SELECT RAISE(ABORT, 'boom'); END;",
            )
            .unwrap();
        assert!(snapshot_write(&mut db, &draft).is_err());
        db.conn().execute_batch("DROP TRIGGER quota_fail").unwrap();

        let counts: (i64, i64, i64) = db
            .conn()
            .query_row(
                "SELECT (SELECT COUNT(*) FROM account_snapshots),
                        (SELECT COUNT(*) FROM balances),
                        (SELECT COUNT(*) FROM quota_windows)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(counts, (0, 0, 0));

        // 触发条件消失后同一份数据正常写入
        assert!(snapshot_write(&mut db, &draft).is_ok());
    }

    #[test]
    fn snapshot_write_and_latest_roundtrip() {
        let mut db = db();
        let account = new_account(&db, "DeepSeek");
        let secret = SecretString::new("sk-synthetic-roundtrip");
        let credential = credential_save(
            &db,
            &vault_ok(),
            &account.account_id,
            CredentialType::ApiKey,
            None,
            &secret,
        )
        .unwrap();

        let draft = NewSnapshot {
            account_id: account.account_id.clone(),
            credential_id: Some(credential.credential_id.clone()),
            queried_at: ts(30),
            // 平台时间与本机时间分开存，不能互相顶替
            provider_as_of: Some("2026-10-11T10:00:00Z".into()),
            evidence: Some("official:v1".into()),
            expires_at: Some("2026-12-31T00:00:00Z".into()),
            balances: vec![
                BalanceEntry::new(BalanceKind::Available, "10.00", "USD")
                    .unwrap()
                    .with_formula("total = granted + topped_up"),
                BalanceEntry::new(BalanceKind::Grant, "2.00", "USD").unwrap(),
                BalanceEntry::new(BalanceKind::Paid, "8.00", "USD").unwrap(),
                BalanceEntry::new(BalanceKind::Credit, "300", "credits").unwrap(),
            ],
            quota_windows: vec![
                QuotaWindow::from_used_ratio("glm:plan:5h:tokens", QuotaUnit::Tokens, "0.25")
                    .unwrap(),
                QuotaWindow::from_used_ratio("glm:plan:7d:requests", QuotaUnit::Requests, "0.5")
                    .unwrap(),
            ],
            usage_metrics: vec![
                UsageMetric::new("lifetime_tokens", "12345", QuotaUnit::Tokens)
                    .unwrap()
                    .with_scope("account"),
                UsageMetric::new("streak_days", "7", QuotaUnit::VendorUnit)
                    .unwrap()
                    .with_quality(Quality::Derived),
            ],
        };
        let snapshot_id = snapshot_write(&mut db, &draft).unwrap();

        let (snapshot, is_stale) = snapshot_latest(&db, &account.account_id).unwrap().unwrap();
        assert!(!is_stale);
        assert_eq!(snapshot.snapshot_id, snapshot_id);
        assert_eq!(snapshot.status, SnapshotStatus::Ok);
        assert_eq!(snapshot.queried_at, ts(30));
        assert_eq!(
            snapshot.provider_as_of.as_deref(),
            Some("2026-10-11T10:00:00Z")
        );
        assert_eq!(snapshot.evidence.as_deref(), Some("official:v1"));
        assert_eq!(snapshot.expires_at.as_deref(), Some("2026-12-31T00:00:00Z"));
        assert_eq!(snapshot.balances, draft.balances);
        assert_eq!(snapshot.quota_windows, draft.quota_windows);
        assert_eq!(snapshot.usage_metrics, draft.usage_metrics);
    }

    /// usage_metrics 与快照/余额/窗口同事务；写入失败整体回滚。
    #[test]
    fn usage_metrics_are_written_in_the_snapshot_transaction() {
        let mut db = db();
        let account = new_account(&db, "Codex");
        let mut draft = ok_snapshot(&account.account_id, &ts(1), "1.00");
        draft.usage_metrics = vec![UsageMetric::new("lifetime_tokens", "42", QuotaUnit::Tokens)
            .unwrap()
            .with_scope("account")];

        db.conn()
            .execute_batch(
                "CREATE TEMP TRIGGER metric_fail BEFORE INSERT ON usage_metrics
                 BEGIN SELECT RAISE(ABORT, 'boom'); END;",
            )
            .unwrap();
        assert!(snapshot_write(&mut db, &draft).is_err());
        db.conn().execute_batch("DROP TRIGGER metric_fail").unwrap();
        let count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM usage_metrics", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "指标随快照一起回滚");

        snapshot_write(&mut db, &draft).unwrap();
        let (snapshot, _) = snapshot_latest(&db, &account.account_id).unwrap().unwrap();
        assert_eq!(snapshot.usage_metrics, draft.usage_metrics);
        // 指标不是余额，不进入余额合计
        assert!(snapshot.balances.iter().all(|b| b.currency != "tokens"));
    }

    #[test]
    fn usage_metric_rejects_bad_values_and_duplicates() {
        let mut db = db();
        let account = new_account(&db, "Codex");
        assert!(matches!(
            UsageMetric::new("lifetime_tokens", "1e5", QuotaUnit::Tokens),
            Err(AccountsError::InvalidDecimal { .. })
        ));
        assert!(matches!(
            UsageMetric::new("  ", "1", QuotaUnit::Tokens),
            Err(AccountsError::EmptyField(_))
        ));

        let mut draft = ok_snapshot(&account.account_id, &ts(1), "1.00");
        let metric = UsageMetric::new("m", "1", QuotaUnit::Tokens).unwrap();
        draft.usage_metrics = vec![metric.clone(), metric];
        assert!(matches!(
            snapshot_write(&mut db, &draft),
            Err(AccountsError::DuplicateUsageMetric(_))
        ));
    }

    #[test]
    fn snapshot_write_rejects_foreign_credential_and_duplicate_windows() {
        let mut db = db();
        let first = new_account(&db, "账户一");
        let second = new_account(&db, "账户二");
        let vault = MemoryVault::new();
        let foreign = credential_save(
            &db,
            &vault,
            &second.account_id,
            CredentialType::ApiKey,
            None,
            &SecretString::new("sk-synthetic-foreign"),
        )
        .unwrap();

        let mut draft = ok_snapshot(&first.account_id, &ts(1), "1.00");
        draft.credential_id = Some(foreign.credential_id.clone());
        assert!(matches!(
            snapshot_write(&mut db, &draft),
            Err(AccountsError::CredentialAccountMismatch { .. })
        ));

        let mut draft = ok_snapshot(&first.account_id, &ts(2), "1.00");
        draft.quota_windows.push(draft.quota_windows[0].clone());
        assert!(matches!(
            snapshot_write(&mut db, &draft),
            Err(AccountsError::DuplicateWindowId(_))
        ));

        let mut draft = ok_snapshot(&first.account_id, &ts(3), "1.00");
        draft.balances[0].amount = "1e5".into();
        assert!(matches!(
            snapshot_write(&mut db, &draft),
            Err(AccountsError::InvalidMoney(_))
        ));
    }

    /// 同一窗口跨快照各存一行（历史保留）；窗口值随快照更新。
    #[test]
    fn quota_window_history_is_kept_per_snapshot() {
        let mut db = db();
        let account = new_account(&db, "GLM");
        let first = ok_snapshot(&account.account_id, &ts(1), "1.00");
        let second = ok_snapshot(&account.account_id, &ts(2), "1.00");
        let first_id = snapshot_write(&mut db, &first).unwrap();
        let second_id = snapshot_write(&mut db, &second).unwrap();

        let rows: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM quota_windows", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 2, "两次快照的窗口行都应保留");
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT snapshot_id FROM quota_windows WHERE window_id = ?1 ORDER BY snapshot_id",
            )
            .unwrap();
        let owners: Vec<i64> = stmt
            .query_map(params!["deepseek:api:account:5h:tokens"], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert_eq!(owners, vec![first_id, second_id]);
    }

    // ---- stale 规则 --------------------------------------------------------

    #[test]
    fn stale_rule_keeps_last_success_after_failure() {
        let mut db = db();
        let account = new_account(&db, "DeepSeek");
        let id = account.account_id.clone();
        let mut draft = ok_snapshot(&id, &ts(1), "10.00");
        draft.provider_as_of = Some("2026-10-11T09:00:00Z".into());
        draft.quota_windows.clear();
        let snapshot_id = snapshot_write(&mut db, &draft).unwrap();

        let (snapshot, is_stale) = snapshot_latest(&db, &id).unwrap().unwrap();
        assert_eq!(snapshot.snapshot_id, snapshot_id);
        assert!(!is_stale);
        assert_eq!(snapshot.effective_status(is_stale), SnapshotStatus::Ok);
        assert_eq!(
            snapshot_latest_status(&db, &id).unwrap(),
            Some(SnapshotStatus::Ok)
        );

        // 网络失败：保留最近成功快照与平台时间，并标 stale
        let signal = FailureSignal {
            transport_failed: true,
            ..Default::default()
        };
        assert_eq!(
            failure_record(&db, &id, None, &ts(2), &signal).unwrap(),
            SnapshotStatus::Unavailable
        );
        let (snapshot, is_stale) = snapshot_latest(&db, &id).unwrap().unwrap();
        assert_eq!(snapshot.snapshot_id, snapshot_id);
        assert_eq!(snapshot.queried_at, ts(1));
        assert_eq!(
            snapshot.provider_as_of.as_deref(),
            Some("2026-10-11T09:00:00Z")
        );
        assert_eq!(snapshot.balances.len(), 1);
        assert!(is_stale);
        assert_eq!(snapshot.effective_status(is_stale), SnapshotStatus::Stale);
        assert_eq!(
            snapshot_latest_status(&db, &id).unwrap(),
            Some(SnapshotStatus::Unavailable)
        );

        // 再次成功：不再 stale
        let mut second = ok_snapshot(&id, &ts(3), "12.00");
        second.provider_as_of = Some("2026-10-11T10:00:00Z".into());
        snapshot_write(&mut db, &second).unwrap();
        let (snapshot, is_stale) = snapshot_latest(&db, &id).unwrap().unwrap();
        assert!(!is_stale);
        assert_eq!(snapshot.queried_at, ts(3));
        assert_eq!(snapshot.balances[0].amount, "12.00");
    }

    #[test]
    fn failure_without_any_success_yields_no_snapshot_but_a_status() {
        let db = db();
        let account = new_account(&db, "DeepSeek");
        let signal = FailureSignal {
            http_status: Some(401),
            ..Default::default()
        };
        assert_eq!(
            failure_record(&db, &account.account_id, None, &ts(1), &signal).unwrap(),
            SnapshotStatus::AuthRequired
        );
        assert!(snapshot_latest(&db, &account.account_id).unwrap().is_none());
        assert_eq!(
            snapshot_latest_status(&db, &account.account_id).unwrap(),
            Some(SnapshotStatus::AuthRequired)
        );
    }

    // ---- 失败信号：HTTP 状态 vs 业务码 ------------------------------------

    #[test]
    fn business_code_error_is_a_failure_even_with_http_200() {
        let db = db();
        let account = new_account(&db, "DeepSeek");

        // HTTP 200 + 业务码：失败
        let signal = FailureSignal {
            http_status: Some(200),
            business_code: Some("1004".into()),
            ..Default::default()
        };
        assert!(signal.is_failure());
        assert_eq!(
            failure_record(&db, &account.account_id, None, &ts(1), &signal).unwrap(),
            SnapshotStatus::Unavailable
        );
        let code: String = db
            .conn()
            .query_row(
                "SELECT error_code FROM account_snapshots WHERE account_id = ?1",
                params![account.account_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(code, "1004");

        // HTTP 200 且无业务码：不是失败，不能记成失败
        let ok = FailureSignal {
            http_status: Some(200),
            ..Default::default()
        };
        assert!(!ok.is_failure());
        assert!(matches!(
            failure_record(&db, &account.account_id, None, &ts(2), &ok),
            Err(AccountsError::NotAFailure { status: 200 })
        ));
    }

    #[test]
    fn failure_signal_maps_to_contract_statuses() {
        let auth = FailureSignal {
            http_status: Some(403),
            ..Default::default()
        };
        assert_eq!(auth.status(), SnapshotStatus::AuthRequired);

        let limited = FailureSignal {
            http_status: Some(429),
            ..Default::default()
        };
        assert_eq!(limited.status(), SnapshotStatus::RateLimited);

        let down = FailureSignal {
            http_status: Some(502),
            ..Default::default()
        };
        assert_eq!(down.status(), SnapshotStatus::Unavailable);

        let no_response = FailureSignal {
            transport_failed: true,
            ..Default::default()
        };
        assert_eq!(no_response.status(), SnapshotStatus::Unavailable);

        let schema = FailureSignal {
            http_status: Some(200),
            schema_changed: true,
            ..Default::default()
        };
        assert_eq!(schema.status(), SnapshotStatus::Incompatible);

        let unsupported = FailureSignal {
            unsupported: true,
            ..Default::default()
        };
        assert_eq!(unsupported.status(), SnapshotStatus::Unsupported);

        // 适配器知道平台语义时可覆盖默认推断（例如业务码表示凭据失效）
        let hinted = FailureSignal {
            http_status: Some(200),
            business_code: Some("invalid_credential".into()),
            status_hint: Some(SnapshotStatus::AuthRequired),
            ..Default::default()
        };
        assert_eq!(hinted.status(), SnapshotStatus::AuthRequired);
    }

    #[test]
    fn snapshot_status_roundtrips_through_db() {
        let db = db();
        let account = new_account(&db, "DeepSeek");
        let all = [
            SnapshotStatus::Ok,
            SnapshotStatus::Stale,
            SnapshotStatus::Refreshing,
            SnapshotStatus::AuthRequired,
            SnapshotStatus::RateLimited,
            SnapshotStatus::Unsupported,
            SnapshotStatus::Incompatible,
            SnapshotStatus::Unavailable,
        ];
        for status in all {
            // wire 形态固定，且与契约枚举值一致
            assert_eq!(SnapshotStatus::parse(status.as_str()), Some(status));
            assert_eq!(
                serde_json::to_value(status).unwrap(),
                serde_json::Value::String(status.as_str().into())
            );
            db.conn()
                .execute(
                    "INSERT INTO account_snapshots (account_id, queried_at, status)
                     VALUES (?1, ?2, ?3)",
                    params![
                        account.account_id,
                        format!("2026-10-11T00:{:02}:00Z", status as usize),
                        status.as_str()
                    ],
                )
                .unwrap();
        }
        assert!(SnapshotStatus::parse("bogus").is_none());

        // 逐行读回，顺序与写入一致
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT status FROM account_snapshots WHERE account_id = ?1 ORDER BY queried_at",
            )
            .unwrap();
        let read: Vec<SnapshotStatus> = stmt
            .query_map(params![account.account_id], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|raw| SnapshotStatus::parse(&raw.unwrap()).unwrap())
            .collect();
        assert_eq!(read, all.to_vec());
        assert_eq!(
            snapshot_latest_status(&db, &account.account_id).unwrap(),
            Some(SnapshotStatus::Unavailable)
        );
    }

    // ---- 比例-only 窗口 ----------------------------------------------------

    #[test]
    fn ratio_only_window_keeps_absolute_values_null() {
        let mut db = db();
        let account = new_account(&db, "GLM");
        let window =
            QuotaWindow::from_used_ratio("glm:coding_plan:5h:tokens", QuotaUnit::Tokens, "0.58")
                .unwrap();

        // 只给比例：绝对量必须保持 NULL，且不反推
        assert_eq!(window.used, None);
        assert_eq!(window.limit, None);
        assert_eq!(window.remaining, None);
        assert_eq!(window.used_ratio.as_deref(), Some("0.58"));
        assert_eq!(window.remaining_ratio.as_deref(), Some("0.42"));
        // 没有证据就不填周期
        assert_eq!(window.duration_seconds, None);
        assert_eq!(window.period_kind, None);

        let mut draft = ok_snapshot(&account.account_id, &ts(1), "1.00");
        draft.quota_windows = vec![window.clone()];
        snapshot_write(&mut db, &draft).unwrap();

        let (snapshot, _stale) = snapshot_latest(&db, &account.account_id).unwrap().unwrap();
        assert_eq!(snapshot.quota_windows, vec![window]);

        // 非法比例被拒绝，不会写出脏数据
        assert!(matches!(
            QuotaWindow::from_used_ratio("w", QuotaUnit::Percent, "58%"),
            Err(AccountsError::InvalidDecimal { .. })
        ));
    }

    #[test]
    fn window_units_use_contract_enum_values() {
        for (unit, wire) in [
            (QuotaUnit::Tokens, "tokens"),
            (QuotaUnit::Requests, "requests"),
            (QuotaUnit::Credits, "credits"),
            (QuotaUnit::Currency, "currency"),
            (QuotaUnit::Percent, "percent"),
            (QuotaUnit::VendorUnit, "vendor_unit"),
        ] {
            assert_eq!(unit.as_str(), wire);
            assert_eq!(QuotaUnit::parse(wire), Some(unit));
        }
        assert!(QuotaUnit::parse("token").is_none());
        assert_eq!(PeriodKind::parse("rolling"), Some(PeriodKind::Rolling));
        assert_eq!(
            CredentialType::parse("management_key"),
            Some(CredentialType::ManagementKey)
        );
        assert_eq!(BalanceKind::parse("grant"), Some(BalanceKind::Grant));
    }

    // ---- 钱包：不自动合并 --------------------------------------------------

    #[test]
    fn identical_balances_are_never_auto_merged() {
        let mut db = db();
        let first = new_account(&db, "账户一");
        let second = new_account(&db, "账户二");
        let draft_first = ok_snapshot(&first.account_id, &ts(1), "10.00");
        let draft_second = ok_snapshot(&second.account_id, &ts(1), "10.00");
        snapshot_write(&mut db, &draft_first).unwrap();
        snapshot_write(&mut db, &draft_second).unwrap();

        // 余额相同、没有确认链接：各自独立，不合并
        assert_eq!(
            confirmed_wallet_group(&db, &first.account_id).unwrap(),
            vec![first.account_id.clone()]
        );
        let solo = wallet_group_balances(&db, &first.account_id).unwrap();
        assert_eq!(solo.len(), 1);
        assert_eq!(solo[0].amount, "10.00");

        // 未确认的链接不参与分组
        wallet_link(&db, "wallet-1", &first.account_id, false).unwrap();
        wallet_link(&db, "wallet-1", &second.account_id, false).unwrap();
        assert_eq!(
            confirmed_wallet_group(&db, &first.account_id).unwrap(),
            vec![first.account_id.clone()]
        );

        // 双方确认同一 wallet_id 才分组，且总额相加
        wallet_link(&db, "wallet-1", &first.account_id, true).unwrap();
        wallet_link(&db, "wallet-1", &second.account_id, true).unwrap();
        // 分组按字典序返回，同一毫秒创建的两个 id 顺序不固定，按集合比较
        let mut expected = vec![first.account_id.clone(), second.account_id.clone()];
        expected.sort();
        assert_eq!(
            confirmed_wallet_group(&db, &first.account_id).unwrap(),
            expected
        );
        let grouped = wallet_group_balances(&db, &first.account_id).unwrap();
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].amount, "20.00");

        // 取消确认后回到独立
        wallet_link(&db, "wallet-1", &second.account_id, false).unwrap();
        assert_eq!(
            wallet_group_balances(&db, &first.account_id).unwrap()[0].amount,
            "10.00"
        );
        wallet_unlink(&db, "wallet-1", &first.account_id).unwrap();
        wallet_unlink(&db, "wallet-1", &second.account_id).unwrap();
    }

    #[test]
    fn total_balance_does_not_double_count_included_components() {
        let entries = vec![
            BalanceEntry::new(BalanceKind::Available, "10.00", "USD")
                .unwrap()
                .with_formula("total = granted + topped_up"),
            BalanceEntry::new(BalanceKind::Grant, "2.00", "USD").unwrap(),
            BalanceEntry::new(BalanceKind::Paid, "8.00", "USD").unwrap(),
        ];
        let totals = grouped_balances(&entries).unwrap();
        assert_eq!(totals.len(), 1);
        assert_eq!(totals[0].currency, "USD");
        assert_eq!(totals[0].amount, "10.00", "总值已含组件时不能再相加");

        // 没有总值行时才相加组件
        let components = vec![
            BalanceEntry::new(BalanceKind::Grant, "2.00", "USD").unwrap(),
            BalanceEntry::new(BalanceKind::Paid, "8.00", "USD").unwrap(),
        ];
        assert_eq!(grouped_balances(&components).unwrap()[0].amount, "10.00");

        // 负余额保留真实值，不静默归零
        let negative = vec![BalanceEntry::new(BalanceKind::Available, "-3.50", "USD").unwrap()];
        assert_eq!(grouped_balances(&negative).unwrap()[0].amount, "-3.50");
    }

    #[test]
    fn different_currencies_and_units_are_never_added() {
        let entries = vec![
            BalanceEntry::new(BalanceKind::Available, "10.00", "USD").unwrap(),
            BalanceEntry::new(BalanceKind::Available, "70.00", "CNY").unwrap(),
            BalanceEntry::new(BalanceKind::Available, "300", "credits").unwrap(),
        ];
        let totals = grouped_balances(&entries).unwrap();
        assert_eq!(totals.len(), 3);
        let by_currency: BTreeMap<&str, &str> = totals
            .iter()
            .map(|total| (total.currency.as_str(), total.amount.as_str()))
            .collect();
        assert_eq!(by_currency.get("USD"), Some(&"10.00"));
        assert_eq!(by_currency.get("CNY"), Some(&"70.00"));
        assert_eq!(by_currency.get("credits"), Some(&"300"));
        // 不同 wallet_scope 也不合并
        let mut scoped = entries.clone();
        scoped[0].wallet_scope = Some("organization".into());
        assert_eq!(grouped_balances(&scoped).unwrap().len(), 3);
    }

    // ---- 标识 --------------------------------------------------------------

    #[test]
    fn generated_uuids_are_unique_and_vault_compatible() {
        let mut seen = BTreeSet::new();
        for _ in 0..1000 {
            let id = new_uuid();
            assert_eq!(id.len(), 36);
            assert_eq!(id.as_bytes()[14], b'4', "version nibble");
            assert!(id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
            assert!(seen.insert(id));
        }
        // vault_ref 校验接受该形态（store 成功即证明）
        let vault = MemoryVault::new();
        let id = new_uuid();
        vault
            .store(&id, &SecretString::new("sk-synthetic-x"))
            .unwrap();
        assert!(vault.exists(&id).unwrap());
    }

    fn vault_ok() -> MemoryVault {
        MemoryVault::new()
    }
}
