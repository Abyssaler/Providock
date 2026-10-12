import { describe, expect, it } from "vitest";
import { formatTokenCompact, formatTokenFull } from "../src/lib/format";

describe("formatTokenCompact", () => {
  it("未知值显示占位符，不显示 0", () => {
    expect(formatTokenCompact(null)).toBe("—");
    expect(formatTokenCompact("abc")).toBe("—");
  });

  it("小数值保持原样", () => {
    expect(formatTokenCompact("0")).toBe("0");
    expect(formatTokenCompact("999")).toBe("999");
  });

  it("大数值压缩且不以浮点运算", () => {
    expect(formatTokenCompact("1200")).toBe("1.2K");
    expect(formatTokenCompact("2300000")).toBe("2.3M");
    expect(formatTokenCompact("1000000")).toBe("1M");
    // 超过 2^53 也不能失真
    expect(formatTokenCompact("9007199254740993123")).toBe("9007199254.7B");
  });
});

describe("formatTokenFull", () => {
  it("千分位且保留原值", () => {
    expect(formatTokenFull("2300000")).toBe("2,300,000");
    expect(formatTokenFull(null)).toBe("未提供");
  });
});

describe("金额两位小数显示", () => {
  it("只舍入显示，不丢失大整数精度", async () => {
    const { formatMoneyAmount } = await import("../src/lib/format");
    expect(formatMoneyAmount("180.8315768378")).toBe("180.83");
    expect(formatMoneyAmount("9.995")).toBe("10.00");
    expect(formatMoneyAmount("9007199254740993.125")).toBe("9007199254740993.13");
    expect(formatMoneyAmount("0")).toBe("0.00");
    expect(formatMoneyAmount("0.0049")).toBe("0.00");
    expect(formatMoneyAmount("unknown")).toBe("—");
  });
});
