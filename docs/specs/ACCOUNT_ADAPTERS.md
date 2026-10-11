# 账户、余额与计划额度适配规范

## 1. 能力原则与证据分级

账户管理独立于 Agent。每个平台、区域和凭据类型都有自己的 capabilities；“支持添加 Key”不代表该 Key 能查询余额。

| 等级 | 依据 | 接入规则 |
| --- | --- | --- |
| E1 | 官方公开 API / 官方本机 Server 文档 | 可实施，标明稳定/实验及版本 |
| E2 | 官方客户端或官方插件源码中使用的只读接口 | 用户已接受；可实施，标“官方客户端接口，兼容可能变化” |
| E3 | 只有 cc-switch / CodexBar 等第三方源码或社区文章 | 仅研究线索；不能发布接入 |
| E0 | 没有接口依据或官方已停止服务 | 不查询，不显示假余额；账户条目仍可管理 |

研究时有源码不等于服务当前可用。所有适配器发布前重新检查官方公告、认证要求和地区。每个接口登记 method、exact host、path、auth_scheme、credential_type、scope、unit、source_url、source_commit、checked_at、schema、fixtures、compatibility。不能靠包含 `deepseek.com` 的字符串判断 host，必须精确匹配规范化域名。

## 2. 已有官方依据的优先实现项

| ID / 能力 | 接口与认证 | 返回/显示 | 状态与边界 |
| --- | --- | --- | --- |
| B01 DeepSeek API 余额 | GET `https://api.deepseek.com/user/balance`；Bearer API Key | balance_infos 中币种、total_balance、granted_balance、topped_up_balance | E1；总额已含赠金与充值，按币种分开；不自动得到逐 Key 消费 |
| B02 OpenRouter Key 用量/预算 | GET `https://openrouter.ai/api/v1/key`；Bearer 当前 Key | usage/limit/remaining 与预算字段，按实际字段版本映射 | E1；这是 Key/成员预算，不能冒充账户现金余额 |
| B03 OpenRouter 账户 Credits | GET `https://openrouter.ai/api/v1/credits`；**管理密钥** | total_credits 与 total_usage，可用额按官方语义求差 | E1；普通推理 Key 不保证有权限；提示所需凭据 |
| B04 Novita 余额 | GET `https://api.novita.ai/openapi/v1/billing/balance/detail`；Bearer API Key | 可用额、现金、授信、待结算、欠款；原值单位 1/10000 USD | E1；优先当前官方文档，不照抄 cc-switch 的旧 `/v3/user/balance` 路径 |
| B05 GLM Coding Plan 中国/国际 | 对应 host 的 `/api/monitor/usage/quota/limit`、`/model-usage`、`/tool-usage` | 多窗口和资源池、模型/工具用量；实际单位映射 | E2：官方 zai-coding-plugins 的 query-usage.mjs；Authorization 是原 token，不擅自加 Bearer；不是现金余额 |
| B06 Kimi Code 官方本机额度 | 用户已运行 Server 的 GET `/api/v1/oauth/usage` | 计划窗口和额外付费钱包，按存在的字段显示 | E1，实验 API；本机令牌不等于厂商推理 Key；Server 不在线时显示不可用，不启动推理 |
| B07 Codex 官方账户能力 | 官方 App Server `account/rateLimits/read`，版本支持时 `account/usage/read` | 官方账户额度窗口、credits/usage 的可用字段 | E1；先协商实际协议版本；只读方法，不驱动 turn；没有相应能力则降级 |

