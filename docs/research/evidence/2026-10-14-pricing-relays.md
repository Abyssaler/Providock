# 官网定价与中转站协议审计（2026-10-14）

## 官网定价 v2

来源为用户指定的 [MiMo](https://mimo.mi.com/docs/zh-CN/price/pay-as-you-go)、[DeepSeek](https://api-docs.deepseek.com/zh-cn/quick_start/pricing/)、[智谱](https://docs.bigmodel.cn/cn/guide/start/pricing)。下载公开 HTML，使用 HTML5 DOM，不运行脚本、不读取 Cookie、不发送凭据。fixtures/pricing-pages 保存仅价格表的结构片段，保留 rowspan/colspan，删除页面属性/脚本/导航；预期值在测试中独立填写。

原问题：自制去标签器将单元格内部 div 换成行边界、丢弃跨行/跨列；只接受模型第一列的竖表；全页混币种直接拒绝；中文“未命中”会误映射缓存命中；“输入模态”也可能覆盖输入价格；辅助提示强制分档返回 null；候选 DTO/表单没有策略传输字段。

新解析支持币种与实时/Batch 显式选择、DeepSeek 横表、MiMo 合并单元格/同价别名、智谱免费/上下文输入与输出组合分档。百万 Token 的 K 阈值按 1000，组合范围下限包含、上限不含。按小时缓存存储不映射为 cacheWrite。未证明支持的 Batch 折扣、音频/图片/搜索费用不推导。单行条件价也保留条件；重叠分档拒绝，缺少条件用量或超范围保持未知。

DeepSeek 时区/高峰时间以页面原文校验；排除节假日的 2026 日历依据 [国务院办公厅 2026 年安排](https://www.gov.cn/zhengce/zhengceku/202511/content_7047091.htm)。周末仍为空闲时段（含调休上班周末，因为定价明确周一至周五）；未覆盖年份的工作日不猜节假日。旧规则默认不启用新日历，保留旧含义。

模型辅助仍只由主动选择账户/模型后一次请求触发；包含币种、Batch、全体策略字段，原文数值证据校验、独立解析复核；不一致/复杂条件无法独立核实则拒绝，不静默按最低档。原文证据支持十进制等值（3 与 3.00），不会让 2 匹配 0.2。未做真实付费调用，不将模拟响应称为实测。

## B08 relay-v1

研究的是协议家族，不把任意名称的中转站都视为同一实现：

- New API 官方固定提交 `7de10f2c602b50122017556e8a09cb7c488c273c`：[路由](https://github.com/QuantumNous/new-api/blob/7de10f2c602b50122017556e8a09cb7c488c273c/router/api-router.go)、[用户](https://github.com/QuantumNous/new-api/blob/7de10f2c602b50122017556e8a09cb7c488c273c/controller/user.go)、[Token](https://github.com/QuantumNous/new-api/blob/7de10f2c602b50122017556e8a09cb7c488c273c/controller/token.go#L231-L298)、[旧 billing](https://github.com/QuantumNous/new-api/blob/7de10f2c602b50122017556e8a09cb7c488c273c/controller/billing.go)。[Token usage 官方文档](https://doc.newapi.pro/en/api/token-usage/)、[认证](https://docs.newapi.pro/en/llms.mdx/api/management/auth)。
- One API 官方固定提交 `8df4a2670b98266bd287c698243fff327d9748cf`：[用户](https://github.com/songquanpeng/one-api/blob/8df4a2670b98266bd287c698243fff327d9748cf/controller/user.go)、[旧 billing](https://github.com/songquanpeng/one-api/blob/8df4a2670b98266bd287c698243fff327d9748cf/controller/billing.go)。
- OpenRouter 已有独立官方适配，继续使用其 Key / Credits 能力。
- Veloera、Neko API 和站点自有接口不能只按品牌推断；未验证的分叉不标兼容，不提供盲探测或跨站自动尝试。

| 连接 | 凭据 / 请求 | 语义 |
| --- | --- | --- |
| New API Key | Bearer 推理 Key；GET /api/usage/token | data.object=token_usage；total_available/total_used 为 Key 额度；unlimited_quota 不代表账户无限余额 |
| New API 账户 | Bearer 个人访问令牌；New-Api-User 用户 ID；GET /api/user/self | success=true，data.id 与配置一致；quota=剩余，used_quota=已用 |
| One API 账户 | Bearer 个人访问令牌；GET /api/user/self | 同账户额度字段；不使用 New-Api-User；不伪造普通 Key 的账户权限 |

旧 dashboard/billing/subscription + usage 的 *_USD 命名不能证明美元：上游可按 CNY / TOKENS / 原始额度展示，usage 又乘 100；无限额还用大数哨兵。因此本版采用上述明确接口，不默认相减当作现金。quota_per_unit 可自定义，不把常见 500000 假定为任意站点标准。

默认展示 vendor_unit。用户明确填写 CNY/USD 与每货币单位整数额度后，以 i128 算术换算（最多 9 位小数截断、derived）；仅账户通道生成 Available 余额，Key 只展示额度窗口；仅上游明确授权并返回 data.user_quota 时额外创建账户范围（可查询余额），无返回则保持未知。小到会被截断为零的正数拒绝。0、负余额、未知、无限额保留各自语义。账户 scope 含站点/用户，钱包不跨账户求和。

配置在 credential_refs.capabilities.relay 的版本化形状中持久化，不变更数据库 schema；Key 留在独立 vault。轮换凭据继承非秘密配置。请求只到明确输入的 HTTPS 根地址：443、无 userinfo/参数/片段或 /v1；DNS 全部为公网并固定解析结果，不使用代理、不跟随重定向；超时/响应上限沿用账户服务。401/403/429/schema/HTTP200 业务失败保留旧成功快照并标 stale。生产不发推理、不注册或登录站点、不读取浏览器 Cookie。

状态：协议/合成样本验证，三份官方价格页另有 macOS 联网验证；没有真实中转站账号或 Windows 原生联网实测。主版本/域名不会证明协议兼容；必需字段变化时 incompatible，不补零。
