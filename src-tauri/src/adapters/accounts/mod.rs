//! 账户查询适配器：请求构造 + 纯响应解析（不触网、不碰凭据库）。
//! HTTP 与凭据注入在服务层（ARCHITECTURE 第 4 节）。

pub mod codex_appserver;
pub mod deepseek;
pub mod glm_plan;
pub mod kimi_server;
pub mod novita;
pub mod openrouter;
