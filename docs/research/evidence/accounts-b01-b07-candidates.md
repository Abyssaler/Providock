# 账户查询证据登记（T00）：B01–B07 与候选审计

核查日期：2026-10-11（所有 `checked_at` 均为该日）。

本文只做公开文档与公开源码的 GET 阅读，未使用任何真实凭据、未发送推理请求、未查询任何真实账户，因此所有结论是“接口依据”而非“实测可用”。证据等级含义见 `docs/specs/ACCOUNT_ADAPTERS.md` 第 1 节：E1 官方公开 API/本机 Server 文档；E2 官方客户端或官方插件源码中的只读接口；E3 只有第三方源码/社区文章；E0 无接口依据或官方已停用。

## 0. 固定基线

| 对象 | 固定提交 | 日期/说明 |
| --- | --- | --- |
| cc-switch（线索来源，非依据） | `df40d53c36b604e044b4d5b9091aeb961aa56990` | 研究时 main，版本线 v4.0.8 |
| GLM 官方插件 zai-coding-plugins | `0446d0bb0bc537d97d3ab3664c4b8b9c4a0e1254` | B05 依据 |
| openai/codex（App Server 协议） | `8c0f7011674145da7e10627769fb221c74fbf7a0` | main HEAD，2026-10-11T06:08:41Z |
| anomalyco/opencode（OpenCode 官方仓库） | `e31136bdbdb43e934d8bbd81e75a2065a2625699` | dev 分支 HEAD |
| MoonshotAI/kimi-code（Kimi Code 官方 CLI） | `c9f01f07388abc2cdca40c9c5267f58c65086710` | B06/候选依据 |
| google-gemini/gemini-cli（Gemini 官方 CLI） | `9b6e0265d16bbd29ca51e33c9e0c01dc4cec5e83` | main HEAD，2026-10-09T21:57:32Z |

说明：主分支与文档页会变化，以上结论只针对上述固定提交与 2026-10-11 抓取的文档快照。cc-switch / CodexBar / 各类 dsh 插件只作为“线索”，一律不作为接入许可依据。

---

## A. 优先项 B01–B07

### B01 DeepSeek API 余额 — E1

| 字段 | 值 |
| --- | --- |
| method | GET |
| exact host | `api.deepseek.com` |
| path | `/user/balance` |
| auth_scheme | `Authorization: Bearer <API_KEY>` |
| credential_type | api_key |
| scope | Key 所属账户（账户级，不逐 Key） |
| unit | 由 `currency` 决定（`CNY` / `USD`），金额为字符串小数 |
| source_url | https://api-docs.deepseek.com/zh-cn/api/get-user-balance/ |
| source_commit | 无（官方文档，非源码） |
| checked_at | 2026-10-11 |

响应 schema（自文档 Schema 段逐字摘录）：

```
{
  "is_available": true,
  "balance_infos": [
    { "currency": "CNY", "total_balance": "110.00",
      "granted_balance": "10.00", "topped_up_balance": "100.00" }
  ]
}
```

关键原文（官方文档字段说明）：

- `is_available`：boolean，当前账户是否有余额可供 API 调用
- `total_balance`：string，**总的可用余额，包括赠金和充值余额**
- `granted_balance`：string，未过期的赠金余额
- `topped_up_balance`：string，充值余额

总额包含关系：`total_balance = granted_balance + topped_up_balance`，按每个币种条目分别成立；多币种时按 `currency` 分开显示，不可跨币种相加。

边界：不返回逐 Key 消费；赠金与充值不可混算；`total_balance` 为字符串，解析要兼容字符串与数字。

### B02 OpenRouter 当前 Key 用量/预算 — E1

| 字段 | 值 |
| --- | --- |
| method | GET |
| exact host | `openrouter.ai` |
| path | `/api/v1/key` |
| auth_scheme | `Authorization: Bearer <token>`（当前推理 Key 本身即可） |
| credential_type | api_key |
| scope | 当前 Key 及其成员/工作区预算（`data.budgets[].scope` 可为 `member`） |
| unit | USD |
| source_url | https://openrouter.ai/docs/api/api-reference/api-keys/get-current-api-key |
| checked_at | 2026-10-11 |

