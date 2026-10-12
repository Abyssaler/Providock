# 兼容登记

最近更新：2026-10-13（A10 Harness、B07 stdio 本地接线；验证见 2026-10-13 记录）；此前 2026-10-12（GLM 错误分类修正 + B-04 Kimi Code 直连真实凭据补验）；此前为 2026-10-11（T00 证据核实完成一轮）。状态含义：`researched` = 阅读固定提交源码/官方文档；`fixture_verified` = 合成/脱敏样本 + 独立预期值通过；`live_verified` = 真实环境实测。

## 实现状态（2026-10-11，随验收更新）

| 条目 | 解析器 | 夹具 | 扫描接入 | 实机 | 状态 |
| --- | --- | --- | --- | --- | --- |
| A01 Claude Code | ✅ claude_code.rs | 7 用例通过 | ✅ scanner | 未 | fixture_verified |
| A02 Codex | ✅ codex_rollout.rs v2 | 8 用例通过 + 升级/父时间线回归 | ✅ scanner（日期目录、.zst、两遍 fork） | **macOS 只读快照：10 文件 338 事件，六项 Token 全量/当天独立核对一致、重扫守恒** | fixture_verified + macOS live |
| A03 Gemini CLI | ✅ gemini_cli.rs（JSON+JSONL 回放） | 10 用例通过 | ✅ scanner | 未 | fixture_verified |
| A04 Grok Build | ✅ grok_build.rs | 9 用例通过 | ✅ scanner | 未 | fixture_verified |
| A05 OpenCode | ✅ opencode_db.rs（V1/V2 探测） | 3 库用例通过（含反向验证） | ✅ scanner（mtime+WAL 指纹整库重读） | **macOS 实测：真实库 V2 249 事件、重扫一致** | fixture_verified + macOS live |
| A06 Pi | ✅ pi_log.rs | 13 用例通过（含 usage kind 条目） | ✅ scanner | 未 | fixture_verified |
| A07 MCode | ✅ mcode_db.rs | 6 用例通过 | ✅ scanner（row_id 游标+重叠回看） | 未 | fixture_verified |
| A08 Kimi Code | ✅ kimi_wire.rs | 7 用例通过 | ✅ scanner | **macOS 实测：63 文件 4147 事件、六项 Token 全量/当天独立核对一致、重扫幂等**（Windows 待验） | fixture_verified + macOS live |
| A10 DeepSeek Harness | ✅ deepseek_harness.rs v1 | 4 纯解析 + 2 扫描集成用例，独立预期 612 Token | ✅ scanner（v3/v4、.zstd、多代/父链） | 未 | fixture_verified |
| A09 Desktop 网关 | 未开始（T00-G 演示待做） | — | — | 未 | researched |
| 旧 Python Kimi | 未开始（独立适配器） | — | — | 未 | researched |

数据核心（T02）、凭据库/路径守卫/诊断脱敏（T03）、聚合服务与用量页纵线（T04）已实现并有测试。全部 Windows 行为未经实机验证。

