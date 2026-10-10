> **Providock** is an independent product built upon [CC Switch](https://github.com/farion1231/cc-switch) **v3.20.4** (MIT License, © Jason Young).
> This repository is not affiliated with the upstream project.

# Providock

All-in-one desktop manager for AI coding tool providers: Claude Code, Claude Desktop, Codex, Kimi Code, Grok Build, OpenCode, OpenClaw, Hermes, Pi, and MiniMax Code.

English | [中文](README_ZH.md) | [Changelog](CHANGELOG.md)

## Features

- **Provider switching** — add, edit, and switch API providers for each supported tool with one click, including live config synchronization and endpoint speed testing.
- **Unified MCP management** — configure MCP servers once and sync them to every supported app.
- **Skills & Prompts** — manage Skills and prompt files across apps from a single panel.
- **Session management** — browse, search, and resume CLI sessions, with unified Codex session history.
- **Local proxy & routing** — built-in local proxy with per-app routing, failover, and request overrides.
- **Usage statistics** — track token usage, costs, and subscription quotas across providers.
- **Sync & backup** — import/export configuration, WebDAV/S3 sync, and deep-link import via `providock://`.

## Getting Started

Download the latest build from [GitHub Releases](https://github.com/Abyssaler/providock/releases), or build from source.

### Build from source

Prerequisites: Node.js with pnpm, the Rust toolchain, and the Tauri 2 system dependencies for your platform.

```bash
pnpm install
pnpm dev    # run in development mode
pnpm build  # produce release bundles
```

## Data Storage

Providock stores its database and configuration in `~/.providock/`.

## Documentation

- [User manual](docs/user-manual/README.md) — English / 中文 / 日本語
- [Guides](docs/guides/) — routing, proxy, and workflow guides

## Contributing

Bug reports, feature requests, and pull requests are welcome via [GitHub Issues](https://github.com/Abyssaler/providock/issues). See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## License

MIT. Based on CC Switch v3.20.4 (MIT License, © Jason Young). See [LICENSE](LICENSE).
