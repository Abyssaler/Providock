# 上游参考与第三方许可

本次重建交付包括原创规划文档、桌面实现和演示原型，未将 cc-switch、Kimi Code 等上游完整实现、品牌图标或截图打包为项目资产。接口/字段名称用于说明互操作格式。

- [cc-switch](https://github.com/farion1231/cc-switch)：UI v3.20.4 和当前用量/账户实现，固定提交见研究记录；原仓库 MIT 归属保留。
- [Kimi Code](https://github.com/MoonshotAI/kimi-code) 与 [旧 Kimi CLI](https://github.com/MoonshotAI/kimi-cli)：官方日志/协议来源。
- [Z.AI Coding Plugins](https://github.com/zai-org/zai-coding-plugins)：官方 GLM 计划查询依据。
- [OpenAI Codex](https://github.com/openai/codex)：官方日志与 App Server 互操作来源。

若后续复制或修改上游代码，必须核实固定版本许可证，在文件/发行包保留所需完整版权与许可文本，并记录源路径、提交和改动；不能因“开源”而删掉归属。新增 npm/Rust 依赖后生成实际依赖许可清单；实际依赖版本以 package.json/pnpm-lock.yaml 和 Cargo.toml/Cargo.lock 为准；完整逐依赖许可清单仍待补齐，不把上游研究清单视为已完成的依赖许可审计。

原仓库 LICENSE 的 `Copyright (c) 2025 Jason Young` 沿用保留；新文档和原型补充 Providock contributors 归属。厂商品牌只用于识别兼容对象，不表示赞助或官方合作。

## v2.0.3 更新插件

新增官方 Tauri updater 2.13.3 与 process 2.4.0（JS/Rust），来源 [tauri-apps/plugins-workspace](https://github.com/tauri-apps/plugins-workspace)，双许可 MIT OR Apache-2.0；本发行选择 MIT 并保留以下许可文本。实际依赖许可元数据清单见 [记录](docs/research/dependency-licenses-v2.0.3.md)，其范围和未获取目标包在记录中说明，完整依赖义务审计仍待补齐。

```text
MIT License

Copyright (c) 2017 - Present Tauri Apps Contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
