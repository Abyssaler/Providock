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
- 响应取 `json.data || json`；业务失败（HTTP200 上 `code` 非 0/200，或无数值码时
  `success=false`）按失败处理。**数值 `code` 先于 `success` 判定**（2026-10-12 修复）：
  真实 401 信封 `{"code":401,"msg":"token expired or incorrect","success":false}` 若先看
  `success` 会被误分类为 `success_false`（unavailable），修复后正确落入 `auth_required`。
  此前没有任何夹具编码旧误判，故无需改动既有 expected.json。
- `limits[]` 按 `type` 归一：`TOKENS_LIMIT` → 5 小时 Token 窗口；`TIME_LIMIT` → 月度 MCP 用量。
  只拿得到 `percentage`（**0–100**，可能超 100），故 `QuotaWindow` 只填 `used_ratio` /
  `remaining_ratio`，**不反推绝对 Token/请求数**。超 100% 时 `used_ratio` 封顶 1、
  `remaining_ratio` 0，原比例进 `raw_ratio`、`over_limit=true`。
  `CREDIT_LIMIT`（新积分制套餐）官方插件**未解析**（2026-10-12 复核固定提交与 main 分支），
  按规则不从第三方线索实现：保持 `unknown_limit_type` 诊断，待官方依据。

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
`code = 401`（非 0/200，无 `success` 字段）→ 适配器返回 `Business { code: "401" }`，映射 `auth_required`。

### business-error-auth（真实 401 信封形态，无 expected.json）
`{"code":401,"msg":"token expired or incorrect","success":false}`：数值 `code` 优先于
`success:false` 判定 → `Business { code: "401" }` → `auth_required`。
这是 2026-10-12 修复误分类（原先落入 `success_false` / unavailable）的回归夹具。

### no-plan（无套餐业务应答，无 expected.json）
`{"code":500,"msg":"当前用户不存在coding plan","success":false}`（2026-10-12 按量账户在
`open.bigmodel.cn` 与 `api.z.ai` 双站的实测原文）→ 适配器返回
`NoPlan { code: "500" }`，映射独立状态 `no_plan`（UI 显示“无 Coding Plan 套餐”，
不是通用失败也不是凭据失效）。

## expected.json 字段

- `plan_level`：`data.level`（可空）。
- `windows[]`：`window_id`、`label`、`resource`、`scope`、`unit`、`used_ratio`、
  `remaining_ratio`、`raw_ratio`、`over_limit`。
- `diagnostics[]`：解析诊断子串。
