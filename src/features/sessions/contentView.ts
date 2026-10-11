/**
 * 会话页的纯逻辑（无 React、无 IPC），便于单元测试。
 *
 * 安全口径（SECURITY_PRIVACY 第 5 节 / UI_UX 第 6 节）：正文是不可信输入，
 * 只做**白名单纯文本渲染**——代码块 <pre>、段落，其余一律纯文本；
 * 外部链接不自动加载、不生成可点击元素，只在点击前展示目标域名。
 */

export const AGENT_LABELS: Record<string, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  gemini_cli: "Gemini CLI",
  grok_build: "Grok Build",
  opencode: "OpenCode",
  pi: "Pi",
  mcode: "MiniMax Code",
  kimi_code: "Kimi Code",
  claude_desktop: "Claude Desktop（网关）",
};

export function agentLabel(agent: string): string {
  return AGENT_LABELS[agent] ?? agent;
}

/** 只接受 http/https 的绝对 URL，返回规范化主机名；其余（含 javascript:）返回 null。 */
export function displayDomain(href: string): string | null {
  let url: URL;
  try {
    url = new URL(href);
  } catch {
    return null;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return null;
  return url.hostname || null;
}

export interface TextSegment {
  kind: "text" | "code";
  value: string;
}

/** 按 ``` 围栏切分文本：围栏内为代码块（<pre>），围栏外为普通段落。 */
export function splitCodeSegments(text: string): TextSegment[] {
  const segments: TextSegment[] = [];
  let buffer: string[] = [];
  let kind: "text" | "code" = "text";

  const flush = () => {
    if (buffer.length === 0) return;
    const value = buffer.join("\n");
    if (kind === "code" || value.trim().length > 0) {
      segments.push({ kind, value });
    }
    buffer = [];
  };

  for (const line of text.split("\n")) {
    if (line.trimStart().startsWith("```")) {
      flush();
      kind = kind === "code" ? "text" : "code";
      continue;
    }
    buffer.push(line);
  }
  flush();
  return segments;
}

export interface LinkSegment {
  kind: "text" | "link";
  value: string;
  /** 链接的目标域名；无法解析或无安全协议时为 undefined */
  domain?: string;
}

const URL_PATTERN = /https?:\/\/[^\s<>"'`)\]]+/g;

/** 把文本按 http(s) URL 切分；URL 片段携带域名，供 UI 在点击前展示。 */
export function splitLinks(text: string): LinkSegment[] {
  const out: LinkSegment[] = [];
  let last = 0;
  for (const match of text.matchAll(URL_PATTERN)) {
    const start = match.index ?? 0;
    if (start > last) {
      out.push({ kind: "text", value: text.slice(last, start) });
    }
    const domain = displayDomain(match[0]);
    out.push({ kind: "link", value: match[0], domain: domain ?? undefined });
    last = start + match[0].length;
  }
  if (last < text.length) {
    out.push({ kind: "text", value: text.slice(last) });
  }
  return out;
}

/** 归一化 IPC 抛出的错误；未知形状归入 Internal，绝不假装成功。 */
export function parseContentError(err: unknown): { code: string; message: string } {
  if (typeof err === "object" && err !== null) {
    const candidate = err as { code?: unknown; message?: unknown };
    if (typeof candidate.code === "string" && candidate.code.length > 0) {
      return {
        code: candidate.code,
        message:
          typeof candidate.message === "string" && candidate.message.length > 0
            ? candidate.message
            : contentReasonLabel(candidate.code),
      };
    }
  }
  if (typeof err === "string" && err.length > 0) {
    return { code: "Internal", message: err };
  }
  return { code: "Internal", message: contentReasonLabel("Internal") };
}

/** 正文读取失败原因的中文分类说明（UI_UX 第 6/8 节：分别提示原因）。 */
export function contentReasonLabel(code: string): string {
  switch (code) {
    case "ContentDisabled":
      return "对话查看未开启。开启后只读取你选择的那个源与会话，不复制进应用数据库。";
    case "SessionNotFound":
      return "该会话不在本机索引中，已拒绝读取。";
    case "SourceGone":
      return "源文件已删除或移动。统计仍保留，但正文不可读。";
    case "PermissionDenied":
      return "权限不足，无法读取该源文件。";
    case "Incompatible":
      return "会话文件与读取器不兼容（版本或格式差异），未猜字段。";
    case "UnsupportedSource":
      return "该源的正文读取器尚未实现；统计可用，正文不伪造。";
    case "PathRejected":
      return "路径未通过根目录约束校验，已拒绝。";
    case "TooLarge":
      return "会话文件超过读取上限，未读取。";
    default:
      return "读取失败，未获得正文。";
  }
}

/** 正文块的展示角色名。 */
export function roleLabel(role: string): string {
  if (role.startsWith("other:")) return `其它（${role.slice(6)}）`;
  switch (role) {
    case "user":
      return "用户";
    case "assistant":
      return "助手";
    case "tool_use":
      return "工具调用";
    case "tool_result":
      return "工具结果";
    case "system":
      return "系统";
    default:
      return role;
  }
}
