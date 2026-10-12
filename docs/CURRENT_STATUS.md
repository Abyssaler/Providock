# 当前状态与交接（2026-10-12）

本文是交接事实表：每块工作按 [开发手册](DEVELOPMENT_HANDBOOK.md) 第 10 节的格式给出，并遵守 [AGENTS.md](../AGENTS.md) 的交付约定。状态分级：`样本验证`（合成夹具+独立预期值通过）< `macOS 实测` < `live_verified`（真实凭据/双平台）。**当前没有任何条目达到完整 live_verified。各版本发行依据与验证边界分别记录。**

## 怎么跑起来

```bash
pnpm install --frozen-lockfile
pnpm tauri dev        # 只读 Agent 默认目录；写入 Providock 自有数据库，默认启动自动监测
pnpm test && cargo test --manifest-path src-tauri/Cargo.toml
```

实机只读冒烟（不写任何文件、不输出正文）：`cargo test --manifest-path src-tauri/Cargo.toml --test live_smoke -- --ignored --nocapture`

## 本次交付验证

自动监测、逐账户截止时间、持久化开关、用量页与 Codex v2 的行为和验证步骤详见 [验证记录](research/validation/2026-10-11-auto-monitoring.md)，版本变更见 [CHANGELOG](../CHANGELOG.md)。本机已通过前端 48 项测试、TypeScript/Prettier/Vite 构建、Rust 完整回归、fmt 与 Clippy；两个需主动读取真实环境/凭据库的测试仍为 ignored，Kimi/Codex 另用只读 usage-only 快照独立核对。Chrome 模拟 IPC 验证不等于原生桌面端到端，CI 构建不等于 Windows 实机验收。

