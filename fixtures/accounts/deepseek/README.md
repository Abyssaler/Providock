# DeepSeek 账户夹具（B01）

接口：`GET https://api.deepseek.com/user/balance`，`Authorization: Bearer <API_KEY>`。
来源 URL：https://api-docs.deepseek.com/zh-cn/api/get-user-balance/ （2026-10-11 快照）。
依据登记：docs/research/evidence/accounts-b01-b07-candidates.md「B01 DeepSeek API 余额」（E1）。

全部为**合成**响应，按官方文档字段手工构造，不含任何真实账户数据。

字段语义（官方原文）：`total_balance` = 总的可用余额，**包括赠金和充值余额**；
`granted_balance` = 未过期的赠金余额；`topped_up_balance` = 充值余额。

## 用例

- `single-currency/`：单币种 CNY。
  手工换算：`total_balance=110.00 = granted 10.00 + topped_up 100.00`（总额已含组件）。
- `multi-currency/`：CNY 与 USD 两组（CNY 110.00；USD 25.50），按币种分开，绝不跨币种相加。
- `business-error-200/`：HTTP 200 但返回业务错误体 `{"error":{...}}` → 失败，`business_code` 取 `code`。
- `schema-change/`：HTTP 200 但缺少 `balance_infos` → `incompatible`，不补零。

映射规则（见 src-tauri/src/adapters/accounts/deepseek.rs）：每个币种产出一组
`available(total_balance)` + `grant(total_balance→granted_balance)` + `paid(topped_up_balance)`；
`available` 行 formula 记 `total = granted + topped_up`，UI 端不得重复相加。
