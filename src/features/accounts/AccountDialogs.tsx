import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type {
  AccountDeleteInput,
  AccountDeleteResult,
  AccountRotateInput,
  AccountSummaryDto,
  CredentialDto,
} from "../../contracts/accounts";
import { CREDENTIAL_TYPE_LABELS, activeCredentials } from "./accountDisplay";

// ---------------------------------------------------------------------------
// 删除账户
// ---------------------------------------------------------------------------

interface DeleteProps {
  account: AccountSummaryDto;
  onClose: () => void;
  onDeleted: (result: AccountDeleteResult) => void;
}

/**
 * 删除本地账户记录。明确范围：只删本地记录与凭据引用，**不撤销平台 Key**；
 * 历史快照默认保留，可由用户选择一并删除。
 */
export function DeleteAccountDialog({ account, onClose, onDeleted }: DeleteProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [deleteHistory, setDeleteHistory] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<AccountDeleteResult | null>(null);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (dialog && !dialog.open && typeof dialog.showModal === "function") dialog.showModal();
  }, []);

  const close = () => {
    dialogRef.current?.close();
    onClose();
  };

  const onConfirm = () => {
    const input: AccountDeleteInput = {
      accountId: account.accountId,
      keepHistory: !deleteHistory,
    };
    setBusy(true);
    setError(null);
    invoke<AccountDeleteResult>("account_delete", { input })
      .then((value) => {
        setResult(value);
        onDeleted(value);
      })
      .catch((e) => setError(String(e)))
      .finally(() => setBusy(false));
  };

  return (
    <dialog
      ref={dialogRef}
      className="acct-dialog"
      aria-label={`删除账户 ${account.displayName}`}
      onCancel={(event) => {
        event.preventDefault();
        close();
      }}
    >
      <div className="acct-drawer-head">
        <h2>删除本地账户记录</h2>
        <button onClick={close} aria-label="关闭删除确认">
          关闭
        </button>
      </div>

      {result === null ? (
        <div className="acct-form">
          <p>
            即将删除 <strong>{account.displayName}</strong>（{account.platform}）。
          </p>
          <ul className="acct-scope">
            <li>
              <strong>会删除：</strong>本地账户条目、凭据引用，以及（可选）历史快照。
            </li>
            <li>
              <strong>不会撤销：</strong>平台上的 API Key / 令牌仍然有效，需你到平台侧自行吊销。
            </li>
            <li>
              <strong>不会删除：</strong>已采集的用量记录与数据源配置——账户域与用量账本相互独立。
            </li>
          </ul>
          <label className="acct-check">
            <input
              type="checkbox"
              checked={deleteHistory}
              onChange={(e) => setDeleteHistory(e.target.checked)}
            />
            <span>同时删除历史快照（默认保留，便于追溯过去的时间点）</span>
          </label>
          {error && (
            <p className="acct-error" role="alert">
              {error}
            </p>
          )}
          <div className="acct-form-actions">
            <button onClick={close}>取消</button>
            <button className="primary" disabled={busy} onClick={onConfirm}>
              {busy ? "删除中…" : "确认删除本地记录"}
            </button>
          </div>
        </div>
      ) : (
        <div className="acct-form">
          <p role="status">
            <strong>已删除本地记录。</strong>
          </p>
          <p className="small muted">
            清理凭据引用 {result.credentials.refsRemoved} 条 · 凭据库条目{" "}
            {result.credentials.vaultEntriesRemoved} 条
            {result.credentials.sharedRefsKept > 0 &&
              ` · 因共享引用保留 ${result.credentials.sharedRefsKept} 条`}
            {` · 删除快照 ${result.snapshotsRemoved} 条`}
          </p>
          <p className="small muted">
            {result.historyKept ? "历史快照已保留。" : "历史快照已一并删除。"}
            删除本地记录不等于撤销平台 Key。
          </p>
          <div className="acct-form-actions">
            <button className="primary" onClick={close}>
              完成
            </button>
          </div>
        </div>
      )}
    </dialog>
  );
}

