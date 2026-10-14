/**
 * 定价策略（分时段高峰 / 长上下文分档）的表单状态：序列化、校验、回填。
 * 契约见 contracts/pricing.ts（RateSet / PeakWindowSpec / LongContextSpec）。
 * 保存时只写入已启用 section 的字段；未启用 = 键不存在（后端按未配置处理）。
 */
import type { LongContextSpec, ModelPrice, PeakWindowSpec, RateSet } from "../../contracts/pricing";

export const RATE_KEYS = ["input", "output", "cacheRead", "cacheWrite"] as const;
export type RateKey = (typeof RATE_KEYS)[number];

export const DEFAULT_RANGES = ["09:00-12:00", "14:00-18:00"];
export const PEAK_TIMEZONE = "Asia/Shanghai";

export interface StrategyForm {
  peakOn: boolean;
  weekdaysOnly?: boolean;
  excludeChinaHolidays?: boolean;
  longOn: boolean;
  /** 空字符串 = 未填；保存时剔除空值键。 */
  peakRates: RateSet;
  ranges: string[];
  longRates: RateSet;
  /** 长上下文阈值，十进制整数字符串。 */
  threshold: string;
}

const DECIMAL = /^\d{1,9}(\.\d{1,9})?$/;
const RANGE = /^([01]\d|2[0-3]):([0-5]\d)-([01]\d|2[0-3]):([0-5]\d)$/;

/** "HH:MM-HH:MM"，零点补齐、开始早于结束（零填充下字典序即时间序）。 */
export function isValidRange(range: string): boolean {
  const m = RANGE.exec(range.trim());
  if (!m) return false;
  return `${m[1]}:${m[2]}` < `${m[3]}:${m[4]}`;
}

function cleanRates(rates: RateSet): RateSet {
  const out: RateSet = {};
  for (const key of RATE_KEYS) {
    const value = rates[key]?.trim();
    if (value) out[key] = value;
  }
  return out;
}

/** 表单 → 保存契约。未启用的策略字段为 undefined（序列化后键不存在）。 */
export function serializeModelPrice(draft: ModelPrice, form: StrategyForm): ModelPrice {
  const longRates = cleanRates(form.longRates);
  const longContext: LongContextSpec | undefined = form.longOn
    ? { inputOverTokens: Number(form.threshold.trim()), ...longRates }
    : undefined;
  const peakWindows: PeakWindowSpec | undefined = form.peakOn
    ? {
        timezone: PEAK_TIMEZONE,
        weekdaysOnly: form.weekdaysOnly ?? true,
        excludeChinaHolidays: form.excludeChinaHolidays ?? false,
        ranges: form.ranges.map((r) => r.trim()).filter(Boolean),
      }
    : undefined;
  return {
    ...draft,
    peak: form.peakOn ? cleanRates(form.peakRates) : undefined,
    peakWindows,
    longContext,
  };
}

/** 返回第一条错误文案；全部通过返回 null。 */
export function validateModelPrice(draft: ModelPrice, form: StrategyForm): string | null {
  for (const key of RATE_KEYS) {
    const value = draft[key]?.trim();
    if (value && !DECIMAL.test(value)) return "价格需为最多 9 位整数、9 位小数的数字";
  }
  if (form.peakOn) {
    for (const key of RATE_KEYS) {
      const value = form.peakRates[key]?.trim();
      if (value && !DECIMAL.test(value)) return "高峰价格需为最多 9 位整数、9 位小数的数字";
    }
    if (!form.ranges.some((range) => range.trim())) return "请至少填写一个高峰时段";
    for (const range of form.ranges) {
      if (range.trim() && !isValidRange(range))
        return "高峰时段格式应为 HH:MM-HH:MM，且开始早于结束";
    }
  }
  if (form.longOn) {
    if (!/^\d+$/.test(form.threshold.trim()) || Number(form.threshold) <= 0)
      return "长上下文阈值需为大于 0 的整数 Token 数";
    for (const key of RATE_KEYS) {
      const value = form.longRates[key]?.trim();
      if (value && !DECIMAL.test(value)) return "长上下文价格需为最多 9 位整数、9 位小数的数字";
    }
  }
  return null;
}

export function blankStrategyForm(): StrategyForm {
  return {
    peakOn: false,
    weekdaysOnly: true,
    excludeChinaHolidays: false,
    longOn: false,
    peakRates: {},
    ranges: [...DEFAULT_RANGES],
    longRates: {},
    threshold: "",
  };
}

/** 已保存规则 → 表单状态（编辑回填；后端字段缺失时按默认值补齐）。 */
export function strategyFormOf(rule: ModelPrice): StrategyForm {
  const peak = rule.peak ?? {};
  const long = rule.longContext ?? null;
  return {
    peakOn: rule.peak != null || rule.peakWindows != null,
    weekdaysOnly: rule.peakWindows?.weekdaysOnly ?? true,
    excludeChinaHolidays: rule.peakWindows?.excludeChinaHolidays ?? false,
    longOn: long != null,
    peakRates: { ...peak },
    ranges: rule.peakWindows?.ranges?.length ? [...rule.peakWindows.ranges] : [...DEFAULT_RANGES],
    longRates: long
      ? {
          input: long.input,
          output: long.output,
          cacheRead: long.cacheRead,
          cacheWrite: long.cacheWrite,
        }
      : {},
    threshold: long ? String(long.inputOverTokens) : "",
  };
}
