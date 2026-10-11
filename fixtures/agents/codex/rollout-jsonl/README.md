# Codex rollout JSONL 合成夹具

来源：全部手工合成，无真实日志/密钥/正文。
语义依据：docs/research/evidence/a01-a04-ccswitch-importers.md A02 节（cc-switch `df40d53c`）。
预期值由人工按官方语义独立计算（算术过程见各用例），不由 parser 反推。

## 通用语义

- `input_tokens` 为 TOTAL：含 cached read 与 cache write（互斥子集）。
  fresh = input − cached − write（钳制不取负）；input_total = input_tokens。
- `output_tokens` 为输出；`reasoning_output_tokens` 是输出附属细分，不另加。
- 有 `last_token_usage` 优先直接采用（granularity=request，request_count=1）；
  否则对 `total_token_usage` 高水位差分（granularity=session_delta，request_count=未知）。
- 首快照基线为零；重复快照零增量不发事件；字段倒退=计数器重置：记诊断、重定基线、不发事件。
- fork：子文件前缀与父签名时间线对齐的部分跳过；父时间线不可用 → deferred。
- 相同数字的独立请求按不同事件计入（不按数字判重）。

## 用例

### basic-last-usage（2 事件）
- e1 last: in 1000(cached 300, write 100), out 50 → fresh 600, total 1050
- e2 last: in 500, out 40 → fresh 500, total 540
- 合计：fresh 1100, read 300, write 100, input_total 1500, output 90, total 1590, requests 2

### cumulative-diff（3 事件，无 last_token_usage）
- s1 {1000,300,100,50}（基线零）→ fresh 600
- s2 {1200,300,100,90} → Δ{200,0,0,40} → fresh 200
- s3 同 s2 → 重复快照跳过
- s4 {1350,380,160,110} → Δ{150,80,60,20} → fresh 10
- 合计：fresh 810, read 380, write 160, input_total 1350, output 110, total 1460；
  request_count 全未知（null）。校验：810+380+160=1350 ✓

### counter-reset（3 事件）
- s1 {500,10} → 500/10；s2 {800,30} → Δ300/20
- s3 {100,5} → 倒退=重置：诊断 counter_reset，不发事件，基线重定为 s3
- s4 {260,20} → Δ160/15
- 合计：fresh 960, input_total 960, output 45, total 1005；重置前区间用量未知（不补零）

### fork-replay（1 事件）
- 子 rollout 复制父事件 A{100,0,0,10}、B{200,0,0,20}，后接新事件 C{50,0,0,5}
- 前缀对齐跳过 2 条；合计：fresh 50, input_total 50, output 5, total 55, requests 1
- 诊断含 fork_replay_skipped:2

### deferred-no-parent-timeline
- 声明 forked_from_id 但父时间线不可用 → status deferred，0 事件，不消费游标

### truncated-tail（2 事件）
- e1 last {64,16,0,8} → fresh 48；e2 {128,0,4,32} → fresh 124
- 尾行未写完不消费；合计 fresh 172, read 16, write 4, input_total 192, output 40, total 232

### identical-two-requests（2 事件）
- 两次真实请求的 last_token_usage 完全相同 {70,10,0,5}（各 fresh 60, total 75）
- 两条都计入：fresh 120, read 20, input_total 140, output 10, total 150, requests 2

### replayed-cumulative（2 事件）

同一 total+last 快照在同一或相邻 limit_id 重播，不是新消费。七条 token_count（包含零 last 心跳）只有两次累计推进：每次输入 100（缓存 20）、输出 20，因此总量 240；相同 last 数字但累计推进的两次请求都保留。预期人工计算。
