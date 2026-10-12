# Agent 采集规范

基线：cc-switch 当前监测实现固定在 `df40d53c36b604e044b4d5b9091aeb961aa56990`；UI 另固定 v3.20.4。下列定义稳定版必须具备的能力；八个本地解析器与扫描链路已实现，验证级别和未完成范围见 [兼容登记](../research/compatibility-register.md) 与 [当前状态](../CURRENT_STATUS.md)。源码入口和证据详见 [研究记录](../research/UPSTREAM_AUDIT.md)。

## 1. 必需支持矩阵

| ID / Agent | 基础来源 | 必需覆盖 | 精度限制 |
| --- | --- | --- | --- |
| A01 Claude Code | projects 下会话 JSONL | 主会话、子 Agent、已有归档/分支布局 | 只反映保存的 usage；消息更新不可重复加 |
| A02 Codex | sessions 下 JSONL 及已支持压缩形式 | 单次 usage、累计计数、分叉来源 | 计数范围/重置须按版本识别 |
| A03 Gemini CLI | tmp 下 chats 会话 JSON / JSONL | 消息计量、thoughts、cached | 缓存是否包含在输入、thoughts 是否另计须验证 |
| A04 Grok Build | sessions、archived_sessions 的 updates.jsonl | turn_completed 多次模型调用的回合聚合 | 回合记录数不等于模型请求数 |
| A05 OpenCode | opencode.db，只读 SQLite | V1 与 V2 布局，V2 优先规则 | 源可能更新消息，不能只按新增行读取 |
| A06 Pi | 会话 JSONL | message、compaction、branch_summary 的真实 usage | 嵌套复制历史不重复统计 |
| A07 MiniMax Code / MCode | runtime-state.sqlite | CLI/TUI/桌面共享的用量投影表 | 同一数据库只采集一次 |
| A08 Kimi Code | wire.jsonl + session 元数据 | 新 TypeScript 协议、子 Agent、旧 Python 格式独立适配 | 迁移后无 usage 的记录不能猜测 |
| A09 Claude Desktop（条件性） | 可选网关 | 经本机模型映射路径的请求 usage | 不能捕获未经过网关的原生流量 |

OpenClaw / Hermes 不是本基线已确认的用量导入器，后续研究；不把“配置管理支持”误算为“用量统计支持”。Codex 桌面与 CLI 若共享同源日志，按同源采集，未发现桌面内部调用时标记覆盖缺口。

## 2. 路径发现

顺序：用户显式目录 → 对应 Agent 的已知环境变量/配置 → 系统默认。图形应用不能假设继承终端环境；UI 提供覆盖目录和“为什么选中此路径”。用系统用户目录 API，不在 Windows 硬编码 `/home`。只读取路径定位必需字段，不导入 Agent 的密钥。

| Agent | 研究发现的默认路径/依据 |
| --- | --- |
| Claude Code | 用户主目录下 `.claude/projects`，配置覆盖及子 Agent 目录按版本探测 |
| Codex | `CODEX_HOME` 或主目录 `.codex` 下 `sessions/YYYY/MM/DD`；归档路径单独识别 |
| Gemini CLI | 主目录 `.gemini/tmp/<project-hash>/chats/session-*`，使用其配置目录覆盖规则 |
| Grok Build | 主目录 `.grok/sessions`、`.grok/archived_sessions` 下 `<encoded-cwd>/<session-id>/updates.jsonl` |
| OpenCode | 常见 `.local/share/opencode/opencode.db`；必须核实各 OS/XDG 与官方目录规则，不把 Unix 示例硬套 Windows |
| Pi | 根据 cc-switch `pi_config/mod.rs` 及 Pi 官方环境变量/目录规则解析；P0 固定具体默认目录，当前不猜测 Windows 路径 |
| MCode | `MINIMAX_DATA_DIR`，其次兼容 `MAVIS_DATA_DIR`，否则主目录 `.minimax`；数据库相对路径 `v2/sqlite/runtime-state.sqlite` |
| Kimi Code 新版 | `KIMI_CODE_HOME` 或主目录 `.kimi-code`；Windows 为 `%USERPROFILE%\.kimi-code` |
| Kimi 旧 Python | 历史 `.kimi` 数据目录；与新版分开注册，迁移后的重复来源需识别 |

WSL、网络盘、外置盘只接受用户添加目录，不递归扫描整个主目录。源丢失保留已导入统计并标记离线。符号链接/目录联接经规范化后去重，扫描范围受选定根目录约束。

### 当前自动采集策略（D10）

应用启动且 Agent 自动同步开关开启时同步一次；之后每轮结束等待 60 秒再扫描。关闭后只等待设置通知，不启动文件监听或设置轮询；手动同步仍可用。开启后立即唤醒采集。并发触发只保留正在进行的扫描，不排队重复扫描；逐源检查 enabled，并在源之间释放数据库锁。扫描沿用各 adapter 的变化检测与游标策略，变化文件不保证只读取新增字节。关闭窗口停止任务，已开始的一次阻塞读取允许收尾。

