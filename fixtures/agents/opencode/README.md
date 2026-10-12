# OpenCode 用量夹具（A05）

来源：全部手工合成，无真实日志、数据库、路径或对话正文。
语义依据：`docs/research/evidence/a05-a07-ccswitch-importers.md`「A05 OpenCode」节
（cc-switch 固定提交 `df40d53c36b604e044b4d5b9091aeb961aa56990`）。
预期值由人工按证据与数据契约独立计算（每个用例的算术见其 README），不由 parser 反推。

数据源是**单个 SQLite 文件**（默认 `~/.local/share/opencode/opencode.db`），不是 JSONL。
因此夹具输入是提交进仓库的二进制 `.db`（每个用例 `input/opencode.db`），
由同目录 `generate.py`（仅用 Python 标准库 `sqlite3`）生成；`expected.json` 描述断言。

## 用例

| 目录 | 布局 | 覆盖 |
| --- | --- | --- |
| `sqlite-v1/` | V1（`session` + `message`） | 常规多消息、cache 分列、reasoning 相加、cost 忽略、非 assistant 跳过、消息行更新后重读 |
| `sqlite-v2/` | V2（`session_v2` + `session_message`） | 常规消息、全零 token 行（reported 0）、compaction（completed/failed 有 token 计入、无 token 跳过） |
| `sqlite-v1v2-both/` | V1 冻结行 + V2 并存 | 探测优先 V2，冻结的 V1 行必须被忽略 |

## 通用语义（与解析器 `src-tauri/src/adapters/agents/opencode_db.rs` 对齐）

- **只读**：调用方以 `SQLITE_OPEN_READ_ONLY` 打开，`busy_timeout ≤2s`，禁止 `immutable=1`，
  不 checkpoint、不迁移源库（WAL/冻结表详见证据 1.3–1.4 与 1.7）。
- **版本探测**：`session_v2` 与 `session_message` **同时存在**才判 V2，否则 V1；
  检测到 V2 时**完全不读 V1**（V1 表被上游冻结）。依据证据 1.2。
- **字段映射**（`data.tokens`）：`input_fresh = input`、`input_cache_read = cache.read`、
  `input_cache_write = cache.write`、`input_total = 三者之和`（不补零）；
  `output_total = output + reasoning`、`output_reasoning = reasoning`、
  `total = input_total + output_total`。
- **身份与写入**：`event_id = "opencode:{session_id}:{message_id}"`（原生 id）；
  消息行会被上游后续更新，故每个事件 `replace_existing = true`，调用方重扫时整行覆盖。
- **粒度**：`granularity = message`，`request_count = 1`。
- **时间**：`occurred_at = data.time.created`，**epoch 毫秒**（证据 1.3 `:386-390`）。
  上游只 SELECT `id, data, type`，时间只能取自 `data`，不是表列 `time_created`。
- **费用**：v2.0.4 起忽略 `data.cost`，不影响 Token 完整性。
- 删除检测不做：V1 冻结后不重复导入即可。

## 决策与存疑记录

1. **全零 token 行按 reported 0 处理**（不丢弃）。证据显示 cc-switch 在
   input/output/reasoning/cache.read/cache.write 全为 0 时丢弃该消息
   （`session_usage_opencode.rs:361-369`），但无法区分「源真实写 0」与「未提供」。
   按任务口径取保守的 reported 0，而非丢弃；`sqlite-v2/` 的 `a2` 即此用例。
2. **`output` 是否已含 `reasoning` 未获官方样本确认**（证据第 5 节第 1 条）。
   本夹具与解析器按「`output` 只含可见输出」处理：`output_total = output + reasoning`，
   并在 `reasoning > 0` 时把 tokens 质量标为 `derived`；`reasoning == 0` 时为 `reported`。
   **待官方样本确认**。
3. **compaction 的计入条件**：任务口径要求「有实际 token 值才计入」。本实现为
   `tokens` 对象存在且至少一个分量非零才计入（`completed` / `failed` 都可能有消耗）；
   `tokens` 缺失或全零则跳过并记诊断。若官方口径为「有 `tokens` 字段即计入」，
   需调整此处（`c3` 用例）。
4. **费用字段不再读取**：源 cost 保留在合成输入，验证其不影响 Token。
5. **单条 usage 记录的粒度**：`granularity = message`、`request_count = 1`
   （一条消息一次模型调用的 usage），与 A05 任务口径一致。

## 重新生成

```sh
for d in sqlite-v1 sqlite-v2 sqlite-v1v2-both; do
  python3 fixtures/agents/opencode/$d/generate.py
done
```

建库 DDL 与逐行算术见各用例 README。测试：`cargo test --test opencode_db_fixtures`。
