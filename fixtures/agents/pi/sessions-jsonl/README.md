# Pi 会话 JSONL 合成夹具（A06）

来源：全部为本仓库手工编写的**合成数据**，不含任何真实用户日志、密钥、项目路径或对话正文。
语义依据：
- docs/research/evidence/a05-a07-ccswitch-importers.md「A06 Pi」节（cc-switch `df40d53c`，
  `src-tauri/src/services/session_usage_pi.rs`）。
- Pi 官方 `packages/coding-agent/docs/session-format.md`（earendil-works/pi），用于核对 entry 形状与字段名。

预期值由人工按语义独立计算（本 README 给出算术过程），并由独立脚本复核；不是 parser 输出反推。

## 通用映射（本项目口径）

文件：每会话一个 JSONL，首行 `type=="session"`（`id` 校验为 `[A-Za-z0-9._-]`、首尾字母数字、≤256 字节）。
之后每条 entry 有 `type`/`id`/`parentId`/`timestamp`（entry 时间戳为 ISO 8601 字符串）。

计费记录（本次采集范围）与 usage 位置：

| entry 类型 | 条件 | usage 位置 | granularity | request_count |
| --- | --- | --- | --- | --- |
| `message` | `message.role=="assistant"` | `message.usage` | message | 1 |
| `compaction` | — | entry 顶层 `usage` | request | 1 |
| `branch_summary` | — | entry 顶层 `usage` | request | 1 |
| `usage` | 独立计量条目（带 `kind`） | entry 顶层 `usage` | request | 1 |

- `message.role=="toolResult"` 不采集；其他 entry 类型忽略。
- 映射：`input_fresh=input`、`input_cache_read=cacheRead`、`input_cache_write=cacheWrite`、
  `input_total=三者之和`、`output_total=output`、`total=input_total+output_total`。
  任一输入分量缺失则 `input_total` 保持未知（不补零）。
- `output_reasoning=reasoning` 仅作**附属细分**保留，不另加进 `output_total`。
  **待官方确认**：reasoning 是否已包含在 output 中（第三方监测工具称是子集，Pi 官方 usage 定义未核实）。
  上游 cc-switch 完全丢弃 reasoning，本项目改为保留附属字段。
- `details.retainedTail` 等嵌套旧消息对象是复制历史，**不递归采集**（上游夹具已证实该语义）。
- 响应 ID 可能复用，不用于身份；身份 = `pi:{file_identity}:g{generation}:o{字节偏移}`。
- 同一文件内重复出现的同一 entry（`id` 相同，覆盖 compaction / branch_summary / usage）只计一次；
  跨文件复制/分叉不在解析器层归并。
- 尾行未换行不消费；畸形行记诊断。
- 四项全零的 usage 仍以「明确的零」产出事件（源确实记录了一次调用）；上游 cc-switch 会丢弃这类记录，
  本项目保留，是否展示由上层决定。
- 官方 `type=="usage"` entry（如 `cache_warm`）计入会话总量；未知 `kind` 也按普通 usage 处理、不丢弃。
  kind 记入诊断：已知 kind（`cache_warm`）记 `usage_kind:{kind}`，未知 kind 记 `usage_kind_unknown:{kind}`，
  每种 kind 只记一次。事件本身不携带 kind（数据契约无此字段），故用诊断留痕。

## 用例（每例一个 `input/<timestamp>_<session-id>.jsonl`）

### assistant-usage（2 事件）
- a1: input 100 + read 30 + write 5 = 135，out 40 → total 175
- a2: input 200 + read 0 + write 0 = 200，out 50 → total 250
- `toolResult` 的 usage（999/999）**不采集**。
- 合计 fresh 300，read 30，write 5，input_total 335，output 90，total 425，请求数 2。

### compaction-usage（3 事件）
- a1: 10/0/0 → in_total 10，out 3 → total 13
- compaction（顶层 usage）: 11/2/1 → in_total 14，out 4 → total 18
- a2: 13/0/0 → in_total 13，out 5 → total 18
- 合计 fresh 34，read 2，write 1，input_total 37，output 12，total 49，请求数 3。

### branch-summary-usage（2 事件）
- a1: 7/0/0 → in_total 7，out 2 → total 9
- branch_summary（顶层 usage）: 5/0/0 → in_total 5，out 1 → total 6
- 合计 fresh 12，read 0，write 0，input_total 12，output 3，total 15，请求数 2。

