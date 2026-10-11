# A08 上游证据：Kimi Code 新版（TypeScript）与旧版 kimi-cli（Python）的用量/wire 格式

- 核查日期：2026-10-11
- 新版固定提交：`c9f01f07388abc2cdca40c9c5267f58c65086710`（`MoonshotAI/kimi-code`，commit message `refactor(tower): coordinate missions with deterministic orchestration (#4074)`）
- 旧版仓库：`MoonshotAI/kimi-cli`（**已归档**，`archived: true`，`pushed_at 2026-09-22`），本次读取 `main` = `9ab1286 feat(cli): short-circuit entry points to a Kimi Code installer (#2666)`
- 方法：先用 `curl` 拉任务指定的 raw 文件（7 个路径全部 **200**，无 404，**未使用** tree API 回退）；随后用 `git init` + `git fetch --depth 1 origin <sha>` 浅克隆固定提交到本地，用本地 grep 定位调用方与行号，本文件所有新版行号均取自该固定提交的工作区。旧版同样浅克隆 `main`。**未运行**任何 CLI、未读取任何用户真实日志/密钥/会话目录。
- 证据 URL 前缀（新版）：`https://raw.githubusercontent.com/MoonshotAI/kimi-code/c9f01f07388abc2cdca40c9c5267f58c65086710/`
- 证据 URL 前缀（旧版）：`https://raw.githubusercontent.com/MoonshotAI/kimi-cli/9ab1286/src/...`（下称 `kimi-cli@9ab1286`）

## 0. 读取清单

| 任务指定文件 | 行数 | 状态 |
| --- | --- | --- |
| `packages/agent-core-v2/src/agent/usage/usageOps.ts` | 34 | 200 |
| `packages/agent-core-v2/src/agent/usage/usageEvents.ts` | 96 | 200 |
| `packages/agent-core-v2/src/session/usage/usageAgentModel.ts` | 90 | 200 |
| `packages/agent-core-v2/src/human/llm/usage.ts` | 40 | 200 |
| `packages/agent-core-v2/src/wire/record.ts` | 52 | 200 |
| `packages/agent-core-v2/src/wire/tree/fork.ts` | 171 | 200 |
| `packages/migration-legacy/src/sessions/wire-writer.ts` | 118 | 200 |

为回答“何时发 session scope”“fork/undo 记录什么”“磁盘布局”“迁移丢失什么”“Server API 字段”这几问，另读了 30+ 个支撑文件（本地克隆内检索得出），下文逐处引用。**未发生路径修正**：任务给的 7 个路径与固定提交中的实际路径完全一致。

---

## 1. 磁盘目录布局

### 1.1 会话根与 workDirKey

- 数据根默认 `~/.kimi-code`，可由 `KIMI_CODE_HOME` 覆盖，优先级为“环境变量 > `~/.kimi-code`”：`apps/kimi-code/src/utils/paths.ts:36-44`（`getDataDir()`，注释在 `:36-37`）。
- 会话根 = `<KIMI_CODE_HOME>/sessions`：`packages/agent-core-v2/src/app/bootstrap/bootstrapService.ts:45`（`this.sessionsDir = join(options.homeDir, 'sessions')`），scope 名在 `:49-57` 的 `scopes.sessions = relative(homeDir, join(homeDir,'sessions'))`。
- 会话目录 = `<homeDir>/sessions/<workspaceId>/<sessionId>`：`packages/agent-core-v2/src/workspace/sessionLifecycle/internal/addressing.ts:11-13`（`sessionDirOf`）+ `:3-5`（`workspacePersistenceScope`），而 `handlerScope = workspaceContext.persistenceScope`（`sessionLifecycleService.ts:202-204`），该值在 `packages/agent-core-v2/src/workspace/workspaceInstance/workspaceInstanceManagerService.ts:203` 形如 `` `${scope('sessions')}/${workspace.id}` ``。
- `workDirKey` 的确切算法：`packages/agent-core-v2/src/_base/utils/workdir-slug.ts:3-23`
  - 常量：`MAX_WORKDIR_SLUG_LENGTH = 40`、`WORKDIR_KEY_PREFIX = 'wd_'`、`HASH_LENGTH = 12`（`:3-5`）
  - slug 化：小写、非 `[a-z0-9._-]` 折叠为 `-`、去首尾 `-`、截 40 字符，空/`.`/`..` → `workspace`（`:7-15`）
  - 键：`wd_<slug>_<sha256(normalized).slice(0,12)>`，其中 `normalized = workDir.replace(/\\/g,'/').replace(/\/+$/,'')`，slug 取路径最后一段（`:17-23`）
- 会话 id 形如 `session_<uuid>`：`sessionLifecycleService.ts:903-905`（`createSessionId()`）；**迁移来的旧会话不是这个形状**，而是 `ses_<旧uuid>`（`migration-legacy/src/sessions/migrate-one.ts:37`）。

### 1.2 会话目录内容

官方文档给出的两层结构（`docs/zh/guides/sessions.md:7-25`、`docs/zh/configuration/data-locations.md:28-57`）：

```
$KIMI_CODE_HOME
├── session_index.jsonl          # 会话索引（顶层，不在会话目录内）
└── sessions/
    └── <workDirKey>/
        └── <sessionId>/
            ├── state.json
            └── agents/
                ├── main/wire.jsonl
                └── <subagentId>/wire.jsonl
```

`data-locations.md:73-84` 补充会话目录内的其他文件：`upcoming-goals.json`、`agents/main/plans/<id>.md`、`logs/kimi-code.log`（仅当有诊断事件）、`tasks/<task_id>.json` 与 `tasks/<task_id>/output.log`、`cron/`。

代码侧印证：

- wire 文件名固定 `'wire.jsonl'`：`packages/agent-core-v2/src/wire/record.ts:3`（`AGENT_WIRE_RECORD_KEY`）。
- agent 子目录 `<sessionScope>/agents/<agentId>`：`addressing.ts:15-17`（`agentScopeOf`）。
- wire reader 的路径形状也按 `sessions/<workDirKey>/<sessionId>/agents/<agentId>/wire.jsonl` 解析：`packages/agent-core-v2/src/wire/wireService.ts:597-611`（`extractLegacyPlanRevisionKey` 要求 `segments[0]==='sessions'`、`segments[3]==='agents'`）。
- `session_index.jsonl` 每行三字段 `{ sessionId, sessionDir, workDir }`：`sessionLifecycleService.ts:325-333`（`appendSessionIndexEntry`），与文档 `data-locations.md:73` 一致。
- **易错点**：session picker 定位会话**只按** `readdir(encodeWorkDirKey(workDir))`，**不读** `session_index.jsonl`（`packages/migration-legacy/src/sessions/workdir-bucket.ts:5-19` 注释明说）。索引与目录桶是两套机制，采集时不要只用其中一个。
- `state.json` 内容（新引擎）：`id/version/cwd/createdAt/updatedAt/archived/archivedAt/title/titleKind/forkedFrom/agents/lastPrompt/lastTurnReason/custom`，见 `sessionLifecycleService.ts:641-655`；`agents` 每项为 `{homedir,type,parentAgentId,forkedFrom?,labels?}`（`:627-639`）。

### 1.3 Windows 路径

- 数据根在 Windows 为 `C:\Users\<name>\.kimi-code`：`docs/zh/configuration/data-locations.md:11`。
- 托管二进制在 Windows 为 `bin/rg.exe` / `bin/fd.exe`：`data-locations.md:46-47`。
- 会话桶名对 Windows **不敏感于分隔符**：`encodeWorkDirKey` 先 `replace(/\\/g,'/')` 再哈希（`workdir-slug.ts:18`），即 `C:\a\b` 与 `C:/a/b` 得到同一 hash。但注意 slug 只取**最后一段**，路径尾部分隔符会被去掉（`:18`）。
- 工作区 id 去重对 Windows 形路径做**小写归一**：`workdir-slug.ts:25-31`（`WIN_SHAPED = /^(?:[A-Za-z]:[\\/]|\\\\|\/\/)/`，命中则 `toLowerCase()`）。即 `\\HOST\SHARE\Proj` 与 `//host/share/proj` 视为同一工作区。
- `migration-legacy` 有一处显式历史坑：早期本地实现用 `node:path` 会产生反斜杠，而 `encodeWorkDirKey` 用 `pathe`（全平台正斜杠），导致迁移会话落到 picker 读不到的桶；现已改为直接复用 `encodeWorkDirKey`（`workdir-bucket.ts:14-20`）。

