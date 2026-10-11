-- MCode runtime-state.sqlite 合成夹具：常规多行。
-- 语义依据：docs/research/evidence/a05-a07-ccswitch-importers.md「A07 MCode」节
-- （cc-switch df40d53c，session_usage_mcode.rs 自测建表 DDL）。
-- table 名与列集来自上游自测夹具（session_usage_mcode.rs:104-107），
-- 列顺序与本脚本一致；id/ts 单位为手工假设，见 README。
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

-- 常规三行：正常一行、合法零 cost 一行、全零 token + cost NULL 一行。
INSERT INTO local_runtime_token_usage
  (id, session_id, model, ts, input_tokens, output_tokens, reasoning_tokens,
   cache_read_tokens, cache_write_tokens, cost_usd)
VALUES
  (1, 'sess-alpha', 'MiniMax-M2',     1791705600000, 1000, 200, 50, 300, 100, 0.0125),
  (2, 'sess-alpha', 'MiniMax-M2',     1791705601000,  500,  80,  0,   0,   0, 0.0),
  (3, 'sess-alpha', 'abab6.5s-chat',  1791705602000,    0,   0,  0,   0,   0, NULL);
