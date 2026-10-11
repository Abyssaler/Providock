# sqlite-v1v2-both：V1 冻结 + V2 并存夹具

输入：`input/opencode.db`（由 `generate.py` 生成）。
同一库内同时存在 V1 表（`session` / `message`）与 V2 表（`session_v2` / `session_message`）。

## 建库 SQL

```sql
CREATE TABLE session (
    id TEXT PRIMARY KEY, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL
);
CREATE TABLE message (
    id TEXT PRIMARY KEY, session_id TEXT NOT NULL,
    time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL
);
CREATE TABLE session_v2 (
    id TEXT PRIMARY KEY, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL
);
CREATE TABLE session_message (
    id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL,
    time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL
);
```

## 探测预期

`session_v2` 与 `session_message` 同存 → **判 V2**（证据 1.2）。
冻结的 V1 行（会话 `ses-frozen`、消息 `old-1`，数值被夸大以暴露双读）**必须被忽略**；
所有事件 `event_id` 前缀应为 `opencode:ses-both:`。

## V2 逐消息算术（唯一计入来源）

| id | type | input | output | reasoning | cache.read | cache.write | cost | input_total | output_total | total |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| b1 | assistant | 100 | 20 | 5 | 10 | 0 | 0.5 | 100+10+0 = **110** | 20+5 = **25** | **135** |
| b2 | assistant | 50 | 10 | 0 | 0 | 0 | 无 | 50+0+0 = **50** | 10+0 = **10** | **60** |

## 合计（2 事件）

- input_fresh = 100 + 50 = **150**
- input_cache_read = 10 + 0 = **10**
- input_cache_write = 0 + 0 = **0**
- input_total = 110 + 50 = **160**（校验 150+10+0 = 160 ✓）
- output_total = 25 + 10 = **35**
- total = 135 + 60 = **195**（校验 160+35 = 195 ✓）
- request_count = **2**

若误读冻结的 V1 行 `old-1`（input 9999 / output 9999 / reasoning 9999），合计会明显偏大，
因此本用例是「V2 优先、V1 不双读」的回归保护。