---

## 2. wire.jsonl 的记录信封

### 2.1 协议头与磁盘形状

- 头部记录类型与字段：`packages/agent-core-v2/src/wire/record.ts:23-27`

  ```ts
  export interface WireMetadataRecord extends WireRecord {
    readonly type: 'metadata';
    readonly protocol_version: string;
    readonly created_at: number;
  }
  ```

  构造为 `{ type:'metadata', protocol_version: WIRE_PROTOCOL_VERSION, created_at: now }`（`record.ts:38-44`）；`isWireMetadataRecord` 要求两个字段类型匹配（`:46-52`）。

- **当前 protocol_version = `"1.5"`**：`packages/agent-core-v2/src/wire/migration/migration.ts:19`（`export const WIRE_PROTOCOL_VERSION = '1.5'`）。迁移链为 `1.0 → 1.1 → 1.2 → 1.3 → 1.4 → 1.5`（`migration.ts:5-9, 29-35`；版本比较函数 `:96-107`；读到更高版本时 `isNewerWireVersion` 为真，`:37-39`）。
- 磁盘形状（官方生成文档原话，`packages/agent-core-v2/docs/wire-manifest.d.ts:14-20`）：
  - 日志首行是 `{"type":"metadata","protocol_version","created_at"}`；
  - 之后每条记录是 `{"type", ...payload, "time"}`，**payload 是展开在顶层的**（不是嵌套在 `payload` 下）。
- 生成器声明 `protocol_version: "1.5"`，并注明“一个声明对应一个 durable 记录类型”（`wire-manifest.d.ts:7-13`）。

### 2.2 record 类型枚举

`wire-manifest.d.ts:23-91` 是**完整索引，共 64 个 durable 类型**（注释 `:23` “Index (64 record types)”）。全部列出：

```
config.update                       context.append_loop_event      context.append_message
context.apply_compaction            context.clear                  context.undo
cron.add                            cron.cursor                    cron.delete
file_history.checkpoint             file_history.tracked           forked
full_compaction.begin               full_compaction.cancel         full_compaction.complete
goal.clear                          goal.create                    goal.update
interaction.request                 interaction.resolved           interruptionReminder.recorded
llm.request                         llm.tools_snapshot             mcp.tools_discovered
permission.record_approval_result   permission.set_mode            plan_mode.cancel
plan_mode.enter                     plan_mode.exit                 plan.revision
plugin.session_start                profile.bind                   prompt.aborted
prompt.completed                    prompt.steered                 runtime.set_binding
subagent.cancelled                  subagent.completed             subagent.failed
subagent.spawned                    subagent.started               swarm_mode.enter
swarm_mode.exit                     task.started                   task.terminated
task.waitDelivered                  token_counting.measured        token_counting.rebased
token_counting.truncated            token_counting.turn_recorded   tools.register_user_tool
tools.reset_active_tools            tools.set_active_tools         tools.unregister_user_tool
tools.update_store                  tower_mode.enter               tower_mode.exit
turn.cancel                         turn.ended                     turn.prompt
turn.steer                          turn.step.interrupted          turn.step.retrying
usage.record
```

关键结构性事实：

- `durable` 与 `observable` 是**两个独立开关**，默认都为 `false`：`packages/agent-core-v2/src/app/event/event2.ts:22-25`。只有 `durable === true` 才落盘（`event2.ts:71` “`if (!cls.durable) return;`”）。生成器注释明确“列出的每个类型按构造即 durable；瞬时 Event2 永不进入注册表，因此没有 persisted 标志”（`wire-manifest.d.ts:19-20`）。
- **磁盘上会出现不在该 64 项清单里的类型，采集时必须容错**，至少以下三类：
  1. `metadata`（首行头部，见上）。
  2. undo/fork 分支切换写入的原始记录 `agent.switched` 与 **`context.undone`**——清单里只有 `context.undo`，没有 `context.undone`。它们由 `wireService.switchBranch` 直接 `appendRecord`（`wireService.ts:220-222`），类型常量在 `wire/tree/tree.ts:4`（仅 `AGENT_SWITCHED_TYPE`）与 `fork.ts:169`。
  3. 旧的 human 模式记录：`packages/agent-core-v2/src/wire/human.ts:4-21` 定义了额外集合 `agent.message.appended` / `agent.turn.started` / `agent.turn.ended`，以及任何以 `human.` 为前缀的类型，都会被 wire reader 当作 human 记录处理（`isHumanRecordType`）。
- **`time` 不属于 payload schema，是写盘层追加的。** 生成文档的 payload 声明里没有 `time`（`wire-manifest.d.ts:952-958` 的 `usage.record` 只有 `agentId/model/usage/usageScope`），但磁盘上的例子里带 `"time"`（`apps/vis/server/test/fixtures/sessions/sample-main/agents/main/wire.jsonl:10`），而磁盘形状说明也写明是 `{"type", ...payload, "time"}`（`wire-manifest.d.ts:16-17`）。每个记录类型是否带 `time`、`time` 是毫秒还是别的单位，**只能以实际文件为准**（本提交的样例夹具均为 epoch 毫秒）。

---

## 3. usage.record 完整字段与两种 usageScope

### 3.1 字段

`packages/agent-core-v2/src/agent/usage/usageOps.ts:13-30`：

```ts
const usageRecordSchema = z.object({
  agentId: z.string(),
  model: z.string(),
  usage: z.custom<TokenUsage>(),
  usageScope: z.custom<UsageRecordScope>().optional(),
});
export class UsageRecord extends AgentEvent2<...> {
  static override readonly type = 'usage.record';
  static override readonly durable = true;      // ← 每次调用一条 durable 记录
  static override readonly schema = usageRecordSchema;
}
```

`UsageRecordScope = 'session' | 'turn'`（`usageOps.ts:7`）。生成文档的 payload 形状一致（`wire-manifest.d.ts:952-958`：`agentId`、`model`、`usage: TokenUsage`、`usageScope?: 'session' | 'turn'`）。

`TokenUsage` 的**四个必填字段**（`packages/agent-core-v2/src/human/llm/usage.ts:1-7`）：

```ts
export interface TokenUsage {
  inputOther: number;
  output: number;
  inputCacheRead: number;
  inputCacheCreation: number;
  raw?: Record<string, unknown>;   // 可选，上游原始载荷
}
```

- 空值：`emptyUsage()` → 四项全 0（`usage.ts:9-11`）。
- 派生量：`inputTotal = inputOther + inputCacheRead + inputCacheCreation`（`:13-15`）；`grandTotal = inputTotal + output`（`:17-19`）。
- 累加：`addUsage` 四项逐项相加（`:21-28`）；`mergeUsagePatch` 用于部分补丁（`:30-39`）。
- **口径警告**：`inputOther` 是“不含缓存的输入”，三者相加才是输入总量；与旧版 Python 的 `input_other/output/input_cache_read/input_cache_creation` 同名同义（见 §8），但**不要**把 `inputOther` 当成 `input_tokens`。

### 3.2 谁发 record、`usageScope` 怎么定

唯一生产者是 LLM 请求完成后的记账调用：

- `packages/agent-core-v2/src/agent/llmRequester/llmRequesterService.ts:477-485`：

  ```ts
  void this.usage.record(this.scopeContext.agentContext, request.modelAlias, usage ?? emptyUsage(), request.source);
  if (usage !== undefined) { this.tokenCounting.measured(this.scopeContext.agentContext, request.messages, [message], usage); }
  ```

- scope 判定在 `packages/agent-core-v2/src/session/usage/usageAgentModel.ts:34-44`：

  ```ts
  const usageScope: UsageRecordScope = input.source?.type === 'turn' ? 'turn' : 'session';
  ```

  即：**source 是 `turn` 就写 `turn`；其它一切（含 `operation`、以及完全没有 source 的调用）都写 `session`。**

- `source` 的取值域只有两个（`packages/agent-core-v2/src/agent/llmRequester/llmRequester.ts:14-26`）：`{type:'turn', turnId, step?, logFields?}` 与 `{type:'operation', turnId?, requestKind?, logFields?}`。

在固定提交里，**全仓库只有一处**把 `source.type` 设为 `'operation'`：全量压缩。

