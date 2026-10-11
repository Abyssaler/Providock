/**
 * 会话 IPC 契约（与 src-tauri 的 serde DTO 对应，rename_all = camelCase）。
 *
 * 统计数字一律为十进制字符串或 null；前端不做汇总计算（ARCHITECTURE 第 6 节）。
 * `ContentErrorCode` 的取值与后端 `ContentError::code()` 一一对应：正文读取失败时
 * `invoke` 会 reject 一个 `{ code, message }` 对象，前端据此分类展示原因，不猜测。
 */

export interface SessionFilter {
  agent?: string;
  /** RFC3339 半开区间 [from, to) */
  from?: string;
  to?: string;
  /** 只匹配索引元数据（session_id / 标题），不扫正文 */
  query?: string;
}

export interface SessionItem {
  sessionKey: string;
  sourceId: string;
  agent: string;
  sessionId: string;
  parentSessionKey: string | null;
  parentSessionId: string | null;
  /** 可能为空；为空表示源未提供标题（不是空标题） */
  title: string | null;
  /** 本机项目引用，可能敏感，展示可隐藏 */
  projectRef: string | null;
  archived: boolean;
  hasParent: boolean;
  hasSubagents: boolean;
  /** 最后活跃时间；null = 时间未知 */
  lastActiveAt: string | null;
  eventCount: number;
  /** 十进制字符串；null = 没有已知总量的记录（不显示 0） */
  tokenTotal: string | null;
  tokenKnownEvents: number;
  /** 该会话出现过的模型名（去重、排序）；空数组 = 源未提供 */
  models: string[];
}

export interface SessionsPage {
  items: SessionItem[];
  /** `(lastActiveAt, sessionKey)` 的不透明游标；null = 已到末尾 */
  nextCursor: string | null;
  hasMore: boolean;
}

/** 正文读取请求：只含索引身份（源 + 会话 + 可选 actor），不含任何系统路径。 */
export interface SessionRef {
  sourceId: string;
  sessionId: string;
  actorId?: string;
}

export interface ContentBlock {
  role: string;
  text: string | null;
  toolName: string | null;
  isCollapsibleTool: boolean;
  truncated: boolean;
  time: string | null;
}

export interface ContentPage {
  sourceId: string;
  sessionId: string;
  actorId: string | null;
  agent: string;
  /** 源根目录内的相对定位；不含本机绝对路径 */
  locatedPath: string | null;
  blocks: ContentBlock[];
  /** 下一页字节游标（十进制字符串）；null = 已到末尾 */
  nextCursor: string | null;
  hasMore: boolean;
  /** 只含偏移与短码，不含正文与原始 JSON */
  diagnostics: string[];
}

export type ContentErrorCode =
  | "ContentDisabled"
  | "SessionNotFound"
  | "SourceGone"
  | "PermissionDenied"
  | "Incompatible"
  | "UnsupportedSource"
  | "PathRejected"
  | "TooLarge"
  | "Internal";

export interface ContentError {
  code: ContentErrorCode | string;
  message: string;
}
