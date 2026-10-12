# AIHOT 价格导入证据（2026-10-12）

用户指定来源：https://aihot.news/leaderboard 。详情示例：https://aihot.news/leaderboard/gpt-6-1-sol 。

本轮直接读取公开 HTTPS 页面并核对详情。页面说明价格以人民币/百万 Token 展示，美元按页面记录日期的汇率折算；缓存价格仅指命中输入，不含写入、存储或订阅。2026-10-12 页面数据的 run.generatedAt=2026-10-12T06:06:34.739Z，fx.rate=6.6921、asOf=2026-10-09、sourceName=欧洲央行。价格不是 Providock 独立核实的厂商报价，不替代平台账单。

页面使用 React Router SSR：streamController.enqueue 的 JSON 字符串内是引用表，loaderData → leaderboard-boards → models 是完整模型目录；run 提供来源更新时间和 fx。每项 model 提供 slug/name，price 提供 inputCny/outputCny/cachedCny/officialUrl；未定价为 null。直接解码 JSON 与引用表，不执行网页脚本，也不依赖 HTML 样式或排行榜名次。读取 inputCny 等人民币字段，规整到至多九位小数以消除 JS 浮点尾数；不二次计算汇率。

合成形状样本：fixtures/pricing/aihot/catalogue.html。不是线上价格快照；测试预期独立为 input=2.125/output=12.5/cacheRead=未知/cacheWrite=未知，显式免费保持零。

模型名仅做大小写/分隔符规范化和明确的提供商前缀处理；Kimi Code k3 可对应 Kimi K3，带 256k、日期、preview 等后缀不会模糊合并。多个目录项同时匹配时跳过。手动、停用、来源专属与分档规则优先。缓存创建、分时段和长上下文数据未提供，不自动猜测。

网络仅在用户点击时请求固定来源，不发送 Agent 日志、模型名、账户或凭据。格式变化、网络失败、体积超限都保留现有规则。导入批次事务回滚；来源、抓取时间、汇率日期随价格版本保存。
