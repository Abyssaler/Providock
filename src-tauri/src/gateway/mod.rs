//! 可选采集网关的 wire 协议 usage 纯解析（架构第 4、7 节）。
//!
//! 本模块只做解析：把上游非流式响应体 / SSE 事件序列转成 [`UsageSet`]，
//! 再经 [`UsageSet::to_usage_event`] 变成标准 [`crate::domain::UsageEvent`]
//! （`stream_domain = gateway`，`granularity = request`，`request_count = 1`）。
//!
//! 不转发、不监听、不注入凭据、不写源文件——网关本体不在本任务范围。
//! 覆盖四种 wire 协议：Anthropic Messages、OpenAI Chat Completions、
//! OpenAI Responses、Gemini Native。语义与差异见 [`usage`] 模块文档。

pub mod usage;

pub use usage::*;
