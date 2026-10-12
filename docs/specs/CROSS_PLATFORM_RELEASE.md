# 跨平台与发布设计

## 1. 首版平台

| 平台 | 目标 | 安装形式 | 支持定位 |
| --- | --- | --- | --- |
| macOS Apple Silicon | aarch64-apple-darwin，拟定 macOS 13+ | .app + .dmg | 正式支持；Intel Mac 后续评估 |
| Windows Intel/AMD | x86_64-pc-windows-msvc，Windows 11 x64 | 每用户普通 .exe 安装包（NSIS） | 正式支持；不要求管理员是目标，按安装策略验证 |
| Windows 10 22H2 x64 | 相同架构 | 同安装包 | 独立兼容验收目标，未验证前不能宣称正式支持 |
| Windows ARM64 | aarch64-pc-windows-msvc | 后续原生构建 | 不在首版承诺；x64 仿真运行不能视为已原生支持 |
| Linux / Windows 便携版 | 待定 | 后续 | 非首版发布门槛 |

x64 是普通 Intel/AMD Windows 电脑常见架构，Windows 也有 ARM64，不能说所有个人电脑都是 x64。[微软 Windows on Arm](https://learn.microsoft.com/en-us/windows/arm/overview)。此处为产品支持策略，不是 Tauri 的理论最低系统要求。

## 2. 目录与系统差异

Providock 自有数据目录通过 Tauri/系统 API 解析（拟定标识 `io.providock.app`），避免跨系统硬编码。应用数据库、设置、缓存、诊断分开；凭据按用户选择位于独立 credentials/keys.sqlite3 明文文件，不访问系统凭据库。包标识在首次发布前冻结，因为改变会影响目录和凭据访问。

Agent 的目录遵循各自规则，不等同 Providock 数据目录。用户目录、环境变量、XDG、Windows APPDATA/LOCALAPPDATA 逐项核实；GUI 不一定继承 shell 的环境变量。UI 支持选择自定义目录。中文、空格、长路径、大小写、符号链接/junction、外置盘断开均需验证。

文件观察使用平台兼容库，监视事件可能丢失，低频对账补偿。Windows 文件共享模式不得阻止 Agent 继续追加写；文件被轮转/删除时释放句柄。SQL 只读连接与 WAL 不依赖复制主 DB 文件；如必须做副本，使用一致性快照策略，不能漏掉活动 WAL。

WSL 文件作为用户主动选择的额外源，Windows 原生与 WSL 是独立实例；不默认执行 wsl 命令、访问所有发行版或将两份相同来源重复计入。网络盘不作为实时延迟保证范围。

## 3. 构建与依赖

CI 分别在原生 macOS ARM64 runner 和 Windows x64 runner 构建/测试；若托管 runner 规格不符，选用对应自托管 runner或明确交叉构建限制。锁定 Rust toolchain、Node 活跃 LTS、pnpm 与 lockfiles；不能在文档给出未经验证的漂移版号。

Rust SQLite、TLS、系统凭据库和文件监视库都做两个 target 的编译验证。前端不要依赖仅 Chromium 存在的 API，需覆盖 macOS WebKit 与 Windows WebView2。Windows 安装器检测 WebView2，提供官方安装引导/引导安装，不假设所有电脑已装；离线安装策略在发布时说明依赖。

macOS 正式分发需要 Developer ID 签名与公证，Windows 代码签名用于可信分发。没有证书时只发明确标注的测试构建，不伪称已签名。签名凭据放 CI secrets，PR/外部贡献构建不得读取发布密钥。

## 4. 发布流水线

依次完成：锁文件安装 → 类型检查/前端测试 → Rust fmt/clippy/tests → 两平台构建 → 安装运行冒烟 → 凭据库/文件读取/账户 mock验收 → 原生签名与公证 → 校验和、许可证/第三方清单 → release notes → 手动发布。

v2.0.3 根据用户明确要求接入定期检查通知及用户点击后的签名安装；没有无人值守自动安装。当前不变更 schema，后续 migration 更新仍须单独满足备份/兼容/恢复门槛。不能为了更新检查上传账户、日志或凭据。发布资产按 OS/架构清晰命名；发布说明附适配器版本矩阵、未实测项和升级说明。

## 5. 升级、回滚、卸载

自有 DB migration 带版本、事务、升级前可恢复备份；源 Agent 数据不迁移。升级失败保留旧数据和错误说明。回滚旧应用不能盲读新 schema，提供版本检查/恢复备份。解析器版本升级与 DB schema 升级分别管理。

正常卸载默认保留统计/账户元数据，让用户重新安装可恢复；UI 提供明确的“清理本地数据及凭据”操作。卸载脚本不擅自删用户 Agent 日志或厂商配置。便携版后续必须重新设计数据目录、密钥库与移动介质语义，不能简单压缩安装目录便宣称完成。

## 6. 发布验收矩阵

两个正式目标都必须验证：全新用户安装、启动、升级、卸载重装、中文路径、缺源目录、文件同时追加、SQLite WAL/锁定、历史重扫不变、凭据保存/更新/删除、断网/代理、睡眠唤醒、深色模式、缩放、网关默认关闭/启停/端口冲突、正文选择性读取。

记录系统版本、CPU 架构、Agent/协议版本、安装包 hash、签名状态、测试步骤和结果。真实 Windows 环境缺失会阻塞正式发布，不能把通过 cross-compile 当运行验收。CI 与人工报告共同构成发布证据。

## 7. 当前人工发布流程

维护者在 2026-10-12 明确要求以 v2.0.0 正式 Release / Latest 发布当前实现，此决定优先于此前只发预览的安排；现有验证缺口、产品范围和后续验收目标保持如实登记，不能据正式标签把未验项目改为已验证。

当前应用标识固定为 io.providock.app，历史 v1.0.0 为 com.providock.desktop；不自动迁移旧数据，不使用旧 updater 协议，不发布旧版 latest.json。v2.0.3 起通过官方 updater 插件接入应用内更新，使用独立 providock-v2-latest.json；旧 v2.0.2 及更早安装仍需手动替换一次，详见 [更新规范](APP_UPDATES.md)。

1. 统一 package.json、src-tauri/tauri.conf.json、src-tauri/Cargo.toml 和 Cargo.lock，新增 docs/releases/ 对应说明；同步 CHANGELOG、README、当前状态、决策及兼容表。
2. macOS 显式构建 app,dmg（只构建 dmg 会清理中间 .app，且不会生成 .app.tar.gz 更新包），signingIdentity 为 -，使完整应用包 ad-hoc 签名；CI 用 codesign --verify --deep --strict 校验。Windows 显式构建 NSIS，不默认额外生成 MSI。
3. 启用 createUpdaterArtifacts，通过已有 GitHub Secrets 提供 Tauri/minisign 私钥与密码，配置中只存公钥。PR 禁用更新包生成且不读取私钥；发布 push 要求安装包、更新包及签名齐全。密钥不是 Apple Developer ID 或 Windows Authenticode 证书。
4. 推送版本提交到 main，确认该完整 SHA 的两个 CI job 成功。用 gh run download 下载同一次运行的两个平台产物，核对包内版本/架构/完整性、签名及 SHA-256，不混用旧构建。
5. 在确切提交创建版本标签，先创建草稿并上传安装包、更新包、签名、公钥、许可证与 SHA256SUMS.txt；v2.0.3 起用 scripts/create-updater-manifest.mjs 从已独立验签的两平台 .sig 生成 providock-v2-latest.json（2 个 platform），清单也计入摘要。11 项资产白名单：DMG+sig、app.tar.gz+sig、Windows EXE+sig、providock.key.pub、LICENSE.txt、THIRD_PARTY_NOTICES.md、providock-v2-latest.json、SHA256SUMS.txt。核对后将当前版本设为非 prerelease、非 draft、latest=true，旧标签/资产不覆盖。
6. 从新 Release 下载 Mac 包、核对 hash、保留已有应用数据、安装并检查启动。Windows CI 不等于 Windows 实机验收；后续结果在验证记录中补记。

正式证书签名/公证、双平台完整实机、迁移/回滚与性能门槛仍按上文规划继续完成，不因本次发布决定省略验证事实。

发布包只使用 CI 干净检出构建的白名单资产；本机数据库、日志、账户配置、私钥、密码及本机备份不得进入发行包。发布前检查 app/tar 文件清单与安装包来源，只有公钥用于公开验签。

## v2.0.2 发布授权与核验

用户明确要求验证后推送 GitHub 并发布 v2.0.2；沿用第 7 节同一提交双平台 CI、独立验签、包清单、草稿核对再公开的步骤，版本替换为 2.0.2，历史 v2.0.0 标签/资产不覆盖。macOS 需验证菜单栏可见、关窗后进程持续、菜单恢复窗口、⌘Q 和菜单退出；Windows 未增加托盘关窗策略，仍按原流程退出。Windows 原生实机与真实账户联网边界不因发布而改标。
