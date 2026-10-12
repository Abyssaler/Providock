import type { UsageFilter } from "../../contracts/usage";

export type Period = "1h" | "3h" | "6h" | "today" | "7d" | "30d" | "all" | "custom";
export const AGENTS = [
  ["claude_code", "Claude Code"],
  ["codex", "Codex"],
  ["gemini_cli", "Gemini CLI"],
  ["grok_build", "Grok Build"],
  ["opencode", "OpenCode"],
  ["pi", "Pi"],
  ["mcode", "MCode"],
  ["kimi_code", "Kimi Code"],
  ["deepseek_harness", "DeepSeek Harness"],
] as const;
export function agentLabel(id: string): string {
  return AGENTS.find(([key]) => key === id)?.[1] ?? id;
}

// ---------------------------------------------------------------------------
// Agent 可见性（设置页「用量页显示的 Agent」，后端契约 agent_visibility）
// ---------------------------------------------------------------------------

/** 全部 Agent id，顺序与 AGENTS 一致。 */
export function allAgentIds(): string[] {
  return AGENTS.map(([id]) => id);
}

/**
 * 规范化可见集合：去掉未知 id、按 AGENTS 顺序去重；空集合、null/undefined
 * （含浏览器预览拿不到 IPC）一律视为全部可见。
 */
export function normalizeVisibleAgents(visible: string[] | null | undefined): string[] {
  if (!visible) return allAgentIds();
  const known = new Set(visible);
  const ordered = allAgentIds().filter((id) => known.has(id));
  return ordered.length ? ordered : allAgentIds();
}

/** 用量页选项卡列表：只含可见 Agent，保持 AGENTS 顺序（「全部」由调用方单独渲染）。 */
export function visibleAgentTabs(visible: string[] | null | undefined): (typeof AGENTS)[number][] {
  const set = new Set(normalizeVisibleAgents(visible));
  return AGENTS.filter(([id]) => set.has(id));
}

/** 单个 Agent 是否在可见集合内。 */
export function isAgentVisible(visible: string[] | null | undefined, id: string): boolean {
  return normalizeVisibleAgents(visible).includes(id);
}

/**
 * 查询过滤的 `agents` 字段：可见集合是全部 Agent（或未知）时省略（后端不加限制），
 * 否则返回需要限定的 id 列表。
 */
export function agentsFilter(visible: string[] | null | undefined): string[] | undefined {
  const normalized = normalizeVisibleAgents(visible);
  return normalized.length === allAgentIds().length ? undefined : normalized;
}

/** 日期按用户本地日历转换为 UTC 半开区间；结束日期包含整天。 */
export function periodFilter(period: Period, now = new Date(), start = "", end = ""): UsageFilter {
  if (period === "all") return {};
  if (period === "1h" || period === "3h" || period === "6h") {
    return {
      from: new Date(now.getTime() - Number(period[0]) * 3_600_000).toISOString(),
      to: now.toISOString(),
    };
  }
  if (period === "custom") {
    if (start.includes("T") || end.includes("T")) {
      const pattern = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/;
      const from = new Date(start);
      const to = new Date(end);
      const exact = (date: Date, raw: string) => {
        if (!Number.isFinite(date.getTime())) return false;
        const pad = (n: number) => String(n).padStart(2, "0");
        return (
          `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}` ===
          raw
        );
      };
      if (
        !pattern.test(start) ||
        !pattern.test(end) ||
        !exact(from, start) ||
        !exact(to, end) ||
        from >= to
      )
        throw new Error("请选择有效的起止时间，结束时间须晚于开始时间");
      return { from: from.toISOString(), to: to.toISOString() };
    }
    if (!/^\d{4}-\d{2}-\d{2}$/.test(start) || !/^\d{4}-\d{2}-\d{2}$/.test(end) || start > end)
      throw new Error("请选择有效的开始和结束日期");
    const from = new Date(`${start}T00:00:00`);
    const to = new Date(`${end}T00:00:00`);
    const matchesCalendar = (date: Date, value: string) =>
      Number.isFinite(date.getTime()) &&
      date.getFullYear() === Number(value.slice(0, 4)) &&
      date.getMonth() + 1 === Number(value.slice(5, 7)) &&
      date.getDate() === Number(value.slice(8, 10));
    if (!matchesCalendar(from, start) || !matchesCalendar(to, end)) throw new Error("日期无效");
    to.setDate(to.getDate() + 1);
    if (!Number.isFinite(from.getTime()) || !Number.isFinite(to.getTime()))
      throw new Error("日期无效");
    return { from: from.toISOString(), to: to.toISOString() };
  }
  const from = new Date(now);
  from.setHours(0, 0, 0, 0);
  if (period !== "today") from.setDate(from.getDate() - (period === "7d" ? 6 : 29));
  return { from: from.toISOString(), to: now.toISOString() };
}

export function usesHourlyBuckets(filter: UsageFilter): boolean {
  return (
    !!filter.from &&
    !!filter.to &&
    new Date(filter.to).getTime() - new Date(filter.from).getTime() <= 48 * 3_600_000
  );
}

export function percentage(value: string | null): string {
  return value === null ? "—" : `${(Number(value) * 100).toFixed(1)}%`;
}

/** 中文紧凑显示；只处理呈现，不重算后端汇总。 */
export function formatUsageCompact(value: string | null): string {
  if (value === null) return "—";
  try {
    const amount = BigInt(value);
    const [divisor, unit] =
      amount >= 100_000_000n
        ? [100_000_000n, "亿"]
        : amount >= 10_000n
          ? [10_000n, "万"]
          : [1n, ""];
    if (!unit) return amount.toLocaleString("zh-CN");
    const scaled = (amount * 100n + divisor / 2n) / divisor;
    const fraction = (scaled % 100n).toString().padStart(2, "0").replace(/0+$/, "");
    return `${scaled / 100n}${fraction ? `.${fraction}` : ""} ${unit}`;
  } catch {
    return "—";
  }
}

/** Describe the exact filter used for the last completed dashboard request. */
export function usageScopeText(filter: UsageFilter): string {
  const date = (value: string) =>
    new Date(value).toLocaleString("zh-CN", {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    });
  if (!filter.from && !filter.to) return "全部时间";
  return `${filter.from ? date(filter.from) : "最早记录"} — ${filter.to ? date(filter.to) : "现在"}（本机时间）`;
}
