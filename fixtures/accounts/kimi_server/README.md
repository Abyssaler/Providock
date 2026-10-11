# Kimi 本机 Server（B06）合成夹具

来源：全部为本仓库手工编写的**合成数据**，不含真实账户、令牌或用量。
语义依据：`docs/research/evidence/accounts-b01-b07-candidates.md` §B06 与
`docs/research/evidence/a08-kimi-official.md` §7（E1，官方文档 `cli-server-api` /
`docs/zh/reference/server-api.md`；**官方标注实验接口**）。响应字段逐项照
`packages/oauth/src/managed-usage.ts` 与 `packages/kap-server/src/routes/{meta,oauth}.ts`
的键名构造（固定提交 `c9f01f07388abc2cdca40c9c5267f58c65086710`）。

预期值由人工按证据语义独立计算（下文给出算术过程），不是 parser 输出反推。

## 归一化规则

- 默认 base `http://127.0.0.1:58627`；适配器只接受 loopback，`Authorization: Bearer <本机令牌>`
  （凭据注入在服务层）。统一信封 `{ code, msg, data, request_id }`：**HTTP 几乎总是 200，
  `code != 0` 即业务错误**（40101 = 认证失败；42901 = 限流）。400+ 状态直接按传输层状态。
- 只有 `/api/v1/oauth/usage` 携带计划额度；`data.kind = "ok"` 才有 `quota`，`"error"` 表示
  上游失败（in-band，不使信封失败）。
- `quota.usages` 的 `limit5h / limit7d / monthTotal / monthCode`（存在即显示）各
  `{ usedRatio(0–1), resetAt? }` → 4 个独立 `QuotaWindow`；`duration_seconds` 只在窗口名有据
  时填：5h=18000、7d=604800；月窗按 `resetAt` 推不出周期 → `period_kind=calendar`、duration 留 null。
- `extraUsage`（可空）是**按量付费钱包**，金额单位为 **cents**：`balanceCents`（剩余）→
  `available`，`totalCents`（总额）→ `credit`，均 **÷100 精确换算**为 USD。

## 用例

### meta（版本协商）
`server_version = "0.38.0"`、`backend = "v2"`；`capabilities` 中值为 true 的键（排序后）
`["file_upload","fs_query","mcp","tasks","terminal","websocket"]`；`features[].name` → `"web-search"`。

### oauth-usage（4 窗口 + 钱包）
- `limit5h.usedRatio = 0.42` → used_ratio **0.42**，remaining **0.58**（1 − 0.42）；rolling，18000s
- `limit7d.usedRatio = 0.61` → **0.61** / **0.39**；rolling，604800s
- `monthTotal.usedRatio = 0.33` → **0.33** / **0.67**；calendar，duration null
- `monthCode.usedRatio = 0.12` → **0.12** / **0.88**；calendar，duration null
- `extraUsage.balanceCents = 1234` → 1234 / 100 = **12.34** USD（available）
- `extraUsage.totalCents = 5000` → 5000 / 100 = **50** USD（credit）
  （`available` 已是总值行，分组时不会再叠加 `credit`，避免双算。）

### oauth-usage-null-extra（缺窗口 + 空钱包）
只有 `limit5h`（0.9 / 0.1）；`extraUsage = null` → 无余额行。

### oauth-usage-inband-error（上游失败）
`data.kind = "error"`、`status = 401` → 适配器返回 `UpstreamError`，映射 `unavailable`
（上游临时失败，非“无能力”）。

### business-error（HTTP200 业务码）
`code = 40101` → `Business { code: "40101" }`，映射 `auth_required`。

## expected.json 字段

- `windows[]`：`window_id`、`label`、`resource`、`unit`、`period_kind`、`duration_seconds`、
  `resets_at`、`used_ratio`、`remaining_ratio`。
- `balances[]`：`kind`、`amount`、`currency`、`wallet_scope`、`formula`。
- `diagnostics[]`：解析诊断子串。
