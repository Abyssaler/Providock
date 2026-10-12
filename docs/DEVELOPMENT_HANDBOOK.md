# Providock 开发手册

版本 0.1 · 2026-10-11 · 状态：核心链路与主要 UI 已实现，发布验收尚未完成；逐任务事实见 [CURRENT_STATUS.md](CURRENT_STATUS.md)。

本手册用于人类开发者、Codex、Claude Code、Kimi Code 等后续开发 Agent 接手。先明确统计语义和适配证据，再实现 UI；任何“已支持”结论必须有可重放验证。

## 1. 五分钟接手

1. 先读 [当前状态](CURRENT_STATUS.md)、[产品需求](PRODUCT_REQUIREMENTS.md) 和 [AGENTS.md](../AGENTS.md)，确认已实现范围、验证边界与用户选择。
2. 读 [数据契约](specs/DATA_CONTRACTS.md)，特别是互斥输入、reasoning 包含关系、累计/增量与未知值。
3. 在 [路线图](ROADMAP_AND_ACCEPTANCE.md) 选一个任务编号，查其依赖和完成门槛。
4. 查相关 adapter 的 [采集规范](specs/AGENT_ADAPTERS.md) 或 [账户规范](specs/ACCOUNT_ADAPTERS.md)，从 [研究证据](research/UPSTREAM_AUDIT.md) 跳到固定源码。
5. 确认当前代码、工作区和最近变更再操作。下文实施顺序保留任务目标，不能据此把现有模块当作未实现，也不能把规划项报告为“测试已通过”。

需求冲突时优先最新用户明确决定，再更新 PRD/ADR；不能悄悄改变边界。可以自行解决包名、内部函数、文件划分等工程选择。没有证据的端点、路径和字段语义必须保留待核实状态。

## 2. 不要走偏的产品事实

账户管理不是 Agent 接入管理。用户可以只添加 DeepSeek 账户而不装任何 Agent；也可以只监测 Kimi 本地日志而不添加账户。

八个必需本地 Agent + Claude Desktop 条件网关是稳定版范围。中途可以交付内部 alpha，但不能把只有三种 Agent 的 alpha 写成需求已完成。账户接口按官方能力支持，实在没有依据可以不提供指标；这是已确认例外，不应用于缩减 Agent 清单。

正文可查看，默认不复制存储。网关可选且默认关闭。视觉基线是 cc-switch v3.20.4，功能基线是研究时较新的当前源码；这两个固定提交不要混为一谈。

## 3. 推荐实施顺序

### 第一步：P0 证据与最小探针

完成所有 Agent 的路径/版本/schema 清单。只写隔离测试探针，使用临时目录和合成日志，不扫描开发者完整主目录。对 Kimi 当前与旧 Python 协议分别确认 usage scope，对 Gemini/OpenCode/MCode 确认 reasoning/cache 包含关系。做一次 Claude Desktop 模型映射经过本机网关的可重复演示。

账户候选逐项审核 E1/E2/E3/E0。当前优先 DeepSeek、OpenRouter、Novita、官方 GLM 插件、Kimi Server、Codex App Server；其余有官方证据再接入。不能复用 cc-switch 仍保留的已停用 SiliconFlow 旧接口。

交付 `docs/research/compatibility-register.md`、各版本 fixture 说明和探针结果。无法实测的项写明，不编造成功。

### 第二步：搭建工程和数据核心

创建 Tauri/Rust/React/TypeScript 工程；保留本仓库文档，不从旧业务代码恢复。冻结 bundle identifier，设置系统数据路径、系统凭据库抽象和最小权限 IPC。

先实现 UsageEvent、Money、QuotaWindow、质量/完整性类型与序列化，再建数据库迁移、事务游标、去重身份和聚合。给两个 target 建 CI。此时前端可以采用合成数据展示框架，但必须标演示，不能混入用户数据库。

### 第三步：打通一条真实采集纵线

建议用 Kimi 新版或 Codex：纯解析 → 标准事件 → 身份去重 → 本机 DB → 聚合查询 → 用量页面。验证重扫、重启、尾行、复制/分叉和缓存公式后，再复用框架实现其他源。

不能只写“通用 JSON parser”覆盖所有产品。通用层负责基础读写与契约，产品版本层负责语义、身份与路径。一个格式变更不应迫使其他 Agent 停止工作。

### 第四步：完成全部采集与独立账户

按 A01–A09 任务补全本地覆盖及可选网关。账户模块从零账户也能使用，不要求选择 Agent。实现 vault → query service → snapshot → UI，错误/过期状态和权限不足与成功路径同等优先。

网关单独做协议/流式/取消测试，不为获取更多数据在后台自动接管。无法匹配日志身份时按独立域展示。

