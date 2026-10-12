/**
 * 账户域 IPC 契约（与 src-tauri 的 serde DTO 对应，`rename_all = "camelCase"`）。
 *
 * 对应 Rust 类型（src-tauri/src/lib.rs / src-tauri/src/services）：
 * - `CredentialDto`        ← lib.rs `CredentialDto`（由 `services::accounts::CredentialRef` 转换）
 * - `AccountSummaryDto`    ← lib.rs `AccountSummaryDto`（`build_account_summary`）
 * - `Snapshot`             ← `services::accounts::Snapshot`
 * - `BalanceEntry`         ← `services::accounts::BalanceEntry`
 * - `QuotaWindow`          ← `services::accounts::QuotaWindow`
 * - `UsageMetric`          ← `services::accounts::UsageMetric`
 * - `SnapshotOutcome`      ← `services::account_query::SnapshotOutcome`（account_refresh 返回）
 * - `AccountDeleteResult`  ← `services::cleanup::AccountDeleteResult`（含 `CredentialCleanupSummary`）
 *
 * 金额、比例、额度一律为十进制字符串（`null` 表示平台未提供），前端不做数值汇总与换算
 * （ACCOUNT_ADAPTERS 第 6 节、ARCHITECTURE 第 6 节）；快照由 Rust 侧生成，前端只做展示。
 * 响应绝不回显秘密：`credentialRefs` 只含掩码与指纹。
 */

/** `services::accounts::ConfigKind`。 */
export type ConfigKind = "api_payg" | "coding_plan" | "both";

/** `services::accounts::SnapshotStatus`（DATA_CONTRACTS 第 8 节，取值固定）。 */
export type SnapshotStatus =
  | "ok"
  | "stale"
  | "refreshing"
  | "auth_required"
  | "rate_limited"
  | "unsupported"
  | "incompatible"
  | "unavailable"
  /** 平台侧确认该账户没有任何 Coding Plan 套餐（非失败，仅提示）。 */
  | "no_plan";

/** `services::accounts::CredentialType`（ACCOUNT_ADAPTERS 第 4 节）。 */
export type CredentialType =
  "api_key" | "management_key" | "oauth_handle" | "local_server_token" | "ak_sk";

/** `services::accounts::BalanceKind`。 */
export type BalanceKind = "paid" | "grant" | "available" | "credit";

/** `services::accounts::QuotaUnit`。 */
export type QuotaUnit = "tokens" | "requests" | "credits" | "currency" | "percent" | "vendor_unit";

/** `services::accounts::PeriodKind`。 */
export type PeriodKind = "rolling" | "calendar" | "fixed" | "unknown";

/** `domain::Quality`。 */
export type Quality = "reported" | "derived" | "estimated" | "unknown";

/** `CredentialDto`：账户的凭据引用视图（不含秘密，也不含 vault_ref）。 */
export interface CredentialDto {
  credentialId: string;
  credentialType: CredentialType;
  /** 掩码，仅保留可辨识尾部（如 `********mnop`）。 */
  mask: string;
  fingerprint: string;
  /** 该凭据声明的能力（JSON）；无声明为 null。 */
  capabilities: unknown | null;
  createdAt: string;
  /** 退休时间：更换 Key 后旧引用保留行与时间信息。 */
  retiredAt: string | null;
}

/** `BalanceEntry`：单条余额（金额是十进制字符串，不用二进制浮点）。 */
export interface BalanceEntry {
  kind: BalanceKind;
  amount: string;
  /** ISO 货币或厂商单位（如 USD / credits）；不同币种/单位分行，绝不合并。 */
  currency: string;
  walletScope: string | null;
  expiresAt: string | null;
  /** 来源公式（如 `total = granted + topped_up`），用于避免“已含项”重复相加。 */
  formula: string | null;
  quality: Quality;
}

