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
      </div>
    );
  }

  const snapshot = account.snapshot;
  const balanceGroups = snapshot ? groupBalances(snapshot.balances ?? []) : [];
  const windows = snapshot ? sortWindows(snapshot.quotaWindows ?? []) : [];
  const metrics = snapshot?.usageMetrics ?? [];
  const staleAt = snapshot ? formatTime(snapshot.queriedAt) : null;

  return (
    <div className="acct-metrics">
      {mode === "auth" && (
        <div className="acct-alert" role="alert">
          <div>
            <strong>{account.platform === "codex" ? "请在 Codex 中登录" : "需要更新凭据"}</strong>
            <p className="small muted">
              {account.platform === "codex"
                ? "请先在官方 Codex 中使用 ChatGPT 账户登录，再刷新额度。"
                : account.credentialReady === false
                  ? "请重新保存 API Key，以启用账户查询。"
                  : "平台返回认证失败或权限不足。"}
            </p>
          </div>
          {account.platform !== "codex" && <button onClick={onRotateCredential}>更新凭据</button>}
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
                  <span className="acct-window-title">{display.title}</span>
                  {display.overLimit && <span className="badge acct-badge-danger">超限</span>}
                </div>
                <div className="acct-window-value">
                  <span className="small muted">{kindText}</span>
                  <strong>{display.percent === null ? "—" : `${display.percent}%`}</strong>
                </div>
                {display.resetText && (
                  <p className="acct-window-reset small muted">{display.resetText}</p>
                )}
                <div
                  className={`acct-progress${display.overLimit ? " danger" : ""}`}
                  role="presentation"
                >
                  <span style={{ width: `${display.barPercent ?? 0}%` }} />
                </div>
                {display.percent === null && display.absoluteRemaining !== null && (
                  <p className="small muted">
                    剩余 {display.absoluteRemaining} {display.unitLabel}
                  </p>
                )}
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

      {snapshot === null && <p className="muted small">尚无数据</p>}

      {snapshot !== null && balanceGroups.length === 0 && windows.length === 0 && (
        <p className="muted small">平台未提供余额或额度</p>
      )}

      {snapshot?.providerAsOf && (
        <p className="small muted">
          平台数据时间 {formatTime(snapshot.providerAsOf) ?? snapshot.providerAsOf}
        </p>
      )}
    </div>
  );
}
