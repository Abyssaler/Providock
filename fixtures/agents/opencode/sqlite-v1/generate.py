#!/usr/bin/env python3
"""生成 A05 OpenCode V1 夹具库（fixtures/agents/opencode/sqlite-v1/input/opencode.db）。

只用 Python 标准库 sqlite3；运行后重写 input/opencode.db。合成数据，无真实路径/正文。
预期值见 expected.json，算术过程见 README.md。
"""
import json
import os
import sqlite3

HERE = os.path.dirname(os.path.abspath(__file__))
OUT_DIR = os.path.join(HERE, "input")
os.makedirs(OUT_DIR, exist_ok=True)
DB_PATH = os.path.join(OUT_DIR, "opencode.db")
if os.path.exists(DB_PATH):
    os.remove(DB_PATH)

conn = sqlite3.connect(DB_PATH)
cur = conn.cursor()

# V1 布局：会话表 session，消息表 message（证据 1.2）。
cur.executescript(
    """
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
    """
)

cur.execute(
    "INSERT INTO session (id, time_created, time_updated) VALUES (?, ?, ?)",
    ("ses-v1-a", 1760000000000, 1760000120000),
)

messages = [
    # 常规：input(fresh) 与 cache 并列，reasoning 相加。
    (
        "m1",
        "ses-v1-a",
        1760000000000,
        {
            "role": "assistant",
            "modelID": "claude-sonnet-4-20250514",
            "cost": 0.05,
            "tokens": {
                "input": 1000,
                "output": 100,
                "reasoning": 20,
                "cache": {"read": 300, "write": 50},
            },
            "time": {"created": 1760000000000, "completed": 1760000001000},
        },
    ),
    # 合法 cost = 0 必须保留为 reported 0（上游当未知是缺陷）。
    (
        "m2",
        "ses-v1-a",
        1760000060000,
        {
            "role": "assistant",
            "modelID": "free-model",
            "cost": 0.0,
            "tokens": {
                "input": 200,
                "output": 30,
                "reasoning": 0,
                "cache": {"read": 500, "write": 25},
            },
            "time": {"created": 1760000060000, "completed": 1760000061000},
        },
    ),
    # reasoning 明显大于 output，验证相加。
    (
        "m3",
        "ses-v1-a",
        1760000120000,
        {
            "role": "assistant",
            "modelID": "reason-model",
            "tokens": {
                "input": 10,
                "output": 5,
                "reasoning": 100,
                "cache": {"read": 0, "write": 0},
            },
            "time": {"created": 1760000120000, "completed": 1760000121000},
        },
    ),
    # 非 assistant，必须跳过。
    (
        "m4",
        "ses-v1-a",
        1760000180000,
        {"role": "user", "tokens": {"input": 5}, "time": {"created": 1760000180000}},
    ),
]

for mid, sid, created, data in messages:
    cur.execute(
        "INSERT INTO message (id, session_id, time_created, time_updated, data) "
        "VALUES (?, ?, ?, ?, ?)",
        (mid, sid, created, created, json.dumps(data, ensure_ascii=False)),
    )

conn.commit()
conn.close()
print(f"wrote {DB_PATH}")
