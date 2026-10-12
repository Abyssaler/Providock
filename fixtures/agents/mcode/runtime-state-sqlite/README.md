# MCode（MiniMax Code）runtime-state.sqlite 合成夹具

来源：全部手工合成，无真实日志/密钥/正文。
语义依据：`docs/research/evidence/a05-a07-ccswitch-importers.md`「A07 MCode」节
（cc-switch `df40d53c`，`src-tauri/src/services/session_usage_mcode.rs`）。

## 数据源与只读规则

- 数据源：`<MINIMAX_DATA_DIR>/v2/sqlite/runtime-state.sqlite` 的单表
  `local_runtime_token_usage`（表名与列集来自上游自测建表 DDL `session_usage_mcode.rs:104-107`；
  真实 DDL 未获官方证实，见证据第 5 节第 7 条）。
- 列：`id, session_id, model, ts, input_tokens, output_tokens, reasoning_tokens,
  cache_read_tokens, cache_write_tokens, cost_usd`。
- 每个用例目录含：`build.sql`（建库脚本）、`runtime-state.sqlite`（用该脚本生成）、`expected.json`。
- 重新生成：在用例目录内 `rm -f runtime-state.sqlite && sqlite3 runtime-state.sqlite < build.sql`。
- 只读打开规则（解析器 `open_read_only`）：`SQLITE_OPEN_READ_ONLY` + `busy_timeout`；
  **不**加 `immutable`、**不**动 WAL（理由见 `mcode_db.rs` 模块注释）。

## 映射口径（手工计算依据）

```
input_fresh        = input_tokens
input_cache_read   = cache_read_tokens
input_cache_write  = cache_write_tokens
input_total        = input_fresh + input_cache_read + input_cache_write
output_total       = output_tokens            # reasoning 不另加
output_reasoning   = reasoning_tokens         # 仅附属，不进 output_total
total              = input_total + output_total
model_raw          = model 原样（保留全名，含 '/'）
event_id           = "mcode:{id}"；granularity=request；request_count=1
occurred_at        = ts 毫秒 → RFC3339 UTC
源 cost_usd        = v2.0.4 起忽略，不影响 Token 完整性
```

- `ts` 单位取**毫秒**：上游 `created_at = ts / 1000`（`session_usage_mcode.rs:79`）。
  **单位是秒还是毫秒待官方样本确认**；本夹具按毫秒，`1791705600000 ms = 2026-10-11T08:00:00Z`。
- `input_fresh` 走「fresh 语义」是沿用上游的**声明**（`:79` 写 `INPUT_TOKEN_SEMANTICS_FRESH`），
  **上游声明的 input 是否真的已排除 cache 未经官方样本确认**；本夹具只验证映射，不证明语义。

### 与上游的差异（有意为之）

1. **reasoning 不另加**：上游 `output_tokens + reasoning_tokens`（`session_usage_mcode.rs:49`）。
   本适配器 `output_total = output_tokens`、`output_reasoning = reasoning_tokens`。
   上游 output/reasoning 是否互斥未经证实，宁可少加也不重复计；如后续官方确认互斥，
   此处会低估 output。
2. **model 全名保留**：上游 `split_once('/')` 丢首段（`:44-46`）。本适配器不复制。
3. **event_id**：上游 `mcode:{session_id}:{id}`（`:67`）；本适配器 `mcode:{id}`（按任务口径）。
   同库只采一次的规则由调用方保证（`source_instance = 规范化库路径`），故身份只需行 id。
4. **cost 字段忽略**：不读取源费用，不做定价回退。
5. **不丢弃全零行**：上游无零值过滤（`:40-83`），本适配器亦保留（区别于 Kimi `emptyUsage` 兜底）。

## 用例与算术

### basic-multi-row（3 事件）

| id | in | out | reason | cr | cw | cost_usd |
| -- | -- | --- | ------ | -- | -- | -------- |
| 1 | 1000 | 200 | 50 | 300 | 100 | 0.0125 |
| 2 | 500 | 80 | 0 | 0 | 0 | 0.0 |
| 3 | 0 | 0 | 0 | 0 | 0 | NULL |

