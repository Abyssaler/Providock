import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type {
  SourceScanReport,
  UsageFilter,
  UsageSeriesPoint,
  UsageSummary,
} from "../../contracts/usage";
import { formatTokenCompact, formatTokenFull } from "../../lib/format";

type Period = "today" | "7d" | "30d";

function periodFilter(p: Period): UsageFilter {
  const now = new Date();
  const from = new Date(now);
  if (p === "today") {
    from.setUTCHours(0, 0, 0, 0);
  } else {
    from.setUTCDate(now.getUTCDate() - (p === "7d" ? 7 : 30));
  }
  return { from: from.toISOString(), to: now.toISOString() };
}

export default function UsagePage() {
  const [period, setPeriod] = useState<Period>("7d");
  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [series, setSeries] = useState<UsageSeriesPoint[]>([]);
  const [syncing, setSyncing] = useState(false);
  const [syncReports, setSyncReports] = useState<SourceScanReport[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback((p: Period) => {
    const filter = periodFilter(p);
    invoke<UsageSummary>("usage_summary", { filter })
      .then(setSummary)
      .catch((e) => setError(String(e)));
    invoke<UsageSeriesPoint[]>("usage_series", { filter })
      .then(setSeries)
      .catch(() => setSeries([]));
  }, []);

  useEffect(() => refresh(period), [period, refresh]);

  const onSync = () => {
    setSyncing(true);
    setError(null);
    invoke<SourceScanReport[]>("sync_start")
      .then((reports) => {
        setSyncReports(reports);
        refresh(period);
      })
      .catch((e) => setError(String(e)))
      .finally(() => setSyncing(false));
  };

  const empty = summary !== null && summary.events === 0;
  const maxSeries = series.reduce((m, [, v]) => Math.max(m, v ? Number(v) : 0), 0);

  return (
    <section aria-label="用量页面">
      <div className="page-head">
        <div>
          <h1>每一份用量，都有来源</h1>
          <p className="muted">查看本机 Agent 消耗，追踪输入、输出与缓存。</p>
        </div>
        <div className="actions">
          <button className={period === "today" ? "active" : ""} onClick={() => setPeriod("today")}>
            今天
          </button>
          <button className={period === "7d" ? "active" : ""} onClick={() => setPeriod("7d")}>
            最近 7 天
          </button>
          <button className={period === "30d" ? "active" : ""} onClick={() => setPeriod("30d")}>
            最近 30 天
          </button>
          <button className="primary" disabled={syncing} onClick={onSync}>
            {syncing ? "同步中…" : "同步记录"}
          </button>
        </div>
      </div>

      <p className="muted small">
        数据域：本地日志（网关与导入域单独统计，不混入）· 趋势按 UTC 日聚合
      </p>

      {error && (
        <div className="card" role="alert">
          <p className="muted">查询失败：{error}</p>
        </div>
      )}

      {syncReports !== null && (
        <div className="card">
          {syncReports.length === 0 ? (
            <p className="muted">
              未发现 Kimi Code / Codex 的默认目录。可在「数据源」页了解发现规则；其余六个 Agent
              的适配器在后续任务接入。
            </p>
          ) : (
            <ul className="report-list">
              {syncReports.map((r) => (
                <li key={r.sourceId}>
                  <strong>{r.agent}</strong>：扫描 {r.filesScanned}/{r.filesSeen} 个文件，新增{" "}
                  {r.eventsInserted} 条，已存在 {r.eventsAlreadyPresent} 条（重扫不重复计数）
                  {r.filesDeferred > 0 && `，${r.filesDeferred} 个文件待父时间线`}
                  {r.diagnostics.length > 0 && (
                    <span className="muted small"> · {r.diagnostics.length} 条诊断</span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {summary === null && !error && (
        <div className="card">
          <p className="muted">读取中…</p>
        </div>
      )}

      {empty && (
        <div className="card">
          <p>
            <strong>所选范围内没有计量记录。</strong>
          </p>
          <p className="muted small">
            点击「同步记录」扫描本机 Kimi Code / Codex 日志。未知值显示为 —，不会当作 0。
          </p>
        </div>
      )}

      {summary !== null && summary.events > 0 && (
        <>
          <div className="hero">
            <div className="stats">
              <div className="stat">
                <div className="muted small">总 Token</div>
                <div className="num big">{formatTokenCompact(summary.total.value)}</div>
                <div className="small muted">
                  {summary.total.value === null
                    ? "全部未知"
                    : `${formatTokenFull(summary.total.value)} · 覆盖 ${summary.total.knownEvents}/${summary.events}`}
                </div>
              </div>
              <div className="stat">
                <div className="muted small">输入</div>
                <div className="num">{formatTokenCompact(summary.inputTotal.value)}</div>
                <div className="small muted">
                  含缓存读写 · 覆盖 {summary.inputTotal.knownEvents}/{summary.events}
                </div>
              </div>
              <div className="stat">
                <div className="muted small">输出</div>
                <div className="num">{formatTokenCompact(summary.outputTotal.value)}</div>
                <div className="small muted">reasoning 不重复计入</div>
              </div>
              <div className="stat">
                <div className="muted small">读缓存 Token 占比</div>
                <div className="num">
                  {summary.cacheReadRatio === null
                    ? "—"
                    : `${(Number(summary.cacheReadRatio) * 100).toFixed(1)}%`}
                </div>
                <div className="small muted">
                  参与 {summary.cacheRatioEligibleEvents}/{summary.events} 条 · 加权分母为已知输入
                </div>
              </div>
              <div className="stat">
                <div className="muted small">计量记录</div>
                <div className="num">{summary.events}</div>
                <div className="small muted">
                  已知请求数{" "}
                  {summary.requestCount === null ? "—" : formatTokenFull(summary.requestCount)}
                  {summary.timeUnknownEvents > 0 && ` · ${summary.timeUnknownEvents} 条时间未知`}
                </div>
              </div>
            </div>
          </div>

          <div className="lower">
            <div className="card">
              <div className="cardhead">
                <h2>Token 趋势</h2>
                <span className="badge">UTC · 每日</span>
              </div>
              {series.length === 0 ? (
                <p className="muted small">范围内没有带时间戳的记录。</p>
              ) : (
                <div className="chart" role="img" aria-label="每日 Token 趋势">
                  {series.map(([day, v]) => (
                    <div className="barcol" key={day} title={`${day} · ${v ?? "未知"}`}>
                      <small>{v === null ? "—" : formatTokenCompact(v)}</small>
                      <div
                        className="bar"
                        style={{
                          height: v && maxSeries > 0 ? `${(Number(v) / maxSeries) * 100}%` : "2%",
                        }}
                      />
                      <small>{day.slice(5)}</small>
                    </div>
                  ))}
                </div>
              )}
            </div>
            <div className="card">
              <div className="cardhead">
                <h2>Agent 分布</h2>
              </div>
              {summary.byAgent.map((a) => (
                <div className="rankline" key={a.agent}>
                  <span>{a.agent}</span>
                  <strong className="num">{formatTokenFull(a.total.value)}</strong>
                  <span className="muted small">{a.events} 条</span>
                </div>
              ))}
            </div>
          </div>

          {(summary.costReported.length > 0 || summary.costEstimated.length > 0) && (
            <div className="card">
              <div className="cardhead">
                <h2>费用</h2>
                <span className="small muted">已报告与估算分列 · 币种不合并</span>
              </div>
              {summary.costReported.length > 0 && (
                <p>
                  已报告：
                  {summary.costReported.map((c) => (
                    <strong key={c.currency} className="num" style={{ marginRight: 12 }}>
                      {c.amount} {c.currency}
                    </strong>
                  ))}
                </p>
              )}
              {summary.costEstimated.length > 0 && (
                <p className="muted">
                  估算：
                  {summary.costEstimated.map((c) => (
                    <span key={c.currency} style={{ marginRight: 12 }}>
                      {c.amount} {c.currency}
                    </span>
                  ))}
                </p>
              )}
            </div>
          )}
        </>
      )}
    </section>
  );
}