- `packages/agent-core-v2/src/agent/fullCompaction/fullCompactionService.ts:665-678`：

  ```ts
  const request = this.llmRequester.start({
    messages, maxOutputSize: compactionMaxOutputSize,
    source: { type: 'operation', turnId: active.originTurnId, requestKind: 'full_compaction', logFields: { droppedCount } },
  }, undefined, signal);
  ```

  检索方式：`grep -rn "type: 'operation'" --include=*.ts packages apps`（排除测试）在该提交下只命中 `llmRequester.ts:22`（类型定义）与 `fullCompactionService.ts:670`（唯一使用点）。

**标题生成不产生 usage.record**：`SessionTitleService` 不走 agent 的 llmRequester，而是直接调托管 OAuth 的 `fetchChatTitle`（`packages/agent-core-v2/src/session/sessionTitle/sessionTitleService.ts:4-5` 导入，`:122` 调用；失败只记 debug 日志 `:118,135,141`）。因此：

- `usageScope === 'session'` 的记录，当前唯一来源是**全量压缩（auto/manual 都会，`fullCompactionService.ts:535` 是 auto 入口）**；
- 标题生成、以及任何非 turn 的旁路请求，都不记 usage；
- 若未来新增 `operation` 调用方，它会自动落到 `session` scope——这是“默认落到 session”的设计后果，采集端不要假设 session scope 只等于“压缩”。

### 3.3 是否每次调用一条 durable record

是。`UsageRecord.durable = true`（`usageOps.ts:22`），`llmRequesterService.ts:477-482` 在**每次成功的 LLM 请求**（含 `usage` 缺失时用 `emptyUsage()` 兜底）后无条件发一条；`record()` 本身在 `recorded` promise 完成后才继续（`usageAgentModel.ts:34-61`）。

因此对同一 agent 的 wire.jsonl：**一次请求 = 一条 `usage.record`**，`usageScope` 标注明细属于当前轮次还是会话级旁路调用。注意 usage 缺失时会写出**全零**记录（`emptyUsage()`），全零记录是真实记录而非占位，采集端不能按“全零即丢弃”无条件过滤（对比 `cc-switch` 的 OpenCode 导入器确实丢弃全零行——那是另一上游的行为，不适用于此处）。

### 3.4 内存模型

`UsageAgentModel`（`usageAgentModel.ts:21-31`）用 `byModel: Record<string, TokenUsage>` 保存**按模型的累计值**，收到每条 `UsageRecord` 就 `addUsage` 进去；`record()` 之后再发一条 `AgentStatusUpdated`（`:57-60`）。模型状态是 `byModel`，`status()` 再做汇总（见 §4）。

---

## 4. agent.status.updated 的 total/currentTurn 累计语义

### 4.1 字段与产生时机

`packages/agent-core-v2/src/agent/usage/usageEvents.ts:9-27`：

```ts
export interface AgentStatusUpdatedPayload {
  readonly agentId: string;
  usage?: UsageStatus;
  swarmMode?; towerMode?; towerExitReason?; planMode?; model?; thinkingEffort?;
  maxContextTokens?: number; contextTokens?: number; permission?: PermissionMode;
}
export class AgentStatusUpdated extends AgentEvent2<AgentStatusUpdatedPayload> {
  static override readonly type = 'agent.status.updated';
  static override readonly observable = true;      // ← 只有 observable
}
```

**`durable` 没有声明 → 默认 `false`（`event2.ts:24`）→ 不写 wire.jsonl。** 交叉验证：64 个 durable 类型的清单里**没有** `agent.status.updated`（`wire-manifest.d.ts:23-91`）。它是纯运行时广播事件。

产生点有两处：

1. 每次 `usage.record` 之后：`usageAgentModel.ts:57-59`（`new AgentStatusUpdated({ agentId, usage: this.status() })`）。
2. 每次 token 计数变化：`packages/agent-core-v2/src/session/tokenCounting/tokenCountingAgentModel.ts:39,50,61,78`（`new AgentStatusUpdated({ agentId, contextTokens: tokens })`）。

### 4.2 total / currentTurn 的确切含义

`UsageStatus`（`packages/agent-core-v2/src/agent/usage/usage.ts:21-25`）：

```ts
export interface UsageStatus {
  readonly byModel?: Record<string, TokenUsage>;
  readonly total?: TokenUsage;
  readonly currentTurn?: TokenUsage;
}
```

`usageAgentModel.ts:63-79` 的实现：

- `byModel`：本 agent 生命周期内**所有** `usage.record`（不分 `turn`/`session` scope）按模型累计；
- `total`：`byModel` 各项之和，即**本 agent 的会话累计**（`total = Σ byModel`，`:69-73`）；
- `currentTurn`：只累加**当前 turnId** 的记录，`turnId` 变了就重置为该轮首条 usage（`:45-56`）；非 turn 的请求不参与，因此 `currentTurn` 可能为 `undefined`（`:77`）。

### 4.3 为什么不能再计入

三条互相独立的理由：

1. **total 不是 delta，是快照。** 每次 `usage.record` 后都把“会话累计”的**全量值**重新发一遍（`usageAgentModel.ts:57-59`）。同一次调用会广播多次同值/递增值的状态（usage 变化一次 + 每次 token 计数变化一次）。把每条 `agent.status.updated` 当事件累加，就是同一批 token 被重复计 N 遍。
2. **服务端自己也是这么读的——直接取快照字段，不做求和。** `packages/kap-server/src/routes/snapshot.ts:169-180` 的 `toSnapshotUsage` 把 `status.usage?.total` 的四项**直接映射**到 `input_tokens/output_tokens/cache_read_tokens/cache_creation_tokens`，`context_tokens` 取 `status.contextTokens`；`readLegacyStatus` 也只是 `usageService.status(context)` 一次取值（`packages/kap-server/src/services/legacyStatus/legacyStatus.ts:118-146`）。没有任何一处对 `agent.status.updated` 做累加。
3. **恢复会话时 total 会从 wire 重放得出，历史已在里面。** `usage.record` 是 durable 且挂在 `UsageAgentModel` 上（`usageAgentModel.ts:89` `events: [UsageRecord]`、`features/usage/usageFeature.ts:13` `contributeAgentModel`），resume 时历史记录被重放进 `byModel`，所以 `total` 天然覆盖整个会话（含 resume 前）。再叠一笔“启动时读 wire 里所有 usage.record 求和”，等于把历史算两遍。

一句话结论（供数据契约引用）：`agent.status.updated.usage.total` 与 `currentTurn` 是“最后写入胜出”的累计快照，`contextTokens` 同理。合法用法是进程内覆盖（last-wins），需要历史明细时**直接读 wire 的每一行 `usage.record`**，而不是对状态事件求和。

另注：`agent.status.updated` 只在进程存活期间可见，冷会话没有任何 `agent.status.updated` 可读；冷会话的 usage 只能由 `usage.record` 重放（或 `kap-server` 的 `readLegacyStatus` 在进程内解析）得到。

顺带一个下游口径细节：`packages/agent-core-v2/src/features/goal/goalService.ts:549-554` 把 `ctx.usage.output`（**仅输出 token**）累进 goal 的 tokensUsed 预算——所以 goal 的 token 计数与 `total` 的新增口径是两回事。

---

## 5. fork / undo / 子 Agent 的记录关系

### 5.1 fork 如何表达继承

fork **不在 wire 记录里写父会话 id**，继承关系分两处：

1. **`state.json` 的 `forkedFrom`**：`sessionLifecycleService.ts:641-655` 中 `forkedFrom: sourceId`（fork 出来的会话指向源会话）；每个 agent 条目还各自带 `forkedFrom: sourceAgent.forkedFrom`（`:637`）。
2. **每个 agent 的 wire.jsonl 末尾追加一条 `forked` 记录**：`sessionLifecycleService.ts:713-736`（`copyAgentWire` 在复制/切片后 `records.push(forkedRecord(args.agentId))`），定义在 `:907-909`：

   ```ts
   function forkedRecord(agentId: string): WireRecord { return { type: 'forked', agentId, time: Date.now() }; }
   ```

   payload 只有 `agentId`（生成文档 `wire-manifest.d.ts` 中 `ForkedPayload { _name:'forked'; agentId: string }`），另一条路径 `appendForkedMarker` 对已有 wire 追加时也保证 metadata 在首位（`:738-768`）。

