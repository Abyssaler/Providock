# Providock

本地优先的 AI Agent 用量统计与账户额度管理桌面应用。读取电脑上已有的 Agent 日志，查看 Token、缓存、模型成本估算和会话；API 余额与 Coding Plan 独立管理，账户无需绑定 Agent。

[下载 v2.1.1](https://github.com/Abyssaler/Providock/releases/tag/v2.1.1) · [发布说明](docs/releases/v2.1.1.md) · [当前状态](docs/CURRENT_STATUS.md) · [变更记录](CHANGELOG.md)

v2.1.1 新增 Windows 关闭到托盘、左键恢复和右键退出。v2.1.0 的 DeepSeek Harness、时间筛选、Codex 官方额度和模型定价保持。见[Windows 验证记录](docs/research/validation/2026-10-13-windows-tray-v2.1.1.md)。

## 安装与更新

| 系统                    | 安装包                                            |
| ----------------------- | ------------------------------------------------- |
| Apple Silicon macOS 13+ | `Providock_2.1.1_aarch64.dmg`，打开后拖入应用程序 |
| Windows x64             | `Providock_2.1.1_x64-setup.exe`，每用户安装       |

v2.0.3 起支持应用内更新：在「设置 → 软件更新」检查，点击「下载、安装并重启」完成升级，保留本机统计、账户与价格规则。默认启动及运行每 6 小时检查 GitHub 正式 Release，可关闭自动检查。应用完全退出后不检查。

v2.0.2 及更早版本需要退出旧应用，手动替换应用包一次。v1 使用独立标识和数据目录，不自动迁移。Release 同时提供 SHA-256 摘要、公钥及 minisign 签名；Mac 应用有 ad-hoc 完整性签名，尚无 Apple Developer ID 公证和 Windows Authenticode。

## 用量统计

支持 Claude Code、Codex、Gemini CLI、Grok Build、OpenCode、Pi、MCode（MiniMax Code）、Kimi Code、DeepSeek Harness 九个本地数据源。设置中可选择用量页展示的 Agent。

- 按 Agent、来源、精确模型名和日期筛选，查看输入、输出、缓存创建、缓存命中、加权缓存命中率与成本估算。
- **默认范围为本机当天零点至最近一次统计刷新**。摘要、六张指标卡、趋势和明细使用同一筛选，页面直接显示起止时间。另可选滚动最近 1/3/6 小时、自定义起止日期时间（本机时间，结束时刻不包含）；48 小时以内按小时绘图。macOS 菜单栏另用最近 24 小时。
- 趋势包含分量曲线、渐变、独立成本右轴、提示与图例；提供模型/Agent 汇总和分页计量明细。
- 金额摘要、明细与提示显示两位小数，底层精确十进制计算。没有价格或未知 Token 保留为空，部分估算显示覆盖率，不把未知当免费；不同币种分别统计。

应用默认启动同步一次，之后每 60 秒扫描，跳过未变化内容。用量页「显示刷新」只重新查询统计库，不改变采集周期。「设置 → 采集与刷新」可分别关闭 Agent 自动同步、账户自动刷新，重启保持选择。

关闭窗口后继续后台运行：macOS 驻留菜单栏，⌘Q 或菜单栏退出才停止；Windows 隐藏到托盘，左键图标或右键「打开 Providock」恢复，右键「退出 Providock」才停止。隐藏期间自动任务仍遵循设置开关。采集只读，不改 Agent 配置、不调用模型做测试补全。

## 模型价格

在用量页打开「模型定价」，或从模型行点击「设置价格」。规则按 Agent、精确模型名和可选来源保存，输入/输出/缓存读取/缓存创建均为每百万 Token 价格。支持 CNY、USD、北京时间工作日分时段和长上下文整请求升档；空白表示未知，0 表示明确免费。

**选定模型后获取**：先选 Agent 和精确模型名，再点击「从 AIHOT 导入」，读取 [AIHOT 模型排行榜](https://aihot.news/leaderboard) 的人民币报价并填入当前表单；核对后点击「保存并用于估算」。不会批量写入其它模型；保存采用既有版本追加机制。来源未给出的缓存创建价保持未知，失败不改已有价格。来源/汇率日期可在本次候选报价中查看，保存后的规则作为用户核定价格。

也可展开「从官网链接获取」，输入公开 HTTPS 计费页，点击「程序解析」。程序支持单位和币种明确的静态价格表，不执行网页脚本。无法可靠识别时可选已有 DeepSeek、GLM、OpenRouter API 账户和模型，点击「用所选模型解析」发起一次付费 API 请求，发送公开价格文本和目标模型，不发送本地日志或其它账户数据。DeepSeek/OpenRouter 可获取账户模型列表；GLM 直接输入模型 ID。结果仍需核对并保存，不运行工具、代理循环或后台解析。

价格估算用于比较所选历史用量，不是平台账单，不改写 Token 或历史估值。v2.0.4 已移除源「已报告成本」采集、统计和展示。

## 账户与隐私

账户页分别展示 API 余额、Coding Plan 各窗口剩余比例和重置时间。长来源信息折叠显示；每个额度窗口独立排列。Codex / ChatGPT 官方额度使用本机官方 Codex 当前登录；选择该平台即可添加，无需 API Key。需先安装官方 CLI/桌面并使用 ChatGPT 登录；自定义 CLI 路径可设置 `CODEX_CLI_PATH`。只读 App Server 额度窗口，不复制 OAuth 令牌，不发起对话，API Key 登录不支持订阅额度。

账户按自身最近查询时间刷新：API 余额默认 15 分钟，Coding Plan 默认 5 分钟，带少量抖动。

Key 按用户选择**明文保存在本机**独立 `credentials/keys.sqlite3`，重启直接使用，不访问系统钥匙串。Unix 目录/文件权限为 0700/0600，Windows 使用用户目录 ACL。完整复制应用数据目录会包含 Key，备份需要保护；普通统计库、前端响应及 Release 不包含明文 Key。

默认无遥测、不上传日志、不复制保存对话正文。可选正文查看按需读取源文件，禁用脚本和远程资源。日志只代表本机保留的记录；平台额度可能覆盖其他设备或成员，两者不相加。

## 验证范围

九源解析器均有合成样本回归（DeepSeek Harness 当前支持 session v3/v4，含 .zstd；旧 v0–2、未知版本与自定义 sessionsDir 未接入，缺少可核对父会话的分叉暂缓采集并保留历史）；Codex、Kimi Code、OpenCode 已有 macOS 真实日志核对，Kimi Code 直连与 GLM 无套餐/认证错误已有局部真实验证，Codex 官方额度已有 macOS 真实只读验证；其余账户能力以样本验证为主。AIHOT 读取、匹配和导入已用隔离数据库及 macOS 原生 UI 实测。

Windows 有 CI 构建和测试，尚未完成 Windows 实机安装、更新与缩放验收。可选网关只有 usage 解析层，转发本体未实现；导出/清理尚无 UI，更多正文读取和完整性能/休眠验收待完成。逐项事实以[当前状态](docs/CURRENT_STATUS.md)和[本版核验记录](docs/research/validation/2026-10-13-release-v2.1.0.md)为准。

## 本地开发

工具链：Node 24、pnpm 12.11.2、Rust 1.99.0，版本文件已固定。

```bash
pnpm install --frozen-lockfile
pnpm dev          # 前端预览，原生能力不可用
pnpm tauri dev    # 桌面联调
pnpm typecheck
pnpm lint
pnpm test
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
pnpm tauri build
```

CI 在 macos-15 ARM64 和 windows-2022 x64 检查并产出签名安装包；发布按[人工流程](docs/specs/CROSS_PLATFORM_RELEASE.md#7-当前人工发布流程)。Codex 只读核验可运行 `cargo run --manifest-path src-tauri/Cargo.toml --example verify_codex_limits`，不写 Providock 数据库、不输出身份或令牌。真实价格只读核验可运行 `cargo run --manifest-path src-tauri/Cargo.toml --example verify_aihot_prices`，仅写内存测试库。

| 文档                                                                                                        | 内容                 |
| ----------------------------------------------------------------------------------------------------------- | -------------------- |
| [开发手册](docs/DEVELOPMENT_HANDBOOK.md) / [AGENTS](AGENTS.md)                                              | 接手流程与工程约定   |
| [产品需求](docs/PRODUCT_REQUIREMENTS.md) / [路线图](docs/ROADMAP_AND_ACCEPTANCE.md)                         | 用户决定与待验收任务 |
| [架构](docs/ARCHITECTURE.md) / [架构决策](docs/adr/0001-core-decisions.md)                                  | 模块、IPC 与取舍     |
| [数据契约](docs/specs/DATA_CONTRACTS.md) / [界面设计](docs/specs/UI_UX.md)                                  | 统计口径与交互       |
| [Agent 规范](docs/specs/AGENT_ADAPTERS.md) / [账户规范](docs/specs/ACCOUNT_ADAPTERS.md)                     | 来源与适配边界       |
| [兼容登记](docs/research/compatibility-register.md) / [AIHOT 证据](docs/research/evidence/aihot-pricing.md) | 样本、实机与价格依据 |
| [安全隐私](docs/specs/SECURITY_PRIVACY.md) / [软件更新](docs/specs/APP_UPDATES.md)                          | 凭据、网络及更新策略 |

MIT 许可证；上游参考与版权说明见 [THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES.md)。
