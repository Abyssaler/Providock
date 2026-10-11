# A05–A07 上游证据：cc-switch 的 OpenCode / Pi / MCode 导入器与路径配置

- 核查日期：2026-10-11
- 固定提交：`df40d53c36b604e044b4d5b9091aeb961aa56990`（cc-switch，研究时 main，版本线 v4.0.8）
- 方法：`curl` 直读该提交的 raw 源码全文，逐行引用。未运行 cc-switch、未读取任何用户真实日志或数据库。所有结论来自实际读到的代码行；仓库内无法确认的项写入文末「待官方版本核实事项」，不推测补零。
- 证据 URL 前缀：`https://raw.githubusercontent.com/farion1231/cc-switch/df40d53c36b604e044b4d5b9091aeb961aa56990/`

## 0. 读取清单与清单差异

任务指定的 5 个文件全部 200 获取成功，无 404，因此**未使用** tree API 回退。但 tree API（`?recursive=1`，返回 `truncated: false`）被用于确认支撑文件存在。

| 任务指定文件 | 行数 | 作用 |
| --- | --- | --- |
| `src-tauri/src/services/session_usage_opencode.rs` | 857 | A05 导入器 |
| `src-tauri/src/services/session_usage_pi.rs` | 1538 | A06 导入器 |
| `src-tauri/src/services/session_usage_mcode.rs` | 206 | A07 导入器 |
| `src-tauri/src/mcode_config.rs` | 621 | MCode 数据目录 |
| `src-tauri/src/pi_config/mod.rs` | 640 | Pi agent 目录（仅 `models.json` / `settings.json`） |

**清单差异（需记录）**：任务给定的 5 个文件**不包含**三个适配器的路径与游标真正定义处，以下支撑文件必须一并读取，本文所有路径/游标结论引自行号均指这些文件：

| 补充读取文件 | 行数 | 为什么必须读 |
| --- | --- | --- |
| `src-tauri/src/opencode_config.rs` | 1445 | OpenCode 的 `get_opencode_db_path()` 只在这里；导入器仅 `use` 它 |
| `src-tauri/src/session_manager/providers/mcode.rs` | 221 | MCode 的 `database_path()` / `open_database()` 只在这里；`mcode_config.rs` 只给数据目录 |
| `src-tauri/src/session_manager/providers/pi.rs` | 1121 | Pi 的会话文件发现 `session_files()` 与 `PI_CODING_AGENT_SESSION_DIR` 只在这里 |
| `src-tauri/src/services/session_usage.rs` | 2228 | `SyncCursor` / `load_sync_cursors` / `update_sync_state` / `metadata_modified_nanos` |
| `src-tauri/src/services/usage_stats.rs` | 5739 | `DedupKey` / `should_skip_session_insert` / 代理行指纹去重窗口 |
| `src-tauri/src/services/sql_helpers.rs` | 214 | `INPUT_TOKEN_SEMANTICS_*` 常量与 `fresh_input_sql` 归一规则 |
| `src-tauri/src/database/schema.rs` | 3987 | `proxy_request_logs` / `session_log_sync` / `session_usage_dedup` 建表与默认值 |
| `src-tauri/src/config.rs`、`src-tauri/src/settings.rs` | 1077 / 1289 | `get_home_dir()` 与 Pi 覆盖目录解析 |

---

## 1. A05 OpenCode

### 1.1 各 OS 默认路径与环境变量 / XDG 规则

`get_opencode_db_path()` 的三级优先级（`opencode_config.rs:73-89`）：

1. `OPENCODE_DB` 环境变量（`:77-86`）。非空才生效；绝对路径原样使用（`:80-82`），相对路径拼到数据目录下（`:84`）。
2. `XDG_DATA_HOME`（非空）→ `$XDG_DATA_HOME/opencode`（`opencode_config.rs:91-97`）。
3. 默认 `~/.local/share/opencode`（`opencode_config.rs:99-105`），库文件名固定 `opencode.db`（`:88`）。

**Windows 与 Unix 无差异**，源码有明确注释说明这是有意为之：

- `opencode_config.rs:99-100`：「OpenCode 使用 xdg-basedir，不遵守 macOS/Windows 平台约定，所有平台默认都落在 `~/.local/share/opencode`」。
- 因此 Windows 上默认是 `%USERPROFILE%\.local\share\opencode\opencode.db`（`get_home_dir()` = `dirs::home_dir()`，`config.rs:22-31`），**不是** `%APPDATA%` / `%LOCALAPPDATA%`。
- `get_home_dir()` 的 Windows 行为见 `config.rs:13-16` 注释：使用 `SHGetKnownFolderPath(FOLDERID_Profile)`，明确**不**直接读 `HOME`（避免 Git/Cygwin/MSYS 注入）。测试/调试可用 `CC_SWITCH_TEST_HOME` 短路（`config.rs:23-25`）。

另有一个**易混淆点**：`get_opencode_dir()` 是配置文件目录 `~/.config/opencode`（`opencode_config.rs:49-57`），与数据库所在的数据目录**不是同一个**；数据库路径只走 `get_opencode_data_dir()`（`:91-105`）。

### 1.2 存储格式与 schema 探测

- 数据源是单个 SQLite 文件（`:3`、`:7`）。
- 存储布局枚举 `OpenCodeSchema { V1, V2 }`（`:52-56`），探测函数 `detect_schema`（`:58-66`）：`session_v2` 与 `session_message` **同时存在**才判 V2，否则回退 V1（`detect_schema` 用的是 `sqlite_master` 查询，`sqlite_table_exists`，`:68-75`）。
- 表名映射（`query_sessions:221-228`）：
  - V1：会话表 `session`，消息表 `message`；
  - V2：会话表 `session_v2`，消息表 `session_message`。
- 优先规则（`:14-15`、`:48-51`）：「V2 把会话数据迁移到 `session_v2` / `session_message` 并冻结 V1 表，因此检测到 V2 表时优先读取 V2；未迁移的库继续走 V1」。即**不做合并读取**，V2 存在时完全不读 V1（避免双读冻结表）。
- 消息行差异（`query_assistant_messages:265-274`）：
  - V1：`SELECT id, data, NULL FROM message WHERE session_id = ?1 ORDER BY time_created`，再按 `data.role == "assistant"` 过滤（`:302-306`）。
  - V2：`SELECT id, data, type FROM session_message WHERE session_id = ?1 AND type IN ('assistant','compaction') ORDER BY time_created`——**按 `type` 列过滤，不再看 role**（`:295-296` 注释）。
- 自测印证：`test_detect_schema_prefers_v2_when_present:669-681`、`test_query_sessions_v2_uses_message_update_watermark:721-740`。

### 1.3 usage 字段映射

`parse_message_data`（`:341-401`）从 `message.data` JSON 取值：

