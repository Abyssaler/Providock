# Gemini CLI 会话用量合成夹具

来源：全部手工合成，无真实日志、密钥、完整路径或私密对话。
语义依据：docs/research/evidence/a01-a04-ccswitch-importers.md 的 A03 节（cc-switch `df40d53c`，
`services/session_usage_gemini.rs`、`session_manager/providers/gemini.rs#L75-L122`）。
预期值由人工按官方语义独立计算（算术过程见下），不由 parser 反推，也不照抄 cc-switch 的总数。

对应实现：`src-tauri/src/adapters/agents/gemini_cli.rs`；驱动测试：`src-tauri/tests/gemini_cli_fixtures.rs`。

## 通用语义

形态由调用方决定（`.jsonl` 与同名 `.json` 并存的优先级属路径发现层）：

- **完整 JSON**：`{sessionId, messages[]}`。一次快照、整体覆盖；解析失败或没有 `messages`
  数组时 `consumed_bytes = 0`，不推进游标，下一轮重试。
- **JSONL 回放**：带 `sessionId` 的行是元数据；带字符串 `id` 的行是消息（同 id 原位覆盖、
  保留首次出现顺序）；`{"$set":{...}}` 合并元数据、带 `messages` 时整体替换、带 `id` 时按
  message id 更新（浅合并）；`{"$rewindTo": id}` 删掉该条及之后的消息，找不到 id 时清空。
  回放后得到「当前消息集合」再取用量。尾行未以 `\n` 结尾不消费。

字段映射（`tokens{input, output, thoughts, cached}`；`tool`/`total` 只读不参与校验）：

| 标准字段 | 取值 |
| --- | --- |
| `input_total` | `input`（reported） |
| `input_cache_read` | `cached`（`cached > input` 的异常行钳制为 `input` 并记诊断） |
| `input_fresh` | `input − cached`（钳制不取负） |
| `input_cache_write` | **null**：源不提供写分量；未知 ≠ 0，禁止照抄上游恒写 0 |
| `output_total` | `output + thoughts` |
| `output_reasoning` | `thoughts` |
| `total` | `input_total + output_total`（不读源 `total`） |
| `quality` | `derived`（fresh 是减法、output_total 是加法，不冒称 reported） |
| `completeness` | `complete`（输入总量与输出总量都已知；缺失的写分量不影响任何总量） |

只处理 `type == "gemini"` 的消息。无 `tokens` 字段、`tokens: null`、四项计数全 0 都不产生
事件（不制造零消耗）。

身份与替换：`event_id = "gemini_cli:{sessionId}:{message.id}"`；`message.id` 缺失或为空时
跳过并记诊断（不复制上游兜底 `"unknown"` 撞主键的缺陷）。`$set` 同 id 更新、消息行重复出现、
完整 JSON 快照 → `replace_existing = true`；JSONL 中只写入过一次的消息为 false。
`request_count = 1`，`granularity = message`，`occurred_at` 取消息 `timestamp`（无则 null）。
数字相同的独立消息各自计入，不做数值判重。`$rewindTo` 删掉的历史消息在本层不再产生事件；
已入账的历史事件是否保留属账本层。

### `output` / `thoughts` / `cached` 包含关系的证据级别

级别：**E2 —— 官方 API 字段口径 + 上游源码及其测试算术**，尚未用真实官方 Gemini CLI 会话
文件与 `usageMetadata` 逐字段对照（**待官方样本确认**）。

- Gemini API `usageMetadata`：`promptTokenCount`（输入）、`cachedContentTokenCount`（缓存
  命中，是 `promptTokenCount` 的子集）、`candidatesTokenCount`（可见输出）、
  `thoughtsTokenCount`（思考，与 `candidatesTokenCount` 分离）。
- 上游 A03 测试夹具 `{input:8522, output:29, cached:3138, thoughts:405, total:8956}` 满足
  `8522 + 29 + 405 = 8956`：`total` 不含 cached ⇒ cached ⊂ input，不能再加一次；
  `total` 含 thoughts ⇒ thoughts 在输出侧。本夹具的 `thoughts-in-output` 用例沿用这组数字。
- 待官方样本确认项：`tokens` 四元组与 `usageMetadata` 的一一对应、`thoughtsTokenCount` 的
  计价口径、是否存在 cache 写入分量（确认前保持 `null`，不补 0）。
- 与上游明确不同的地方（AGENT_ADAPTERS A03「不要为对齐上游复制潜在重复加法」）：
  上游把 `cached` 计入 `input_tokens` 却又拿 `cache_creation_tokens = 0` 硬写，
  并丢失 `output`/`thoughts` 的拆分；本解析器拆开保存、写分量留 `null`。

## 用例与算术

每条消息都按 `fresh = input − cached`、`output_total = output + thoughts`、
`total = input + output_total` 手工计算。

### full-json-multi（完整 JSON，3 事件）
- m1 是 `user` 消息，忽略。
- m2 in 1000 cached 300 → fresh 700, read 300；out 50 + thoughts 20 = 70；total 1070
- m3 in 200 cached 0 → fresh 200, read 0；out 10 + 0 = 10；total 210
- m4 in 500 cached 100 → fresh 400, read 100；out 30 + thoughts 40 = 70；total 570
  （`tool: 12` 不在 total 里，故忽略 `tool` 不影响任何数字）
- 合计：fresh 1300, read 400, write null, input_total 1700, output_total 150,
  reasoning 60, total 1850, requests 3

