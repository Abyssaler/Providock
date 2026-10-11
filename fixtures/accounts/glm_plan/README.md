# GLM Coding Plan（B05）合成夹具

来源：全部为本仓库手工编写的**合成数据**，不含真实账户、令牌或用量。
语义依据：`docs/research/evidence/accounts-b01-b07-candidates.md` §B05（E2）与官方插件
`zai-org/zai-coding-plugins@0446d0bb0bc537d97d3ab3664c4b8b9c4a0e1254`
`plugins/glm-plan-usage/skills/usage-query-skill/scripts/query-usage.mjs`（2026-10-11 抓取）。
响应字段（`code/msg/data.level/data.limits[].type|percentage|currentValue|usage|usageDetails`）
逐项照官方插件读取的键名构造；`nextResetTime` 为上游同族字段，本适配器不读取（测试容错）。

预期值由人工按证据语义独立计算（下文给出算术过程），不是 parser 输出反推。

## 归一化规则

- host 由 `ANTHROPIC_BASE_URL` 的 origin 决定（`api.z.ai` 国际；`open.bigmodel.cn` /
  `dev.bigmodel.cn` 中国）；`Authorization` 是**原 token，不加 `Bearer`**，适配器只声明 scheme。
- `GET /api/monitor/usage/quota/limit` 无查询串；`model-usage` / `tool-usage` 带
  `?startTime=<yyyy-MM-dd HH:mm:ss>&endTime=<...>` 且 `encodeURIComponent` 编码。
- 响应取 `json.data || json`；业务失败（HTTP200 上 `code` 非 0/200 或 `success=false`）按失败处理。
- `limits[]` 按 `type` 归一：`TOKENS_LIMIT` → 5 小时 Token 窗口；`TIME_LIMIT` → 月度 MCP 用量。
  只拿得到 `percentage`（**0–100**，可能超 100），故 `QuotaWindow` 只填 `used_ratio` /
  `remaining_ratio`，**不反推绝对 Token/请求数**。超 100% 时 `used_ratio` 封顶 1、
  `remaining_ratio` 0，原比例进 `raw_ratio`、`over_limit=true`。

## 用例

### quota-limit（2 窗口，包装形态）
- `TOKENS_LIMIT.percentage = 43.5` → used_ratio = 43.5 / 100 = **0.435**；remaining = 1 − 0.435 = **0.565**
  - window_id `glm:coding_plan:account:5h:tokens`
- `TIME_LIMIT.percentage = 8.0` → used_ratio = 8.0 / 100 = **0.08**；remaining = 1 − 0.08 = **0.92**
  - window_id `glm:coding_plan:account:month:mcp`
- `data.level = "pro"` → `plan_level = "pro"`（仅元信息，不参与窗口）

### quota-limit-unwrapped（无 `data` 包装）
响应直接是 `{ "limits": [...] }`（插件 `json.data || json` 的另一分支）。
- `TOKENS_LIMIT.percentage = "12.5"`（字符串数字）→ **0.125** / **0.875**
- `TIME_LIMIT.percentage = 100`（整数）→ **1** / **0**（恰好满，不算超限）

### business-error（HTTP200 业务错误）
`code = 401`（非 0/200）→ 适配器返回 `Business { code: "401" }`，映射 `auth_required`。

## expected.json 字段

- `plan_level`：`data.level`（可空）。
- `windows[]`：`window_id`、`label`、`resource`、`scope`、`unit`、`used_ratio`、
  `remaining_ratio`、`raw_ratio`、`over_limit`。
- `diagnostics[]`：解析诊断子串。
