# A10 DeepSeek Harness 官方日志证据

核查日期：2026-10-13。固定官方源码 `deepseek-ai/deepseek-harness@d743267388641bc76f17c45ce8b4c231aed1d32c`。通过 GitHub 只读工具读取指定文件；没有运行 Harness 或读取其凭据。

- [home-paths](https://github.com/deepseek-ai/deepseek-harness/blob/d743267388641bc76f17c45ce8b4c231aed1d32c/packages/util/home-paths/src/index.ts)：`DSH_HOME` > `~/.dsh`；空环境变量忽略。
- [base 配置](https://github.com/deepseek-ai/deepseek-harness/blob/d743267388641bc76f17c45ce8b4c231aed1d32c/packages/bundle/base/cordis.patch.yml)：默认 `sessions` 根。
- [物理格式](https://github.com/deepseek-ai/deepseek-harness/blob/d743267388641bc76f17c45ce8b4c231aed1d32c/packages/session/session-persistence-jsonl/src/format.ts)：`sessions/<project>/<session>/session.vN.jsonl[.zstd]`；默认压缩为 checksummed Zstandard 独立帧。当前逻辑版本 4；同目录保留旧代数，不能把各代累加。
- [事件与版本](https://github.com/deepseek-ai/deepseek-harness/blob/d743267388641bc76f17c45ce8b4c231aed1d32c/packages/core/session/src/types.ts)：头 `type=session,version,id,isSeeded,parentSession?`；事件 `seq,time(epoch ms),type,data`。`assistant/message` 同时带 usage 和 compact stream；`assistant/attempt` 是无表面消息的独立调用。二者均不得递归把流里重复 usage 再计入。
- [计量语义](https://github.com/deepseek-ai/deepseek-harness/blob/d743267388641bc76f17c45ce8b4c231aed1d32c/packages/llm/llm/src/types.ts)：inputTokens 是互斥的新鲜输入，cacheReadTokens/cacheWriteTokens 分列，reasoningTokens 为 output 子集，totalTokens 可选。缺失缓存字段保持未知；明确 total 可用于可靠得到 input_total。
- [流记录](https://github.com/deepseek-ai/deepseek-harness/blob/d743267388641bc76f17c45ce8b4c231aed1d32c/packages/llm/llm/src/assistant-stream.ts)：usage 是 `{type:chunk,chunk:{type:usage,usage:...}}`；一次调用取最后一个 usage，而非累加快照。
- [重试结算](https://github.com/deepseek-ai/deepseek-harness/blob/d743267388641bc76f17c45ce8b4c231aed1d32c/packages/llm/token-meter/src/turn-usage.ts)：每次 assistant settlement 对应独立调用尝试，不将重试相同数字视为重复。
- [压缩调用](https://github.com/deepseek-ai/deepseek-harness/blob/d743267388641bc76f17c45ce8b4c231aed1d32c/packages/compaction/compaction/src/types.ts)：compaction/summary 带独立 usage、model、provider；shadowedTokenCount 是估计上下文量，不当消费。
- [分叉继承](https://github.com/deepseek-ai/deepseek-harness/blob/d743267388641bc76f17c45ce8b4c231aed1d32c/packages/session/session-format-v3-to-v4/README.md)：最后一个 `session/end-seed` 的 `data.inherited=true` 标识继承前缀；marker 自身不属继承。父不可用时延后而非猜测。

实现范围：已核实 v3/v4 的逐事件 JSONL/压缩表示；选择每会话最高物理版本，不回退绕过未知新版本。v0–v2 可能含历史 packed rows，暂不解析，保留已入库数据并报告不兼容。首次接入使用稳定的 session+turn+step+attempt 序号身份；同一 session 的原文件复制及 v3→v4 插入非计量事件不重复计数。支持 compactionId 独立计量。自定义 persistence root、未记录 usage 的第三方插件/辅助请求不推断；不声称覆盖全部服务端账单。尚无 Windows 实机证据。