- 若 harness 已落 metadata 头，`copyAgentWire` 保证首行是 metadata 且**尾部**是 `forked`（`:720-729`）。
- fork 语义副作用：`forked` 记录在重放时清除 goal（`packages/agent-core-v2/src/agent/replayBuilder/fold.ts:155-157` 调用，`:450-465` 实现 `applyForked`：`this.goal = undefined` 并追加一条 `origin.kind==='system_trigger', name:'goal_fork_cleared'` 的系统提醒）。文档侧印证：“已保存的 `/goal` 不会复制到派生会话”（`docs/zh/guides/sessions.md:88`）。
- 两种 fork 模式：`turnIndex` 给定时按轮切片（`sliceMainRecordsAtTurn`，`packages/agent-core-v2/src/workspace/sessionLifecycle/internal/forkTurnSlice.ts:32-68`），不给定时整份复制+打标记（`sessionLifecycleService.ts:613-623`）。子 Agent 按**时间水位**切片：`sliceSubagentRecordsAtTime`（`forkTurnSlice.ts:70-84`），cutoff 取主 agent 保留记录里最大的 `time`（`:59-66`）。

### 5.2 undo 记录什么

undo 是“分支切换”，一次写**三条**记录（`buildUndoSwitchRecords`，`packages/agent-core-v2/src/wire/tree/fork.ts:140-170`，由 `wireService.switchBranch` 按序 append，`wireService.ts:206-222`）：

| 记录 | `type` | 字段 | 出处 |
| --- | --- | --- | --- |
| switched | `agent.switched` | `agentId, branch, reason, base:{branch,line}, turns, legacyUndoLine, time` | `fork.ts:157-166` |
| legacyUndo | `context.undo` | `agentId, count, time` | `fork.ts:168` |
| undone | `context.undone` | `agentId, turns, fromTurnId, time` | `fork.ts:169` |

- `branch` 命名为 `b<N>`，`N = 已有边数+1`（`wireService.ts:204`）；`base` 指向被切走的旧分支与其行号（`fork.ts:150-154` 的 `base:{branch,line}`）。
- **`agent.switched` 是分支树的唯一结构信息来源**：`tree.ts:39-57`（`readSwitchEdge`）校验 `branch`/`base.branch`/`base.line` 类型，非法则计入 `malformedSwitchLines`；`parseTree` 据此切成 `segments`（`:68-106`）；`activeChain` 沿 `edge.base` 递归回到根、拼出有效链，并检测环与悬空分支（`:108-131`）。
- `turns` 是撤销的轮次数；`computeForkLine` 从链尾往前数可撤销锚点（`fork.ts:59-101`），遇到压缩摘要会抛 `ForkLineError('compaction_boundary')`，轮次不足抛 `'insufficient'`（`:11-21, 86-98`）。
- 兼容字段：`legacyUndoLine` 让 `parseTree` 把配对的老式 `context.undo` 行标记进 `pairedLegacyUndoLines`（`tree.ts:44-56, 80`），恢复链时跳过它们（`fork.ts:107-111`），避免新写的 `context.undo` 被当成“真正的撤销”重复折叠。
- 恢复链 `restorableChain` 会剔除 `context.undone` 与配对的 legacy undo，并把仍归属存活 prompt 的注入消息重新纳入（`fork.ts:103-133`）；`flattenChain` 是它对内存记录数组的入口（`:135-138`）。

### 5.3 子 Agent 与主 Agent 的关系

- **目录与身份**：每个子 Agent 是 `sessionDir/agents/<agentId>/wire.jsonl` 下**独立的 wire 文件**（`addressing.ts:15-17`）；`state.json.agents[agentId] = { type:'sub', parentAgentId:'main', ... }`（`sessionLifecycleService.ts:630-639`，主 agent 为 `type:'main', parentAgentId: undefined`）。
- **wire 内的关联**：主子关系由 `subagent.spawned` 的 `parentToolCallId` / `parentAgentId` / `callerAgentId` 字段表达（生成文档 `SubagentSpawnedPayload`：`subagentId, subagentName, parentToolCallId, parentToolCallUuid?, parentAgentId?, callerAgentId?, ...`），并配套 `subagent.started / completed / failed / cancelled`（清单 `wire-manifest.d.ts:64-68`，实现在 `packages/agent-core-v2/src/session/subagent/mirrorAgentRun.ts`）。
- **子 Agent 有独立的 usage**：usage 模型是 per-agent 的（`UsageAgentModelDefinition` 注册为 agent model，`usageFeature.ts:13`），`agentId` 落在每条 `usage.record` 里，因此子 Agent 的 token 记在**子 agent 的 wire**，不在主 agent 的 `total` 里。要算“会话总用量”必须**遍历所有 `agents/*/wire.jsonl`**，不能只读 main。
- 采集含义：主 agent wire 里的 `task.started/task.terminated` 与 `subagent.*` 记录是**元数据**，其 token 不在这条 wire 里。

---

## 6. migration-legacy 写出什么、迁移后无法恢复什么

### 6.1 目标位置与协议版本

- 目标目录：`<targetHome>/sessions/<encodeWorkDirKey(workdir)>/ses_<旧uuid>`（`migrate-one.ts:35-37`，桶名算法与运行期完全同一函数，`workdir-bucket.ts:20`）。
- 迁移写出的 wire 头部协议版本是 **`"1.0"`**，不是 1.5：`packages/migration-legacy/src/sessions/wire-writer.ts:7`（`export const WIRE_PROTOCOL_VERSION = '1.0'`），头部由 `:24-29` 写入。**依赖 migration 与 wire migration 链（`1.0→…→1.5`）把导入文件升级**，`migration.ts:29-35`。
- `state.json` 由 `writeSessionState` 写（`state-writer.ts:24-105`），并把来源信息压在 `custom` 下：`imported_from_kimi_cli: true`、`kimi_cli_source_path`、`kimi_cli_session_id`、`kimi_cli_wire_protocol`（旧 wire 头部版本）、`imported_at`、`auto_archive_exempt`、`vscode_legacy_approval`（`:88-102`）。`imported_from_kimi_cli` 也是重跑幂等判定（`migrate-one.ts:240-248`）。
- 同时向 `<targetHome>/session_index.jsonl` 追加 `{sessionId, sessionDir, workDir}`（`migration-legacy/src/session-index.ts:5-18`，路径 `paths.ts:27`）。

### 6.2 主 agent wire 写了哪些记录

`writeMainAgentWire`（`wire-writer.ts:20-67`）按顺序写：

1. `metadata`（protocol_version `1.0`，`created_at` 取旧 `wire_mtime`×1000，否则取源文件 mtime，`migrate-one.ts:104-115`）；
2. 由 `context.jsonl` 翻译出的轮次记录：每个分组轮写一条 `turn.prompt` + 若干 `context.append_message` + （有 assistant 内容时）一条 `turn.ended`（`turn-structure.ts:52-82`；分轮规则 `:25-46`：**每条 user 消息开一轮**，并以 `turnId` 序号递增）；
3. `token_counting.measured`（仅当能从旧 context 取到最后一次用量，见下）；
4. `tools.update_store`（`key:'todo'`，仅当旧 `state.json` 有 todos，`wire-writer.ts:55-65`）；
5. 子 Agent 的 `task.started` / `task.terminated` 被**插到对应位置**（`insertSubagentTaskRecords`，`wire-writer.ts:74-117`：有主工具调用时 `task.started` 插在携带该 Agent 工具调用的 assistant 消息前，`task.terminated` 插在工具结果后；定位不到就追加到尾部）。子 Agent 各自另有 `<target>/agents/<agentId>/wire.jsonl`（`subagents.ts:41-52`），且也写自己的 `token_counting.measured`（`subagents.ts:265`）。

注释说明了为什么必须合成 `turn.prompt/turn.ended`：只有裸 `context.append_message` 会让恢复后的 turn 时钟停在 0，第一个实时轮次会编号为 t0 与导入历史冲突（`wire-writer.ts:30-34`）。

### 6.3 `token_counting.measured` / `contextTokens` 语义

- 记录形状（`packages/agent-core-v2/src/agent/tokenCounting/tokenCountingOps.ts:19-25`，schema 在 `:14-18`）：

  ```ts
  { type:'token_counting.measured', agentId, length: number, tokens: number, time }
  ```

  同族还有 `rebased{agentId,length,tokens,measured}`（`:47-55`）、`truncated`（`:27-36`）、`turn_recorded{...turnId}`（`:59-67`）。
