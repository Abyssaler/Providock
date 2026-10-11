> **Providock** is an independent product built upon [CC Switch](https://github.com/farion1231/cc-switch) **v3.20.4** (MIT License, © Jason Young). It is not affiliated with the upstream project.

# Providock

**An all-in-one desktop manager for AI coding tool providers** — manage API providers, MCP servers, Skills, prompts, and usage statistics for Claude Code, Claude Desktop, Codex, Kimi Code, Grok Build, OpenCode, OpenClaw, Hermes, Pi, and MiniMax Code in a single app.

[中文](README.md) | English | [Changelog](CHANGELOG.md)

---

## Features

### Provider management

- Add, edit, delete, and switch API providers for each supported tool, writing directly to each tool's native config file
- **Two modes**: switch mode (Claude Code, Codex, Kimi Code, Grok Build — one active live config) and additive mode (OpenCode, OpenClaw, Hermes, Pi, MiniMax Code — providers coexist in the native config and are enabled independently)
- Built-in presets for major vendors (Kimi, DeepSeek, Zhipu GLM, MiniMax, SiliconFlow, and more), custom endpoints, and endpoint speed testing
- Common config snippets merged into the live config on every switch
- Official subscription account awareness (Claude / Codex OAuth login preservation and backfill)

### Unified MCP management

- Configure once, sync to any subset of supported apps (Claude Code, Codex, Kimi Code, Grok Build, OpenCode, Hermes, MiniMax Code)
- stdio / HTTP / SSE transports; import from existing app configs
- One-click import via `providock://` deep links

### Skills & prompts

- Centralized Skill storage with per-app sync (install, update, per-app enablement, unmanaged skill discovery)
- Prompt management per app with one-click activation, written to each tool's prompt file (e.g. `CLAUDE.md`, `AGENTS.md`)

### Session management

- Browse, search, and resume CLI sessions across apps (Claude Code, Codex, Kimi Code, OpenCode, OpenClaw, Hermes, Pi, and more)
- Unified Codex session history

### Usage statistics & quota

- Token usage and cost tracking per app / provider / model (built-in pricing table, customizable)
- Coding-plan quota queries: Kimi For Coding, Zhipu GLM, MiniMax, ZenMux, Volcengine, OpenCode Go
- Custom usage query scripts (JavaScript) for arbitrary providers

### Local proxy & routing (Claude Code / Codex / Grok Build)

- Built-in local proxy with routing takeover, failover, request overrides, and protocol conversion (e.g. Codex Responses ↔ Chat, Claude ↔ Gemini Native)
- Takeover status monitoring and recovery

### Miscellaneous

- System tray with quick provider switching
- Config import/export, WebDAV / S3 sync and backup
- Tool health check with one-click install/upgrade (including Kimi Code via `@moonshot-ai/kimi-code`)
- UI languages: 简体中文 / 繁體中文 / English / 日本語

## Supported apps

| App | Config file | Mode | MCP | Skills | Prompt | Sessions | Local proxy |
|---|---|---|---|---|---|---|---|
| Claude Code | `~/.claude/settings.json` | Switch | ✅ | ✅ | `CLAUDE.md` | ✅ | ✅ |
| Claude Desktop | 3rd-party provider config | Switch | — | — | — | — | — |
| Codex | `~/.codex/config.toml` | Switch | ✅ | ✅ | `AGENTS.md` | ✅ | ✅ |
| **Kimi Code** | `~/.kimi-code/config.toml` | Switch | ✅ | ✅ | `AGENTS.md` | ✅ | — |
| Grok Build | `~/.grok/` | Switch | ✅ | ✅ | ✅ | ✅ | ✅ |
| OpenCode | `~/.config/opencode/opencode.json` | Additive | ✅ | ✅ | ✅ | ✅ | — |
| OpenClaw | `~/.openclaw/` | Additive | — | — | ✅ | ✅ | — |
| Hermes | `~/.hermes/config.yaml` | Additive | ✅ | ✅ | ✅ | ✅ | — |
| Pi | `~/.pi/agent/` | Additive | — | ✅ | — | ✅ | — |
| MiniMax Code | `~/.minimax/config.yaml` | Additive | ✅ | ✅ | ✅ | ✅ | — |

> All Kimi Code paths follow the `KIMI_CODE_HOME` environment variable (default `~/.kimi-code`).

## Installation

Download the latest build from [GitHub Releases](https://github.com/Abyssaler/providock/releases):

- **macOS (Apple Silicon)**: `Providock_x.y.z_aarch64.dmg`
- **Windows (x64)**: `Providock_x.y.z_x64-setup.exe` (NSIS) or `.msi`

### First launch on macOS

The app is not signed with an Apple Developer certificate, so Gatekeeper may block the first launch. Either run:

```bash
xattr -d com.apple.quarantine /Applications/Providock.app
```

or click "Open Anyway" in System Settings → Privacy & Security.

### In-app updates

When enabled, the app checks `latest.json` on the Releases page and installs updates automatically (update packages are verified with minisign signatures).

## Kimi Code guide

Providock supports Kimi Code as a first-class app:

1. **Add a provider**: open the Kimi Code tab → Add Provider. Choose the "Kimi Official" preset or a custom type (`kimi` / `anthropic` / `openai` / `openai_responses` / `google-genai` / `vertexai`)
2. **Credentials**: enter an API key directly, or an environment variable name (e.g. `KIMI_API_KEY`) — at least one of the two
3. **Models**: each provider can define multiple model aliases (real model ID, context size, capabilities, thinking effort, etc.)
4. **Switching**: on switch, Providock writes the provider and its models into `~/.kimi-code/config.toml` and sets `default_model`. Your own providers and other sections (`[permission]`, `[hooks]`, …) are left untouched
5. **Quota display**: use a Kimi For Coding key (generated from kimi.com/code membership benefits) as the provider credential, enable "Usage query" with the Coding Plan template on the provider card, and the remaining quota across the 5-hour / weekly / monthly windows is shown

> Note: OAuth managed accounts (`managed:kimi-code`, logged in via `/login` in the CLI) are recognized and imported, but their quota display is not supported yet (no public API). The OAuth login itself is unaffected.

## Build from source

Prerequisites: Node.js 22 + pnpm 10, Rust 1.95+ (see `rust-toolchain.toml`), and the Tauri 2 platform dependencies (see the [Tauri docs](https://tauri.app/start/prereqs/)).

```bash
pnpm install          # install frontend dependencies
pnpm dev              # run in development mode
pnpm build            # produce release bundles (dmg / exe / msi)

# Tests and checks
pnpm test:unit        # frontend unit tests (vitest)
pnpm typecheck        # TypeScript type checking
cd src-tauri && cargo test   # Rust tests
```

Release process: push a tag like `v1.2.3` and GitHub Actions builds, signs, and publishes the macOS arm64 and Windows x64 installers to Releases.

## Data storage

Everything lives in `~/.providock/`:

```
~/.providock/
├── providock.db        # main database (providers, MCP, prompts, usage; SQLite)
├── settings.json       # app settings
├── model-pricing.json  # model pricing table
├── backups/            # automatic backups (including pre-migration snapshots)
└── logs/               # runtime logs
```

Migrating from CC Switch: quit Providock, then copy `~/.cc-switch/cc-switch.db` to `~/.providock/providock.db`. Providock runs the database schema migration automatically on launch.

## Documentation

- [User manual](docs/user-manual/) — 中文 / English / 日本語
- [Guides](docs/guides/) — routing, proxy, and integration guides

## Contributing

Bug reports, feature requests, and pull requests are welcome via [GitHub Issues](https://github.com/Abyssaler/providock/issues). See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## License

MIT. Based on CC Switch v3.20.4 (MIT License, © Jason Young). See [LICENSE](LICENSE).
