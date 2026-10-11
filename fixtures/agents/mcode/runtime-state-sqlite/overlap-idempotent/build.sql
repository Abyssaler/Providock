-- MCode runtime-state.sqlite 合成夹具：重叠回看幂等。
-- 测试配置 after_id=4, overlap=8：lower = max(0, 4-8) = 0，应读到全部 6 行（1..6）。
-- 其中 id 1..4 是已处理行，去重由入库层按 event_id 完成；真正新增只有 5、6。
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
  (1, 'sess-gamma', 'MiniMax-M2', 1791705600000,  10, 1, 0, 0, 0, NULL),
  (2, 'sess-gamma', 'MiniMax-M2', 1791705601000,  20, 2, 0, 0, 0, NULL),
  (3, 'sess-gamma', 'MiniMax-M2', 1791705602000,  30, 3, 0, 0, 0, NULL),
  (4, 'sess-gamma', 'MiniMax-M2', 1791705603000,  40, 4, 0, 0, 0, NULL),
  (5, 'sess-gamma', 'MiniMax-M2', 1791705604000,  50, 5, 0, 0, 0, NULL),
  (6, 'sess-gamma', 'MiniMax-M2', 1791705605000,  60, 6, 0, 0, 0, NULL);
