# Claude Code projects JSONL 合成夹具

来源：全部为本仓库手工编写的**合成数据**，不含任何真实用户日志、密钥或正文。
语义依据：docs/research/evidence/a01-a04-ccswitch-importers.md A01 节（cc-switch `df40d53c`）
与 docs/specs/AGENT_ADAPTERS.md 第 3 节 A01。
预期值由人工按 Anthropic 口径独立计算（下列算术过程），**不是** parser 输出反推。

## 通用映射（Anthropic 口径）

- 只取 `type == "assistant"` 且带 `message.usage` 对象的行；其余行忽略。
- `input_tokens` 是 fresh（**不含**缓存读写）；缓存单列：
  - `input_fresh = input_tokens`
  - `input_cache_read = cache_read_input_tokens`（缺失按 0）
  - `input_cache_write = cache_creation_input_tokens`（缺失按 0）
  - `input_total = input_fresh + input_cache_read + input_cache_write`
  - `output_total = output_tokens`；`total = input_total + output_total`
- `cache_creation.ephemeral_1h_input_tokens` 是 cache_write 的 **TTL 子集**，只校验不重复加。
- `granularity = request`，`request_count = 1`；`occurred_at` 用行级 `timestamp` 原样保留。
- 身份 `event_id = claude_code:{sessionId}:{message.id}`；同一 `message.id` 多行只出一条，
  取舍「有 stop_reason 优先，否则 output_tokens 更大」。输出 `replace_existing = true`。
- **不同 message.id 即使数值完全相同也分别计入**（不做按数值判重）。
- 无 usage 的消息、usage 四项全零的消息都不产生事件。
- 日志无自报费用，且本解析器不定价：`cost_reported`/`cost_estimated` 均为 null。

## 用例

### multi-message/（主会话，3 事件）
行：user、system 干扰 + 1 行畸形 JSON（记诊断）+ 3 条 assistant。
- msg_001: in 120, out 45, read 800, write 300 → input_total 1220, total 1265
- msg_002: in 60, out 20, read 1000, write 0 → input_total 1060, total 1080
- msg_003: in 15, out 8, read 0, write 0 → input_total 15, total 23
- 合计：fresh 195，read 1800，write 300，input_total 2295，output 73，total 2368，请求数 3。
  校验：195+1800+300 = 2295 ✓；2295+73 = 2368 ✓
- 诊断含 `malformed_line_at_offset`。

### stream-updates/（同一 message.id 多次更新，3 事件）
- msg_s1：两行（null out 40 / end_turn out 25）→ stop_reason 优先取后者 → in 200, out 25, write 100
  → input_total 300, total 325（即使输出更小也取有 stop_reason 的版本）
- msg_s2：两行都无 stop_reason（out 7 / out 12）→ 取 output 更大者 → in 80, out 12
  → input_total 80, total 92
- msg_s3：单行 tool_use → in 10, out 3 → input_total 10, total 13
- 合计：fresh 290，read 0，write 100，input_total 390，output 40，total 430，请求数 3。
  校验：290+0+100 = 390 ✓；390+40 = 430 ✓（被合并的重复行 2 条 → 诊断 `streaming_updates_merged:2`）

### subagent/（子 Agent 文件，1 事件）
配置 `is_subagent=true`、`subagent_actor_id="agent-7f3a"`；文件内容 `sessionId="sess-parent"`。
- msg_sub1: in 300, out 60, read 100, write 0 → input_total 400, total 460
- 合计：fresh 300，read 100，write 0，input_total 400，output 60，total 460，请求数 1。
- 身份：session_id=sess-parent，parent_session_id=sess-parent，actor_id=agent-7f3a。
  Claude Code 子 Agent 记录沿用父会话 `sessionId`（证据未给独立子会话 id 字段），
  故 parent 与 session 同源，子 Agent 靠 actor_id 区分。

### cache-ttl-subset/（TTL 细分不重复加，2 事件）
- msg_t1: in 100, out 50, read 0, `cache_creation_input_tokens=1000`，
  其 `ephemeral_1h_input_tokens=600`（子集）→ write 记 1000，**不是** 1600
  → input_total 1100, total 1150
- msg_t2: in 20, out 5, read 30, write 0 → input_total 50, total 55
- 合计：fresh 120，read 30，write 1000，input_total 1150，output 55，total 1205，请求数 2。
  校验：120+30+1000 = 1150 ✓；1150+55 = 1205 ✓

### identical-independent/（相同数字独立请求，2 事件）
- msg_i1 与 msg_i2 message.id 不同、usage 数值完全相同（in 100, out 20, read 50, write 0），
  各 input_total 150、total 170。
- 两条都计入：fresh 200，read 100，write 0，input_total 300，output 40，total 340，请求数 2。
  校验：200+100+0 = 300 ✓；300+40 = 340 ✓

### truncated-tail/（尾行未写完，2 事件）
- msg_c1: in 64, out 16, read 0, write 0 → input_total 64, total 80
- msg_c2: in 128, out 32, read 16, write 4 → input_total 148, total 180
- 尾行 `INCOMPLETE-LINE {...` 无换行结尾 → 不消费。
- 合计：fresh 192，read 16，write 4，input_total 212，output 48，total 260，请求数 2。
  校验：192+16+4 = 212 ✓；212+48 = 260 ✓；consumed 停在 `INCOMPLETE-LINE` 之前。

### no-usage-message/（无 usage / 全零不产生事件，1 事件）
- msg_n1 无 `message.usage` → 不产生事件。
- msg_n2 usage 四项全零 → 不产生事件（诊断 `zero_usage_message_at_offset`）。
- msg_n3: in 40, out 10, read 0, write 0 → input_total 40, total 50
- 合计：fresh 40，read 0，write 0，input_total 40，output 10，total 50，请求数 1。
