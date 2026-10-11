# 证据 a09：cc-switch 本地代理 / 模型映射的用量捕获（重点：Claude Desktop 模型映射路径）

- 核查日期：2026-10-11
- 固定提交：`df40d53c36b604e044b4d5b9091aeb961aa56990`（cc-switch，版本线 v4.0.8 / main 监测提交）
- 核查方法：用 GitHub Tree API 列出 `src-tauri/src` 全部文件，再经 `raw.githubusercontent.com` 拉取相关源码逐一细读；辅以该提交的 `README_ZH.md` 与 `docs/user-manual/zh/2-providers/2.6-claude-desktop.md`。**未运行 cc-switch、未构造真实请求、未读取用户真实日志或查询账户**，本文只记录「读到的代码行为」，不是实机兼容证明。
- 行号引用格式：`文件#Lx-Ly`，链接指向该固定提交的 GitHub blob（`.../blob/df40d53c.../路径#Lx-Ly`）。

---

## 1. 本地转发监听形式、目标 host、令牌

**监听地址/端口（默认 127.0.0.1:15721）**

- 进程内配置默认值：`ProxyConfig::default()` → `listen_address = "127.0.0.1"`、`listen_port = 15721`。
  [`proxy/types.rs#L42-L55`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/types.rs#L42-L55)
- 数据库 `proxy_config` 表默认值：`listen_address TEXT NOT NULL DEFAULT '127.0.0.1'`、`listen_port INTEGER NOT NULL DEFAULT 15721`；注意该表 `CHECK (app_type IN ('claude','codex','gemini','grokbuild'))`，**不含 `claude-desktop`**。
  [`database/schema.rs#L129-L142`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/database/schema.rs#L129-L142)
- 绑定实际执行：`SocketAddr = format!("{listen_address}:{listen_port}").parse()`，再 `tokio::net::TcpListener::bind(&addr)`；端口允许为 `0`（系统分配），绑定后把实际端口回写。
  [`proxy/server.rs#L108-L126`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/server.rs#L108-L126)、
  [`services/proxy.rs#L160-L175`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/proxy.rs#L160-L175)
- `listen_address` 可为 `0.0.0.0` / `::`（监听所有网卡）；给客户端用的 URL 用 `proxy_origin()` 把它替换成回环地址。
  [`services/proxy.rs#L44-L57`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/proxy.rs#L44-L57)

**路由表**（一个 axum `Router`，同端口多路径）

- `GET /health`、`GET /status`
- `POST /v1/messages`、`POST /claude/v1/messages` → Claude/Claude Code
- `POST /claude-desktop/v1/messages`、`GET /claude-desktop/v1/models` → Claude Desktop 3P 网关（**独立 provider 命名空间**）
- OpenAI Chat/Responses、Gemini、images 等另有路径。
  [`proxy/server.rs#L299-L421`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/server.rs#L299-L421)

**目标 host（上游）如何配置**

- 由 provider 配置决定。Claude 系适配器 `extract_base_url()` 取值优先级：Codex/xAI OAuth 强制端点 → `env.ANTHROPIC_BASE_URL` → `base_url` → `baseURL` → `apiEndpoint`；缺则报错。
  [`proxy/providers/claude.rs#L717-L764`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/providers/claude.rs#L717-L764)
- 目标 URL 由适配器 `build_url(base_url, endpoint)` 拼接；认证头由 `get_auth_headers` 依凭据策略注入（`ANTHROPIC_API_KEY`→`x-api-key` 官方协议；`ANTHROPIC_AUTH_TOKEN` 或中转→`Authorization: Bearer`）。
  [`proxy/providers/claude.rs#L681-L846`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/providers/claude.rs#L681-L846)

**令牌**

- **仅 `/claude-desktop/*` 要求网关令牌**：校验 `Authorization: Bearer`（区分大小写前缀，去空格）是否等于 DB settings 键 `claude_desktop_gateway_token` 的值，不匹配返回 `AuthError`。
  [`proxy/handlers.rs#L401-L426`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/handlers.rs#L401-L426)、
  [`proxy/handlers.rs#L251-L281`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/handlers.rs#L251-L281)
- 令牌按需生成：`ccs-<uuid simple>`，写入 settings（未加密的判断见第 8 节）。它会被写进 Claude Desktop 3P profile 的 `inferenceGatewayApiKey`。
  [`claude_desktop_config.rs#L293-L304`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/claude_desktop_config.rs#L293-L304)
- 通用 `/v1/messages`、`/v1/chat/completions` 等**不校验该网关令牌**，用的是 provider 自身凭据。

---

## 2. 支持的 wire 协议 与 usage 的提取方式

**协议判定**：由 provider 的 `api_format` 决定，取值 `anthropic`（默认，透传）/ `openai_chat` / `openai_responses` / `gemini_native`；其中 `codex_oauth`/`xai_oauth` 强制 `openai_responses`。
[`proxy/providers/claude.rs#L35-L98`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/providers/claude.rs#L35-L98)

**Claude Desktop 客户端始终说 Anthropic Messages**：它访问 `/claude-desktop/v1/messages`，处理器固定走 Anthropic Messages 入参；上游若是 OpenAI/Gemini 格式则由转换层转换。Claude Desktop 与 Claude Code 复用同一套 Claude 处理器、适配器与解析配置。
[`proxy/handlers.rs#L251-L265`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/handlers.rs#L251-L265)、
[`proxy/providers/mod.rs#L266-L268`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/providers/mod.rs#L266-L268)

**各协议 usage 提取位置（`proxy/usage/parser.rs`，均由 `TokenUsage` 承载）**

| 协议 | 非流式 | 流式（SSE） | 引用 |
| --- | --- | --- | --- |
| Anthropic Messages | `from_claude_response`：`usage.input_tokens` / `output_tokens` / `cache_read_input_tokens` / `cache_creation_input_tokens`；`model` 取响应 `model`，`message_id` 取响应 `id` | `from_claude_stream_events`：**分事件字段合并**，非累加 | [#L119-L143](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/parser.rs#L119-L143)、[#L147-L268](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/parser.rs#L147-L268) |
| OpenAI Chat Completions | `from_openai_response`：`usage.prompt_tokens` / `completion_tokens`；缓存命中 `prompt_tokens_details.cached_tokens`（含多级回退与「显式 0 让位」） | `from_openai_stream_events`：取**最后一个带非空 `usage` 的 chunk**（即最终快照） | [#L370-L396](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/parser.rs#L370-L396)、[#L399-L417](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/parser.rs#L399-L417) |
| OpenAI Responses | `from_codex_response_auto`：`usage.input_tokens` / `output_tokens`（检测 `prompt_tokens` 则转 OpenAI 分支） | `from_codex_stream_events_auto`：取 `response.completed` 事件内 `response.usage`（最终快照） | [#L317-L367](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/parser.rs#L317-L367) |
| Gemini Native | `from_gemini_response`：`usageMetadata.promptTokenCount` / `totalTokenCount`，`output = total - input`；缓存 `cachedContentTokenCount` | `from_gemini_stream_chunks`：取最后一个 chunk 的 usage | [#L420-L506](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/parser.rs#L420-L506) |

**Anthropic SSE 的具体合并逻辑**（问题核心）：

- `message_start` → 从 `message.usage` 取 `input_tokens`、`cache_read_input_tokens`、`cache_creation_input_tokens`（及 1h 子集）；同时记录 `model`、`id`。
- `message_delta` → 从 `usage` 取 `output_tokens`；**若 delta 的 `input_tokens` 更小且 > 0**，则用 delta 覆盖 input，并同步采用同块里的缓存计数（应对「start 报含缓存的整段上下文、delta 报修正后的 fresh input」的中转）；delta 缺缓存字段时保留 start 值。
  [`parser.rs#L156-L251`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/parser.rs#L156-L251)
- 落账闸门用 `has_billable_tokens()`（input/output/cache_read/cache_creation 任一 > 0），保留「纯缓存命中、无输出」的真实计费流。
  [`parser.rs#L104-L114`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/parser.rs#L104-L114)、[#L257-L267](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/parser.rs#L257-L267)

各 handler 使用的解析配置集中定义在 `handler_config.rs`（`CLAUDE_PARSER_CONFIG` 的 `app_type_str="claude"`，但落账时以 `ctx.app_type_str` 覆盖为 `claude-desktop`）。
[`proxy/handler_config.rs#L138-L172`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/handler_config.rs#L138-L172)、
[`proxy/response_processor.rs#L520-L523`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/response_processor.rs#L520-L523)

---

## 3. 转发日志字段（`proxy_request_logs`）与「请求 id 关联」

**建表列**（`database/schema.rs`）：

```
request_id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, app_type TEXT NOT NULL,
model TEXT NOT NULL, request_model TEXT, pricing_model TEXT,
input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens (均 INTEGER DEFAULT 0),
input_token_semantics INTEGER DEFAULT 0,
input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd (TEXT 金额字符串),
latency_ms INTEGER, first_token_ms INTEGER, duration_ms INTEGER,
status_code INTEGER, error_message TEXT, session_id TEXT,
provider_type TEXT, is_streaming INTEGER DEFAULT 0,
cost_multiplier TEXT DEFAULT '1.0', created_at INTEGER, data_source TEXT DEFAULT 'proxy'
```

[`database/schema.rs#L197-L214`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/database/schema.rs#L197-L214)

**写入结构体与实际 INSERT 列**：`RequestLog` 定义 `request_id / provider_id / app_type / model / request_model / pricing_model / usage / cost / latency_ms / first_token_ms / status_code / error_message / session_id / provider_type / is_streaming / cost_multiplier`。
[`proxy/usage/logger.rs#L62-L87`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/logger.rs#L62-L87)、
[`logger.rs#L168-L209`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/logger.rs#L168-L209)

> 注意两处「字段存在但未落库」：`duration_ms` 不在 INSERT 列表（走列默认/NULL）；`TokenUsage.cache_creation_1h_tokens` 也不在 INSERT 列表（见第 6 节）。

**请求 id / 上游 request id 的关联**：

- **没有单独的「上游/provider request id」列**。日志主键 `request_id` 由 `TokenUsage.dedup_request_id()` 派生：
  - `message_id` 来自**上游响应体的 `id` 字段**（Anthropic 为 `msg_xxx`）；有 `message_id` 时 → `session:{message_id}`（Claude / Claude Desktop 不加 app/provider 作用域，以便与本地 session JSONL 的 `session:{message_id}` 主键收敛）；其他 app → `session:{app}:{provider}:{message_id}`；无 `message_id` → 随机 UUID。
  [`parser.rs#L50-L102`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/parser.rs#L50-L102)、
  [`response_processor.rs#L673-L674`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/response_processor.rs#L673-L674)
  - 结论：**对 Claude / Claude Desktop，本地日志主键就等于上游 message id**，可与上游消息关联；但没有「原始请求 id / provider 侧 request-id」这种独立列。
- `session_id` 列来自客户端会话：Claude 从 `metadata.user_id`（`user_x_session_y` 取 y 段）或 `metadata.session_id`，Codex 从 header `session_id`/`x-session-id` 或 `metadata.session_id`；都没有则新生成。
  [`proxy/session.rs#L1-L100`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/session.rs#L1-L100)
- `data_source` 列区分来源（默认 `proxy`；`session_log` 等用于本地会话导入）。写入时若主键已存在且为 `session_log` 行，会用代理行 `INSERT OR REPLACE` 覆盖；相同语义的重复代理行则 `INSERT OR IGNORE`（幂等）。
  [`logger.rs#L122-L167`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/logger.rs#L122-L167)

---

## 4. Claude Desktop 模型映射具体走哪条路径、覆盖边界

**用户要改什么（配置面）**：在 Claude Desktop 供应商里开启「需要模型映射」（对应 provider `meta.claude_desktop_mode = Proxy`），并配置 `meta.claude_desktop_model_routes`：每个 `route_id`（必须是 `claude-sonnet-*` / `claude-opus-*` / `claude-haiku-*` / `claude-fable-*`，或 `anthropic/claude-*`）映射到 `{model: 真实上游模型, label_override?: 菜单显示名, supports_1m?: bool}`。
[`claude_desktop_config.rs#L251-L276`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/claude_desktop_config.rs#L251-L276)、
[`claude_desktop_config.rs#L575-L637`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/claude_desktop_config.rs#L575-L637)

**CC Switch 到底写了什么（文件面）**：`Proxy` 模式会把 Claude Desktop 指向本地网关——
`inferenceProvider="gateway"`、`inferenceGatewayBaseUrl = http://<listen_address>:<listen_port>/claude-desktop`、`inferenceGatewayApiKey = <ccs-… 网关令牌>`、`inferenceGatewayAuthScheme="bearer"`、`coworkEgressAllowedHosts=["*"]`、`disableDeploymentModeChooser=true`、`inferenceModels=[{name: route_id, labelOverride?, supports1m?}]`；并把 `deploymentMode` 设为 `"3p"`，`_meta.json` 的 `appliedId` 指向 `PROFILE_ID`。
[`claude_desktop_config.rs#L971-L1011`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/claude_desktop_config.rs#L971-L1011)、
[`claude_desktop_config.rs#L1198-L1218`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/claude_desktop_config.rs#L1198-L1218)、
[`claude_desktop_config.rs#L935-L949`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/claude_desktop_config.rs#L935-L949)
落盘路径（macOS）：`~/Library/Application Support/Claude/claude_desktop_config.json`、`~/Library/Application Support/Claude-3p/claude_desktop_config.json`、`.../Claude-3p/configLibrary/_meta.json`、`.../Claude-3p/configLibrary/00000000-0000-4000-8000-000000157210.json`。
[`claude_desktop_config.rs#L1323-L1385`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/claude_desktop_config.rs#L1323-L1385)

**请求时如何映射**：Desktop 发来的 `model` 是 `route_id`，代理按 `route_id` 查 `upstream_model`，支持角色关键词/日期后缀/opus 别名/fable→opus 回落（回落通过后才发上游）。
[`claude_desktop_config.rs#L701-L769`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/claude_desktop_config.rs#L701-L769)

**覆盖边界（哪些 Desktop 流量经过 / 不经过）**

- **经过**：Claude Desktop 在「模型映射」模式下，发往 `http://127.0.0.1:15721/claude-desktop/...` 的推理请求（`POST /claude-desktop/v1/messages`）与模型清单（`GET /claude-desktop/v1/models`）。这些才进入网关，才被记账。
  [`proxy/server.rs#L307-L315`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/server.rs#L307-L315)
- **不经过**：
  1. **官方模式**：`deploymentMode="1p"` / 选择 Claude Desktop Official 时，Desktop 直连官方登录，不经本地网关。
  2. **直连模式**：3P profile 的 `inferenceGatewayBaseUrl` 直接指向第三方端点（不指向本地 `/claude-desktop`），也不经网关、不记账。
  3. Desktop 的非推理网络流量（遥测、更新、账号等）不经过本地代理。
- 该提交的 README 功能表把 Claude Desktop 的「本地路由」「用量统计」都标注为 **「模型映射时」**，即原生 Desktop 流量不被网关覆盖。
  [`README_ZH.md#L331-L349`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/README_ZH.md#L331-L349)
- 运行前提：模型映射模式下 CC Switch（含路由服务）必须保持运行，Desktop 切换后需重启应用。
  [`docs/user-manual/zh/2-providers/2.6-claude-desktop.md`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/docs/user-manual/zh/2-providers/2.6-claude-desktop.md)

---

## 5. 断流 / 取消 / 工具调用 / 重试 时 usage 的完整性与标记

- **客户端取消 / 连接断开**：SSE 收集器带 `SseUsageFinishGuard`，其 `Drop` 会在流句柄被丢弃时 `spawn(collector.finish())`，即用**已收集到的事件尽力落账**；正常结束则在流循环末尾显式 `collector.finish()` 并 `disarm()` 守卫（避免重复落账）。
  [`response_processor.rs#L461-L489`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/response_processor.rs#L461-L489)、
  [`response_processor.rs#L851-L857`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/response_processor.rs#L851-L857)
- **超时**：首字节超时 / 静默期超时命中时 `yield Err(...)` 后 `break`，随后仍执行 `collector.finish()`；上游流错误同理。
  [`response_processor.rs#L762-L857`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/response_processor.rs#L762-L857)
- **非流式**：整包读完再解析（超时可配），完整读到才记 usage。
  [`response_processor.rs#L223-L318`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/response_processor.rs#L223-L318)
- **工具调用**：usage 解析与是否含 `tool_use` 无关，走同一条 `message_start`/`message_delta`（或末 chunk）路径，无额外标记。
- **重试 / 故障转移**：`forward_with_retry` 会重试并切换 provider；usage 只在**最终返回给客户端的那次响应**上记账（处理器在 forward 返回后处理响应）。从代码看**没有逐个失败尝试的 usage 记录**——失败的尝试若已在上游产生消耗，不会被网关记下。**此点为读码结论，未见针对每 attempt 的记账分支，建议实机确认。**
  [`proxy/handlers.rs#L332-L354`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/handlers.rs#L332-L354)、
  [`proxy/forwarder.rs#L783-L900`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/forwarder.rs#L783-L900)
- **「缺 usage」的完整性标记（读码发现的一处不一致）**：
  - 转换路径（`handle_claude_transform`）：流式若解析不到 usage，只打 debug 日志、不写行。
    [`handlers.rs#L636-L638`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/handlers.rs#L636-L638)
  - 透传路径（`create_usage_collector`）：解析不到 usage 的** else 分支仍会以 `TokenUsage::default()`（全 0）调用记账**（其日志文案却写「跳过消费记录」）。由于 `log_with_calculation` 本身没有 `has_billable_tokens` 闸门，理论上会产生一条全 0 行（且无 `message_id` 时 `request_id` 为随机 UUID）。
    [`response_processor.rs#L561-L588`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/response_processor.rs#L561-L588)、
    [`logger.rs#L374-L428`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/logger.rs#L374-L428)
    此行为与注释冲突，建议实机验证是否真的写入全 0 行。

---

## 6. 上游实现里 Token 字段的口径（input 是否含 cache）

- **Anthropic 口径**：`input_tokens` 为 **fresh**（不含缓存读/写）；`cache_read_input_tokens`、`cache_creation_input_tokens` 单列。源码注释明确：Anthropic 报 fresh，缓存读取单列。
  [`services/sql_helpers.rs#L1-L7`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/sql_helpers.rs#L1-L7)、
  [`parser.rs#L128-L143`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/parser.rs#L128-L143)
- **哪些 app 的 input 含缓存**：`CACHE_INCLUSIVE_APP_TYPES = ["codex","gemini","grokbuild"]`（OpenAI Responses/`prompt_tokens` 与 Gemini `promptTokenCount` 含缓存部分）；**Claude / Claude Desktop 不在该集合**，即按 fresh 处理。
  [`services/sql_helpers.rs#L23-L28`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/sql_helpers.rs#L23-L28)
- **落库标记**：写入时 `input_token_semantics` = `TOTAL(1)`（若 `is_cache_inclusive_app`）否则 `FRESH(2)`；枚举 `LEGACY=0/TOTAL=1/FRESH=2`。因此 claude/claude-desktop 行标记为 FRESH。
  [`logger.rs#L122-L128`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/logger.rs#L122-L128)、
  [`sql_helpers.rs#L30-L32`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/sql_helpers.rs#L30-L32)
- **聚合归一**：`fresh_input_sql()` 把 cache-inclusive 行的 `input_tokens` 减去 cache_read（legacy）/cache_read+cache_creation（total），统一成 fresh 口径。
  [`sql_helpers.rs#L43-L67`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/sql_helpers.rs#L43-L67)
- **1 小时缓存 TTL 的口径缺口**：`cache_creation_1h_tokens`（`cache_creation.ephemeral_1h_input_tokens` 子集）在内存中被解析，但 **INSERT 列表里没有对应列**，故未持久化到 `proxy_request_logs`。
  [`parser.rs#L42-L48`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/parser.rs#L42-L48)、
  [`logger.rs#L168-L177`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/logger.rs#L168-L177)

---

## 7. 独立实现最小网关演示（转发 Anthropic Messages 并提取 usage）所需组件

按该源码，最小可行清单（只做 Anthropic 透传 + usage 提取，砍掉故障转移/格式转换/整流器/定价/UI）：

1. **HTTP 服务器 + 路由**：一个 axum `Router`，暴露 `POST /v1/messages`（如需 Claude Desktop，加 `POST /claude-desktop/v1/messages` 与 `GET /claude-desktop/v1/models`）。
   [`proxy/server.rs#L299-L315`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/server.rs#L299-L315)
2. **Provider 适配器**：`trait ProviderAdapter`（`extract_base_url`/`build_url`/`get_auth_headers`/`needs_transform`）+ Claude 实现。
   [`proxy/providers/adapter.rs#L16-L57`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/providers/adapter.rs#L16-L57)、
   [`proxy/providers/claude.rs#L717-L846`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/providers/claude.rs#L717-L846)
3. **转发器**：`forward_with_retry`（最小可只做单次转发）+ hyper/http 客户端。
   [`proxy/forwarder.rs#L783-L900`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/forwarder.rs#L783-L900)
4. **请求上下文**：`RequestContext`（provider、`request_model`、`outbound_model`、`session_id`、`start_time`、`app_type`）。
   [`proxy/handler_context.rs`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/handler_context.rs)
5. **响应处理 + SSE 收集器**：`process_response` / `handle_streaming` / `handle_non_streaming` / `SseUsageCollector` / `create_logged_passthrough_stream`（含 `SseUsageFinishGuard`）。
   [`proxy/response_processor.rs#L161-L349`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/response_processor.rs#L161-L349)、[#L355-L489](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/response_processor.rs#L355-L489)
6. **SSE 解析工具**：`append_utf8_safe` / `take_sse_block` / `strip_sse_field`。
   [`proxy/sse.rs`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/sse.rs)
7. **usage 解析器**：`TokenUsage::from_claude_response` 与 `TokenUsage::from_claude_stream_events`（核心的 message_start/message_delta 合并）。
   [`proxy/usage/parser.rs#L119-L268`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/parser.rs#L119-L268)
8. **落库/记录**：`RequestLog` + `UsageLogger` + `proxy_request_logs` 表（成本可先记 0，`calculator.rs` 可后补）。
   [`proxy/usage/logger.rs#L62-L223`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/logger.rs#L62-L223)、
   [`database/schema.rs#L200-L214`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/database/schema.rs#L200-L214)
9. **配置/端口**：`ProxyConfig` + DAO（监听地址/端口、是否记日志）。
   [`proxy/types.rs#L42-L55`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/types.rs#L42-L55)、
   [`database/dao/proxy.rs#L345-L384`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/database/dao/proxy.rs#L345-L384)
10. **（可选）session id 提取**：`extract_session_id`。
    [`proxy/session.rs#L71-L100`](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/session.rs#L71-L100)

可裁掉：故障转移/熔断（forwarder/provider_router/circuit_breaker）、OpenAI/Gemini/Responses 转换（transform*）、各整流器（thinking/media/opaque_state）、Copilot 优化器、定价与回填、Claude Desktop 的 profile 写入逻辑（演示只需自造一份指向本地网关的配置）。

---

## 证据链接汇总（均固定提交 `df40d53c…`）

| 主题 | 源码/文档 |
| --- | --- |
| 监听地址/端口、路由表 | [proxy/server.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/server.rs)、[proxy/types.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/types.rs)、[services/proxy.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/proxy.rs) |
| 协议判定/适配器 | [proxy/providers/claude.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/providers/claude.rs)、[proxy/providers/adapter.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/providers/adapter.rs)、[proxy/providers/mod.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/providers/mod.rs) |
| 网关令牌 / Claude Desktop handler | [proxy/handlers.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/handlers.rs)、[claude_desktop_config.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/claude_desktop_config.rs) |
| usage 解析 | [proxy/usage/parser.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/parser.rs)、[proxy/handler_config.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/handler_config.rs) |
| SSE 收集与落账 | [proxy/response_processor.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/response_processor.rs)、[proxy/sse.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/sse.rs) |
| 日志表/写入 | [database/schema.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/database/schema.rs)、[proxy/usage/logger.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/usage/logger.rs) |
| Token 口径归一 | [services/sql_helpers.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/services/sql_helpers.rs) |
| session id 提取 | [proxy/session.rs](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/src-tauri/src/proxy/session.rs) |
| 用户文档 | [README_ZH.md](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/README_ZH.md)、[docs/user-manual/zh/2-providers/2.6-claude-desktop.md](https://github.com/farion1231/cc-switch/blob/df40d53c36b604e044b4d5b9091aeb961aa56990/docs/user-manual/zh/2-providers/2.6-claude-desktop.md) |

---

## 未证实 / 需实机演示事项

以下为本次「只读源码」无法确证、需实机或抓包验证的点，**不得当作已证实结论**：

1. **Claude Desktop 真实流量覆盖**：只从代码+文档推断「仅模型映射模式下的 `/claude-desktop/v1/messages` 会经过网关并被记账」；未实测 Desktop 各功能（cowork、子 agent、图片/附件、后台任务）究竟哪些请求真的走该 base URL、哪些不走。
2. **断流时 usage 的完整性**：`SseUsageFinishGuard` 的 `Drop`→`finish()` 是「尽力而为」；客户端在半途取消、或上游在 `message_delta`（output_tokens 所在）之前断开时，落账值可能只有 `message_start` 的 input/缓存、缺 output。需实机验证落账值是否偏小。
3. **全 0 行的写入**：透传路径 `create_usage_collector` 的 else 分支以 `TokenUsage::default()` 落账，与其日志文案矛盾；需实机确认是否真的会给「上游未上报 usage」的流式请求写入一条全 0 行（以及是否污染请求计数）。
4. **重试/故障转移的消耗漏记**：读码未见逐 attempt 记账；失败切换前的上游调用若已产生消耗，是否漏记，需实机用可计费上游验证。
5. **1h 缓存 TTL 未落库**：`cache_creation_1h_tokens` 只在内存解析、无对应列；其对费用/用量展示的影响需确认（是否有别的表承接）。
6. **令牌存储位置**：`claude_desktop_gateway_token` 存入 settings 表；本次未确认其落盘是否加密、是否满足「API Key 只存系统凭据库」的更强要求。
7. **`listen_address` 与安全**：默认回环 127.0.0.1；若用户改为 `0.0.0.0`，`/claude-desktop/*` 仅靠 `ccs-…` 令牌保护、通用 `/v1/messages` 无网关令牌——暴露面需评估，未实机验证。
8. **`app_type` CHECK 不含 `claude-desktop`**：`proxy_config` 只允许 claude/codex/gemini/grokbuild；Claude Desktop 复用 claude 的全局代理/计价配置（logger.rs 注释亦印证）。这一「继承」语义需在产品设计中显式处理，避免与「独立 app 命名空间」的其他表混淆。

（本文结论仅来自以上固定提交处读到的代码与文档；读不到的项已在正文标注「未获取」或列入本清单。）