### retained-tail-not-collected（2 事件）
- a1: 10/0/0 → in_total 10，out 3 → total 13
- compaction 顶层 usage: 3/0/0 → in_total 3，out 3 → total 6
- `details.retainedTail[0].message.usage` 内嵌 999/999/999/999 **不计**（校验：合计不含 999）。
- 合计 fresh 13，read 0，write 0，input_total 13，output 6，total 19，请求数 2。

### reasoning-attached（1 事件）
- usage: input 1000，output 300，reasoning 200。
- output_total = 300（**不**加 reasoning）；output_reasoning = 200；total = 1300。
- 合计 fresh 1000，input_total 1000，output_total 300，output_reasoning 200，total 1300。

### zero-cost-preserved（1 事件）
- usage: 50/0/0，out 10；`cost` 五项全 0。
- 合法 0 保留：cost_reported = 0 USD，quality=reported，completeness=complete。
- 合计 fresh 50，input_total 50，output 10，total 60，请求数 1。

### cost-mismatch-diagnostic（1 事件）
- usage: 100/0/0，out 20；`cost={input:"0.01",output:"0.02",cacheRead:"0",cacheWrite:"0",total:"0.10"}`。
- 分项和 0.03 ≠ 总额 0.10 → 记诊断 `cost_component_total_mismatch`，保留源总额 0.10（不挑大值），
  completeness=partial。
- 合计 fresh 100，input_total 100，output 20，total 120，请求数 1。

### truncated-tail（2 事件）
- a1: 64/16/0 → in_total 80，out 8 → total 88
- a2: 100/0/4 → in_total 104，out 30 → total 134
- 第 5 行未写完（无换行结尾），不消费；`consumed_bytes` 停在最后一个换行之后。
- 合计 fresh 164，read 16，write 4，input_total 184，output 38，total 222，请求数 2。

### identical-requests（2 事件）
- 两次真实请求 usage 完全相同（70/10/0，out 10 → in_total 80，total 90），entry id 不同。
- 两条都计入（不按数字判重）：fresh 140，read 20，input_total 160，output 20，total 180，请求数 2。

### duplicate-compaction（2 事件）
- assistant: 20/0/0，out 5 → total 25
- compaction `id="dead0001"`: 8/0/0，out 2 → total 10；该行在同一文件内**原样重复一次**，只计一次
  （诊断 `duplicate_summary_entry`）。
- 合计 fresh 28，input_total 28，output 7，total 35，请求数 2。

### fork-copies-counted（3 事件）
- 会话头含 `parentSession`，文件内前两行是从父会话复制来的历史。
- 解析器层不做跨文件归并，本文件内 3 条都计入（归并由后续扫描层按可证实祖先处理）。
- a1: 10/1/0 → in_total 11，out 1 → total 12
- a2: 20/2/0 → in_total 22，out 2 → total 24
- a3: 30/3/0 → in_total 33，out 3 → total 36
- 合计 fresh 60，read 6，write 0，input_total 66，output 6，total 72，请求数 3。

### malformed-line-diagnostic（2 事件）
- a1: 5/0/0 → in_total 5，out 1 → total 6
- 第 3 行是非法 JSON，记诊断 `malformed_line_at_offset` 并跳过。
- a2: 9/0/0 → in_total 9，out 2 → total 11
- 合计 fresh 14，read 0，write 0，input_total 14，output 3，total 17，请求数 2。

### usage-kind-entries（4 事件）
- assistant a1: 10/0/0 → in_total 10，out 2 → total 12（granularity message）
- `type=="usage" kind="cache_warm"`: 0/50000/0 → in_total 50000，out 0 → total 50000（granularity request）
- `type=="usage" kind="future_kind"`（未知 kind，不丢弃、照计）: 4/0/0 → in_total 4，out 1 → total 5
- `type=="usage" kind="cache_warm"` 全零: 0/0/0 → in_total 0，out 0 → total 0（「明确的零」仍产出）
- 诊断：`usage_kind:cache_warm`（两条同 kind 只记一次）、`usage_kind_unknown:future_kind`。
- 合计 fresh 14，read 50000，write 0，input_total 50014，output 3，total 50017，请求数 4。