| 目标 | 来源 | 行 |
| --- | --- | --- |
| `input_tokens` | `tokens.input` | `:344` |
| `output_tokens` | `tokens.output` | `:345` |
| `reasoning_tokens` | `tokens.reasoning` | `:346-349` |
| `cache_read_tokens` | `tokens.cache.read` | `:351-355` |
| `cache_write_tokens` | `tokens.cache.write` | `:356-359` |
| `cost` | `cost`（`as_f64()`，取不到为 0.0） | `:371` |
| `model_id` | V1 `modelID`（字符串）；V2 `model.id` 或 `model` 为字符串 | `:373-384` |
| 时间戳 | `time.created`（毫秒） | `:386-390` |

**input 是否含 cache**：落库时**不**包含。`insert_opencode_message:422-424` 注释「OpenCode 使用 Anthropic 风格：input 是新鲜输入，cache 单独计」，`cache_read_tokens` / `cache_write_tokens` 分别写入 `cache_read_tokens` / `cache_creation_tokens` 两列（`:505-506`）。间接印证来自上游测试夹具（`:538-546`）：`total=56554`，而 `input 3272 + output 383 + reasoning 419 + cache.read 52480 + cache.write 0 = 56554`，说明上游 `input` 与 cache 是并列相加关系（本仓库只能看到夹具，见文末待核实项）。

**output 与 reasoning：相加，非互斥**。`:424`：`let output_with_reasoning = msg.output_tokens + msg.reasoning_tokens;`，随后 `output_tokens` 列写入 `output_with_reasoning`（`:504`，参数表 `params!` 第 7 个）。注释理由（`:423`）：「output 包含 reasoning tokens（按输出计费）」。

**全零消息被丢弃**（`:361-369`）：input/output/reasoning/cache read/cache write 全为 0 时返回 `None`。解析前还要求存在 `tokens` 字段，否则跳过（`:308-311`）。

未完成消息的判定（`:313-327`）：`compaction` 行要求 `status ∈ {completed, failed}`；其他（assistant）行要求存在 `time.completed`。未完成则**跳过该消息并要求下轮重试**（`:324-327`）。

### 1.4 更新语义与游标

**消息行会被后续更新，因此不能用 rowid 增量**——但上游并没有用 rowid，而是「会话级 `time_updated` 水位 + 逐消息 `request_id` 去重」：

- 会话水位（`query_sessions:229-236`）：
  `MAX(s.time_updated, COALESCE(MAX(m.time_updated), s.time_updated)) AS sync_watermark`，`GROUP BY s.id ORDER BY sync_watermark`。即**用 `time_updated`（不是 version）**，并把该会话所有消息的 `time_updated` 最大值也纳入。
- 会话级游标键 `format!("{db_path}:{session_id}")`，与水位比较后决定跳过（`:144-148`）；同步成功后写回水位（`:191`）。水位不前进时整会话跳过。
- 文件级游标键 = 数据库路径本身（`:201`，`:108`），值是**mtime 纳秒**（`metadata_modified_nanos`，`session_usage.rs:952-959`）。
- **WAL 特殊处理**（`:94-105`）：源码注释说明 opencode.db 运行在 WAL 模式，新提交先落 `-wal` 文件，主库只在 checkpoint 时更新；因此取 `db_path` 与 `db_path.with_extension("db-wal")`（即 `opencode.db-wal`）两者 mtime 的**最大值**（`:102-105`）。
- 文件未变化即整体早退（`:111-120`）。
- 游标推进的失败保护：会话写状态失败 → `has_sync_errors = true`（`:181-184`）；会话有未完成 usage → `continue` 不推进该会话游标（`:186-188`）；文件级游标只在**本轮无写错误**时推进（`:199-202`）。

**关键限制**：写消息用 `INSERT OR IGNORE`（`:489-496`），主键 `request_id`。`:313-314` 的注释自己点明：「跳过未完成的消息：进行中只有半截 token，且因 `INSERT OR IGNORE` 无法回填」。**推论（源自代码，非文档承诺）**：一条已导入的 assistant 消息若事后被上游改写 usage，cc-switch 不会校正该已存在行——重跑只会命中 `should_skip_session_insert` 而计入 `skipped`（`:435-437`）。

### 1.5 身份与去重键

- `request_id = format!("opencode_session:{session_id}:{message_id}")`（`:158`）。即身份 = 会话 + 消息 id，天然抵抗「同一会话重扫」。
- 插入前经统一去重判定 `should_skip_session_insert`（`:435`；实现 `usage_stats.rs:615-624`）：
  1. `request_id` 已存在 → 跳过（`usage_stats.rs:620-621`、`:626-630`）；
  2. 或在 `±SESSION_PROXY_DEDUP_WINDOW_SECONDS = 600s`（10 分钟，`usage_stats.rs:317`）窗口内存在指纹匹配的 **proxy** 行（app_type / model / input / output / cache_read / cache_creation / created_at 全等，且 `status_code ∈ [200,300)`）→ 跳过（`usage_stats.rs:634-694`）。`app_type = "opencode"` 时允许 proxy 行 `cache_creation_tokens` 缺失（`:672`）；模型名比较大小写不敏感，且 `unknown` 双向放行（`:658-662`）。
- **压缩摘要**：V2 把上下文压缩请求的用量记在 `type='compaction'` 行，导入器**明确纳入采集**（`:269-272`）；V1 中压缩摘要是 assistant 消息，无需特判（`:262-264`）。`compaction` 的终态用 `status`（completed/failed）而非 `time.completed`（`:316-323`），且**failed 也算**（注释「失败的压缩请求同样产生了计费用量」，`:818`）。自测：`test_query_assistant_messages_v2_includes_completed_compaction:788-856` 断言采到 `c1(completed)`、`c3(failed)`、`a1`，跳过 `c2(pending)`。
- **分叉 / 共享历史**：本适配器**没有**跨会话/跨文件去重；每个会话的消息按 `request_id` 独立入库。同一消息被复制到新会话文件时会以新 `session_id` 生成新 `request_id`，即**视为新事件**（最终仍会被 10 分钟 proxy 指纹窗口兜底，仅当存在对应 proxy 行时）。

### 1.6 费用字段语义

- 上游 `cost` 为**聚合值 f64**，来源字段 `data.cost`（`:371`）。
- `msg.cost > 0.0` 时**原样当总额使用**，四个分项成本全部写 `"0"`（`:440-450`），注释：「opencode 的 cost 是聚合值，无法精确拆分」。
- `msg.cost <= 0.0`（含缺字段的 0.0，也含异常负值）时回退到 cc-switch 自带 `model_pricing` 计算（`:451-487`）；找不到定价则全部为 `"0"`（`:479-485`）。
- 于是「0 = 未知」而不是「0 = 免费」：真实免费模型的 `cost = 0` 会被当未知处理并尝试估算（`:452` 注释「opencode 费用为 0（如免费模型），尝试用 cc-switch 自带的模型定价计算」）。
- 落库单位：`total_cost_usd` 等列为 TEXT，直接存 f64 的 `to_string()`（`:449`）。`cost_multiplier` 固定 `"1.0"`（`:519`）。

### 1.7 SQLite 读取方式

