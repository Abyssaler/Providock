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

describe("滚动小时与自定义时间", () => {
  it("按实际小时跨越本机零点，而非按当天截断", () => {
    const now = new Date(2026, 9, 13, 0, 30);
    for (const [period, hours] of [
      ["1h", 1],
      ["3h", 3],
      ["6h", 6],
    ] as const) {
      const range = periodFilter(period, now);
      expect(new Date(range.to!).getTime() - new Date(range.from!).getTime()).toBe(
        hours * 3_600_000,
      );
      expect(new Date(range.from!).getDate()).toBe(12);
    }
  });
  it("自定义精确到分钟，结束边界不扩展一天", () => {
    const range = periodFilter("custom", new Date(), "2026-10-12T22:15", "2026-10-13T00:20");
    expect(new Date(range.from!).getMinutes()).toBe(15);
    expect(new Date(range.to!).getHours()).toBe(0);
    expect(new Date(range.to!).getMinutes()).toBe(20);
    for (const [start, end] of [
      ["2026-02-30T12:00", "2026-03-01T12:00"],
      ["2026-10-13T00:00", "2026-10-13T00:00"],
      ["2026-10-13T02:00", "2026-10-13T01:00"],
    ])
      expect(() => periodFilter("custom", new Date(), start, end)).toThrow();
  });
});
