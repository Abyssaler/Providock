# 旧版 Python kimi-cli wire.jsonl 合成夹具（协议 1.10 族）

来源：全部为本仓库手工编写的**合成数据**，不含任何真实用户日志、密钥或正文。
语义依据：docs/research/evidence/a08-kimi-official.md 第 8 节与「补充核实（2026-10-11）」小节
（`MoonshotAI/kimi-cli` @ `9ab1286`，已归档）。
预期值由人工按官方语义独立计算（本 README 给出算术过程），不是 parser 输出反推。

真实安装中的路径（仅供参考，夹具只提供文件字节）：

```
<KIMI_SHARE_DIR 或 ~/.kimi>/sessions/<md5(workdir)>/<session_uuid>/
├── context.jsonl
├── wire.jsonl                       ← 本目录 用例 input/wire.jsonl
├── state.json
└── subagents/<agent_id>/wire.jsonl   ← subagent-own-wire 用例的对应位置
```

## 通用映射（官方语义）

- 每行 `{timestamp: <浮点秒>, message: {type, payload}}`；首行是 `metadata`。
- 用量只出现在 `StatusUpdate.token_usage`，**每条是该 step 的绝对值**（不是累计），
  在每次成功的 LLM step 之后发一条 → granularity=request、request_count=1，
  会话总量由读取方求和（旧版 `vis/api/statistics.py` 就是这么求和的）。
- 映射：`input_other`→fresh、`input_cache_read`→read、`input_cache_creation`→write、
  `output`→output；input_total = 三者之和，total = input_total + output。
- `token_usage: null` 的 StatusUpdate（计划模式/上下文测量等）不产生事件；
  四项全零的 token_usage 也不产生零消耗事件（诊断计数）。
- `context_usage`/`context_tokens`/`max_context_tokens` 是上下文测量，不是消费。
- 子 Agent：同一消耗在「子 Agent 自有 wire.jsonl」和「父 wire 的 SubagentEvent 镜像」
  各有一份物理观察，必须二选一计入。默认以自有 wire 为准
  （`count_mirrored_subagent_usage=false`、子 Agent 文件 `mirrored_in_parent=true`）。
- fork/undo：`/fork` 复制整份 wire、`/undo` 复制到目标轮次为止的前缀，**wire 内无标记**；
  继承前缀的边界只能由调用方跨文件比对后给出（`fork: inherited_before_line:<N>`），
  给不出边界就全部保留并记诊断。

## 用例

### basic-multi-step/  （主 Agent，常规多 step）

2 轮 3 个成功 step，夹杂 TurnBegin/StepBegin/ContentPart/TurnEnd 与 1 条
`token_usage: null` 的纯状态更新（计划模式类，不算消费）。

- step1（chatcmpl-1）：fresh 1000、read 400、write 0 → input 1400；out 100 → total 1500
- step2（chatcmpl-2）：fresh 500、read 2000、write 100 → input 2600；out 20 → total 2620
- step3（chatcmpl-3）：fresh 0、read 0、write 0 → input 0；out 40 → total 40
- 合计：fresh 1000+500+0 = **1500**，read 400+2000+0 = **2400**，
  write 0+100+0 = **100**，input_total 1400+2600+0 = **4000**，
  output 100+20+40 = **160**，total 1500+2620+40 = **4160**，请求数 **3**。
- 诊断：`status_without_token_usage:1`（那条 null 状态更新）。
- 消费边界：全部（文件以换行结尾）。

### subagent-own-wire/  （子 Agent 自有文件；消耗已由父镜像计入）

`subagents/a1b2c3d4/wire.jsonl` 的形态，配置 `mirrored_in_parent=true`。
内含本 Agent 的 1 条用量记录（fresh 200/out 30）与 1 条 `token_usage: null`，
外加 1 条指向更深一层子 Agent `a9f9f9f9` 的 SubagentEvent 镜像（fresh 50/out 5）。

- 预期 **0 事件**：本文件的消耗（200/30）已由父 wire 的 SubagentEvent 镜像计入，
  镜像里的更深一层（50/5）也由父 wire 的嵌套镜像计入，本文件不重复。
- 诊断：`subagent_mirrored_in_parent:1`、`mirrored_subagent_records_skipped:1`、
  `status_without_token_usage:1`。

### parent-subagent-mirror/  （父 wire；从镜像恢复子 Agent 用量）

