# 凭据保存方案比较与决定

用户要求：避免 macOS Keychain 授权弹窗；曾选择本地加密、Providock 专用密码解锁，随后要求先核查 CC Switch 并比较方案。随后用户明确选择 A：参考 CC Switch 明文保存。临时 SessionVault 已替换为 LocalPlaintextVault；正式安装包与真实账户数据尚未修改。

## CC Switch 源码核查

核查日期：2026-10-12。仓库 farion1231/cc-switch，main 提交 `28dc5198115575ced2e3502f52181e7d211859c9`。通过 GitHub 连接器读取该提交源码，未访问用户本机的 CC Switch 配置或凭据。

- [providers.rs](https://github.com/farion1231/cc-switch/blob/28dc5198115575ced2e3502f52181e7d211859c9/src-tauri/src/database/dao/providers.rs#L269)：save_provider 直接将 settings_config 序列化为 JSON 写入 providers.settings_config；读取直接解析 JSON。
- [database/mod.rs](https://github.com/farion1231/cc-switch/blob/28dc5198115575ced2e3502f52181e7d211859c9/src-tauri/src/database/mod.rs#L108)：普通 SQLite Connection::open；[Cargo.toml](https://github.com/farion1231/cc-switch/blob/28dc5198115575ced2e3502f52181e7d211859c9/src-tauri/Cargo.toml#L76) 使用 bundled SQLite，未在此路径启用 SQLCipher。
- [官方配置说明](https://github.com/farion1231/cc-switch/blob/28dc5198115575ced2e3502f52181e7d211859c9/docs/user-manual/zh/5-faq/5.1-config-files.md)：默认数据目录 ~/.cc-switch/，数据库 cc-switch.db；直连模式的密钥还写到对应工具配置，数据库备份含供应商配置。

结论限定为普通供应商 API Key 的保存路径：明文 JSON 字段位于普通 SQLite 数据库中，不是应用主密码加密或系统 Keychain 保存。不能由这一结论推断所有 OAuth 子系统的所有保存实现。

## 提供的选择

A. CC Switch 风格本地明文保存：启动直接使用；限制本机文件权限，但拥有读取权限的进程仍可读取密钥，备份同样敏感。

B. 本地密码加密（建议）：用户设置 Providock 专用密码，启动后解锁一次；密码不落盘，不依赖 Keychain，备份保留密文。忘记密码须重置密钥库并重新输入厂商 Key。未解锁时仍显示账户元数据与统计，暂停需要凭据的网络查询。

C. 仅本次运行内存：不保存密钥文件，每次重启重新输入厂商 Key。

所有选项均不自动读取、迁移或删除旧 Keychain 条目。加密只保护 Providock 自身保存的副本，不会自动保护其他客户端既有的明文配置。最终采用 A；实现采用独立 credentials/keys.sqlite3，限制 Unix 目录/文件权限，避免统计数据库包含秘密。已同步安全规范与验证记录。
