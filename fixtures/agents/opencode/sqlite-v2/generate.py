#!/usr/bin/env python3
"""生成 A05 OpenCode V2 夹具库（fixtures/agents/opencode/sqlite-v2/input/opencode.db）。

V2 布局：session_v2 + session_message（按 type 过滤）。合成数据，无真实路径/正文。
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

cur.executescript(
    """
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
    """
)

cur.execute(
    "INSERT INTO session_v2 (id, time_created, time_updated) VALUES (?, ?, ?)",
    ("ses-v2-a", 1760001000000, 1760001240000),
)

rows = [
    (
        "a1",
        "assistant",
        1760001000000,
        {
            "model": {"id": "claude-opus-4"},
            "cost": 1.25,
            "tokens": {
                "input": 500,
                "output": 40,
                "reasoning": 10,
                "cache": {"read": 100, "write": 20},
            },
            "time": {"created": 1760001000000, "completed": 1760001001000},
        },
    ),
    # 全零 token 行：按 reported 0 处理（证据无法区分「真实 0」与「未提供」）。
    (
        "a2",
        "assistant",
        1760001060000,
        {
            "model": {"id": "zero-model"},
            "cost": 0.0,
            "tokens": {
                "input": 0,
                "output": 0,
                "reasoning": 0,
                "cache": {"read": 0, "write": 0},
            },
            "time": {"created": 1760001060000, "completed": 1760001061000},
        },
    ),
    # compaction（completed）带真实 token：计入。
    (
        "c1",
        "compaction",
        1760001120000,
        {
            "status": "completed",
            "tokens": {
                "input": 300,
                "output": 0,
                "reasoning": 50,
                "cache": {"read": 0, "write": 0},
            },
            "time": {"created": 1760001120000},
        },
    ),
    # compaction（failed）同样可能有消耗：带 token 则计入。
    (
        "c2",
        "compaction",
        1760001180000,
        {
            "status": "failed",
            "tokens": {
                "input": 100,
                "output": 20,
                "reasoning": 0,
                "cache": {"read": 10, "write": 5},
            },
            "time": {"created": 1760001180000},
        },
    ),
    # compaction 无 tokens 字段：跳过。
    (
        "c3",
        "compaction",
        1760001240000,
        {"status": "completed", "time": {"created": 1760001240000}},
    ),
    # 非 assistant/compaction：SQL 已排除。
    (
        "u1",
        "user",
        1760001300000,
        {"tokens": {"input": 7}, "time": {"created": 1760001300000}},
    ),
]

for mid, ty, created, data in rows:
    cur.execute(
        "INSERT INTO session_message (id, session_id, type, time_created, time_updated, data) "
        "VALUES (?, ?, ?, ?, ?, ?)",
        (mid, "ses-v2-a", ty, created, created, json.dumps(data, ensure_ascii=False)),
    )

conn.commit()
conn.close()
print(f"wrote {DB_PATH}")
