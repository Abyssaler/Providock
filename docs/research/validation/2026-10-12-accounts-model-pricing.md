# 账户与模型定价验证（2026-10-12）

## 范围与结论

针对用户报告的账户白屏、macOS Keychain 密码弹窗和用量界面不足，修复账户 IPC 字段，按用户明确选择采用本地明文 Key 持久化，并增加模型统计、定价、成本趋势和分页明细。本轮是 v2.0.0 之后的未发布源码改进，未替换已安装正式应用，未修改正式发布标签或资产。

### 账户与凭据

- 白屏根因：账户外层 DTO 使用 camelCase，嵌套 Snapshot 仍为领域 snake_case，前端访问 quotaWindows 等字段失败。新增显式嵌套 DTO，列表与刷新统一转换，不改变适配器样本格式。
- Rust 从真实内存数据库构建账户及成功快照，序列化结果与 fixtures/interop/account-summary.json 逐字段一致；前端渲染同一夹具，验证余额 18 CNY、5 小时剩余 42% 和指标 123。页面错误边界保留导航与重试。
- 用户在比较方案后选择 A：本地明文保存。生产装配 LocalPlaintextVault（应用数据目录 credentials/keys.sqlite3），不构造 KeyringVault，不读取、迁移或删除旧 Keychain。已有账户的旧 Key 需要重新输入一次。
- 合成凭据测试覆盖独立文件的保存、重启、覆盖、删除、其他凭据隔离、不存在时不创建、无效引用、损坏文件不覆盖、错误不回显内容/路径，以及 Unix 目录 0700/文件 0600 和符号链接拒绝。查询响应只返回掩码/指纹/credentialReady。
- 凭据文件独立于普通统计数据库。保存为明文是明确产品选择，不是加密，完整复制应用数据目录会带出 Key。Windows 继承用户目录 ACL，尚未实机验证。

### 用量与定价

- 按 Agent + 精确模型 + 可选来源追加价格版本；来源必须属于 Agent，具体来源优先。输入/输出/缓存读取/缓存创建均按每百万 Token 定价，USD/CNY 分开，空白表示未知，0 表示明确免费。
- 独立预期值：fresh 1000 × 3/1M + read 500 × 0.3/1M + output 200 × 12/1M = 0.00555 USD；输入总量不再重复计价。
- 回归覆盖未知数量、缺失费率、零价、非法价格、完整/部分估算、版本追加/停用、来源优先级、多币种、模型/Agent 汇总、筛选、分页和带时区时间归桶。源 reported 0 保留，既有历史估值不改写。
- 计量明细每页 20 条，与汇总/趋势使用同一成功查询范围；显示刷新保留当前页，用户改变筛选时回到第一页。数据按当前价格比较，明确标注估算及覆盖率，不当作平台账单。

## 自动检查

在隔离的开发副本、缓存依赖、macOS ARM64 环境执行：

```sh
node node_modules/typescript/bin/tsc --noEmit
node node_modules/vitest/vitest.mjs run
node node_modules/prettier/bin/prettier.cjs --check .
node node_modules/vite/bin/vite.js build
cargo test --offline --manifest-path src-tauri/Cargo.toml
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --offline --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
git diff --check
```

前端 50 项 / 8 文件通过；Rust 321 项通过、2 项 ignored（主动实测项未启用），22 个测试套件无失败。完整 Rust 回归中的 HTTP 测试仅访问合成的本机回环服务；没有读取真实 API Key 或系统钥匙串。格式、类型、Clippy 与生产构建通过。

## 界面与构建边界

使用临时本地预览页注入合成 IPC 数据，浏览器验证浅深色、960×800 最小桌面窗口、模型筛选、费用/Token 趋势切换、模型行定价预填/保存、Agent 统计、20 条明细与第二页、账户快照和明文保存说明。960px 时页面 scrollWidth 与可视宽度均为 945px，表格单独横向滚动。该预览页和 mock 不进入生产源码或安装包。截图含“合成数据预览”标识。

浏览器 mock 的成本数据为固定合成值，价格保存交互不作为重新计算正确性的证据；精确计算由上述独立预期值和数据库回归证明。浏览器验证不等于原生 Tauri 全链路实机验收。

本机另构建 Providock Dev.app（io.providock.dev，独立数据目录），包含真实 Rust 后端与生产前端，供本地检查；采用完整 ad-hoc 签名并严格校验，不是 Developer ID 公证或正式发布。包内只含执行文件、图标、Info.plist 与签名资源，不含本机数据库、日志或 Key。未启动此测试应用，未自动迁移正式数据。

尚未验证：Windows 实机、真实平台账户联网、旧 Keychain 自动迁移（有意不实现）、百万事件性能、网关转发、统计导出/清理 UI。八源解析器与账户端点范围保持原状。
