# 官网价格表结构夹具（2026-10-14）

由三份用户指定官网公开 HTML 裁剪价格表而成；只保留结构和事实费率，去掉脚本、属性和站点导航。不是真实账户数据。保留 div、code、rowspan、colspan、免费与分档条件以复现旧解析器失败；独立预期在 src-tauri/tests/official_price_pages.rs。

- MiMo：CNY 实时 mimo-v2.6-pro 输入 3、输出 6、缓存命中 0.025；USD Batch 输入 0.2175、输出 0.435、缓存 0.0018。同价旧别名取同一行。
- DeepSeek：deepseek-flash 空闲 1/4/0.02，高峰 2/8/0.04；北京时间周一至周五 09:00–12:00 / 14:00–18:00，排除中国节假日。
- 智谱：GLM-5.3 输入 8、输出 28；GLM-5.1 32K 输入边界；GLM-4.7 包含 32K 输入 / 200 输出的组合档位；GLM-4.7-Flash 免费。按小时缓存存储不当作 Token 写入费率。

来源和协议边界见 [证据](../../docs/research/evidence/2026-10-14-pricing-relays.md)。