### jsonl-set-rewind（JSONL 回放 + `$set` + `$rewindTo`，3 事件）
- 回放：m1, m2, m3 依次写入；`$set` 按 id 更新 m2；`$rewindTo: m3` 删掉 m3 及之后；再写入 m4
  → 当前集合 = m1、m2（更新后）、m4。三条都标记 `replace_existing`：m1/m4 首次写入为 false，
  m2 被 `$set` 更新为 true。
- m1 in 100 cached 0 → fresh 100, read 0；out 10 + 0 = 10；total 110
- m2 更新后 in 220 cached 60 → fresh 160, read 60；out 22 + thoughts 6 = 28；total 248
- m4 in 400 cached 100 → fresh 300, read 100；out 40 + thoughts 10 = 50；total 450
- m3（in 300, cached 0）被 rewind 掉 → 不产生事件
- 合计：fresh 560, read 160, write null, input_total 720, output_total 88,
  reasoning 16, total 808, requests 3
- 诊断：`rewind_to:m3:removed:1`

### thoughts-in-output（1 事件）
- in 8522 cached 3138 → fresh 5384, read 3138；out 29 + thoughts 405 = 434；total 8956
- 合计：fresh 5384, read 3138, write null, input_total 8522, output_total 434,
  reasoning 405, total 8956, requests 1
- 校验：8956 与源 `total` 相同（源 `total` 本身不读取，仅作交叉验证）

### cached-subset（1 事件）
- DATA_CONTRACTS 第 3 节场景：in 1000 cached 400 → fresh 600, read 400；
  input_total 保持 1000（**不是 1400**）；out 0 + thoughts 0 = 0；total 1000
- 合计：fresh 600, read 400, write null, input_total 1000, output_total 0,
  reasoning 0, total 1000, requests 1

### missing-message-id（4 条消息 → 2 事件）
- n1 in 100 cached 0 → fresh 100；out 10 + 0 = 10；total 110
- 第 2 条无 `id`、第 3 条 `id: ""` → 跳过并记 `missing_message_id`（其 input 999 / 888
  不进入任何合计），不兜 `"unknown"`
- n2 in 200 cached 50 → fresh 150, read 50；out 20 + thoughts 5 = 25；total 225
- 合计：fresh 250, read 50, write null, input_total 300, output_total 35,
  reasoning 5, total 335, requests 2

### truncated-tail（2 事件）
- m1 in 64 cached 16 → fresh 48, read 16；out 8 + 0 = 8；total 72
- m2 in 128 cached 0 → fresh 128, read 0；out 32 + thoughts 4 = 36；total 164
- 尾行 `INCOMPLETE-TAIL{...}`（m3, in 900）未以换行结尾 → 不消费、不产生事件；
  `consumed_bytes` = 该行起点
- 合计：fresh 176, read 16, write null, input_total 192, output_total 44,
  reasoning 4, total 236, requests 2

### no-tokens-message（4 条消息 → 1 事件）
- e1 无 `tokens` 字段、e2 `tokens: null` → `missing_tokens`；e3 四项全 0 → `all_zero_tokens`。
  三条都不产生事件（不制造零消耗）
- e4 in 120 cached 20 → fresh 100, read 20；out 12 + thoughts 3 = 15；total 135
- 合计：fresh 100, read 20, write null, input_total 120, output_total 15,
  reasoning 3, total 135, requests 1

### identical-usage-distinct（3 事件）
- s1/s2/s3 数字完全相同（in 1000, out 100, cached 0, thoughts 0；各 fresh 1000，total 1100），
  id 不同 → 三条分别计入
- 合计：fresh 3000, read 0, write null, input_total 3000, output_total 300,
  reasoning 0, total 3300, requests 3

### cached-exceeds-input（1 事件，异常行）
- in 100 但 cached 400（异常）→ read 钳制为 100，fresh 0，input_total 100；记诊断
  `cached_exceeds_input`。out 10 + thoughts 5 = 15；total 115
- 合计：fresh 0, read 100, write null, input_total 100, output_total 15,
  reasoning 5, total 115, requests 1

### set-messages-replace（JSONL `$set.messages` 整体替换，2 事件）
- r1、r2 写入；`$set.messages` 只带 r2 的新版本 → 当前集合被整体替换为 {r2}，r1 离开当前集合
  （其历史事件是否保留由账本层决定）；随后写入 r3 → 当前集合 = r2（新版本）、r3
- r2 in 260 cached 60 → fresh 200, read 60；out 26 + thoughts 6 = 32；total 292（替换 → true）
- r3 in 50 cached 10 → fresh 40, read 10；out 5 + 0 = 5；total 55（首次写入 → false）
- 合计：fresh 240, read 70, write null, input_total 310, output_total 37,
  reasoning 6, total 347, requests 2
- 诊断：`set_messages_replaced:kept:1:dropped:1`

## 未覆盖边界（待后续任务）

- 真实官方会话样本对照（包含关系与 `thoughtsTokenCount` 计价口径的最终确认）。
- `.jsonl` 与同名 `.json` 并存的路径优先级、文件截短/轮转：属路径与游标层（调用方）。
- `$rewindTo` 后账本是否删除历史事件：属账本层，本解析器只处理当前集合。
- `sessionId` 缺失时用文件身份回退建立身份的行为由单元测试覆盖，未单列夹具。
