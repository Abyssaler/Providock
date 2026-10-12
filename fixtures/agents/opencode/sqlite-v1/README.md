# sqlite-v1：OpenCode V1 布局夹具

输入：`input/opencode.db`（由 `generate.py` 生成）。V1 布局：会话表 `session`，消息表 `message`。

## 建库 SQL

```sql
CREATE TABLE session (
    id TEXT PRIMARY KEY,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL
);
CREATE TABLE message (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL,
    data TEXT NOT NULL
);
```

会话 `ses-v1-a`。消息 `data` 为 JSON，`tokens` 见证据 1.3。

## 逐消息算术

映射规则：`input_fresh = input`、`cache_read = cache.read`、`cache_write = cache.write`、
`input_total = input + cache.read + cache.write`、`output_total = output + reasoning`、
`total = input_total + output_total`。

| id | role | input | output | reasoning | cache.read | cache.write | cost | input_total | output_total | total |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| m1 | assistant | 1000 | 100 | 20 | 300 | 50 | 0.05 | 1000+300+50 = **1350** | 100+20 = **120** | **1470** |
| m2 | assistant | 200 | 30 | 0 | 500 | 25 | 0.0 | 200+500+25 = **725** | 30+0 = **30** | **755** |
| m3 | assistant | 10 | 5 | 100 | 0 | 0 | 无 | 10+0+0 = **10** | 5+100 = **105** | **115** |
| m4 | user | 5 | — | — | — | — | — | — | — | 跳过 |

- m1：`reasoning=20 > 0` → tokens quality = `derived`。
- m2：`reasoning=0` → `reported`；源 `cost = 0.0` 忽略。
- m3：源 `cost` 字段忽略；`reasoning=100 > 0` → `derived`。
- m4：V1 仅取 `role == "assistant"`，跳过。

## 合计（3 事件）

- input_fresh = 1000 + 200 + 10 = **1210**
- input_cache_read = 300 + 500 + 0 = **800**
- input_cache_write = 50 + 25 + 0 = **75**
- input_total = 1350 + 725 + 10 = **2085**（校验 1210+800+75 = 2085 ✓）
- output_total = 120 + 30 + 105 = **255**
- total = 1470 + 755 + 115 = **2340**（校验 2085+255 = 2340 ✓）
- request_count = **3**

费用字段均忽略。

## 消息行更新后重读（expected.json `update_case`）

在库的**临时副本**上执行 `UPDATE message SET data = ?1 WHERE id = 'm2'`，把 m2 的
tokens 改为 `input 555 / output 55 / reasoning 5 / cache.read 5 / cache.write 5`、`cost 0.02`，
再只读重读：

- input_total = 555+5+5 = **565**、output_total = 55+5 = **60**、total = **625**，cost 忽略
- `event_id` 仍为 `opencode:ses-v1-a:m2`（同 message id 替换而非新增），事件总数不变（3）。
