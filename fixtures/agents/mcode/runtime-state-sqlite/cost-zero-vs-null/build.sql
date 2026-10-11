-- MCode runtime-state.sqlite 合成夹具：合法 cost 0 与 NULL 区分。
-- id=1 cost_usd=0.0 → reported USD "0"（合法零保留）；id=2 NULL → None（未知）；
-- id=3 cost_usd=0.0000125 → reported USD "0.0000125"。
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

INSERT INTO local_runtime_token_usage
  (id, session_id, model, ts, input_tokens, output_tokens, reasoning_tokens,
   cache_read_tokens, cache_write_tokens, cost_usd)
VALUES
  (1, 'sess-eps', 'MiniMax-M2', 1791705600000, 10, 1, 0, 0, 0, 0.0),
  (2, 'sess-eps', 'MiniMax-M2', 1791705601000, 10, 1, 0, 0, 0, NULL),
  (3, 'sess-eps', 'MiniMax-M2', 1791705602000, 10, 1, 0, 0, 0, 0.0000125);
