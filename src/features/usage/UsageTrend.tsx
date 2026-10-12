import { formatMoneyAmount } from "../../lib/format";
import { SelectField as Select } from "../../components/Select";
import { useState, type MouseEvent as ReactMouseEvent } from "react";
import { smoothPath } from "./smoothPath";
import type { UsagePoint, UsageTokens } from "../../contracts/pricing";
import { formatTokenFull } from "../../lib/format";

/** 成本线颜色 = --danger（#ff7088）；SVG 展示属性不支持 var()，HTML 处也可直接用同值。 */
const COST_COLOR = "#ff7088";

const TOKEN_LINES: { key: keyof UsageTokens; label: string; color: string }[] = [
  { key: "inputFresh", label: "新增输入", color: "#56b4ff" },
  { key: "outputTotal", label: "输出", color: "#45edb8" },
  { key: "inputCacheRead", label: "缓存命中", color: "#bc8aff" },
  { key: "inputCacheWrite", label: "缓存创建", color: "#ffbe55" },
];

/** 左轴 Token 刻度：zh-CN 紧凑格式（万 / 亿）。 */
const COMPACT = new Intl.NumberFormat("zh-CN", { notation: "compact", maximumFractionDigits: 1 });
export function formatAxisTokens(value: number): string {
  return COMPACT.format(value);
}

export function currencySymbol(currency: string): string {
  if (currency === "CNY") return "¥";
  if (currency === "USD") return "$";
  return `${currency} `;
}

/** Presentation only; backend decimal amounts remain unchanged. */
export function formatCostFull(amount: string): string {
  return formatMoneyAmount(amount);
}

/** 右轴成本刻度：与 Token 轴相互独立。 */
export function formatCostTick(value: number, currency: string | undefined): string {
  if (!currency) return "";
  const abs = Math.abs(value);
  return `${currencySymbol(currency)}${abs >= 1 || abs === 0 ? value.toFixed(2) : value.toFixed(4)}`;
}

