# Novita 账户夹具（B04）

接口：`GET https://api.novita.ai/openapi/v1/billing/balance/detail`，
`Authorization: Bearer <API_KEY>`，请求头 `Content-Type: application/json`（文档标 required）。
来源 URL：https://docs.novita.ai/api-reference/basic-get-user-balance （2026-10-11 快照，
页面 Last modified: May 25, 2026）。
依据登记：docs/research/evidence/accounts-b01-b07-candidates.md「B04 Novita 余额」（E1）。

全部为**合成**响应，按官方文档字段手工构造，不含任何真实账户数据。

字段与单位（官方原文）：`availableBalance` 用户 credit 余额；`cashBalance` 充值剩余余额；
`creditLimit` 授信上限（最多可欠）；`pendingCharges` 待结算（默认 0）；`outstandingInvoices`
当前欠款。**全部单位 1/10000 USD**（`10000 = $1.00`），换算为**整数除以 10000**（十进制字符串移位，
不使用二进制浮点）。

## 用例（手工换算）

- `balance-detail/`：
  - availableBalance 1000000 → `1000000 / 10000 = 100` USD
  - cashBalance 800000 → `80`
  - creditLimit 200000 → `20`
  - pendingCharges 0 → `0`；outstandingInvoices 0 → `0`
- `negative-available/`：availableBalance `-5000` → `-5000 / 10000 = -0.5`（负值保留，不放零）；
  outstandingInvoices 250000 → `25`。
- `unit-precision/`：availableBalance 12345 → `1.2345`；cashBalance 1 → `0.0001`；
  pendingCharges 9999 → `0.9999`。
- `schema-change/`：缺 `creditLimit` 等字段 → `incompatible`，不补零。

映射规则（见 src-tauri/src/adapters/accounts/novita.rs）：`availableBalance`→`available`、
`cashBalance`→`paid`；`creditLimit` / `pendingCharges` / `outstandingInvoices` 归入 `credit`
并在 formula 标注“非现金”语义（`BalanceKind` 无“负债”种类），UI 不得当作可用现金。
