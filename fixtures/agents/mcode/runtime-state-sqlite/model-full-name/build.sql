-- MCode runtime-state.sqlite 合成夹具：model 全名保留。
-- 上游 cc-switch 用 split_once('/') 丢首段（session_usage_mcode.rs:44-46）；
-- 本适配器不复制该做法，model_raw 保留全名，归一化留给另一层。
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
  (1, 'sess-zeta', 'custom_provider:router/vendor/model', 1791705600000, 1, 1, 0, 0, 0, NULL),
  (2, 'sess-zeta', 'MiniMax-M2.1',                        1791705601000, 1, 1, 0, 0, 0, NULL);