- `opencode_config.rs` 只给路径；导入器以 `rusqlite::Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)` 打开（`:122-125`），注释「打开 opencode 的 SQLite 数据库（只读）」。
- **没有** `immutable=1`、**没有** `PRAGMA query_only`、**没有**显式 `busy_timeout`。
- WAL 是**间接识别**的：只通过比较 `-wal` 文件 mtime 来避免漏读（`:94-105`），未做 checkpoint，也未以任何 flag 声明 WAL。
- 会话消息的读取是全量重扫：一旦某会话水位前进，就重查该会话**全部** assistant/compaction 行（`:154`、`:257-274`），不做行内偏移。跨表 JOIN 只用 `session_id`（`:233`）。
- 对自身（cc-switch）库的写入**没有显式事务**：插入是单条 `INSERT OR IGNORE`（`:489`），会话游标另用 `update_sync_state` 单独写（`:191`），二者非原子。

### 1.8 可疑上游行为清单（OpenCode）

1. **已导入消息无法校正**：`INSERT OR IGNORE` + 稳定 `request_id` 使 usage 事后改写不可反映（`:313-314` 注释自认「无法回填」）。
2. **文件级 mtime 门会吞掉重试**：会话因「未完成 usage」被跳过时**不推进该会话游标**（`:186-188`），但文件级游标在无写错误时仍会推进（`:199-202`）。下一轮文件未变化就直接早退（`:111`），因此该会话要等 DB/WAL mtime 再次前进才重试——依赖「消息完成必然带来一次写入」这一隐含前提。
3. **cost 无法拆分**：有上游 cost 时四分项成本恒为 0（`:444-450`），只有总额可信；下游按分项做的成本分析对这些行失效。
4. **`cost <= 0` 被当作未知**：合法零价会被估算覆盖（`:452`），与「未知不等于零」原则冲突。
5. **只读打开 WAL 库的健壮性未在代码中处理**：无 `immutable`、无 `busy_timeout`、无打开失败降级（`:122-125`）；失败即整轮报错（`Err(AppError::Database)`）。
6. **V2 判定是「两表同时存在」，不看内容**：若迁移中途中断（只有 `session_v2` 没有 `session_message`），会静默回退 V1 读取可能已冻结的表（`:58-66`）。
7. **reasoning 直接并入 output**：与 Pi 的做法相反（Pi 完全丢弃 reasoning，见 2.3），跨适配器同口径需自行对齐。
8. **`time.completed` 是「是否计费」唯一判据**（assistant）：若上游改字段名，全部 assistant 消息会被判未完成并永久跳过（`:322`）。

---

## 2. A06 Pi

### 2.1 各 OS 默认路径与环境变量 / XDG 规则

Pi 有**两个独立**的路径概念：agent 目录与会话根目录。

**agent 目录**（`pi_config/mod.rs:34-73`），优先级由高到低：

