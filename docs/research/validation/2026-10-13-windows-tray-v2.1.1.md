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
- NSIS 构建、同提交双平台 CI、资产验签与公开状态完成后在下方补记；未完成前不宣称通过。未安装测试 NSIS 或替换正式应用。

正式安装、覆盖、实际更新由用户操作；完整缩放、休眠、九 Agent 真实日志及真实付费模型解析不在本次托盘改动的已验范围。仅本机价格操作和备份不提交、不进入发行资产。
