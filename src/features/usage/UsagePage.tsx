import { SelectField as Select } from "../../components/Select";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  MetricTotal,
  SourceScanReport,
  SyncStatus,
  UsageOptions,
  UsageFilter,
  UsageSummary,
} from "../../contracts/usage";
import { formatTokenFull } from "../../lib/format";
import {
  agentLabel,
  agentsFilter,
  allAgentIds,
  formatUsageCompact,
  isAgentVisible,
  normalizeVisibleAgents,
  percentage,
  periodFilter,
  usageScopeText,
  usesHourlyBuckets,
  visibleAgentTabs,
  type Period,
} from "./usageDisplay";
import {
  AgentGlyph,
  BoltIcon,
  CalendarIcon,
  CacheReadIcon,
  CacheWriteIcon,
  ClockIcon,
  InputIcon,
  ModelIcon,
  OutputIcon,
  PriceIcon,
  RatioIcon,
  SourceIcon,
} from "./usageIcons";
import Hint from "../../components/Hint";
import ModelPricing from "./ModelPricing";
import UsageTrend from "./UsageTrend";
import UsageBreakdown, { moneyText } from "./UsageBreakdown";
import type { UsageDashboard } from "../../contracts/pricing";
import "./usage.css";

const EMPTY_STATUS: SyncStatus = {
  running: false,
  lastCompletedAt: null,
  lastError: null,
  reports: [],
};

