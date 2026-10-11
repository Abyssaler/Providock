# 兼容登记：实施前初始状态

日期 2026-10-11。这里的 researched 只表示阅读了代码/官方说明，不表示应用已实现，也不表示通过实机测试。

| ID | 当前证据 | 协议/版本待办 | 状态 |
| --- | --- | --- | --- |
| A01 Claude Code | cc-switch 固定导入器 | 固定官方版本，主子/分叉夹具、两平台路径 | researched |
| A02 Codex | cc-switch 固定导入器 + 官方协议文档 | 累计语义、压缩格式、fork 范围 | researched |
| A03 Gemini CLI | cc-switch 固定导入器 | 官方输入缓存/thoughts包含关系 | researched |
| A04 Grok Build | cc-switch 固定导入器 | turn_completed 身份和模型调用字段 | researched |
| A05 OpenCode | cc-switch 固定导入器 | V1/V2更新规则、reasoning包含关系 | researched |
| A06 Pi | cc-switch 固定导入器与路径配置 | 官方路径、摘要计量与跨文件身份 | researched |
| A07 MCode | cc-switch 固定导入器与路径配置 | 官方投影表schema/更新规则 | researched |
| A08 Kimi | 固定官方 TS 源码、官方目录/Server文档 | 新wire版本夹具、旧Python独立格式、fork/迁移 | researched |
| A09 Desktop | cc-switch 条件路由范围 | 一次可重复的手动模型映射演示 | researched |
| B01 DeepSeek | E1 官方文档 | 响应夹具、权限、实测时间 | researched |
| B02/B03 OpenRouter | E1 两个不同官方接口 | 普通/管理Key分开、预算scope | researched |
| B04 Novita | E1 当前官方余额文档 | 新路径、1/10000USD换算 | researched |
| B05 GLM | E2 固定官方插件源码 | 区域、窗口单位、个人/团队区别 | researched |
| B06 Kimi | E1 官方实验本机API | 版本协商、成功/错误双层envelope | researched |
| B07 Codex | E1 官方App Server文档 | 安装版本能力、只读方法范围 | researched |

其他账户候选的初始状态见 [账户规范](../specs/ACCOUNT_ADAPTERS.md)，逐项审核后增补，不默认为支持。

每项后续增加：product_versions、protocol_or_schema、OS、source_commit、checked_at、fixture_ids、expected_totals、字段覆盖、缺口、live_validation、发布状态。fixture_verified / live_verified 分开；没有真实权限就不能伪造 live_verified。正式支持范围必须能回溯到对应版本和验收记录。
