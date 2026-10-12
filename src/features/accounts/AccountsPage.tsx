import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type {
  AccountDeleteResult,
  AccountSummaryDto,
  SnapshotOutcome,
} from "../../contracts/accounts";
import AccountMetrics from "./AccountMetrics";
import AddAccountDrawer from "./AddAccountDrawer";
import { DeleteAccountDialog, RotateCredentialDialog } from "./AccountDialogs";
import {
  CONFIG_KIND_LABELS,
  accountInitial,
  credentialSummary,
  evidenceLabel,
  formatRelative,
  formatTime,
  metricMode,
  statusBucket,
  type MetricMode,
  type StatusBucket,
} from "./accountDisplay";
import { platformLabel, platformTemplate, regionLabel } from "./platforms";
import "./accounts.css";

type TypeFilter = "all" | "api" | "plan";

const TYPE_FILTERS: { value: TypeFilter; label: string }[] = [
  { value: "all", label: "全部" },
  { value: "api", label: "API 付费" },
  { value: "plan", label: "Coding Plan" },
];

const STATUS_FILTERS: { value: StatusBucket | "all"; label: string }[] = [
  { value: "all", label: "全部状态" },
  { value: "ok", label: "正常" },
  { value: "stale", label: "快照已过期" },
  { value: "auth", label: "需要更新凭据" },
  { value: "unsupported", label: "暂未接入官方查询能力" },
  { value: "no_plan", label: "无 Coding Plan 套餐" },
  { value: "failed", label: "暂时不可用" },
];

/** 卡片状态徽标：只反映最近一次查询的失败原因，正常/过期另行标记。 */
function latestStatusBadge(
  account: AccountSummaryDto,
): { label: string; dangerous: boolean } | null {
  switch (account.latestStatus) {
    case "auth_required":
      return { label: "需要更新凭据", dangerous: true };
    case "rate_limited":
      return { label: "平台限流", dangerous: false };
    case "unavailable":
      return { label: "暂时不可用", dangerous: true };
    case "incompatible":
      return { label: "接口不兼容", dangerous: true };
    // 平台确认没有套餐是提示信息，不是故障；沿用非警示徽标样式。
    case "no_plan":
      return { label: "无 Coding Plan 套餐", dangerous: false };
    default:
      return null;
  }
}

/**
 * 无官方查询能力的判定：后端状态优先（`unsupported`，如 Codex 进程协议占位）；
 * 还没有任何查询状态时，用平台模板登记的已知能力做初始提示；刷新被
 * 「unsupported platform」拒绝时记入本地集合。
 */
function isUnsupportedAccount(account: AccountSummaryDto, hints: Set<string>): boolean {
  if (hints.has(account.accountId)) return true;
  const status = account.latestStatus ?? account.snapshot?.status ?? null;
  if (status !== null) return status === "unsupported";
  return platformTemplate(account.platform)?.querySupported === false;
}

/** 类型筛选：两者并存的账户在「API 付费」「Coding Plan」两个筛选下都出现。 */
function matchesType(account: AccountSummaryDto, filter: TypeFilter): boolean {
  if (filter === "all") return true;
  if (filter === "api") return account.configKind === "api_payg" || account.configKind === "both";
  return account.configKind === "coding_plan" || account.configKind === "both";
}