- **`tokens` 是“上下文大小的锚点”，不是累计用量。** 迁移侧取值来源：`extractLastUsageTokenCount`（`translator.ts:73-99`）扫描旧 `context.jsonl` 里最后一条 `{"role":"_usage","token_count":N}` 的 `token_count`；注释直接写明“kimi-cli 自己在会话末尾测得的上下文大小，用来种一个 measured 锚点，使恢复会话显示实测上下文而非估算值”（`translator.ts:74-78`）。`length` 写的是翻译后消息条数（`wire-writer.ts:45`）。
- 运行侧同一语义：`tokenCounting.measured(agent, messages, [message], usage)` 在每次 LLM 调用后被调用（`llmRequesterService.ts:483-485`），`contextTokens` 由 `tokenCountingAgentModel` 发进 `agent.status.updated`（`tokenCountingAgentModel.ts:39,50,61,78`），最终出现在 snapshot 的 `context_tokens`（`snapshot.ts:177`）。
- **因此迁移得到的 `token_counting.measured.tokens` 只能用于“上下文占用”展示，绝不能当作累计 token 用量或额度消耗。** 二者不可混用（这也与本次任务的既有约束一致：Token/上下文长度/额度/余额不可混用）。

### 6.4 迁移后哪些历史无法恢复（本次核查的明确结论）

| 丢失/不可恢复项 | 证据 |
| --- | --- |
| **逐次调用的 `usage.record` 全部不存在**：迁移流程**从不写任何 `usage.record`**；主 agent wire 只写一条 `token_counting.measured`（`wire-writer.ts:41-51`），每个子 agent wire 也各写一条（`subagents.ts:265`）。因此新引擎的 `UsageStatus.total` / `byModel` 对导入会话**从 0 开始**（在第一次实时调用之前），历史 token 用量与额度消耗**不可恢复**。 | `wire-writer.ts:20-67`（通读无 usage.record）；`usageAgentModel.ts:63-79`（total 仅来自 usage.record） |
| **每次请求的输入/输出/缓存明细**（旧 `StatusUpdate.token_usage`）**不迁移**：迁移侧只读 `context.jsonl`，旧 `wire.jsonl` 仅用于“UI 展示增强”（工具调用输入展示），读失败也不影响主流程。 | `migrate-one.ts:67-79`（`extractToolCallDisplays(oldWireText)` 注释：“missing/corrupt wire … only means UI display enrichment is unavailable”） |
| **原始轮次边界与时间**：轮次按“每条 user 消息一轮”**重新合成**，`turn.prompt`/`turn.ended` 的 `time` 全部写同一个 `createdAtMs`，`turn.ended` 的 `reason` 恒为 `'completed'`。真实的轮次时长、被取消/失败的轮次终态、`StepBegin` 步号、thinking 段落的时序**无法恢复**。 | `turn-structure.ts:52-82`（`time: opts.time`、`reason:'completed'`）；`migrate-one.ts:143-149` |
| **旧 wire 的完整事件流**：不重放、不导入，只提取工具调用展示文本。旧协议版本号仅作为字符串留档在 `custom.kimi_cli_wire_protocol`。 | `migrate-one.ts:117-135`（读首行 `protocol_version`）、`state-writer.ts:92` |
| **`context.jsonl` 中非消息行**：`_system_prompt` 之外，`_usage` 只取最后一个数字、`_checkpoint` 等控制行不迁移；翻译仅保留 user/assistant/tool 等可用角色。 | `translator.ts:43-63`（`analyzeContextContent` 的 `USABLE_ROLES` 判定）、`:79-99` |
| **悬空工具调用被补齐**：`closeDanglingToolCalls` 会给没有结果的工具调用补上收尾，因此“当时确实没返回”和“被补齐”在迁移后不可区分。 | `migrate-one.ts:77-79` |
| **子 Agent 状态机**：只按终态映射（`completed/failed/lost`，`mapSubagentStatus`，`subagents.ts:175-179`；判定集合在 `:23-27`），历史中的中间状态（`running_foreground` 等）不保留。 | `subagents.ts:175-179` |
| **旧会话 id**：新 id 变为 `ses_<旧uuid>`；旧 uuid 保存在 `custom.kimi_cli_session_id`。旧桶名（`md5(path)`）只用于反查，不参与新布局。 | `migrate-one.ts:37`、`state-writer.ts:91`、`workdir-bucket.ts:22-24` |

**不丢失的**：文件 mtime 会被回写到原始时间以保住“最近会话”排序（`migrate-one.ts:171-178, 206-215`）；todos、custom title、archived、additional_dirs、审批模式、子 Agent 上下文历史都会迁移（`wire-writer.ts:55-65`、`state-writer.ts:27-37, 54-102`、`subagents.ts:41-52`）。

---

## 7. Kimi 本机 Server API

官方文档站页面 **可达**：`https://www.kimi.ai/help/kimi-code/cli-server-api` 返回 200（HTML SPA，内容在 RSC 载荷中，含 `/api/v1/meta`、`snapshot`、`oauth/usage` 等字符串）。仓库内同一份文档的权威副本是 `docs/zh/reference/server-api.md`（2418 行）与 `docs/en/reference/server-api.md`；文档明确“每个端点精确的机器可读 schema 以服务的在线规范为准：`GET /openapi.json` 与 `GET /asyncapi.json`，两者都需鉴权；本页与在线规范不一致时以在线规范为准”（`server-api.md:5`），并警告这是**实验性**接口、端点与字段可能随版本变化（`:7-9`）。

### 7.1 通用约定

- 默认地址 `http://127.0.0.1:58627`；端口占用时最多重试 100 个端口；`--port/--host` 可改；运行中的实例登记在 `~/.kimi-code/server/instances/`（`server-api.md:15`）。
- 鉴权：除 `OPTIONS` 预检、`GET /api/v1/healthz`、非 `/api/` 静态资源外，**所有** `/api/*`（含 `/openapi.json`、`/asyncapi.json`）都要 bearer token。REST 用 `Authorization: Bearer <token>`；WebSocket 升级接受同请求头或子协议 `kimi-code.bearer.<token>`（`:19-25`）。失败 HTTP 401 且信封 `code=40101`；非 loopback 绑定下同源 60 秒内失败 10 次封禁 60 秒、期间 HTTP 429 / `code=42901`（`:27`）。
- 响应信封：`{ code, msg, data, request_id }`，`code=0` 成功；**HTTP 状态几乎总是 200，业务结果以 `code` 为准**；例外：401/429、创建供应商 201、删除供应商 204、二进制/流式端点 206/304、部分下载错误用真实 404/500（`:31-56`）。错误码分段见 `:58-73`。

代码印证：`packages/kap-server/src/middleware/auth.ts:40`（`GET /api/v1/healthz` 免鉴权）、`:14`（`BEARER_PREFIX = 'Bearer '`）、`:12`（`AUTH_ERROR_CODE = 40101`）。

### 7.2 三个被点名的端点

#### `GET /api/v1/meta`

文档 `server-api.md:159-176`；实现 `packages/kap-server/src/routes/meta.ts:27-64`（`registerMetaRoute`；`defineRoute({method:'GET', path:'/meta', success:{data: metaResponseSchema}, tags:['meta']})` 在 `:46-61`，注册在 `:63`）。

- 方法/路径：`GET /api/v1/meta`，需 bearer token（除免鉴权白名单外）。
- 请求参数：无。
- 响应 `data`（`:28-44` 的 `staticData` + `:57-58` 的两个实时字段）：

  | 字段 | 类型 | 说明 |
  | --- | --- | --- |
  | `server_version` | string | 服务版本 |
  | `capabilities` | object | `websocket/file_upload/fs_query/mcp/tasks/terminal`，**均恒为 `true`**（`meta.ts:30-38`） |
  | `server_id` | string | 实例唯一 id |
  | `started_at` | string | ISO 8601 |
  | `open_in_apps` | array | 可为 `open-in` 目标的宿主应用；目前恒为空 |
  | `dangerous_bypass_auth` | boolean | 是否以 `--dangerous-bypass-auth` 启动 |
  | `backend` | string | `v1`/`v2`，本服务恒为 `'v2'`（`meta.ts:42`） |
  | `web_title` | string? | `--web-title`，未设置时省略 |
  | `experimental_flags` | object | 开关 id → 布尔，按请求实时解析（`meta.ts:57`） |
  | `features` | array | `{name,state,meta}`，`state` ∈ `Pending/Activating/Active/Unloading/Failed`（`meta.ts:58`） |

  与文档 `server-api.md:165-176` 逐项一致。