/** `QuotaWindow`：额度窗口。`windowId` 是平台身份，不能只用 “5h”。 */
export interface QuotaWindow {
  windowId: string;
  label: string | null;
  resource: string | null;
  scope: string | null;
  unit: QuotaUnit;
  periodKind: PeriodKind | null;
  /** 实际已知才填；没有证据为 null，绝不推断“默认 7 天”。 */
  durationSeconds: number | null;
  startsAt: string | null;
  resetsAt: string | null;
  used: string | null;
  limit: string | null;
  remaining: string | null;
  usedRatio: string | null;
  remainingRatio: string | null;
  rawRatio: string | null;
  overLimit: boolean | null;
  quality: Quality;
  evidenceRef: string | null;
}

/** `UsageMetric`：账户级用量指标（不是现金余额，不参与余额合计）。 */
export interface UsageMetric {
  name: string;
  value: string;
  unit: QuotaUnit;
  scope: string | null;
  quality: Quality;
}

/** `Snapshot`：一次成功查询的标准化快照。 */
export interface Snapshot {
  snapshotId: number;
  accountId: string;
  credentialId: string | null;
  /** 本机请求时间（RFC3339）。 */
  queriedAt: string;
  /** 平台数据时间；未知为 null，不用 queriedAt 顶替。 */
  providerAsOf: string | null;
  status: SnapshotStatus;
  evidence: string | null;
  errorCode: string | null;
  expiresAt: string | null;
  balances: BalanceEntry[];
  quotaWindows: QuotaWindow[];
  usageMetrics: UsageMetric[];
}

/** `AccountSummaryDto`：账户卡片所需的全部字段。 */
export interface AccountSummaryDto {
  accountId: string;
  platform: string;
  region: string | null;
  displayName: string;
  configKind: ConfigKind;
  createdAt: string;
  credentials: CredentialDto[];
  /** 最近成功快照；没有成功查询为 null。 */
  snapshot: Snapshot | null;
  /** 最近成功快照已过期（其后有失败查询）。 */
  stale: boolean;
  /** 最近一行查询的状态（含失败），无任何查询为 null。 */
  latestStatus: SnapshotStatus | null;
  /** Key is present in this process; false after restart until re-entered. */
  credentialReady: boolean;
}

/** `account_save` 入参（lib.rs `AccountSaveInput`）。 */
export interface AccountSaveInput {
  /** null = 新建账户；有值 = 更新既有账户。 */
  accountId: string | null;
  platform: string;
  region: string | null;
  displayName: string;
  scope: string | null;
  configKind: ConfigKind;
  credentialType: CredentialType | null;
  /** 仅在提交表单瞬间传入；后端写入本地明文凭据文件，响应不回显。 */
  secret: string | null;
  capabilities: unknown | null;
}

/** `account_rotate_credential` 入参（lib.rs `AccountRotateInput`）。 */
export interface AccountRotateInput {
  credentialId: string;
  secret: string;
  capabilities: unknown | null;
}

/** `account_delete` 入参（lib.rs `AccountDeleteInput`）。 */
export interface AccountDeleteInput {
  accountId: string;
  /** 默认保留历史快照；false 时连同快照一并删除。 */
  keepHistory: boolean;
}

/** `SnapshotOutcome`：`account_refresh` 的返回（成功或失败后保留旧值）。 */
export interface SnapshotOutcome {
  accountId: string;
  status: SnapshotStatus;
  /** 本次写入的快照 id（成功）或最近成功快照 id（失败时旧值保留）；都没有为 null。 */
  snapshotId: number | null;
  queriedAt: string;
  providerAsOf: string | null;
  evidence: string | null;
  errorCode: string | null;
  balances: BalanceEntry[];
  quotaWindows: QuotaWindow[];
  usageMetrics: UsageMetric[];
  /** 最近成功快照是否已过期（失败后仍保留旧值）。 */
  stale: boolean;
}

/** `services::cleanup::CredentialCleanupSummary`。 */
export interface CredentialCleanupSummary {
  refsRemoved: number;
  vaultEntriesRemoved: number;
  sharedRefsKept: number;
}

/** `AccountDeleteResult`：删除本地账户记录的结果说明。 */
export interface AccountDeleteResult {
  accountId: string;
  credentials: CredentialCleanupSummary;
  snapshotsRemoved: number;
  historyKept: boolean;
  /** 稳定文案键（`cleanup.*`），前端映射为中文说明。 */
  noteKeys: string[];
}
