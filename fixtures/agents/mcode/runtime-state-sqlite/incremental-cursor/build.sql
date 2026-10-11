-- MCode runtime-state.sqlite 合成夹具：增量游标只读新增。
-- 测试配置 after_id=2, overlap=0：应只读到 id 3、4。
-- 重新生成：rm -f runtime-state.sqlite && sqlite3 runtime-state.sqlite < build.sql
DROP TABLE IF EXISTS local_runtime_token_usage;
CREATE TABLE local_runtime_token_usage (
  id INTEGER PRIMARY KEY,
  session_id TEXT,
  model TEXT,
  ts INTEGER,
  input_tokens INTEGER,
  output_tokens INTEGER,
  reasoning_tokens INTEGER,
  cache_read_tokens INTEGER,
  cache_write_tokens INTEGER,
  cost_usd REAL
);

-- id 1、2 是「上轮已处理」，3、4 是本轮新增。
INSERT INTO local_runtime_token_usage
  (id, session_id, model, ts, input_tokens, output_tokens, reasoning_tokens,
   cache_read_tokens, cache_write_tokens, cost_usd)
VALUES
  (1, 'sess-beta', 'MiniMax-M2', 1791705600000, 100, 10, 0, 0, 0, NULL),
  (2, 'sess-beta', 'MiniMax-M2', 1791705601000, 200, 20, 0, 0, 0, NULL),
  (3, 'sess-beta', 'MiniMax-M2', 1791705602000, 300, 30, 0, 0, 0, NULL),
  (4, 'sess-beta', 'MiniMax-M2', 1791705603000, 400, 40, 0, 0, 0, NULL);