// ---------------------------------------------------------------------------
// 更新凭据
// ---------------------------------------------------------------------------

interface RotateProps {
  account: AccountSummaryDto;
  onClose: () => void;
  onRotated: () => void;
}

/** 更新凭据：新秘密替换旧引用，旧引用保留行与时间信息（不是撤销平台 Key）。 */
export function RotateCredentialDialog({ account, onClose, onRotated }: RotateProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const active = useMemo(() => activeCredentials(account.credentials), [account.credentials]);
  const [credentialId, setCredentialId] = useState(active[0]?.credentialId ?? "");
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [rotated, setRotated] = useState<CredentialDto | null>(null);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (dialog && !dialog.open && typeof dialog.showModal === "function") dialog.showModal();
  }, []);

  const close = () => {
    dialogRef.current?.close();
    onClose();
  };

  const onConfirm = () => {
    if (!credentialId) {
      setError("该账户没有可更新的当前凭据，请重新添加账户。");
      return;
    }
    if (secret.length === 0) {
      setError("请填写新的凭据（密码框输入，不回显）。");
      return;
    }
    const input: AccountRotateInput = { credentialId, secret, capabilities: null };
    setBusy(true);
    setError(null);
    invoke<CredentialDto>("account_rotate_credential", { input })
      .then((value) => {
        setSecret("");
        setRotated(value);
        onRotated();
      })
      .catch((e) => setError(String(e)))
      .finally(() => setBusy(false));
  };

  return (
    <dialog
      ref={dialogRef}
      className="acct-dialog"
      aria-label={`更新 ${account.displayName} 的凭据`}
      onCancel={(event) => {
        event.preventDefault();
        close();
      }}
    >
      <div className="acct-drawer-head">
        <h2>更新凭据</h2>
        <button onClick={close} aria-label="关闭更新凭据">
          关闭
        </button>
      </div>

      {rotated === null ? (
        <form
          className="acct-form"
          onSubmit={(event) => {
            event.preventDefault();
            onConfirm();
          }}
        >
          <p className="muted small">
            当前凭据：{active.length > 0 ? active[0].mask : "无"}。更新后旧错误与快照保留时间信息；
            更新本地引用不等于平台侧撤销。
          </p>
          <p className="small muted">
            API Key 明文保存在本机凭据文件中，重启后直接使用；不访问系统钥匙串。
          </p>
          {active.length > 1 && (
            <label>
              <span>要替换的凭据</span>
              <select value={credentialId} onChange={(e) => setCredentialId(e.target.value)}>
                {active.map((c) => (
                  <option key={c.credentialId} value={c.credentialId}>
                    {CREDENTIAL_TYPE_LABELS[c.credentialType]} {c.mask}
                  </option>
                ))}
              </select>
            </label>
          )}
          <label>
            <span>新凭据（密码框，不回显）</span>
            <input
              type="password"
              autoComplete="off"
              spellCheck={false}
              value={secret}
              placeholder="粘贴新凭据"
              onChange={(e) => setSecret(e.target.value)}
            />
          </label>
          {error && (
            <p className="acct-error" role="alert">
              {error}
            </p>
          )}
          <div className="acct-form-actions">
            <button type="button" onClick={close}>
              取消
            </button>
            <button type="submit" className="primary" disabled={busy}>
              {busy ? "更新中…" : "更新凭据"}
            </button>
          </div>
        </form>
      ) : (
        <div className="acct-form">
          <p role="status">
            <strong>
              已更新：{CREDENTIAL_TYPE_LABELS[rotated.credentialType]} {rotated.mask}
            </strong>
          </p>
          <p className="small muted">明文未回显；可到卡片操作菜单点「刷新」重新做一次只读验证。</p>
          <div className="acct-form-actions">
            <button className="primary" onClick={close}>
              完成
            </button>
          </div>
        </div>
      )}
    </dialog>
  );
}
