> **Providock** 是基于 [CC Switch](https://github.com/farion1231/cc-switch) **v3.20.4**（MIT 许可证，© Jason Young）独立二次开发的产品，与上游项目无关联。

# Providock

**AI 编程工具供应商的一站式桌面管理器**——在一个应用里管理 Claude Code、Claude Desktop、Codex、Kimi Code、Grok Build、OpenCode、OpenClaw、Hermes、Pi、MiniMax Code 的 API 供应商、MCP 服务器、Skills、提示词与用量统计。

中文 | [English](README_EN.md) | [更新日志](CHANGELOG.md)

---

## 目录

- [功能特性](#功能特性)
- [支持的应用](#支持的应用)
- [安装](#安装)
- [Kimi Code 使用指南](#kimi-code-使用指南)
- [从源码构建](#从源码构建)
- [数据存储](#数据存储)
- [文档](#文档)
- [贡献](#贡献)
- [许可证](#许可证)

## 功能特性

### 供应商管理（Provider）

- 为每个支持的应用添加、编辑、删除、切换 API 供应商，一键写入对应工具的原生配置文件
- **两种模式**：切换模式（Claude Code、Codex、Kimi Code、Grok Build——写入当前生效的 live 配置）与共存模式（OpenCode、OpenClaw、Hermes、Pi、MiniMax Code——多个供应商共存于原生配置，独立启停）
- 内置主流供应商预设（Kimi、DeepSeek、智谱 GLM、MiniMax、SiliconFlow 等），支持自定义端点与端点测速
- 通用配置片段：为每个应用维护一段公共配置（如环境变量、TOML 片段），切换供应商时自动合并
- 官方订阅账号识别（Claude / Codex OAuth 登录态保护与回填）

### MCP 服务器统一管理

- 一处配置，按需勾选同步到各个应用（Claude Code、Codex、Kimi Code、Grok Build、OpenCode、Hermes、MiniMax Code）
- 支持 stdio / HTTP / SSE 三种传输方式，支持从各应用现有配置导入
- 支持通过 `providock://` 深链接一键导入 MCP 配置

### Skills 与提示词

- Skills 统一存储与多应用同步（安装、更新、启用范围勾选、未管理 Skills 发现）
- 提示词（Prompt）管理：按应用维护多份提示词并切换启用，自动写入对应文件（如 Claude 的 `CLAUDE.md`、Kimi Code 的 `AGENTS.md`）

### 会话管理

- 浏览、搜索、恢复各应用的 CLI 历史会话（Claude Code、Codex、Kimi Code、OpenCode、OpenClaw、Hermes、Pi 等）
- Codex 会话历史统一归集

### 用量统计与额度查询

- 按应用/供应商/模型统计 token 用量与费用（内置模型定价表，可自定义）
- 编程套餐额度查询：Kimi For Coding、智谱 GLM、MiniMax、ZenMux、火山方舟、OpenCode Go
- 支持自定义用量查询脚本（JavaScript），适配任意供应商

### 本地代理与路由（Claude Code / Codex / Grok Build）

- 内置本地代理，支持路由接管、故障转移、请求改写与协议转换（如 Codex Responses ↔ Chat、Claude ↔ Gemini Native 格式）
- 代理接管状态监控与恢复

### 其他

- 系统托盘常驻，快捷切换供应商
- 配置导入/导出，WebDAV / S3 云同步与备份
- 工具体检：检测各 CLI 是否安装，支持一键安装/升级（含 Kimi Code：`@moonshot-ai/kimi-code`）
- 界面语言：简体中文 / 繁體中文 / English / 日本語

## 支持的应用

| 应用 | 配置文件 | 模式 | MCP | Skills | Prompt | 会话 | 本地代理 |
|---|---|---|---|---|---|---|---|
| Claude Code | `~/.claude/settings.json` | 切换 | ✅ | ✅ | `CLAUDE.md` | ✅ | ✅ |
| Claude Desktop | 第三方 provider 配置 | 切换 | — | — | — | — | — |
| Codex | `~/.codex/config.toml` | 切换 | ✅ | ✅ | `AGENTS.md` | ✅ | ✅ |
| **Kimi Code** | `~/.kimi-code/config.toml` | 切换 | ✅ | ✅ | `AGENTS.md` | ✅ | — |
| Grok Build | `~/.grok/` | 切换 | ✅ | ✅ | ✅ | ✅ | ✅ |
| OpenCode | `~/.config/opencode/opencode.json` | 共存 | ✅ | ✅ | ✅ | ✅ | — |
| OpenClaw | `~/.openclaw/` | 共存 | — | — | ✅ | ✅ | — |
| Hermes | `~/.hermes/config.yaml` | 共存 | ✅ | ✅ | ✅ | ✅ | — |
| Pi | `~/.pi/agent/` | 共存 | — | ✅ | — | ✅ | — |
| MiniMax Code | `~/.minimax/config.yaml` | 共存 | ✅ | ✅ | ✅ | ✅ | — |

> Kimi Code 的所有路径遵循 `KIMI_CODE_HOME` 环境变量（默认 `~/.kimi-code`）。

## 安装

### 下载安装包（推荐）

从 [GitHub Releases](https://github.com/Abyssaler/providock/releases) 下载最新版本：

- **macOS（Apple Silicon，M 系列芯片）**：`Providock_x.y.z_aarch64.dmg`
- **Windows（x64）**：`Providock_x.y.z_x64-setup.exe`（NSIS 安装包）或 `.msi`

### macOS 首次打开说明

应用未使用 Apple 开发者证书签名，首次打开可能被 Gatekeeper 拦截。两种解决方式任选其一：

```bash
xattr -d com.apple.quarantine /Applications/Providock.app
```

或在「系统设置 → 隐私与安全性」中点击「仍要打开」。

### 应用内自动更新

开启后，应用会通过 Releases 页面的 `latest.json` 自动检查并安装更新（更新包使用 minisign 签名校验）。

## Kimi Code 使用指南

Providock 将 Kimi Code 作为一等公民支持：

1. **添加供应商**：进入 Kimi Code 页签 → 添加供应商，可选择「Kimi 官方」预设，或自定义类型（`kimi` / `anthropic` / `openai` / `openai_responses` / `google-genai` / `vertexai`）
2. **填写凭据**：API Key 直接填写，或填写环境变量名（如 `KIMI_API_KEY`，二者至少一项）
3. **模型配置**：每个供应商可配多个模型别名（真实模型 ID、上下文长度、能力标签、思考强度等）
4. **切换**：点击切换后，Providock 将供应商与模型写入 `~/.kimi-code/config.toml` 并设置 `default_model`；你手工添加的其他供应商和 `[permission]`、`[hooks]` 等配置原样保留
5. **额度显示**：使用 Kimi 会员在 kimi.com/code 生成的 Kimi For Coding Key 作为供应商凭据，在供应商卡片上启用「用量查询」并选择 Coding Plan 模板，即可显示 5 小时 / 周 / 月窗口的剩余额度

> 说明：Kimi Code CLI 内 `/login` 的 OAuth 托管账号（`managed:kimi-code`）会被 Providock 识别导入；其额度显示暂不支持（官方未公开对应查询接口），OAuth 登录态本身不受影响。

## 从源码构建

环境要求：Node.js 22 + pnpm 10、Rust 1.95+（见 `rust-toolchain.toml`）、Tauri 2 平台依赖（见 [Tauri 官方文档](https://tauri.app/start/prereqs/)）。

```bash
pnpm install          # 安装前端依赖
pnpm dev              # 开发模式运行（前后端热更新）
pnpm build            # 构建发布包（dmg / exe / msi 等）

# 测试与检查
pnpm test:unit        # 前端单元测试（vitest）
pnpm typecheck        # TypeScript 类型检查
cd src-tauri && cargo test   # Rust 测试
```

发布流程：推送形如 `v1.2.3` 的 tag，GitHub Actions 会自动构建 macOS arm64 与 Windows x64 安装包、签名并发布到 Releases。

## 数据存储

所有数据存放在 `~/.providock/`：

```
~/.providock/
├── providock.db        # 主数据库（供应商、MCP、提示词、用量等，SQLite）
├── settings.json       # 应用设置
├── model-pricing.json  # 模型定价表
├── backups/            # 自动备份（含数据库迁移前备份）
└── logs/               # 运行日志
```

从 CC Switch 迁移：退出 Providock 后，将 `~/.cc-switch/cc-switch.db` 复制为 `~/.providock/providock.db` 即可（Providock 启动时会自动执行数据库结构迁移）。

## 文档

- [用户手册](docs/user-manual/)：中文 / English / 日本語
- [专题指南](docs/guides/)：路由、代理、接入实践等

## 贡献

欢迎通过 [GitHub Issues](https://github.com/Abyssaler/providock/issues) 提交问题与建议，或直接发起 Pull Request。详见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可证

MIT。基于 CC Switch v3.20.4（MIT 许可证，© Jason Young）二次开发，详见 [LICENSE](LICENSE)。