响应 schema（200，`data` 对象，逐字摘录字段名）：

```
data: {
  limit, limit_remaining, limit_reset,
  usage, usage_daily, usage_weekly, usage_monthly,
  byok_usage, byok_usage_daily, byok_usage_weekly, byok_usage_monthly,
  is_free_tier, is_management_key, is_provisioning_key,
  budgets: [{ budget_id, guardrail_id, include_byok, limit_usd,
              remaining_usd, reset_interval, resets_at, scope, spend_usd }],
  effective_budget: { ...同上结构... },
  free_model_daily_requests: { limit, remaining, used },
  rate_limit: { interval, note: "deprecated", requests },
  expires_at, label, organization_id, workspace_id, creator_user_id,
  allowed_data_regions
}
```

关键原文：`Authorization string header required — API key as bearer token in Authorization header`；接口描述为 “Get information on the API key associated with the current authentication session”。文档 200/401/500 三种响应，401 为 `Missing Authentication header`。

边界：这是 **Key/成员预算**（`limit` = 该 Key 的消费上限，未设限时可能为 `null`），**不能冒充账户现金余额**；`rate_limit` 字段已被官方标注 deprecated。要拿账户现金需走 B03。

### B03 OpenRouter 账户 Credits — E1（需管理密钥）

| 字段 | 值 |
| --- | --- |
| method | GET |
| exact host | `openrouter.ai` |
| path | `/api/v1/credits` |
| auth_scheme | `Authorization: Bearer <MANAGEMENT_KEY>` |
| credential_type | management_key |
| scope | 账户（purchased − used） |
| unit | USD（credits） |
| source_url | https://openrouter.ai/docs/api/api-reference/credits/get-credits |
| checked_at | 2026-10-11 |

响应 schema：`{ "data": { "total_credits": 100.5, "total_usage": 25.75 } }`。错误：401 `Missing Authentication header`；**403 `Only management keys can perform this operation`**；500。

关键原文：“Get total credits purchased and used for the authenticated user. **Management key required.**”

边界：普通推理 Key 不保证有权限（会 403），UI 必须提示需要管理密钥；可用额按官方语义为 `total_credits − total_usage`，不要用普通 Key 去试探该接口。

### B04 Novita 余额 — E1

| 字段 | 值 |
| --- | --- |
| method | GET |
| exact host | `api.novita.ai` |
| path | `/openapi/v1/billing/balance/detail` |
| auth_scheme | `Authorization: Bearer <API_KEY>`，且请求头 `Content-Type: application/json`（文档标 required） |
| credential_type | api_key |
| scope | 账户 |
| unit | **1/10000 USD**（文档原文：`10000 = $1.00`） |
| source_url | https://docs.novita.ai/api-reference/basic-get-user-balance |
| checked_at | 2026-10-11（页面 Last modified: May 25, 2026） |

响应 schema：

```
{ "availableBalance": "1000000", "cashBalance": "800000",
  "creditLimit": "200000", "pendingCharges": "0", "outstandingInvoices": "0" }
```

字段说明（逐字）：`availableBalance` 用户 credit 余额；`cashBalance` 充值剩余余额；`creditLimit` 授信上限（最多可欠）；`pendingCharges` 待结算（目前默认 0）；`outstandingInvoices` 当前欠款金额。全部单位为 1/10000 USD。

边界：解析时除以 10000 得 USD；cc-switch 仍保留旧 `/v3/user/balance` 路径线索，本项目**采用当前官方路径**。

### B05 GLM Coding Plan（中国/国际） — E2

