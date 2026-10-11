# 证据 a01–a04：cc-switch 四个 Agent 会话用量导入器的语义事实

- 核查日期：2026-10-11
- 固定提交：`df40d53c36b604e044b4d5b9091aeb961aa56990`（cc-switch）
- 核查范围：A01 Claude Code、A02 Codex、A03 Gemini CLI、A04 Grok Build（Grok CLI）四个「会话日志 → 用量」导入器，以及它们引用的共享模块（去重、路径、定价、输入语义）。
- 核查方法：用 GitHub Tree API（`.../git/trees/df40d53c...?recursive=1`）枚举 `src-tauri/src` 全部路径（1668 项，未截断），再经 `raw.githubusercontent.com` 按固定提交拉取源码逐一细读。四个目标文件均存在，**无需按 404 回退定位，未发现路径差异**。**未运行 cc-switch、未构造真实会话日志、未读取用户真实日志**；本文只记录「读到的代码行为」，不是实机兼容证明。
- 行号引用格式：`文件#Lx-Ly`，证据链接给出该固定提交的 raw URL + 行号范围。
- 贯穿性约束（本项目 `AGENTS.md`）：未知不等于零；累计快照不能直接求和；相同数字不能自动视为重复事件。本文第 8 点集中列出与之相关的上游行为。

## 0. 四个导入器的共同骨架（先读这一节）

