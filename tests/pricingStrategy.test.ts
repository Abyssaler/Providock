import { describe, expect, it } from "vitest";
import {
  DEFAULT_RANGES,
  blankStrategyForm,
  isValidRange,
  serializeModelPrice,
  strategyFormOf,
  validateModelPrice,
} from "../src/features/usage/pricingStrategy";
import type { ModelPrice } from "../src/contracts/pricing";

const base: ModelPrice = {
  version: "",
  agent: "codex",
  sourceId: null,
  model: "gpt-5",
  currency: "CNY",
  input: "1",
  output: null,
  cacheRead: null,
  cacheWrite: null,
  enabled: true,
};

describe("定价策略序列化", () => {
  it("基础规则不携带任何策略字段", () => {
    const payload = serializeModelPrice(base, blankStrategyForm());
    expect(payload.peak).toBeUndefined();
    expect(payload.peakWindows).toBeUndefined();
    expect(payload.longContext).toBeUndefined();
    const json = JSON.stringify(payload);
    expect(json).not.toContain("peakWindows");
    expect(json).not.toContain("longContext");
    expect(payload.input).toBe("1");
    expect(payload.enabled).toBe(true);
  });
  it("高峰规则写入费率与时间窗，空费率键被剔除", () => {
    const form = {
      ...blankStrategyForm(),
      peakOn: true,
      peakRates: { input: "2", output: "", cacheRead: "0.5" },
    };
    const payload = serializeModelPrice(base, form);
    expect(payload.peak).toEqual({ input: "2", cacheRead: "0.5" });
    expect(payload.peakWindows).toEqual({
      timezone: "Asia/Shanghai",
      weekdaysOnly: true,
      excludeChinaHolidays: false,
      ranges: DEFAULT_RANGES,
    });
    expect(payload.longContext).toBeUndefined();
  });
  it("长上下文阈值序列化为数字", () => {
    const form = {
      ...blankStrategyForm(),
      longOn: true,
      threshold: "272000",
      longRates: { input: "8" },
    };
    const payload = serializeModelPrice(base, form);
    expect(payload.longContext).toEqual({ inputOverTokens: 272000, input: "8" });
    expect(payload.peak).toBeUndefined();
  });
  it("关闭 section 后再次序列化不再携带字段", () => {
    const form = { ...blankStrategyForm(), peakOn: true, longOn: true, threshold: "1" };
    expect(serializeModelPrice(base, form).peak).toBeDefined();
    const off = serializeModelPrice(base, { ...form, peakOn: false, longOn: false });
    expect(off.peak).toBeUndefined();
    expect(off.longContext).toBeUndefined();
  });
  it("编辑回填：已保存策略还原为表单状态", () => {
    const rule: ModelPrice = {
      ...base,
      version: "v1",
      peak: { input: "2" },
      peakWindows: { timezone: "Asia/Shanghai", weekdaysOnly: true, ranges: ["09:00-12:00"] },
      longContext: { inputOverTokens: 100000, output: "9" },
    };
    const form = strategyFormOf(rule);
    expect(form.peakOn).toBe(true);
    expect(form.peakRates).toEqual({ input: "2" });
    expect(form.ranges).toEqual(["09:00-12:00"]);
    expect(form.longOn).toBe(true);
    expect(form.threshold).toBe("100000");
    expect(form.longRates).toEqual({
      input: undefined,
      output: "9",
      cacheRead: undefined,
      cacheWrite: undefined,
    });
    expect(strategyFormOf(base).peakOn).toBe(false);
  });
});

describe("定价策略校验", () => {
  it("时段格式为 HH:MM-HH:MM 且开始早于结束", () => {
    expect(isValidRange("09:00-12:00")).toBe(true);
    expect(isValidRange("23:59-00:00")).toBe(false);
    expect(isValidRange("12:00-09:00")).toBe(false);
    expect(isValidRange("24:00-25:00")).toBe(false);
    expect(isValidRange("9:00-12:00")).toBe(false);
  });
  it("小数超界、时段缺失/非法、阈值非法时报错；通过时返回 null", () => {
    const ok = blankStrategyForm();
    expect(validateModelPrice(base, ok)).toBeNull();
    expect(validateModelPrice({ ...base, input: "1234567890" }, ok)).not.toBeNull();
    expect(validateModelPrice({ ...base, output: "1.0123456789" }, ok)).not.toBeNull();
    expect(validateModelPrice({ ...base, input: "123456789.1" }, ok)).toBeNull();
    expect(validateModelPrice(base, { ...ok, peakOn: true, ranges: [""] })).not.toBeNull();
    expect(
      validateModelPrice(base, { ...ok, peakOn: true, ranges: ["25:00-26:00"] }),
    ).not.toBeNull();
    expect(
      validateModelPrice(base, { ...ok, peakOn: true, ranges: ["12:00-09:00"] }),
    ).not.toBeNull();
    expect(validateModelPrice(base, { ...ok, peakOn: true, ranges: DEFAULT_RANGES })).toBeNull();
    expect(validateModelPrice(base, { ...ok, longOn: true, threshold: "" })).not.toBeNull();
    expect(validateModelPrice(base, { ...ok, longOn: true, threshold: "0" })).not.toBeNull();
    expect(validateModelPrice(base, { ...ok, longOn: true, threshold: "272000" })).toBeNull();
    expect(validateModelPrice(base, { ...ok, longOn: true, threshold: "1.5" })).not.toBeNull();
  });
});

describe("官网策略编辑保存", () => {
  it("保留节假日条件与组合分档，避免回填后静默丢失", () => {
    const peak: ModelPrice = {
      ...base,
      peak: { input: "2", output: "8" },
      peakWindows: {
        timezone: "Asia/Shanghai",
        weekdaysOnly: true,
        excludeChinaHolidays: true,
        ranges: DEFAULT_RANGES,
      },
    };
    expect(serializeModelPrice(peak, strategyFormOf(peak)).peakWindows).toEqual(peak.peakWindows);
    const tiers: ModelPrice = {
      ...base,
      contextTiers: [
        {
          label: "input >=32K",
          inputMin: 32000,
          inputMax: null,
          outputMin: null,
          outputMax: null,
          input: "8",
          output: "28",
        },
      ],
    };
    expect(serializeModelPrice(tiers, strategyFormOf(tiers)).contextTiers).toEqual(
      tiers.contextTiers,
    );
  });
});