| 字段 | 值 |
| --- | --- |
| method | GET |
| exact host | 中国：`open.bigmodel.cn`（`dev.bigmodel.cn` 亦可）；国际：`api.z.ai`。host 由 `ANTHROPIC_BASE_URL` 的 origin 决定，不是硬编码 |
| path | `/api/monitor/usage/quota/limit`、`/api/monitor/usage/model-usage`、`/api/monitor/usage/tool-usage` |
| auth_scheme | `Authorization: <token>` —— **原 token，不加 `Bearer`**（源码头是 `'Authorization': authToken`） |
| credential_type | api_key（`ANTHROPIC_AUTH_TOKEN`） |
| scope | 该账号 Coding Plan（**官方插件仅支持个人版**） |
| unit | `quota/limit`：`TOKENS_LIMIT` 为百分比（5 小时 Token 窗口）；`TIME_LIMIT` 为 MCP 月度用量（计数/时间，含 `currentValue`、`usage`、`usageDetails`）。`model-usage`/`tool-usage` 为明细统计 |
| source_url | https://raw.githubusercontent.com/zai-org/zai-coding-plugins/0446d0bb0bc537d97d3ab3664c4b8b9c4a0e1254/plugins/glm-plan-usage/skills/usage-query-skill/scripts/query-usage.mjs |
| source_commit | `0446d0bb0bc537d97d3ab3664c4b8b9c4a0e1254` |
| checked_at | 2026-10-11 |

平台判定与 URL 构造（源码逐字）：
`baseUrl.includes('api.z.ai')` → 国际；`open.bigmodel.cn` 或 `dev.bigmodel.cn` → 中国；否则报错。三条路径都拼在 `${protocol}//${host}` 之后。

请求细节：
- `model-usage`、`tool-usage` 追加查询串 `?startTime=<yyyy-MM-dd HH:mm:ss>&endTime=<yyyy-MM-dd HH:mm:ss>`（本地时区，昨天当前小时 ~ 今天当前小时）；`quota/limit` **不带**查询串。
- 额外头 `Accept-Language: en-US,en`、`Content-Type: application/json`。
- 响应取 `json.data || json`。

`quota/limit` 归一化（`processQuotaLimit`）：`limits[]` 中 `type === 'TOKENS_LIMIT'` → `percentage`；`type === 'TIME_LIMIT'` → `percentage`、`currentValue`、`usage`（源码写死键名 `totol` 为笔误）、`usageDetails`。

边界：**不是现金余额**；只有百分比就只显示百分比，不能反推 Token；`?type=2` 团队版与个人版同路径（见候选审计 Zhipu 团队版）。上线前须重新确认 token 头格式（是否加 Bearer）与路径未变。

### B06 Kimi 官方本机 Server — E1（官方标注实验接口）

| 字段 | 值 |
| --- | --- |
| method | GET |
| exact host | 本机 `127.0.0.1`（默认端口 `58627`，端口占用则顺延，最多重试 100 次；`--port`/`--host` 可改） |
| base | `http://127.0.0.1:58627` |
| path | `/api/v1/meta`、`/api/v1/sessions/{session_id}/snapshot`、`/api/v1/oauth/usage` |
| auth_scheme | `Authorization: Bearer <token>`；除 `GET /api/v1/healthz`、`OPTIONS` 预检、非 `/api/` 静态资源外，所有 `/api/*` 都要求 token。认证失败 HTTP 401 + 信封 code 40101 |
| credential_type | local_server_token |
| scope | 本机 Server 实例 / 托管 Kimi 账号 |
| unit | `oauth/usage`：`usedRatio` 为 0–1 浮点；`extraUsage` 金额单位为 **cents**（`balanceCents` 等）+ `currency` |
| source_url | https://www.kimi.ai/help/kimi-code/cli-server-api |
| checked_at | 2026-10-11 |

统一响应信封：`{ "code": 0, "msg": ..., "data": ..., "request_id": "..." }`；HTTP 状态几乎总是 200，业务结果看 `code`。

- `GET /api/v1/meta` → `data`：`server_version`、`capabilities`（`websocket`/`file_upload`/`fs_query`/`mcp`/`tasks`/`terminal` 恒为 true）、`server_id`、`started_at`(ISO 8601)、`open_in_apps`、`dangerous_bypass_auth`、`backend`（v1/v2，当前恒 v2）、`web_title`、`experimental_flags`、`features[]`。
- `GET /api/v1/sessions/{session_id}/snapshot` → `data`：`{ as_of_seq, epoch, session, messages, in_flight_turn, subagents?, pending_approvals, pending_questions }`。内嵌 `session.usage` 为 token 汇总 `{ input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, context_tokens, context_limit?, total_cost_usd?, turn_count? }`；只有 snapshot 端点带真实 usage，`session.agent_config.model` 也只有在此处为实时模型。40401 = session not found。
- `GET /api/v1/oauth/usage` → `data` 二选一：`{ kind: "ok", quota }` 或 `{ kind: "error", message, status? }`（上游失败以 in-band 返回，不使信封失败）。`quota = { usages, extraUsage }`：`usages` 为存在即显示的窗口 `limit5h` / `limit7d` / `monthTotal` / `monthCode`，每项 `{ usedRatio, resetAt? }`（`usedRatio` 0–1，`resetAt` RFC3339）；`extraUsage`（可为 null）为按量付费钱包 `{ balanceCents, totalCents, monthlyChargeLimitEnabled, monthlyChargeLimitCents, monthlyUsedCents, currency }`。