首轮 [GitHub CI](https://github.com/Abyssaler/providock/actions/runs/38137942242)：macOS 检查及测试安装包构建通过；Windows 编译/前端检查通过，但新增 Codex 升级回归测试因合成身份中的路径分隔符不一致失败（应 2 条、实际 3 条）。已修正测试为读取实际游标身份，保留全部计数断言，并通过本机相关回归与 Clippy；修复提交 d9f11f9 的 [双平台 CI](https://github.com/Abyssaler/Providock/actions/runs/38138488820) 已全部通过，macOS ARM64 / Windows x64 安装包均已产出；仍不替代 Windows 实机验收。

## v2.0.2 改进（2026-10-12）

用户已在比较三种方案后明确选择 A：参考 CC Switch 的本地明文保存；详见 [方案比较与决定](research/2026-10-12-credential-storage-options.md)。

- T03/T09：用户要求避免系统钥匙串授权；生产默认 LocalPlaintextVault，独立 credentials/keys.sqlite3 明文保存，重启直接使用；Unix 目录 0700/文件 0600，Windows 继承用户目录 ACL。旧 Keychain 不读取/迁移/删除，已有账户需重新输入一次。账户与快照保留；不得称为加密。此行为不同于已公开 v2.0.0 包。
- T12：账户成功快照改经显式 IPC DTO，所有嵌套字段统一 camelCase；领域/适配器夹具仍保留原命名。真实 DB→IPC 与前端共享合成夹具覆盖余额、窗口、指标；页面错误兜底保证导航可用。
- T12/T14：新增多分量趋势（当天按小时、其余 UTC 日）、模型/Agent 汇总、20 条分页明细及模型定价 UI/IPC。规则按 Agent+模型+可选来源，输入为 USD/CNY 每百万 Token 费率；追加版本，停用也留版本。显示“当前定价估算”，按用户当前规则比较全部筛选记录，不改写既有费用、估值或 Token。
- 本机验证：前端 50 测试、Rust 321 通过/2 ignored、类型/构建/fmt/Clippy；合成 IPC 浏览器检查浅深色、960px、模型筛选、定价保存、分页及有快照账户。未做 Windows 实机或真实账户联网验收。证据见 [记录](research/validation/2026-10-12-accounts-model-pricing.md)。

## v2.0.0 发布

[正式 Release](https://github.com/Abyssaler/Providock/releases/tag/v2.0.0) 已公开并设为 Latest，固定提交 2662489 的 [双平台 CI](https://github.com/Abyssaler/Providock/actions/runs/38195092032) 全部成功；10 项公开资产的远端摘要匹配，实际包内清单未含本机数据或签名秘密。本机 Mac 已安装 v2.0.0，启动、数据库完整性、旧事件保留与自动扫描周期观察通过。完整来源、校验和及验收边界见 [发布核验记录](research/validation/2026-10-12-release-v2.0.0.md)。

维护者在 2026-10-12 明确要求将当前实现以 **v2.0.0 正式 Release / Latest** 发布，替代尚未发布的 1.1.0-preview.1 候选。[发布说明](releases/v2.0.0.md) 保留八源矩阵、已知缺口和手动安装方式；正式发布标记不代表 T15/T16、真实账户或所有 P0 门槛已经验收。

此前 [候选 CI](https://github.com/Abyssaler/Providock/actions/runs/38139628694) 的 macOS 构建通过，但下载包的链接器签名没有封装完整应用资源；本机副本补签后曾安装，现已替换为 v2.0.0。Windows 测试通过，默认 MSI 打包因预发布后缀失败。本次改为显式 NSIS、Mac 完整 ad-hoc 签名/严格校验、签名资产齐全检查；正式资产必须来自 v2.0.0 对应完整提交成功的双平台 CI。

Tauri/minisign 采用既有发布公钥及 CI Secrets，只给 push 构建签名；PR 禁用更新包签名生成，不需要私钥。该密钥不是 Apple Developer ID 或 Windows Authenticode 证书。

旧 v1.0.0 属于重建前实现，标识为 com.providock.desktop；当前标识为 io.providock.app，数据目录和凭据域独立。手动安装不会自动迁移旧账户/统计。当前未接入自动更新 UI，不发布旧版 latest.json。

## 逐任务状态

```text
任务：T00（证据与兼容登记）
状态：已验证（研究级）。五份证据文档在 docs/research/evidence/，含逐行 URL。
下一步：T00-G 的 Claude Desktop 实机演示（需要装 Desktop 的用户手工配合）。

任务：T01（工程骨架）
状态：已验证（macOS ARM64：typecheck/lint/test/build/dmg 打包全过；Windows x64 CI 检查/测试/安装包构建通过，实机安装运行待验）。
实现：Tauri 2 / Rust 1.99 / React 19 / TS 5.9 / pnpm 12.11.2；标识 io.providock.app 已冻结；
      CI=.github/workflows/ci.yml（macos-15 + windows-2022，产出安装包与签名更新包；Mac ad-hoc 签名）。

任务：T02（契约/存储/游标/去重）
状态：已有核心单测验证（逐批事务回滚、重扫幂等、替换不累加、墓碑、溢出、禁存秘密列）；完整发布门槛未满足。
实现：src-tauri/src/domain/、storage/（migrations.rs 是唯一的 schema 真相）、ingestion/。
注意：schema_version 与 parser_version 分开；改契约先升版本再迁移。
缺口：通用整源 parse_run 原子切换尚未实现，失败运行不能假定回滚已提交的早期批次；
      截断/未知格式替换时历史保留，以及既有 schema v1 的升级路径仍需补完。
      本次没有变更 DB schema；Codex v2 只对已证实重复身份执行逐批精确退休。

任务：T03（凭据库/路径守卫/诊断脱敏）
状态：样本验证；真实 Keychain 往返测试存在但 #[ignore]（避免弹窗），Windows 凭据库未实机。
实现：src-tauri/src/vault/（无明文回退）、safepath.rs、services/redact.rs。
缺口：真实钥匙串/Windows Credential Manager 实测；锁定时 UX。

任务：T04（Codex/Kimi 纵线）
状态：macOS 只读计量快照核对（Kimi：63 文件、4147 事件、697159437 Token；Codex：10 文件、
      338 事件、38817948 Token；两者全量/当天的六项 Token 与独立计算一致，重扫守恒）。
      数字为本次快照，活跃日志会继续增长；不代表平台订阅额度。
实现：adapters/agents/{kimi_wire,codex_rollout}.rs + kimi_legacy.rs（旧 Python 协议 1.10，
      含子 Agent 镜像防双算）；ingestion/scanner.rs；services/aggregate.rs；IPC+用量页。
新增：默认启动同步、之后每 60 秒扫描，两类独立开关持久化；手动/自动触发合并。
      Codex v2 修正日期目录漏扫、total+last 重播及零心跳计数；升级只退休已证明的旧重复身份，
      子会话更新可使用未变化父文件的时间线。核对脚本 scripts/verify_local_usage.py。
缺口：旧 Python Kimi 未接扫描器（需 ForkContext 与镜像判定）；Windows 未验；
      独立实机核对不覆盖累计重置、fork 与压缩日志（遇到时明确停止，相关语义只经夹具验证）。

任务：T05–T07（A01/A03/A04/A05/A06/A07 适配器）
状态：样本验证（夹具+人工预期）；A05 OpenCode 另获 macOS 实测（真实 V2 库 249 事件）。
实现：adapters/agents/{claude_code,gemini_cli,grok_build,opencode_db,pi_log,mcode_db}.rs，
      全部接入 scanner.rs；fixtures/agents/ 下各夹具与 README 手工算术。
缺口：Gemini/MCode/OpenCode 的 output↔reasoning 包含关系为保守口径（少加不重复），
      待官方样本确认；Grok 归档/活跃双副本同会话会各计一次（解析器既定行为，待收敛）。

任务：T08（可选网关）
状态：解析层样本验证（4 协议×5 形态 20 夹具：fixtures/gateway/）；**转发/监听本体未实现**；
      T00-G 实机演示未做。实现：src-tauri/src/gateway/usage.rs。
下一步：实现 loopback 转发（随机令牌、目标白名单、断流标 partial）+ Desktop 演示。

任务：T09+T10（账户域与 B01–B07）
状态：服务层回归通过；适配器全部**样本验证**（mock listener + 合成夹具），
      未做真实凭据实测。实现：services/accounts.rs、account_query.rs（逐跳重定向剥
      Authorization 有 mock 验证）、account_scheduler.rs（已接后台循环与独立开关）、
      adapters/accounts/（common.rs 收口）+ fixtures/accounts/。
缺口：Codex App Server 是进程协议只占位 unsupported；GLM 双 TOKENS_LIMIT 区分待官方证据；
      usage_metrics 表就绪但暂无写入方（只有 B07 能产出）；Retry-After/指数退避、
      credential/scope 级并发与手动限频仍待实现，当前只有同账户请求排重。
新增：逐账户到期时间（余额 15 分钟/计划 5 分钟、失败及手动均计入尝试），关闭时跳过启动首轮；
      HTTP 已移出 DB 锁，提交复核删除/编辑/换凭据，同账户并发不重复发请求，刷新事件更新账户页。
验证：截止时间/禁用首轮/无账户、锁外 HTTP、并发合并、请求期间删除均有回归；未真实账户联网。

任务：T12（四页 UI）
状态：四页与调度设置可用（前端 48 测试）。用量页参考所给 cc-switch 截图重排，加入来源/模型/日期
      筛选、显示刷新选择、指标覆盖率和未知趋势断点。
验证：Chrome + 模拟 IPC 检查筛选、手动同步、两开关刷新后保存、设置入口、浅/深色及 960px 无横向溢出；
      这是前端交互验证，非原生 Tauri/Windows 全流程验收。
缺口：隐私开关/清理/导出 UI、完整截图矩阵（缩放/长别名/多币种）、可选时区未做。

任务：T13（正文查看）
状态：样本验证 + 安全边界测试（safepath 逃逸拒绝、默认关闭、不落库）。
实现：adapters/content/（kimi/claude/codex 三读取器）+ services/session_queries.rs + 会话页抽屉。
缺口：其余源返回 UnsupportedSource；跨文件活跃链重建 TODO；Codex .zst 正文不支持。

任务：T14（导出/清理/价格）
状态：服务层样本验证（23 测试）；**未接 IPC/UI**（export_statistics 等在服务层待命令化）。
实现：services/{export,cleanup,pricing}.rs。

任务：T15/T16（双平台实机/性能）
状态：部分完成。v2.0.0 双平台 CI/发行包核验通过；Mac 安装、启动、旧事件保留与自动扫描冒烟通过。
      完整原生 GUI/凭据库、Windows 实机、睡眠唤醒、通用升级/回滚及百万事件基准未做。
```

## 关键不变量（改动前必读）

1. 未知 ≠ 0；无价格 ≠ 免费；本机账本与平台账户永不合并。
2. Token 在 IPC/JSON 是十进制字符串；SQLite 单值 ≤ i64::MAX；聚合用 u128。
3. 适配器是纯函数：不触网、不碰凭据库、不写源文件；证据→夹具→解析器→入库→UI 的顺序不变。
4. 相同数字不是重复；event_id 是原生 ID 或「文件身份+代+偏移」，不是数值哈希。
5. 累计快照做差分或当基线，不求和；快照重放/重置有显式规则。
6. 凭据按用户选择仅进独立本地明文文件；诊断/日志/统计导出不出现秘密、路径（用 redact::anon_file_id）、正文。
7. 网关默认关闭；未关联的网关流量与本地日志分域统计。

## 协作约定

- 文件归属按任务划分；共享文件（mod.rs/lib.rs/migrations.rs/契约文档）先协调再改。
- 交付即更新 docs/research/compatibility-register.md 的「实现状态」表与本文件。
- 测试纪律：采集语义、去重、凭据隔离、迁移必须有回归；样本验证 ≠ 实机验证，措辞不混淆。

## v2.0.2 界面和菜单栏交付

账户白屏/本地明文 Key、模型定价和模型/Agent/明细统计随 v2.0.2 一起交付。全局纯黑白字、黄绿强调和原创黑底图标已实现；修复旧 trend-line 样式覆盖所有 stroke 为紫色，蓝/绿/紫/橙四分量使用平滑曲线，缺失保留断点。macOS 原生菜单栏提供最近 24 小时计量小计和八 Agent、同步状态、打开/同步/退出，关窗隐藏后台运行，退出时取消调度；Windows 保持原关窗退出。

本机自动化验证：52 项前端测试及 322 项 Rust 测试通过（2 项真实环境测试忽略），类型检查、格式、Clippy 与本地包构建通过。Mac 原生界面验证因电脑控制权限未授予而未完成；本次双平台 CI/发行包核验进度见 [v2.0.2 核验记录](research/validation/2026-10-12-release-v2.0.2.md)。
