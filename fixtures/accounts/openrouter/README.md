# OpenRouter 账户夹具（B02 / B03）

来源 URL（2026-10-11 快照）：
- B02 当前 Key：https://openrouter.ai/docs/api/api-reference/api-keys/get-current-api-key
- B03 账户 Credits：https://openrouter.ai/docs/api/api-reference/credits/get-credits

依据登记：docs/research/evidence/accounts-b01-b07-candidates.md「B02 / B03」（E1）。
全部为**合成**响应，按官方文档字段手工构造，不含任何真实账户数据。

## B02 `GET /api/v1/key`（Bearer 推理 Key）

这是 **Key / 成员预算，不是账户现金余额**，因此 `balances` 为空，只产出 `quota_windows`
（`scope=key`，单位 USD）。映射字段：`limit` / `limit_remaining` / `limit_reset`、
`usage`、`usage_daily` / `usage_weekly` / `usage_monthly`、`free_model_daily_requests`。
`limit` 未设限时为 `null`（合法，非 schema 变化）。已弃用的 `rate_limit` 不映射。

- `key-usage/`：完整字段。手工换算：`limit_remaining 74.25 = limit 100 − usage 25.75`（源直接给出）。
  `free_model_daily_requests` 为请求数单位：`remaining 37 = limit 50 − used 13`。

## B03 `GET /api/v1/credits`（需管理密钥）

- `credits/`：`available = total_credits − total_usage = 100.5 − 25.75 = 74.75`（精确十进制相减，质量 derived）。
- `credits-negative/`：`total_usage > total_credits` → `available = 10 − 25.75 = −15.75`，负值保留不放零。
- `credits-forbidden-403/`：普通推理 Key 403（`Only management keys ...`）→ `auth_required`。
- `schema-change/`：缺 `total_usage` → `incompatible`，不补零。