边界：官方**明确标注为实验接口**（“interface stability is not guaranteed … endpoints, fields, and event types may change in any release”）；须按实例实际 `/openapi.json` 协商版本与字段，不视为长期稳定厂商公开 API。Server 不在线时显示不可用，**不得为此启动推理或拉起进程**。

### B07 Codex 官方 App Server 账户能力 — E1

| 字段 | 值 |
| --- | --- |
| method | JSON-RPC 请求（`{"method","params","id"}`，响应回 `result`/`error`）；传输：stdio（默认）、`ws://`、`unix://` |
| exact host | 本机进程（无远端 host）；`initialize` 无版本号协商 |
| method 名 | `account/read`、`account/rateLimits/read`、`account/usage/read`（另有 `account/rateLimitResetCredit/consume` 等） |
| auth | 由 App Server 持有：ChatGPT / 外部 ChatGPT tokens（`chatgptAuthTokens`）/ agent identity / personal access token。**`account/usage/read` 不支持 API-key-only 与 Bedrock 认证** |
| scope | ChatGPT 账户；rate limit 可为 workspace/member 维度 |
| unit | `usedPercent` 百分比、`windowDurationMins` 分钟、`resetsAt` Unix 秒；usage 为 token 计数与 credits micros |
| source_url | https://learn.chatgpt.com/docs/app-server |
| source_commit | `8c0f7011674145da7e10627769fb221c74fbf7a0` |
| checked_at | 2026-10-11 |

协议版本协商方式（**无数字版本号**，靠能力协商）：客户端先发 `initialize`，参数 `clientInfo { name, title, version }` + 可选 `capabilities`；`capabilities.experimentalApi = true` 才启用被标记的实验方法/字段（否则报 `<descriptor> requires experimentalApi capability`）。`initialize` 响应返回 `user_agent`、`codex_home`、`platform_family`、`platform_os`。官方另提供 `codex app-server generate-ts` / `generate-json-schema` 生成**与该 Codex 版本精确对应**的 schema，可用于版本对齐。（固定提交中 `account/rateLimits/read` 与 `account/usage/read` **未**被 `#[experimental]` 标注。）

`account/rateLimits/read`：
- params（可空）：`{ supports_luna_reserve?: bool, exclude_reset_credit_details?: bool }`
- result：`{ ordinary_usage_allowed?: bool, rate_limits: RateLimitSnapshot, rate_limits_by_limit_id?: {[limit_id]: RateLimitSnapshot}, rate_limit_reset_credits?: {...}, account_id?: string, rate_limit_upsell?: object }`
- `RateLimitSnapshot`：`{ limit_id, limit_name?, primary?: RateLimitWindow, secondary?: RateLimitWindow, credits?, rate_limit_reached_type? }`；`RateLimitWindow = { used_percent, window_duration_mins, resets_at(unix 秒) }`
- `rate_limit_reset_credits = { available_count, credits?: [{ id, reset_type, status, granted_at, expires_at?, title?, description? }] }`（`credits` 为 null 表示只知数量；空数组表示已查详情但无可用）

`account/usage/read`（源码结构 `GetAccountTokenUsage*`）：
- params（可空）：`{ threadId? }`（给了就读该 thread 的估算用量）
- result：`{ summary: { lifetimeTokens, peakDailyTokens, longestRunningTurnSec, currentStreakDays, longestStreakDays }, daily_usage_buckets?: [{ startDate, tokens }], thread_usage?: { threadId, estimated_usage_credits_micros, estimated_usage_usd_micros?, groups: [{ model?, reasoning_effort?, speed?, estimated_usage_credits_micros, net_new_input_tokens?, cached_input_tokens?, input_tokens?, output_tokens?, total_tokens? }] } }`
- summary 各值可为 null；`daily_usage_buckets` 可为 null

