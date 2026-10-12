import { SelectField as Select } from "../../components/Select";
import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type {
  AccountSaveInput,
  AccountSummaryDto,
  ConfigKind,
  CredentialType,
  SnapshotOutcome,
} from "../../contracts/accounts";
import { CONFIG_KIND_LABELS, STATUS_LABELS, credentialSummary } from "./accountDisplay";
import { PLATFORM_TEMPLATES, platformTemplate } from "./platforms";

interface Props {
  onClose: () => void;
  onSaved: (account: AccountSummaryDto) => void;
}

const CONFIG_KINDS: ConfigKind[] = ["api_payg", "coding_plan", "both"];

/**
 * 添加账户抽屉。步骤：平台与区域 → 账户类型 → 凭据类型与最小权限说明 → 密码框输入 →
 * 保存后显示别名与掩码确认，并提供「立即验证」。
 *
 * 这里**不出现**「选择接入哪个 Agent」的步骤：账户与 Agent 采集是两个独立领域
 * （AGENTS.md 约束 1）。
 */
export default function AddAccountDrawer({ onClose, onSaved }: Props) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [platform, setPlatform] = useState(PLATFORM_TEMPLATES[0].key);
  const [region, setRegion] = useState(PLATFORM_TEMPLATES[0].regions[0]?.value ?? "");
  const [displayName, setDisplayName] = useState("");
  const [scope, setScope] = useState("");
  const [configKind, setConfigKind] = useState<ConfigKind>("api_payg");
  const [credentialType, setCredentialType] = useState<CredentialType>(
    PLATFORM_TEMPLATES[0].credentials[0].type,
  );
  const [secret, setSecret] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<AccountSummaryDto | null>(null);
  const [verifying, setVerifying] = useState(false);
  const [verification, setVerification] = useState<{ ok: boolean; text: string } | null>(null);

  const template = useMemo(() => platformTemplate(platform), [platform]);
  const credential = template?.credentials.find((c) => c.type === credentialType) ?? null;

  useEffect(() => {
    const dialog = dialogRef.current;
    if (dialog && !dialog.open && typeof dialog.showModal === "function") dialog.showModal();
  }, []);

  const close = () => {
    dialogRef.current?.close();
    onClose();
  };

  const onPlatformChange = (key: string) => {
    setPlatform(key);
    if (key === "codex") setConfigKind("coding_plan");
    const next = platformTemplate(key);
    setRegion(next?.regions[0]?.value ?? "");
    setCredentialType(next?.credentials[0]?.type ?? "api_key");
  };

  const onSave = () => {
    if (!template) return;
    if (displayName.trim().length === 0) {
      setError("请先填写账户别名。");
      return;
    }
    if (platform !== "codex" && secret.length === 0) {
      setError("请填写凭据（密码框输入，不回显、不落普通数据库）。");
      return;
    }
    const input: AccountSaveInput = {
      accountId: null,
      platform,
      region: region || null,
      displayName: displayName.trim(),
      scope: scope.trim() || null,
      configKind,
      credentialType: platform === "codex" ? null : credentialType,
      secret: platform === "codex" ? null : secret,
      capabilities: null,
    };
    setSaving(true);
    setError(null);
    invoke<AccountSummaryDto>("account_save", { input })
      .then((account) => {
        setSecret("");
        setSaved(account);
        onSaved(account);
      })
      .catch((e) => setError(String(e)))
      .finally(() => setSaving(false));
  };

  const onVerify = () => {
    if (!saved) return;
    setVerifying(true);
    setVerification(null);
    invoke<SnapshotOutcome>("account_refresh", { accountId: saved.accountId })
      .then((outcome) => {
        const label = STATUS_LABELS[outcome.status];
        const detail =
          outcome.status === "unsupported"
            ? "该平台暂未接入官方查询能力，保留账户记录。"
            : outcome.balances.length === 0 && outcome.quotaWindows.length === 0
              ? "本次查询未返回余额或额度字段（未提供 ≠ 0）。"
              : `返回 ${outcome.balances.length} 条余额、${outcome.quotaWindows.length} 个额度窗口。`;
        setVerification({ ok: outcome.status === "ok", text: `${label}：${detail}` });
      })
      .catch((e) => {
        const message = String(e);
        setVerification({
          ok: false,
          text: message.includes("unsupported platform")
            ? "该平台暂未接入官方查询能力，保留账户记录。"
            : `验证失败：${message}`,
        });
      })
      .finally(() => setVerifying(false));
  };

  return (
    <dialog
      ref={dialogRef}
      className="acct-drawer"
      aria-label="添加账户"
      onCancel={(event) => {
        event.preventDefault();
        close();
      }}
    >
      <div className="acct-drawer-head">
        <h2>添加账户</h2>
        <button onClick={close} aria-label="关闭添加账户">
          关闭
        </button>
      </div>

      {saved === null ? (
        <form
          className="acct-form"
          onSubmit={(event) => {
            event.preventDefault();
            onSave();
          }}
        >
          <fieldset>
            <legend>平台与区域</legend>
            <label>
              <span>平台</span>
              <Select
                aria-label="账户平台"
                value={platform}
                onChange={(e) => onPlatformChange(e.target.value)}
              >
                {PLATFORM_TEMPLATES.map((t) => (
                  <option key={t.key} value={t.key}>
                    {t.label}
                  </option>
                ))}
              </Select>
            </label>
            <label>
              <span>区域</span>
              <Select
                aria-label="账户区域"
                value={region}
                onChange={(e) => setRegion(e.target.value)}
              >
                <option value="">未指定区域</option>
                {template?.regions.map((r) => (
                  <option key={r.value} value={r.value}>
                    {r.label}
                  </option>
                ))}
              </Select>
            </label>
            <label>
              <span>账户别名</span>
              <input
                value={displayName}
                maxLength={80}
                placeholder="例如：DeepSeek 主账户"
                onChange={(e) => setDisplayName(e.target.value)}
              />
            </label>
            <label>
              <span>组织 / 项目范围（可选）</span>
              <input
                value={scope}
                maxLength={120}
                placeholder="例如：org_xxx / project_xxx"
                onChange={(e) => setScope(e.target.value)}
              />
            </label>
            {platform === "codex" && <p className="small muted">{template?.note}</p>}
          </fieldset>

          <fieldset>
            <legend>账户类型</legend>
            <div className="acct-radios">
              {(platform === "codex" ? ["coding_plan" as const] : CONFIG_KINDS).map((kind) => (
                <label key={kind} className="acct-radio">
                  <input
                    type="radio"
                    name="configKind"
                    value={kind}
                    checked={configKind === kind}
                    onChange={() => setConfigKind(kind)}
                  />
                  <span>{CONFIG_KIND_LABELS[kind]}</span>
                </label>
              ))}
            </div>
          </fieldset>

          {platform !== "codex" && (
            <fieldset>
              <legend>凭据</legend>
              <label>
                <span>凭据类型</span>
                <Select
                  aria-label="凭据类型"
                  value={credentialType}
                  onChange={(e) => setCredentialType(e.target.value as CredentialType)}
                >
                  {template?.credentials.map((c) => (
                    <option key={c.type} value={c.type}>
                      {c.label}
                    </option>
                  ))}
                </Select>
              </label>
              {credential && (
                <details className="acct-permission">
                  <summary className="small muted">连接说明</summary>
                  <p className="small muted">
                    {template?.note} {credential.minimalPermission}
                  </p>
                </details>
              )}
            </fieldset>
          )}

          {platform !== "codex" && (
            <fieldset>
              <legend>凭据输入</legend>
              <label>
                <span>API Key / 令牌</span>
                <input
                  type="password"
                  autoComplete="off"
                  spellCheck={false}
                  value={secret}
                  placeholder="粘贴凭据"
                  onChange={(e) => setSecret(e.target.value)}
                />
              </label>
              <p className="small muted">凭据仅在本机明文保存。</p>
            </fieldset>
          )}

          {error && (
            <p className="acct-error" role="alert">
              {error}
            </p>
          )}

          <div className="acct-form-actions">
            <button type="button" onClick={close}>
              取消
            </button>
            <button type="submit" className="primary" disabled={saving}>
              {saving ? "保存中…" : "保存账户"}
            </button>
          </div>
        </form>
      ) : (
        <div className="acct-form">
          <div className="acct-saved" role="status">
            <p>
              <strong>已保存：{saved.displayName}</strong>
            </p>
            <p className="small muted">
              {saved.platform === "codex"
                ? "使用本机 ChatGPT 登录"
                : credentialSummary(saved.credentials)}{" "}
              · {saved.platform}
              {saved.region ? ` · ${saved.region}` : ""}
            </p>
          </div>

          <div className="acct-form-actions">
            <button onClick={close}>完成</button>
            <button className="primary" disabled={verifying} onClick={onVerify}>
              {verifying ? "验证中…" : "立即验证"}
            </button>
          </div>

          {verification && (
            <p className={verification.ok ? "small" : "small acct-error"} role="status">
              {verification.text}
            </p>
          )}
        </div>
      )}
    </dialog>
  );
}
