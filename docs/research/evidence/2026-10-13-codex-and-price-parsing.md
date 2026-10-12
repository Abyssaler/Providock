# Codex 额度与价格解析接线依据（2026-10-13）

## Codex 官方额度

依据 [官方 App Server 文档](https://learn.chatgpt.com/docs/app-server) 及既有 [B07 固定源码证据](accounts-b01-b07-candidates.md)。使用官方 CLI 的 `app-server` stdio JSON-RPC，仅发送 `initialize`、`initialized`、`account/read`（refreshToken=false）、`account/rateLimits/read`。不创建 thread/turn、不驱动工具，不读取、复制或存储 OAuth token。

官方进程自己读取已有登录；仅 ChatGPT 认证支持本轮订阅窗口，API Key 登录不能替代。账户记录跟随本机当前官方登录，不能通过别名连接多个独立 ChatGPT 身份。CLI 发现顺序为 CODEX_CLI_PATH、PATH、已知官方 macOS 应用内路径和常见 CLI 安装路径；找不到、未登录、协议变化分别降级，不从本地 Token 推算额度。

2026-10-13 macOS 实际只读查询通过，返回两个窗口：18000 秒和 604800 秒。验证程序只输出窗口/比例，不打印身份、完整响应或凭据；结果随账户使用变化，不能写作固定额度。Windows 未实测。

## 可选模型辅助解析

用户明确选择程序优先、可选模型辅助。本轮支持已有 DeepSeek、GLM、OpenRouter 的 API Key，端点依据官方文档：

- [DeepSeek Chat Completions](https://api-docs.deepseek.com/api/create-chat-completion/)：POST `https://api.deepseek.com/chat/completions`。
- [DeepSeek Models](https://api-docs.deepseek.com/api/list-models/)：GET `https://api.deepseek.com/models`，读取 data[].id。
- [OpenRouter Chat](https://openrouter.ai/docs/api/api-reference/chat/send-chat-completion-request)：POST `https://openrouter.ai/api/v1/chat/completions`。
- [OpenRouter Models](https://openrouter.ai/docs/api/api-reference/models/list-all-models-and-their-properties)：GET `https://openrouter.ai/api/v1/models`。
- [GLM 中国站官方示例](https://docs.bigmodel.cn/cn/best-practice/case/ai-search-engine)：基础地址 `https://open.bigmodel.cn/api/paas/v4`，chat/completions。
- [Z.AI Chat Completion](https://docs.z.ai/api-reference/llm/chat-completion)：POST `https://api.z.ai/api/paas/v4/chat/completions`。

GLM 模型列表暂不接入，用户直接输入可用模型 ID。Coding Plan 专属密钥不保证能用于付费 API；API 错误只提示失败，不切换渠道或重试。

官网页用独立无认证客户端读取公开 HTTPS；DNS 解析后检查公网地址并固定解析结果，不跟随跳转、执行脚本或带账户 Key。程序识别有明确每百万 Token 单位、单一 USD/CNY、唯一模型行及输入/输出列的静态表格；其它结构、动态网页、歧义/分档不能保证识别。

模型辅助是一次普通非流式 API 请求，最多 3000 输出 Token、60 秒，响应上限 256 KiB。只发送目标模型及最多 60000 UTF-8 字节公开可见文本，没有工具或循环。不将模型 JSON 当指令，校验模型、数值及原文依据后仅填表，不能保证模型正确理解地区/分档，保存前必须由用户核对。未用真实 Key 发起付费推理；凭据隔离与单次请求由合成测试验证。
