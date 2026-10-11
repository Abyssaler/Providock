# 正文读取器合成夹具（T13）

来源：全部为本仓库手工编写的**合成数据**，不含任何真实用户日志、密钥或正文。
语义依据：`docs/research/evidence/a08-kimi-official.md`（kimi-code `c9f01f07`）、
`docs/research/evidence/a01-a04-ccswitch-importers.md`（cc-switch `df40d53c`，A01/A02 节）、
`docs/specs/UI_UX.md` 第 6 节（分页 / 工具块折叠 / 缺失原因）。

预期值由人工按各源记录形状独立计算（下列算术过程），**不是**读取器输出反推。
每个用例由 `src-tauri/tests/content_readers_fixtures.rs` 驱动，断言：块数、role 序列、
text 是否存在、`truncated` 标志、工具名、可折叠标志、`has_more` 与字节游标（两页取完），
以及诊断原因（只含短码，不含原始 JSON / 正文）。

## 统一模型（`adapters::content`）

- `ContentBlock { role, text, tool_name, is_collapsible_tool, truncated, time }`。
  `role ∈ User | Assistant | ToolUse | ToolResult | System | Other(name)`。
- `list_messages(bytes, cursor, limit) -> MessagePage { blocks, next_cursor, has_more, diagnostics }`。
  **纯读取**：不读文件系统、不访问网络与凭据、不写任何东西，也不解析 usage 做统计。
- `cursor` 是**字节偏移**（行首），`limit` 钳制到 `[1, MAX_PAGE_LIMIT=200]`。
  页按**行原子**切分：一条记录产出的块整条进本页或整条留给下一页，游标始终落在行首。
- 正文长度上限 `MAX_TEXT_CHARS = 4000` 字符，超出即截断并置 `truncated = true`；
  控制字符（保留 `\n`/`\t`）被剔除。

## 用例

### claude_code/mixed（limit=3，两页取完，共 6 块）

`input/session.jsonl` 行序：user 文本 → assistant（text + tool_use）→ user（tool_result）→
assistant（4300 个 `x` 的长文本）→ assistant（thinking 块）。
- 页 1（3 块）：`user`、`assistant`、`tool_use`（tool_name=`Read`，可折叠）；游标指向第 3 行
  （user tool_result）行首，`has_more=true`。
- 页 2（3 块）：`tool_result`（可折叠）、`assistant`（**truncated=true**，截到 4000 字符）、
  `other:thinking`（未映射块类型 → `Other`，并记诊断 `other_content_block_at_offset`）。
- 校验：6 块 = 1+2+1+1+1 ✓；两页取完，末页 `has_more=false`、游标清空。

### claude_code/wrong-shape（缺失原因）

`input/session.jsonl` 只有一行 `type=summary`（非会话消息）。预期 0 块，
诊断含 `no_conversation_records`（文件不是预期形状 → 说明原因，不崩溃、不伪造正文）。

### codex/rollout-items（limit=3，两页取完，共 6 块）

`input/rollout-*.jsonl` 行序：session_meta → turn_context → event_msg(token_count)
→ response_item(message/user) → response_item(message/assistant) → response_item(function_call)
→ response_item(function_call_output) → response_item(reasoning) → response_item(web_search_call)
→ event_msg(user_message)。
- 页 1（3 块）：`user`、`assistant`、`tool_use`（tool_name=`shell`，可折叠）；
  游标指向 function_call_output 行首，`has_more=true`。
- 页 2（3 块）：`tool_result`（可折叠）、`other:reasoning`、`user`（来自 event_msg/user_message）。
- 未知名目 `web_search_call`：跳过并记诊断 `unknown_response_item_type_at_offset`（分组诊断）。
- token_count / session_meta / turn_context 均不产出正文块（统计记录由 `agents::codex_rollout` 负责）。

### codex/wrong-shape（缺失原因）

`input/rollout-*.jsonl` 只有 session_meta。预期 0 块，诊断含 `no_conversation_records`。

### kimi_code/wire-human（limit=3，两页取完，共 6 块）

`input/wire.jsonl` 行序：metadata → usage.record → token_counting.measured → turn.prompt
→ context.append_message(assistant) → context.append_message(tool) → forked
→ agent.message.appended(assistant) → turn.ended → agent.switched。
- 页 1（3 块）：`user`（turn.prompt 的 `prompt`）、`assistant`、`tool_result`
  （tool_name=`Bash`，可折叠）；游标指向 forked 行首，`has_more=true`。
- 页 2（3 块）：`system`（forked → 「此处分叉…」）、`assistant`、`system`
  （agent.switched → 「此处撤销…」）。`turn.ended` 只作回合边界标记记入诊断
  （`turn_boundary_at_offset`），不产出正文块。
- usage.record / token_counting.measured 不产出正文块（统计由 `agents::kimi_wire` 负责）。
- 注：一次 undo 在 wire 里是三条记录（agent.switched + context.undo + context.undone），
  读取器同组只出一个 `System` 块（见 `kimi_content` 单元测试）；本夹具只放 agent.switched。

### kimi_code/wrong-shape（缺失原因）

`input/wire.jsonl` 只有一行 `type=llm.request`。预期 0 块，
诊断含 `not_kimi_wire`（首行不是 metadata）与 `no_conversation_records`。

## 未覆盖 / 已知缺口

- 跨文件活跃链重建（DATA_CONTRACTS 第 4 节）不在本层，见 `content/mod.rs` 的 TODO。
- `context.append_message` / `agent.message.appended` 的正文与角色字段名证据未完全给出，
  读取器按「payload 展开在顶层」取 `role` 与 `content`/`text`/`message`；字段名变化时
  应回到证据修订，不得凭猜测补字段。
