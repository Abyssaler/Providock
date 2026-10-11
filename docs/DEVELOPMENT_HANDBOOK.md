# Providock 开发手册

版本 0.1 · 2026-10-11 · 状态：设计已建立，应用尚未搭建。

本手册用于人类开发者、Codex、Claude Code、Kimi Code 等后续开发 Agent 接手。先明确统计语义和适配证据，再实现 UI；任何“已支持”结论必须有可重放验证。

## 1. 五分钟接手

1. 读 [产品需求](PRODUCT_REQUIREMENTS.md) 和 [AGENTS.md](../AGENTS.md)，确认本任务没有改变用户已确认选择。
2. 读 [数据契约](specs/DATA_CONTRACTS.md)，特别是互斥输入、reasoning 包含关系、累计/增量与未知值。
3. 在 [路线图](ROADMAP_AND_ACCEPTANCE.md) 选一个任务编号，查其依赖和完成门槛。
4. 查相关 adapter 的 [采集规范](specs/AGENT_ADAPTERS.md) 或 [账户规范](specs/ACCOUNT_ADAPTERS.md)，从 [研究证据](research/UPSTREAM_AUDIT.md) 跳到固定源码。
5. 确认当前代码状态再操作。本文规划的目录/命令尚不存在时先搭建，不能报告“测试已通过”。

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

## 6. 工程搭建后应提供的命令

以下是**拟定接口**，尚未实现；T01 必须把脚本和 CI 一并落地，之后 README 再给用户可运行命令。

```text
pnpm install --frozen-lockfile       # 已提交 lockfile 后可用
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

搭建时选定可用版本的 Node 活跃 LTS、Rust toolchain、pnpm 和依赖并提交锁文件。不要在两 OS 采用不同语义的 formatter/linter。系统依赖安装按 Tauri 对应版本官方文档核实。

## 7. 数据迁移与解析器升级

自有数据库 schema_version 与 adapter/parser_version 分开。DB migration 先备份、自检、事务提交；源数据从不迁移。解析规则修正通过 parse_run 重建与切换活跃版本，不能同时累加旧结果。

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

本次设计交付的现状：应用代码、运行测试、真实账户查询、安装包、签名与跨平台实机测试均尚未完成。下一步从 T00 兼容证据登记和 T01 工程搭建开始，不需要再次讨论已经确认的产品方向。

（2026-10-11 更新：T00–T10 主线已实施，逐任务交接见 [CURRENT_STATUS.md](CURRENT_STATUS.md)。）