1. cc-switch 设置项 `pi_config_dir`（`get_pi_override_dir()`，`settings.rs:1013-1019`），经 `resolve_override_path` 解析 `~` / `~/` / `~\`（`settings.rs:778-801`）。命中即用（`pi_config/mod.rs:57`，标签 "Pi settings override"）。
2. 环境变量 `PI_CODING_AGENT_DIR`，同样经 `resolve_override_path`（`:59-62`）。
3. 默认 `~/.pi/agent`（`:41`、`:47`）。

最终必须**绝对路径**，否则返回 `InvalidInput`（`:66-71`；测试 `relative_agent_directory_is_rejected:551-560`）。设置覆盖优先于环境变量（测试 `settings_directory_precedes_the_environment:562-577`）。由此派生 `models.json` / `settings.json`（`:75-81`）。注意 `pi_config/mod.rs` **只管这两个 JSON**，不定义会话文件位置。

**会话根目录**（`session_manager/providers/pi.rs:124-158`），优先级由高到低：

1. 环境变量 `PI_CODING_AGENT_SESSION_DIR`（非空，`:126-134`），经 `classify_configured_session_dir(..., "environment")`。
2. `settings.json` 的 `sessionDir` 字段（`:136-147`，字段定义 `pi_config/mod.rs:31`、`:98`）。
3. 默认 `<pi agent dir>/sessions`，布局为 `ProjectDirectories`（`:149-157`）。

`classify_configured_session_dir`（`:160-196`）：路径经 `resolve_global_session_dir` 展开后必须是**绝对路径**且是**已存在可读的目录**；命中时布局为 **`Flat`**（`:174-177`）——即「配置的 sessionDir 是平铺 JSONL 目录」。无法全局展开时返回 `RequiresProjectContext`，browser 与 importer 都**不枚举**、不猜 cwd（`:78-81`、`:192-194`、`session_files_from_resolution:107-109`）。

`resolve_global_session_dir`（`:198-210`）显式支持两种 home 前缀：`~`、`~/`、**`~\`（Windows 风格）**，`~\` 的处理是本适配器唯一的 Windows/Unix 差异点；未识别的相对路径直接判非绝对并回 `None`（`:209`）。

**文件级筛选**（导入器 `session_usage_pi.rs:182-196`）：

- 必须是普通文件（`symlink_metadata().file_type().is_file()`）且扩展名恰为 `jsonl`，否则报错（`:184-190`）：
  > `Pi 会话路径不是普通 JSONL 文件`。
- 大小上限 `MAX_SESSION_BYTES = 128 * 1024 * 1024`（`pi.rs:20`），超限**报错而非静默跳过**（`:191-196`），使「不完整导入」不会被当成成功（`pi.rs:90-92` 注释）。
- 枚举用 `collect_jsonl_files(root, layout, /* enforce_size_limit */ false)`（`pi.rs:103`）——超限文件仍作为候选，交给导入器报错。
- 其他上限：`MAX_TREE_ENTRIES = 500_000`（`pi.rs:18`，导入器 `:392-397`、`:372-376`）。
- 会话头校验：首个有效 JSON 必须是 `type == "session"`（`:409-414`），且 `id` 必须通过 `is_valid_tree_id`（`is_valid_tree_id:658-667`：非空、`<= MAX_TREE_ID_BYTES = 256`（`pi.rs:19`）、首尾为 ASCII 字母数字、字符集 `[A-Za-z0-9._-]`），否则硬报错（`:415-422`）。

### 2.2 存储格式（含更新语义）

Pi **不是 SQLite**，而是每会话一个 JSONL 文件，正文即「entry 树」。导入器逐行流式解析，**不建树**（对比 browser 会建树）。

**文件修订指纹** `PiFileRevision { modified_nanos, file_size, tail_fingerprint, complete }`（`:102-108`）：

- `tail_fingerprint` = 文件末尾 `REVISION_TAIL_BYTES = 4096` 字节（或全文件）的 SHA-256 前 4 字节，域分隔前缀 `"pi-session-tail-v1"`（`:30`、`:294-317`、`:319-338`）。
- `complete` = 文件最后一个字节是否为 `\n`（`:309`）。
- 整个修订被**编码进 `session_log_sync.last_synced_at`** 这一个 i64（`encoded():110-117`：bit61-63 标记 `0b101`，bit60 = complete，bit32-59 = size，低 32 位 = tail fingerprint；`REVISION_SIZE_MASK = 2^28-1`，`:35`）。解码时校验标记、size 上限、行号范围（`:249-275`）。`:277-292` 同时写正规游标与这个编码值，注释说明该值是 Pi 私有格式、无生产消费者解读。

**游标与增量规则**：

- 修订完全相同 → 整文件跳过（`:202-204`）。
- **仅当**「上一轮 complete == true」且「新 size > 旧 size」且「旧 EOF 处的 4096 字节尾部指纹仍匹配」时，才认定是**纯追加**，从 `(last_line_offset, last_size)` 起点续读（`:206-219`、`:319-330`）。任何一个条件不满足 → **从 0 重扫全文件**（`:218`），重复导入由持久去重账本拦截（`:206-209` 注释：「Any mismatch is a rewrite and must rescan from the header; the durable request ledger makes that safe.」）。
- 行号推进：解析时 `line_number` 单调递增，`session_id` 已确定且 `line_number <= start_after_line` 的行直接跳过（`:398-400`）；`last_complete_line = line_number`（`:449`）；追加路径下需重新 seek 到 `start_at_byte` 并**把 `line_number` 重置为 `start_after_line`**（`:425-431`），否则行号会重复计数。
- 断行（无 `\n` 且 JSON 解析失败）→ `incomplete_tail = true`，停止解析且不推进到该行（`:377-390`、`:401-407`），结果计入 `deferred_files`（`:243-245`）。测试 `valid_unterminated_final_record_is_imported:1314-1331` 显示：**合法且无换行的末行仍会被导入**，此时 `complete=false`，于是下轮会因「非纯追加」而全量重扫。
- 读期间文件被截断 → 报错（`:368-370`）。读取上限与 `snapshot_size` 绑定（`:360-367`），保证解析到的是打开时刻的大小（测试 `parser_stops_at_the_captured_file_size:1289-1312`）。
- 写入是**事务内**的：`unchecked_transaction` → 逐条插入 → 写游标 → `commit`（`:227-242`），游标与数据原子提交。
- 游标预取失败**必须中止本轮**，不得当空表全量重导（`:146-153`，理由见 `session_usage.rs:93-98`：`rollup_and_prune` 会删 30 天前明细，空表回退会导致已剪条目重复累加）。

entry 类型分派（`parse_usage_record:454-473`）：

| 上游 `type` / 子条件 | 内部 kind | usage 取值位置 |
| --- | --- | --- |
| `message` + `message.role == "assistant"` | `assistant` | `message.usage` |
| `message` + `message.role == "toolResult"` | `tool_result` | `message.usage` |
| `compaction` | `compaction` | 顶层 `usage` |
| `branch_summary` | `branch_summary` | 顶层 `usage` |
| 其他 | 忽略（返回 `None`） | — |

注意 **compaction / toolResult 不在 usage 中带 `model`/`provider`**，因此 provider 写占位 `"_pi_session"`、model 写 `"unknown"`（`:502-517`、`:25-26`），进而拿不到模型定价（只影响估算，见 2.6）。

### 2.3 usage 字段映射

| 目标 | 来源 | 行 |
| --- | --- | --- |
| `input_tokens` | `usage.input` | `:483` |
| `output_tokens` | `usage.output` | `:484` |
| `cache_read_tokens` | `usage.cacheRead` | `:485` |
| `cache_write_tokens` | `usage.cacheWrite` → 落 `cache_creation_tokens` | `:486`、`:851` |

`token_count` 只做 `as_u64` + 截断到 u32，取不到为 0（`:597-603`）。

**input 是否含 cache**：落库时声明为**新鲜输入（不含 cache）**。`:852` 写 `input_token_semantics = INPUT_TOKEN_SEMANTICS_FRESH`（值 2，`sql_helpers.rs:32`），即 Pi 的 `input` 与 `cacheRead`/`cacheWrite` 分列相加。自测注释把这条语义写死（`:1053-1054`）：「Pi input is already fresh; cache buckets are priced in addition to all 10 input tokens rather than subtracted from them」，对应断言总额 `0.0000155`（`:1055-1058`）。

**output 与 reasoning：reasoning 被完全丢弃**。`:483-487` 只读 `input` / `output` / `cacheRead` / `cacheWrite` 四个键，**从不读取** `usage.reasoning`。上游自测夹具含 `"reasoning":3`（`:941`），而断言的 `SUM(output_tokens) = 37`（`:971`；`7+4+12+14+0 = 37`）——若 reasoning 被相加应为 40。因此结论是**丢弃**，既不加到 output，也不单独成列。

**零值过滤与失败保留**（`:492-500`）：input/output/cacheRead/cacheWrite **全为 0** 且**无 reported cost** 且**非失败**时丢弃；失败判定 = `assistant` 的 `stopReason ∈ {"error","aborted"}`（`:488-491`）。即「零 token 但失败」的记录会被保留（自测 `imports_all_pi_usage_carriers_with_source_semantics:933-1017` 断言 6 个候选行中导入 5 条）。

### 2.4 身份与去重键

两套 id，均由 SHA-256 生成（`:651-719`）：

- `semantic_id = "pi_session_semantic:" + hex(hash)`（`:700`）。哈希域（`:658-699`）：固定前缀 `"pi-session-semantic-v1"` + kind + entry 时间戳 + message 时间戳 + `message` 的 `provider` / `model` / `responseModel` / `responseId` / `api` / `toolCallId` / `toolName` / `stopReason` / `errorMessage` / `content`（或 `entry.summary`）+ `usage`。`hash_json` 是**规范化**的：对象按键排序、显式写入类型标签与长度前缀（`:721-759`），因此 **JSON 键顺序变化或等值数字的不同写法不会改变 semantic_id**（自测 `stable_entry_id_and_canonical_json_prevent_rewrite_duplicates:1390-1439` 用重排键的 legacy 行验证不重复导入）。
- `request_id = "pi_session:" + hex(...)`，域 = `"pi-session-request-v3"` + kind + **entry 的 `id`** + entry 时间戳（`:702-710`）。entry **没有 `id` 时退化为 `request_id = semantic_id`**（`:711-713`），并用 `has_entry_id = false` 记录（`:717`）。

去重账本 `session_usage_dedup`（表结构 `database/schema.rs:325-334`，主键 `(data_source, request_id)`，索引 `(data_source, semantic_id, has_entry_id)`），`insert_pi_record:761-795`：

1. 先查 `request_id` 是否已存在（`PI_REQUEST_DEDUP_SQL:36-39`，`:762-768`）。
2. 再查 semantic：**有 entry_id 时**用 `PI_LEGACY_SEMANTIC_DEDUP_SQL`（要求 `has_entry_id = 0`，`:44-47`），**无 entry_id 时**用 `PI_SEMANTIC_DEDUP_SQL`（`:40-43`，`:771-779`）。这个不对称正是「Pi 把 v1 原地迁移、给旧行补 generated id」时避免重复导入的机制（自测 `rollup_then_legacy_migration_and_fork_do_not_reimport_history:1163-1248`）。
3. 命中即 `Ok(false)`（计 `skipped`）；否则写账本（`:784-795`）后插 `proxy_request_logs`（`:828-869`）。

该账本是**持久**的，专为「明细行被 rollup 剪掉后仍能去重」设计（`schema.rs:322-323` 注释；自测 `rollup_then_legacy_migration_...:1183`、`:1228` 用 `rollup_and_prune` 后再同步断言不重复）。

### 2.5 分叉 / 共享历史 / 压缩摘要

- **压缩摘要与分支摘要按「各自 entry 的顶层 usage」计费，一次且仅一次**：`compaction` 与 `branch_summary` 是独立 kind（`:470-471`），provider/model 记为 `_pi_session` / `unknown`（`:511-517`），cost 只能取 reported（见 2.6）。
- **不递归嵌套旧消息**。`parse_usage_record` 只读 `entry.get("usage")`（`:470-471`），从不进入 `details`。自测夹具刻意在 compaction 行里塞了 `details.retainedTail[0].message.usage = {input:999,...}`（`:943`），而断言 `SUM(input_tokens) = 37`（`:971`，等于 10+3+11+13+0）——**999 未被计入**，即 retainedTail 内的嵌套旧消息被排除，符合 `docs/research/UPSTREAM_AUDIT.md:46` 的设计输入「不能递归统计嵌套旧消息」。
- **分叉/共享历史靠 semantic_id 跨文件去重**：`forked_history_is_not_imported_twice:1117-1161` 断言父文件与含有相同父历史行的 fork 文件合起来 `imported=2, skipped=1`；并且**父文件缺失时 fork 自己就是完整恢复源**（`:1146-1150` 断言 orphan 场景导入 2）。自测 `distinct_ids_keep_identical_usage_events_separate:1063-1080` 反证：payload 完全相同但 entry id 不同 → 视为两条（`request_id` 不同）。
- **导入器不做分支筛选**（与 browser 不同）：browser 的 `read_tree` 会沿 `parentId` 从最新 entry回溯、只保留活跃链（`pi.rs:362-430`、`:500-502`），把 `branchSummary` / `compactionSummary` 显示为 system 文案（`pi.rs:585-592`）。而**导入器会把被放弃分支上的 usage 也导入**（它只按行遍历并去重）。这是有意的（被放弃分支的请求确实被计费），但「会话树活跃链」与「计费集合」因此不是同一集合，跨适配器对比时需注意。

### 2.6 费用字段语义

`PiCosts { input, output, cache_read, cache_write, total }`（`:49-56`），解析 `usage.cost`（`:605-620`）：

- 每个分量取 `cost.input` / `cost.output` / `cost.cacheRead` / `cost.cacheWrite` / `cost.total`，缺失或不可解析 → `Decimal::ZERO`，并**统一 `.max(Decimal::ZERO)` 截断负值**（`:606-612`）。
- 值接受 JSON 数字**或字符串**（`:622-631`，`Decimal::from_str` 再退 `from_scientific`）。
- `reported()`（`:58-73`）语义：
  - `total = cost.total`（若 `> 0`），否则 `total = 四个分量之和`；
  - 只有当 `total > 0` 才返回 `Some((input, output, cache_read, cache_write, total))`，否则 `None`。
- 落库（`:806-826`）：reported 有值就用 reported（**分项与总额都保留**）；否则用 `find_model_pricing(model)` + `CostCalculator::calculate_for_app("pi", ...)`；两者都没有 → 全 `"0"`。
- 结论：**reported 里的四个分项分列保留**（不像 OpenCode 只能给总额），但 **`total == 0` 会被当未知**，即便 input/output 分量本身为正（因为 `total` 优先且 `total > 0` 才返回）。这是与 AGENTS.md「未知不等于零 / 没有价格不等于免费」冲突的点，已在 `UPSTREAM_AUDIT.md:49` 记录为本项目的改进方向。
- 自测：`zero_reported_cost_uses_fresh_input_pricing:1020-1060` 断言 reported 全 0 时用本地定价算出 `0.0000155`；reported 非 0 时 `imports_all_...:997-1000` 断言 `total_cost_usd = 0.0000255` 原样保留。

### 2.7 SQLite 读取方式

Pi 导入器**不读 SQLite**（纯 JSONL 文本读取，`BufReader` + `read_line`，`:340-349`）。它只对**自身**（cc-switch）库写：`lock_conn!(db.conn)` + `unchecked_transaction`（`:227-230`），事务内插入并推进游标（`:232-242`）——所以本项对本适配器而言更接近「写入语义」：

- 文件读取是 `File::open`（`:302`、`:324`、`:347`），无共享锁、无 mmap；读取期间被截断即报错（`:368-370`）。
- 无 WAL/immutable 概念。唯一的「快照」语义靠自己限定 `snapshot_size`（`:360-367`）。
- 单文件最大 128 MiB（`pi.rs:20`），最多 500k entry（`pi.rs:18`）。

### 2.8 可疑上游行为清单（Pi）

1. **`usage.reasoning` 被静默丢弃**（`:483-487`），而 OpenCode/MCode 都把它并入 output——三个适配器同口径需要本项目自己决定，不能照抄（`UPSTREAM_AUDIT.md:48`）。
2. **reported 0 被当未知**（`PiCosts::reported:58-73`）：合法免费/零价事件会被本地定价覆盖。
3. **`total` 优先级导致分项与总额可自相矛盾**：`cost.total > 0` 但四分量全 0 时，落库为分项全 0 + 总额 = reported（`:61-72`、`:819-826`）。
4. **非 assistant 记录丢失 model**：compaction / branch_summary / toolResult 一律 `model = "unknown"`（`:511-517`），因此 `SUM` 与模型维度统计都失去这些行的模型归属。
5. **导入器不做分支筛选**（对比 browser 的活跃链回溯，`pi.rs:362-430`）：被放弃分支的用量也计入（`UPSTREAM_AUDIT.md:45-46` 已知）。
6. **重扫依赖「全量重扫 + 账本去重」**：任何指纹不匹配（含同大小同 mtime 的中段改写）都会整文件重扫（`:206-219`），大文件（上限 128 MiB）会出现明显开销；正确性由账本保证，代价是性能。
7. **修订值塞进 `last_synced_at`**（`:277-292`）：语义重载，`:284-285` 注释自认「no production consumer interprets last_synced_at」。若其他路径/未来代码解释该列会互踩。
8. **`update_sync_state_on_conn` 用 `INSERT OR REPLACE`**（`session_usage.rs:987-992`）会清掉 `last_byte_offset` / `last_tail_fingerprint`（这两列只被 Claude 路径使用，`session_usage.rs:900-931`），Pi 场景无害但共享函数语义不干净。
9. **零 token + 失败保留**（`:492-500`）：会产出 `input=0` 的失败行，`status_code ∈ {499,500}`（`:525-552`）；对「失败也占额度」的展示是好事，但会让「有记录 = 有 token」的假设失效。
10. **`MAX_TREE_ENTRIES` 同时限制「行号」**（`:392-397`）：一个 entry 一行，因此上限即文件行数上限。

---

## 3. A07 MCode（MiniMax Code）

### 3.1 各 OS 默认路径与环境变量 / XDG 规则

**数据目录** `data_dir()`（`mcode_config.rs:12-18`），优先级由高到低：

1. `MINIMAX_DATA_DIR`；
2. `MAVIS_DATA_DIR`；
3. 默认 `~/.minimax`。

`explicit_data_dir`（`:20-27`）对两个环境变量都做 `trim()`，**空白字符串视为未设置**并继续下探（受测于 `:293-304`：`Some(" \t")` 被判空并回退到 MAVIS，`None + Some("")` 回 `None`）。解析结果**不做 `~` 展开、不要求绝对路径**（`:26` 只做 `PathBuf::from`）——若环境变量是相对路径，最终会相对进程 cwd 解析。默认拼 `.minimax` 用的是 `get_home_dir()`（`:17`），因此 Windows 上是 `%USERPROFILE%\.minimax`（`config.rs:22-31`）。

**数据库路径**（`session_manager/providers/mcode.rs:9-11`）：

```
data_dir().join("v2/sqlite/runtime-state.sqlite")
```

即相对子路径固定为 **`v2/sqlite/runtime-state.sqlite`**（`mcode_config.rs` 本身**不含**这段拼接）。同文件 `scan_sessions` 里再次重复同一字面量（`:21`）。

**Windows 与 Unix 差异（本适配器的实际差异在「恢复命令」，不在路径）**（`session_manager/providers/mcode.rs:37-46`）：

- 非 Windows：`env MINIMAX_DATA_DIR=<shell-escaped> mcode`
- Windows：`$env:MINIMAX_DATA_DIR = '<单引号转义>'; mcode`
- 两者都追加 `--session <id>`，且仅当 session id 全部为 `[A-Za-z0-9_-]` 时才给出 resume 命令（`:57-61`）。自测同时断言两种平台的字符串（`:157-182`）。

**数据目录读取路径存在轻微不一致**：`scan_sessions` 用 `std::path::absolute(data_dir())`（`:18`），而 `database_path()` / `sync_mcode_usage` 用原始 `data_dir()`（`:12`、`:16`）。若环境变量给的是相对路径，会话扫描与用量导入会得到不同的 key/路径。

其他：MCode 的 `config.yaml` 也在数据目录下（`mcode_config.rs:59-61`）；`:1` 注释说明 TUI 与桌面共享同一文件与同一本地运行时库（`UPSTREAM_AUDIT.md:47`「不能当三份用量」）。

### 3.2 存储格式与 schema

**不是 JSONL，是 SQLite 投影表**，`local_runtime_token_usage`。导入器一次性读取（`session_usage_mcode.rs:33-37`）以下 **10 列**：

| 顺序 | 列 | 含义 / 用途 |
| --- | --- | --- |
| 0 | `id` | 增量游标主键（见 3.4），同时进 `request_id` |
| 1 | `session_id` | 进 `request_id` 与 `session_id` 列 |
| 2 | `COALESCE(model, 'unknown')` | 模型名，再做 `split_once('/')` 处理 |
| 3 | `ts` | 毫秒时间戳（落库时 `/1000`） |
| 4 | `input_tokens` | |
| 5 | `output_tokens` | |
| 6 | `reasoning_tokens` | 与 output 相加 |
| 7 | `cache_read_tokens` | |
| 8 | `cache_write_tokens` | 落 `cache_creation_tokens` |
| 9 | `cost_usd` | 可为 NULL（`Option<f64>`） |

列清单的**唯一仓库内证据是自测的建表 DDL**（`:104-107`、`:137-140`）：

```sql
CREATE TABLE local_runtime_token_usage (
  id INTEGER PRIMARY KEY, session_id TEXT, model TEXT, ts INTEGER,
  input_tokens INTEGER, output_tokens INTEGER, reasoning_tokens INTEGER,
  cache_read_tokens INTEGER, cache_write_tokens INTEGER, cost_usd REAL)