CI 补记：自动监测首轮 macOS CI 通过；Windows 在 Codex 升级回归测试中暴露测试构造的 `/` 与 `\` 文件身份差异。测试已改用实际入库游标的身份并保留原计数断言，本机相关回归通过；生产 parser/扫描器未变更，[修复提交 d9f11f9 的双平台 CI](https://github.com/Abyssaler/Providock/actions/runs/38138488820) 检查、测试及安装包构建均已通过；v2.0.0 按维护者要求作为正式 Release 分发，保留上述验证边界，未新增 Windows 实机声明；1.1.0-preview.1 候选未发布。

## 账户适配器实现状态（2026-10-12）

| ID | 适配器 | 验证级别 | 备注 |
| --- | --- | --- | --- |
| B01 DeepSeek | ✅ deepseek.rs | 样本验证（夹具+mock listener） | 未真实凭据实测 |
| B02 OpenRouter Key | ✅ openrouter.rs | 样本验证 | scope=key 预算非现金 |
| B03 OpenRouter Credits | ✅ openrouter.rs | 样本验证 | 管理密钥要求已建模型 |
| B04 Novita | ✅ novita.rs | 样本验证 | 新路径 + 1/10000 USD 精确换算 |
| B05 GLM Coding Plan | ✅ glm_plan.rs | 样本验证 | E2；双 TOKENS_LIMIT 区分待官方证据；`no_plan`/401 分类经 2026-10-12 真实凭据验证；`CREDIT_LIMIT` 保持未知诊断 |
| B06 Kimi 本机 Server | ✅ kimi_server.rs | 样本验证 | 需用户已运行 Server；meta 协商已实现 |
| B-04 Kimi Code 直连 | ✅ kimi_code.rs | 实机验证（仅 `/coding/v1/usages`，2026-10-12）+ 夹具 | `credential_type = api_key` → 直连；server token 保持 B06；`booster_wallet` 未解析 |
| B07 Codex App Server | ✅ codex_appserver.rs + codex_connection.rs | 样本 + macOS 真实只读验证（2026-10-13） | stdio 初始化/account/read/rateLimits；本机 ChatGPT 登录，Windows 未验 |
| 账户域服务 | services/{accounts,account_query,account_scheduler}.rs | 单测 + mock HTTP 实测（重定向剥 Auth 等） | 后台循环及持久化开关已接；逐账户到期；禁用启动首轮、锁外 HTTP、并发和删除中请求回归通过 |

账户页 UI（T12）已接 IPC。Kimi Code 直连额度和 GLM 无套餐/401 状态已有局部 macOS 实测，其余保持样本验证；不代表完整平台或 Windows 账户验收。

证据文档（均为 2026-10-11 按固定提交核实，含逐行 URL）：

- [A01–A04 cc-switch 导入器](evidence/a01-a04-ccswitch-importers.md)（cc-switch `df40d53c`）
- [A05–A07 cc-switch 导入器](evidence/a05-a07-ccswitch-importers.md)（cc-switch `df40d53c`）
- [A08 Kimi 新/旧官方格式](evidence/a08-kimi-official.md)（kimi-code `c9f01f07`、kimi-cli 归档仓库）
- [A10 DeepSeek Harness 固定官方源码](evidence/a10-deepseek-harness.md)
- [Codex 接线与价格解析 API](evidence/2026-10-13-codex-and-price-parsing.md)
- [A09 Desktop 网关捕获范围](evidence/a09-desktop-gateway.md)（cc-switch `df40d53c`）
- [B01–B07 与账户候选审计](evidence/accounts-b01-b07-candidates.md)

## Agent 适配器

| ID | 已核实要点（摘要） | 与上游的关键差异/风险 | fixture 计划 | 状态 |
| --- | --- | --- | --- | --- |
| A01 Claude Code | `~/.claude/projects/**/*.jsonl`（含 subagents 固定深度）；assistant 消息 usage；input 为 fresh 不含缓存；cache TTL 细分存在 | 上游有「相同 Token 四元组 ±600s」代理指纹去重，与契约冲突，不沿用；截断钉 EOF 策略待评估；上游不读 CLAUDE_CONFIG_DIR（需按官方规则补） | 流式多次更新、相同用量独立请求、截断尾行、子 Agent、分叉 | researched |
| A02 Codex | `sessions/YYYY/MM/DD` + archived；`.zst` 压缩；`last_token_usage` 优先、`total_token_usage` 受控差分；input 为 TOTAL（含 cached）需换算 fresh/read | 上游游标为行号且每轮全量重读、未见截断检测；fork 靠签名子序列对齐——我们按契约做祖先身份归并 | 累计重复/重置/倒退、fork 前缀、压缩继承、截断首累计量 | researched |
| A03 Gemini CLI | `tmp/<hash>/chats/session-*`；JSON 与 JSONL（`$set`/`$rewindTo` 回放）；字段 input/output/cached/thoughts | 上游 output+thoughts 相加、input 含 cached 但未标语义——包含关系必须对官方版本核实，不照抄；缺 message id 撞主键的上游缺陷要修 | 包含关系验证、rewind、缺 id、工具回显 | researched |
| A04 Grok Build | `sessions|archived_sessions/**/updates.jsonl`；`turn_completed` 回合聚合；`costUsdTicks` 1 tick=1e-10 USD；reasoning ⊂ output | 上游 60s 沉降窗/接管守卫无 session 维度会漏记；无价时分项记 0 违反「未知≠0」 | 连续同数字两轮、部分计价、归档移动 | researched |
| A05 OpenCode | `OPENCODE_DB` > XDG > `~/.local/share/opencode`（Windows 同样走 XDG 规则，待实测）；V2(session_v2/session_message) 存在时弃读 V1 | 上游 INSERT OR IGNORE 无法校正后续更新的消息行——需 updated_at 重叠回看游标；compaction(failed) 也计费待官方确认 | V1/V2、消息更新、WAL/锁、compaction | researched |
| A06 Pi | `PI_CODING_AGENT_DIR` / sessionDir / `~/.pi/agent`；compaction、branch_summary 计费；retainedTail 嵌套历史不计入 | 上游丢弃 `usage.reasoning`；`total==0` 当未知——合法 0 要保留；分叉按可证实祖先归并 | 摘要消费、嵌套复制、响应 ID 复用、费用矛盾 | researched |
| A07 MCode | `MINIMAX_DATA_DIR`>`MAVIS_DATA_DIR`>`~/.minimax` + `v2/sqlite/runtime-state.sqlite`；`local_runtime_token_usage` 10 列；id 单调游标；cost_usd=0 合法保留 | 上游 output+reasoning 相加待官方确认；id 是否 append-only 待核实（否则加重叠窗）；真实 DDL 未获取 | DDL 核实、更新行、共享库单实例 | researched |
| A08 Kimi Code | 新版：`KIMI_CODE_HOME`/`~/.kimi-code`，`sessions/<wd_key>/<sid>/agents/<agentId>/wire.jsonl` + `session_index.jsonl`；wire 首行 metadata `protocol_version:"1.5"`；**一次 LLM 请求一条 durable `usage.record`**（inputOther/inputCacheRead/inputCacheCreation/output）；`usageScope` 仅按 source.type=='turn' 判 turn，session scope 目前唯一来源是全量压缩；`agent.status.updated` 为累计快照不durable、不计入；fork 经 `state.json.forkedFrom`+wire 末尾 `forked`；undo 三条记录不撤销账本；迁移写 protocol 1.0 + `token_counting.measured`（上下文锚点，非消费） | 旧 Python kimi-cli（已归档）：`KIMI_SHARE_DIR`/`~/.kimi`，`sessions/<md5>/<uuid>/`，wire `{timestamp,message{type,payload}}`（协议 1.10），snake_case 字段，逐 step `token_usage` 绝对值、无会话累计——与新版分开适配 | 新协议头、turn/session scope、主子 Agent、fork 前缀、undo、压缩、崩溃尾行、unknown protocol、旧 Python、迁移测量、同数字独立调用 | researched |
| A09 Desktop 网关 | 上游默认 `127.0.0.1:15721`，`/claude-desktop/*` 需 Bearer 令牌；协议：Anthropic Messages / OpenAI Chat / Responses / Gemini Native；流式按事件合并（message_start/delta）或末 chunk 快照；request_id 派生自上游响应 id（`session:{msg_id}`），无独立 provider request-id 列 | 上游缺口：透传缺 usage 落全 0 行、重试不逐次记账、1h 缓存未落库；我们按契约标记 partial/unknown，不落假 0 | 非流式/流式/断流/取消/工具调用/重试、与日志重叠域 | researched（实机演示未做，见下） |

T00-G 状态：Desktop 模型映射最小网关演示**未完成**——需要本机安装 Claude Desktop 并手工配置，属于需要用户环境配合的实机验证；上游源码路径已固定（见证据文档），演示在 T08 前完成。

## 账户适配器（B01–B07 优先项，证据详见账户审计文档）

| ID | 结论 | 接口与认证 | 关键字段/单位 | 状态 |
| --- | --- | --- | --- | --- |
| B01 DeepSeek | E1 | `GET https://api.deepseek.com/user/balance`，Bearer | `balance_infos[]{currency,total_balance,granted_balance,topped_up_balance}`，总额已含赠金+充值，分币种 | researched |
| B02 OpenRouter Key | E1 | `GET https://openrouter.ai/api/v1/key`，Bearer 当前 Key | usage/limit/limit_remaining/budgets 等；Key 级预算≠现金余额 | researched |
| B03 OpenRouter Credits | E1 | `GET https://openrouter.ai/api/v1/credits`，**管理密钥** | total_credits/total_usage，普通 Key 403 | researched |
| B04 Novita | E1 | `GET https://api.novita.ai/openapi/v1/billing/balance/detail`，Bearer | available/cash/creditLimit/pending/outstanding，单位 1/10000 USD | researched |
| B05 GLM Coding Plan | E2（官方插件 `0446d0bb`） | host `open.bigmodel.cn`/`dev.bigmodel.cn`/`api.z.ai`；`/api/monitor/usage/quota/limit`、`/model-usage`、`/tool-usage`；Authorization 为原 token | TOKENS_LIMIT=5h 百分比、TIME_LIMIT=MCP 月度；非现金 | researched |
| B06 Kimi 本机 Server | E1（官方实验） | `http://127.0.0.1:58627`，Bearer；`/api/v1/meta`、`/sessions/{id}/snapshot`、`/oauth/usage` | 信封 `{code,msg,data,request_id}`；quota.usages{limit5h,limit7d,monthTotal,monthCode}{usedRatio,resetAt}+extraUsage 钱包 cents | researched |
| B07 Codex App Server | E1 | `account/rateLimits/read`、`account/usage/read`；initialize 协商 | rate_limits 窗口 used_percent/resets_at；usage 有 lifetimeTokens 等；API-key-only/Bedrock 不支持 usage | researched |

## 账户候选审计结论（2026-10-11，详见审计文档）

- E1：StepFun `/v1/accounts`；SiliconFlow **国际站** `/v1/user/info`（单位待实测）；MiniMax `/v1/token_plan/remains`（**非** cc-switch 的 coding_plan 路径）；ZenMux 管理 API（`sk-mg-v1-*`）；火山引擎 `GetAFPUsage`（V4 签名 AK/SK，host 为 `ark.<region>.volcengineapi.com`）；Moonshot 开放平台 `/v1/users/me/balance`；GitHub Copilot **组织/企业** metrics；OpenAI/Anthropic 组织 Admin usage/cost（非充值余额）。
- E2：Kimi Coding Key `api.kimi.com/coding/v1/usages`（官方 CLI 源码 `managed-usage.ts`）；OpenCode Go zen usage（官方 console 路由）；Gemini `retrieveUserQuota`（官方 gemini-cli 源码，订阅配额非现金）。
- E3：Zhipu 团队版、CommandCode alpha、Claude 订阅 OAuth、Grok 账户、GLM 现金、Copilot 个人。
- E0：SiliconFlow **中国站** `/v1/user/info`（官方公告 2026-08-14 停用）；Xiaomi MiMo（仅控制台 Cookie 线索，接近 E0）。
- 与 `docs/specs/ACCOUNT_ADAPTERS.md` 第 3 节的两处表述差异已按证据修正（MiniMax 路径、ZenMux 升级 E1），理由见该文档与审计文档。

## 发布核验补记（2026-10-12）

v2.0.0 固定提交 2662489 的 [双平台 CI](https://github.com/Abyssaler/Providock/actions/runs/38195092032) 全部成功；干净包清单、独立验签、远端摘要及 Mac 安装/启动/自动扫描冒烟通过，详见 [发布核验记录](validation/2026-10-12-release-v2.0.0.md)。这不增加适配器语义或真实账户的验证范围，不构成 Windows 实机证据。

## 下一步

1. 按 fixture 计划编写合成样本与独立预期值（随 T04–T07 逐适配器落地，先 Codex/Kimi）。
2. 账户适配器实现前的 mock 响应夹具（B01–B07，按审计文档 schema 摘要）。
3. 需要真实凭据实测的 17 项清单见审计文档末尾——无凭据不标 live_verified。

## 2026-10-12 账户与模型定价增量

本轮只修复 IPC 快照字段、默认凭据保存和统计/定价呈现；八源解析器、平台查询端点及许可证据未变。新增本地凭据文件往返/重启/删除/权限/损坏回归和模型费用独立预期值测试，不将既有 fixture_verified 升为 live_verified。没有访问用户真实 Key、旧 Keychain 或改写源 Agent 文件。Windows 新凭据文件行为尚待实机验证。

## v2.0.2 界面与运行策略

全局纯黑主题，原创矢量应用图标；趋势 SVG 平滑插值保持端点区间，分量颜色经浏览器计算样式核对。macOS 菜单栏使用 Tauri 2.12.3 tray-icon API，后台统计只读数据库；Windows 不增加该生命周期策略。Agent 解析器版本和已有适配范围未改变。账户明文文件选择与既有重新输入要求见安全规范。

## v2.0.3 更新与界面交付

八源解析器版本未变。Agent 可见性 IPC 保存参数修复；多币种趋势选择恢复且保持未知断点。Kimi Code 与 GLM 局部真实验证沿用上表，不使用已删除的测试密钥重新查询。

新增 Tauri updater 2.13.3 / process 2.4.0（JS/Rust 锁文件固定），支持 darwin-aarch64 与 windows-x86_64；使用 io.providock.app 及独立 providock-v2-latest.json，v1 更新协议不兼容。macOS/Windows 的构建、原生及更新验证范围见 [v2.0.3 记录](validation/2026-10-12-release-v2.0.3.md)。不引入 DB schema 变更，不改变凭据保存策略。

## v2.0.3 发行与原生补验结果

同 SHA 双平台 CI 成功（Mac Rust 349、Windows 344，各 4 ignored，前端各 77）；11 项公开资产回下载/摘要与签名核对通过。Mac 实测 Agent 设置保存与重启、双币种趋势切换、真实 GitHub 签名更新到 2.0.3 并自动重启、正式 DMG 安装及旧数据/六规则/凭据保留。Windows 只做 CI 与 NSIS 解包，不标实机完成；慢网络首次检查超时后重试成功。详见 v2.0.3 核验记录。

## v2.0.4 价格导入与成本移除

新增用户指定 AIHOT 公开价格页，证据见 [AIHOT 定价](evidence/aihot-pricing.md)，合成夹具 `fixtures/pricing/aihot/catalogue.html`。有界 SSR JSON 参考表、精确名称匹配、CNY/汇率来源、未知/零价、规则保护与事务回滚已做样本验证；真实页面读取、内存库导入及 Mac 原生隔离库一键导入已验证。第三方定价不改变账户接入证据等级。

Grok/OpenCode/Pi/MCode 不再读取源自报成本，其 Token、身份、游标、去重和既有来源范围保持；费用矛盾不再影响 Token 完整性，旧 cost 输入夹具保留以验证被忽略。历史数据库成本列留作兼容且不再读写。没有新增 Agent 或账户实机声明。发行/Windows 边界见 [v2.0.4 记录](validation/2026-10-12-release-v2.0.4.md)。

### v2.0.4 发行补验

同 SHA 双平台 CI、11 项公开资产摘要和独立验签通过。Mac 正式应用内更新至 v2.0.4 并保留用户数据已实测；隔离测试产物已清理。Windows 仅 CI/NSIS 解包，未扩展为实机验证。详见本版核验记录。

## 2026-10-13 本地价格解析边界

AIHOT 当前 UI 只取选定模型报价，不批量写库；程序官网解析限明确单位/单币种/唯一模型行静态表。DeepSeek/GLM/OpenRouter 单次模型辅助的凭据隔离、无工具、原文校验有合成回归；未做真实付费推理或所有官网结构验收。详见[依据](evidence/2026-10-13-codex-and-price-parsing.md)。

## v2.1.0 发行核验（2026-10-13）

固定代码 `fcebe85` 的[双平台 CI](https://github.com/Abyssaler/Providock/actions/runs/38285861608) 已通过；11 项公开资产回下载摘要、三个二进制独立验签、Mac DMG/更新归档一致性与 Windows NSIS/PE 清单核验完成。新增 DSH 仍为 fixture_verified，真实 Harness 日志未验；B07 仅 macOS 当前登录只读额度接线实测，未扩大为 Windows 实机。用户明确自行安装，本轮没有执行正式 2.1.0 安装/更新；开发测试残余已清理，正式源码、应用、数据和恢复备份保留。详见[本版核验记录](validation/2026-10-13-release-v2.1.0.md)。
