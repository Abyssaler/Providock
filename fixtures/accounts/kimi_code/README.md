# Kimi Code Coding Key 直连通道（审计 B-04）合成夹具

来源：全部为本仓库手工编写的**合成数据**，不含真实账户、令牌或用量。
语义依据：`docs/research/evidence/accounts-b01-b07-candidates.md` §B-04（E2，官方 CLI
`MoonshotAI/kimi-code@c9f01f07388abc2cdca40c9c5267f58c65086710`
`packages/oauth/src/managed-usage.ts`）。
`usages/` 用例的整体结构（`usage` / `limits` / `booster_wallet` / `usages` 四段并存）
对照 2026-10-12 真实凭据验证过的响应形态构造，**数值全部虚构**。

预期值由人工按证据语义独立计算（下文给出算术过程），不是 parser 输出反推。

## 归一化规则

- 默认 base `https://api.kimi.com/coding/v1`（国际站 `https://api.kimi.ai/coding/v1`），
  `GET {base}/usages`，`Authorization: Bearer <key>`（凭据注入在服务层），
  `Accept: application/json`。
- 只解析 `usages.{limit_5h, limit_7d, limit_month_total, limit_month_code}`，各
  `{used_ratio(0–1), reset_time?}` → `QuotaWindow`；窗口映射（window_id / label /
  period_kind / duration_seconds）与 B06 本机 Server 通道一致：5h/7d 为 rolling
  （18000s / 604800s），月窗为 calendar 且 duration 留 null（`reset_time` 推不出周期）。
- `reset_time` 仅接受 RFC3339，**容忍有无小数秒**，校验后原样保留；非法值记诊断且留空。
- `used_ratio` 缺失/不可解析 ⇒ schema 变化（不补零）；窗口条目不是对象 ⇒ 记诊断并跳过；
  `usages` 里出现未知键 ⇒ 记 `unknown_usage_key:<键>` 诊断。
- `booster_wallet`、`usage`、`limits` 本轮**不解析**（证据不足/CLI 未读取），容忍其存在。
- HTTP 401/403 → `auth_required`；429 → `rate_limited`；错误体
  `{"error":{"message","type"}}` 的 `type` 记入业务码。

## 用例

### usages（4 窗口 + 未解析段）
- `limit_5h.used_ratio = 0.25` → used_ratio **0.25**，remaining = 1 − 0.25 = **0.75**；
  rolling，18000s
- `limit_7d.used_ratio = 0.6` → **0.6** / **0.4**；rolling，604800s；
  `reset_time` 带小数秒（`.250000Z`）原样保留
- `limit_month_total.used_ratio = 0.4` → **0.4** / **0.6**；calendar，duration null
- `limit_month_code.used_ratio = 0.1` → **0.1** / **0.9**；calendar；无 `reset_time` → null
- `booster_wallet` 存在但**不产生任何余额行**；`usage` / `limits` 段被忽略

### usages-minimal（无月窗 + 字符串比例 + 小数秒）
对照 2026-10-12 实测账户形态：该账户**没有** `limit_month_total` / `limit_month_code`，
解析必须容忍月窗缺失（只回存在的窗口）。
- `limit_5h.used_ratio = 0.0835` → **0.0835** / 1 − 0.0835 = **0.9165**；
  `reset_time` 为 6 位小数秒，原样保留
- `limit_7d.used_ratio = "0.5"`（字符串数字）→ **0.5** / **0.5**；无小数秒也接受

### unauthorized-401（无 expected.json，错误在测试内断言）
HTTP 401 + `{"error":{"message":...,"type":"invalid_authentication_error"}}` →
`auth_required`，业务码 `invalid_authentication_error`。

## expected.json 字段

- `windows[]`：`window_id`、`label`、`resource`、`unit`、`period_kind`、`duration_seconds`、
  `resets_at`、`used_ratio`、`remaining_ratio`。
- `diagnostics[]`：解析诊断子串（精确数组比较）。