```

上游真实 schema（是否还有别的列、`id` 是否 `AUTOINCREMENT`、是否有唯一约束或 `updated_at`）**在仓库内未获取**（只有这段测试夹具）。另有 `native_validation` 模块（`:179-205`）被 `#[ignore]`，需在隔离 `CC_SWITCH_TEST_HOME` 下跑真实 MCode 才能校验；该被忽略的测试断言「原生 `SUM(input)`, `SUM(output+reasoning)`, `SUM(cache_read)`, `SUM(cache_write)`」与导入端一致（`:200-203`），是唯一把「source 总量 == 导入总量」写下来的地方，但**未在 CI 中执行**。

会话浏览用的是另一批表：`local_runtime_sessions`（`session_manager/providers/mcode.rs:47-52`：过滤 `visibility <> 'hidden' AND archived = 0 AND parent_session_id IS NULL AND session_kind NOT IN ('peek','channel','cron')`）与 `local_runtime_message_rows` / `local_runtime_message_row_migrations` / `local_runtime_messages`（`:81-96`，有迁移标记时优先读行表、忽略旧的 `display_messages_json` blob）。这些与用量导入无关。

### 3.3 usage 字段映射

`TokenUsage` 组装（`session_usage_mcode.rs:47-55`）：

| 目标 | 来源 | 行 |
| --- | --- | --- |
| `input_tokens` | 列 `input_tokens` | `:48` |
| `output_tokens` | 列 `output_tokens` **+** 列 `reasoning_tokens`（`saturating_add`） | `:49` |
| `cache_read_tokens` | 列 `cache_read_tokens` | `:50` |
| `cache_creation_tokens` | 列 `cache_write_tokens` | `:51` |

