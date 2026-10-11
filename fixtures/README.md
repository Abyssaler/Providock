# Fixtures 目录约定

状态：T00 进行中，目录结构先行建立；各适配器样本随证据核实逐步补齐。

布局（docs/DEVELOPMENT_HANDBOOK.md 第 5 节）：

```text
fixtures/agents/<adapter>/<schema>/    # input、expected、README
fixtures/accounts/<provider>/<schema>/ # input、expected、README
```

规则：

- expected 由人工按官方语义独立计算，不能用当前 parser 的输出反推。
- 优先合成样本；真实样本必须经授权并脱敏（秘密、项目路径、用户名、正文），README 说明改动。
- 未知 Token 保持 null，不为测试方便补 0。
- 禁止提交开发者本机真实日志、密钥、Cookie、完整项目路径或私密对话。
