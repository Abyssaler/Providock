> **Providock** 是基于 [CC Switch](https://github.com/farion1231/cc-switch) **v3.20.4**（MIT 许可证，© Jason Young）独立二次开发的产品。
> 本仓库与上游项目无关联。

# Providock

AI 编程工具供应商的一站式桌面管理器，支持 Claude Code、Claude Desktop、Codex、Kimi Code、Grok Build、OpenCode、OpenClaw、Hermes、Pi、MiniMax Code。

[English](README.md) | 中文 | [更新日志](CHANGELOG.md)

## 功能特性

- **供应商切换** — 为各个支持的工具添加、编辑并一键切换 API 供应商，支持实时配置同步与端点测速。
- **统一 MCP 管理** — 一次配置 MCP 服务器，同步到所有支持的应用。
- **Skills 与 Prompts** — 在统一面板中管理各应用的 Skills 与 Prompt 文件。
- **会话管理** — 浏览、搜索并恢复 CLI 会话，支持统一的 Codex 会话历史。
- **本地代理与路由** — 内置本地代理，支持按应用路由、故障转移与请求覆盖。
- **用量统计** — 统计各供应商的 Token 用量、费用与订阅额度。
- **同步与备份** — 配置导入导出、WebDAV/S3 同步，以及通过 `providock://` 深链接导入。

## 快速开始

从 [GitHub Releases](https://github.com/Abyssaler/providock/releases) 下载最新构建，或从源码构建。

### 从源码构建

前置条件：Node.js 与 pnpm、Rust 工具链，以及对应平台的 Tauri 2 系统依赖。

```bash
pnpm install
pnpm dev    # 开发模式运行
pnpm build  # 构建发行包
```

## 数据存储

Providock 的数据库与配置保存在 `~/.providock/`。

## 文档

- [用户手册](docs/user-manual/README.md) — English / 中文 / 日本語
- [指南](docs/guides/) — 路由、代理与工作流指南

## 贡献

欢迎通过 [GitHub Issues](https://github.com/Abyssaler/providock/issues) 提交 Bug 报告、功能请求与 Pull Request。详见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 许可证

MIT。基于 CC Switch v3.20.4（MIT 许可证，© Jason Young）。详见 [LICENSE](LICENSE)。