### 第五步：正文、导出和两平台发布

正文读取实现于统计之后，但仍是首版需求；用最小内容块类型支持各已支持会话格式，能力缺失要明确标注。完成导出、隐私开关、性能基准和安装包验收后才能发布稳定版。

## 4. Adapter 开发模板

每个 Agent adapter 必须在兼容登记中填写：

```text
adapter_id / product / product_versions / protocol_or_schema
os_and_path_rules / source_commit / source_urls / checked_at
counter_kind_and_scope / input_semantics / output_semantics
native_identity / fork_and_copy_rules / incremental_cursor
supported_fields / missing_fields / cost_semantics
fixture_ids / expected_totals / live_validation / known_gaps
compatibility_status / owner_or_task_id
```

账户 adapter 增加：credential_type、exact_hosts、method/path、auth_scheme、minimum_permission、scope、units、evidence_level、public_or_client_api、refresh_policy、error_mapping。用实际字段写映射表，不留下“按供应商处理”的空泛说明。

实现流程固定为：证据 → fixture → 纯 parser → 标准化 → 事务导入/快照 → UI → 两平台/兼容登记。解析器不允许访问网络，账户响应解析不允许取 vault，方便隔离验证。

## 5. Fixture 与断言

计划目录：`fixtures/agents/<adapter>/<schema>/`、`fixtures/accounts/<provider>/<schema>/`，每组包含 input、expected、README。expected 由人工按官方语义独立计算；不能运行当前 parser 生成预期后再测试它自己。

尽量合成 fixture；使用真实脱敏样本必须拥有授权，移除秘密、项目路径、用户名和正文。脱敏保留 Token、ID 关联、fork 区间与时序语义，README 说明改动。未知 Token 仍为 null，不能为了测试方便补 0。

必须覆盖：增量与全量一致；重复扫描；事务中断；尾行追加；文件截断/替换；相同数字独立请求；累计重复和重置；共享祖先；子 Agent；缓存子集；reasoning 子集/互斥；费用为 0；未知 schema；DB WAL/锁定；多币种；只给比例；凭据过期；HTTP200 业务错误；429；stale 快照。

对单位换算、字段包含关系和支持格式合成样本要求零算术误差。服务器已消费但日志从未落盘不是 parser 可修复的问题，应在覆盖说明披露。

## 6. 当前工程命令

以下命令已落地，与 README 和双平台 CI 对齐。前端浏览器模式没有原生 IPC；它可用于布局与模拟 IPC 检查，真实本机数据需要桌面应用。

```text
pnpm install --frozen-lockfile       # 使用已提交的锁文件
pnpm dev                           # 仅前端，演示数据需明确标记
pnpm tauri dev                     # 桌面联调
pnpm typecheck
pnpm lint
pnpm test
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
pnpm tauri build                   # 在目标 OS 构建
```

工具链已由 .node-version、rust-toolchain.toml、packageManager 和锁文件固定；升级时同步检查两个目标系统。不要在两 OS 采用不同语义的 formatter/linter。系统依赖安装按 Tauri 对应版本官方文档核实。

### 自动采集联调与独立核对

应用默认启动同步 Agent 日志，此后每轮结束等待 60 秒；账户按各自到期时间查询。用量页「数据采集与调度」和设置页可分别关闭两类自动任务，开关重启后保留。显示刷新只读取 Providock 数据库，不能用它触发文件扫描或账户网络请求。详细行为、回归与实机核对见 [本次验证记录](research/validation/2026-10-11-auto-monitoring.md)。

`python3 scripts/verify_local_usage.py` 只读取 Kimi/Codex 计量数据，临时提取无正文快照，用内存数据库运行正式扫描器并与独立计算比较。核对器的协议/格式边界见验证记录；不支持的输入必须停止，不能跳过后宣布通过。该命令不查询订阅额度、不访问凭据库、不启动推理。

## 7. 数据迁移与解析器升级

自有数据库 schema_version 与 adapter/parser_version 分开。DB migration 先备份、自检、事务提交；源数据从不迁移。目标是通过 parse_run 重建与原子切换活跃版本，不能同时累加旧结果；当前通用的整源切换仍未完成，不能把逐批事务当作整源回滚。Codex v2 本次采用精确身份退休与游标同事务提交，范围见验证记录。

删除某源统计与重建的操作必须同时维护 event identity、游标和聚合缓存。保留统计历史但源丢失时继续可读；不能因找不到原文件就删历史。统计清理和账户清理不互相影响。

聚合以账本版本作缓存键；时区、筛选、费用版本变化必须失效缓存。账户快照用 provider_as_of 和 queried_at 区分平台时间与请求时间，不能用本地刷新时间伪装平台刚结算。

