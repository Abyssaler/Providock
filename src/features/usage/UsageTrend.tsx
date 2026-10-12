import { useState } from "react";
import { smoothPath } from "./smoothPath";
import type { UsagePoint, UsageTokens } from "../../contracts/pricing";
import { formatTokenFull } from "../../lib/format";
import { formatUsageCompact } from "./usageDisplay";

const LINES: { key: keyof UsageTokens; label: string; color: string }[] = [
  { key: "inputFresh", label: "新增输入", color: "#56b4ff" },
  { key: "outputTotal", label: "输出", color: "#45edb8" },
  { key: "inputCacheRead", label: "缓存命中", color: "#bc8aff" },
  { key: "inputCacheWrite", label: "缓存创建", color: "#ffbe55" },
];
export default function UsageTrend({ series, hourly }: { series: UsagePoint[]; hourly: boolean }) {
  const [mode, setMode] = useState("tokens");
  const [hidden, setHidden] = useState<string[]>([]);
  const currencies = [...new Set(series.flatMap((p) => p.currentCost.map((c) => c.currency)))];
  const costMode = mode !== "tokens";
  const currency = currencies.includes(mode) ? mode : currencies[0];
  const lines = costMode
    ? [{ key: "cost", label: `当前定价估算 · ${currency ?? "未定价"}`, color: "#ef5979" }]
    : LINES;
  const value = (point: UsagePoint, key: string) =>
    key === "cost"
      ? (point.currentCost.find((c) => c.currency === currency)?.amount ?? null)
      : point.tokens[key as keyof UsageTokens];
  const max = Math.max(
    1e-9,
    ...series.flatMap((p) =>
      lines.filter((l) => !hidden.includes(l.key)).map((l) => Number(value(p, l.key) ?? 0)),
    ),
  );
  const x = (i: number) => (series.length === 1 ? 500 : 85 + (i * 865) / (series.length - 1));
  const y = (v: string) => 255 - (Number(v) / max) * 205;
  const dateLabel = (at: string) =>
    hourly
      ? new Date(at).toLocaleTimeString("zh-CN", {
          hour: "2-digit",
          minute: "2-digit",
          hour12: false,
        })
      : at.slice(5, 10);
  return (
    <div className="card usage-chart-card">
      <div className="cardhead">
        <div>
          <h2>使用趋势</h2>
          <p className="small muted">
            {hourly ? "按小时聚合 · 时间按本机时区显示" : "按 UTC 日聚合"} ·
            已知小计，缺失值保留断点
          </p>
        </div>
        <div className="usage-segmented" aria-label="趋势指标">
          <button className={!costMode ? "active" : ""} onClick={() => setMode("tokens")}>
            Token
          </button>
          {currencies.map((c) => (
            <button
              key={c}
              className={costMode && currency === c ? "active" : ""}
              onClick={() => setMode(c)}
            >
              {c} 成本
            </button>
          ))}
        </div>
      </div>
      {series.length ? (
        <>
          <svg
            className="usage-chart"
            viewBox="0 0 1000 305"
            role="img"
            aria-label={costMode ? "当前定价估算成本趋势" : "输入、输出和缓存 Token 趋势"}
          >
            <defs>
              {lines.map((l) => (
                <linearGradient id={`fill-${l.key}`} key={l.key} x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor={l.color} stopOpacity="0.2" />
                  <stop offset="100%" stopColor={l.color} stopOpacity="0" />
                </linearGradient>
              ))}
            </defs>
            {[0, 1, 2, 3, 4].map((row) => (
              <g key={row}>
                <line
                  x1="85"
                  x2="950"
                  y1={50 + (row * 205) / 4}
                  y2={50 + (row * 205) / 4}
                  className="trend-grid"
                />
                <text x="70" y={55 + (row * 205) / 4} textAnchor="end">
                  {costMode
                    ? (max * (1 - row / 4)).toFixed(3)
                    : formatUsageCompact(String(Math.round(max * (1 - row / 4))))}
                </text>
              </g>
            ))}
            {lines
              .filter((l) => !hidden.includes(l.key))
              .map((l) => {
                const segments: { i: number; v: string }[][] = [];
                let current: { i: number; v: string }[] = [];
                series.forEach((p, i) => {
                  const v = value(p, l.key);
                  if (v === null) {
                    if (current.length) segments.push(current);
                    current = [];
                  } else {
                    current.push({ i, v });
                  }
                });
                if (current.length) segments.push(current);
                return (
                  <g key={l.key} data-series={l.key}>
                    {segments.map((segment, i) => {
                      const path = smoothPath(segment.map((p) => ({ x: x(p.i), y: y(p.v) })));
                      return (
                        <g key={i}>
                          <path
                            d={`${path} L${x(segment[segment.length - 1].i)},255 L${x(segment[0].i)},255 Z`}
                            fill={`url(#fill-${l.key})`}
                          />
                          <path d={path} stroke={l.color} className="trend-line" />
                        </g>
                      );
                    })}
                    {series.map((p, i) => {
                      const v = value(p, l.key);
                      return v === null ? null : (
                        <circle key={p.at} cx={x(i)} cy={y(v)} r="3" fill={l.color}>
                          <title>
                            {new Date(p.at).toLocaleString("zh-CN")} · {l.label} ·{" "}
                            {costMode ? `${v} ${currency}` : `${formatTokenFull(v)} Token`}
                          </title>
                        </circle>
                      );
                    })}
                  </g>
                );
              })}
            {series.map(
              (p, i) =>
                (series.length <= 10 ||
                  i % Math.ceil(series.length / 9) === 0 ||
                  i === series.length - 1) && (
                  <text key={p.at} x={x(i)} y="285" textAnchor="middle">
                    {dateLabel(p.at)}
                  </text>
                ),
            )}
          </svg>
          <div className="trend-legend">
            {lines.map((l) => (
              <button
                key={l.key}
                aria-pressed={!hidden.includes(l.key)}
                className={hidden.includes(l.key) ? "muted" : ""}
                onClick={() =>
                  setHidden((prev) =>
                    prev.includes(l.key) ? prev.filter((key) => key !== l.key) : [...prev, l.key],
                  )
                }
              >
                <i style={{ background: l.color }} />
                {l.label}
              </button>
            ))}
          </div>
          <details className="trend-data">
            <summary>查看趋势数据表</summary>
            <div className="usage-table-scroll">
              <table>
                <thead>
                  <tr>
                    <th>时间</th>
                    {lines.map((l) => (
                      <th key={l.key}>{l.label}</th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {series.map((p) => (
                    <tr key={p.at}>
                      <td>{dateLabel(p.at)}</td>
                      {lines.map((l) => (
                        <td key={l.key}>{value(p, l.key) ?? "未提供"}</td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </details>
        </>
      ) : (
        <div className="usage-empty">
          <strong>暂无趋势数据</strong>
          <p>所选范围内没有带时间戳的计量记录。</p>
        </div>
      )}
    </div>
  );
}