**output 与 reasoning：相加，非互斥**（`:49` `row.get::<_, u32>(5)?.saturating_add(row.get(6)?)`）。自测断言 `output_tokens = 23`，来源 `output=20 + reasoning=3`（`:141`、`:151-152`）。

**input 是否含 cache**：落库声明为**新鲜输入**——`:79` 写 `INPUT_TOKEN_SEMANTICS_FRESH`（=2，`sql_helpers.rs:32`），`:5` 显式 `use`。注意 `sql_helpers.rs:23` 的 `CACHE_INCLUSIVE_APP_TYPES = ["codex","gemini","grokbuild"]` **不含 `mcode`**，且 `INPUT_TOKEN_SEMANTICS_FRESH` 行在 `fresh_input_sql` 中直接原样返回（`sql_helpers.rs:56`），所以这套声明是自洽的。**但「本地运行时表的 `input_tokens` 究竟是否已排除 cache」在本仓库内无法验证**，只有测试夹具里的 5 个整数（`:141`），其 `input=10, cache_read=40, cache_write=5` 无法证明包含关系。

**模型名处理**（`:44-46`）：`native_model.split_once('/')` 取**第一个 `/` 之后**的部分。`"custom_provider:router/vendor/model"` → `"vendor/model"`（自测 `:109`、`:129-132` 断言最终 model 为 `"another/model"` 与 `"vendor/model"`，即**丢掉** `custom_provider:router` 段）。

