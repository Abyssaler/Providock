import { describe, expect, it } from "vitest";
import { periodFilter, percentage, formatUsageCompact } from "../src/features/usage/usageDisplay";

describe("用量日期与未知值", () => {
  it("当天以本地零点为边界，最近七天包含当天", () => {
    const now = new Date(2026, 9, 11, 18, 30);
    const today = periodFilter("today", now);
    expect(new Date(today.from!).getDate()).toBe(11);
    expect(new Date(today.from!).getHours()).toBe(0);
    expect(today.to).toBe(now.toISOString());
    expect(new Date(periodFilter("7d", now).from!).getDate()).toBe(5);
    expect(periodFilter("all", now)).toEqual({});
  });
  it("自定义结束日包含整天，并拒绝无效范围", () => {
    const range = periodFilter("custom", new Date(), "2026-10-01", "2026-10-11");
    expect(new Date(range.to!).getDate()).toBe(12);
    expect(() => periodFilter("custom", new Date(), "2026-10-11", "2026-10-01")).toThrow();
    expect(() => periodFilter("custom")).toThrow();
    expect(() => periodFilter("custom", new Date(), "2026-02-30", "2026-03-01")).toThrow();
  });
  it("中文大数字保持精度且不把未知显示为零", () => {
    expect(formatUsageCompact("17946711")).toBe("1794.67 万");
    expect(formatUsageCompact("697159437")).toBe("6.97 亿");
    expect(formatUsageCompact(null)).toBe("—");
  });
  it("缓存比例未知不冒充零", () => {
    expect(percentage(null)).toBe("—");
    expect(percentage("0")).toBe("0.0%");
    expect(percentage("0.936")).toBe("93.6%");
  });
});
