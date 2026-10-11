#!/usr/bin/env python3
"""生成 A05 OpenCode V1+V2 并存夹具库。

同时存在 V1 表（session/message，冻结）与 V2 表（session_v2/session_message）。
探测规则：两表同存时只读 V2（证据 1.2），冻结的 V1 行必须被忽略。
合成数据，无真实路径/正文。预期值见 expected.json，算术过程见 README.md。
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

cur.executescript(
    """
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
    """
)

# 冻结的 V1 历史：数值夸张，若被计入会立刻暴露双读。
cur.execute(
    "INSERT INTO session (id, time_created, time_updated) VALUES (?, ?, ?)",
    ("ses-frozen", 1700000000000, 1700000000000),
)
cur.execute(
    "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?, ?, ?, ?, ?)",
    (
        "old-1",
        "ses-frozen",
        1700000000000,
        1700000000000,
        json.dumps(
            {
                "role": "assistant",
                "modelID": "frozen-model",
                "cost": 99.0,
                "tokens": {"input": 9999, "output": 9999, "reasoning": 9999},
                "time": {"created": 1700000000000},
            }
        ),
    ),
)

cur.execute(
    "INSERT INTO session_v2 (id, time_created, time_updated) VALUES (?, ?, ?)",
    ("ses-both", 1760002000000, 1760002060000),
)

rows = [
    (
        "b1",
        "assistant",
        {
            "model": {"id": "both-model"},
            "cost": 0.5,
            "tokens": {
                "input": 100,
                "output": 20,
                "reasoning": 5,
                "cache": {"read": 10, "write": 0},
            },
            "time": {"created": 1760002000000, "completed": 1760002001000},
        },
    ),
    (
        "b2",
        "assistant",
        {
            "tokens": {
                "input": 50,
                "output": 10,
                "reasoning": 0,
                "cache": {"read": 0, "write": 0},
            },
            "time": {"created": 1760002060000, "completed": 1760002061000},
        },
    ),
]

for mid, ty, data in rows:
    created = data["time"]["created"]
    cur.execute(
        "INSERT INTO session_message (id, session_id, type, time_created, time_updated, data) "
        "VALUES (?, ?, ?, ?, ?, ?)",
        (mid, "ses-both", ty, created, created, json.dumps(data, ensure_ascii=False)),
    )

conn.commit()
conn.close()
print(f"wrote {DB_PATH}")
