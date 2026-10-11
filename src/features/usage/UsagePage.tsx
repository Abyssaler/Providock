import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  MetricTotal,
  MonitoringSettings,
  SourceScanReport,
  SyncStatus,
  UsageOptions,
  UsageSeriesPoint,
  UsageSummary,
} from "../../contracts/usage";
import { formatTokenFull } from "../../lib/format";
import {
  AGENTS,
  agentLabel,
  formatUsageCompact,
  percentage,
  periodFilter,
  type Period,
} from "./usageDisplay";
import MonitoringControls from "./MonitoringControls";
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
}: {
  label: string;
  metric: MetricTotal;
  events: number;
  hint?: string;
}) {
  return (
    <div className="usage-metric">
      <span className="muted">{label}</span>
      <strong>{formatUsageCompact(metric.value)}</strong>
      <small className="muted" title={hint}>
        {hint ?? `覆盖 ${metric.knownEvents}/${events} 条记录`}
      </small>
    </div>
  );
}
function Trend({ series }: { series: UsageSeriesPoint[] }) {
  const known = series.map(([, value]) => (value === null ? null : Number(value)));
  const max = Math.max(1, ...known.map((value) => value ?? 0));
  const points = known.map((value, index) => ({
    x: series.length === 1 ? 480 : 40 + (index * 880) / (series.length - 1),
    y: value === null ? null : 215 - (value / max) * 175,
  }));
  // 未知值断开曲线，不能画成零消费。
  let path = "";
  let connected = false;
  for (const point of points) {
    if (point.y === null) {
      connected = false;
      continue;
    }
    path += `${connected ? "L" : "M"}${point.x},${point.y} `;
    connected = true;
  }
  return (
    <div className="usage-trend">
      <svg viewBox="0 0 960 255" role="img" aria-label="每日 Token 使用趋势">
        {[0, 1, 2, 3].map((row) => (
          <g key={row}>
            <line
              x1="40"
              x2="920"
              y1={40 + (row * 175) / 3}
              y2={40 + (row * 175) / 3}
              className="trend-grid"
            />
            <text x="40" y={30 + (row * 175) / 3}>
              {formatUsageCompact(String(Math.round(max * (1 - row / 3))))}
            </text>
          </g>
        ))}
        <path d={path} className="trend-line" />
        {points.map((point, i) => (
          <g key={series[i][0]}>
            {point.y !== null && (
              <circle cx={point.x} cy={point.y} r="4" className="trend-point">
                <title>
                  {series[i][0]} · {formatTokenFull(series[i][1])} Tokens
                </title>
              </circle>
            )}
            {(series.length <= 12 ||
              i % Math.ceil(series.length / 10) === 0 ||
              i === series.length - 1) && (
              <text x={point.x} y="245" textAnchor="middle">
                {series[i][0].slice(5)}
              </text>
            )}
          </g>
        ))}
      </svg>
      {series.some(([, value]) => value === null) && (
        <p className="muted small">部分日期未提供总 Token，曲线保留缺口。</p>
      )}
    </div>
  );
}

