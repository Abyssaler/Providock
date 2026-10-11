# 自动监测与用量页交付验证（2026-10-11）

本记录对应 T04/T09/T10/T12 的本次改动，区分正式代码、合成测试、macOS 日志核对与未完成验收；不将规划当作实现，也不将本机日志当作平台账单。

## 1. 问题与最终行为

| 原问题 | 结果 | 主要实现 |
| --- | --- | --- |
| Agent 用量依赖手动采集 | 默认启动同步一次，每轮结束后等待 60 秒；不启动文件监听，已有扫描时合并触发 | services/usage_sync.rs、services/background.rs |
| 每个短周期查询所有账户 | 逐账户按最近查询尝试与类型计算截止时间，只查询到期账户 | services/account_scheduler.rs |
| disabled 检查晚于启动首轮 | 自动任务在首次读取/查询前检查持久化开关，源/账户之间再次检查 | 两个调度服务 |
| 没有可见开关 | 用量页与设置页提供两开关，事务保存后唤醒对应任务 | lib.rs、MonitoringControls.tsx |
| 用量界面缺少参考页的筛选/汇总 | 八 Agent、来源、模型、日期、独立显示刷新、Token/缓存/费用/覆盖率、趋势 | UsagePage.tsx、aggregate.rs |
| 实机 Codex 数据漏算/重复 | 修正日期深度、累计重播和零心跳，v2 精确退休旧重复身份 | scanner.rs、codex_rollout.rs、ingestion/mod.rs |

账户默认余额 900 秒、计划/混合 300 秒，加 ±10% 抖动；抖动固定于该账户的本次尝试。成功/失败快照包含手动查询；未落快照的自动失败用运行期尝试时间避免忙循环。有新近快照的账户启动时延续截止时间，不强制刷新。当前同账户并发请求会拒绝第二次发送，不等同于跨账户 credential/scope 合并。

Agent 与账户开关分别为 `usage.auto_sync_enabled`、`accounts.scheduler.enabled`，复用 settings 表，不修改 DB schema。关闭后任务只等待通知，不轮询设置；开启/相关账户变化会唤醒重算。已有阻塞读取或 HTTP 可以收尾，停止后不开始下一项。关闭窗口停止两任务，未实现系统自启动或独立守护进程。

显示刷新默认 30 秒，可选关闭/5/10/30/60 秒，只发本地统计 IPC；隐藏页面暂停其定时查询。自动扫描完成事件仍可更新页面。后台 60 秒周期和账户截止时间不受显示选择影响。扫描器沿用自身游标/mtime/WAL 策略，变化文件部分仍整文件重读；未测得空闲 CPU/RSS，不能承诺资源目标已达标。

## 2. 依据和版本

- 产品选择 D10 与 [ADR](../../adr/0001-core-decisions.md) 记录用户确认的自动监测方向。
- cc-switch 对照固定在 `f88068b9e59a43379e14cc8f294467aebb71e26b`；60 秒后台任务、独立显示刷新与 Codex 快照处理的源码链接见 [证据补充](../evidence/a01-a04-ccswitch-importers.md#自动采集交付补充2026-10-11)。
- Codex parser_version：`codex-rollout-1` → `codex-rollout-2`；parser 版本变化强制重读，但保持原文件代数/偏移身份。只删除已证实重复/零心跳身份，与同批事件/游标一起提交，不清空其他已存历史。
- 输入 TOTAL 包含缓存读写子集，fresh 由互斥分量推导；reasoning 不再次累加到 output。未知值和缺失费用保留未知，费用按币种分开。

## 3. 已执行验证

以下是对应的可复现入口。本次前端直接调用已安装且锁定的 tsc、Prettier、Vitest 和 Vite（与 package.json 脚本相同），Rust 使用 --offline --locked，在隔离工作副本中验证；应用到目标仓库后逐文件比较一致。真实源日志只读，未重新安装依赖。

```bash
pnpm typecheck
pnpm lint
pnpm test
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --offline --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --offline --locked --manifest-path src-tauri/Cargo.toml
python3 scripts/verify_local_usage.py
```

前端 48 项测试、类型检查/格式/构建和 Rust 全套回归通过。两个需要主动实机/凭据库访问的 Rust 测试保持 ignored，本次另运行下节只读数字核对，不把 ignored 当作通过。

关键回归包括：余额/计划不同截止时间，失败与手动快照推迟下一次到期，禁用首轮/无账户不查询，禁用等待可唤醒/停止；HTTP 期间数据库锁可用，同账户不重复发请求，请求期间删除不写回；摘要/趋势筛选一致，未知总量不补零；标准日期目录发现，v1→v2 只退休已证实重复，last+total 后接 total-only 不重复累计，子文件追加可对齐未变化父文件。

合成 `replayed-cumulative`：七条 token_count 含相同/相邻来源重播、相同 last 但累计推进的独立请求、零 last 心跳。人工预期两次请求：fresh 160、read 40、write 0、input 200、output 40、total 240；夹具通过。

Chrome + 模拟 IPC 检查通过：筛选参数、手动同步状态、两开关刷新后保留、设置入口、显示刷新不会调用 sync_start、关闭显示定时器停止查询、浅/深色及 960px 宽度无横向溢出。该检查验证前端交互与布局，不代表原生 Tauri 或 Windows 端到端实测；演示数据截图未作为真实用量证据提交。

## 4. macOS 本机计量快照核对

| 来源 | 文件数 | 计量记录 | 全量总 Token | 当天总 Token |
| --- | ---: | ---: | ---: | ---: |
| Kimi Code（wire 1.5） | 63 | 4147 | 697159437 | 251868900 |
| Codex（未压缩、累计单调） | 10 | 338 | 38817948 | 27107332 |

数字固定于本次快照，活跃会话会继续增长；它们不是平台订阅额度。核对六项字段：fresh、cache read、cache write、input total、output total、total，分别检查全量和当天；两者全部一致，再次扫描零新增/零替换且汇总不变。

独立算法：Kimi 对 durable usage.record 的四个互斥分量求和，排除累计状态快照；Codex 使用每个完整会话最终累计量，并以累计推进时间计算当天增量，不拿生产 parser 的 last 求和结果作为预期。

为避免活跃会话在两次读取之间增长，脚本先只读提取无提示词/回复/工具输出/认证内容的 usage-only 临时快照，保持发现所需的相对目录和文件名；Rust 扫描写入内存数据库，结束后删除临时快照。仓库只保留核对器和合成夹具，不提交本机真实日志、用户名、完整项目路径或凭据。

核对器明确限制：Kimi 协议必须为 1.5；Codex 需要 total/last 并存、累计单调；压缩日志、显式 fork 或重置等超出本次独立核对范围时停止说明，不宣称通过。fork、压缩和重置的部分语义由夹具覆盖，Windows 文件/路径行为未实测。

## 5. 仍需完成

- 真实账户余额/计划接口联网、系统凭据库和 Codex App Server 进程协议接线。
- 账户 credential/scope 并发合并、手动平台限频、Retry-After/指数退避与有限重试；当前按账户周期重查。
- 通用整源 parse_run 原子版本切换、截断/未知格式替换时历史保留、既有数据库升级路径。
- 原生 Tauri 全流程、Windows 实机、睡眠唤醒矩阵、百万事件和空闲资源测量。
- 其余 Agent 的实机验证、旧 Python Kimi 扫描接线、网关转发、隐私/导出/清理 UI，详见 CURRENT_STATUS。