两类开关与前端显示刷新相互独立。设置键和 IPC 见 [架构](../ARCHITECTURE.md) 与 [验证记录](../research/validation/2026-10-11-auto-monitoring.md)。

## 3. 各适配器实现要点

### A01 Claude Code

从 assistant 消息的 usage 提取 input、output、cache_read、cache_creation；原始缓存 TTL 细分保留。输入互斥语义在样本中验证。消息可能分块写入或被补全，用 session+message/request identity 做 upsert，不能把每次出现的 message usage 都相加。

扫描主项目日志及子 Agent 记录，保留 parent_session_id 和 actor_id。父会话里对子任务的摘要或回显不是另一份消耗；已有真实子调用 usage 才计费。发现时间、会话内容分支与账本分别处理。需要夹具：流式多次更新、工具调用、cache-only、子 Agent、分叉、相同用量独立请求、截断尾行。

### A02 Codex

解析 session_meta、turn_context、token_count 相关事件，并保存模型随回合变化的上下文。优先可信的 last_token_usage；total_token_usage 用于对账及单次量缺失时的受控差分。cached_input 是输入子集、reasoning_output 通常是输出子集，须按被支持版本声明，不能二次加总。

支持累计快照重复、重置、模型切换、压缩/归档文件、父会话分叉、缺少 session_meta。截断日志首个累计量不可全部记为当日新增。文件 fork 来源和继承区间决定去重，不按相同总数去重。CLI 自身日志没有记录的桌面内调用不能凭平台额度反推。

当前 parser_version 为 `codex-rollout-2`：同一 total+last 快照在同一 limit_id 或相邻记录重播时不产生新消费；没有 total 时，完全相同的 last 仍作为独立请求保留。全零 last 心跳不增加请求数；读取 last 时同时维护累计基线，后续只有 total 继续差分。合成 replayed-cumulative 用例人工预期两次请求共 240 Token，另有日期目录发现、累计回退、旧解析器升级与未变化父文件对齐回归。

升级时强制重读 parser_version 不一致的文件，保留既有代数和事件身份；只将已证明的重复/零心跳身份退休，与该批事件和游标同事务提交，不清空其他历史。完整会话首个 total 目前以零为基线；对实际被截断的累计日志安全保留历史仍是发布缺口，不能宣称已解决所有截断/重放问题。压缩和 fork 为夹具验证，本次独立实机数字核对仅覆盖未压缩、无 fork、累计单调的日志。

账户额度另外采用官方 App Server 只读能力（当前进程协议尚未接线），不能把额度快照混进 Token 账本。App Server 的实时通知仅覆盖其连接可见活动，不能假设订阅一条连接就监视所有独立 Codex 进程。

### A03 Gemini CLI

支持 JSON 完整会话文件及已识别 JSONL 版本。按消息 ID upsert；完整 JSON 文件更新时重建该文件当前消息集合并保持事件身份。字段通常有 input、output、thoughts、cached；cc-switch 基线将 output+thoughts 作为输出计量。

P0 必须对官方版本源码核实 input 是否已含 cached、output 是否已含 thoughts，再映射标准字段。不要为“对齐上游”复制潜在重复加法。thoughts 不能因为在 UI 隐藏就漏计，工具消息中的回显 usage 不能重复计。无 token 字段的消息保留会话索引，但不制造零消耗事件。

### A04 Grok Build

`turn_completed` 的 Token 是**单个用户回合内多次模型推理的聚合**，不是会话累计。两个连续回合数字相同也必须分别计入。以 session+turn identity 或可靠事件序号建立身份，granularity=turn_aggregate。

modelCalls / numTurns 仅在源语义确认后填写 request_count，不用一条回合记录替代一个请求。reasoning 是输出子集时不另加。v2.0.4 起忽略 `costUsdTicks` 及其部分计价标记；计量完整性只由 Token 字段决定。归档与活跃目录移动不能重复计。

### A05 OpenCode

只读查询 SQLite；先探测表与列，区分 V1 session/message 和 V2 session_v2/session_message。cc-switch 基线在 V2 存在时优先 V2，冻结 V1 不能再重复导入。旧版本切换需夹具验证迁移身份。

输入按上游 fresh/cache 分列语义处理。cc-switch 当前实现将 output 与 reasoning 相加；Providock 必须先核实对应官方版本实际是否互斥，再写 output_semantics，不能仅凭字段名字判断。原生费用合法时优先 reported，0 也保留。

不能只用递增 rowid 读取：消息的 tokens/cost 可能后续更新。使用已验证 updated_at/version 游标加重叠回看，或分会话校验变化；主键 upsert 并校验删除/迁移。活动 WAL、DB 锁定和 schema 变化是必测场景。

### A06 Pi

解析正规 message 的 assistant usage，以及 compaction、branch_summary 等真正产生模型消费的事件。input/output/cacheRead/cacheWrite 按已验证语义映射；压缩摘要中携带的 retainedTail 等历史对象不可递归当新事件采集。

