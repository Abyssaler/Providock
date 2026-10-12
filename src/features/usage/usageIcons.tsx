/**
 * 用量页原创图标：全部是简单几何描边（stroke 继承 currentColor），
 * 不使用任何第三方品牌资产或 emoji 字符。
 */
import type { ReactNode } from "react";

const STROKE = {
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.5,
  strokeLinecap: "round",
  strokeLinejoin: "round",
} as const;

function Glyph({ children }: { children: ReactNode }) {
  return (
    <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true" {...STROKE}>
      {children}
    </svg>
  );
}

/** Agent 选项卡/设置行的小图标；未知 id 回退为圆点占位。 */
export function AgentGlyph({ id }: { id: string }) {
  switch (id) {
    case "all":
      return (
        <Glyph>
          <rect x="2.2" y="2.2" width="4.6" height="4.6" rx="1.2" />
          <rect x="9.2" y="2.2" width="4.6" height="4.6" rx="1.2" />
          <rect x="2.2" y="9.2" width="4.6" height="4.6" rx="1.2" />
          <rect x="9.2" y="9.2" width="4.6" height="4.6" rx="1.2" />
        </Glyph>
      );
    case "claude_code":
      return (
        <Glyph>
          <path d="M8 2.4v11.2M2.4 8h11.2M4 4l8 8M12 4l-8 8" />
        </Glyph>
      );
    case "codex":
      return (
        <Glyph>
          <circle cx="8" cy="8" r="5.6" />
          <circle cx="8" cy="8" r="1.3" fill="currentColor" stroke="none" />
        </Glyph>
      );
    case "gemini_cli":
      return (
        <Glyph>
          <path d="M8 2.2c.55 3.1 2.7 5.25 5.8 5.8-3.1.55-5.25 2.7-5.8 5.8-.55-3.1-2.7-5.25-5.8-5.8 3.1-.55 5.25-2.7 5.8-5.8Z" />
        </Glyph>
      );
    case "grok_build":
      return (
        <Glyph>
          <path d="M8 2.2 13.8 8 8 13.8 2.2 8Z" />
          <path d="M5.6 8h4.8" />
        </Glyph>
      );
    case "opencode":
      return (
        <Glyph>
          <path d="M6 3.6 2.8 8 6 12.4M10 3.6 13.2 8 10 12.4" />
        </Glyph>
      );
    case "pi":
      return (
        <Glyph>
          <path d="M2.8 5.4h10.4M5.4 5.4v6.8M10.6 5.4v6.8" />
        </Glyph>
      );
    case "mcode":
      return (
        <Glyph>
          <path d="M3.4 12.8V3.6L8 9l4.6-5.4v9.2" />
        </Glyph>
      );
    case "deepseek_harness":
      return (
        <Glyph>
          <path d="M3 4.5h6.5a4 4 0 0 1 0 8H6L3 9.5V4.5ZM10 4.5l2-2" />
        </Glyph>
      );
    case "kimi_code":
      return (
        <Glyph>
          <path d="M4.4 3v10M12.2 3.4 6.6 8l5.6 4.6" />
        </Glyph>
      );
    default:
      return (
        <Glyph>
          <circle cx="8" cy="8" r="2" />
        </Glyph>
      );
  }
}

/** 数据源：三层堆叠。 */
export function SourceIcon() {
  return (
    <Glyph>
      <path d="m8 2.4 5.8 2.7L8 7.8 2.2 5.1Z" />
      <path d="m2.2 8.2 5.8 2.7 5.8-2.7" />
      <path d="m2.2 11.2 5.8 2.7 5.8-2.7" />
    </Glyph>
  );
}

/** 模型：标签。 */
export function ModelIcon() {
  return (
    <Glyph>
      <path d="M2.4 6.9V3.4a1 1 0 0 1 1-1h3.5a1 1 0 0 1 .7.3l5.9 5.9a1 1 0 0 1 0 1.4l-3.9 3.9a1 1 0 0 1-1.4 0L2.7 7.6a1 1 0 0 1-.3-.7Z" />
      <circle cx="5.6" cy="5.6" r="1" fill="currentColor" stroke="none" />
    </Glyph>
  );
}

/** 显示刷新：时钟。 */
export function ClockIcon() {
  return (
    <Glyph>
      <circle cx="8" cy="8" r="5.7" />
      <path d="M8 4.7V8l2.3 1.5" />
    </Glyph>
  );
}

/** 日期范围：日历。 */
export function CalendarIcon() {
  return (
    <Glyph>
      <rect x="2.4" y="3.4" width="11.2" height="10.2" rx="1.4" />
      <path d="M2.4 6.6h11.2M5.4 1.9v2.9M10.6 1.9v2.9" />
    </Glyph>
  );
}

/** 模型定价入口：菱形。 */
export function PriceIcon() {
  return (
    <Glyph>
      <path d="M8 2.2 13.8 8 8 13.8 2.2 8Z" />
    </Glyph>
  );
}

/** 概览主数：闪电。 */
export function BoltIcon() {
  return (
    <Glyph>
      <path d="M8.9 1.9 3.7 9.1h3.5l-.9 5 5.9-7.5H8.6Z" />
    </Glyph>
  );
}

/** 指标卡：新增输入（箭头进入）。 */
export function InputIcon() {
  return (
    <Glyph>
      <path d="M2.2 8h8.2M7 4.6 10.4 8 7 11.4" />
      <path d="M13.8 3.4v9.2" />
    </Glyph>
  );
}

/** 指标卡：输出（箭头离开）。 */
export function OutputIcon() {
  return (
    <Glyph>
      <path d="M2.2 3.4v9.2" />
      <path d="M5.6 8h8.2M10.4 4.6 13.8 8l-3.4 3.4" />
    </Glyph>
  );
}

/** 指标卡：缓存创建（方块加号）。 */
export function CacheWriteIcon() {
  return (
    <Glyph>
      <rect x="2.4" y="2.4" width="11.2" height="11.2" rx="2" />
      <path d="M8 5.6v4.8M5.6 8h4.8" />
    </Glyph>
  );
}

/** 指标卡：缓存命中（方块对勾）。 */
export function CacheReadIcon() {
  return (
    <Glyph>
      <rect x="2.4" y="2.4" width="11.2" height="11.2" rx="2" />
      <path d="m5.4 8.2 1.8 1.8 3.4-3.8" />
    </Glyph>
  );
}

/** 指标卡：缓存命中率（百分号）。 */
export function RatioIcon() {
  return (
    <Glyph>
      <circle cx="5" cy="5" r="2.1" />
      <circle cx="11" cy="11" r="2.1" />
      <path d="m12.6 3.4-9.2 9.2" />
    </Glyph>
  );
}
