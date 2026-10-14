import type { MoneyTotal } from "./usage";

/** 每百万 Token 费率；字段可选，缺省 = 未定价。十进制字符串。 */
export interface RateSet {
  input?: string;
  output?: string;
  cacheRead?: string;
  cacheWrite?: string;
}

/** 高峰时间窗（后端当前固定 Asia/Shanghai + 仅工作日）。 */
export interface PeakWindowSpec {
  timezone: string;
  weekdaysOnly: boolean;
  excludeChinaHolidays?: boolean;
  /** `"HH:MM-HH:MM"`（如 `"09:00-12:00"`），开始早于结束。 */
  ranges: string[];
}

/** 长上下文分档：超过 inputOverTokens 的整请求按此档费率计费。 */
export type LongContextSpec = { inputOverTokens: number } & RateSet;

export type ContextTier = RateSet & {
  label: string;
  inputMin: number | null;
  inputMax: number | null;
  outputMin: number | null;
  outputMax: number | null;
};

export interface ModelPrice {
  contextTiers?: ContextTier[] | null;
  version: string;
  agent: string;
  sourceId: string | null;
  model: string;
  currency: "USD" | "CNY";
  input: string | null;
  output: string | null;
  cacheRead: string | null;
  cacheWrite: string | null;
  enabled: boolean;
  source?: {
    sourceUrl: string;
    officialUrl: string;
    retrievedAt: string;
    sourceUpdatedAt: string;
    fxRate: string;
    fxDate: string;
  } | null;
  /** 分时段（高峰）费率；null/缺省 = 未配置。 */
  peak?: RateSet | null;
  peakWindows?: PeakWindowSpec | null;
  longContext?: LongContextSpec | null;
}
export interface UsageTokens {
  inputFresh: string | null;
  inputCacheRead: string | null;
  inputCacheWrite: string | null;
  inputTotal: string | null;
  outputTotal: string | null;
  total: string | null;
}
export interface UsageGroup {
  name: string;
  agent: string | null;
  events: number;
  tokens: UsageTokens;
  currentCost: MoneyTotal[];
  pricedEvents: number;
  completeCostEvents: number;
}
export interface UsagePoint {
  at: string;
  tokens: UsageTokens;
  currentCost: MoneyTotal[];
}
export interface UsageDashboard {
  models: UsageGroup[];
  agents: UsageGroup[];
  series: UsagePoint[];
  currentCost: MoneyTotal[];
  pricedEvents: number;
  completeCostEvents: number;
}
export interface UsageEventRow {
  eventId: string;
  occurredAt: string | null;
  agent: string;
  model: string | null;
  sourceId: string;
  granularity: string;
  tokens: UsageTokens;
  costCalculated: (MoneyTotal & { complete: boolean; version: string }) | null;
}
export interface UsageEventPage {
  rows: UsageEventRow[];
  total: number;
  page: number;
  pageSize: number;
}

export interface PriceImportResult {
  imported: number;
  preserved: number;
  unmatched: string[];
  catalogueModels: number;
  sourceUpdatedAt: string;
  fxRate: string;
  fxDate: string;
}

export interface PriceQuote {
  peak?: RateSet | null;
  peakWindows?: PeakWindowSpec | null;
  contextTiers?: ContextTier[] | null;
  agent: string;
  model: string;
  currency: "USD" | "CNY";
  input: string | null;
  output: string | null;
  cacheRead: string | null;
  cacheWrite: string | null;
  sourceUrl: string;
  method: string;
  notes: string[];
}