逐行（fresh=in；input_total=in+cr+cw；output_total=out；total=input_total+out）：
- id1：fresh 1000，read 300，write 100，input_total 1400，output_total 200，reasoning 50，total 1600
- id2：fresh 500，input_total 500，output_total 80，total 580，源 cost 忽略
- id3：全零保留：fresh 0，input_total 0，output_total 0，total 0；源 cost 忽略

合计：fresh **1500**，read **300**，write **100**，input_total **1900**，output_total **280**，
reasoning **50**，total **2180**，requests **3**。
校验：1500+300+100=1900 ✓；1900+280=2180 ✓。
`occurred_at`：`ts` 1791705600000/1000/2000 → 08:00:00 / 08:00:01 / 08:00:02 Z。

### incremental-cursor（2 事件）

配置 `after_id=2, overlap=0`：`WHERE id > 2` → 只读 id 3、4。

- id3：fresh 300，output_total 30，input_total 300，total 330
- id4：fresh 400，output_total 40，input_total 400，total 440

合计：fresh **700**，input_total **700**，output_total **70**，total **770**，requests **2**；max_id **4**。

### overlap-idempotent（重叠回看幂等，6 事件）

配置 `after_id=4, overlap=8`：lower = max(0, 4−8) = 0 → 读到全部 id 1..6。
id 1..4 是已处理行，入库层按 `event_id` 去重；真正新增为 id 5、6。

| id | in | out |
| -- | -- | --- |
| 1 | 10 | 1 |
| 2 | 20 | 2 |
| 3 | 30 | 3 |
| 4 | 40 | 4 |
| 5 | 50 | 5 |
| 6 | 60 | 6 |

- 全部 6 行合计：fresh **210**，input_total **210**，output_total **21**，total **231**，requests **6**
- 仅新增（id 5、6）合计：fresh **110**，input_total **110**，output_total **11**，total **121**，requests **2**
- max_id **6**
校验：10+20+30+40+50+60=210 ✓；50+60=110 ✓。

### reasoning-not-added（1 事件）

id1：in 100，out 20，reason 7。
`output_total = 20`（**不是 27**），`output_reasoning = 7`，input_total 100，total **120**。
反证：若按上游相加，total 应为 127；本夹具期望 120。

### cost-zero-vs-null（3 事件）

| id | cost_usd | 期望 |
| -- | -------- | ---- |
| 1 | 0.0 | 忽略，Token 不变 |
| 2 | NULL | 忽略，Token 不变 |
| 3 | 0.0000125 | 忽略，Token 不变 |

token 均为 in 10 / out 1。合计：fresh **30**，input_total **30**，output_total **3**，total **33**。

### model-full-name（2 事件）

- id1：`custom_provider:router/vendor/model` 原样保留（上游会截成 `vendor/model`）
- id2：`MiniMax-M2.1` 原样保留
合计：fresh **2**，input_total **2**，output_total **2**，total **4**，requests **2**。

## 语义存疑点（待官方样本确认）

1. `local_runtime_token_usage` 真实 DDL：列全集、`id` 是否 append-only、是否有 `updated_at`/唯一约束
   （证据第 5 节第 7 条）。本适配器用 `overlap` 回看缓解 `id` 非单调的风险。
2. `input_tokens` 是否已排除 cache（第 5 节第 8 条）。
3. `output_tokens` 是否已含 `reasoning_tokens`（第 5 节第 8 条）：若是，则本适配器少计 reasoning。
4. `cost_usd` 单位与是否已是 USD（列名 `cost_usd`，无单位声明，第 5 节第 9 条）。
5. `ts` 单位是秒还是毫秒（本夹具按毫秒）。
6. MCode 运行时库的 WAL/checkpoint 行为（第 5 节第 10 条）：未加 `immutable` 是否足够读到最新提交，未实测。