主 Agent wire，含 1 条自己的用量记录 + 2 条 SubagentEvent（第 2 条嵌套一层，
外层 agent_id `a1b2c3d4`、内层 `a9f9f9f9`）。配置 `count_mirrored_subagent_usage=true`，
用于子 Agent 自有文件不可达、只能从父镜像恢复的场景。

- main（chatcmpl-main-1）：fresh 300、out 30 → total 330
- a1b2c3d4（chatcmpl-s1）：fresh 200、out 30 → total 230
- a9f9f9f9（chatcmpl-g1）：fresh 50、out 5 → total 55
- 合计：fresh 300+200+50 = **550**，read/write 0，input_total **550**，
  output 30+30+5 = **65**，total 330+230+55 = **615**，请求数 **3**。
- actor 归属：main 1 条 / a1b2c3d4 1 条 / a9f9f9f9 1 条（嵌套取最内层 agent_id）。
- 诊断：`mirrored_subagent_records_counted:2`。

### context-measurement-only/  （迁移后仅上下文测量）

只有 `token_usage: null` 但带 `context_tokens`/`context_usage` 的状态更新（8192、0），
对应迁移产物只写 `token_counting.*` 的情形。

- 预期 **0 事件**、兼容 ok、各 Token 合计为 null：上下文测量不是消费，不猜测、不补零。
- 诊断：`status_without_token_usage:2`。

### fork-prefix-copy/  （/fork 或 /undo 的完整前缀复制）

第 2、3 行是从复制源继承来的记录（10/out 5、20/out 5），第 4 行起才是本会话新记录。
配置 `fork: inherited_before_line:4`（第 4 行起始字节偏移 = 695，即继承前缀 = 前 3 行）。

- 继承：fresh 10+20 = 30、out 5+5 = 10 → 跳过（复制源已计）
- 新记录（chatcmpl-new-1）：fresh 7、out 2 → input 7、total 9
- 预期合计：fresh **7**、read 0、write 0、input_total **7**、output **2**、total **9**、请求数 **1**。
- 诊断：`fork_inherited_skipped:2`。

### legacy-1.1-malformed-truncated/  （历史遗留 1.1 + 畸形行 + 崩溃尾行）

`protocol_version: "1.1"`（旧版 `WIRE_PROTOCOL_LEGACY_VERSION`，同族可解析）。
含 1 行非法 JSON（`{not json at all`）与 1 行未写完的尾行（无结尾换行）。

- 完整可用记录：fresh 100/out 30，fresh 5/out 1
- 预期：**2 事件**，fresh **105**、read 0、write 0、input_total **105**、
  output **31**、total **136**、请求数 **2**；畸形行只记诊断
  （`malformed_line_at_offset:…`），不崩溃。
- 消费边界：`before_trailing_partial`（最后一个换行之后不消费），因此尾行里的
  999/99 **不计入**。

### unknown-protocol/  （未知版本，不猜测）

`protocol_version: "1.11"`（超出已知 1.10 族）。

- 预期：**0 事件**、compatibility = `unsupported_protocol:1.11`、
  只消费首行（`first_line_only`）、诊断含 `wire_compatibility`。
- 说明：文件存在但没有 metadata 首行时同样不解析（`MissingMetadata`），
  不沿用上游「缺 header 就当 1.1」的降级默认（见解析器模块注释与证据文档）。

## 夹具与断言约定

`expected.json` 字段：

| 字段 | 含义 |
| --- | --- |
| config.agent_id | 目录给出的 Agent 身份（main / 子 Agent id） |
| config.mirrored_in_parent | 本文件消耗是否已由父 wire 镜像计入（子 Agent 文件用 true） |
| config.count_mirrored_subagent_usage | 是否计入本文件内的 SubagentEvent 镜像 |
| config.fork | `none` / `inherited_before_line:<N>`（1 起，第 N 行为首个非继承行）/ `unresolved` / `source_unavailable` |
| compatibility | `ok` / `unsupported_protocol:<v>` / `missing_metadata` |
| events | 事件条数 |
| totals | 六项 Token 合计与 request_count；`null` 表示该字段全部未知 |
| actors | 可选；每个 actor 的事件条数与 total 之和（须恰好覆盖全部事件） |
| completeness | 可选；所有事件的 completeness |
| consumed | `all` / `first_line_only` / `before_trailing_partial` |
| diagnostics_include | 诊断里必须出现的子串 |
