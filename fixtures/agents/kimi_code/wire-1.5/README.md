# Kimi Code wire.jsonl 合成夹具（协议 1.5 族）

来源：全部为本仓库手工编写的**合成数据**，不含任何真实用户日志、密钥或正文。
语义依据：docs/research/evidence/a08-kimi-official.md（kimi-code `c9f01f07`）。
预期值由人工按官方语义独立计算（本 README 给出算术过程），不是 parser 输出反推。

## 通用映射（官方语义）

- `usage.record` 一次 LLM 请求一条；`inputOther/inputCacheRead/inputCacheCreation` 三者互斥，
  input_total = 三者之和；total = input_total + output。
- `usageScope: turn` 与 `session` 都是独立调用记录，都计入。
- `token_counting.*` 是上下文测量，不计入；undo 三条记录不撤销真实消耗。
- 四项全零 = 上游 emptyUsage 兜底（用量未知），映射为 null 字段 + partial，请求数 = 1。
- `forked` 标记之前的行为继承历史：父会话在采集范围内则跳过；父不可达则计入并记诊断。

## 用例

### basic/
3 次调用（2 个 turn scope + 1 个压缩产生的 session scope），夹杂 turn.prompt、
token_counting.measured、undo 三条干扰记录。
- rec1: 100/40/5, out 30 → input 145, total 175
- rec2: 200/0/0, out 50 → input 200, total 250
- rec3: 1000/400/0, out 100 → input 1400, total 1500
- 合计：fresh 1300，read 440，write 5，input_total 1745，output 180，total 1925，请求数 3。

### zero-usage-fallback/
1 条全零 usage.record → 1 事件，Token 字段全 null，completeness=partial，request_count=1。

### fork-parent-present/ 与 fork-parent-missing/
同一份 wire：2 条继承 usage.record + `forked` 标记 + 1 条新记录。
- 继承 rec1: 10/0/0 out 5；rec2: 20/0/0 out 5；新 rec: 7/3/0 out 2。
- 父在范围内：1 事件，合计 fresh 7, read 3, write 0, input_total 10, output 2, total 12；
  诊断含 fork_inherited_skipped:2。
- 父不可达：3 事件，合计 fresh 37, read 3, write 0, input_total 40, output 12, total 52；
  诊断含 fork_parent_missing。

### truncated-tail/
2 条完整记录 + 1 行未写完（无换行结尾）。预期 2 事件；
consumed_bytes 停在尾行起始（标记 `INCOMPLETE-LINE` 之前一行结束后）。

### migration-context-only/
protocol_version 1.0（迁移产物）：只有 token_counting.measured 等记录，无 usage.record。
预期 0 事件、compatibility ok（上下文测量不是消费，不产生零消耗事件）。

### unknown-protocol/
protocol_version 1.9（未来版本）。预期 0 事件，compatibility = unsupported_protocol，
不猜测字段。