边界：只读方法，不驱动 turn；没有相应能力则降级。字段名以 `#[serde(rename_all = "camelCase")]` 序列化为驼峰。第三方（如 cc-switch 的 `chatgpt.com/backend-api/wham/*`）走的是私有后端路由，**不作为依据**。

---

## B. 候选审计

结论逐项给出 E 等级与理由。E1/E2 可转实现任务；E3 仅研究线索；E0 不查询、不显示假余额（账户条目仍可管理）。

| # | 平台/类别 | 结论 | 依据与理由 |
| --- | --- | --- | --- |
| B-01 | StepFun API `/v1/accounts` | **E1** | 官方文档：GET `https://api.stepfun.com/v1/accounts`，`Authorization: Bearer <KEY>`。字段 `object`（固定 `account`）、`type`（`prepaid`/`postpaid`）、`balance`（可用余额）、`total_cash_balance`（总充值）、`total_voucher_balance`（总赠送），单位 ¥。来源：https://platform.stepfun.com/docs/zh/api-reference/accounts/get |
| B-02 | SiliconFlow 中国 `/v1/user/info` | **E0（官方已停用）** | 官方发布记录写明：`/user/info` “将于 **2026-08-14（周五）** 正式停止服务，届时接口将不再可用”，并称“后续平台将适时提供账户层面的替代 API”，截至目前**无替代接口公告**。国内文档页 `docs.siliconflow.cn/en/api-reference/userinfo/get-user-info` 现已 404。来源：https://docs.siliconflow.cn/docs/release-notes/overview |
| B-03 | SiliconFlow 国际站 | **E1** | 国际文档 `docs.siliconflow.com` 仍在线收录 GET `https://api.siliconflow.com/v1/user/info`（Bearer），返回 `{code,message,status,data{id,name,image,email,isAdmin,balance,status,introduction,role,chargeBalance,totalBalance}}`。国际站发布记录中**未见** `/user/info` 停用公告，不能由中国公告类推同日停用。单位/币种页面未明确标注，须实测确认。来源：https://docs.siliconflow.com/en/api-reference/userinfo/get-user-info |
| B-04 | Kimi Coding Key 直接查询 `https://api.kimi.com/coding/v1/usages` | **E2** | 官方 CLI 源码（MoonshotAI/kimi-code `c9f01f0`，`packages/oauth/src/managed-usage.ts`）：`DEFAULT_KIMI_CODE_BASE_URL = 'https://api.kimi.com/coding/v1'`，`GLOBAL_KIMI_CODE_BASE_URL = 'https://api.kimi.ai/coding/v1'`，请求 `${base}/usages`，头 `Authorization: Bearer <accessToken>`。上游原始 payload 为 snake_case：`usages.{limit_5h,limit_7d,limit_month_total,limit_month_code}` 每项 `{used_ratio(0–1), reset_time}`，`boosterWallet.{balanceCents,totalCents,monthlyChargeLimitEnabled,monthlyChargeLimitCents,monthlyUsedCents,currency}`；CLI 本地 server 再归一化为驼峰（对应 B06 的 `oauth/usage`）。**该端点未出现在厂商公开 Open Platform 参考中**，契约由官方 CLI 定义，兼容可能随 CLI 变化。来源：https://github.com/MoonshotAI/kimi-code/blob/c9f01f07388abc2cdca40c9c5267f58c65086710/packages/oauth/src/managed-usage.ts |
| B-05 | Zhipu（智谱）团队版 | **E3** | 未找到官方公开文档或官方源码证明团队版额度接口。cc-switch 用 `https://open.bigmodel.cn/api/monitor/usage/quota/limit?type=2` + 头 `bigmodel-organization` / `bigmodel-project`，但这是第三方反推。官方 zai-coding-plugins 的 `query-usage.mjs` **仅个人版**，且智谱官方文档 `docs.bigmodel.cn/cn/coding-plan/extension/usage-query-plugin` 明确写“仅支持个人版套餐”。第三方 issue（sub2api#6266）亦称团队版拿不到额额度。故团队版仅 E3，不得把个人 Key 当团队凭据。 |
| B-06 | MiniMax `coding_plan/remains` | **E1（但路径应为 `/v1/token_plan/remains`；`coding_plan/remains` 路径 E3/E0）** | 官方 FAQ 明确给出“方式三：使用 API 接口查询”：中国站 `curl https://www.minimax.cn/v1/token_plan/remains -H 'Authorization: Bearer <API Key>' -H 'Content-Type: application/json'`；国际站为 `https://www.minimax.io/v1/token_plan/remains`。cc-switch 使用的 `/v1/api/openplatform/coding_plan/remains` 在官方文档中**查无此路径**。额度窗口口径：非视频模型受 5 小时+周窗口，视频仅受周窗口；额度用完不自动扣按量余额。来源：https://platform.minimax.cn/docs/m-plan/faq 、https://platform.minimax.io/docs/m-plan/faq |
| B-07 | ZenMux Plan | **E1（需 Management API Key）** | ZenMux 官方提供 Platform Management API（`Authorization: Bearer <sk-mg-v1-*>`）：`GET https://zenmux.ai/api/v1/management/subscription/detail` 返回 `data.plan{tier,amount_usd,interval,expires_at}`、`quota_5_hour`/`quota_7_day`（`usage_percentage` 0–1、`resets_at`、`max_flows`、`used_flows`、`remaining_flows`、`used_value_usd`、`max_value_usd`）与 `quota_monthly`；`GET https://zenmux.ai/api/v1/management/payg/balance` 返回 `data{currency:"usd", total_credits, top_up_credits, bonus_credits}`（`total = top_up + bonus`）。**仅接受个人账户创建的 Management Key**，普通 API Key / OAuth token / 组织 Key 不支持。来源：https://zenmux.ai/docs/zh/api/platform/subscription-detail.html 、https://zenmux.ai/docs/zh/api/platform/payg-balance.html |
| B-08 | 火山引擎 Agent Plan / Coding Plan | **E1** | 官方管控面 API，签名 V4（AK/SK）。`POST https://ark.cn-beijing.volcengineapi.com/?Action=GetAFPUsage&Version=2024-01-01`（Service=`ark`，Region 如 `cn-beijing`）返回 `Result{PlanType, AFPFiveHour/AFPDaily/AFPWeekly/AFPMonthly 各 {Quota,Used,SubscribeTime,ResetTime}}`；同族官方 Action 还有 `GetUsageDetails`、`GetSeatInfoUsage`、`GetPersonalPlan`、`RenewPersonalPlan`、`CreatePersonalPlan`、`ListArkCodingPlanModel`，分组 “Coding Plan API / Agent Plan API / Agent Plan Team API”。注意 cc-switch 用 host `open.volcengineapi.com`，官方文档示例 host 为 `ark.<region>.volcengineapi.com`，须以官方文档 host 为准。来源：https://docs.volcengine.com/docs/82379/2479847?lang=en 、https://www.volcengine.com/docs/ark/get-usage-details-api |
| B-09 | OpenCode Go `https://opencode.ai/zen/go/v1/usage` | **E2** | 官方仓库（anomalyco/opencode `e31136b`）存在该路由源码 `packages/console/app/src/routes/zen/go/v1/usage.ts`：`GET`，从 `authorization` 头匹配 `^Bearer (\S+)$` 取 workspace key，响应 `{usage:{rolling,weekly,monthly:{status:"ok"|"rate-limited", percent(已用整数0–100), resetsAt(ISO8601)}}}`；401 缺/错 Key，403 `EntitlementError "OpenCode Go subscription required."`。**第一方但未文档化**（官方 docs `zen.mdx` 未收录该端点），故 E2 而非 E1。来源：https://github.com/anomalyco/opencode/blob/e31136bdbdb43e934d8bbd81e75a2065a2625699/packages/console/app/src/routes/zen/go/v1/usage.ts |
| B-10 | CommandCode `/alpha/usage/summary` | **E3** | 官方文档仅覆盖推理侧 `https://api.commandcode.ai/alpha/generate` 与 `https://commandcode.ai/docs/provider`（OpenAI/Anthropic 兼容），`/alpha/whoami`、`/alpha/billing/credits`、`/alpha/billing/subscriptions`、`/alpha/usage/summary` 等控制面路由**未出现在官方文档**。仅第三方（cc-switch、`pi-commandcode-provider` 等）称其为官方 CLI `/usage` 所用。alpha 路径本身已属实验，故 E3。 |
| B-11 | Claude 订阅 OAuth usage | **E3** | 未找到官方公开文档或官方开源客户端源码证明 `GET https://api.anthropic.com/api/oauth/usage`。该端点仅出现在 cc-switch / CodexBar / 各类第三方抓取器源码中（Claude Code 客户端闭源）。凭据来自本机 `.credentials.json` / Keychain。故 E3；且不得默认扫描本机登录凭据或借用浏览器 Cookie。 |
| B-12 | Gemini `retrieveUserQuota` | **E2** | 官方 CLI（google-gemini/gemini-cli `9b6e0265`）`packages/core/src/code_assist/server.ts`：`CODE_ASSIST_ENDPOINT = 'https://cloudcode-pa.googleapis.com'`、`CODE_ASSIST_API_VERSION = 'v1internal'`，`retrieveUserQuota(req)` → `requestPost('retrieveUserQuota', req)`，即 `POST https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota`；`RetrieveUserQuotaRequest{project}`，`RetrieveUserQuotaResponse{buckets?: BucketInfo[]}`（每桶 `remainingFraction` 0–1、`resetTime`、`modelId`），另有 `creditBalance`。**这是 Code Assist 订阅配额，不等同 Gemini API 现金余额**；项目 ID 需经 `loadCodeAssist`（`cloudaicompanionProject`）解析。来源：https://github.com/google-gemini/gemini-cli/blob/9b6e0265d16bbd29ca51e33c9e0c01dc4cec5e83/packages/core/src/code_assist/server.ts |
| B-13 | GitHub Copilot 账户 | **E3（个人）/ E1（组织·企业）** | 官方 GitHub Docs 明确个人账户**没有** Copilot 配额 REST API（社区讨论 #192642/#185848 确认“组织/企业有 metrics 端点，个人没有”）。个人配额只能走内部端点 `GET https://api.github.com/copilot_internal/user`（第三方使用，无官方文档）→ E3。组织/企业级官方 metrics：https://docs.github.com/rest/copilot/copilot-usage-metrics （需 org/enterprise 管理员权限）→ E1（但属组织范围，非个人账户）。 |
| B-14 | Grok（xAI）账户 | **E3** | 未找到官方公开文档或官方源码证明余额/配额查询接口。cc-switch 用 Grok CLI `/v1/billing?format=credits`（第三方称“官方 CLI 接口”），通用模板落到 `api.x.ai/v1/user/balance` 会 404。故 E3；与 Grok Build 本机用量采集无绑定依赖。 |
| B-15 | Moonshot 开放平台现金 | **E1** | 官方文档：GET `https://api.moonshot.cn/v1/users/me/balance`（Bearer），返回 `{code, data{available_balance, voucher_balance, cash_balance}, scode, status}`；`available_balance ≤ 0` 时推理 API 报 `exceeded_current_quota_error`。国际站为 `platform.kimi.ai` / `api.moonshot.ai`，与国内站 Key 独立（混用 401）。来源：https://platform.moonshot.cn/docs/api/balance |
| B-16 | GLM 现金余额 | **E3** | 官方 Coding Plan 查询（B05）只给计划窗口百分比，不返回现金钱包。未找到官方公开的 GLM 现金余额 API；第三方用控制台主机 `www.bigmodel.cn/api/biz/account/query-customer-account-report`（无官方文档）。故 E3；计划窗口可显示，现金暂不显示。 |
| B-17 | Xiaomi MiMo Token Plan | **E3（近 E0）** | 未找到官方公开余额/额度查询接口。第三方（cc-switch #2488 等）通过控制台 `platform.xiaomimimo.com/console/plan-manage` 的 Cookie 查询 usage，属私有端点。官方仅提供控制台页面与 Token Plan 产品页（https://mimo.mi.com/docs/zh-CN/tokenplan ），不构成可调用接口证据。按用户边界不接入。 |
| B-18 | OpenAI / Anthropic 组织 usage/cost | **E1（需 Admin Key，非充值余额）** | OpenAI 官方 Admin API：`GET /organization/costs`、`/organization/usage/completions` 等，需 org Admin API Key（Bearer），仅 owner 可创建（https://developers.openai.com/api/reference/resources/admin/ ）。Anthropic 官方 Usage & Cost Admin API：`GET /v1/organizations/usage_report/messages`、`/v1/organizations/cost_report`，需 Admin API Key（https://docs.anthropic.com/en/api/usage-cost-api ）。**这是组织 usage/cost 能力，与“充值余额”不是同一接口**；没有充值余额依据时不声称能查余额，更不能用 Codex/Claude 订阅额度替代。 |

