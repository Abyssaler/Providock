# 上游实现核查记录

核查日期：2026-10-11。通过 GitHub MCP 读取仓库源码、官方文档检索和版本截图视觉检查。本文记录研究结论；未运行这些 Agent、未读取用户真实日志或查询用户账户，不能视为实机兼容证明。

## 1. 固定基线

| 对象 | 版本/提交 | 用途 |
| --- | --- | --- |
| cc-switch UI | v3.20.4 → `43e1d99084ed9b2f5dc252fd35c5adaf29d6876e` | 唯一 UI 参考版本 |
| cc-switch 用量/账户 | 研究时 main，版本线 v4.0.8 → `df40d53c36b604e044b4d5b9091aeb961aa56990` | 对齐当前已实现的监测范围 |
| Kimi Code 官方 | `c9f01f07388abc2cdca40c9c5267f58c65086710` | 新版 TS usage 与 wire 格式 |
| GLM 官方插件 | `0446d0bb0bc537d97d3ab3664c4b8b9c4a0e1254` | 官方客户端接口依据 |

main 会变化，本次结论只针对这些固定提交。UI 标签为 annotated tag，标签对象 `93994110505d4d4aae3a7a3582797aae48c2dc74` 与实际代码提交不同，引用使用目标提交。

## 2. cc-switch 为什么能统计

它不是从进程 CPU/内存、字符数或系统级网络拦截计算 Token。主要路径为本地会话日志/SQLite 导入，以及代理转发响应中的 usage。不同来源解析后进入共享用量表，按 Agent、模型、时间、供应商等聚合，成本可来自源记录或价格估算。

关键源码（均固定当前监测提交）：

| 功能 | 源码 |
| --- | --- |
| Claude Code 导入 | [session_usage.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/session_usage.rs) |
| Codex 导入 | [session_usage_codex.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/session_usage_codex.rs) |
| Gemini 导入 | [session_usage_gemini.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/session_usage_gemini.rs) |
| Grok Build 导入 | [session_usage_grokbuild.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/session_usage_grokbuild.rs) |
| OpenCode 导入 | [session_usage_opencode.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/session_usage_opencode.rs) |
| Pi 导入 | [session_usage_pi.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/session_usage_pi.rs) |
| MCode 导入 | [session_usage_mcode.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/session_usage_mcode.rs) |
| MCode 数据路径 | [mcode_config.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/mcode_config.rs) |
| Pi 路径/覆盖配置 | [pi_config/mod.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/pi_config/mod.rs) |
| 余额 | [balance.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/balance.rs) |
| Coding Plan | [coding_plan.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/coding_plan.rs) |
| 订阅 | [subscription.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/subscription.rs) |

七个本地导入器明确存在；Claude Desktop 用量依赖模型映射本地路由，不能推论为所有原生 Desktop 流量。OpenClaw/Hermes 的配置支持不自动形成用量支持。

需注意：cc-switch 将日志导入复用到 proxy_request_logs 等现有模型；Providock 从头设计，不必继承其表名和所有默认值。我们明确区分 reported/estimated/unknown、不同 stream_domain 与计量粒度。

## 3. 源码中值得借鉴和需要核实的点

- Codex 有单次/累计、重复快照、分叉和压缩格式处理，不是简单累加所有 token_count。
- Grok Build turn_completed 是独立回合聚合；费用 tick 精度和部分计价需要保留。
- OpenCode V2 存在时避免与冻结 V1 重复读取；SQLite 不能一律当 append-only。
- Pi 处理压缩/分支摘要的消费和跨文件语义去重；不能递归统计嵌套旧消息。
- MCode CLI/TUI/桌面共用数据目录和投影表，不能当三份用量。
- Gemini/OpenCode/MCode 的 output 与 thoughts/reasoning 关系需对实际官方版本核实，不能只照抄一个加法。
- Pi 等源 reported 0 cost 的处理可以改进；本项目明确合法零值优先于估算。

以上是设计输入，后续 P0 使用官方产品源码及样本确认具体语义，差异修正须写入兼容登记。

## 4. Kimi 官方源码核查