#### `GET /api/v1/sessions/{session_id}/snapshot`

文档 `server-api.md:874-884`；实现 `packages/kap-server/src/routes/snapshot.ts:63-80`（描述：“Atomic session snapshot for client rebuild: state + as_of_seq watermark + epoch”，`tags:['sessions']`，路由定义 `:66-78`）。

- 方法/路径：`GET`，需 token；路径参数 `session_id`（schema `snapshot.ts:43-45`，非空）。
- 响应 `data`：`{ as_of_seq, epoch, session, messages, in_flight_turn, subagents?, pending_approvals, pending_questions }`。
  - `session` 为标准的 session 线上对象；**与其它会话端点不同，这里内嵌的 session 会填真实 `usage` 与实时 `agent_config.model`**（文档 `:876`，代码 `snapshot.ts:122-123`）。
  - `messages` 为最新 100 条消息的分页对象（常量 `SNAPSHOT_MESSAGE_PAGE_SIZE = 100`，`snapshot.ts:34`；切片 `:127-128`）。
  - `in_flight_turn`：空闲为 `null`，已知时带 `current_prompt_id`（`:130-131`）。
- `usage` 字段的确切映射（`snapshot.ts:169-180`）：`input_tokens←total.inputOther`、`output_tokens←total.output`、`cache_read_tokens←total.inputCacheRead`、`cache_creation_tokens←total.inputCacheCreation`、`context_tokens←status.contextTokens`、`context_limit←status.maxContextTokens`。
  - **注意**：`input_tokens` 在这里等于 `inputOther`（**不含缓存输入**）；要还原输入总量须自行相加三项。这与 session 对象文档里的字段名 `usage{input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,context_tokens,context_limit?,total_cost_usd?,turn_count?}` 一致（`server-api.md:595`），并且文档明确“**在 snapshot 端点之外 usage 全为零**”（同处）——即普通列表/详情端点里的 usage 不可信。
- 错误：`40401` 会话不存在（schema 声明在 `snapshot.ts:71-74`，抛出/映射在 `:36-41` 与 `:85-89`）。

#### `GET /api/v1/oauth/usage`

文档 `server-api.md:246-254`；实现 `packages/kap-server/src/routes/oauth.ts:143-160`：

- 方法/路径：`GET /api/v1/oauth/usage`，需 token；query 参数 `provider`（默认 `managed:kimi-code`；schema `oauthLoginQuerySchema`）。
- 语义：“实时取自账号服务；**上游失败不会让信封失败**，以 `kind:'error'` 带内返回”（文档 `:248`）。
- 响应 `data`：`{ kind:'ok', quota }` 或 `{ kind:'error', message, status? }`（`status` 为上游 HTTP 状态码）。
- `quota = { usages, extraUsage }`（`packages/oauth/src/managed-usage.ts:85-89`）：
  - `usages` 按窗口：`limit5h` / `limit7d` / `monthTotal` / `monthCode`，每项 `{ usedRatio, resetAt? }`（`managed-usage.ts:61-73`；`usedRatio` 为 0–1 浮点，`resetAt` 为 RFC3339）。
  - `extraUsage`（可空）为按量付费钱包：`{ balanceCents, totalCents, monthlyChargeLimitEnabled, monthlyChargeLimitCents, monthlyUsedCents, currency }`（`managed-usage.ts:75-83`）。
- **上游真实端点**（对采集很有用）：`GET <base>/usages`，`base` 默认 `https://api.kimi.com/coding/v1`（`managed-usage.ts:8`），全球站 `https://api.kimi.ai/coding/v1`（`:9`），可用 `KIMI_CODE_BASE_URL` 覆盖（`:17-19`）；请求头 `Authorization: Bearer <token>` + `Accept: application/json`，超时 8 秒（`:228-234, 226`）。
- **上游原始字段名与本地投影的映射**（`parseManagedUsagePayload`，`managed-usage.ts:108-137`）：

  | 上游字段 | 本地字段 |
  | --- | --- |
  | `usages.limit_5h` / `limit_7d` / `limit_month_total` / `limit_month_code` | `limit5h` / `limit7d` / `monthTotal` / `monthCode` |
  | 每项 `used_ratio` | `usedRatio`（字符串或数字都接受） |
  | 每项 `reset_time` | `resetAt`（空串则省略） |
  | `boosterWallet.balance.amountLeft` | `balanceCents`（定点 ÷1e6 后四舍五入为分；>0 且 <1 分记 1 分） |
  | `boosterWallet.balance.amount` | `totalCents` |
  | `boosterWallet.monthlyChargeLimit` / `monthlyUsed`（`priceInCents`+`currency`） | `monthlyChargeLimitCents` / `monthlyUsedCents`、`monthlyChargeLimitEnabled` |
  | `boosterWallet.balance.type` 必须为 `'BOOSTER'`，否则 `extraUsage` 为 `null` | — |

  上游 401/404 的错误提示文案也写死在代码里（`:235-243`：401 提示检查 API key / `/login`，404 提示 “Usage endpoint not available. Try Kimi For Coding.”）。

- 相关同族端点（均需 token）：`GET /api/v1/oauth/login`、`POST /api/v1/oauth/logout`、`GET /api/v1/oauth/userinfo`（`packages/kap-server/src/routes/oauth.ts:163-176`）、`GET /api/v1/oauth/region`。**`GET /api/v1/auth` 不在这个文件里**，它在 `packages/kap-server/src/routes/auth.ts:18-33`（`registerAuthRoute`，`path:'/auth'`，返回 `authSummarySchema`）。

### 7.3 与采集直接相关的另外两个端点（顺带记录）

- `GET /api/v1/sessions/{session_id}/status`：返回 `{ busy, model?, thinking_level, permission, plan_mode, swarm_mode, context_tokens, max_context_tokens?, context_usage? }`，是冷会话会被**恢复**的读操作（`server-api.md:802-810`）。
- `GET /api/v1/sessions/{session_id}/transcript`：按 agent 组织、按轮次分页，`agent_id` 必填，返回 `{ agent_id, items, has_more, tasks, interactions, attachments, todos, meta, agents, pending_interactions, seq? }`（`:939-954`）。`usage` 不在其中——用量只能走 snapshot 或直接读 wire。

---

## 8. 旧 Python 版 `kimi-cli`（已归档）

读取 `main` = `9ab1286`。任务要求“找不到就明确未找到”，本节所有结论均来自实读代码。

### 8.1 数据根与目录布局

- 数据根默认 `~/.kimi`，可用 **`KIMI_SHARE_DIR`** 覆盖（不是 `KIMI_CODE_HOME`）：`src/kimi_cli/share.py:6-14`（`get_share_dir()`：先用环境变量，否则 `Path.home()/'.kimi'`）。
- 会话根 = `<share>/sessions/<bucket>/`，bucket 名 = `md5(work_dir路径)` 的十六进制；**非 local KAOS 时**前缀为 `<kaos>_`：`src/kimi_cli/metadata.py:33-40`

  ```python
  path_md5 = md5(self.path.encode("utf-8")).hexdigest()
  dir_basename = path_md5 if self.kaos == local_kaos.name else f"{self.kaos}_{path_md5}"
  session_dir = get_share_dir() / "sessions" / dir_basename
  ```

  与新版 `wd_<slug>_<sha256前12>` **完全不同**。
- 工作目录清单在 `<share>/kimi.json`（`metadata.py:17-18`），含 `work_dirs: [{path, kaos, last_session_id}]`（`:21-31`）——**旧版没有 `session_index.jsonl`**，会话列表靠 `readdir(sessions_dir)` 扫目录（`session.py:238-275`）。
- 每个会话目录：`<share>/sessions/<bucket>/<session_uuid>/`，内含：

  | 文件/目录 | 出处 |
  | --- | --- |
  | `context.jsonl`（消息历史，**权威**） | `session.py:146-151`（`session_dir / "context.jsonl"`） |
  | `wire.jsonl`（wire 事件日志） | `session.py:175`（`WireFile(path=session_dir / "wire.jsonl")`） |
  | `state.json`（运行态） | `session_state.py:12`（`STATE_FILE_NAME = "state.json"`） |
  | `metadata.json`（历史遗留；会被合并读，合并成功或确认无需合并后删除，不可读时保留待重试） | `session_state.py:47`（文件名常量）、`:50-96`（`_migrate_legacy_metadata` 合并逻辑）、`:114-125`（写回与 `unlink`） |
  | `subagents/<agent_id>/` | `session.py:55-60`（`subagents_dir`） |