---

## C. 需要真实凭据实测的项

以下接口的“依据”已确认，但**单位、权限、字段实际取值、区域差异与稳定性**必须由拥有对应权限的账户持有人在合规环境实测后，才能从“样本验证”升级为“已实测”。本机实测前不得写入公共夹具、不得使用开发者个人账户数据。

1. **B01 DeepSeek**：多币种返回形态、`total_balance` 是否恒等于两项之和、负数/零余额、字符串 vs 数字。
2. **B02 OpenRouter 当前 Key**：`limit` 未设限时的实际返回（null？）、`budgets[].scope` 取值集合、`rate_limit` deprecated 后是否仍返回。
3. **B03 OpenRouter Credits**：普通推理 Key 是否实际 403、管理密钥创建方式、`total_credits − total_usage` 与 UI 是否一致。
4. **B04 Novita**：`creditLimit`/`outstandingInvoices` 在无授信/无欠款账户的实际值、单位换算边界。
5. **B05 GLM Coding Plan**：`Authorization` 是否**确实**不加 Bearer、国际站 `api.z.ai` 与中国站字段差异、`TIME_LIMIT.usage`（源码 `totol` 笔误）真实键名、团队版 `?type=2` 是否可用（当前 E3）。
6. **B06 Kimi 本机 Server**：本机是否运行、`/openapi.json` 实际版本、`oauth/usage` 的 `usages` 键集合与 `extraUsage` 是否非空、信封 `code` 非 0 时的降级。
7. **B07 Codex App Server**：本机 Codex 版本是否支持 `account/usage/read`、`rateLimits` 的 workspace/member 维度、认证方式（API-key-only 明确不支持 usage）。
8. **B-01 StepFun**：`type=postpaid` 时 `balance` 语义与单位。
9. **B-03 SiliconFlow 国际**：币种/单位（文档未标注，需实测确认 USD）。
10. **B-04 Kimi Coding Key**：用 `sk-kimi-*` API Key 与 OAuth token 两种凭据分别验证 `Bearer` 是否都可用、5h/周/月窗口实际返回。
11. **B-06 MiniMax**：`/v1/token_plan/remains` 实际响应结构与单位、中国站 `minimax.cn` 与国际站 `minimax.io` 字段差异。
12. **B-07 ZenMux**：Management Key（`sk-mg-v1-*`）权限验证、`usage_percentage` 精度、PAYG 与订阅口径区分。
13. **B-08 火山引擎**：AK/SK 最小权限、正确 host（`ark.<region>.volcengineapi.com` 对官方文档）、`GetAFPUsage` → `GetCodingPlanUsage` 回落、各窗口 `Quota/Used` 单位（AFP 积分）。
14. **B-09 OpenCode Go**：401/403 分支、`percent` 语义、`resetsAt` 在 percent=0 时的占位值处理。
15. **B-12 Gemini**：`project` 解析、403 PERMISSION_DENIED 场景、`buckets` 与 `creditBalance` 实际字段。
16. **B-15 Moonshot**：`available_balance` / `voucher_balance` / `cash_balance` 三者关系、国际站端点。
17. **B-18 OpenAI/Anthropic**：Admin Key 权限、usage/cost 的时间粒度与分页、与账户充值余额的区分。

---

参考规范：`docs/specs/ACCOUNT_ADAPTERS.md`、`docs/research/UPSTREAM_AUDIT.md`。本文所有官方链接与源码提交均为 2026-10-11 抓取快照；主分支与文档页会变化，实现前须按固定提交或重新抓取复核。