**时间**：`created_at = ts / 1000`（`:79`，整数除法截断毫秒）。**无** 时间合法性 clamp（对比 Pi 有 `MIN/MAX_SQLITE_UNIX_MILLIS` clamp，`session_usage_pi.rs:28-29`、`:519-523`），异常 `ts`（0、负数、溢出）会原样入库。

**无零值过滤**：与 OpenCode / Pi 不同，MCode **没有**任何「全零则丢弃」的判断——`input=0, output=0, cache=0, cost_usd=0` 的行照常入库（`:40-83` 全循环无过滤条件）。

### 3.4 更新语义与游标

- **游标只用 `id`，用 `>` 严格递增**（`:36`：`WHERE id > ?1 ORDER BY id`）。
- 游标读取自**自身**库的 `session_log_sync.last_line_offset`，key = `format!("mcode:{}", mcode::database_path().display())`（`:16-17`、`:28-32`）；`COALESCE(MAX(...), 0)` 缺省 0。
- 每处理一行就把 `cursor = id` 上推（`:83`），最后把 `cursor` 写回（`:86-91`）——**写游标与插入行在同一事务内**（`:27` 开事务、`:92` commit），注释明确动机（`:85`）：「Commit the watermark with the imported rows; pruned logs must never be reimported.」。
- 因此**完全没有更新检测**：既不读 `ts` 做时间水位，也不读任何 `updated_at`/`version`。源码**假设 `id` append-only 且单调**。自测验证的是「已处理 id 不再重导」与「新 id 会被导入」（`:161-175`：删空 `proxy_request_logs` 后重导 0 条，插入 id=2 后导入 1 条）。
- **`id` 是否真的是 append-only 未获取**（见 3.2 的 schema 证据局限）。若上游对同一 `id` 做 UPDATE（补写 usage / 修正确认费用），cc-switch 永远看不到；若上游重建库使 id 变小，则 `id > cursor` 会**永久漏导**这些行（游标不会回退）。这两个方向都无防护。

### 3.5 身份与去重键

- `request_id = format!("mcode:{session_id}:{id}")`（`:67`）→ 落 `proxy_request_logs.request_id`（主键，`database/schema.rs:201`）。
- 唯一防线是 `INSERT OR IGNORE`（`:68-80`）+ 上述 id 游标。**MCode 不使用 `session_usage_dedup` 账本**（整个文件无该表引用），也不使用 `should_skip_session_insert` / 代理行指纹去重（对比 OpenCode 用了，`session_usage_opencode.rs:435`；Pi 用自己的账本）。因此：
  - 与 proxy 直录行之间**没有交叉去重**；若同一 MCode 流量同时被 cc-switch 代理记录，存在双计风险（**推论，未在代码中看到防护**）。
  - 明细行被 `rollup_and_prune` 剪掉后**靠游标**而不是账本防止重导（自测 `:161-175` 覆盖）。
- **无「摘要/压缩」特判**：`local_runtime_token_usage` 只有一个扁平的使用表，没有 compaction/branch 概念，所以也不存在「嵌套旧消息排除」问题（对比 Pi）。
- **无分叉概念**：身份含 `session_id`，同一 `id` 在 `local_runtime_sessions` 里若 `parent_session_id` 非空（子会话）也不会被 `request_id` 区分——但子会话的 usage 行本身带自己的 `session_id`，所以仍能区分（会话浏览侧才过滤父/子，`:48-51`）。

### 3.6 费用字段语义

`cost_usd` 为 `Option<f64>`，判定（`:56-66`）：

```rust
let cost = match native_cost {
    Some(cost) if cost.is_finite() && cost >= 0.0 => cost.to_string(),
    _ => find_model_pricing(&tx, model)
            .map(|p| CostCalculator::calculate_for_app("mcode", &usage, &p, Decimal::ONE)
                      .total_cost.to_string())
            .unwrap_or_else(|| "0".into()),
};
```

- **有值、有限、非负 → 原样使用**，包含**合法的 0.0**。自测 `mcode_usage_preserves_native_zero_cost_and_does_not_reimport_pruned_rows:134-176` 断言 `cost_usd = 0` 最终 `total_cost_usd = "0"`（`:152`），**不做定价回退**。这与 OpenCode（`cost <= 0` 回退，`session_usage_opencode.rs:441`）和 Pi（`total == 0` 回退，`session_usage_pi.rs:66-73`）**相反**——MCode 是三适配器中唯一「合法零值优先于估算」的。
- `NULL` / NaN / Inf / 负数 → 回退本地定价；无定价则 `"0"`。
- **费用不分列**：`input_cost_usd` / `output_cost_usd` / `cache_read_cost_usd` / `cache_creation_cost_usd` 在 SQL 里**硬编码为 `'0'`**（`:75-76`），只有 `total_cost_usd` 有值；且当走定价回退时**只取 `total_cost`**（`:62-64`），分项成本被丢弃。
- 单位：直接写 `f64` 的 `to_string()`（`:58`），列为 TEXT（`database/schema.rs:207-209`）。`cost_multiplier` 固定 `'1'`（`:76`）。

### 3.7 SQLite 读取方式

- `mcode::open_database()`：`Connection::open_with_flags(database_path(), OpenFlags::SQLITE_OPEN_READ_ONLY)`（`session_manager/providers/mcode.rs:13-15`）。源码注释「without modifying its database」（`session_usage_mcode.rs:1`）。
- **没有** `immutable=1`、**没有** `PRAGMA query_only`、**没有** `busy_timeout`、**没有** WAL/`-wal` mtime 处理（对比 OpenCode 有，`session_usage_opencode.rs:94-105`）。若 MCode 的运行时库处于 WAL 模式且未 checkpoint，只读连接能否读到最新提交**未在代码中处理，也未获取官方说明**。
- 读侧只 `prepare` + `query`（`:33-40`），写侧全部在 cc-switch 自己的库上，且包在一个 `conn.transaction()` 里（`:26-27`、`:92`）。
- 存在性预检：库文件不存在时直接返回空结果（`SessionSyncResult::default()`，`:12-14`），不算错误。

### 3.8 可疑上游行为清单（MCode）

