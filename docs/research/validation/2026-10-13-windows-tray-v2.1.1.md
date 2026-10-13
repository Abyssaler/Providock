# Windows 托盘与 v2.1.1 核验（2026-10-13）

## 范围和基线

用户最新明确要求 Windows 关闭隐藏到托盘，托盘右键退出才停止；验证后推送并发行，再仅为本机配置官网模型价格，最后由用户自行更新正式应用。该决定替代原 Windows 关闭即退出约定。不改 Mac 行为、数据库 schema、依赖版本或正式更新身份/公钥/地址。

仓库克隆到本机非同步工作区，main 为 `f73cc05a7a092839ed994f7c0a97886ff426fd3c`，无未提交改动；v2.1.0 标签为 `fcebe85e678abfd913f7e92360831ae10e0cd09f`。Git 直连失败，使用本机已有代理进行单命令配置，未永久修改代理或执行策略。

## 本机环境与基线

- Windows 11 25H2，build 26200.9457，AMD x64；不是 ARM64 仿真。
- Git 2.54.0.windows.1，Node 24.15.0；工作区安装 pnpm 12.11.2、Rust 1.99.0 x86_64-pc-windows-msvc（rustfmt/clippy），不改全局 PATH。
- WebView2 Runtime 147.0.3912.72。
- 冻结安装、typecheck、lint、80 项前端测试和 Vite build 通过；Rust fmt 基线通过。
- 初始无 MSVC/SDK；旧微软引导器失败 5008（安装器版本不匹配），当前 Build Tools 2026 引导器失败 1602（取消）。用户配合安装中。首次 Clippy 明确失败于 `link.exe` 缺失，不能作为代码失败或通过结论。

## 实现和本机验证

`windows_tray.rs` 仅 Windows 编译。setup 必须先成功创建托盘；CloseRequested 只隐藏 main，隐藏成功才阻止关闭。左键释放/菜单打开解除最小化、显示、聚焦。退出直接 app.exit(0)，沿用 ExitRequested 取消后台任务。无新线程、轮询或新依赖。macOS 实现不变。

用户随后完成 Visual Studio Community 2026 18.10.3 安装：MSVC 14.51.36231、Windows SDK 10.0.26100.0，不要求重启。新 PowerShell 进程显式导入 DevShell 后找到 cl/link/rc；不依赖旧 GUI 的 PATH。

- Rust fmt、Clippy all-targets/locked/-D warnings 通过；cargo test --locked 实际 359 项通过/4 ignored（Windows 条件编译数量），不启用真实环境测试。
- `pnpm tauri dev --config` 实际启动原生 Windows Review 窗口；独立 identifier `io.providock.winreview`，实际读取 Roaming 下同名目录。使用 prepare_local_review 全新合成库，UI 显示 4 请求、612 Token（输入 150、输出 62、缓存命中 400）。自动采集/账户刷新关闭。
- 本地覆盖禁用签名产物并清空更新 endpoints，隔离应用设置中另关闭自动更新；正式配置的 identifier、公钥和地址保留。
- 实际点击原生标题栏关闭后，可见窗口列表不再包含 Review，原进程继续运行。
- 任务栏不是当前自动化工具暴露的目标窗口，四项交互由用户实机完成并明确答复「四步全部正常」：托盘左键恢复、最小化后恢复、菜单打开、最后右键退出。随后系统进程列表确认 providock 进程消失。
- 本地隔离 NSIS 构建成功，安装器约 4.45 MiB，SHA-256 `ea713c576f4ea71c0db467b82baaf975ff5a03a37dd3fb208635c21ce0c4faca`。清单仅应用及标准安装运行文件。未运行安装器或替换正式应用；此隔离包不进入正式 Release。

正式安装、覆盖、实际更新由用户操作；完整缩放、休眠、九 Agent 真实日志及真实付费模型解析不在本次托盘改动的已验范围。仅本机价格操作和备份不提交、不进入发行资产。

## 同提交发行与公开核验