依据：[DeepSeek](https://api-docs.deepseek.com/zh-cn/api/get-user-balance/)、[OpenRouter 当前 Key](https://openrouter.ai/docs/api/api-reference/api-keys/get-current-api-key)、[OpenRouter Credits](https://openrouter.ai/docs/api/api-reference/credits/get-credits)、[Novita](https://docs.novita.ai/api-reference/basic-get-user-balance)、[GLM 官方插件](https://github.com/zai-org/zai-coding-plugins/blob/main/plugins/glm-plan-usage/skills/usage-query-skill/scripts/query-usage.mjs)、[Kimi Server](https://www.kimi.ai/help/kimi-code/cli-server-api)、[Codex App Server](https://learn.chatgpt.com/docs/app-server)。文档/主分支链接会变化，P0 将实际采用的版本或文档摘录哈希加入证据登记。

## 3. 对齐 cc-switch 的逐项候选审计

下列均须在 P0 做证据/认证/schema 验证。只有 E1/E2 可转为实现任务；调查无结果可按用户允许保留管理条目、隐藏查询指标。不得把候选表写成“已支持”。

2026-10-11 已完成一轮逐项审计，E 级结论已更新入表，逐项证据与“需真实凭据实测”清单见 [账户审计记录](../research/evidence/accounts-b01-b07-candidates.md)。修正记录：MiniMax 官方路径为 `/v1/token_plan/remains`（非 cc-switch 的 coding_plan 路径）；ZenMux 由 E3 升级为 E1（官方 Management API）。

| 平台/类别 | cc-switch 当前线索 | 本项目当前结论 / 下一步 |
| --- | --- | --- |
| StepFun API | GET `https://api.stepfun.com/v1/accounts` | E1（2026-10-11 审计找到官方文档）；响应 object/balance/total_cash_balance/total_voucher_balance，¥；实现前做响应夹具 |
| SiliconFlow 中国 API | GET `/v1/user/info` 旧实现 | E0：官方公告已于 2026-08-14 停用 `/user/info`；禁用旧查询，等待官方替代 |
| SiliconFlow 国际 API | `.com` 旧 user/info | E1：docs.siliconflow.com 仍收录 `GET /v1/user/info`，国际站无停用公告；响应单位未标注，需实测后才可显示数值 |
| Kimi Coding Key 直接查询 | `https://api.kimi.com/coding/v1/usages` | E2：官方 CLI 源码 `managed-usage.ts`（kimi-code `c9f01f07`）确认 Bearer + `${base}/usages`；当前仍优先本机 Server 通道（B06），直接 HTTP 通道作为其上游来源标注 |
| Zhipu 团队版 | 相同 quota host + org/project | E3：官方插件与官方文档均只支持个人版；不得将个人 Key 当团队凭据 |
| MiniMax 中国/国际 Coding Plan | `/v1/api/openplatform/coding_plan/remains` | E1，但官方路径是 `/v1/token_plan/remains`（中国 `www.minimax.cn` / 国际 `www.minimax.io`，官方 FAQ）；cc-switch 的 coding_plan 路径官方查无，不采用；general 与其他模型池分开 |
| ZenMux Plan | 上游 coding_plan 服务专用查询 | E1（2026-10-11 升级）：官方 Management API `/api/v1/management/subscription/detail` 与 `/payg/balance`，需 `sk-mg-v1-*` 个人管理密钥；实现前确认单位 |
| 火山引擎 Agent Plan / Coding Plan | OpenAPI Action 签名查询，区域区分 | E1：官方 Action `GetAFPUsage` 等，V4 签名 AK/SK；官方 host 为 `ark.<region>.volcengineapi.com`，不是 cc-switch 的 `open.volcengineapi.com`；最小权限待实测 |
| OpenCode Go | `https://opencode.ai/zen/go/v1/usage`，上游指出官方 console 路由 | E2：官方 repo `packages/console/app/src/routes/zen/go/v1/usage.ts` 确认 Bearer + `{usage:{rolling,weekly,monthly{percent,resetsAt}}}`；第一方但未文档化 |
| CommandCode | `/alpha/usage/summary` 等 | E3：官方文档未覆盖 alpha 控制面路由；保持线索 |
| Claude 订阅 | cc-switch subscription 中 OAuth usage | E3：无官方文档/开源客户端依据；不默认读取所有本机登录凭据或借用浏览器 Cookie |
| Gemini 订阅 | retrieveUserQuota 等官方服务线索 | E2：官方 gemini-cli 源码确认 `POST cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota`；属订阅配额非现金；账户/项目范围与协议版本实现前审核 |
| GitHub Copilot / Grok 账户 | 上游认证模块的额度线索 | Copilot：个人 E3（官方明确无个人配额 API），组织/企业 E1（官方 metrics）；Grok 账户 E3（`api.x.ai/v1/user/balance` 404）；与 Grok Build 本机采集无绑定 |
| Moonshot API 现金 | 可能有独立开放平台能力 | E1：官方 `GET /v1/users/me/balance`（available/voucher/cash_balance）；与 Kimi Code 订阅分开建模 |
| GLM API 现金 | Coding Plan 查询不返回现金钱包 | E3：仅控制台私有端点线索，暂不显示；计划窗口仍可显示 |
| Xiaomi MiMo API / Token Plan | 第三方工具通过控制台 Cookie 查询 | E3（近 E0）：仅第三方私有端点证据，不接入；等待官方公开接口/官方客户端依据。Token Plan 若以后接入，按 Credits/月度等实际单位建模，不预设 5h/7d |
| OpenAI API 现金 / Anthropic API 现金 | 组织 usage/cost 能力与充值余额不是同一接口 | E1（组织 Admin API，需 Admin Key）：是组织用量/成本，不是充值余额；没有充值余额依据时不声称能查余额，更不能拿 Codex/Claude 订阅额度替代 |

SiliconFlow 官方停用依据：[官方发布记录](https://docs.siliconflow.cn/docs/release-notes/overview)。MiMo 产品入口：[官方 Token Plan 页面](https://mimo.mi.com/docs/zh-CN/tokenplan)；该页面不能单独证明存在可调用余额接口。

稳定版前完成这张表的审计并留下支持/不支持的原因。与 Agent 采集必需矩阵不同，账户查询允许因为没有官方能力而不接入，这是用户已确认的范围。

## 4. 凭据与账户建模

支持 api_key、management_key、oauth_handle、local_server_token、ak_sk 等不同 credential_type。平台模板只显示所需字段，例如区域、organization/project ID。保存多个凭据到同一账户时，不重复加总其钱包。

初期以手动添加 Key、本机官方 Server 和官方只读进程协议为主。若以后新增 OAuth，必须使用厂商允许的登录流程，不能复制官方客户端 client_id 擅自模拟受限客户端。已存在的官方登录凭据仅在用户明确选择对应连接方式、兼容和安全评估通过后读取；不作为默认目录扫描的一部分。

Key 指纹使用本机带盐 HMAC，UI 默认显示掩码和别名。删除账户同时清理归属凭据（共享引用先检查）；删除本地条目不撤销厂商 Key。更换 Key 更新引用，旧错误和快照保留时间信息。

## 5. 调度与失败

当前已实现的自动调度默认开启，开关 `accounts.scheduler.enabled` 保存于 settings，启动首轮先检查开关。纯 API 余额按 15 分钟，Coding Plan/混合账户按 5 分钟计算，每个账户有稳定到本次尝试的 ±10% 抖动。最近成功/失败快照（含手动查询）与运行期尝试时间共同决定截止时间；只刷新已到期账户，随后等待最近一个截止时间。没有快照的账户在开启时可立即查询；有新近快照则延续其到期时间，启动不会无条件刷新所有账户。

关闭调度或无账户时只等待设置/账户变更通知。手动刷新继续可用，同账户已有请求时拒绝重复请求；跨账户不会因为 HTTP 占用全局数据库锁而阻塞本地统计。查询前在短锁内取账户与凭据引用，锁外读取 vault/发送 HTTP，重新加锁后复核删除、编辑、换凭据再写回。已有请求允许收尾；停止后不开始下一账户。两类开关位于用量页和设置页，Agent 的 60 秒扫描不改变账户周期。

以下是后续发布要求，**当前没有实现 credential/scope 级并发合并、Retry-After/指数退避或自动网络重试**：前台手动刷新需平台限频，同一 credential/scope 合并并发请求。每个平台根据官方限制覆盖，带 10% 抖动、连接/总超时、有界响应体与重试。后台只读查询可以有限重试，网关推理不采用这一重试策略。当前失败按自身账户周期重查；不能将此描述为已遵守服务端退避。

401/403：auth_required 或权限不足；暂停高频重试，保留旧快照标过期。429：遵守 Retry-After，指数退避；5xx/网络失败：stale 或 unavailable，保留最近成功值；业务 code 错误即使 HTTP200 也失败；JSON 字段未知/必需类型变化：incompatible，不补零。

机器休眠后合并补刷，不能瞬间补发所有错过周期。系统凭据库锁定时只显示已保存快照，不不断弹出授权窗口。删账户、关窗口后台策略和应用退出能取消任务。默认未开启自启时不后台常驻。

## 6. 展示规则

每项值有平台范围、币种/单位、数据时间、最近查询、兼容说明。无对应能力的指标隐藏；编辑页提供“平台暂未提供查询能力”。暂时失败的已有指标不隐藏，保持旧值并明确过期。

计划显示已用/剩余比例和 reset 时间。只有百分比就只显示百分比，不能反推 Token。多窗口并排，模型池/组织/Key 预算分组；月度 Credits 与现金分别显示。UI 倒计时只是根据平台时间展示，不在归零后本地“补满”额度。

手动账户关联不能自动得出逐 Key 消耗。只有供应商明确返回 Key 级 metrics 时才标逐 Key。导出带 scope、质量和查询时间，避免数字脱离含义。

## 7. 实现验收

每项适配器先有 evidence manifest，再有脱敏响应夹具、请求 header 构造测试、单位换算和错误映射测试。mock 验证不发送推理请求、不向非白名单 host 传密钥、不跨 host 跟随鉴权重定向。实测需要拥有者提供对应权限，不用开发者的个人账户数据当公共夹具。

余额 0、负余额、多币种、赠金包含关系、只给百分比、多个不同单位窗口、同账户多 Key、接口停用、业务错误 HTTP200、认证过期、断网、429、schema 变化和旧快照显示全部验收。没有真实凭据可验证的适配器标“样本验证”，不标“已实测”。