1. **无更新检测**：只看 `id > cursor`（`:36`），同一 id 的 UPDATE 永远不可见。
2. **游标不可回退**：源库被替换/重建导致 id 变小 → 那些行永久漏导；无校验、无告警。
3. **零值不过滤 / 时间不 clamp**：全零行入库、`ts/1000` 无合法性检查（`:40-83`、`:79`），异常时间戳会进入 `created_at`。
4. **无跨源去重**：不查 proxy 行、不写 `session_usage_dedup`（对比 OpenCode/Pi）；若 MCode 流量也走 cc-switch 代理，理论双计。
5. **费用分项恒为 0**（`:75-76`），且定价回退只保留总额（`:62-64`）；下游按分项分析失效。
6. **模型名被截断**：`split_once('/')` 丢弃首段（`:44-46`），`custom_provider:router/vendor/model` 与另一个 provider 的 `x/vendor/model` 会折叠成同一个 `vendor/model`，模型维度可能合并不同来源。
7. **环境变量相对路径未规范化**：`data_dir()` 不展开、不强制绝对（`mcode_config.rs:26`），而 `scan_sessions` 私自 `std::path::absolute`（`session_manager/providers/mcode.rs:18`）→ 扫描与导入的路径/key 可能不一致。
8. **真实 schema 只用测试夹具固定**（`:104-107`）：`id` 的 append-only 假设来自测试，不是来自上游证据。
9. **唯一的一致性验证测试被 `#[ignore]`**（`:179-205`），CI 不跑；「source 总量 == 导入总量」在自动化中未受保护。
10. **只读打开无 WAL/锁降级**（`:13-15`）：MCode 正在写库时可能取不到一致快照，行为未定义。

---

## 4. 三适配器横向对照（均为源码事实，非推测）

| 维度 | A05 OpenCode | A06 Pi | A07 MCode |
| --- | --- | --- | --- |
| 数据形态 | SQLite（V1/V2 双布局） | JSONL（每会话一文件） | SQLite（投影表） |
| 路径来源 | `OPENCODE_DB` > `XDG_DATA_HOME` > `~/.local/share/opencode`（全平台一致，`opencode_config.rs:73-105`） | `PI_CODING_AGENT_DIR` / 设置覆盖 > `~/.pi/agent`；会话根另有 `PI_CODING_AGENT_SESSION_DIR` > `settings.json:sessionDir` > `<agent>/sessions`（`pi_config/mod.rs:34-73`、`pi.rs:124-158`） | `MINIMAX_DATA_DIR` > `MAVIS_DATA_DIR` > `~/.minimax`，再拼 `v2/sqlite/runtime-state.sqlite`（`mcode_config.rs:12-27`、`mcode.rs:9-11`） |
| 增量游标 | 文件 mtime（含 `-wal`）+ 会话 `time_updated` 水位 | 文件 mtime + size + 4096B 尾部指纹 + 行号 | 单调整数 `id` |
| 消息级身份 | `opencode_session:{session}:{message_id}` | `pi_session:{hash(entry_id,kind,ts)}` + semantic 哈希 | `mcode:{session_id}:{id}` |
| 去重机制 | `request_id` PK + 10 分钟 proxy 指纹窗口 | 持久账本 `session_usage_dedup`（request + semantic） | 仅 `request_id` PK |
| input 含 cache | 否（声明） | 否（声明 FRESH） | 否（声明 FRESH） |
| output 与 reasoning | **相加**（`:424`） | **丢弃 reasoning**（`:483-487`） | **相加**（`:49`） |
| cache read / write | 分列 | 分列 | 分列 |
| cost 零值 | 回退定价（`:441`） | 回退定价（`:58-73`） | **保留 0**（`:57-58`） |
| cost 分项 | 有 reported 时全 0（`:444-450`） | reported 分项保留（`:819-826`） | 恒为 0（`:75-76`） |
| 嵌套旧消息 | 不适用 | 不递归（`retainedTail` 排除，自测 `:943`/`:971`） | 不适用 |
| SQLite flags | `READ_ONLY` | 不读 SQLite | `READ_ONLY` |

## 5. 待官方版本核实事项

以下**无法**从本仓库固定提交的代码得出，须以官方产品源码或官方产物样本核实后才能写入兼容登记（对应 `docs/research/compatibility-register.md` A05/A06/A07 的「需核实」栏）：

1. **OpenCode `tokens.input` 是否真的不含 cache read/write**：cc-switch 只以注释断言（`session_usage_opencode.rs:422-423`），并以自测夹具的全部为 0 的 `cache.write` 反推（`:538-546`）。OpenCode 官方 usage schema 未获取。
2. **OpenCode V2 的 `session_v2` / `session_message` 真实列名与 `type` 取值全集**：查询侧只用到 `s.id` / `s.time_updated` / `m.session_id` / `m.time_updated` / `m.id` / `m.data` / `m.type` / `m.time_created`（`:229-236`、`:267-274`），自测 DDL 同样只有这八列（`:724-732`）。`type IN ('assistant','compaction')` 是否穷尽可计费类型未获取。
3. **OpenCode 消息行是否会在完成后被改写 `data.tokens` / `data.cost`**：若会，则 `INSERT OR IGNORE` 造成的「不可校正」是真实缺陷；若不会，影响可忽略。上游行为未获取。
4. **只读打开 WAL 库的可行性**：`SQLITE_OPEN_READ_ONLY` 无 `immutable`、无 `busy_timeout`（OpenCode `:122-125`、MCode `mcode.rs:13-15`）在「MCode/OpenCode 正在写库」时是否会 `SQLITE_BUSY` / 读到过期快照，未实测。
5. **Pi `usage.output` 是否已包含 `usage.reasoning`**：导入器丢弃 reasoning（`:483-487`），若上游 `output` 不含 reasoning，则 Pi 的 reasoning token 从未被统计。Pi 官方 usage 定义未获取。
6. **Pi 的 `details.retainedTail` 是否会包含未计费的历史消息**：当前实现按「不递归」处理（自测 `:943`/`:971` 明确排除），需确认这与官方计费语义一致（会不会实际重复计费）。
7. **MCode `local_runtime_token_usage` 真实 DDL**：列全集、`id` 是否 append-only、是否有 `updated_at`/`version`、是否有唯一约束。仓库内只有测试夹具（`session_usage_mcode.rs:104-107`）。
8. **MCode `input_tokens` 是否含 cache、`output_tokens` 是否已含 `reasoning_tokens`**：代码把两者相加并按「互斥」处理（`:49`），官方语义未获取；若上游 output 已含 reasoning，则会双计。
9. **MCode `cost_usd` 的单位与是否已是 USD**：列名是 `cost_usd`（`:35`）但无单位声明，也未获取官方定义。
10. **MCode 的 WAL/checkpoint 行为**：是否存在需要 `-wal` mtime 或 `immutable` 才能读到最新提交（OpenCode 明确需要，MCode 未处理）。
11. **`local_runtime_sessions.session_kind` 的取值全集**：代码排除 `('peek','channel','cron')` 并只取 `parent_session_id IS NULL`（`mcode.rs:48-51`），是否还有别的非对话 kind 未获取。
12. **三个上游 usage 表的字段名稳定性**：如 Pi 的 `cacheRead`/`cacheWrite` 驼峰、OpenCode 的 `modelID`（V1）→ `model.id`（V2）迁移是否有官方文档背书，还是仅由 cc-switch 反向工程。
13. **本机未运行任何 Agent**：以上全部为源码静态阅读，不构成实机兼容证明（`docs/research/UPSTREAM_AUDIT.md:3` 同款声明）。