- 代码/标签 `v2.1.1` 固定 `b3500e33d36932c26e0582d408b468d3fc87e082`，main 已推送；后续仅补记文档，不移动标签或替换资产。
- [CI 38340576160](https://github.com/Abyssaler/Providock/actions/runs/38340576160) 的 macos-15/windows-2022 均成功，包含冻结安装、前端检查/测试、Rust fmt/Clippy/tests、构建、签名资产检查；Mac codesign 完整性检查通过。
- 正式包全部来自该次 CI：Mac artifact `11786528620`，Windows artifact `11787036202`，未混用本地 Review 包。
- 三个二进制使用 Python cryptography 独立执行 Ed25519/minisign 验签，文件签名、可信注释全局签名、公钥 key ID、文件名和版本均吻合。更新清单的两个 URL/签名分别对应已验资产。
- Mac 更新归档只含 Info.plist、主程序、icon.icns、CodeResources 四文件，未发现链接/绝对路径/上级路径；identifier `io.providock.app`、版本 2.1.1、arm64 核实。Windows 主程序解包核实 PE machine `0x8664`、FileVersion/ProductVersion 2.1.1；NSIS 仅 8 项标准文件。本机未运行 Mac 的 hdiutil 或安装任一正式包。
- [Release](https://github.com/Abyssaler/Providock/releases/tag/v2.1.1) ID `411226995`，发布时间 `2026-10-13 11:26:52 +08:00`（UTC `2026-10-13T03:26:52Z`）；非 draft、非 prerelease、Latest。
- 草稿先核对 11 项资产白名单、大小、GitHub SHA-256，再公开。公开后匿名回下载全部 11 项，摘要一致；回下载二进制再次独立验签通过，固定 Latest 更新端点返回 2.1.1，公开标签 SHA 一致。公钥与 v2.1.0 逐字节相同。
- 无 schema/依赖变更；正式安装、覆盖、实际更新、数据保留由用户后续操作。发行不替代上述未验项。Windows 无 Authenticode，Mac ad-hoc 完整性签名不等于 Developer ID 公证。

## 公开资产 SHA-256

`SHA256SUMS.txt` 包含其余 10 项，下表另记录自身摘要。

| 文件 | 字节数 | SHA-256 |
| --- | ---: | --- |
| LICENSE.txt | 1110 | `426d2c7037ff8de2f915cbaf6b456288a86871929e05106f25b33f77ad892432` |
| providock-v2-latest.json | 2431 | `3ce252af8fc85ca161651e7186a698f78b10f0b274da67386eee38247d222363` |
| Providock.app.tar.gz | 6012199 | `998ed3a4e41cc0cd5b5f7b2fb918184b78eb5259d31d307d4aa16b6e85bdb9ec` |
| Providock.app.tar.gz.sig | 428 | `c3954a501f4d2b1d73138458068da51b5ab3103cebd6cb8a6e671e28e054f6a4` |
| providock.key.pub | 153 | `b7ed57b72c93cd9e2379d39a57a7265eee5c7f4785c098bcc782c7c4021c9cbc` |
| Providock_2.1.1_aarch64.dmg | 5852122 | `21a1d0e032173c3d2291897b4fad1dac6408ba9b995d21818945733a8e599cb5` |
| Providock_2.1.1_aarch64.dmg.sig | 436 | `46bb2f0ba82a706211724a662ebb9a2fcabd0a17de878d3d5eaacb09d6c7743e` |
| Providock_2.1.1_x64-setup.exe | 4648672 | `6ddc3bf847a244732bb8d44d5e99417edf8fefe599f493603a9c96687449d791` |
| Providock_2.1.1_x64-setup.exe.sig | 440 | `da1bafd9fc2171159b03cc3d5edd2829abcd20bc0d34790645f4ef7d186f35b9` |
| SHA256SUMS.txt | 908 | `64b47ca14d117c22bff4c25506f83cde62904c46532578ea9861ae033bcc73af` |
| THIRD_PARTY_NOTICES.md | 2982 | `200156f350f7ef22d35a5f8f63a34c25f1672bf2b78ef189d9e2102baced60e3` |
