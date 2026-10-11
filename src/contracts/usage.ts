/**
 * IPC 契约（与 src-tauri 的 serde DTO 对应，rename_all = camelCase）。
 * 统计数字一律为十进制字符串或 null；前端不做汇总计算（ARCHITECTURE 第 6 节）。
 */

export interface MetricTotal {
  value: string | null;
  knownEvents: number;
}

export interface MoneyTotal {
  currency: string;
  amount: string;
}

export interface AgentBreakdown {
  agent: string;
  total: MetricTotal;
  events: number;
}

export interface UsageSummary {
  events: number;
  timeUnknownEvents: number;
  requestCount: string | null;
  requestCountKnownEvents: number;
  inputFresh: MetricTotal;
  inputCacheRead: MetricTotal;
  inputCacheWrite: MetricTotal;
  inputTotal: MetricTotal;
  outputTotal: MetricTotal;
  total: MetricTotal;
  cacheReadRatio: string | null;
  cacheRatioEligibleEvents: number;
  costReported: MoneyTotal[];
  costEstimated: MoneyTotal[];
  byAgent: AgentBreakdown[];
}

export interface UsageFilter {
  from?: string;
  to?: string;
  agent?: string;
  streamDomain?: string;
}

export type UsageSeriesPoint = [string, string | null];

export interface SourceScanReport {
  sourceId: string;
  agent: string;
  filesSeen: number;
  filesScanned: number;
  filesUnchanged: number;
  filesDeferred: number;
  eventsInserted: number;
  eventsReplaced: number;
  eventsAlreadyPresent: number;
  diagnostics: string[];
}

export interface SourceInfo {
  sourceId: string;
  agent: string;
  rootPath: string;
  displayLabel: string | null;
  parserVersion: string;
  compatStatus: string;
  enabled: boolean;
  lastSyncAt: string | null;
  events: number;
}
