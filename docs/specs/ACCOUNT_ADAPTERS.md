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

| 平台/类别 | cc-switch 当前线索 | 本项目当前结论 / 下一步 |
| --- | --- | --- |
| StepFun API | GET `https://api.stepfun.com/v1/accounts` | 待补官方文档/客户端证据和响应单位；E3 线索，不先承诺可查 |
| SiliconFlow 中国 API | GET `/v1/user/info` 旧实现 | 官方公告已于 2026-08-14 停用 `/user/info`；禁用旧查询，等待官方替代 |
| SiliconFlow 国际 API | `.com` 旧 user/info | 单独核实国际站，不能由中国公告直接推出同日停用，也不能默认仍可用；未核实不发布 |
| Kimi Coding Key 直接查询 | `https://api.kimi.com/coding/v1/usages` | 当前优先已确认的官方 Server 通道；直接 HTTP 通道补官方证据与认证后再启用 |
| Zhipu 团队版 | 相同 quota host + org/project | 官方支持范围、org/project 字段、所需权限单独核实；不得将个人 Key 当团队凭据 |
| MiniMax 中国/国际 Coding Plan | `/v1/api/openplatform/coding_plan/remains` | 核实官方文档/插件；general 与其他模型池分开；不能强制所有套餐都有周额度 |
| ZenMux Plan | 上游 coding_plan 服务专用查询 | 补官方接口依据、认证和单位；第三方线索未满足用户边界 |
| 火山引擎 Agent Plan / Coding Plan | OpenAPI Action 签名查询，区域区分 | AK/SK 与 V4 签名，不是推理 API Key；分别核实官方 Action、Version 和最小权限 |
| OpenCode Go | `https://opencode.ai/zen/go/v1/usage`，上游指出官方 console 路由 | 到官方 repo 固定路由源码提交，确认 token 与 plan scope 后进入 E2 |
| CommandCode | `/alpha/usage/summary` 等 | 核实官方 SDK/源码和组织范围；alpha 路径要标实验 |
| Claude 订阅 | cc-switch subscription 中 OAuth usage | 补官方客户端依据/凭据方式；不默认读取所有本机登录凭据或借用浏览器 Cookie |
| Gemini 订阅 | retrieveUserQuota 等官方服务线索 | 官方客户端依据、账户/项目范围和协议版本审核后决定；不等同 Gemini API 现金余额 |
| GitHub Copilot / Grok 账户 | 上游认证模块的额度线索 | 单独审计官方来源；与 Grok Build 本机用量采集无绑定依赖 |
| Moonshot API 现金 | 可能有独立开放平台能力 | 与 Kimi Code 订阅分开；本次未确认当前官方余额文档，不虚构端点 |
| GLM API 现金 | Coding Plan 查询不返回现金钱包 | 未确认官方余额能力，暂不显示；计划窗口仍可显示 |
| Xiaomi MiMo API / Token Plan | 第三方工具通过控制台 Cookie 查询 | 仅第三方私有端点证据，不接入；等待官方公开接口/官方客户端依据。Token Plan 若以后接入，按 Credits/月度等实际单位建模，不预设 5h/7d |
| OpenAI API 现金 / Anthropic API 现金 | 组织 usage/cost 能力与充值余额不是同一接口 | 有官方权限可研究 usage/cost；没有充值余额依据时不声称能查余额，更不能拿 Codex/Claude 订阅额度替代 |

SiliconFlow 官方停用依据：[官方发布记录](https://docs.siliconflow.cn/docs/release-notes/overview)。MiMo 产品入口：[官方 Token Plan 页面](https://mimo.mi.com/docs/zh-CN/tokenplan)；该页面不能单独证明存在可调用余额接口。

稳定版前完成这张表的审计并留下支持/不支持的原因。与 Agent 采集必需矩阵不同，账户查询允许因为没有官方能力而不接入，这是用户已确认的范围。

## 4. 凭据与账户建模

支持 api_key、management_key、oauth_handle、local_server_token、ak_sk 等不同 credential_type。平台模板只显示所需字段，例如区域、organization/project ID。保存多个凭据到同一账户时，不重复加总其钱包。

初期以手动添加 Key、本机官方 Server 和官方只读进程协议为主。若以后新增 OAuth，必须使用厂商允许的登录流程，不能复制官方客户端 client_id 擅自模拟受限客户端。已存在的官方登录凭据仅在用户明确选择对应连接方式、兼容和安全评估通过后读取；不作为默认目录扫描的一部分。

Key 指纹使用本机带盐 HMAC，UI 默认显示掩码和别名。删除账户同时清理归属凭据（共享引用先检查）；删除本地条目不撤销厂商 Key。更换 Key 更新引用，旧错误和快照保留时间信息。

## 5. 调度与失败

拟定默认轮询：余额 15 分钟，计划额度 5 分钟；前台手动刷新可用但限频，同一 credential/scope 合并并发请求。每个平台根据官方限制覆盖，带 10% 抖动、连接/总超时、有界响应体与重试。后台只读查询可以有限重试，网关推理不采用这一重试策略。

401/403：auth_required 或权限不足；暂停高频重试，保留旧快照标过期。429：遵守 Retry-After，指数退避；5xx/网络失败：stale 或 unavailable，保留最近成功值；业务 code 错误即使 HTTP200 也失败；JSON 字段未知/必需类型变化：incompatible，不补零。

机器休眠后合并补刷，不能瞬间补发所有错过周期。系统凭据库锁定时只显示已保存快照，不不断弹出授权窗口。删账户、关窗口后台策略和应用退出能取消任务。默认未开启自启时不后台常驻。

## 6. 展示规则

每项值有平台范围、币种/单位、数据时间、最近查询、兼容说明。无对应能力的指标隐藏；编辑页提供“平台暂未提供查询能力”。暂时失败的已有指标不隐藏，保持旧值并明确过期。

计划显示已用/剩余比例和 reset 时间。只有百分比就只显示百分比，不能反推 Token。多窗口并排，模型池/组织/Key 预算分组；月度 Credits 与现金分别显示。UI 倒计时只是根据平台时间展示，不在归零后本地“补满”额度。

手动账户关联不能自动得出逐 Key 消耗。只有供应商明确返回 Key 级 metrics 时才标逐 Key。导出带 scope、质量和查询时间，避免数字脱离含义。

## 7. 实现验收

每项适配器先有 evidence manifest，再有脱敏响应夹具、请求 header 构造测试、单位换算和错误映射测试。mock 验证不发送推理请求、不向非白名单 host 传密钥、不跨 host 跟随鉴权重定向。实测需要拥有者提供对应权限，不用开发者的个人账户数据当公共夹具。

余额 0、负余额、多币种、赠金包含关系、只给百分比、多个不同单位窗口、同账户多 Key、接口停用、业务错误 HTTP200、认证过期、断网、429、schema 变化和旧快照显示全部验收。没有真实凭据可验证的适配器标“样本验证”，不标“已实测”。
