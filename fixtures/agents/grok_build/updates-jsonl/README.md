# Grok Build `updates.jsonl` 合成夹具

来源：全部为本仓库手工编写的**合成数据**，不含真实用户日志、密钥或正文。
语义依据：docs/research/evidence/a01-a04-ccswitch-importers.md「A04 Grok Build」节（cc-switch `df40d53c`）。
预期值由人工按证据语义独立计算（本 README 给出算术过程），不是 parser 输出反推。

## 通用映射

- 只认 `method == "_x.ai/session/update"` 且 `params.update.sessionUpdate == "turn_completed"`
  且带对象 `params.update.usage`、顶层 `timestamp` 为数字 epoch 秒的记录。
  其它 sessionUpdate 或其它 method 即使带 usage 也跳过并记诊断（中途快照与轮末事件并存会双算）。
- **turn_aggregate，不做差分**：`turn_completed` 的 usage 是单回合内多次模型推理的聚合，
  不是会话累计；两个连续回合数字完全相同也**各计一次**，禁止按数值判重。
- 计数分量：`inputTokens` → `input_fresh`，`cachedReadTokens` → `input_cache_read`（两者分列），
  `input_cache_write` 源不提供保持 null；`input_total = inputTokens + cachedReadTokens`。
  `outputTokens` → `output_total`；`reasoningTokens ⊂ outputTokens` → `output_reasoning`
  （**不另加**）；`total = input_total + outputTotal`。
- `request_count = modelCalls`（缺失为 null，不强赋 1）。
- 费用：v2.0.4 起 `costIsPartial` / `costUsdTicks` 均忽略，不影响 Token 完整性。
- 身份：`grok_build:{file_identity}:g{generation}:{会话目录名}:{prompt_id 或 o字节偏移}`。
- 尾行未换行不消费；`consumed_bytes` 停在尾行起点。
- `archived=true` 仅标记来源（`archived_sessions`），身份按会话目录名建立，归档/活跃副本收敛不双算。

## 用例

### two-turns-identical（2 事件）
两轮 usage 数字完全相同（input 1000 / output 200 / cached 400 / reasoning 50 / modelCalls 2），
分别计入，不判重。
- 单轮：fresh 1000，read 400，input_total 1400，output 200，reasoning 50，total 1600
- 合计：fresh 2000，read 800，input_total 2800，output 200×2=400，reasoning 100，total 3200，请求 2×2=4

### model-usage-and-top-fallback（2 事件）
- 轮1 带 `modelUsage`（单模型 `grok-4.5-build`）→ 取该模型计数器，`model_raw="grok-4.5-build"`；
  1000/400/200/50/2 → fresh 1000，read 400，input_total 1400，output 200，reasoning 50，total 1600
- 轮2 无 `modelUsage` → 退回顶层 usage；300/100/30/0/1 → fresh 300，read 100，input_total 400，output 30，total 430，`model_raw=null`
- 合计：fresh 1300，read 500，input_total 1800，output 230，reasoning 50，total 2030，请求 3

### cost-ticks-precision（2 事件）
- 轮1：源 ticks 338_880_000 忽略；
  13793/13696/21/7 → fresh 13793，read 13696，input_total 13793+13696=27489，output 21，total 27510
- 轮2：源 ticks 1 忽略；
  1/0/1/0 → fresh 1，read 0，input_total 1，output 1，total 2
- 合计：fresh 13794，read 13696，input_total 27489+1=27490，output 22，reasoning 7，total 27512，请求 2

### cost-partial（2 事件）
- 轮1：源 ticks 和 `costIsPartial=true` 忽略，Token 仍完整；
  100/20/10/0/1 → fresh 100，read 20，input_total 120，output 10，total 130
- 轮2：源 ticks 忽略，Token 完整；
  200/0/20/0/2 → fresh 200，read 0，input_total 200，output 20，total 220
- 合计：fresh 300，read 20，input_total 320，output 30，reasoning 0，total 350，请求 3

### no-cost-null（2 事件）
两轮无费用字段不影响 Token，事件 completeness=complete。
- 轮1 1000/0/100/0/1 → fresh 1000，input_total 1000，output 100，total 1100
- 轮2 2000/500/200/10/3 → fresh 2000，read 500，input_total 2500，output 200，total 2700
- 合计：fresh 3000，read 500，input_total 3500，output 300，reasoning 10，total 3800，请求 4

### reasoning-subset-not-added（1 事件）
`outputTokens=200`、`reasoningTokens=150`（⊂ output）。
- output_total=200（**不是** 200+150=350），output_reasoning=150
- 100/0/200/150/1 → fresh 100，read 0，input_total 100，total=100+200=**300**（不是 450）

### archived-session-dir（2 事件）
配置 `archived=true`（来自 `archived_sessions`），解析与活跃目录一致，仅记诊断 `archived_sessions`。
- 轮1 1000/0/100/0/1 → fresh 1000，input_total 1000，output 100，total 1100
- 轮2 2000/0/200/0/1 → fresh 2000，input_total 2000，output 200，total 2200
- 合计：fresh 3000，read 0，input_total 3000，output 300，total 3300，请求 2

### truncated-tail（2 事件）
2 条完整记录 + 1 行未写完（以 `INCOMPLETE-LINE` 开头、无换行结尾）。预期 2 事件；
`consumed_bytes` 停在尾行起点（标记 `INCOMPLETE-LINE` 处）。
- 轮1 500/100/50/0/1 → fresh 500，read 100，input_total 600，output 50，total 650
- 轮2 600/200/60/10/2 → fresh 600，read 200，input_total 800，output 60，total 860
- 合计：fresh 1100，read 300，input_total 1400，output 110，reasoning 10，total 1510，请求 3

### non-turn-event-with-usage-skipped（1 事件）
- 行1 `sessionUpdate:"turn_in_progress"` 带 usage → 跳过，诊断 `usage_bearing_non_turn_event_skipped`
- 行2 `method:"_x.ai/session/usage"` 带 usage → 跳过，诊断同上（`...:_x.ai/session/usage:none`）
- 行3 `turn_completed` 但无顶层数字时间戳 → 跳过，诊断 `turn_completed_without_numeric_timestamp`
- 行4 `turn_completed` 且时间戳齐备 → 1 事件：1000/0/100/0/1 → fresh 1000，input_total 1000，output 100，total 1100
- 合计：fresh 1000，read 0，input_total 1000，output 100，reasoning 0，total 1100，请求 1；
  跳过的 usage 事件计数 2

## expected.json 字段

- `config.archived`：是否归档目录来源。
- `events`：事件数。
- `totals`：各 Token 分量、`request_count` 的已知值合计（null = 全部事件该字段未知）。
- `event_expect[]`：逐事件断言（`turn_id`、`model_raw`、`occurred_at`、`granularity`、
  `completeness`）。旧 cost 输入保留以验证被忽略。
- `consumed`：`all` 或 `before_marker:<标记>`（消费字节停在标记处）。
- `diagnostics_include[]`：诊断必须包含的子串。
- `diagnostics_count{}`（可选）：诊断子串出现次数。
