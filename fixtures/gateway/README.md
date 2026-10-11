# 网关 usage 解析合成夹具（四协议）

来源：全部为本仓库手工编写的**合成数据**，不含任何真实用户日志、密钥、Cookie 或正文。
语义依据：docs/research/evidence/a09-desktop-gateway.md（cc-switch `df40d53c` 的
`proxy/usage/parser.rs`、`services/sql_helpers.rs`）、docs/specs/DATA_CONTRACTS.md
第 3/4/5 节、docs/specs/AGENT_ADAPTERS.md A09。
预期值由人工按官方语义独立计算（本 README 给出算术过程），不是 parser 输出反推。

## 目录与用例矩阵

```
fixtures/gateway/<protocol>/<case>/
  input/response.json     # kind = non_stream
  input/stream.sse        # kind = stream
  expected.json
```

| protocol | non-stream | stream-multi-frame | stream-truncated | no-usage | stream-tool-call |
| --- | --- | --- | --- | --- | --- |
| anthropic_messages | ✓ | ✓ | ✓ | ✓ | ✓ |
| openai_chat_completions | ✓ | ✓ | ✓ | ✓ | ✓ |
| openai_responses | ✓ | ✓ | ✓ | ✓ | ✓ |
| gemini_native | ✓ | ✓ | ✓ | ✓ | ✓ |

`expected.json` 字段：`protocol` / `kind` / `input` / `request_id` / `model` /
`quality` / `completeness` / `tokens{}` / `diagnostics_include[]`。
Token 与计数用十进制字符串，未知为 `null`。

## 通用口径

- **输入分列 vs 子集**：Anthropic 把 fresh/read/write 分列，三者互斥，`input_total` = 三者之和
  （上游记 FRESH）。OpenAI/Gemini 的 prompt/input 总量**含** cached 子集，
  故 `input_total` = 源报总量，`cache_read` = cached，`fresh` = 总量 − cached，
  `input_cache_write` 无此概念保持 `null`（不补零）。二者都禁止把 cached 再加一遍。
- **output_reasoning 是子集**：源 output 已含 reasoning 时不另加；Gemini 分列时按证据
  以 `candidates + thoughts` 求输出总量，标 `derived`。
- **流式合并是完成语义**：多帧快照合并成一条 UsageSet，不逐帧累加。
- **断流 / 无 usage**：缺终止帧或上游未上报 usage → `completeness = partial`，
  已解析的量保留；无 usage 时字段全 `null`（禁止落全 0 行）并记 `usage_absent`。

---

## anthropic_messages/

非流式取 `usage{input_tokens, output_tokens, cache_read_input_tokens,
cache_creation_input_tokens}`；流式取 `message_start` 的 input/cache，
`message_delta` 的 `usage.output_tokens` **最后值**（delta 是累计 output 快照，不是增量）；
`message_stop` 为终止帧。

### non-stream/
`input 1000/read 200/write 50`，`output 300`；id `msg_01ABC`。
- input_total = 1000 + 200 + 50 = **1250**；total = 1250 + 300 = **1550**。

### stream-multi-frame/
`message_start` input 800/read 120/write 30（start 的 output=1 忽略）；
两个 `message_delta` output 250、410 → 取最后 **410**（不是 250+410，也不是 start 的 1）。
- input_total = 800 + 120 + 30 = **950**；total = 950 + 410 = **1360**。

### stream-truncated/
`message_start` input 500/read 0/write 0；一个 `message_delta` output 42；**无 message_stop**。
- input_total = 500；total = 500 + 42 = **542**；completeness = **partial**（保留已解析量）。

### no-usage/
`message_start` 无 `usage` 字段 + `message_stop`。
- 全部 Token 字段 `null`；completeness = **partial**；诊断含 `usage_absent:anthropic_messages`。

### stream-tool-call/
含 `tool_use`/`input_json_delta` 帧，走同一 `message_start`/`message_delta` 路径，无额外标记。
`input 1200/read 300/write 0`，`message_delta` output 88。
- input_total = 1200 + 300 + 0 = **1500**；total = 1500 + 88 = **1588**。

---

## openai_chat_completions/

非流式取 `usage{prompt_tokens, completion_tokens, prompt_tokens_details.cached_tokens,
completion_tokens_details.reasoning_tokens}`；流式取**末 chunk** 的 usage
（`stream_options.include_usage`），`data: [DONE]` 为终止帧。

