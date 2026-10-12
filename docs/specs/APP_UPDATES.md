# 应用内更新（v2.0.3）

采用官方 [Tauri updater](https://v2.tauri.app/plugin/updater/) 与 [process](https://v2.tauri.app/plugin/process/) 插件，JS/Rust 均在锁文件固定。现有 minisign 公钥验证下载，私钥仅由 GitHub CI Secrets 提供。ad-hoc 应用完整性签名与更新包签名分别核验，不等于 Apple 公证或 Authenticode。

- 固定 HTTPS 端点：`https://github.com/Abyssaler/Providock/releases/latest/download/providock-v2-latest.json`。
- 正式版清单仅含 `darwin-aarch64` 的 `Providock.app.tar.gz` 和 `windows-x86_64` 的当前版本 NSIS EXE；不发布旧 v1 的 `latest.json`。版本、UTC 发布时间、说明、下载 URL 与完整 `.sig` 内容通过生成器写入；签名不是 `.sig` URL。
- 默认启动检查，之后每 6 小时；WebView 恢复可见时检查是否到期。macOS 隐藏窗口时计时器可能被系统节流，不能承诺后台精确每 6 小时；退出应用不检查。
- 自动检查开关保存本机 WebView localStorage（`providock.updates.autoCheck`），关闭不影响手动检查。检查超时 15 秒，失败可重试；没有新版显示当前已是最新。
- 新版横幅仅应用内通知，无 OS 通知/邮件订阅。可稍后处理；设置页始终提供入口。发布说明当纯文本显示。
- 检查与下载分离，用户点击才 `downloadAndInstall`。下载进度后由原生插件校验签名再安装；失败不请求重启，可重试。macOS 完成后调用 relaunch，Windows 由 NSIS 更新流程退出并重启，采用 passive 安装模式。没有任意 shell 执行权限。
- 同标识 v2 更新保留统计/账户/价格/凭据和设置。本版无 schema 变更；不保证跨未来 schema 回滚，未来数据库升级须额外落实备份与兼容方案。
- v2.0.3 是启用首版。v2.0.2 及更早没有此入口，需要手动替换一次；旧 v1 标识/数据目录独立，仍无自动迁移。

发布前必须同 SHA 双平台 CI 成功并独立验签两平台二进制，再生成清单，检查 URL 对应本版真实资产并将清单纳入 SHA256SUMS。草稿核对后发布 Latest；公开回下载全部资产，测试固定检查端点返回匹配清单。

回归覆盖仅通知、显式安装、签名/网络错误不重启、并发检查、资源释放、重启失败重试与清单结构。实际签名检验与原生验证另见本版记录；mock 控制器通过不代表已经在 Windows 实机完成更新。
