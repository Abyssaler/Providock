# Providock

本地优先的 Agent 用量监测与 AI 账户额度管理桌面应用。

**当前状态：核心链路已实现并通过样本级验证（2026-10-11）。** 八个本地 Agent 的解析器、采集入库、聚合与用量/账户/会话/数据源四页 UI 均已落地；macOS 实机只读验证了 Kimi Code 与 OpenCode。账户查询全部接口为样本验证（mock），未做真实凭据实测；Windows 实机、Claude Desktop 网关演示、正文查看的实机联调尚未完成。**准确的逐项状态见 [CURRENT_STATUS.md](docs/CURRENT_STATUS.md)，不要把本文的功能描述当作已实机验收。**

## 开发命令

```bash
pnpm install --frozen-lockfile
pnpm dev          # 仅前端浏览器预览
pnpm tauri dev    # 桌面联调
pnpm typecheck && pnpm lint && pnpm test
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
pnpm tauri build  # 在目标 OS 构建安装包
```

工具链：Node 24（.node-version）、pnpm 12.11.2（packageManager 字段）、Rust 1.99.0（rust-toolchain.toml）。CI 在 macos-15（ARM64）与 windows-2022（x64）双平台执行上述检查并产出未签名安装包。

Providock 聚合电脑上的 Agent Token 用量、输入输出、缓存读写及会话信息；独立管理 API Key、平台余额和 Coding Plan 额度。账户无需绑定 Agent，也不负责替用户切换模型或改写 Agent 配置。

首个稳定版本目标：Claude Code、Codex、Gemini CLI、Grok Build、OpenCode、Pi、MiniMax Code、Kimi Code；另以默认关闭的可选采集网关覆盖 cc-switch 已有的 Claude Desktop 模型映射请求范围。界面参考 cc-switch **v3.20.4** 的布局和视觉风格，监测能力对齐研究时的当前源码基线。

首版语言为简体中文，正式平台为 Apple Silicon macOS 和 Windows x64 普通安装包。可选查看原始对话，默认只保存统计元数据。

## 阅读入口

| 文档                                               | 用途                                             |
| -------------------------------------------------- | ------------------------------------------------ |
| [当前状态](docs/CURRENT_STATUS.md)                 | 逐任务交接事实表（最先读）                       |
| [开发手册](docs/DEVELOPMENT_HANDBOOK.md)           | 开发者和 AI Agent 的统一入口、实施顺序及交接要求 |
| [产品需求](docs/PRODUCT_REQUIREMENTS.md)           | 已确认需求、功能边界、用户流程                   |
| [架构设计](docs/ARCHITECTURE.md)                   | 模块、数据流、目录、IPC 与任务调度               |
| [Agent 采集规范](docs/specs/AGENT_ADAPTERS.md)     | 逐项采集来源、格式、去重与精度验证               |
| [账户适配规范](docs/specs/ACCOUNT_ADAPTERS.md)     | 余额、计划额度、官方证据和接入状态               |
| [数据契约](docs/specs/DATA_CONTRACTS.md)           | Token、缓存、费用、配额和存储语义                |
| [界面设计](docs/specs/UI_UX.md)                    | 页面、组件、状态与 v3.20.4 参考边界              |
| [离线交互原型](docs/prototypes/dashboard.html)     | 下载后用浏览器打开；全部为演示数据               |
| [隐私与凭据](docs/specs/SECURITY_PRIVACY.md)       | 密钥、正文查看、网络和导出规则                   |
| [跨平台发布](docs/specs/CROSS_PLATFORM_RELEASE.md) | macOS / Windows 路径、构建及安装验收             |
| [路线图与验收](docs/ROADMAP_AND_ACCEPTANCE.md)     | 任务编号、发布门槛和验收场景                     |
| [上游研究](docs/research/UPSTREAM_AUDIT.md)        | 固定提交、源码证据、已知缺口                     |
| [架构决策](docs/adr/0001-core-decisions.md)        | 决策理由与变更条件                               |
| [AGENTS.md](AGENTS.md)                             | 后续 AI Agent 的执行约定                         |

## 数据边界

本机日志只能反映本机保留下来的记录；平台额度可能覆盖其他设备和团队成员。两者分别展示。没有官方文档或官方客户端源码依据的余额接口暂不接入；未知值显示“未提供”，不显示零。费用估算与真实账单明确区分。

许可证沿用 MIT。上游参考与版权处理见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