function Metric({
  label,
  metric,
  events,
  hint,
  tone,
  icon,
}: {
  label: string;
  metric: MetricTotal;
  events: number;
  hint?: string;
  tone: "input" | "output" | "cacheWrite" | "cacheRead";
  icon: ReactNode;
}) {
  return (
    <div
      className="usage-metric"
      data-tone={tone}
      title={hint ?? `覆盖 ${metric.knownEvents}/${events} 条记录`}
    >
      <span className="usage-metric-head">
        {icon}
        {label}
      </span>
      <strong>{formatUsageCompact(metric.value)}</strong>
      {events > 0 && metric.knownEvents < events && (
        <small className="metric-partial">部分数据</small>
      )}
    </div>
  );
}
export default function UsagePage() {
  const [period, setPeriod] = useState<Period>("today");
  const [dates, setDates] = useState({ start: "", end: "" });
  const [dateDraft, setDateDraft] = useState({ start: "", end: "" });
  const [dateError, setDateError] = useState<string | null>(null);
  const [agent, setAgent] = useState("");
  const [sourceId, setSourceId] = useState("");
  const [model, setModel] = useState("");
  const [refreshSeconds, setRefreshSeconds] = useState(30);
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [dashboard, setDashboard] = useState<UsageDashboard | null>(null);
  const [revision, setRevision] = useState(0);
  const [detailFilter, setDetailFilter] = useState<UsageFilter>({});
  const [priceTarget, setPriceTarget] = useState<{ agent: string; model: string } | null>(null);
  const [options, setOptions] = useState<UsageOptions>({ sources: [], models: [] });
  const [visibleAgents, setVisibleAgents] = useState<string[]>(() => allAgentIds());
  const [status, setStatus] = useState<SyncStatus>(EMPTY_STATUS);
  const [manualBusy, setManualBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const request = useRef(0);

  const refresh = useCallback(async () => {
    const version = ++request.current;
    try {
      if (period === "custom" && (!dates.start || !dates.end)) return;
      const queryFilter = {
        ...periodFilter(period, new Date(), dates.start, dates.end),
        agent: agent || undefined,
        sourceId: sourceId || undefined,
        // 模型是跨 Agent 的全局维度（同一模型可出现在多个 Agent 里），不随可见性裁剪。
        model: model || undefined,
        agents: agentsFilter(visibleAgents),
      };
      const [nextSummary, nextDashboard, nextOptions] = await Promise.all([
        invoke<UsageSummary>("usage_summary", { filter: queryFilter }),
        invoke<UsageDashboard>("usage_dashboard", {
          filter: queryFilter,
          hourly: usesHourlyBuckets(queryFilter),
        }),
        invoke<UsageOptions>("usage_options"),
      ]);
      if (version !== request.current) return;
      setSummary(nextSummary);
      setDashboard(nextDashboard);
      setDetailFilter(queryFilter);
      setRevision((v) => v + 1);
      setOptions(nextOptions);
      setError(null);
    } catch (e) {
      if (version === request.current) setError(String(e));
    }
  }, [period, dates.start, dates.end, agent, sourceId, model, visibleAgents]);

  useEffect(() => {
    void refresh();
    return () => {
      request.current++;
    };
  }, [refresh]);
  useEffect(() => {
    if (refreshSeconds === 0) return;
    const timer = window.setInterval(() => {
      if (!document.hidden) void refresh();
    }, refreshSeconds * 1000);
    const onVisible = () => {
      if (!document.hidden) void refresh();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [refreshSeconds, refresh]);
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    void listen<SyncStatus>("usage-sync-updated", ({ payload }) => {
      if (!active) return;
      setStatus(payload);
      if (!payload.running) void refresh();
    })
      .then((stop) => {
        if (active) unlisten = stop;
        else stop();
      })
      .catch(() => {});
    void invoke<SyncStatus>("usage_sync_status")
      .then((value) => {
        if (active) setStatus(value);
      })
      .catch(() => {});
    return () => {
      active = false;
      unlisten?.();
    };
  }, [refresh]);

  // 用量页可见 Agent：设置页可裁剪；IPC 不可用（浏览器预览）时保持全部可见。
  useEffect(() => {
    let active = true;
    invoke<{ visibleAgents: string[] }>("agent_visibility")
      .then((value) => {
        if (active) setVisibleAgents(normalizeVisibleAgents(value.visibleAgents));
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, []);

  // 选中的 Agent 被隐藏时退回「全部」，避免停留在一个不可见的选择上。
  useEffect(() => {
    if (agent && !isAgentVisible(visibleAgents, agent)) {
      setAgent("");
      setSourceId("");
    }
  }, [agent, visibleAgents]);

  const onSync = async () => {
    setManualBusy(true);
    setError(null);
    try {
      await invoke<SourceScanReport[]>("sync_start");
      setStatus(await invoke<SyncStatus>("usage_sync_status"));
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setManualBusy(false);
    }
  };
  const busy = status.running || manualBusy;
  return (
    <section className="usage-page" aria-label="用量页面">
      <div className="usage-heading">
        <div>
          <h1>使用统计</h1>
        </div>
        <div className="actions">
          <button
            className="usage-price-button"
            onClick={() => setPriceTarget({ agent: agent || "codex", model })}
          >
            <PriceIcon /> 模型定价
          </button>
          <button disabled={busy} onClick={() => void onSync()}>
            {busy ? "同步中…" : "同步"}
          </button>
        </div>
      </div>
      <div className="usage-toolbar">
        <div className="agent-tabs" role="group" aria-label="Agent 筛选">
          <button
            className={!agent ? "active" : ""}
            aria-pressed={!agent}
            onClick={() => {
              setAgent("");
              setSourceId("");
            }}
          >
            <span className="agent-glyph" aria-hidden="true">
              <AgentGlyph id="all" />
            </span>
            全部
          </button>
          {visibleAgentTabs(visibleAgents).map(([id, label]) => (
            <button
              key={id}
              className={agent === id ? "active" : ""}
              aria-pressed={agent === id}
              onClick={() => {
                setAgent(id);
                setSourceId("");
              }}
            >
              <span className="agent-glyph" aria-hidden="true">
                <AgentGlyph id={id} />
              </span>
              <span>{label}</span>
            </button>
          ))}
        </div>
        <div className="usage-selects">
          <span className="usage-select">
            <Select
              icon={<SourceIcon />}
              aria-label="数据源筛选"
              value={sourceId}
              onChange={(e) => setSourceId(e.target.value)}
            >
              <option value="">全部来源</option>
              {options.sources
                .filter(
                  (source) =>
                    (!agent || source.agent === agent) &&
                    isAgentVisible(visibleAgents, source.agent),
                )
                .map((source) => (
                  <option key={source.sourceId} value={source.sourceId}>
                    {agentLabel(source.agent)} · {source.label}
                  </option>
                ))}
            </Select>
          </span>
          <span className="usage-select">
            <Select
              icon={<ModelIcon />}
              aria-label="模型筛选"
              value={model}
              onChange={(e) => setModel(e.target.value)}
            >
              <option value="">全部模型</option>
              {options.models.map((model) => (
                <option key={model}>{model}</option>
              ))}
            </Select>
          </span>
          <span className="usage-select">
            <Select
              icon={<ClockIcon />}
              aria-label="界面刷新周期"
              title="只刷新统计显示，不改变每 60 秒的日志采集周期"
              value={refreshSeconds}
              onChange={(e) => setRefreshSeconds(Number(e.target.value))}
            >
              {[0, 5, 10, 30, 60].map((seconds) => (
                <option key={seconds} value={seconds}>
                  {seconds ? `${seconds}s` : "暂停刷新"}
                </option>
              ))}
            </Select>
          </span>
          <span className="usage-select">
            <Select
              icon={<CalendarIcon />}
              aria-label="日期范围"
              value={period}
              onChange={(e) => {
                const next = e.target.value as Period;
                if (next === "custom" && !dateDraft.start && !dateDraft.end) {
                  const to = new Date();
                  const from = detailFilter.from
                    ? new Date(detailFilter.from)
                    : new Date(to.getFullYear(), to.getMonth(), to.getDate());
                  if (to.getTime() - from.getTime() < 60_000)
                    from.setTime(to.getTime() - 3_600_000);
                  const local = (d: Date) => {
                    const pad = (n: number) => String(n).padStart(2, "0");
                    return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
                  };
                  setDateDraft({ start: local(from), end: local(to) });
                }
                setDateError(null);
                setPeriod(next);
              }}
            >
              <option value="1h">最近 1 小时</option>
              <option value="3h">最近 3 小时</option>
              <option value="6h">最近 6 小时</option>
              <option value="today">当天</option>
              <option value="7d">最近 7 天</option>
              <option value="30d">最近 30 天</option>
              <option value="all">全部时间</option>
              <option value="custom">自定义时间</option>
            </Select>
          </span>
        </div>
      </div>
      {period === "custom" && (
        <div className="actions usage-dates">
          <label>
            开始{" "}
            <input
              type="datetime-local"
              value={dateDraft.start}
              onChange={(e) => setDateDraft({ ...dateDraft, start: e.target.value })}
            />
          </label>
          <label>
            结束{" "}
            <input
              type="datetime-local"
              value={dateDraft.end}
              min={dateDraft.start}
              onChange={(e) => setDateDraft({ ...dateDraft, end: e.target.value })}
            />
          </label>
          <button
            className="primary"
            onClick={() => {
              try {
                periodFilter("custom", new Date(), dateDraft.start, dateDraft.end);
                setDates(dateDraft);
                setDateError(null);
              } catch (e) {
                setDateError(e instanceof Error ? e.message : String(e));
              }
            }}
          >
            应用范围
          </button>
          {dateError && (
            <span role="alert" className="monitor-error">
              {dateError}
            </span>
          )}
        </div>
      )}
      <div className="usage-sync-line">
        <span className="muted small" aria-live="polite">
          {busy
            ? "正在同步…"
            : status.lastCompletedAt
              ? `同步于 ${new Date(status.lastCompletedAt).toLocaleTimeString("zh-CN")}`
              : "等待同步"}
        </span>
        <span className="muted small">{summary ? usageScopeText(detailFilter) : ""}</span>
      </div>
      {(error || status.lastError) && (
        <div className="card" role="alert">
          {error ?? status.lastError}
        </div>
      )}
      {summary === null && !error && <div className="card muted">正在读取统计…</div>}
      {summary && (
        <>
          <div className="usage-overview card">
            <div className="usage-hero-row">
              <div className="usage-total">
                <span className="usage-bolt" aria-hidden="true">
                  <BoltIcon />
                </span>
                <div>
                  <span className="muted">
                    本机 Tokens{" "}
                    <Hint label="用量统计说明">
                      按所选范围统计本地日志。缓存创建为新写入缓存的输入，缓存命中为读取缓存的输入；缺失数据保留未知。缓存命中率按命中
                      Token / 输入总量加权计算，费用为当前定价估算。各指标悬停可查看覆盖范围。
                    </Hint>
                  </span>
                  <div className="usage-total-number">
                    {formatTokenFull(summary.total.value)}{" "}
                    {summary.total.value !== null && BigInt(summary.total.value) >= 10000n && (
                      <small>≈ {formatUsageCompact(summary.total.value)}</small>
                    )}
                  </div>
                  {summary.total.knownEvents < summary.events && (
                    <small className="metric-partial">
                      部分数据 · {summary.total.knownEvents}/{summary.events} 条
                    </small>
                  )}
                </div>
              </div>
              <div className="usage-mini-stats">
                <div>
                  <span className="muted">已知请求数</span>
                  <strong>{formatTokenFull(summary.requestCount)}</strong>
                  {summary.requestCountKnownEvents < summary.events && (
                    <small className="metric-partial">部分记录</small>
                  )}
                </div>
              </div>
            </div>
            <div className="usage-metrics">
              <Metric
                label="新增输入"
                metric={summary.inputFresh}
                events={summary.events}
                tone="input"
                icon={<InputIcon />}
              />
              <Metric
                label="输出"
                metric={summary.outputTotal}
                events={summary.events}
                tone="output"
                icon={<OutputIcon />}
              />
              <Metric
                label="缓存创建"
                metric={summary.inputCacheWrite}
                events={summary.events}
                tone="cacheWrite"
                icon={<CacheWriteIcon />}
              />
              <Metric
                label="缓存命中"
                metric={summary.inputCacheRead}
                events={summary.events}
                tone="cacheRead"
                icon={<CacheReadIcon />}
              />
              <div className="usage-metric" data-tone="cost">
                <span className="usage-metric-head">
                  <PriceIcon />
                  成本估算
                </span>
                <strong className="usage-cost">{moneyText(dashboard?.currentCost ?? [])}</strong>
                {(dashboard?.pricedEvents ?? 0) > 0 &&
                  dashboard?.completeCostEvents !== summary.events && (
                    <small
                      className="metric-partial"
                      title={`完整估算 ${dashboard?.completeCostEvents ?? 0}/${summary.events} 条`}
                    >
                      部分估算
                    </small>
                  )}
              </div>
              <div className="usage-metric" data-tone="ratio">
                <span className="usage-metric-head">
                  <RatioIcon />
                  缓存命中率
                </span>
                <strong className="usage-cost">{percentage(summary.cacheReadRatio)}</strong>
                <progress
                  aria-label="缓存命中率"
                  max="1"
                  value={
                    summary.cacheReadRatio === null ? undefined : Number(summary.cacheReadRatio)
                  }
                />
                {summary.cacheRatioEligibleEvents < summary.events && (
                  <small className="metric-partial">部分数据</small>
                )}
              </div>
            </div>
          </div>
          {summary.events === 0 && (
            <div className="card">
              <strong>所选范围内没有计量记录。</strong>
              <p className="muted">调整时间范围或同步日志。</p>
            </div>
          )}
          {dashboard && (
            <UsageTrend series={dashboard.series} hourly={usesHourlyBuckets(detailFilter)} />
          )}
          {dashboard && (
            <UsageBreakdown
              dashboard={dashboard}
              filter={detailFilter}
              options={options}
              revision={revision}
              scopeKey={JSON.stringify([
                period,
                dates.start,
                dates.end,
                agent,
                sourceId,
                model,
                visibleAgents,
              ])}
              onPrice={(agent, model) => setPriceTarget({ agent, model })}
            />
          )}
          {summary.timeUnknownEvents > 0 && (
            <p className="muted small">{summary.timeUnknownEvents} 条记录时间未知</p>
          )}
        </>
      )}
      {priceTarget && (
        <ModelPricing
          initial={priceTarget}
          onClose={() => setPriceTarget(null)}
          onSaved={() => void refresh()}
        />
      )}
    </section>
  );
}
