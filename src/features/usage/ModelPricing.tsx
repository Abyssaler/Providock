import { SelectField as Select } from "../../components/Select";
import Hint from "../../components/Hint";
import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { ModelPrice, PriceQuote } from "../../contracts/pricing";
import type { UsageOptions } from "../../contracts/usage";
import { AGENTS, agentLabel } from "./usageDisplay";
import {
  DEFAULT_RANGES,
  PEAK_TIMEZONE,
  blankStrategyForm,
  serializeModelPrice,
  strategyFormOf,
  validateModelPrice,
  type RateKey,
  type StrategyForm,
} from "./pricingStrategy";

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

function RateFields({
  rates,
  currency,
  onChange,
}: {
  rates: { [key in RateKey]?: string | null };
  currency: string;
  onChange: (key: RateKey, value: string) => void;
}) {
  return (
    <div className="pricing-rates">
      {FIELDS.map(([key, label]) => (
        <label key={key}>
          {label}
          <div className="price-input">
            <input
              inputMode="decimal"
              pattern="[0-9]{1,9}([.][0-9]{1,9})?"
              placeholder="未定价"
              value={rates[key] ?? ""}
              onChange={(e) => onChange(key, e.target.value)}
            />
            <span>{currency} / 1M</span>
          </div>
        </label>
      ))}
    </div>
  );
}

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
  const [form, setForm] = useState<StrategyForm>(blankStrategyForm);
  const [options, setOptions] = useState<UsageOptions>({ sources: [], models: [] });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [importResult, setImportResult] = useState<PriceQuote | null>(null);
  const [pricingUrl, setPricingUrl] = useState("");
  const [helperAccounts, setHelperAccounts] = useState<{ accountId: string; label: string }[]>([]);
  const [helperAccount, setHelperAccount] = useState("");
  const [helperModel, setHelperModel] = useState("");
  const [helperEnabled, setHelperEnabled] = useState(false);
  const [helperModels, setHelperModels] = useState<string[]>([]);
  const [importing, setImporting] = useState(false);
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
      setImportResult(null);
      setSaved(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const importPrices = async (fromPage = false, assisted = false) => {
    if (!draft.model.trim()) {
      setError("请先选择 Agent 和模型");
      return;
    }
    setBusy(true);
    setImporting(true);
    setError(null);
    setImportResult(null);
    setSaved(false);
    try {
      const target = { agent: draft.agent, model: draft.model.trim() };
      const result = fromPage
        ? await invoke<PriceQuote>("model_price_from_page", {
            input: {
              ...target,
              url: pricingUrl.trim(),
              accountId: assisted ? helperAccount : null,
              parserModel: assisted ? helperModel.trim() : null,
            },
          })
        : await invoke<PriceQuote>("model_prices_import", { target });
      if ((form.peakOn || form.longOn) && result.currency !== draft.currency)
        throw new Error("报价币种与现有分档不同，请先核对并调整分档。");
      setDraft((prev) => ({
        ...prev,
        currency: result.currency,
        input: result.input,
        output: result.output,
        cacheRead: result.cacheRead,
        cacheWrite: result.cacheWrite,
      }));
      setImportResult(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      setImporting(false);
    }
  };
  useEffect(() => {
    invoke<{ accountId: string; label: string }[]>("price_parser_accounts")
      .then(setHelperAccounts)
      .catch(() => {});
  }, []);
  const update = (patch: Partial<StrategyForm>) => {
    setForm((prev) => ({ ...prev, ...patch }));
    setSaved(false);
  };
  const updatePeakRate = (key: RateKey, value: string) => {
    setForm((prev) => ({ ...prev, peakRates: { ...prev.peakRates, [key]: value } }));
    setSaved(false);
  };
  const updateLongRate = (key: RateKey, value: string) => {
    setForm((prev) => ({ ...prev, longRates: { ...prev.longRates, [key]: value } }));
    setSaved(false);
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
          <h2>
            模型成本定价{" "}
            <Hint label="模型定价说明">
              留空表示未知，0
              表示明确免费。缺少用量或价格时只估算已知分量。特定来源规则优先；订阅用量的 API
              等价估算不代表实际账单。
            </Hint>
          </h2>
        </div>
        <button autoFocus onClick={onClose} aria-label="关闭模型定价">
          关闭
        </button>
      </div>
      <form
        noValidate
        onSubmit={(e) => {
          e.preventDefault();
          if (!draft.model.trim()) {
            setError("请填写模型名称");
            return;
          }
          const problem = validateModelPrice(draft, form);
          if (problem) {
            setError(problem);
            return;
          }
          void save(serializeModelPrice(draft, form));
        }}
      >
        <div className="pricing-grid">
          <label>
            Agent
            <Select
              disabled={busy}
              aria-label="定价 Agent"
              value={draft.agent}
              onChange={(e) => {
                setDraft(blank(e.target.value));
                setForm(blankStrategyForm());
                setImportResult(null);
                setSaved(false);
              }}
            >
              {AGENTS.map(([id, label]) => (
                <option key={id} value={id}>
                  {label}
                </option>
              ))}
            </Select>
          </label>
          <label>
            适用来源
            <Select
              disabled={busy}
              aria-label="定价适用来源"
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
            </Select>
          </label>
          <label className="pricing-model">
            模型名称
            <input
              required
              disabled={busy}
              maxLength={200}
              list="price-model-options"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              placeholder="与日志中的模型名完全匹配"
              value={draft.model}
              onChange={(e) => {
                setDraft({
                  ...draft,
                  model: e.target.value,
                  ...(importResult
                    ? { input: null, output: null, cacheRead: null, cacheWrite: null }
                    : {}),
                });
                setImportResult(null);
                setSaved(false);
              }}
            />
            <datalist id="price-model-options">
              {(options.modelsByAgent
                ? (options.modelsByAgent[draft.agent] ?? [])
                : options.models
              ).map((m) => (
                <option key={m} value={m} />
              ))}
            </datalist>
          </label>
          <label>
            币种
            <Select
              aria-label="定价币种"
              value={draft.currency}
              onChange={(e) => setDraft({ ...draft, currency: e.target.value as "USD" | "CNY" })}
            >
              <option value="USD">USD · 美元</option>
              <option value="CNY">CNY · 人民币</option>
            </Select>
          </label>
        </div>
        <div className="pricing-fetch">
          <div className="actions">
            <button
              type="button"
              disabled={busy || !draft.model.trim()}
              onClick={() => void importPrices()}
            >
              {importing ? "获取中…" : "从 AIHOT 导入"}
            </button>
            <span className="small muted">每百万 Token · 获取后核对保存</span>
          </div>
          <details className="pricing-section">
            <summary>从官网链接获取</summary>
            <div className="pricing-url-row">
              <input
                type="url"
                aria-label="模型计费官网链接"
                placeholder="https://… 模型计费官网"
                value={pricingUrl}
                disabled={busy}
                onChange={(e) => setPricingUrl(e.target.value)}
              />
              <button
                type="button"
                disabled={busy || !pricingUrl.trim() || !draft.model.trim()}
                onClick={() => void importPrices(true)}
              >
                程序解析
              </button>
            </div>
            <label className="pricing-enable">
              <input
                type="checkbox"
                checked={helperEnabled}
                onChange={(e) => setHelperEnabled(e.target.checked)}
              />
              可选：模型辅助解析
            </label>
            {helperEnabled && (
              <div className="pricing-helper">
                <Select
                  aria-label="解析用 API 账户"
                  value={helperAccount}
                  disabled={busy}
                  onChange={(e) => {
                    setHelperAccount(e.target.value);
                    setHelperModel("");
                    setHelperModels([]);
                  }}
                >
                  <option value="">选择已有 API 账户</option>
                  {helperAccounts.map((a) => (
                    <option key={a.accountId} value={a.accountId}>
                      {a.label}
                    </option>
                  ))}
                </Select>
                <input
                  aria-label="解析用模型 ID"
                  autoCapitalize="none"
                  autoCorrect="off"
                  spellCheck={false}
                  list="parser-model-options"
                  placeholder="选择或输入解析模型 ID"
                  value={helperModel}
                  disabled={busy}
                  onChange={(e) => setHelperModel(e.target.value)}
                />
                <datalist id="parser-model-options">
                  {helperModels.map((id) => (
                    <option key={id} value={id} />
                  ))}
                </datalist>
                <button
                  type="button"
                  disabled={busy || !helperAccount}
                  onClick={async () => {
                    setBusy(true);
                    setError(null);
                    try {
                      setHelperModels(
                        await invoke<string[]>("price_parser_models", { accountId: helperAccount }),
                      );
                    } catch (e) {
                      setError(String(e));
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  获取账户模型列表
                </button>
                <p className="small muted">
                  点击「用所选模型解析」会发送公开价格页文本，产生一次 API 调用费用。支持
                  DeepSeek、GLM、OpenRouter 的 API Key。
                </p>
                <button
                  type="button"
                  disabled={
                    busy ||
                    !helperAccount ||
                    !helperModel.trim() ||
                    !pricingUrl.trim() ||
                    !draft.model.trim()
                  }
                  onClick={() => void importPrices(true, true)}
                >
                  用所选模型解析
                </button>
              </div>
            )}
          </details>
          {importResult && (
            <div className="pricing-import-result" role="status">
              <strong>
                {importResult.method} · {importResult.model} · 报价已填入，待保存
              </strong>
              <details>
                <summary>来源与核对说明</summary>
                <a href={importResult.sourceUrl} target="_blank" rel="noreferrer">
                  查看计费来源 ↗
                </a>
                {importResult.notes.map((note) => (
                  <p key={note} className="small muted">
                    {note}
                  </p>
                ))}
              </details>
            </div>
          )}
        </div>
        <RateFields
          rates={draft}
          currency={draft.currency}
          onChange={(key, value) => {
            setDraft({ ...draft, [key]: value || null });
            setSaved(false);
          }}
        />

        <details className="pricing-section" open={form.peakOn}>
          <summary>
            分时段价格（高峰）{form.peakOn && <span className="badge">已启用</span>}
          </summary>
          <label className="pricing-enable">
            <input
              type="checkbox"
              checked={form.peakOn}
              onChange={(e) =>
                update({
                  peakOn: e.target.checked,
                  ...(e.target.checked && !form.ranges.length
                    ? { ranges: [...DEFAULT_RANGES] }
                    : {}),
                })
              }
            />
            启用高峰价格
          </label>
          {form.peakOn && (
            <>
              <RateFields
                rates={form.peakRates}
                currency={draft.currency}
                onChange={updatePeakRate}
              />
              <div className="pricing-ranges">
                <span className="small muted">高峰时段 · 北京时间（{PEAK_TIMEZONE}）</span>
                {form.ranges.map((range, i) => (
                  <div className="pricing-range-row" key={i}>
                    <input
                      placeholder="09:00-12:00"
                      value={range}
                      onChange={(e) =>
                        update({
                          ranges: form.ranges.map((r, j) => (j === i ? e.target.value : r)),
                        })
                      }
                    />
                    <button
                      type="button"
                      disabled={form.ranges.length <= 1}
                      onClick={() => update({ ranges: form.ranges.filter((_, j) => j !== i) })}
                    >
                      删除
                    </button>
                  </div>
                ))}
                <button type="button" onClick={() => update({ ranges: [...form.ranges, ""] })}>
                  ＋ 添加时段
                </button>
                <label className="pricing-enable">
                  <input type="checkbox" checked disabled onChange={() => {}} />
                  仅工作日（法定节假日暂不剔除）
                </label>
              </div>
            </>
          )}
        </details>

        <details className="pricing-section" open={form.longOn}>
          <summary>长上下文分档{form.longOn && <span className="badge">已启用</span>}</summary>
          <label className="pricing-enable">
            <input
              type="checkbox"
              checked={form.longOn}
              onChange={(e) => update({ longOn: e.target.checked })}
            />
            启用长上下文分档
          </label>
          {form.longOn && (
            <>
              <label>
                长上下文阈值（Token）
                <input
                  inputMode="numeric"
                  placeholder="272000"
                  value={form.threshold}
                  onChange={(e) => update({ threshold: e.target.value })}
                />
              </label>
              <p className="small muted">超过阈值时整请求按此档计费。</p>
              <RateFields
                rates={form.longRates}
                currency={draft.currency}
                onChange={updateLongRate}
              />
            </>
          )}
        </details>

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
              setForm(blankStrategyForm());
              setImportResult(null);
              setSaved(false);
            }}
          >
            新建规则
          </button>
          <button className="primary" disabled={busy}>
            {busy ? (importing ? "获取中…" : "保存中…") : "保存并用于估算"}
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
              {rule.source && (
                <details className="pricing-provenance">
                  <summary className="badge">AIHOT 导入 · CNY</summary>
                  <p className="small muted">
                    抓取 {new Date(rule.source.retrievedAt).toLocaleString("zh-CN")} · 汇率{" "}
                    {rule.source.fxRate}（{rule.source.fxDate}）<br />
                    <a href={rule.source.sourceUrl} target="_blank" rel="noreferrer">
                      AIHOT 来源
                    </a>{" "}
                    ·{" "}
                    <a href={rule.source.officialUrl} target="_blank" rel="noreferrer">
                      厂商价格
                    </a>
                  </p>
                </details>
              )}
              {(rule.peak || rule.peakWindows) && <span className="badge">分时段</span>}
              {rule.longContext && <span className="badge">长上下文分档</span>}
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
                  setForm(strategyFormOf(rule));
                  setImportResult(null);
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
