import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { UsageDashboard, UsageEventPage, UsageGroup } from "../../contracts/pricing";
import type { MoneyTotal, UsageFilter, UsageOptions } from "../../contracts/usage";
import { formatTokenFull } from "../../lib/format";
import { agentLabel } from "./usageDisplay";
export function moneyText(costs: MoneyTotal[]) {
  return costs.length ? costs.map((c) => `${c.amount} ${c.currency}`).join(" / ") : "未定价";
}
export function GroupTable({
  groups,
  models,
  onPrice,
}: {
  groups: UsageGroup[];
  models: boolean;
  onPrice: (agent: string, model: string) => void;
}) {
  const max = Number(groups[0]?.tokens.total ?? 0);
  return (
    <div className="usage-table-scroll">
      <table className="usage-data-table">
        <thead>
          <tr>
            <th>{models ? "模型 / Agent" : "Agent"}</th>
            <th>计量记录</th>
            <th>输入总量</th>
            <th>输出</th>
            <th>缓存读 / 写</th>
            <th>总 Token</th>
            <th>当前定价估算</th>
            {models && <th>定价</th>}
          </tr>
        </thead>
        <tbody>
          {groups.map((g) => (
            <tr key={`${g.agent}:${g.name}`}>
              <td>
                <strong>{models ? g.name : agentLabel(g.name)}</strong>
                {g.agent && <small>{agentLabel(g.agent)}</small>}
                <div className="usage-rankbar">
                  <i style={{ width: `${max ? (Number(g.tokens.total ?? 0) / max) * 100 : 0}%` }} />
                </div>
              </td>
              <td>{g.events.toLocaleString()}</td>
              <td>{formatTokenFull(g.tokens.inputTotal)}</td>
              <td>{formatTokenFull(g.tokens.outputTotal)}</td>
              <td>
                {formatTokenFull(g.tokens.inputCacheRead)}
                <small>写 {formatTokenFull(g.tokens.inputCacheWrite)}</small>
              </td>
              <td className="usage-cell-total">{formatTokenFull(g.tokens.total)}</td>
              <td>
                <span className="usage-cost">{moneyText(g.currentCost)}</span>
                <small>
                  完整估算 {g.completeCostEvents}/{g.events} 条
                </small>
              </td>
              {models && (
                <td>
                  <button
                    disabled={g.name === "未提供模型"}
                    onClick={() => onPrice(g.agent!, g.name)}
                  >
                    设置价格
                  </button>
                </td>
              )}
            </tr>
          ))}
        </tbody>
      </table>
      {!groups.length && <div className="usage-empty">当前范围没有统计数据。</div>}
    </div>
  );
}
export default function UsageBreakdown({
  dashboard,
  filter,
  options,
  revision,
  scopeKey,
  onPrice,
}: {
  dashboard: UsageDashboard;
  filter: UsageFilter;
  options: UsageOptions;
  revision: number;
  scopeKey: string;
  onPrice: (agent: string, model: string) => void;
}) {
  const [tab, setTab] = useState("models");
  const [page, setPage] = useState(1);
  const [data, setData] = useState<UsageEventPage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const token = useRef(0);
  const filterKey = JSON.stringify(filter);
  useEffect(() => {
    setPage(1);
    setData(null);
  }, [scopeKey]);
  useEffect(() => {
    if (tab !== "events") return;
    const id = ++token.current;
    setLoading(true);
    setError(null);
    void invoke<UsageEventPage>("usage_events", {
      filter: JSON.parse(filterKey),
      page,
      pageSize: 20,
    })
      .then((result) => {
        if (id === token.current) setData(result);
      })
      .catch((e) => {
        if (id === token.current) setError(String(e));
      })
      .finally(() => {
        if (id === token.current) setLoading(false);
      });
    return () => {
      token.current++;
    };
  }, [filterKey, page, tab, revision]);
  const maxPage = data ? Math.max(1, Math.ceil(data.total / data.pageSize)) : 1;
  return (
    <div className="usage-detail-section">
      <div className="usage-detail-head">
        <div className="usage-segmented" role="tablist" aria-label="用量统计维度">
          {[
            ["events", "计量明细"],
            ["agents", "Agent 统计"],
            ["models", "模型统计"],
          ].map(([key, label]) => (
            <button
              key={key}
              role="tab"
              id={`usage-tab-${key}`}
              aria-selected={tab === key}
              className={tab === key ? "active" : ""}
              onClick={() => setTab(key)}
            >
              {label}
            </button>
          ))}
        </div>
        <span className="small muted">与上方筛选一致 · Token 为已知小计</span>
      </div>
      <div className="card usage-detail-card" role="tabpanel" aria-labelledby={`usage-tab-${tab}`}>
        {tab !== "events" ? (
          <GroupTable
            groups={tab === "models" ? dashboard.models : dashboard.agents}
            models={tab === "models"}
            onPrice={onPrice}
          />
        ) : (
          <>
            {error && (
              <p role="alert" className="monitor-error">
                {error}
              </p>
            )}
            {loading && (
              <p className="muted small" role="status">
                读取计量明细…
              </p>
            )}
            {data && (
              <>
                <div className="usage-table-scroll">
                  <table className="usage-data-table">
                    <thead>
                      <tr>
                        <th>时间 / Agent</th>
                        <th>模型 / 来源</th>
                        <th>新增输入</th>
                        <th>输出</th>
                        <th>缓存读 / 写</th>
                        <th>总 Token</th>
                        <th>费用</th>
                        <th>计量粒度</th>
                      </tr>
                    </thead>
                    <tbody>
                      {data.rows.map((row) => (
                        <tr key={row.eventId}>
                          <td>
                            {row.occurredAt
                              ? new Date(row.occurredAt).toLocaleString("zh-CN", {
                                  month: "2-digit",
                                  day: "2-digit",
                                  hour: "2-digit",
                                  minute: "2-digit",
                                  hour12: false,
                                })
                              : "时间未知"}
                            <small>{agentLabel(row.agent)}</small>
                          </td>
                          <td>
                            <strong>{row.model ?? "模型未提供"}</strong>
                            <small>
                              {options.sources.find((s) => s.sourceId === row.sourceId)?.label ??
                                "本地日志"}
                            </small>
                          </td>
                          <td>{formatTokenFull(row.tokens.inputFresh)}</td>
                          <td>{formatTokenFull(row.tokens.outputTotal)}</td>
                          <td>
                            {formatTokenFull(row.tokens.inputCacheRead)}
                            <small>写 {formatTokenFull(row.tokens.inputCacheWrite)}</small>
                          </td>
                          <td>{formatTokenFull(row.tokens.total)}</td>
                          <td>
                            {row.costReported && (
                              <>
                                <span>
                                  {row.costReported.amount} {row.costReported.currency}
                                </span>
                                <small>源已报告</small>
                              </>
                            )}
                            {row.costCalculated && (
                              <>
                                <span
                                  className="usage-cost"
                                  title={`价格版本 ${row.costCalculated.version}`}
                                >
                                  {row.costCalculated.amount} {row.costCalculated.currency}
                                </span>
                                <small>
                                  {row.costCalculated.complete ? "当前定价估算" : "部分估算"}
                                </small>
                              </>
                            )}
                            {!row.costReported && !row.costCalculated && "未定价"}
                          </td>
                          <td>
                            {(
                              {
                                request: "请求",
                                turn_aggregate: "回合汇总",
                                message: "消息",
                                session_delta: "会话增量",
                              } as Record<string, string>
                            )[row.granularity] ?? "未知"}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                  {!data.rows.length && <div className="usage-empty">当前范围没有计量记录。</div>}
                </div>
                <div className="usage-pagination">
                  <span className="small muted">
                    共 {data.total.toLocaleString()} 条计量记录 · 每页 {data.pageSize} 条
                  </span>
                  <div className="actions">
                    <button
                      disabled={loading || data.page <= 1}
                      onClick={() => setPage(data.page - 1)}
                    >
                      上一页
                    </button>
                    <span>
                      {data.page} / {maxPage}
                    </span>
                    <button
                      disabled={loading || data.page >= maxPage}
                      onClick={() => setPage(data.page + 1)}
                    >
                      下一页
                    </button>
                  </div>
                </div>
              </>
            )}
          </>
        )}
      </div>
    </div>
  );
}