export default function UsagePage() {
  const [monitoring, setMonitoring] = useState<MonitoringSettings | null>(null);
  const [period, setPeriod] = useState<Period>("today");
  const [dates, setDates] = useState({ start: "", end: "" });
  const [agent, setAgent] = useState("");
  const [sourceId, setSourceId] = useState("");
  const [model, setModel] = useState("");
  const [refreshSeconds, setRefreshSeconds] = useState(30);
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [series, setSeries] = useState<UsageSeriesPoint[]>([]);
  const [options, setOptions] = useState<UsageOptions>({ sources: [], models: [] });
  const [status, setStatus] = useState<SyncStatus>(EMPTY_STATUS);
  const [manualBusy, setManualBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const request = useRef(0);

  const refresh = useCallback(async () => {
    const version = ++request.current;
    try {
      const filter = {
        ...periodFilter(period, new Date(), dates.start, dates.end),
        agent: agent || undefined,
        sourceId: sourceId || undefined,
        model: model || undefined,
      };
      const [nextSummary, nextSeries, nextOptions] = await Promise.all([
        invoke<UsageSummary>("usage_summary", { filter }),
        invoke<UsageSeriesPoint[]>("usage_series", { filter }),
        invoke<UsageOptions>("usage_options"),
      ]);
      if (version !== request.current) return;
      setSummary(nextSummary);
      setSeries(nextSeries);
      setOptions(nextOptions);
      setError(null);
    } catch (e) {
      if (version === request.current) setError(String(e));
    }
  }, [period, dates.start, dates.end, agent, sourceId, model]);

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
  const costs = summary?.costReported ?? [];
  return (
    <section className="usage-page" aria-label="用量页面">
      <div className="usage-heading">
        <div>
          <h1>使用统计</h1>
          <p className="muted">查看本机 AI Agent 的使用情况与成本统计</p>
        </div>
        <details className="usage-monitor">
          <summary>数据采集与调度</summary>
          <MonitoringControls onChange={setMonitoring} />
        </details>
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
            全部
          </button>
          {AGENTS.map(([id, label]) => (
            <button
              key={id}
              className={agent === id ? "active" : ""}
              aria-pressed={agent === id}
              onClick={() => {
                setAgent(id);
                setSourceId("");
              }}
            >
              {label}
            </button>
          ))}
        </div>
        <div className="usage-selects">
          <select
            aria-label="数据源筛选"
            value={sourceId}
            onChange={(e) => setSourceId(e.target.value)}
          >
            <option value="">全部来源</option>
            {options.sources
              .filter((source) => !agent || source.agent === agent)
              .map((source) => (
                <option key={source.sourceId} value={source.sourceId}>
                  {agentLabel(source.agent)} · {source.label}
                </option>
              ))}
          </select>
          <select aria-label="模型筛选" value={model} onChange={(e) => setModel(e.target.value)}>
            <option value="">全部模型</option>
            {options.models.map((model) => (
              <option key={model}>{model}</option>
            ))}
          </select>
          <select
            aria-label="界面刷新周期"
            title="只刷新统计显示，不改变每 60 秒的日志采集周期"
            value={refreshSeconds}
            onChange={(e) => setRefreshSeconds(Number(e.target.value))}
          >
            {[0, 5, 10, 30, 60].map((seconds) => (
              <option key={seconds} value={seconds}>
                {seconds ? `显示刷新 · ${seconds}s` : "显示刷新 · 关闭"}
              </option>
            ))}
          </select>
          <select
            aria-label="日期范围"
            value={period}
            onChange={(e) => setPeriod(e.target.value as Period)}
          >
            <option value="today">当天</option>
            <option value="7d">最近 7 天</option>
            <option value="30d">最近 30 天</option>
            <option value="all">全部时间</option>
            <option value="custom">自定义日期</option>
          </select>
        </div>
      </div>
      {period === "custom" && (
        <div className="actions usage-dates">
          <label>
            开始{" "}
            <input
              type="date"
              value={dates.start}
              onChange={(e) => setDates({ ...dates, start: e.target.value })}
            />
          </label>
          <label>
            结束{" "}
            <input
              type="date"
              value={dates.end}
              min={dates.start}
              onChange={(e) => setDates({ ...dates, end: e.target.value })}
            />
          </label>
        </div>
      )}
      <div className="usage-sync-line">
        <span className="muted small" aria-live="polite">
          {busy
            ? "正在同步本机日志…"
            : status.lastCompletedAt
              ? `最近同步 ${new Date(status.lastCompletedAt).toLocaleString("zh-CN")}`
              : "等待首次同步"}{" "}
          · 本地日志账本
          {monitoring &&
            ` · ${monitoring.agentAutoSyncEnabled ? "每 60 秒自动同步" : "自动同步已关闭"}`}
        </span>
        <button disabled={busy} onClick={() => void onSync()}>
          {busy ? "同步中…" : "立即同步"}
        </button>
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
                  ϟ
                </span>
                <div>
                  <span className="muted">真实消耗 Tokens</span>
                  <div className="usage-total-number">
                    {formatTokenFull(summary.total.value)}{" "}
                    <small>≈ {formatUsageCompact(summary.total.value)}</small>
                  </div>
                  <small className="muted">
                    本机日志已知小计 · 覆盖 {summary.total.knownEvents}/{summary.events} 条记录
                  </small>
                </div>
              </div>
              <div className="usage-mini-stats">
                <div>
                  <span className="muted">已知请求数</span>
                  <strong>{formatTokenFull(summary.requestCount)}</strong>
                  <small className="muted">
                    覆盖 {summary.requestCountKnownEvents}/{summary.events}
                  </small>
                </div>
                <div>
                  <span className="muted">已报告成本</span>
                  <strong className="usage-cost">
                    {costs.length
                      ? costs.map((cost) => `${cost.amount} ${cost.currency}`).join(" / ")
                      : "未提供"}
                  </strong>
                </div>
              </div>
            </div>
            <div className="usage-metrics">
              <Metric label="新增输入" metric={summary.inputFresh} events={summary.events} />
              <Metric label="输出" metric={summary.outputTotal} events={summary.events} />
              <Metric label="缓存创建" metric={summary.inputCacheWrite} events={summary.events} />
              <Metric label="缓存命中" metric={summary.inputCacheRead} events={summary.events} />
              <div className="usage-metric">
                <span className="muted">缓存命中率</span>
                <strong className="usage-cost">{percentage(summary.cacheReadRatio)}</strong>
                <progress
                  aria-label="缓存命中率"
                  max="1"
                  value={
                    summary.cacheReadRatio === null ? undefined : Number(summary.cacheReadRatio)
                  }
                />
                <small className="muted">
                  加权输入分母 · 覆盖 {summary.cacheRatioEligibleEvents}/{summary.events}
                </small>
              </div>
            </div>
          </div>
          {summary.events === 0 && (
            <div className="card">
              <strong>所选范围内没有计量记录。</strong>
              <p className="muted">
                可切换日期或筛选条件。自动同步开启时每 60 秒读取八种 Agent
                的已发现日志，也可点击「立即同步」。
              </p>
            </div>
          )}
          <div className="card">
            <div className="cardhead">
              <h2>使用趋势</h2>
              <span className="muted small">Token · 按 UTC 日聚合</span>
            </div>
            {series.length ? (
              <Trend series={series} />
            ) : (
              <p className="muted">该范围没有带时间戳的计量记录。</p>
            )}
          </div>
          <div className="usage-bottom">
            <div className="card">
              <div className="cardhead">
                <h2>Agent 分布</h2>
                <span className="muted small">已知总 Token</span>
              </div>
              {summary.byAgent.map((item) => (
                <div className="rankline" key={item.agent}>
                  <span>{agentLabel(item.agent)}</span>
                  <strong>{formatTokenFull(item.total.value)}</strong>
                  <span className="muted small">{item.events} 条</span>
                </div>
              ))}
            </div>
            <div className="card">
              <h2>统计范围</h2>
              <p className="muted">
                本机记录不等于平台全部消费。未知值显示为 —；缓存读取、写入和 reasoning
                按源协议计量。
              </p>
              <p className="muted">
                输入总量 {formatTokenFull(summary.inputTotal.value)} · 计量记录 {summary.events} 条
              </p>
              {summary.costEstimated.map((cost) => (
                <p key={cost.currency}>
                  估算成本：{cost.amount} {cost.currency}
                </p>
              ))}
              {summary.timeUnknownEvents > 0 && (
                <p className="muted">{summary.timeUnknownEvents} 条记录缺少发生时间。</p>
              )}
            </div>
          </div>
        </>
      )}
      <details className="card usage-reports">
        <summary>最近采集结果 · {status.reports.length} 个数据源</summary>
        {status.reports.length ? (
          status.reports.map((report) => (
            <p key={report.sourceId}>
              <strong>{agentLabel(report.agent)}</strong>：扫描 {report.filesScanned}/
              {report.filesSeen} 个文件，新增 {report.eventsInserted} 条，未变化{" "}
              {report.filesUnchanged} 个
              {report.filesDeferred > 0 && `，待处理 ${report.filesDeferred} 个`}
              {report.diagnostics.length > 0 && `，${report.diagnostics.length} 条诊断`}
            </p>
          ))
        ) : (
          <p className="muted">尚无采集结果；未发现数据源时可在「数据源」页查看发现规则。</p>
        )}
      </details>
    </section>
  );
}