- 更老布局存在两种，代码里都做兼容：扁平 `<bucket>/<uuid>.jsonl`（`session.py:309-319` 会把它 rename 成 `<uuid>/context.jsonl`，注释 `:315-318` “Migrated session context file from {old} to {new}”），以及只有 `context.jsonl` 没有 `state.json` 的目录。`migration-legacy` 的读取器同时支持 `<uuid>/context.jsonl` 与扁平 `<uuid>.jsonl`（`packages/migration-legacy/src/sessions/source.ts:19-58`）。
- `subagents/<agent_id>/` 内同样有 `context.jsonl` 等（迁移器就是按 `<srcSession>/subagents/<id>/context.jsonl` 读的，`subagents.ts:82-84`），子 Agent 元数据在 `subagents/store.py`（`_AgentInstanceRecordPayload` 含 `agent_id/subagent_type/status/description/created_at/updated_at/last_task_id/launch_spec`，状态枚举见 `store.py:31-34`）。
- 旧版另有 `<share>/user-history/<uuid>.jsonl`（golden 夹具 `packages/migration-legacy/test/fixtures/golden/.kimi/user-history/c1c0997494d4df46ac89f99f813c077c.jsonl`），迁移器把它映射到 `<targetHome>/user-history/`（`migration-legacy/src/paths.ts:3-5, 17-18`）。

### 8.2 wire.jsonl 格式（旧版）

`src/kimi_cli/wire/file.py`：

- 首行 metadata：`{ type: 'metadata', protocol_version: str }`（`file.py:17-31`，`WireFileMetadata`）。
- 之后每行：`{ timestamp: float, message: WireMessageEnvelope }`（`file.py:33-43`），**payload 嵌在 `message` 下**——与新版“payload 展开在顶层 + `time`”**结构不同**。注意旧版用 `timestamp`（秒，浮点），新版用 `time`（毫秒）。
- 实际样例（仓库夹具，protocol_version 1.10）：

  ```json
  {"type": "metadata", "protocol_version": "1.10"}
  {"timestamp": 1778563951.827712, "message": {"type": "StatusUpdate", "payload": {"context_usage": null, "context_tokens": null, "max_context_tokens": null, "token_usage": null, "message_id": "chatcmpl-fixture-recent-0001", "plan_mode": false, "mcp_status": null}}}
  ```

  （`packages/migration-legacy/test/fixtures/recent-protocol-1.10/wire.jsonl` 第 1、7 行）
- 协议版本常量：`src/kimi_cli/wire/protocol.py` 中 `WIRE_PROTOCOL_VERSION = "1.10"`、`WIRE_PROTOCOL_LEGACY_VERSION = "1.1"`。

### 8.3 usage 字段名

`src/kimi_cli/wire/types.py:175-192`（`StatusUpdate`）：

```python
class StatusUpdate(BaseModel):
    context_usage: float | None = None    # 百分比
    context_tokens: int | None = None     # 当前上下文的 token 数
    max_context_tokens: int | None = None
    token_usage: TokenUsage | None = None # “The token usage statistics of the current step.”
    message_id: str | None = None
    plan_mode: bool | None = None
    mcp_status: MCPStatusSnapshot | None = None
```

`TokenUsage`（`packages/kosong/src/kosong/chat_provider/__init__.py:99-114`）：

```python
class TokenUsage(BaseModel):
    input_other: int          # 不含 input_cache_read / input_cache_creation 的输入
    output: int               # 输出总量
    input_cache_read: int = 0
    input_cache_creation: int = 0
    # 派生：total = input + output；input = input_other + input_cache_read + input_cache_creation
```

**字段名与新版一一对应**（snake_case ↔ camelCase）：`input_other↔inputOther`、`output↔output`、`input_cache_read↔inputCacheRead`、`input_cache_creation↔inputCacheCreation`。含义也一致（`input_other` 是**不含缓存**的输入）。

### 8.4 session scope 是否累计

**旧版没有任何“会话级累计用量”记录。** 两处证据：

1. `StatusUpdate.token_usage` 的语义就是**当前 step**（`wire/types.py:188` 的字段注释 “The token usage statistics of the current step.”），发送点在每个 LLM step 之后：`src/kimi_cli/soul/kimisoul.py:1261-1273`（`usage = result.usage`；`StatusUpdate(token_usage=usage, ...)`；若 usage 非空则先 `await self._context.update_token_count(usage.input)` 再附带 context 三项，最后 `wire_send(status_update)`）。
2. `context.jsonl` 里的 `_usage` 行存的是**绝对值**而非累加：`src/kimi_cli/soul/context.py:242-248`

   ```python
   async def update_token_count(self, token_count: int):
       self._token_count = token_count
       ...
       await f.write(json.dumps({"role": "_usage", "token_count": token_count}) + "\n")
   ```

   重放时也是 `self._token_count = token_count`（`context.py:304-314`）——是赋值，不是累加。而 `usage.input` 就是本次请求的输入总量（`chat_provider/__init__.py:110-113`）。

**要得到会话总输入/输出，必须由读取方对 `StatusUpdate.token_usage` 逐条求和**——旧版自己的可视化就是这么做的：`src/kimi_cli/vis/api/statistics.py:134-144`（命中 `StatusUpdate` 时 `session_input_tokens += input_other + input_cache_read + input_cache_creation`，`session_output_tokens += output`）。

所以新旧差异很关键：**新版把“累计快照”写成了运行时状态（`agent.status.updated.total`，不落盘），历史真相在逐条 `usage.record`；旧版连快照都没有，只有逐 step 明细 + 一个绝对值的 `_usage` 上下文锚点。** 两者都**不能**把同一数字当重复事件——新版 total 重复广播同值，旧版 `_usage` 行是覆盖式绝对值。

### 8.5 旧版小节结论速查

| 问题 | 旧版 `kimi-cli` 答案 |
| --- | --- |
| 会话存哪 | `<KIMI_SHARE_DIR 或 ~/.kimi>/sessions/<md5(workdir)>/<session_uuid>/`（`share.py:6-14`、`metadata.py:33-40`、`session.py:146-151`） |
| 什么格式 | `context.jsonl`（消息，权威）+ `wire.jsonl`（JSONL，首行 metadata，其后 `{timestamp, message:{type,payload}}`）+ `state.json` + `subagents/<id>/`（`session.py:146-175`、`wire/file.py:17-43`） |
| usage 字段名 | `token_usage{input_other, output, input_cache_read, input_cache_creation}` 与 `context_usage/context_tokens/max_context_tokens`（`wire/types.py:175-192`、`chat_provider/__init__.py:99-114`） |
| session scope 是否累计 | **不累计**：`token_usage` 是逐 step；`context.jsonl` 的 `_usage.token_count` 是绝对值；聚合由读取方求和（`kimisoul.py:1261-1273`、`context.py:242-248`、`vis/api/statistics.py:134-144`） |
| 目录布局 | `<share>/kimi.json`（工作目录清单）+ `<share>/sessions/<bucket>/<uuid>/`；无 `session_index.jsonl`；兼容扁平的 `<bucket>/<uuid>.jsonl`（`metadata.py:17-31`、`session.py:238-319`） |
| 协议版本 | `1.10`（常量），历史遗留 `1.1`；夹具中出现 `1.10`（`wire/protocol.py`、`wire/file.py:14`） |

---

## 9. 证据链接

新版（固定提交 `c9f01f07388abc2cdca40c9c5267f58c65086710`，`https://raw.githubusercontent.com/MoonshotAI/kimi-code/c9f01f07388abc2cdca40c9c5267f58c65086710/`）：

