import type { AccountSummaryDto } from "../../contracts/accounts";
import {
  BALANCE_KIND_LABELS,
  QUOTA_UNIT_LABELS,
  formatTime,
  groupBalances,
  qualityNote,
  sortWindows,
  unsupportedNote,
  windowDisplay,
  type MetricMode,
} from "./accountDisplay";

interface Props {
  account: AccountSummaryDto;
  mode: MetricMode;
  now: Date;
  onRotateCredential: () => void;
}

/**
 * 账户卡片指标区。规则见 ACCOUNT_ADAPTERS 第 6 节：
 * - unsupported → 不显示任何余额数字；
 * - auth_required → 引导更新凭据，不显示零余额（旧值仍保留并标过期）；
 * - 其余 → 显示余额与额度窗口，stale 时保留旧值并加橙色「快照已过期」。
 */
export default function AccountMetrics({ account, mode, now, onRotateCredential }: Props) {
  if (mode === "unsupported") {
    return (
      <div className="acct-unsupported">
        <p>
          <strong>{unsupportedNote()}</strong>
        </p>
        <p className="small muted">
          保留别名、区域与凭据管理；不使用第三方控制台 Cookie 查询，也不把未知显示成 0。
        </p>
      </div>
    );
  }

  const snapshot = account.snapshot;
  const balanceGroups = snapshot ? groupBalances(snapshot.balances) : [];
  const windows = snapshot ? sortWindows(snapshot.quotaWindows) : [];
  const metrics = snapshot?.usageMetrics ?? [];
  const staleAt = snapshot ? formatTime(snapshot.queriedAt) : null;

  return (
    <div className="acct-metrics">
      {mode === "auth" && (
        <div className="acct-alert" role="alert">
          <div>
            <strong>需要更新凭据</strong>
            <p className="small muted">
              平台返回认证失败或权限不足。这里不显示余额数字，避免把“无法读取”当成 0。
            </p>
          </div>
          <button onClick={onRotateCredential}>更新凭据</button>
        </div>
      )}

      {account.stale && (
        <p className="acct-stale">
          <span className="badge acct-badge-warn">快照已过期</span>
          {staleAt && <span className="small muted">原快照时间 {staleAt}</span>}
        </p>
      )}

      {balanceGroups.length > 0 && (
        <div className="acct-balances">
          {balanceGroups.map((group) => (
            <div className="acct-balance" key={`${group.currency}:${group.walletScope ?? ""}`}>
              <div className="acct-balance-main">
                <span className="small muted">
                  {group.primary
                    ? BALANCE_KIND_LABELS[group.primary.kind]
                    : "分项余额（未返回总额）"}
                </span>
                {group.primary ? (
                  <div className="acct-balance-value num">
                    {group.primary.amount} <span className="acct-currency">{group.currency}</span>
                  </div>
                ) : (
                  <div className="acct-balance-value num muted">—</div>
                )}
                {group.walletScope && (
                  <span className="small muted">钱包范围：{group.walletScope}</span>
                )}
                {group.primary && qualityNote(group.primary.quality) && (
                  <span className="small muted">（{qualityNote(group.primary.quality)}）</span>
                )}
              </div>

              {group.components.length > 0 && (
                <details className="acct-breakdown">
                  <summary className="small muted">
                    充值 / 赠金 / 授信明细（{group.components.length}）
                  </summary>
                  <ul>
                    {group.components.map((entry, index) => (
                      <li key={`${entry.kind}:${entry.walletScope ?? ""}:${index}`}>
                        <span className="muted">{BALANCE_KIND_LABELS[entry.kind]}</span>
                        <span className="num">
                          {entry.amount} {entry.currency}
                        </span>
                        {qualityNote(entry.quality) && (
                          <span className="small muted">（{qualityNote(entry.quality)}）</span>
                        )}
                      </li>
                    ))}
                  </ul>
                  <p className="small muted">
                    {group.primary
                      ? "明细为来源分项，已含在总额内，不重复相加。"
                      : "平台未返回总额，仅按来源逐项列出。"}
                  </p>
                </details>
              )}
            </div>
          ))}
        </div>
      )}

      {windows.length > 0 && (
        <div className="acct-windows">
          {windows.map((window) => {
            const display = windowDisplay(window, now);
            const kindText = display.percentKind === "used" ? "已用" : "剩余";
            return (
              <div className="acct-window" key={window.windowId}>
                <div className="acct-window-head">
                  <span>
                    {display.title}
                    {kindText}{" "}
                    <strong className="num">
                      {display.percent === null ? "—" : `${display.percent}%`}
                    </strong>
                    {display.resetText && <span className="muted"> · {display.resetText}</span>}
                  </span>
                  {display.overLimit && <span className="badge acct-badge-danger">超限</span>}
                </div>
                <div
                  className={`acct-progress${display.overLimit ? " danger" : ""}`}
                  role="presentation"
                >
                  <span style={{ width: `${display.barPercent ?? 0}%` }} />
                </div>
                <p className="small muted">
                  {display.percent === null &&
                    display.absoluteRemaining !== null &&
                    `平台只返回绝对剩余量 ${display.absoluteRemaining} ${display.unitLabel} · `}
                  {display.percentKind === "used" && "平台只返回已用比例 · "}
                  按平台返回的{display.unitLabel}窗口显示，不反推绝对量
                </p>
              </div>
            );
          })}
        </div>
      )}

      {metrics.length > 0 && (
        <div className="acct-extra-metrics">
          <span className="small muted">账户级指标</span>
          {metrics.map((metric) => (
            <span className="small muted" key={`${metric.name}:${metric.scope ?? ""}`}>
              {metric.name} <span className="num">{metric.value}</span>{" "}
              {QUOTA_UNIT_LABELS[metric.unit]}
              {metric.scope ? `（${metric.scope}）` : ""}
            </span>
          ))}
        </div>
      )}

      {snapshot === null && (
        <p className="muted small">尚无快照。点击操作菜单的「刷新」获取一次只读查询结果。</p>
      )}

      {snapshot !== null && balanceGroups.length === 0 && windows.length === 0 && (
        <p className="muted small">
          这次查询没有返回余额或额度字段。未提供 ≠ 0，刷新失败时保留最近成功值。
        </p>
      )}

      {snapshot?.providerAsOf && (
        <p className="small muted">
          平台数据时间 {formatTime(snapshot.providerAsOf) ?? snapshot.providerAsOf}
        </p>
      )}
    </div>
  );
}
