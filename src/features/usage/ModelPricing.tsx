import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ModelPrice } from "../../contracts/pricing";
import type { UsageOptions } from "../../contracts/usage";
import { AGENTS, agentLabel } from "./usageDisplay";

const blank = (agent = "codex", model = ""): ModelPrice => ({
  version: "",
  agent,
  sourceId: null,
  model,
  currency: "USD",
  input: null,
  output: null,
  cacheRead: null,
  cacheWrite: null,
  enabled: true,
});
const FIELDS = [
  ["input", "新增输入"],
  ["output", "输出"],
  ["cacheRead", "缓存读取"],
  ["cacheWrite", "缓存创建"],
] as const;
export default function ModelPricing({
  onClose,
  onSaved,
  initial,
}: {
  onClose: () => void;
  onSaved: () => void;
  initial?: { agent: string; model: string };
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [rules, setRules] = useState<ModelPrice[]>([]);
  const [draft, setDraft] = useState<ModelPrice>(() => blank(initial?.agent, initial?.model));
  const [options, setOptions] = useState<UsageOptions>({ sources: [], models: [] });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const reload = useCallback(async () => {
    try {
      const [prices, opts] = await Promise.all([
        invoke<ModelPrice[]>("model_prices_list"),
        invoke<UsageOptions>("usage_options"),
      ]);
      setRules(prices);
      setOptions(opts);
    } catch (e) {
      setError(String(e));
    }
  }, []);
  useEffect(() => {
    dialog.current?.showModal();
    void reload();
  }, [reload]);
  const save = async (rule: ModelPrice) => {
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      await invoke("model_price_save", { input: rule });
      await reload();
      onSaved();
      setSaved(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <dialog
      ref={dialog}
      className="pricing-dialog"
      aria-label="模型成本定价"
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
    >
      <div className="pricing-head">
        <div>
          <span className="usage-eyebrow">COST SETTINGS</span>
          <h2>模型成本定价</h2>
        </div>
        <button onClick={onClose} aria-label="关闭模型定价">
          关闭
        </button>
      </div>
      <p className="muted">
        按每百万 Token 填写价格。保存后，用量页会按当前价格估算所选范围，保留原始费用和旧价格版本。
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void save(draft);
        }}
      >
        <div className="pricing-grid">
          <label>
            Agent
            <select
              value={draft.agent}
              onChange={(e) => {
                setDraft({ ...draft, agent: e.target.value, sourceId: null });
                setSaved(false);
              }}
            >
              {AGENTS.map(([id, label]) => (
                <option key={id} value={id}>
                  {label}
                </option>
              ))}
            </select>
          </label>
          <label>
            适用来源
            <select
              value={draft.sourceId ?? ""}
              onChange={(e) => setDraft({ ...draft, sourceId: e.target.value || null })}
            >
              <option value="">该 Agent 的全部来源</option>
              {options.sources
                .filter((s) => s.agent === draft.agent)
                .map((s) => (
                  <option key={s.sourceId} value={s.sourceId}>
                    {s.label}
                  </option>
                ))}
            </select>
          </label>
          <label className="pricing-model">
            模型名称
            <input
              required
              maxLength={200}
              list="price-model-options"
              placeholder="与日志中的模型名完全匹配"
              value={draft.model}
              onChange={(e) => setDraft({ ...draft, model: e.target.value })}
            />
            <datalist id="price-model-options">
              {options.models.map((m) => (
                <option key={m} value={m} />
              ))}
            </datalist>
          </label>
          <label>
            币种
            <select
              value={draft.currency}
              onChange={(e) => setDraft({ ...draft, currency: e.target.value as "USD" | "CNY" })}
            >
              <option value="USD">USD · 美元</option>
              <option value="CNY">CNY · 人民币</option>
            </select>
          </label>
        </div>
        <div className="pricing-rates">
          {FIELDS.map(([key, label]) => (
            <label key={key}>
              {label}
              <div className="price-input">
                <input
                  inputMode="decimal"
                  pattern="[0-9]+([.][0-9]{1,9})?"
                  placeholder="未定价"
                  value={draft[key] ?? ""}
                  onChange={(e) => {
                    setDraft({ ...draft, [key]: e.target.value || null });
                    setSaved(false);
                  }}
                />
                <span>{draft.currency} / 1M</span>
              </div>
            </label>
          ))}
        </div>
        <p className="small muted">
          留空 = 未知；0 =
          明确免费。仅计算已知分量，缺少用量或价格时标为部分估算。特定来源规则优先，不从模型名称猜供应商。订阅用量的
          API 等价估算不代表实际账单。
        </p>
        {error && (
          <p role="alert" className="monitor-error">
            {error}
          </p>
        )}
        {saved && (
          <p role="status" className="usage-cost">
            已保存，统计已按当前价格更新。
          </p>
        )}
        <div className="pricing-actions">
          <button
            type="button"
            disabled={busy}
            onClick={() => {
              setDraft(blank());
              setSaved(false);
            }}
          >
            新建规则
          </button>
          <button className="primary" disabled={busy}>
            {busy ? "保存中…" : "保存并用于估算"}
          </button>
        </div>
      </form>
      <div className="pricing-saved">
        <h3>
          已配置模型 <span className="badge">{rules.length}</span>
        </h3>
        {!rules.length && <p className="muted">尚未配置价格，可以先从实际使用的模型开始。</p>}
        {rules.map((rule) => (
          <div className="pricing-rule" key={rule.version}>
            <div>
              <strong>{rule.model}</strong>
              <p className="small muted">
                {agentLabel(rule.agent)} ·{" "}
                {rule.sourceId
                  ? (options.sources.find((s) => s.sourceId === rule.sourceId)?.label ?? "指定来源")
                  : "全部来源"}{" "}
                · {rule.currency} / 1M
              </p>
              <span className="small muted">
                {FIELDS.map(([key, label]) => `${label} ${rule[key] ?? "—"}`).join(" · ")}
              </span>
            </div>
            <div className="actions">
              <button
                disabled={busy}
                onClick={() => {
                  setDraft(rule);
                  setSaved(false);
                }}
              >
                编辑
              </button>
              <button disabled={busy} onClick={() => void save({ ...rule, enabled: false })}>
                停用
              </button>
            </div>
          </div>
        ))}
      </div>
    </dialog>
  );
}
