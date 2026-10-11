# Codex App Server（B07）合成夹具

来源：全部为本仓库手工编写的**合成数据**，不含真实账户、令牌、thread 或用量。
语义依据：`docs/research/evidence/accounts-b01-b07-candidates.md` §B07（E1）与官方
`openai/codex@8c0f7011674145da7e10627769fb221c74fbf7a0` 的
`codex-rs/app-server-protocol/schema/{json,typescript}/v2/*`（`GetAccountRateLimitsResponse`、
`RateLimitSnapshot`、`RateLimitWindow`、`GetAccountTokenUsageResponse`、
`AccountTokenUsageSummary`、`ThreadUsage`、`InitializeResponse`）以及
`codex-rs/app-server/src/outgoing_message.rs` 的序列化断言样例。

**线上字段是 camelCase**（Rust 侧 `#[serde(rename_all = "camelCase")]`）；响应/请求信封
为 JSON-RPC 2.0 形态 `{"method","params","id"}` → `{"id","result"}` 或 `{"id","error"}`。
预期值由人工按证据语义独立计算（下文给出算术过程），不是 parser 输出反推。

## 归一化规则

- 协议**无数字版本号**：先 `initialize`（`clientInfo` + `capabilities.experimentalApi`）
  协商，响应 `userAgent` 等写入 evidence。
- `RateLimitWindow = { usedPercent, windowDurationMins|null, resetsAt(unix 秒)|null }`：
  `usedPercent` 是 **0–100 百分比**（源码多处按 `>= 100` 判满）→ `used_ratio = usedPercent/100`，
  `remaining_ratio = 1 − used_ratio`；`duration_seconds = windowDurationMins × 60`；
  `resets_at` = Unix 秒 → RFC3339。rate limit 窗口是 rolling。
- `rateLimitsByLimitId`（多桶，键为 metered `limit_id`）存在且非空时优先，否则用单桶
  `rateLimits`；`window_id = codex:account:{limit_id}:{primary|secondary}`。
- `account/usage/read`：`summary` 各值可为 null → 只把非 null 的映射为十进制字符串指标
  （`lifecycleTokens` 等），全 null 时不产出指标；`dailyUsageBuckets` 可 null。
- **API-key-only / Bedrock 不支持 usage**：服务端回 “chatgpt authentication required …”，
  适配器映射 `unsupported`（能力缺失）；完全未登录回 “codex account authentication required …”
  映射 `auth_required`；`-32601`（method not found）映射 `unsupported`。

## 用例

### rate-limits
- `primary.usedPercent = 25` → 25 / 100 = **0.25**；remaining **0.75**；
  `windowDurationMins = 300` → **18000** s；`resetsAt = 1791705600` → **2026-10-11T08:00:00Z**
- `secondary.usedPercent = 60` → **0.6** / **0.4**；`windowDurationMins = 10080` → **604800** s；
  `resetsAt = 1792310400` → **2026-10-18T08:00:00Z**
- `planType = "plus"`、`accountId = "acct-synthetic"`、`ordinaryUsageAllowed = true`；
  `credits = {hasCredits:true, unlimited:false, balance:"12.50"}`（原样透出，不转 Balance）

### usage
- `lifetimeTokens = 123456789` → 指标 `lifetime_tokens`（tokens）
- `peakDailyTokens = 5000000` → `peak_daily_tokens`（tokens）
- `longestRunningTurnSec = 3600` → `longest_running_turn_sec`（seconds）
- `currentStreakDays = 5` → `current_streak_days`（days）
- `longestStreakDays = 30` → `longest_streak_days`（days）
- `dailyUsageBuckets` 2 条原样透出；`threadUsage = null` → 无 thread usage

### usage-thread
`summary` 全 null → **不产出指标**（没有就省）；`threadUsage` 存在 → 透出 thread 估算用量
（`estimatedUsageCreditsMicros = 46000000`，group 的 `totalTokens = 140`）。

### initialize
`userAgent = "codex_cli_rs/0.50.0 (Mac OS 15.0; arm64)"`、`platformFamily = "unix"`、
`platformOs = "macos"`（`codexHome` 属敏感路径，适配置器**不保留**）。

### error-*
- `error-method-not-found`：`code = -32601` → `unsupported`
- `error-chatgpt-required`：chatgpt 认证要求（API-key-only/Bedrock）→ `unsupported`
- `error-auth-required`：未登录 → `auth_required`

## expected.json 字段

- `rate-limits`：`plan_type`、`account_id`、`ordinary_usage_allowed`、`credits`、`windows[]`
  （`window_id`、`label`、`unit`、`period_kind`、`duration_seconds`、`resets_at`、`used_ratio`、`remaining_ratio`）。
- `usage` / `usage-thread`：`metrics[]`、`daily_buckets[]`、`thread_usage`。
- `initialize`：`user_agent`、`platform_family`、`platform_os`。
