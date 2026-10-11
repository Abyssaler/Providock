# sqlite-v2：OpenCode V2 布局夹具

输入：`input/opencode.db`（由 `generate.py` 生成）。
V2 布局：会话表 `session_v2`，消息表 `session_message`（按 `type` 过滤，不再看 `role`）。

## 建库 SQL

```sql
CREATE TABLE session_v2 (
    id TEXT PRIMARY KEY,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL
);
CREATE TABLE session_message (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    type TEXT NOT NULL,
    time_created INTEGER NOT NULL,
    time_updated INTEGER NOT NULL,
    data TEXT NOT NULL
);
```

会话 `ses-v2-a`。读取条件：`type IN ('assistant','compaction')`（证据 1.2）。

## 逐消息算术

| id | type | status | input | output | reasoning | cache.read | cache.write | cost | input_total | output_total | total |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| a1 | assistant | — | 500 | 40 | 10 | 100 | 20 | 1.25 | 500+100+20 = **620** | 40+10 = **50** | **670** |
| a2 | assistant | — | 0 | 0 | 0 | 0 | 0 | 0.0 | **0** | **0** | **0** |
| c1 | compaction | completed | 300 | 0 | 50 | 0 | 0 | 无 | 300+0+0 = **300** | 0+50 = **50** | **350** |
| c2 | compaction | failed | 100 | 20 | 0 | 10 | 5 | 无 | 100+10+5 = **115** | 20+0 = **20** | **135** |
| c3 | compaction | completed | — | — | — | — | — | — | — | — | 跳过（无 tokens） |
| u1 | user | — | 7 | — | — | — | — | — | — | — | SQL 排除 |

- a1：`reasoning=10 > 0` → `derived`；cost 1.25 → reported USD。
- a2：全零 token 行，按 **reported 0** 处理（不丢弃，见顶层 README 决策 1）；cost `0.0` 保留为 `"0"`。
- c1 / c2：compaction **有实际 token 值** → 计入；`completed` 与 `failed` 都可能有消耗。
  诊断记 `compaction_counted:c1`、`compaction_counted:c2`。二者无 `cost` → `None`。
- c3：compaction **无 `tokens` 字段** → 跳过，记诊断 `compaction_without_tokens:c3`。
- u1：`type = 'user'`，SQL `type IN ('assistant','compaction')` 排除。

## 合计（4 事件）

- input_fresh = 500 + 0 + 300 + 100 = **900**
- input_cache_read = 100 + 0 + 0 + 10 = **110**
- input_cache_write = 20 + 0 + 0 + 5 = **25**
- input_total = 620 + 0 + 300 + 115 = **1035**（校验 900+110+25 = 1035 ✓）
- output_total = 50 + 0 + 50 + 20 = **120**
- total = 670 + 0 + 350 + 135 = **1155**（校验 1035+120 = 1155 ✓）
- request_count = **4**

费用：`a1 = "1.25"`、`a2 = "0"`、`c1 = None`、`c2 = None`。
