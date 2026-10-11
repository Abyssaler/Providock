# 当前状态与交接（2026-10-11）

本文是交接事实表：每块工作按 [开发手册](DEVELOPMENT_HANDBOOK.md) 第 10 节的格式给出，并遵守 [AGENTS.md](../AGENTS.md) 的交付约定。状态分级：`样本验证`（合成夹具+独立预期值通过）< `macOS 实测` < `live_verified`（真实凭据/双平台）。**当前没有任何条目达到完整 live_verified。**

## 怎么跑起来

```bash
pnpm install --frozen-lockfile
pnpm tauri dev        # 只读 Agent 默认目录；写入 Providock 自有数据库，默认启动自动监测
pnpm test && cargo test --manifest-path src-tauri/Cargo.toml
```

实机只读冒烟（不写任何文件、不输出正文）：`cargo test --manifest-path src-tauri/Cargo.toml --test live_smoke -- --ignored --nocapture`

## 本次交付验证

自动监测、逐账户截止时间、持久化开关、用量页与 Codex v2 的行为和验证步骤详见 [验证记录](research/validation/2026-10-11-auto-monitoring.md)，版本变更见 [CHANGELOG](../CHANGELOG.md)。本机已通过前端 48 项测试、TypeScript/Prettier/Vite 构建、Rust 完整回归、fmt 与 Clippy；两个需主动读取真实环境/凭据库的测试仍为 ignored，Kimi/Codex 另用只读 usage-only 快照独立核对。Chrome 模拟 IPC 验证不等于原生桌面端到端，CI 构建不等于 Windows 实机验收。

## 逐任务状态

```text
任务：T00（证据与兼容登记）
状态：已验证（研究级）。五份证据文档在 docs/research/evidence/，含逐行 URL。
下一步：T00-G 的 Claude Desktop 实机演示（需要装 Desktop 的用户手工配合）。

任务：T01（工程骨架）
状态：已验证（macOS ARM64：typecheck/lint/test/build/dmg 打包全过；Windows 待 CI 实机）。
实现：Tauri 2 / Rust 1.99 / React 19 / TS 5.9 / pnpm 12.11.2；标识 io.providock.app 已冻结；
      CI=.github/workflows/ci.yml（macos-15 + windows-2022，产出未签名安装包）。

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
状态：未开始。macOS 安装包可构建；Windows 完全未实机；百万事件基准未做。
```

## 关键不变量（改动前必读）

1. 未知 ≠ 0；无价格 ≠ 免费；本机账本与平台账户永不合并。
2. Token 在 IPC/JSON 是十进制字符串；SQLite 单值 ≤ i64::MAX；聚合用 u128。
3. 适配器是纯函数：不触网、不碰凭据库、不写源文件；证据→夹具→解析器→入库→UI 的顺序不变。
4. 相同数字不是重复；event_id 是原生 ID 或「文件身份+代+偏移」，不是数值哈希。
5. 累计快照做差分或当基线，不求和；快照重放/重置有显式规则。
6. 凭据只进系统凭据库；诊断/日志/导出不出现秘密、路径（用 redact::anon_file_id）、正文。
7. 网关默认关闭；未关联的网关流量与本地日志分域统计。

## 协作约定

- 文件归属按任务划分；共享文件（mod.rs/lib.rs/migrations.rs/契约文档）先协调再改。
- 交付即更新 docs/research/compatibility-register.md 的「实现状态」表与本文件。
- 测试纪律：采集语义、去重、凭据隔离、迁移必须有回归；样本验证 ≠ 实机验证，措辞不混淆。