四个导入器都是 `services/session_usage.rs::sync_all_unlocked()` 里串行调用的一步，结果合并进同一个 `SessionSyncResult{ imported, skipped, files_scanned, suspected_duplicates, deferred_files, errors }`。
[`services/session_usage.rs#L137-L173`](https://raw.githubusercontent.com/farion1231/cc-switch/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/session_usage.rs)（下同文件，行号见各节）

共同点：

- 落库目标同一张表 `proxy_request_logs`，靠 `data_source` 区分来源：`session_log`（Claude）/`codex_session`/`gemini_session`/`grok_session`；`provider_id` 用占位符 `_session`/`_codex_session`/`_gemini_session`/`_grok_session`（`usage_stats.rs#L300-L315`）。
- 增量/重复控制都记在 `session_log_sync` 表（`file_path, last_modified, last_line_offset, last_synced_at, last_byte_offset, last_tail_fingerprint`），由 `services/session_usage.rs#L99-L122` 一次性预取。
- 定价都走 `usage_stats::find_model_pricing(conn, model)`（先精确、再前缀模糊匹配，`usage_stats.rs#L2473-L2506`）。
- 去重共用 `usage_stats::DedupKey` / `should_skip_session_insert` / `has_matching_proxy_usage_log`（`usage_stats.rs#L594-L694`）。
- 成本计算共用 `proxy/usage/calculator.rs`，按 app 选择「input 是否含缓存」的语义（`calculator.rs#L52-L120`）。
- 四个导入器**都不读取价格以外的上游费用字段**，只有 A04 Grok 有自报成本（`costUsdTicks`）。

---

## A01 Claude Code（`services/session_usage.rs`）

### 1. 默认目录与各 OS 路径规则（含环境变量覆盖）

- 根目录：`get_claude_config_dir().join("projects")`。
  `session_usage.rs#L367-L378`（`let projects_dir = get_claude_config_dir().join("projects");`，不存在则直接返回 0 结果）。
- `get_claude_config_dir()` = 应用设置里的 `claude_config_dir` 覆盖值，否则 `get_home_dir().join(".claude")`。
  `config.rs#L91-L97`；覆盖值来源 `settings.rs#L957-L963`（`settings.claude_config_dir`），解析 `settings.rs#L778-L801`（支持 `~` / `~/x` / `~\x` 展开，其余按字面路径）。
- `get_home_dir()` = `CC_SWITCH_TEST_HOME`（注释明确「仅用于测试/调试」）→ 否则 `dirs::home_dir()`，失败回退 `.`（当前目录，并打 warning）。
  `config.rs#L22-L31`
- **OS 规则**（代码注释，非本机验证）：Windows 上 `dirs::home_dir()` 走 `SHGetKnownFolderPath(FOLDERID_Profile)` 返回真实用户目录；**刻意不使用 `HOME` 环境变量**，因为可能被 Git/Cygwin/MSYS 注入且不等于用户目录。
  `config.rs#L9-L21`
- **没有 `CLAUDE_CONFIG_DIR` 之类的环境变量支持**：在 `config.rs`、`settings.rs` 全文检索未见读取该变量；唯一以环境变量影响路径的是测试用的 `CC_SWITCH_TEST_HOME`。
- 采集的目录布局（固定深度、不递归，避免死循环）：
  - `projects/<项目目录>/*.jsonl`（主会话）
  - `projects/<项目目录>/<SESSION_ID>/subagents/*.jsonl`（Task/Agent 子 agent）
  - `projects/<项目目录>/<SESSION_ID>/subagents/workflows/wf_*/*.jsonl`（Workflow 子 agent，2019 加的一层）
  `session_usage.rs#L441-L451`（文档）、`#L452-L498`（实现）、`#L500-L510`（`push_jsonl_children`）。注释说明 `journal.jsonl` 也会被收集，但因为它不含 `type=="assistant"` 行，解析时天然产出 0 条，因此未按文件名过滤。

### 2. 文件格式与记录类型

- 格式：JSONL（每行一个 JSON 对象），**无压缩形态处理**（全文未见 `.zst`/`.gz` 分支）。
- 只对 `type == "assistant"` 的行做用量提取；其余行只用于两件事：记录对话链（`uuid`/`parentUuid`/`timestamp`/`type`）以便估算耗时，以及取一次 `sessionId`。
  `session_usage.rs#L723-L736`（`sessionId`、`record_chain_node`、`type=="assistant"` 过滤）、`#L245-L260`（`record_chain_node`）、`#L309-L314`（`assistant_message_id`）。
- 一条 assistant 行内被读取的字段：`message.id`、`message.model`、`message.stop_reason`、`message.usage.*`、行级 `timestamp`、`apiBlockIndex`（仅用于判断是否回复第一块）。
  `session_usage.rs#L738-L788`、`#L338-L345`。

### 3. usage 字段名与包含关系（逐字段）

`session_usage.rs#L755-L788`：

| 原始字段（`message.usage.*`） | 内部字段 | 落库列 |
| --- | --- | --- |
| `input_tokens` | `input_tokens` | `input_tokens` |
| `output_tokens` | `output_tokens` | `output_tokens` |
| `cache_read_input_tokens` | `cache_read_tokens` | `cache_read_tokens` |
| `cache_creation_input_tokens` | `cache_creation_tokens` | `cache_creation_tokens` |
| `cache_creation.ephemeral_1h_input_tokens` | `cache_creation_1h_tokens` | **无对应列**（只参与计价，见第 7 点） |

`cache_creation_1h_tokens` 的提取在 `proxy/usage/parser.rs#L43-L48`（`pointer("/cache_creation/ephemeral_1h_input_tokens")`），它是 `cache_creation_tokens` 的**子集**（计价侧用 `min` 钳制：`calculator.rs#L100-L102`）。

- **input 不含 cached**：Anthropic 口径，`input_tokens` 是 fresh（缓存读/写单列）。证据：`calculator.rs#L39-L50`（`calculate()` 注释「Claude/Anthropic 的 input_tokens 已经不包含 cache_read_tokens」，并传 `input_includes_cache_read=false`）、`services/sql_helpers.rs#L23`（`CACHE_INCLUSIVE_APP_TYPES = ["codex","gemini","grokbuild"]`，**不含 claude**）。
- **output 不含 thoughts/reasoning**：该解析器根本没有读取任何 thinking/reasoning 字段，output 与其它维度之间没有任何相加。
- 落库列与常数（`session_usage.rs#L1182-L1215`）：`input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens` 四列 + `input_cost_usd/output_cost_usd/cache_read_cost_usd/cache_creation_cost_usd/total_cost_usd` + `latency_ms`、`provider_type='session_log'`、`is_streaming=1`、`cost_multiplier='1.0'`、`data_source='session_log'`、`status_code=200`。
  **注意：该 INSERT 不写 `input_token_semantics` 列**，落库为列默认值 `0`＝`LEGACY`（`database/schema.rs#L206`）。

### 4. 计数器类型：单条消息快照（非累计、非差分）

- 每条 assistant 行携带**该行写入时刻的快照**。同一条回复（同一 `message.id`）可能被逐块写成多行，每行各带一份 usage。
- 归并规则：先按 `message.id` 收进 map，取舍为「优先有 `stop_reason` 的行；两者有无 `stop_reason` 一致时取 `output_tokens` 更大者」。
  `session_usage.rs#L790-L809`
- 入库门槛：`input/output/cache_read/cache_creation` 任一 `> 0` 即导入；全 0 跳过。
  `session_usage.rs#L822-L841`（注释说明旧逻辑要求 `stop_reason` 非空且 `output>0`，会丢掉 Workflow/子 agent 只写了 `message_start` 快照的请求，实测系统性低估约 4.1%，其中 92% 集中在 workflow/subagent）。
- **没有差分、没有累计基线**。后续补全走另一条路径：已入库行在输出 token 变大或耗时变长时被 `UPDATE`（`session_usage.rs#L1060-L1131`：`output_grew` 或 `latency_ms` 增大才更新，否则 `Skipped`）。
- 不计时：会话日志没有请求计时，`latency_ms` 由「回复第一块的父行时间戳 → 最后一块时间戳」相减估算，合理区间 `[100ms, 1h]`，估不出写 0。
  `session_usage.rs#L205-L220`、`#L349-L364`、`#L849-L857`。

### 5. 事件身份与去重键

- **request_id = `session:{message.id}`**，即 `parser.rs#L51` 的 `SESSION_REQUEST_ID_PREFIX` 直接拼 message id；**不含 session_id、不含文件路径**。
  `session_usage.rs#L843-L847`；测试断言主键形如 `session:msg_partial`（`#L1624-L1631`），另一处断言 `session:msg_1`（`#L1425`）。
  注释说明 Claude / Claude Desktop 共用该无作用域命名空间，以便与网关侧同一 message id 收敛（`parser.rs#L53-L61`）。
- 库内幂等：主键 + `INSERT OR IGNORE`（`session_usage.rs#L1180-L1222`）；同 request_id 再次出现时走「补全」而不是新插入。
- **跨源指纹去重（关键）**：入库前先查「时间窗内是否存在 token 数完全相同的代理行」：
  `DedupKey{ app_type:"claude", model, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, created_at }`（`session_usage.rs#L1159-L1167`）交给 `usage_stats::has_matching_proxy_usage_log`（`usage_stats.rs#L667-L694`）——条件为：`data_source='proxy'`、`status_code` 2xx、四元 token 数**完全相等**、model 相同（或任一侧为 `'unknown'`）、`created_at` 落在 `±SESSION_PROXY_DEDUP_WINDOW_SECONDS`＝±600 秒内（`usage_stats.rs#L317`）。命中则**跳过插入**（`session_usage.rs#L1173-L1175`）。
- 补全时反向清理：若补全后的用量恰好与某条代理行指纹相同，会把先入库的会话行**删除**（`session_usage.rs#L1095-L1104`）。
- 读侧还有一层同类过滤：`effective_usage_log_filter`（`usage_stats.rs#L399-L440`）在展示时把与代理行匹配的会话行藏起来。
- **分叉 / 复制 / 子 Agent**：没有分叉概念。子 agent 靠目录布局收集，其 assistant 行自带 `message.id`；若同一 `message.id` 出现在两个文件里，会因主键相同而只在库中留一行（合并，非双算）。
- 会话归属：`session_id` 取文件中第一次出现的 `sessionId`，仅作展示列，不参与身份。

### 6. 日志截断、尾行不完整、文件轮转

- 游标：`last_byte_offset`（主）+ `last_modified`（mtime 纳秒）+ `last_tail_fingerprint`（游标边界前最多 4096 字节的 SHA-256 前 4 字节，域标签 `claude-session-tail-v1`）。
  `session_usage.rs#L531-L543`、`#L900-L931`、`#L948-L959`。
- mtime 门：`file_modified <= last_modified` 直接跳过（`#L604-L606`）。
- 旧行号游标迁移：`last_byte_offset` 为 NULL 时按 `last_line_offset` 跳过前 L 行、换算为字节位置，**不回退全量重读**（`#L655-L688`，理由见 `#L568-L585`）。
- 尾行不完整：不推进字节游标，但**若该尾段能解析成 JSON 仍会导入**（靠 request_id 去重防重复），下一轮补全后重新确认。
  `#L707-L712`；测试 `test_complete_tail_without_newline_imports_but_holds_cursor`（`#L1561-L1596`）、`test_partial_line_completed_later_is_not_lost`（`#L1598-L1635`）。
- 解析中途 IO 错误：已提交的行照常入库，游标盖**旧 mtime**（让下轮 mtime 门放行续读），并把错误上报到 `SessionSyncResult.errors` 与 `deferred_files`（`#L690-L712`、`#L870-L889`）。
- **截断 / 重写（非追加变化）**：`offset` 越界（截断）或尾部指纹失配（同尺寸重写）→ 游标**钉到当前 EOF、不重放任何旧区间**，返回 `pinned_rewrite` 并经 `errors` 上报（`#L611-L651`）。设计理由写在函数文档里：`rollup_and_prune` 会汇总并删除 30 天前明细，`request_id` 去重对已剪条目失明，重放会把已剪条目二次累加进汇总（`#L571-L585`，另见 `#L93-L98`）。**取舍是有意丢行优于双算。**
- 文件轮转：Claude Code 正常路径是纯追加，代码假定如此（`#L582-L585`）；无轮转/归档处理。

### 7. 费用字段

- 日志里**没有上游自报费用**，全部为本地估算：`find_model_pricing(conn, model)`（支持精确 + 前缀模糊，`usage_stats.rs#L2473-L2506`）→ `CostCalculator::calculate(usage, pricing, 1)`（`session_usage.rs#L1035-L1058`）。无价时五项全部写 `"0"`（`#L1056`）。
- 金额列是 TEXT 十进制字符串：`input_cost_usd`、`output_cost_usd`、`cache_read_cost_usd`、`cache_creation_cost_usd`、`total_cost_usd`，单位 USD。
- Anthropic 分段缓存写入单独计价：`cache_creation_cost = (cache_creation − 1h) × 5m单价 + 1h × 1.6 × 5m单价`（`calculator.rs#L98-L107`）。测试用 `claude-opus-4-8`（400 × $6.25/M + 600 × $10/M = $0.0085）钉死该换算（`session_usage.rs#L1886-L1913`）。
- `cost_multiplier` 恒 `"1.0"`。

### 8. 可疑或与契约冲突的上游行为

1. **「相同数字＝重复」启发式**（与契约冲突）：A01 在入库前把「四元 token 数完全相同 + model 相同 + ±10 分钟内」的代理行当作同一次请求，直接跳过会话行（`session_usage.rs#L1173-L1175` → `usage_stats.rs#L667-L694`）。这是判重而非证据关联；两笔巧合相同用量的独立请求会被吞掉，同一请求的中间态则可能反向漏并。契约要求「相同数字不能自动视为重复事件」，此处不可照搬。
2. **`status_code` 硬写 200**（`session_usage.rs#L1206`，注释：「会话日志中的请求只要产生计费 token 即视为成功」）——中断/报错但已计费的请求也会被标成成功。
3. **未落 `input_token_semantics`**（默认 `LEGACY`），而同表 codex/grokbuild 路径显式写 `TOTAL`——语义靠读侧 `app_type` 白名单兜住，属跨来源不一致（`database/schema.rs#L206`、`sql_helpers.rs#L55-L66`）。
4. **`cache_creation_1h_tokens` 无落库列**：只在内存里参与计价（`parser.rs#L43-L48` + `calculator.rs#L100-L107`），行内无法还原 1h/5m 拆分（与 a09 文档同一处发现的同源问题）。
5. **`latency_ms` 是估算值，且 0 兼作「无计时」**（`#L204-L220`、`#L1204`）——数值 0 不等于「瞬时响应」，下游不可当真实耗时使用。
6. **全 0 行被跳过**（`#L835-L841`）——与「未知不等于零」方向一致（正向事实）。
7. **「output 是最大块快照」的取舍**：同 message.id 取 `output_tokens` 更大者（`#L790-L809`）。若上游出现「后一块反而更小」的异常写入，会保留更大值（可能高估），上游用 request_id 主键保证只落一行。

### A01 证据链接

- `https://raw.githubusercontent.com/farion1231/cc-switch/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/session_usage.rs`：`#L9-L31`（模块文档/依赖）、`#L99-L122`（游标预取）、`#L137-L173`（总调度）、`#L191-L220`（ParsedAssistantUsage / 耗时边界）、`#L245-L364`（对话链与计时）、`#L367-L390`（目录与入口）、`#L441-L510`（文件收集）、`#L531-L585`（指纹与增量语义文档）、`#L586-L661`（截断/重写检测）、`#L663-L712`（逐行读与尾行）、`#L723-L810`（字段解析与 message.id 归并）、`#L822-L889`（入库门槛、request_id、游标写回）、`#L900-L931`（核心写游标）、`#L948-L994`（共享 timestamp/游标）、`#L1035-L1131`（成本与补全）、`#L1138-L1223`（插入）、`#L1561-L1635`（尾行测试）、`#L1637-L1757`（截断/重写测试）、`#L1845-L1913`（无 stop_reason、1h 缓存测试）
- `.../src-tauri/src/config.rs`：`#L9-L31`（home dir）、`#L91-L97`（claude 目录）
- `.../src-tauri/src/settings.rs`：`#L778-L801`（`~` 展开）、`#L957-L963`（覆盖目录）
- `.../src-tauri/src/proxy/usage/parser.rs`：`#L43-L51`（1h 缓存 + request_id 前缀）、`#L53-L61`（命名空间注释）、`#L70-L115`（TokenUsage）
- `.../src-tauri/src/proxy/usage/calculator.rs`：`#L39-L50`、`#L72-L120`（计费与 1h 倍率）
- `.../src-tauri/src/services/sql_helpers.rs`：`#L23`、`#L30-L32`、`#L43-L67`（输入语义归一）
- `.../src-tauri/src/services/usage_stats.rs`：`#L317`（±600s 窗口）、`#L399-L440`（读侧过滤）、`#L594-L694`（DedupKey 与指纹去重）
- `.../src-tauri/src/database/schema.rs`：`#L206`（`input_token_semantics` 默认 0）

---

## A02 Codex（`services/session_usage_codex.rs`）

### 1. 默认目录与各 OS 路径规则（含环境变量覆盖）

- 根目录：`get_codex_config_dir()` = 应用设置 `codex_config_dir` 覆盖值，否则 `get_home_dir().join(".codex")`。
  `codex_config.rs#L270-L276`；覆盖值来源 `settings.rs#L965-L971`。
- 采集两个位置（`session_usage_codex.rs#L956-L981`）：
  - `<codex>/sessions/YYYY/MM/DD/*.jsonl`——递归，最大深度 3（`#L960-L963`、`#L996-L1011`）
  - `<codex>/archived_sessions/*.jsonl`——扁平目录（`#L965-L976`）
- 两种文件形态：`rollout-*.jsonl` 与压缩后的 `rollout-*.jsonl.zst`；`is_rollout_file` 只看扩展名，`reader_from_file` 对 `.zst` 透明解压（`codex_rollout_file.rs#L28-L38`、`#L69-L76`）。压缩由 Codex 的 `[features] local_thread_store_compression` / app-server `rollout/compress` 产生（7 天未动的 rollout，保留 mtime），继续对话时先解压回 `.jsonl`（`codex_rollout_file.rs#L1-L7`）。两形态并存时只留普通形态（`#L978-L981` → `codex_rollout_file.rs#L121-L133`）。
- 文件名规则：必须以 `rollout-` 开头、`.jsonl` 结尾，且文件名倒数 36 字符可解析为 UUID（`session_usage_codex.rs#L462-L470`）；另有 `rollout-…-<threadId>_<rolloutId>.jsonl` 双段名（`thread/revert` 替换 rollout），前段 UUID 才是逻辑线程（`#L611-L630`）。
- **未读取 `CODEX_HOME` 环境变量**：`get_codex_config_dir` 只认应用设置与 `~/.codex`；源码里出现 `CODEX_HOME` 仅在一句注释中（"兼容用户改过 CODEX_HOME 后遗留的 cursor"，`#L485-L486`）。OS 差异由 `get_home_dir()` 承担（同一 `config.rs#L9-L31`）。

### 2. 文件格式与记录类型

JSONL（可选 zstd 单帧压缩）。解析时只看三类行（先按行首 512 字节快速分类，`#L211`、`#L242-L270`）：

| 行类型 | 判据 | 取出什么 |
| --- | --- | --- |
| `session_meta`（仅首条生效） | `type=="session_meta"` | `payload.id`/`thread_id`/`threadId` → 逻辑线程 id；`payload.forked_from_id`、`payload.source.subagent.thread_spawn.parent_thread_id` → 父线程；行级 `timestamp` |
| `turn_context` | `type=="turn_context"` | `payload.model`（或 `payload.info.model`）→ 当前模型 |
| `event_msg` + `token_count` | `type=="event_msg"` 且 `payload.type=="token_count"` | `payload.info.total_token_usage` / `last_token_usage`、`payload.rate_limits.limit_id`（快照来源）、行级 `timestamp` |
| `token_usage_record`（新版才有） | `payload.type=="token_usage_record"` | 只用于**耗时估计**的结束时刻，不产生用量行 |

`session_usage_codex.rs#L1097-L1270`（主解析循环）、`#L1101-L1148`（session_meta 及一致性校验）、`#L1149-L1159`（turn_context）、`#L1160-L1268`（token_count）。

### 3. usage 字段名与包含关系（逐字段）

解析函数 `parse_signature_counters`（`#L657-L677`）、`parse_cumulative_tokens`（`#L860-L894`）：

| 原始字段 | 用途 |
| --- | --- |
| `input_tokens` | 用量（含缓存读写） |
| `cached_input_tokens`（回退 `cache_read_input_tokens`） | 用量＝缓存读 |
| `cache_write_input_tokens` | 用量＝缓存写 |
| `output_tokens` | 用量＝输出 |
| `reasoning_output_tokens` | **只进签名用于判重**，不落库、不参与 output、不单独计费 |
| `total_tokens` | **只进签名用于判重**，不落库 |

落库映射（`#L1827-L1864`）：`input_tokens=delta.input`、`output_tokens=delta.output`、`cache_read_tokens=delta.cached_input`、`cache_creation_tokens=delta.cache_write_input`，并**显式写 `input_token_semantics = TOTAL`**（`#L1863`，注释「input_tokens 含缓存读写」）。

- **input 含 cached read 与 cache write**：注释与钳制逻辑——「Cache reads and writes are disjoint parts of input_tokens」，先 `cached = min(cached_delta, input_delta)`，再 `cache_write = min(cache_write_delta, input_delta − cached)`（`#L1232-L1238`）。
- **output 不含 reasoning**：`DeltaTokens`（`#L43-L57`）只有 `input/cached_input/cache_write_input/output` 四个字段，`reasoning_output_tokens` 在签名里读入后未再使用（`#L670-L672`）。
- 计价侧一致：`calculate_for_app("codex", …)` 会从 input 中扣掉 cache_read + cache_creation（`calculator.rs#L56-L89`）。测试 `test_cache_write_tokens_are_billed_as_cache_creation` 用 `input=1000, cached=300, cache_write=600` 断言 fresh input = 100（`#L2680-L2747`）。

### 4. 计数器类型：累计计数（total_token_usage）＋精确逐请求值优先

- `total_token_usage` 是**会话累计**，跨模型与跨 rate-limit lane 都持续累加（`#L1037-L1040` 注释）。
- 每条 `token_count` 事件的用量取值优先级（`#L1202-L1238`）：
  1. **有 `last_token_usage` → 直接采用**（上游给的精确逐请求值），注释解释：不要改用累计相减，因为累计快照可能来自多条独立推进的 rate-limit lane（`#L1210-L1219`）。
  2. 否则 `compute_delta(high_water, total)`：逐字段 `saturating_sub`（`#L833-L850`）。
- 累计值的处理：`total_high_water` 逐字段取最大值、只增不减（`#L852-L857`）。
- **倒退/重置**：`saturating_sub` → 增量记 0，不产生负数（`#L841-L848`；测试 `test_delta_saturating_sub` `#L2350-L2369`）。有 `last_token_usage` 时重置仍能正确产生增量（测试 `test_full_snapshot_dedupe_allows_counter_reset` `#L2639-L2677`）。
- **重复快照**：`duplicate_snapshot = 有 total 快照 && (同 limit_id 来源的最后签名相同 || 与紧邻前一个签名相同)`，命中则本次 delta 全 0（`#L1193-L1209`）。规则注释：同源重复由该来源最新完整快照识别，跨源重复必须与紧邻前一次事件匹配——不能与其它来源的更旧快照比较，因为旧签名在计数重置后合法复现（`#L1041-L1046`）。测试：`#L2443-L2475`、`#L2478-L2560`、`#L2639-L2677`。
- `delta.is_zero()` 的事件**不产生行**（`event_index` 只在非零时自增，`#L1239-L1245`）。

### 5. 事件身份与去重键

- **request_id = `codex_session:thread-v1:{root_thread_id}:{event_index}`**。
  前缀常量 `#L47`；拼接处 `#L1672-L1673`。
  - `root_thread_id` = **文件名末尾 UUID**（`thread_id_from_filename`，`#L602-L609`；`#L1532` 传入）；缺失即 deferred（`#L1537-L1544`）。
  - `event_index` = **该文件内非零 delta 事件的顺序号**（1 起，`#L1239-L1245`）。注意它是位置键，不是上游 id。
- `session_id` 列写 root `session_meta` 的线程 id（`#L1656-L1659`，双段文件名时是前置 UUID）。
- 库内幂等：请求级去重走 `should_skip_session_insert`（request_id 已存在 或 指纹匹配代理行，`#L1778-L1780`），插入用 `INSERT OR IGNORE`（`#L1827-L1837`）。
- **分叉 / 子 Agent（本套最精细的部分）**：
  - 父线程来自 `forked_from_id` 或 `source.subagent.thread_spawn.parent_thread_id`（`#L632-L648`）；两者不一致、或父 id 等于自身、或不是合法 UUID → deferred（`#L1126-L1147`）。
  - 子 rollout 在 fork 前复制了父 rollout 的 token 事件，靠**签名子序列对齐**算出 `replay_prefix` 并跳过这些事件（`matching_replay_prefix` `#L1398-L1412`；应用 `#L1605`、`#L1634-L1648`）。父 rollout 找不到、或尚未写到 child fork 时刻 → deferred，等下一轮（`#L1379-L1396`、`#L1591-L1602`）。
  - 父文件的 token 时间线有进程内缓存（按 mtime+size+device/inode 或 Windows volume serial+file id 校验，`#L94-L126`、`#L1288-L1372`）。
  - 子 agent transcript 与父用**文件名线程 id** 区分（测试 `test_subagents_use_filename_thread_ids` `#L3413-L3461`）。
- **疑似重复只告警不拦**：`has_suspected_codex_session_duplicate`（`usage_stats.rs#L730-L769`）统计 `suspected_duplicates` 并打 warning，但**仍然插入**（`#L1781-L1789`）——方向是可能双算，靠 request_id 兜底。

### 6. 日志截断、尾行不完整、文件轮转

- 游标：`last_line_offset`（**消费的完整记录数**，主游标）+ `last_byte_offset`（仅用于变更比对，`#L418-L424` 注释：绝不用于 seek）。写回见 `#L1453-L1478`。
- **每轮解析都从文件头重读**（`#L418-L422` 注释：`observed_bytes` 只用于检测文件变化），这是设计选择，大文件每轮全解析。
- 跳过条件：`file_modified == last_modified && last_byte_offset == size`（`#L1501-L1503`）。Windows 可能保持 mtime 不变，因此 legacy 无字节游标的存量行会强制重扫一次（`#L1498-L1500`）。
- 尾行不完整：读取时若最后一段不以 `\n` 结尾**且不是合法 JSON**，则该行不计入 `line_offset`（下轮重试）；若完整 JSON 无换行，则正常消费并导入。
  `#L1060-L1071`；测试 `test_complete_final_record_without_newline_is_imported_once`（`#L2140-L2168`）、`test_partial_live_record_is_retried_after_append`（`#L2169-L2207`）、`test_unchanged_incomplete_tail_is_skipped_after_cursor_reload`（`#L2047-L2077`）。
- **压缩 / 归档导致路径变化**：按逻辑文件名（去掉 `.zst`）在 `session_log_sync` 里找**最远的**游标并继承（`get_codex_sync_state` `#L757-L784`）。注释明确：「🔴 这是压缩文件不被重复计费的唯一屏障：request_id 去重只覆盖明细保留的 30 天，而被压缩的文件至少 7 天没动、往往更老。」（`#L751-L756`）测试 `test_archived_log_inherits_cursor_and_only_imports_appended_usage`（`#L3464-L3530`）、`test_compressed_rollout_round_trip_never_double_counts`（`#L3560-L3606`）。
- **deferred 机制**：把 (mtime,size,原因) 记进进程内 `pending` 缓存；`Stable` 原因（缺 session_meta、文件名无 UUID、父信息不一致等）下轮仍 defer，`Retryable`（父 rollout 暂时读不到）会重试（`#L1414-L1446`、`#L1505-L1530`）。deferred 不推进游标（`#L1696` 只在最后一批提交游标）。
- 分批事务：每 1000 行一个事务，最后一批把游标推进与数据同事务提交（`#L1448-L1451`、`#L1660-L1709`）。
- **未见截断 / 重写检测**：全文检索 `truncat`/`rewrit`/`fingerprint` 只命中模型名归一化里的 `name.truncate`（`#L813`）。即 Codex 路径**没有**A01 那样的「外部改写 → 钉 EOF 不重放」保护；若 rollout 被外部截短重写，行号游标可能把新内容里行号更小的记录当作已消费而跳过（漏记），或反之产生错配。**此点为读码结论，建议实机验证。**

### 7. 费用字段

- 日志内**没有上游自报费用**，全部本地估算：`find_codex_pricing`（先 `normalize_codex_model` 归一化，再 `find_model_pricing`，`#L1871-L1873`、`#L786-L830`）→ `CostCalculator::calculate_for_app("codex", …)`（`#L1802-L1825`）。无价时五项全 `"0"`。
- `normalize_codex_model` 规则（`#L786-L830`）：转小写 → 去 `provider/` 前缀 → 去 ISO 日期后缀 `-YYYY-MM-DD` → 去紧凑日期后缀 `-YYYYMMDD`。
- 金额列同上表，TEXT 十进制字符串，USD；`latency_ms` = 按事件时间戳估算（复用 `session_usage::estimated_latency_ms`），估不出写 0（`#L1853`）。

### 8. 可疑或与契约冲突的上游行为

1. **`duplicate_snapshot` 以「数字完全相同」判重**（`#L1193-L1209`）：与契约第 6 条方向冲突。命中时该事件 delta 记 0，属**漏记**方向（不会双算）。上游对同源/跨源的判别规则相当细致（`#L1041-L1046`），但仍以「签名相等」为唯一依据。
2. **request_id 是位置键**（`{root_thread_id}:{event_index}`，`#L1672-L1673`）：非零事件的序号一旦因文件前缀被改写（revert/重写、压缩-解压往返口径变化）而整体前移，新事件会撞上旧主键 → **少记**（`INSERT OR IGNORE`）；被删除事件的行保留在库（不删除）。与 A04 用 `prompt_id` 锚定上游 id 的做法相反。
3. **疑似重复仍然插入**（`#L1781-L1789`）——已知会有双算的可能，只做计数与告警。
4. **无价模型成本记 0**（`#L1818-L1825`）——与「没有价格不等于免费」冲突；库内没有「未定价」标记位。
5. **`input_token_semantics=TOTAL` 与 `reasoning_output_tokens` 不落库**：reasoning token 在签名里参与判重却不可见，行级无法还原。
6. **缺截断检测**（见第 6 点）——与 A01 的策略不一致，同一产品内两类来源的「外部改写」鲁棒性不同。
7. **deferred 状态仅在进程内存**（`CODEX_REPLAY_CACHES`，`#L440-L460`）：重启后 deferred 状态丢失，但会因游标未推进而重扫（不丢数据，只是多一轮开销）。

### A02 证据链接

- `https://raw.githubusercontent.com/farion1231/cc-switch/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/session_usage_codex.rs`：`#L1-L47`（模块文档/依赖/前缀）、`#L49-L92`（CumulativeTokens/DeltaTokens/签名）、`#L94-L212`（文件指纹、父时间线、ParsedTokenEvent、行头分类）、`#L242-L398`（计时分类与 RequestTimer）、`#L400-L460`（解析结果结构、replay 缓存）、`#L462-L511`（文件名规则、cursor 路径判定）、`#L602-L648`（线程 id 提取、父解析）、`#L657-L694`（字段解析、来源标记）、`#L703-L784`（pass 状态、游标继承）、`#L786-L894`（模型归一化、delta、累计解析）、`#L906-L981`（入口与文件收集）、`#L996-L1011`（递归扫描）、`#L1013-L1286`（parse_codex_file 全流程）、`#L1288-L1412`（父时间线、replay 前缀）、`#L1414-L1478`（deferred、游标写回）、`#L1481-L1710`（单文件同步、入库循环）、`#L1745-L1873`（插入、input 语义、定价）、`#L2295-L2370`（delta 测试）、`#L2443-L2475`（跨 lane 重放去重测试）、`#L2639-L2747`（重置与缓存写测试）、`#L3413-L3461`（子 agent 命名测试）、`#L3560-L3606`（压缩往返测试）、`#L3696-L3745`（缓存写对 legacy 代理行去重）、`#L3746-L3785`（疑似重复仍插入）、`#L3908-L3929`（cached 钳制测试）
- `.../src-tauri/src/codex_rollout_file.rs`：`#L1-L7`（压缩背景）、`#L28-L38`、`#L41-L49`、`#L64-L86`、`#L92-L133`（读写、逻辑名、同形态去重）
- `.../src-tauri/src/codex_config.rs`：`#L270-L276`（目录）
- `.../src-tauri/src/services/usage_stats.rs`：`#L730-L769`（疑似重复查询）
- `.../src-tauri/src/database/schema.rs`：`#L206`（语义列默认值）

---

## A03 Gemini CLI（`services/session_usage_gemini.rs`）

### 1. 默认目录与各 OS 路径规则（含环境变量覆盖）

- 根目录：`get_gemini_dir()` = 应用设置 `gemini_config_dir` 覆盖值，否则 `get_home_dir().join(".gemini")`。
  `gemini_config.rs#L9-L15`；覆盖值来源 `settings.rs#L973-L979`。
- 采集：`<gemini>/tmp/<project_hash>/chats/session-*`，仅两层目录遍历（`tmp/*/chats/`），文件名必须以 `session-` 开头且通过 `is_session_file`。
  `session_usage_gemini.rs#L92-L131`。
- `is_session_file`（`session_manager/providers/gemini.rs#L67-L73`）：扩展名 `jsonl` → 是；扩展名 `json` → **仅当同名 `.jsonl` 不存在时**才算（迁移期两形态并存时优先新形态）；其余不是。
- 模块文档给出目录样例 `~/.gemini/tmp/<project_hash>/chats/session-*.json(l)`。
  `session_usage_gemini.rs#L1-L15`。
- **未读取任何 Gemini 目录环境变量**（`GEMINI_DIR` 等未见）；OS 差异同样由 `get_home_dir()` 承担。

### 2. 文件格式与记录类型

- 两种形态：
  - 旧版：**单个 JSON 对象** `{ sessionId, messages: [...] }`。
  - 新版：**JSONL**，按 Gemini CLI 的 `loadConversationRecord` 回放还原成同一形状（`session_manager/providers/gemini.rs#L75-L122`）：带 `sessionId` 的行是元数据；带字符串 `id` 的行是消息（同 id **原位覆盖**，保持首次出现顺序）；`{"$set":{...}}` 合并元数据、其中带 `messages` 时整体替换；`{"$rewindTo": id}` 删掉该条及之后的消息，找不到 id 时清空。
- 导入侧只处理 `messages[]` 中 `type == "gemini"` 的元素（`session_usage_gemini.rs#L175-L186`），并跳过四个计数全 0 的消息（`#L187-L190`，注释「跳过全零的空 token 消息」）。
- 消息级字段：`id`（缺失按 `"unknown"`）、`model`（缺失按 `"unknown"`）、`timestamp`、`tokens` 对象（`#L194-L200`）。
- 无压缩形态处理。

### 3. usage 字段名与包含关系（逐字段）

`parse_gemini_tokens`（`session_usage_gemini.rs#L229-L237`）：

| 原始字段（`msg.tokens.*`） | 内部字段 | 落库列 |
| --- | --- | --- |
| `input` | `input` | `input_tokens` |
| `output` | `output` | （与 thoughts 相加后）`output_tokens` |
| `cached` | `cached` | `cache_read_tokens` |
| `thoughts` | `thoughts` | **无独立列**，加进 `output_tokens` |
| `tool`、`total` | 不读取 | — |

- **output 含 thoughts**：`let output_tokens = tokens.output + tokens.thoughts;`，注释「合并 thoughts 到 output（思考 token 按输出计费）」。
  `session_usage_gemini.rs#L263-L264`；测试断言 `29 + 405 = 434`（`#L440-L457`）。
- `cache_creation_tokens` 恒写 `0`（`#L345`，注释「cache_creation_tokens」）。
- **input 含 cached**：属 cache-inclusive 集合（`sql_helpers.rs#L23`），计价用 `calculate_for_app("gemini", …)` 会从 input 扣掉 cache_read（`calculator.rs#L56-L89`）。测试夹具 `{input:8522, output:29, cached:3138, thoughts:405, total:8956}` 中 `8522+29+405 = 8956`，说明 `cached` 是 `input` 的子集（`session_usage_gemini.rs#L440-L457`）。
- 该 INSERT **不写 `input_token_semantics`**（`#L314-L362` 的 24 列清单止于 `data_source`），落库为默认 `LEGACY=0`；读侧 `fresh_input_sql` 对 `gemini + LEGACY` 的分支只减 `cache_read_tokens`（`sql_helpers.rs#L61-L64`），因 cache_creation 恒 0，结果与 `TOTAL` 分支等价；但若出现 `input < cached` 的异常行会落到 `ELSE` 分支、不做扣减（`sql_helpers.rs#L65`）。与 codex/grokbuild 显式写 `TOTAL` 不一致。

### 4. 计数器类型：逐消息独立值（无累计、无差分）

- 模块文档明确列出与 Claude/Codex 的差异：「无需 delta 计算：tokens 字段是 per-message 独立值；无需状态恢复：不依赖前一条消息的累计值；天然去重：每条消息有唯一 id 字段」。
  `session_usage_gemini.rs#L10-L15`。
- 实现中确实没有任何差值/高水位逻辑；每条消息独立算一次成本并 UPSERT。
- 无耗时数据：`latency_ms`、`first_token_ms` 硬写 `0` / NULL（`#L351-L352`）；`status_code` 硬写 200（`#L353`）。

### 5. 事件身份与去重键

- **request_id = `gemini_session:{sessionId}:{message.id}`**（`#L202-L204`）。
  - `sessionId` 取顶层 `sessionId`，缺失时 `"unknown"`（`#L159-L163`、`#L203`）。
  - `message.id` 缺失时 `"unknown"`（`#L195`）——**同会话内所有缺 id 的消息会共用一个 request_id**，UPSERT 会互相覆盖（漏记）。
- 去重：`should_skip_session_insert(conn, request_id, &dedup_key)`（request_id 已存在 或 与 ±600s 内代理行指纹匹配）→ 跳过返回 `false`（`#L266-L277`）。`DedupKey{ app_type:"gemini", model, input, output(含thoughts), cache_read, cache_creation:0, created_at }`（`#L266-L274`）。
- 写入用 **UPSERT**（`INSERT … ON CONFLICT(request_id) DO UPDATE`），更新 `model/input_tokens/output_tokens/cache_read_tokens/四项成本`，仅当四元组或模型变化时才算「有变更」（`#L313-L335`、`#L365-L367`）。注释：全量重读可能携带更新值。
- **分叉 / 复制 / 子 Agent**：无此概念。会话文件即一条会话；`session-*.json(l)` 之间的消息不会互相覆盖（request_id 含 sessionId）。
- **`$rewindTo` 的后果**：回放层会把被 rewind 的消息从 `messages` 中删掉，但**导入层没有任何 DELETE 路径**（全文检索 `DELETE FROM` 只命中测试），因此先前已入库的行会**保留**在 `proxy_request_logs` 中。

### 6. 日志截断、尾行不完整、文件轮转

- 只有 **mtime 门**：`file_modified <= last_modified` → 跳过（`#L143-L151`）；文件一变就**整文件读入 + 全量解析**（`#L153-L157`）。
- 写回 `update_sync_state(db, path, file_modified, gemini_msg_count)`（`#L223-L224`）——`last_line_offset` 存的是「有计数的 gemini 消息条数」，但 Gemini 路径**从不读取该 offset 做跳过**，仅作记录。
- JSON 解析失败（`parse_session_document` 返回 None）→ 整个文件报错、零导入（`#L156-L158`），错误进 `SessionSyncResult.errors`（`#L71-L76`）。
- 坏行/不支持的行在回放层被 `continue` 丢弃（`gemini.rs#L93-L96`）。
- **无截断/重写/轮转处理**，也没有 A01 的尾部指纹。可靠性依赖「全量重读 + UPSERT 幂等 + 指纹去重」；被 rewind 删除的消息其历史行不会被清理（见第 5 点）。

### 7. 费用字段

- 无上游自报费用；本地定价：`find_gemini_pricing` → `find_model_pricing`（精确 + 前缀），`CostCalculator::calculate_for_app("gemini", …)`，`cost_multiplier = 1`（`#L290-L311`）。无价时五项全 `"0"`（`#L304-L311`）。
- thoughts 通过并入 `output_tokens` 间接按输出价计费，无独立列可还原。

### 8. 可疑或与契约冲突的上游行为

1. **`output + thoughts` 直接相加后落库**（`#L263-L264`）：上游把两个上游维度合并成「输出」一个维度，并在 `DedupKey` 中同样使用合并值（`#L266-L274`）。若我们的契约要求 `output` 与 `thoughts` 分别保存，此处会**永久丢失拆分**（与契约第 4 条「Token 维度不可混用」相关）。
2. **message id 缺失 → 全部消息共用一个 request_id**（`#L195` + `#L204`）：UPSERT 会把多条不同消息压成一行（漏记），且不报错。
3. **未写 `input_token_semantics`**，但 app 属 cache-inclusive 集合：语义正确性依赖读侧白名单而不是写入侧显式标记（与 codex/grokbuild 不一致）。
4. **回放层删除、导入层不删除**（`$rewindTo`）：被 rewind 掉的消息，其已入账行会长期留在统计里。这是「已消耗 token 不退」的有意行为还是漏洞，源码未说明。
5. **`latency_ms = 0`、`status_code = 200` 硬编码**（`#L351-L353`）：0 兼作「无计时」，与真实 0 值不可区分。
6. **`tool` 与 `total` 字段被忽略**：上游 `total`（= input+output+thoughts）不参与任何校验，若上游口径变化不会被发现。
7. **无截断处理**：全量重读 + UPSERT 能兜住大部分重复，但文件被截短到少于已入库消息数时，不产生删除，行保留（方向是保留而非双算）。

### A03 证据链接

- `https://raw.githubusercontent.com/farion1231/cc-switch/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/session_usage_gemini.rs`：`#L1-L30`（模块文档与依赖）、`#L32-L39`（GeminiTokens）、`#L42-L89`（入口）、`#L92-L131`（文件收集）、`#L136-L227`（单文件同步）、`#L229-L237`（字段解析）、`#L240-L368`（插入与 UPSERT）、`#L440-L507`（字段/零值/纯缓存测试）
- `.../src-tauri/src/gemini_config.rs`：`#L9-L15`（目录）
- `.../src-tauri/src/session_manager/providers/gemini.rs`：`#L67-L73`（`is_session_file`）、`#L75-L122`（`parse_session_document` 回放规则）、`#L124-L145`（消息表语义）
- `.../src-tauri/src/settings.rs`：`#L973-L979`
- `.../src-tauri/src/services/sql_helpers.rs`：`#L23`、`#L30-L32`、`#L55-L66`
- `.../src-tauri/src/proxy/usage/calculator.rs`：`#L52-L89`
- `.../src-tauri/src/services/usage_stats.rs`：`#L594-L630`、`#L667-L694`

---

## A04 Grok Build / Grok CLI（`services/session_usage_grokbuild.rs`）

### 1. 默认目录与各 OS 路径规则（含环境变量覆盖）

- 根目录：`get_grok_config_dir()` = 应用设置 `grok_config_dir` 覆盖值，否则 `get_home_dir().join(".grok")`。
  `grok_config.rs#L25-L27`；覆盖值来源 `settings.rs#L981-L987`。
- 会话根：`session_manager::providers::grokbuild::session_roots()` = `<grok>/sessions` 与 `<grok>/archived_sessions`（与会话浏览器同根）。
  `session_manager/providers/grokbuild.rs#L36-L42`；消费处 `session_usage_grokbuild.rs#L138-L145`。
- 递归收集**文件名为 `updates.jsonl`** 的文件（容忍布局深度变化）：
  - 最大深度 16（`#L150-L162`）
  - **无条件跳过一切符号链接**（目录 symlink 不递归、文件 symlink 不收集），并打 info 日志便于排查「用量数据静默缺失」（`#L166-L185`）
  - 单文件 > 50 MiB 视为异常/恶意文件，跳过（`#L147-L149`、`#L199-L207`）
- 目标布局（模块文档）：`~/.grok/{sessions,archived_sessions}/<enc-cwd>/<session-id>/updates.jsonl`（`#L1-L10`）。
- **未读取 `GROK_HOME` 之类环境变量**。

### 2. 文件格式与记录类型

- JSONL，每行一个 JSON-RPC 风格记录。识别条件（`parse_grok_usage_events`，`#L312-L384`）：
  - `record.method == "_x.ai/session/update"`
  - `record.params.update.sessionUpdate == "turn_completed"`——**显式标为其它类型的事件即使带 usage 也跳过**（注释：中途快照若与轮末事件并存，双导会双算）；字段缺失时向后兼容放行（`#L328-L337`）
  - `record.params.update.usage` 必须是对象（`#L338-L343`）
  - 顶层 `record.timestamp` 必填（沉降窗与接管守卫都依赖事件时刻），实测为数字 epoch 秒；>1e11 视作毫秒；RFC3339 字符串仅作防御性兜底（`#L344-L347`、`#L402-L414`）
- 每个事件按 `payload.usage.modelUsage`（JSON object，model → 计数器）产出**每模型一行**；缺 `modelUsage` 时退回顶层 `usage` 计数、模型名 `"unknown"`（`#L355-L367`）。遍历后按模型名排序，保证多轮重扫顺序稳定（`#L368-L370`）。

### 3. usage 字段名与包含关系（逐字段）

`parse_grok_counters`（`#L386-L400`）：

| 原始字段 | 内部字段 | 落库列 |
| --- | --- | --- |
| `inputTokens` | `input` | `input_tokens`（**含 cache read**） |
| `outputTokens` | `output` | `output_tokens` |
| `cachedReadTokens` | `cached` | `cache_read_tokens` |
| `apiDurationMs` | `api_ms` | `latency_ms` |
| `modelCalls` | `model_calls` | **不落库** |
| `costUsdTicks` | `cost_ticks` | 用于 `total_cost_usd`（见第 7 点） |
| `costIsPartial` | `cost_partial` | 不落库（影响成本口径） |

- **input 含 cache read**：模块文档与 INSERT 注释都写明，并**显式写 `input_token_semantics = TOTAL`**（`#L521-L522`、`#L574`）。测试断言该列等于 `INPUT_TOKEN_SEMANTICS_TOTAL`（`#L767-L768`）。
- **reasoning 是 output 的子集、不参与计费**：模块文档「`reasoningTokens` ⊂ `outputTokens`（totalTokens = input + output，且 costUsdTicks 反推 output 未加计 reasoning），不参与计费」（`#L21-L22`）。生产解析器**根本不读 `reasoningTokens`**（只在测试夹具里显式写 0，`#L632`）。因此 output 可以直接使用 `outputTokens`，不需要（也没有）相加。
- `cache_creation_tokens` 恒写 `0`（`#L558`）。
- 「input 含 cache read」与计价一致：`calculate_for_app("grokbuild", …)` 会扣掉 cache_read（`calculator.rs#L56-L89`）；测试 `cost_matches_cli_reported_ticks_with_cache_reads` 用 `13793/21/13696` + 0.30 的 cache read 单价钉死（`#L1221-L1252`）。

### 4. 计数器类型：逐轮独立总量（**不是**会话累计，也**不做**差分）

- 模块文档把这条写成红线（`#L12-L20`）：
  > `sessionUpdate == "turn_completed"` 事件的 usage 是【该 user prompt 一轮的独立总量】：轮内跨 inference loop 累加（`modelCalls`/`numTurns` = 本轮 loop 数），下一轮从零起算。【不是】进程或会话累计——进程累计走 CLI 内另一条独立通道（`GetSessionUsage`，"since start or last resume"），不落 updates.jsonl。🔴 勿改回相邻事件差分：那是把每轮总量误当累计快照，会把第二轮记成两轮之差造成巨量漏记（曾犯，实测单进程双 prompt 证伪）。
- 逐事件按面值入账即正确；**两轮数值完全相同 = 两笔真实用量，照常都入账**（`#L19-L20`）。
- 测试钉死该行为：`two_turns_import_at_face_value_matching_reported_ticks`（`#L731-L777`）、`second_turn_with_smaller_counters_imports_at_face_value`（第二轮数值更小，`#L779-L812`）、`identical_turns_both_import`（`#L814-L842`）。
- 没有高水位、没有 saturating_sub、没有累计基线。

### 5. 事件身份与去重键

- **request_id = `grok_session:{session_id}:{turn_key}:{model}`**（`#L277-L282`）。
  - `session_id` = `updates.jsonl` 所在目录名（可执行文件所在会话目录名，与 `summary.json` 的 `info.id` 一致）；注释说明唯一性押在该 UUIDv7 全局唯一上，归档/活跃副本同 id 经 UPSERT 幂等收敛（`#L222-L230`）。
  - `turn_key` = 事件 `prompt_id`（每轮唯一的 UUID）；缺失时回退**文件内序号** `idx{N}`（`#L270-L281`）。注释解释为什么不用序号做主键：updates.jsonl 前缀被改写（rewind 截断）导致事件序号前移时，幸存轮次仍命中原行不会双算；被移除轮次的行保留（rewind 不退还已消耗 token）。同一 `prompt_id` 写多条（未观测到）时 UPSERT 取后者，方向是少记不双算。
  - 键含 `model`，所以同一轮的多模型会产生多行，互不覆盖。
- 写入用 UPSERT，WHERE 里带 `data_source = 'grok_session'` 作为纵深防御（`#L517-L548`）。
- **接管守卫（替代指纹去重）**：插入前按事件时刻查询 ±`SESSION_PROXY_DEDUP_WINDOW_SECONDS`（600s）内是否存在任意 `app_type='grokbuild'` 且 `data_source='proxy'` 的行；命中则整条事件的所有模型行都**跳过并计入 `skipped`**（`#L250-L268`；查询实现 `usage_stats.rs#L696-L728`）。设计理由：接管态下 CLI 照写 updates.jsonl，但轮事件是多 loop 聚合值，与代理逐请求行结构性不相等，指纹永不命中（`#L29-L33`、`usage_stats.rs#L699-L707`）。
- **沉降窗**：只导入 `now - created_at >= 60s` 的事件；遇到第一条未沉降事件即 `break`（后续一起等下一轮），并且**不写同步状态**（`#L51-L66`、`#L240-L248`、`#L302-L307`）。目的是让接管守卫查询必然能看到已落库的代理行（`#L51-L57`）。
- **分叉 / 复制**：rewind 截断的处理见 `turn_key`；测试 `rewind_truncation_does_not_double_count`（`#L1103-L1160`）。无子 Agent 概念。

### 6. 日志截断、尾行不完整、文件轮转

- 只按 **mtime 门**（`#L209-L212`），文件一变就**整文件重读**；注释解释为何不做 offset 增量（事件已是逐轮独立值，改 offset 在正确性上可行，但需另行处理「延后事件的 offset 回退」，收益不值得复杂度）（`#L214-L217`）。
- 坏行/非 JSON 行静默 `continue`（`#L316-L323`）；带 usage 但缺时间戳的事件被丢弃（`#L344-L347`）。
- 超大文件（>50 MiB）跳过，且不报错、不推进游标（`#L199-L207`）。
- deferred（存在未沉降事件）时不写 `session_log_sync`，下一轮整文件重扫（`#L302-L307`、测试 `#L868-L910`）。
- **归档会话**：`archived_sessions` 与 `sessions` 同根收集，同一 session id 的归档/活跃副本靠 UPSERT 收敛（`#L138-L145`、`#L222-L224`）。
- **无截断/重写检测**：靠 UPSERT 幂等 + `prompt_id` 锚定键；已知局限：未来时间戳（时钟误设）会让该文件持续 defer 并整文件重扫，墙钟越过「事件时刻+窗口」后自愈（`#L241-L244`）。
- symlink 循环由「不跟随 symlink」+ 深度上限保护（`#L150-L185`，测试 `#L1391-L1422`）。

### 7. 费用字段（含 `costUsdTicks` 换算）

- **单位**：`costUsdTicks` 是 CLI 自报的**本轮精确成本**，1 tick = 1e-10 USD；换算 `Decimal::from(ticks) / 10_000_000_000`。
  `#L76-L79`（字段注释）、`#L87-L90`（`reported_cost_usd()`）。
  测试：`338_880_000 ticks → 0.033888 USD`（`#L1188-L1219`）；带缓存样本 `44_288_000 ticks`（`#L1221-L1252`）。
- **total_cost 取值优先级**（`#L445-L515`，注释明确「回填机制只补 total<=0 的行、从不修正已有正值，入账后没有修复路径」）：
  1. 有本地价 + 自报完整（非 partial）→ **total 用自报**，分项仍按本地价；偏差超 `max(1%, 1e-6)` 时打漂移告警（提示本地定价该更新），**但 total 仍以自报为准**（`#L455-L479`；测试 `reported_ticks_override_stale_local_pricing` `#L1254-L1294`）。
  2. `costIsPartial`（事件级 `usage.costIsPartial` 或模型级 `costIsPartial`）→ 有本地价则**全额本地复算**（token 数完整）并抑制漂移告警；无价才用自报下界入账（`#L450-L452`、`#L457-L459`；测试 `#L1296-L1326`）。
  3. 无自报 → 本地复算。
  4. 无本地价：有自报则 `total = 自报`、**分项全 0**；彻底无价无自报才整单记 0，两种情况都打 warning（`#L480-L514`；测试 `unpriced_model_falls_back_to_reported_ticks` `#L1328-L1363`）。
- `latency_ms = apiDurationMs`（本轮 API 时长），`#L564`。
- 自报与本地定价的实测一致性：模块文档称 6 个实测样本与本地定价 `grok-4.5-build` 2/6/0.30 分毫不差（`#L23-L28`）。

### 8. 可疑或与契约冲突的上游行为

1. **同值不同轮都入账**（`#L814-L842`）：与契约「相同数字不能自动视为重复事件」方向**一致**（正向事实）。与 A01/A02 的「相同数字判重」形成对照——上游内部口径并不统一，取用时不可一概而论。
2. **无价模型分项记 0**（`#L480-L514`）：total 可能非 0 而四项分项全 0。契约要求「未知不等于零」，此处 0 表示「不可分解」，下游必须能区分，否则会把未知当免费。
3. **接管守卫无 session 维度**（`usage_stats.rs#L705-L707` 自认局限）：±10 分钟内任一 grokbuild 代理行会给**全部**会话事件投下阴影；接管/官方两态交替或并行使用时，官方侧轮次会被跳过（**漏记**而非双算，方向保守）。
4. **沉降窗引入上屏延迟**：官方态用量最多延迟约「一个沉降窗 + 一次后台同步周期（60s）」（`#L56-L57`）。
5. **未来时间戳致持续 defer**（`#L241-L244`）：时钟误设会让该文件每轮整文件重扫，直到墙钟越过「事件时刻+窗口」。
6. **`modelCalls` / `numTurns` 解析但不落库**（`#L393`）：无法从行里还原「本轮是几个 inference loop 聚合出来的」。
7. **模型名回退 `"unknown"`**（`#L364-L367`）：此时 token 照常入账、成本走「无价」路径；跨源去重对 `'unknown'` 侧的 model 比较是放行的（`usage_stats.rs#L658-L662` 同时影响 A01/A02/A03）。
8. **对称的「同值双计」风险**：若上游对同一 `prompt_id` 真的重复写 `turn_completed`，UPSERT 取后者（少记）；但若两次真实轮次恰好同值同 prompt_id 不同 model，会产生两行（正确）。该假设在源码里标注为「未观测到」（`#L273-L276`）。

### A04 证据链接

- `https://raw.githubusercontent.com/farion1231/cc-switch/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/session_usage_grokbuild.rs`：`#L1-L49`（模块文档/事件口径/依赖）、`#L51-L91`（沉降窗、GrokCounters、ticks 换算）、`#L93-L145`（事件结构、入口、文件收集）、`#L147-L185`（上限、深度、symlink）、`#L187-L310`（单文件同步、沉降窗、接管守卫、request_id、UPSERT 调用）、`#L312-L414`（事件与字段解析、时间戳）、`#L416-L587`（插入、成本优先级、UPSERT）、`#L731-L842`（逐轮面值测试）、`#L868-L1052`（沉降窗/守卫测试）、`#L1103-L1185`（rewind/空 prompt_id）、`#L1188-L1363`（ticks 与定价漂移/partial/无价测试）、`#L1391-L1422`（symlink 循环）
- `.../src-tauri/src/session_manager/providers/grokbuild.rs`：`#L36-L42`（`session_roots`）
- `.../src-tauri/src/grok_config.rs`：`#L25-L27`（目录）
- `.../src-tauri/src/settings.rs`：`#L981-L987`
- `.../src-tauri/src/services/usage_stats.rs`：`#L317`（±600s）、`#L696-L728`（接管活动查询）
- `.../src-tauri/src/services/sql_helpers.rs`：`#L23`、`#L30-L32`、`#L55-L66`
- `.../src-tauri/src/proxy/usage/calculator.rs`：`#L52-L89`

---

## 跨适配器横向对照（同一契约维度的差异表）

| 维度 | A01 Claude Code | A02 Codex | A03 Gemini CLI | A04 Grok Build |
| --- | --- | --- | --- | --- |
| 计数器 | 单消息快照 | 累计（`total_token_usage`）+ 优先 `last_token_usage` | 逐消息独立值 | 逐轮独立总量 |
| 差分 | 无 | 有（saturating，高水位只增） | 无 | 明确禁止（红线注释） |
| input 是否含 cache | 否（fresh） | 是（含 read+write） | 是（含 cached） | 是（含 cached read） |
| 落库 `input_token_semantics` | **未写**（默认 LEGACY=0） | 显式 `TOTAL` | **未写**（默认 LEGACY=0） | 显式 `TOTAL` |
| output 是否含 thoughts/reasoning | 无该字段、不相加 | reasoning 只进签名、不相加 | **`output + thoughts` 相加** | reasoning ⊂ outputToken、不相加 |
| request_id | `session:{message.id}` | `codex_session:thread-v1:{threadId}:{eventIndex}` | `gemini_session:{sessionId}:{messageId}` | `grok_session:{dirName}:{prompt_id}:{model}` |
| 身份是否含文件/位置 | 否 | **含位置序号** | 否 | 否（缺 prompt_id 时回退序号） |
| 增量方式 | 字节游标 + 尾部指纹 | 行号游标（每轮从头重读） | mtime 门 + 全量重读 | mtime 门 + 全量重读 |
| 截断/重写保护 | 有（钉 EOF、不重放、上报） | **未见** | 未见 | 未见 |
| 跨源去重依据 | token 四元组 + ±600s + model | 同左（+ 疑似重复另计） | 同左 | **接管活动守卫**（非指纹），读侧过滤列表不含 `grok_session` |
| 上游自报费用 | 无 | 无 | 无 | **有**（`costUsdTicks`，1e-10 USD） |
| 无价时成本 | 五项全 0 | 五项全 0 | 五项全 0 | total 用自报、分项 0 |
| 状态码 | 硬写 200 | 硬写 200 | 硬写 200 | 硬写 200 |

补充事实：读侧 `effective_usage_log_filter` 的去重来源集合为 `'session_log','codex_session','gemini_session','opencode_session'`，**不含 `grok_session`**（`usage_stats.rs#L443-L444`、`#L404-L406`）；`has_matching_proxy_usage_log` 的「允许缺 cache_creation」集合是 `codex|gemini|opencode`，同样不含 grokbuild（`usage_stats.rs#L671-L673`）。即 A04 走的是完全不同的防双算机制。

---

## 待官方版本核实事项

以下为「只读第三方实现源码」无法确证、必须回到官方文档 / 官方客户端源码 / 实机夹具确认的点，**不得当作已证实结论**：

1. **A01**：Claude Code JSONL 的 `message.usage` 是否会新增独立的 thinking/reasoning 计数字段；`output_tokens` 是否包含 thinking。本次读到的解析器**没有**读取任何 thinking 字段，若官方语义是「thinking 单列」，A01 的口径会与官方不一致。
2. **A01**：`cache_creation.ephemeral_1h_input_tokens` 的官方计价系数（上游用 1.6 × 5m 单价，`calculator.rs#L98-L107`）需以官方定价页核实。
3. **A02**：`last_token_usage` / `total_token_usage` 的官方语义（是否为 rate-limit lane 独立累计）、`cache_write_input_tokens` 从哪个 Codex 版本开始出现、`reasoning_output_tokens` 与 `output_tokens` 的包含关系。
4. **A02**：rollout 被 `thread/revert` 截短重写时行号游标的行为——本提交**未见截断检测**，需实机构造 rewind 验证是否漏记/错配。
5. **A02**：压缩（`[features] local_thread_store_compression`、app-server `rollout/compress`）的官方开关名、压缩后文件命名与 mtime 保留行为。
6. **A03**：Gemini CLI `tokens` 对象里 `input` 是否含 `cached`、`thoughts` 是否按输出计价、`total` 的定义；`session-*.json` → `.jsonl` 的迁移时点与 `$set`/`$rewindTo` 记录形式的官方文档。
7. **A03**：消息 `id` 缺失是否真会发生（上游按 `"unknown"` 兜底会让多条消息撞同一个 request_id）。
8. **A04**：Grok CLI `turn_completed` 事件与 `usage.modelUsage` / `costUsdTicks` / `costIsPartial` 是否官方文档化；tick 定义是否为 1e-10 USD；`GetSessionUsage` 通道的口径。
9. **四个适配器共同的路径问题**：官方客户端是否支持用环境变量覆盖配置根目录（`CLAUDE_CONFIG_DIR`、`CODEX_HOME`、Gemini 的自定义目录、Grok 的目录）。上游 cc-switch **一律不读这些环境变量**，只用自身设置里的覆盖值 + `dirs::home_dir()`；若我们的采集器要读环境变量，需先拿到官方依据。
10. **Windows 行为**：Codex 路径的注释称 Windows 下 Codex 持有写句柄时 mtime 可能不变（`#L1498-L1500`），以及文件身份改用 volume serial + file id（`#L128-L145`）。这些都需要在 Windows 上实测确认，不能只凭 macOS 行为宣布可用（项目 `AGENTS.md` 第 10 条）。
11. **A01/A02 的明细保留窗口**：源码注释提到 `rollup_and_prune` 会汇总并删除 30 天前明细、`request_id` 去重对已剪条目失明（`session_usage.rs#L93-L98`、`#L571-L585`、`#L833-L834`）。本次**未读取 rollup 实现本体**（不在四个导入器及其共享模块范围内），窗口边界与汇总口径需另行核实。
12. **跨源去重的实测影响面**：±600 秒 + 「token 四元组相同 + model 相同/unknown」的判重（`usage_stats.rs#L634-L694`）在真实混杂流量下会误合并多少行、以及 A04 的接管守卫会漏掉多少官方态轮次，都只能实机测量。

（本文结论仅来自上述固定提交处读到的代码；未读到的部分已在正文标注「未获取」或列入本清单。）


## 自动采集交付补充（2026-10-11）

本节与上文最初的研究记录分开。此次另读 cc-switch 提交 `f88068b9e59a43379e14cc8f294467aebb71e26b`：

- [后台入口 lib.rs L1287–L1345](https://github.com/farion1231/cc-switch/blob/f88068b9e59a43379e14cc8f294467aebb71e26b/src-tauri/src/lib.rs#L1287)：启动同步、60 秒周期、错过周期 Skip，自动同步开关会阻止源文件采集。
- [前端查询 usage.ts L202–L206](https://github.com/farion1231/cc-switch/blob/f88068b9e59a43379e14cc8f294467aebb71e26b/src/lib/query/usage.ts#L202)：默认 30 秒显示刷新，背景刷新默认关闭；显示周期独立于采集周期。
- [Codex 重播判断 L1193–L1240](https://github.com/farion1231/cc-switch/blob/f88068b9e59a43379e14cc8f294467aebb71e26b/src-tauri/src/services/session_usage_codex.rs#L1193)：按 total+last 签名与来源/前一快照识别重播，last 优先、累计基线同时更新，零增量不成为计费记录。没有 total 时相同 last 数字仍可代表独立请求。

Providock 采用 60 秒源扫描和独立显示刷新，增加可持久化的 Agent/账户两个开关；关闭调度后纯等待通知。保留八源，不启用文件监听。各扫描器仍沿用自身游标/mtime/WAL/行号策略，并非所有源都只读新增字节。

本机只读计量快照发现：原 Codex 发现逻辑在 YYYY/MM/DD 层读取文件前退出，漏掉实际文件；包含 total+last 的重播也被旧解析器重复计数。新 parser_version 为 codex-rollout-2，合成样本 replayed-cumulative 覆盖同源/相邻源重播、同 last 但累计推进的独立请求、零 last 心跳，人工预期两条请求共 240 Token。升级事务只删除已证实重复/心跳的精确事件身份，其他历史保留。

验算方法：Kimi 以 durable usage.record 的互斥四分量独立求和；Codex 使用各完整会话最终累计量，不把生产解析器输出当预期。六项 Token 分别核对全量与当天，再重扫确认守恒。仅临时保存无正文的 usage-only 快照，不提交真实日志。核对结果与格式限制见 CURRENT_STATUS；第三方行为仍不能替代账户端点的官方依据。