响应 ID 可能复用，不能单独当全局唯一键。结合事件 ID、会话/分支来源和 actor。文件被复制或分叉时按可证实祖先事件归并，同时保留内容相同的独立调用。分项费用和总费用矛盾时标记异常，不任意挑较大值。

### A07 MCode

读取 `local_runtime_token_usage`：id、session_id、model、ts、input_tokens、output_tokens、reasoning_tokens、cache_read_tokens、cache_write_tokens、cost_usd。id 是研究基线增量游标；上线前核实是否 append-only，若会更新需加入重叠窗口。

CLI/TUI 与桌面共用数据库，以规范化数据库身份作为 source_instance，不能注册两份后求和。原生 input 为 fresh 的语义与 output/reasoning 的互斥关系在官方样本中确认；cc-switch 基线加 output+reasoning。会话内容来自其他消息表，与统计投影解耦，不依赖正文表才能统计。

### A08 Kimi Code：精度重点

新版官方目录是 `sessions/<workDirKey>/<sessionId>`，包含 state.json 等元数据，主 Agent `agents/main/wire.jsonl`，子 Agent `agents/agent-N/wire.jsonl`。用 session_index 辅助发现，不能认为索引一定完整或最新。

本次固定官方 TypeScript 源提交为 `c9f01f07388abc2cdca40c9c5267f58c65086710`。该源码把每次调用的 usage 发为 durable **usage.record**，并将其累加到模型状态。字段映射为：

| Kimi 原字段 | Providock |
| --- | --- |
| inputOther | input_fresh |
| inputCacheRead | input_cache_read |
| inputCacheCreation | input_cache_write |
| output | output_total |

该基线 `usageScope=turn` 与 `usageScope=session` 都可能是独立调用的记录；session 可能来自标题/压缩等非回合调用，**不能一律丢弃 session scope**。同样，不能把瞬时 agent.status.updated 的累计 total/currentTurn 再计一次。wire metadata 的 protocol_version（研究中声明 1.5）、事件结构和应用版本共同决定解析器路由；不能只看到相同事件名就套所有历史版本。

旧 Python Kimi CLI 的 wire 格式与存储布局单独适配。对旧版本 session scope 是否累计，必须查该版本官方源码与样本，不沿用新版推断。迁移工具产生的 `token_counting.measured`/contextTokens 是上下文测量，不是完整消费账本；迁移记录可能无法恢复历史费用，应显示缺失。

子 Agent 与主 Agent分别采集，避免父汇总再次计入。fork/shared history 必须按官方日志树的继承关系去重；context.undo 不撤销真实已发生用量。进程崩溃前未落盘的 usage 无法保证恢复，需注明“日志记录精度”，不能承诺与服务器账单 100% 相等。

可选连接用户已运行且明确配置的 Kimi 本机 Server：先 GET `/api/v1/meta` 协商版本；会话 `/api/v1/sessions/{id}/snapshot` 用于对账，文档说明 plain 会话可能返回全零，不能覆盖日志已有真实统计；`/api/v1/oauth/usage` 用于账户计划快照。这是官方本机 Server API，不是公开厂商余额 HTTP API。不得为读取用量启动模型调用。

必需夹具：新协议头、turn/session scope、主子 Agent、快照与增量并存、fork 前缀、undo、压缩、崩溃尾行、unknown protocol、旧 Python 格式、迁移后仅上下文测量、同数字独立调用、模型变化。

### A09 Claude Desktop 条件网关

上游仅覆盖模型映射经过本地路由的流量。本项目生成手动配置说明，不自动修改 Desktop。默认关闭，开启后显式注明“仅网关流量”。是否需要具体 bridge、可用协议及版本在 P0 做一次可重复的 Desktop 模型映射演示后冻结。

非流式与流式 usage 都验证；相同 request 的多次状态更新归为一条。断流、取消、工具调用和重试保留 coverage 信息。未知与日志重复时按独立域统计，遵守数据契约。

## 4. 统一兼容状态与验收

每个适配器维护：官方产品版本范围、协议/schema 版本、OS、fixture 列表、最后验证日期、字段能力、已知缺口、证据。状态为 researched / implementing / fixture_verified / live_verified / incompatible；只有 fixture_verified 且两个系统路径/文件行为验证通过，才进入可发布支持表。没有真人账号可做 live 验证时应披露，不伪造。

自动验收包含：全量=多次增量结果、重扫不变、重启恢复、游标事务失败重放、相同数字不丢计、复制/分叉不重计、日志截断、未知字段不崩溃、缓存分量不重复、源文件不被写入。对 supported schema 合成夹具，Token 算术误差必须为 0。

对照 cc-switch 时对比原始记录和双方口径，不要求机械复现其总数；任何修正（例如包含关系）写明源码依据与差异。稳定版必须具备 A01–A09 的声明范围，不能将未完成项目静默移到“以后支持”。