export default function AccountsPage() {
  const [accounts, setAccounts] = useState<AccountSummaryDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [typeFilter, setTypeFilter] = useState<TypeFilter>("all");
  const [statusFilter, setStatusFilter] = useState<StatusBucket | "all">("all");
  const [busyId, setBusyId] = useState<string | null>(null);
  const [refreshingAll, setRefreshingAll] = useState(false);
  const [unsupported, setUnsupported] = useState<Set<string>>(() => new Set());
  const [addOpen, setAddOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<AccountSummaryDto | null>(null);
  const [rotateTarget, setRotateTarget] = useState<AccountSummaryDto | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [now, setNow] = useState(() => new Date());

  useEffect(() => {
    const id = window.setInterval(() => setNow(new Date()), 30_000);
    return () => window.clearInterval(id);
  }, []);

  useEffect(() => {
    if (toast === null) return;
    const id = window.setTimeout(() => setToast(null), 5000);
    return () => window.clearTimeout(id);
  }, [toast]);

  const reload = useCallback(async () => {
    try {
      const list = await invoke<AccountSummaryDto[]>("accounts_list");
      setAccounts(list);
      setError(null);
    } catch (e) {
      setAccounts([]);
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  useEffect(() => {
    let active = true;
    let stop: (() => void) | undefined;
    void listen("accounts-updated", () => {
      if (active) void reload();
    })
      .then((unlisten) => {
        if (active) stop = unlisten;
        else unlisten();
      })
      .catch(() => {});
    return () => {
      active = false;
      stop?.();
    };
  }, [reload]);

  const filtered = useMemo(() => {
    if (!accounts) return [];
    return accounts.filter((account) => {
      if (!matchesType(account, typeFilter)) return false;
      if (statusFilter === "all") return true;
      return statusBucket(account, isUnsupportedAccount(account, unsupported)) === statusFilter;
    });
  }, [accounts, typeFilter, statusFilter, unsupported]);

  const refreshOne = useCallback(
    async (account: AccountSummaryDto) => {
      setBusyId(account.accountId);
      setError(null);
      try {
        const outcome = await invoke<SnapshotOutcome>("account_refresh", {
          accountId: account.accountId,
        });
        if (outcome.status === "unsupported") {
          setUnsupported((prev) => new Set(prev).add(account.accountId));
          setToast("该平台暂未接入官方查询能力，已保留账户记录。");
        } else {
          setToast(`「${account.displayName}」刷新完成：${outcome.status}`);
        }
      } catch (e) {
        const message = String(e);
        if (message.includes("unsupported platform")) {
          setUnsupported((prev) => new Set(prev).add(account.accountId));
          setToast("该平台暂未接入官方查询能力，已保留账户记录。");
        } else {
          setError(`「${account.displayName}」刷新失败：${message}`);
        }
      } finally {
        setBusyId(null);
        await reload();
      }
    },
    [reload],
  );

  const refreshAll = useCallback(async () => {
    if (!accounts || accounts.length === 0) return;
    setRefreshingAll(true);
    const unsupportedIds: string[] = [];
    const failures: string[] = [];
    for (const account of accounts) {
      setBusyId(account.accountId);
      try {
        const outcome = await invoke<SnapshotOutcome>("account_refresh", {
          accountId: account.accountId,
        });
        if (outcome.status === "unsupported") unsupportedIds.push(account.accountId);
      } catch (e) {
        const message = String(e);
        if (message.includes("unsupported platform")) unsupportedIds.push(account.accountId);
        else failures.push(`「${account.displayName}」：${message}`);
      }
    }
    setBusyId(null);
    setRefreshingAll(false);
    if (unsupportedIds.length > 0) {
      setUnsupported((prev) => {
        const next = new Set(prev);
        for (const id of unsupportedIds) next.add(id);
        return next;
      });
    }
    setError(failures.length > 0 ? `刷新失败：${failures.join("；")}` : null);
    await reload();
  }, [accounts, reload]);

  const onDeleted = useCallback(
    (result: AccountDeleteResult) => {
      const kept = result.historyKept ? "历史快照已保留" : "历史快照已一并删除";
      setToast(`已删除本地记录：${kept}，未撤销平台 Key。`);
      void reload();
    },
    [reload],
  );

  const empty = accounts !== null && accounts.length === 0 && error === null;
  const noMatch =
    accounts !== null && accounts.length > 0 && filtered.length === 0 && error === null;

  return (
    <section aria-label="账户页面">
      <div className="page-head">
        <div>
          <h1>账户与额度</h1>
          <p className="muted">API 余额与 Coding Plan 独立管理，无需绑定 Agent。</p>
        </div>
        <div className="actions">
          <button
            onClick={() => void refreshAll()}
            disabled={refreshingAll || !accounts || accounts.length === 0}
            title={accounts && accounts.length > 0 ? undefined : "还没有可刷新的账户"}
          >
            {refreshingAll ? "刷新中…" : "全部刷新"}
          </button>
          <button className="primary" onClick={() => setAddOpen(true)}>
            ＋ 添加账户
          </button>
        </div>
      </div>

      <div className="acct-filters" role="group" aria-label="账户筛选">
        <div className="acct-segmented">
          {TYPE_FILTERS.map((option) => (
            <button
              key={option.value}
              className={typeFilter === option.value ? "active" : ""}
              aria-pressed={typeFilter === option.value}
              onClick={() => setTypeFilter(option.value)}
            >
              {option.label}
            </button>
          ))}
        </div>
        <label className="acct-status-filter">
          <span className="small muted">状态</span>
          <select
            value={statusFilter}
            onChange={(e) => setStatusFilter(e.target.value as StatusBucket | "all")}
          >
            {STATUS_FILTERS.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
        <span className="small muted">额度是平台范围，不与本机 Token 求和</span>
      </div>

      {error && (
        <div className="card" role="alert">
          <p className="muted">账户读取失败：{error}</p>
        </div>
      )}

      {accounts === null && !error && (
        <div className="card">
          <p className="muted">读取中…</p>
        </div>
      )}

      {empty && (
        <div className="card acct-empty">
          <p>
            <strong>还没有账户。</strong>
          </p>
          <p className="muted">
            点击右上角「添加账户」：平台与区域 → 账户类型 → 凭据类型与最小权限说明 → 只读验证。 API
            Key 明文保存在本机，重启后直接使用；不访问系统钥匙串。
          </p>
          <p className="muted small">
            账户不绑定 Agent；平台没有官方查询能力时只保留管理与凭据，不显示虚构余额。
          </p>
        </div>
      )}

      {noMatch && (
        <div className="card">
          <p className="muted">当前筛选下没有账户。可重置筛选查看全部账户。</p>
          <button
            onClick={() => {
              setTypeFilter("all");
              setStatusFilter("all");
            }}
          >
            重置筛选
          </button>
        </div>
      )}

      {filtered.map((account) => {
        const unsupportedAccount = isUnsupportedAccount(account, unsupported);
        const mode: MetricMode = metricMode(account, unsupportedAccount);
        return (
          <AccountCard
            key={account.accountId}
            account={account}
            mode={mode}
            now={now}
            busy={busyId === account.accountId}
            onRefresh={() => void refreshOne(account)}
            onRotate={() => setRotateTarget(account)}
            onDelete={() => setDeleteTarget(account)}
          />
        );
      })}

      {addOpen && (
        <AddAccountDrawer
          onClose={() => setAddOpen(false)}
          onSaved={() => {
            setToast("账户已保存。可用「立即验证」做一次只读查询。");
            void reload();
          }}
        />
      )}

      {deleteTarget && (
        <DeleteAccountDialog
          account={deleteTarget}
          onClose={() => setDeleteTarget(null)}
          onDeleted={onDeleted}
        />
      )}

      {rotateTarget && (
        <RotateCredentialDialog
          account={rotateTarget}
          onClose={() => setRotateTarget(null)}
          onRotated={() => void reload()}
        />
      )}

      {toast && (
        <div className="acct-toast" role="status">
          {toast}
        </div>
      )}
    </section>
  );
}

// ---------------------------------------------------------------------------
// 账户卡片
// ---------------------------------------------------------------------------

interface CardProps {
  account: AccountSummaryDto;
  mode: MetricMode;
  now: Date;
  busy: boolean;
  onRefresh: () => void;
  onRotate: () => void;
  onDelete: () => void;
}

function AccountCard({ account, mode, now, busy, onRefresh, onRotate, onDelete }: CardProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    const onDocClick = (event: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) setMenuOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenuOpen(false);
    };
    document.addEventListener("mousedown", onDocClick);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDocClick);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuOpen]);

  const region = regionLabel(account.region);
  const evidence = evidenceLabel(account.snapshot?.evidence ?? null);
  const badge = latestStatusBadge(account);
  const refreshedAt = formatTime(account.snapshot?.queriedAt ?? null);
  const refreshedAgo = formatRelative(account.snapshot?.queriedAt ?? null, now);

  const run = (action: () => void) => () => {
    setMenuOpen(false);
    action();
  };

  return (
    <article className="card acct-card">
      <div className="acct-icon" aria-hidden="true">
        {accountInitial(account)}
      </div>

      <div className="acct-identity">
        <div className="acct-name">{account.displayName}</div>
        <p className="small muted">
          {platformLabel(account.platform)}
          {region ? ` · ${region}` : ""} · {CONFIG_KIND_LABELS[account.configKind]}
        </p>
        <p className="small muted">{credentialSummary(account.credentials)}</p>
        {evidence && <span className="badge">{evidence}</span>}
      </div>

      <div className="acct-metric-slot">
        {account.credentialReady === false && (
          <div className="acct-session-note">
            <strong>需要重新保存 API Key</strong>
            <p className="small muted">
              账户与历史快照已保留。旧钥匙串条目不会读取，请重新输入一次 Key。
            </p>
            <button onClick={onRotate}>输入 API Key</button>
          </div>
        )}
        <AccountMetrics account={account} mode={mode} now={now} onRotateCredential={onRotate} />
      </div>

      <div className="acct-updated">
        <span className="small muted">
          {busy ? "刷新中…" : refreshedAt ? "最近刷新" : "尚无快照"}
        </span>
        {refreshedAt && !busy && <span className="small">{refreshedAt}</span>}
        {refreshedAgo && !busy && <span className="small muted">{refreshedAgo}</span>}
        {badge && (
          <span className={`badge ${badge.dangerous ? "acct-badge-danger" : "acct-badge-warn"}`}>
            {badge.label}
          </span>
        )}
      </div>

      <div className="acct-more" ref={menuRef}>
        <button
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          aria-label={`${account.displayName} 操作菜单`}
          onClick={() => setMenuOpen((open) => !open)}
        >
          ⋯
        </button>
        {menuOpen && (
          <div className="acct-menu" role="menu">
            <button role="menuitem" disabled={busy} onClick={run(onRefresh)}>
              刷新
            </button>
            <button role="menuitem" onClick={run(onRotate)}>
              更新凭据
            </button>
            <button role="menuitem" className="acct-menu-danger" onClick={run(onDelete)}>
              删除本地记录
            </button>
          </div>
        )}
      </div>
    </article>
  );
}