/** 悬浮提示标题：含年份的完整日期时间。 */
export function formatTooltipTime(at: string): string {
  return new Date(at).toLocaleString("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

/** 每次显示一种币种，其他币种可独立切换；缺少所选币种时保留断点。 */
function costOf(point: UsagePoint, currency: string | undefined): string | null {
  return currency ? (point.currentCost.find((c) => c.currency === currency)?.amount ?? null) : null;
}

export interface TooltipRow {
  key: string;
  label: string;
  color: string;
  text: string;
}

/** 悬浮提示行：未知分量直接缺行，绝不补零。 */
export function tooltipRows(point: UsagePoint, costCurrency: string | undefined): TooltipRow[] {
  const rows: TooltipRow[] = [];
  for (const line of TOKEN_LINES) {
    const value = point.tokens[line.key];
    if (value !== null)
      rows.push({
        key: line.key,
        label: line.label,
        color: line.color,
        text: `${formatTokenFull(value)} Token`,
      });
  }
  const cost = costOf(point, costCurrency);
  if (costCurrency && cost !== null)
    rows.push({
      key: "cost",
      label: `估算成本（${costCurrency}）`,
      color: COST_COLOR,
      text: `${currencySymbol(costCurrency)}${formatCostFull(cost)}`,
    });
  return rows;
}

// 几何：viewBox 1000×352，绘图区 x 64–940、y 24–300，横向虚线网格 5 条，无轴线。
const BOTTOM = 300;
const PLOT_H = BOTTOM - 24;
const X_LABEL_Y = 330;
const rowY = (row: number) => 24 + (row * PLOT_H) / 4;

export default function UsageTrend({ series, hourly }: { series: UsagePoint[]; hourly: boolean }) {
  const [hover, setHover] = useState<number | null>(null);
  const [selectedCurrency, setSelectedCurrency] = useState<string>();
  const currencies = [...new Set(series.flatMap((p) => p.currentCost.map((c) => c.currency)))];
  const costCurrency =
    selectedCurrency && currencies.includes(selectedCurrency) ? selectedCurrency : currencies[0];
  const tokenMax = Math.max(
    1e-9,
    ...series.flatMap((p) => TOKEN_LINES.map((l) => Number(p.tokens[l.key] ?? 0))),
  );
  const costValues = costCurrency
    ? series.map((p) => costOf(p, costCurrency)).filter((v): v is string => v !== null)
    : [];
  const costMax = Math.max(1e-9, ...costValues.map(Number));
  const x = (i: number) => (series.length === 1 ? 502 : 64 + (i * 876) / (series.length - 1));
  const yToken = (v: string) => BOTTOM - (Number(v) / tokenMax) * PLOT_H;
  const yCost = (v: string) => BOTTOM - (Number(v) / costMax) * PLOT_H;
  const dateLabel = (at: string) =>
    hourly
      ? new Date(at).toLocaleTimeString("zh-CN", {
          hour: "2-digit",
          minute: "2-digit",
          hour12: false,
        })
      : at.slice(5, 10);
  const onMove = (e: ReactMouseEvent<SVGSVGElement>) => {
    if (!series.length) return;
    const rect = e.currentTarget.getBoundingClientRect();
    if (!rect.width) return;
    const vx = ((e.clientX - rect.left) / rect.width) * 1000;
    let best = 0;
    for (let i = 1; i < series.length; i++)
      if (Math.abs(x(i) - vx) < Math.abs(x(best) - vx)) best = i;
    setHover(best);
  };
  /** 已知值分段：null 保留断点，分段各自成线/成面。 */
  const segmentsOf = (value: (p: UsagePoint) => string | null) => {
    const segments: { i: number; v: string }[][] = [];
    let current: { i: number; v: string }[] = [];
    series.forEach((p, i) => {
      const v = value(p);
      if (v === null) {
        if (current.length) segments.push(current);
        current = [];
      } else {
        current.push({ i, v });
      }
    });
    if (current.length) segments.push(current);
    return segments;
  };
  const hoverPoint = hover === null ? undefined : series[hover];
  const hoverRows = hoverPoint ? tooltipRows(hoverPoint, costCurrency) : [];
  const hoverX = hoverPoint ? x(series.indexOf(hoverPoint)) : 0;
  return (
    <div className="card usage-chart-card">
      <div className="cardhead">
        <div>
          <h2 title="左轴 Token，右轴估算成本；缺失数据保留断点">
            使用趋势 <span className="small muted">{hourly ? "按小时" : "按日"}</span>
          </h2>
        </div>
        {currencies.length > 1 && (
          <label className="small muted">
            成本币种
            <Select
              aria-label="趋势成本币种"
              value={costCurrency}
              onChange={(event) => setSelectedCurrency(event.target.value)}
            >
              {currencies.map((currency) => (
                <option key={currency} value={currency}>
                  {currency}
                </option>
              ))}
            </Select>
          </label>
        )}
      </div>
      {series.length ? (
        <>
          <div className="trend-plot">
            <svg
              className="usage-chart"
              viewBox="0 0 1000 352"
              role="img"
              aria-label="输入、输出、缓存 Token 与估算成本趋势"
              onMouseMove={onMove}
              onMouseLeave={() => setHover(null)}
            >
              <defs>
                {TOKEN_LINES.map((l) => (
                  <linearGradient id={`fill-${l.key}`} key={l.key} x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor={l.color} stopOpacity="0.18" />
                    <stop offset="100%" stopColor={l.color} stopOpacity="0" />
                  </linearGradient>
                ))}
              </defs>
              {[0, 1, 2, 3, 4].map((row) => (
                <g key={row}>
                  <line x1="64" x2="940" y1={rowY(row)} y2={rowY(row)} className="trend-grid" />
                  <text x="56" y={rowY(row) + 4} textAnchor="end">
                    {formatAxisTokens(Math.round(tokenMax * (1 - row / 4)))}
                  </text>
                  {costCurrency && (
                    <text x="948" y={rowY(row) + 4} textAnchor="start">
                      {formatCostTick(costMax * (1 - row / 4), costCurrency)}
                    </text>
                  )}
                </g>
              ))}
              {TOKEN_LINES.map((l) => {
                const segments = segmentsOf((p) => p.tokens[l.key]);
                return (
                  <g key={l.key} data-series={l.key}>
                    {segments.map((segment, i) => {
                      const path = smoothPath(segment.map((p) => ({ x: x(p.i), y: yToken(p.v) })));
                      return (
                        <g key={i}>
                          <path
                            d={`${path} L${x(segment[segment.length - 1].i)},${BOTTOM} L${x(
                              segment[0].i,
                            )},${BOTTOM} Z`}
                            fill={`url(#fill-${l.key})`}
                          />
                          <path d={path} stroke={l.color} className="trend-line" />
                        </g>
                      );
                    })}
                    {series.map((p, i) => {
                      const v = p.tokens[l.key];
                      return v === null ? null : (
                        <circle key={p.at} cx={x(i)} cy={yToken(v)} r="3" fill={l.color}>
                          <title>
                            {`${new Date(p.at).toLocaleString("zh-CN")} · ${l.label} · ${formatTokenFull(v)} Token`}
                          </title>
                        </circle>
                      );
                    })}
                  </g>
                );
              })}
              {costCurrency && (
                <g data-series="cost">
                  {segmentsOf((p) => costOf(p, costCurrency)).map((segment, i) => (
                    <path
                      key={i}
                      d={smoothPath(segment.map((p) => ({ x: x(p.i), y: yCost(p.v) })))}
                      stroke={COST_COLOR}
                      strokeWidth={2}
                      strokeDasharray="4 4"
                      fill="none"
                      className="trend-cost-line"
                    />
                  ))}
                  {series.map((p, i) => {
                    const v = costOf(p, costCurrency);
                    return v === null ? null : (
                      <circle key={p.at} cx={x(i)} cy={yCost(v)} r="2.5" fill={COST_COLOR}>
                        <title>
                          {`${new Date(p.at).toLocaleString("zh-CN")} · 估算成本（${costCurrency}） · ${currencySymbol(costCurrency)}${formatCostFull(v)}`}
                        </title>
                      </circle>
                    );
                  })}
                </g>
              )}
              {series.map(
                (p, i) =>
                  (series.length <= 10 ||
                    i % Math.ceil(series.length / 9) === 0 ||
                    i === series.length - 1) && (
                    <text key={p.at} x={x(i)} y={X_LABEL_Y} textAnchor="middle">
                      {dateLabel(p.at)}
                    </text>
                  ),
              )}
            </svg>
            {hoverPoint && (
              <div
                className="trend-tooltip"
                role="status"
                style={{
                  left: `min(max(${(hoverX / 10).toFixed(2)}%, 130px), calc(100% - 130px))`,
                }}
              >
                <strong>{formatTooltipTime(hoverPoint.at)}</strong>
                {hoverRows.length ? (
                  hoverRows.map((row) => (
                    <div key={row.key}>
                      <i style={{ background: row.color }} />
                      <span>{row.label}</span>
                      <b>{row.text}</b>
                    </div>
                  ))
                ) : (
                  <p className="muted small">该时段无数值</p>
                )}
              </div>
            )}
          </div>
          <div className="trend-legend">
            {TOKEN_LINES.map((l) => (
              <span key={l.key}>
                <i style={{ background: l.color }} />
                {l.label}
              </span>
            ))}
            {costCurrency && (
              <span>
                <i className="trend-legend-dash" style={{ borderColor: COST_COLOR }} />
                估算成本（{costCurrency}）
              </span>
            )}
          </div>
          <details className="trend-data">
            <summary>查看趋势数据表</summary>
            <div className="usage-table-scroll">
              <table>
                <thead>
                  <tr>
                    <th>时间</th>
                    {TOKEN_LINES.map((l) => (
                      <th key={l.key}>{l.label}</th>
                    ))}
                    <th>{costCurrency ? `估算成本（${costCurrency}）` : "估算成本"}</th>
                  </tr>
                </thead>
                <tbody>
                  {series.map((p) => {
                    const cost = costOf(p, costCurrency);
                    return (
                      <tr key={p.at}>
                        <td>{dateLabel(p.at)}</td>
                        {TOKEN_LINES.map((l) => (
                          <td key={l.key}>{p.tokens[l.key] ?? "未提供"}</td>
                        ))}
                        <td>
                          {cost !== null && costCurrency
                            ? `${currencySymbol(costCurrency)}${formatCostFull(cost)}`
                            : "未提供"}
                        </td>
                      </tr>
                    );
                  })}
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