### non-stream/ 与 stream-multi-frame/
`prompt 1000 / cached 400 / completion 250 / reasoning 60`；id `chatcmpl-01/02`。
- fresh = 1000 − 400 = **600**；input_total = **1000**（不是 1400）；
  output_total = 250；output_reasoning = 60（子集，不另加）；total = 1000 + 250 = **1250**。

### stream-truncated/
前两帧 `usage:null`，末帧 `prompt 500 / cached 50 / completion 100 / reasoning 20`，
**无 `[DONE]`**。
- fresh = 500 − 50 = **450**；input_total = 500；total = 500 + 100 = **600**；
  completeness = **partial**（缺终止帧，量保留）。

### no-usage/
有 `[DONE]` 但所有 chunk `usage:null`。
- 全 `null`；completeness = **partial**；`usage_absent:openai_chat_completions`。

### stream-tool-call/
含 `tool_calls` 增量帧，末帧 `prompt 900 / cached 0 / completion 120 / reasoning 0` + `[DONE]`。
- fresh = 900 − 0 = **900**；input_total = 900；total = 900 + 120 = **1020**；
  read 与 reasoning 的显式 0 保留为 `"0"`（明确零，非未知）。

---

## openai_responses/

非流式 `response.usage{input_tokens, input_tokens_details.cached_tokens, output_tokens,
output_tokens_details.reasoning_tokens}`；流式取 `response.completed` 内
`response.usage` 的最终快照（该事件即终止帧）。

### non-stream/ 与 stream-multi-frame/
`input 2000 / cached 500 / output 400 / reasoning 150`；id `resp_01/02`。
- fresh = 2000 − 500 = **1500**；input_total = **2000**；total = 2000 + 400 = **2400**。

### stream-truncated/
只有 `response.created` 与输出增量，**无 `response.completed`**。
- 全 `null`；completeness = **partial**；诊断含 `stream_missing_terminal_frame:openai_responses`
  与 `usage_absent:openai_responses`。（本协议 usage 只在终止帧里，故断流即无 usage。）

### no-usage/
`response.completed` 但 `response.usage` 缺失。
- 全 `null`；completeness = **partial**；`usage_absent:openai_responses`。

### stream-tool-call/
含 `function_call` 输出项与参数增量，`response.completed` 带
`input 1200 / cached 300 / output 90 / reasoning 40`。
- fresh = 1200 − 300 = **900**；input_total = 1200；total = 1200 + 90 = **1290**。

---

## gemini_native/

`usageMetadata{promptTokenCount, candidatesTokenCount, cachedContentTokenCount,
thoughtsTokenCount}`；cached ⊂ prompt；`output_total = candidates + thoughts`（**derived**，
Gemini 把 thoughts 与 candidates 分列，按证据相加；thoughts 是 output 的子集，
同时记为 output_reasoning，不再加）。流式取**末 chunk** 的 usageMetadata（累计快照），
`candidates[].finishReason` 出现即终止帧。

### non-stream/ 与 stream-multi-frame/
`prompt 1000 / candidates 150 / thoughts 50 / cached 200 / totalTokenCount 1200`；id `gem_01/02`。
- fresh = 1000 − 200 = **800**；input_total = **1000**；
  output_total = 150 + 50 = **200**；output_reasoning = 50；total = 1000 + 200 = **1200**
  （与源 `totalTokenCount` 一致，作为交叉校验）。
- 流式取末 chunk（candidates 150 + thoughts 50），不是首帧的 10。

### stream-truncated/
末 chunk `prompt 500 / candidates 100 / thoughts 0 / cached 0`，**无 finishReason**。
- fresh = 500 − 0 = **500**；input_total = 500；output_total = 100 + 0 = **100**；
  total = 500 + 100 = **600**；completeness = **partial**（缺终止帧）。

### no-usage/
有 `finishReason:STOP` 但无 `usageMetadata`。
- 全 `null`；completeness = **partial**；`usage_absent:gemini_native`。

### stream-tool-call/
含 `functionCall` part；末 chunk `prompt 1200 / candidates 40 / thoughts 10 / cached 300`。
- fresh = 1200 − 300 = **900**；input_total = 1200；output_total = 40 + 10 = **50**；
  total = 1200 + 50 = **1250**。

---

## 覆盖边界（架构第 7 节）

- 仅保证捕获**经过网关**的请求所报告的 usage；断流或上游不返回 usage 标不完整。
- 上游未上报 usage 时字段全 `null`、completeness=partial——不写全 0 行
  （上游 cc-switch 透传路径落 `TokenUsage::default()` 全 0 行是缺陷，见证据第 5 节）。
- 重试/故障转移逐 attempt 的消耗漏记不在本任务；本任务只解析，不自动重发。