| 证据 | 结论 |
| --- | --- |
| [usageOps.ts](https://github.com/MoonshotAI/kimi-code/blob/c9f01f07388abc2cdca40c9c5267f58c65086710/packages/agent-core-v2/src/agent/usage/usageOps.ts) | durable usage.record 的事件定义 |
| [usageAgentModel.ts](https://github.com/MoonshotAI/kimi-code/blob/c9f01f07388abc2cdca40c9c5267f58c65086710/packages/agent-core-v2/src/session/usage/usageAgentModel.ts) | 每次 record 发事件，再累加到模型状态；turn/session scope 都可能是增量调用 |
| [human/llm/usage.ts](https://github.com/MoonshotAI/kimi-code/blob/c9f01f07388abc2cdca40c9c5267f58c65086710/packages/agent-core-v2/src/human/llm/usage.ts) | inputOther/inputCacheRead/inputCacheCreation/output 分量 |
| [usageEvents.ts](https://github.com/MoonshotAI/kimi-code/blob/c9f01f07388abc2cdca40c9c5267f58c65086710/packages/agent-core-v2/src/agent/usage/usageEvents.ts) | agent.status.updated 为状态通知，不再作为一份消费 |
| [wire/record.ts](https://github.com/MoonshotAI/kimi-code/blob/c9f01f07388abc2cdca40c9c5267f58c65086710/packages/agent-core-v2/src/wire/record.ts) | wire 文件协议头与记录方式 |
| [wire/tree/fork.ts](https://github.com/MoonshotAI/kimi-code/blob/c9f01f07388abc2cdca40c9c5267f58c65086710/packages/agent-core-v2/src/wire/tree/fork.ts) | 分叉继承树，需要按祖先身份处理 |
| [迁移 wire writer](https://github.com/MoonshotAI/kimi-code/blob/c9f01f07388abc2cdca40c9c5267f58c65086710/packages/migration-legacy/src/sessions/wire-writer.ts) | 迁移历史的上下文测量不等于完整调用消费 |

官方文档：[数据位置](https://www.kimi.com/en/help/kimi-code/cli-data-locations)、[Server API](https://www.kimi.ai/help/kimi-code/cli-server-api)。本机 Server API 被官方标为实验接口，须协商实例版本和实际 schema，不视为长期稳定厂商公开 API。

旧 [MoonshotAI/kimi-cli](https://github.com/MoonshotAI/kimi-cli) 的 Python 时代格式单独研究。不能将第三方工具“只统计 turn scope”的旧经验直接套到这次固定的新版源码。

## 5. 账户研究中的纠偏

DeepSeek 有公开余额接口；[官方文档](https://api-docs.deepseek.com/zh-cn/api/get-user-balance/) 返回总可用余额、赠金和充值余额。GLM 官方 [query-usage.mjs](https://github.com/zai-org/zai-coding-plugins/blob/0446d0bb0bc537d97d3ab3664c4b8b9c4a0e1254/plugins/glm-plan-usage/skills/usage-query-skill/scripts/query-usage.mjs) 提供计划/模型/工具查询依据，但不等于现金钱包。

[OpenRouter Credits](https://openrouter.ai/docs/api/api-reference/credits/get-credits) 明确管理密钥要求；[当前 Key 查询](https://openrouter.ai/docs/api/api-reference/api-keys/get-current-api-key) 是另一能力。普通推理 Key 的预算不能当账户现金余额。

[Novita 当前官方余额文档](https://docs.novita.ai/api-reference/basic-get-user-balance) 使用 `/openapi/v1/billing/balance/detail`，单位为 1/10000 USD；cc-switch 仍有旧 `/v3/user/balance` 查询线索，本项目优先当前官方接口。

[SiliconFlow 官方公告](https://docs.siliconflow.cn/docs/release-notes/overview) 写明 `/user/info` 于 2026-08-14 停用。上游有旧实现不能证明现在可用。国际站需要单独证据，不能自动类推。

[Codex 官方 App Server](https://learn.chatgpt.com/docs/app-server) 的账户只读能力应按实际支持版本使用，不照抄第三方 wham 私有调用。MiMo 控制台余额/计划端点目前仅有第三方 Cookie 查询线索，按用户边界不接入；其 [Token Plan 产品页面](https://mimo.mi.com/docs/zh-CN/tokenplan) 不是查询 API 的证明。

## 6. UI 核查

已查看 v3.20.4 仓库截图和对应 CSS/App/UsageDashboard/UsageHero/ProviderCard 源码。可借鉴白色/浅灰背景、蓝色强调、分段筛选、圆角横向卡片、等宽数字和趋势/明细层级。截图资源可能滞后于版本功能，不能只根据截图上的 Agent 数量判断能力。

Providock 重新组织为用量、账户、会话和数据源，省去模型接入和供应商切换。原型为原创离线 HTML，不嵌入上游截图、图标或外部资源。

## 7. 尚未验证

各 Agent 的具体可发布版本范围、Windows 实际目录与读取行为、Desktop 模型映射最小依赖、Kimi 旧格式/迁移链完整性、账户候选官方证据、真实权限、跨平台凭据库、性能和签名分发。这些已经列入 T00–T16，不影响完成设计交付，但阻塞对应功能的发布支持声明。
