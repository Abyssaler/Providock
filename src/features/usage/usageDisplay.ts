import type { UsageFilter } from "../../contracts/usage";

export type Period = "today" | "7d" | "30d" | "all" | "custom";
export const AGENTS = [
  ["claude_code", "Claude Code"],
  ["codex", "Codex"],
  ["gemini_cli", "Gemini CLI"],
  ["grok_build", "Grok Build"],
  ["opencode", "OpenCode"],
  ["pi", "Pi"],
  ["mcode", "MCode"],
  ["kimi_code", "Kimi Code"],
] as const;
export function agentLabel(id: string): string {
  return AGENTS.find(([key]) => key === id)?.[1] ?? id;
}

/** 日期按用户本地日历转换为 UTC 半开区间；结束日期包含整天。 */
export function periodFilter(period: Period, now = new Date(), start = "", end = ""): UsageFilter {
  if (period === "all") return {};
  if (period === "custom") {
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
