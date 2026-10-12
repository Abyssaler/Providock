import type { MoneyTotal } from "./usage";
export interface ModelPrice {
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
  costReported: MoneyTotal | null;
  costCalculated: (MoneyTotal & { complete: boolean; version: string }) | null;
}
export interface UsageEventPage {
  rows: UsageEventRow[];
  total: number;
  page: number;
  pageSize: number;
}
