# Relay v1 synthetic response

Official New API token usage and New API / One API user/self response shapes. No live keys or account data. At explicitly configured 500000 quota / USD, available=1.97531 USD, used=0.02469 USD; no conversion configured means vendor quota, never cash. Unlimited token has used=12345 and unknown remaining/limit. `quota` is remaining, not total; scopes account vs key remain distinct.

Evidence: [token usage](https://doc.newapi.pro/en/api/token-usage/), [authentication](https://docs.newapi.pro/en/llms.mdx/api/management/auth), [New API user](https://github.com/QuantumNous/new-api/blob/main/controller/user.go), [One API user](https://github.com/songquanpeng/one-api/blob/main/controller/user.go). Fixed revisions and compatibility limits in docs/research/evidence/2026-10-14-pricing-relays.md.