- `packages/agent-core-v2/src/wire/record.ts`
- `packages/agent-core-v2/src/wire/migration/migration.ts`
- `packages/agent-core-v2/src/wire/wireService.ts`
- `packages/agent-core-v2/src/wire/human.ts`
- `packages/agent-core-v2/src/wire/tree/tree.ts`
- `packages/agent-core-v2/src/wire/tree/fork.ts`
- `packages/agent-core-v2/src/agent/usage/usageOps.ts`
- `packages/agent-core-v2/src/agent/usage/usageEvents.ts`
- `packages/agent-core-v2/src/agent/usage/usage.ts`
- `packages/agent-core-v2/src/agent/usage/cacheProbeService.ts`
- `packages/agent-core-v2/src/agent/llmRequester/llmRequester.ts`
- `packages/agent-core-v2/src/agent/llmRequester/llmRequesterService.ts`
- `packages/agent-core-v2/src/agent/fullCompaction/fullCompactionService.ts`
- `packages/agent-core-v2/src/agent/tokenCounting/tokenCountingOps.ts`
- `packages/agent-core-v2/src/session/usage/sessionUsage.ts`、`sessionUsageService.ts`、`usageAgentModel.ts`
- `packages/agent-core-v2/src/session/tokenCounting/tokenCountingAgentModel.ts`
- `packages/agent-core-v2/src/session/sessionTitle/sessionTitleService.ts`
- `packages/agent-core-v2/src/session/agentLifecycle/forked.ts`
- `packages/agent-core-v2/src/session/subagent/mirrorAgentRun.ts`
- `packages/agent-core-v2/src/human/llm/usage.ts`、`human/agent/context-usage.ts`
- `packages/agent-core-v2/src/features/usage/usageFeature.ts`
- `packages/agent-core-v2/src/features/goal/goalService.ts`
- `packages/agent-core-v2/src/app/event/event2.ts`
- `packages/agent-core-v2/src/app/bootstrap/bootstrapService.ts`
- `packages/agent-core-v2/src/_base/utils/workdir-slug.ts`
- `packages/agent-core-v2/src/workspace/sessionLifecycle/internal/addressing.ts`、`internal/forkTurnSlice.ts`、`sessionLifecycleService.ts`
- `packages/agent-core-v2/src/workspace/workspaceInstance/workspaceInstanceManagerService.ts`
- `packages/agent-core-v2/src/agent/replayBuilder/fold.ts`
- `packages/agent-core-v2/docs/wire-manifest.d.ts`
- `packages/migration-legacy/src/sessions/wire-writer.ts`、`turn-structure.ts`、`translator.ts`、`migrate-one.ts`、`state-writer.ts`、`source.ts`、`subagents.ts`、`workdir-bucket.ts`
- `packages/migration-legacy/src/paths.ts`、`session-index.ts`
- `packages/migration-legacy/test/fixtures/recent-protocol-1.10/wire.jsonl`、`context.jsonl`
- `packages/kap-server/src/routes/meta.ts`、`snapshot.ts`、`oauth.ts`
- `packages/kap-server/src/services/legacyStatus/legacyStatus.ts`
- `packages/kap-server/src/protocol/session.ts`、`protocol/rest-meta.ts`
- `packages/kap-server/src/middleware/auth.ts`
- `packages/oauth/src/managed-usage.ts`
- `apps/kimi-code/src/utils/paths.ts`
- `docs/zh/reference/server-api.md`（另有 `docs/en/reference/server-api.md`）
- `docs/zh/configuration/data-locations.md`、`docs/zh/configuration/env-vars.md`、`docs/zh/guides/sessions.md`
- 官方站点页面：`https://www.kimi.ai/help/kimi-code/cli-server-api`（HTTP 200，页面为 SPA，正文在 RSC 载荷中；与仓库 `server-api.md` 内容一致）

旧版（`MoonshotAI/kimi-cli` @ `9ab1286`，`https://raw.githubusercontent.com/MoonshotAI/kimi-cli/9ab1286/`）：

- `src/kimi_cli/share.py`
- `src/kimi_cli/metadata.py`
- `src/kimi_cli/session.py`
- `src/kimi_cli/session_state.py`
- `src/kimi_cli/soul/context.py`
- `src/kimi_cli/soul/kimisoul.py`
- `src/kimi_cli/wire/file.py`、`wire/protocol.py`、`wire/types.py`
- `src/kimi_cli/subagents/store.py`
- `src/kimi_cli/vis/api/statistics.py`
- `packages/kosong/src/kosong/chat_provider/__init__.py`
- 仓库元数据：`https://api.github.com/repos/MoonshotAI/kimi-cli`（`archived: true`, `default_branch: main`, `pushed_at 2026-09-22`）

---

## 10. 仍未证实事项

以下未在本次核查中证实，**不得**据此下结论：

1. **文档与运行时的实际漂移**：`/api/v1/meta`、`snapshot`、`oauth/usage` 的字段说明均来自同提交的文档 + 同提交的 zod schema 代码，二者一致；但文档自称“以在线 `/openapi.json` 为准”，本次**未启动服务、未抓取真实 `/openapi.json` / `/asyncapi.json`**，因此无法验证运行时实际 schema 是否与提交中的 schema 完全一致（尤其是 `experimental_flags` / `features` 的实际取值集合）。
2. **`snapshot` 响应中的 `subagents` / `pending_approvals` / `pending_questions` 的逐字段形状**：本文只确认了顶层键与 `usage` 映射（`snapshot.ts:169-180`），未逐字段核对 `rest-snapshot.ts`。采集若要依赖这些字段，需另读 `packages/kap-server/src/protocol/rest-snapshot.ts`。
3. **上游 `/usages` 服务端的真实响应**：`managed-usage.ts` 的解析是**客户端实现**，本次未对 `https://api.kimi.com/coding/v1/usages` 发起真实请求（无凭据，且任务约束禁止发送测试补全/余额查询）。因此“上游一定返回 `usages.limit_5h` 等字段”**未证实**，仅有客户端解析代码作为依据。
4. **`usageScope` 是否只由 `full_compaction` 触发**：结论基于固定提交的静态检索（唯一 `type:'operation'` 使用点）。若存在运行时动态构造 `source` 的路径（如插件/外部宿主传入），静态检索无法穷尽；此外未来版本新增 `operation` 调用方会改变结论。
5. **`agent.status.updated` 是否另有持久化副本**：本文确认它非 durable、不在 64 类型清单；但未逐个检查 `sessionIndex`/`store`/`minidb` 等旁路存储是否会缓存该事件（未读 `packages/minidb`、`sessionIndexProjector` 全量）。若采集只依赖 wire.jsonl，不受影响。
6. **`usage.record` 在 resume 时是否 100% 重放进 `byModel`**：本文依据 `usageFeature.ts:13` 注册 agent model + `usageAgentModel.ts:89` 的 `events:[UsageRecord]` + `eventDispatcherService.ts` 的 model 折叠机制推断，**未实际运行 resume 测试**验证“total 在 resume 后等于历史之和”。
7. **旧版 `kimi-cli` 是否还有其它写用量的路径**（如 web API 侧、`vis/api/sessions.py` 的自算逻辑、`notifications/wire.py`）：本文只核对了 `soul/kimisoul.py` 的写入点与 `vis/api/statistics.py` 的求和点，未穷尽 `src/kimi_cli` 全目录（991 个 blob）。
8. **Windows 实际行为未经双平台验证**：Windows 结论来自源码（`workdir-slug.ts:18,25-31`、`data-locations.md:11,46-47`）与 `workdir-bucket.ts:14-18` 的注释，**未在 Windows 上运行**。按项目约束，不得仅凭 macOS 侧阅读宣布 Windows 可用。
9. **`session_index.jsonl` 中 `sessionDir` 是绝对路径还是相对路径**：`sessionLifecycleService.ts:326` 用 `sessionDirOf(bootstrap.homeDir, ...)`（若 `homeDir` 绝对则为绝对路径），但仓库测试夹具里出现了 `'sessions/a/s1'` 这样的相对值（`packages/agent-core-v2/test/app/workspace/workspaceService.test.ts:519-521`）。两种形态都可能存在，采集端应**两者都容错**；本文未确定生产环境下的实际形态。
10. **旧版 `context.jsonl` 的 `_checkpoint` / `_system_prompt` / `_usage` 行的完整枚举**：本文只确认了出现在夹具与 `context.py` 中的三种；未穷尽旧版所有 `role.startswith('_')` 的行类型。
