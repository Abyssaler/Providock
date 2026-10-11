-- MCode runtime-state.sqlite 合成夹具：reasoning 不另加到 output。
-- output_tokens=20、reasoning_tokens=7 → output_total=20（不是 27），output_reasoning=7。
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
  (1, 'sess-delta', 'MiniMax-M2', 1791705600000, 100, 20, 7, 0, 0, NULL);