## 8. UI 联调

按 [UI 规范](specs/UI_UX.md) 实现，不从旧仓库功能反向扩展。先把空/未知/过期/无权限/不兼容状态做好，再做图表细节。可参考 [离线原型](prototypes/dashboard.html) 的信息层次；原型不是生产组件，也没有真实数据。

共享 DTO 由 Rust 类型生成或以 schema 校验，前端不自行重新计算 Token 总量和 Money。表格分页与图表查询一致，tooltip 必须解释缓存分母和覆盖。长别名、多个额度窗口、中文布局和 Windows 缩放纳入截图验收。

## 9. 变更与评审

一个改动围绕一项语义或任务，提交说明写问题、结果和验证。适配器变更附 source commit 与 fixture diff；数字变化附前后口径。数据契约改变先更新版本和迁移，再更新消费者。不要把“消除错误提示”当成修复接口不兼容。

代码评审重点：有没有重复计数、未知补零、scope 混淆、未经验证的余额端点、明文凭据、源写入、UI 主线程大扫描或仅单平台验证。无关 UI 文案改动无需新增镜像实现的测试，关键采集逻辑必须有回归。

## 10. 后续 Agent 交接格式

```text
任务：Txx / Axx / Bxx
当前状态：未开始 / 进行中 / 阻塞 / 已验证
实现文件：...
契约/证据版本：...
验证命令与结果：...
兼容范围：OS、Agent/协议版本、账户权限
已知缺口：...
下一步：具体可执行的一项
```

当前交接：核心采集/聚合、独立账户服务、主要页面和自动调度已实施并有回归；Kimi/Codex 另有 macOS 只读计量快照核对。2026-10-12 的未发布源码另含账户 IPC 白屏修复、用户明确选择的本地明文 Key 保存、模型统计与手动定价（读取当前价比较，不改写历史估值）。真实账户联网、完整原生 GUI、Windows 实机、通用数据升级/保留、网关转发和性能发布门槛仍需完成。具体后续任务以 [CURRENT_STATUS.md](CURRENT_STATUS.md) 为准，不重复搭建已有模块或重新讨论已确认方向。

## v2.0.2 界面与原生行为

当前统一纯黑主题，不再提供浅深切换。图标源文件 assets/app-icon.svg / tray-icon.svg；桌面图标通过 Tauri icon 命令生成，菜单模板的 36×36 RGBA 与 PNG 一起提交。macOS 窗口关闭只隐藏，退出处理集中在 RunEvent::ExitRequested；原生菜单任务独立读取本地统计库，不依赖 WebView。进行 UI 验证时使用独立开发标识及合成数据，不能把浏览器 mock 当作原生关窗/退出证明。

## v2.0.3 交接补记

当前入口和未完成项以 CURRENT_STATUS 为准，早期「未发布」说明仅指当时状态。v2.0.2 已发布账户 IPC/明文凭据/模型统计。v2.0.3 包含 Kimi 直连、GLM 错误状态、定价策略、Agent 可见性与趋势改进；新增 GitHub 更新流程见 specs/APP_UPDATES.md。局部账户已有 macOS 真实验证，不外推到其他平台。后续 Release 必须生成独立 v2 清单，并同时核对其平台 URL/签名及全部公开资产。

## v2.0.4 接手注意

当前费用仅模型价格估算，源自报费用业务已移除；旧 DB 三列不删。精确金额展示两位小数，费率/底层计算不截断。新导出契约为 statistics.v2。AIHOT 导入的证据、合成夹具和测试见 price_import 服务；只在隔离库运行真实价格核验例子，不用用户数据做 UI 夹具。统计卡默认当天口径，沿用筛选并标实际起止；共享 Select 需保持键盘、焦点及 dialog 层级。发布事实见本版核验记录，不将 CI 视为 Windows 实机。

## 2026-10-13 本地开发补充

本轮改进未发布，正式下载仍 v2.0.4。接手先读 CURRENT_STATUS 顶部本地改进和对应验证记录，不能将历史批量 AIHOT 导入或 B07 占位作为当前事实。新增九源 DSH、单次价格辅助约束见 AGENTS/数据/隐私规范。

`cargo run --manifest-path src-tauri/Cargo.toml --example verify_codex_limits` 只通过官方 App Server 查询，不写 Providock 库、不输出身份。`prepare_local_review` 要求显式全新目录，生成四条合成 DSH 记录且关闭自动采集/账户刷新；原生测试必须另用标识/数据目录，不能指向正式账本。

### v2.1.0 发行后接手

用户已在本地验收后授权发布 v2.1.0；此前“未发布/不推送”为阶段记录，不再是当前限制。发行事实与清理结果以 2026-10-13-release-v2.1.0.md 为准。
