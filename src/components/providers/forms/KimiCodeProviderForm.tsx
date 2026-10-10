import { useEffect, useState } from "react";
import { z } from "zod";
import { useForm } from "react-hook-form";
import { useTranslation } from "react-i18next";
import { Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Form } from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import JsonEditor from "@/components/JsonEditor";
import { Switch } from "@/components/ui/switch";
import { configApi } from "@/lib/api";
import { useDarkMode } from "@/hooks/useDarkMode";
import type { ProviderFormData } from "@/lib/schemas/provider";
import type { CustomEndpoint } from "@/types";
import {
  KIMI_CODE_CAPABILITIES,
  KIMI_CODE_DEFAULT_CONFIG,
  KIMI_CODE_PROVIDER_TYPES,
  kimiCodeProviderPresets,
  type KimiCodeModelConfig,
  type KimiCodeProviderType,
  type KimiCodeSettingsConfig,
} from "@/config/kimiCodeProviderPresets";
import { BasicFormFields } from "./BasicFormFields";
import { ProviderPresetSelector } from "./ProviderPresetSelector";
import { ApiKeySection, EndpointField } from "./shared";
import EndpointSpeedTest from "./EndpointSpeedTest";
import { useSpeedTestEndpoints } from "./hooks";
import type { ProviderFormProps } from "./ProviderForm";

const PROVIDER_TYPE_LABELS: Record<KimiCodeProviderType, string> = {
  kimi: "Kimi",
  anthropic: "Anthropic",
  openai: "OpenAI",
  openai_responses: "OpenAI Responses",
  "google-genai": "Google GenAI",
  vertexai: "Vertex AI",
};

const PRESET_ENTRIES = kimiCodeProviderPresets.map((preset, index) => ({
  id: String(index),
  preset,
}));

const modelSchema = z
  .object({
    model: z.string(),
    max_context_size: z.number().int().positive(),
    display_name: z.string().optional(),
    capabilities: z.array(z.string()).optional(),
    support_efforts: z.array(z.string()).optional(),
    default_effort: z.string().optional(),
  })
  .passthrough();

const configSchema = z
  .object({
    type: z.enum(KIMI_CODE_PROVIDER_TYPES),
    base_url: z.string().optional(),
    api_key: z.string().optional(),
    api_key_env: z.string().optional(),
    models: z.record(z.string(), modelSchema),
  })
  .passthrough();

type ModelRow = {
  alias: string;
  value: KimiCodeModelConfig;
};

const toRows = (models: Record<string, KimiCodeModelConfig>): ModelRow[] =>
  Object.entries(models ?? {}).map(([alias, value]) => ({ alias, value }));

const toModels = (rows: ModelRow[]): Record<string, KimiCodeModelConfig> => {
  const models: Record<string, KimiCodeModelConfig> = {};
  for (const row of rows) {
    const alias = row.alias.trim();
    if (!alias) continue;
    models[alias] = row.value;
  }
  return models;
};

/** 序列化前清理：去掉空的可选字段，保留未知的透传字段 */
const sanitizeModel = (value: KimiCodeModelConfig): KimiCodeModelConfig => {
  const next: KimiCodeModelConfig & Record<string, unknown> = {
    ...value,
    model: value.model.trim(),
    max_context_size: Math.trunc(value.max_context_size),
  };
  if (!next.display_name?.trim()) delete next.display_name;
  if (!next.capabilities?.length) delete next.capabilities;
  if (!next.support_efforts?.length) delete next.support_efforts;
  if (!next.default_effort?.trim()) delete next.default_effort;
  return next;
};

export function KimiCodeProviderForm({
  providerId,
  initialData,
  onSubmit,
  onCancel,
  submitLabel,
  showButtons = true,
  onSubmittingChange,
  onSubmitReadyChange,
}: ProviderFormProps) {
  const { t } = useTranslation();
  const isDarkMode = useDarkMode();
  const [config, setConfig] = useState<KimiCodeSettingsConfig>(() => ({
    ...KIMI_CODE_DEFAULT_CONFIG,
    ...((initialData?.settingsConfig ?? {}) as Partial<KimiCodeSettingsConfig>),
    models:
      (initialData?.settingsConfig as KimiCodeSettingsConfig | undefined)
        ?.models ?? {},
  }));
  const [jsonText, setJsonText] = useState(JSON.stringify(config, null, 2));
  const [jsonValid, setJsonValid] = useState(true);
  const [presetId, setPresetId] = useState("custom");
  const preset =
    presetId === "custom"
      ? undefined
      : kimiCodeProviderPresets[Number(presetId)];
  const category = initialData?.category ?? preset?.category ?? "custom";
  const [modelRows, setModelRows] = useState<ModelRow[]>(() =>
    toRows(config.models),
  );
  const [isEndpointModalOpen, setIsEndpointModalOpen] = useState(false);
  const [endpointAutoSelect, setEndpointAutoSelect] = useState<boolean>(
    () => initialData?.meta?.endpointAutoSelect ?? true,
  );
  const [draftCustomEndpoints, setDraftCustomEndpoints] = useState<string[]>(
    () => (initialData ? [] : []),
  );
  // 通用配置片段（原始 TOML，写入 live 配置时由后端合并）
  const [useCommonConfig, setUseCommonConfig] = useState<boolean>(
    () => initialData?.meta?.commonConfigEnabled ?? false,
  );
  const [commonConfigSnippet, setCommonConfigSnippet] = useState("");
  const [commonConfigError, setCommonConfigError] = useState("");
  useEffect(() => {
    let active = true;
    configApi
      .getCommonConfigSnippet("kimicode")
      .then((snippet) => {
        if (active && snippet) setCommonConfigSnippet(snippet);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, []);
  const persistCommonConfigSnippet = (snippet: string) => {
    setCommonConfigSnippet(snippet);
    configApi
      .setCommonConfigSnippet("kimicode", snippet)
      .then(() => setCommonConfigError(""))
      .catch((err) => setCommonConfigError(String(err)));
  };
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const form = useForm<ProviderFormData>({
    defaultValues: {
      name: initialData?.name ?? "",
      notes: initialData?.notes ?? "",
      websiteUrl: initialData?.websiteUrl ?? "",
      icon: initialData?.icon ?? "",
      iconColor: initialData?.iconColor ?? "",
      settingsConfig: jsonText,
    },
  });
  const name = form.watch("name");
  const modelsReady =
    modelRows.length > 0 &&
    modelRows.every(
      (row) =>
        row.alias.trim() &&
        row.value.model.trim() &&
        Number.isInteger(row.value.max_context_size) &&
        row.value.max_context_size > 0,
    );
  const ready = Boolean(
    jsonValid &&
      name.trim() &&
      (config.api_key?.trim() || config.api_key_env?.trim()) &&
      modelsReady,
  );
  useEffect(() => {
    onSubmitReadyChange?.(ready);
  }, [ready, onSubmitReadyChange]);

  const update = (next: KimiCodeSettingsConfig) => {
    setConfig(next);
    setJsonText(JSON.stringify(next, null, 2));
    setJsonValid(true);
  };

  const updateRows = (rows: ModelRow[]) => {
    setModelRows(rows);
    update({ ...config, models: toModels(rows) });
  };

  const updateRow = (index: number, patch: Partial<ModelRow["value"]>) => {
    const rows = modelRows.map((row, i) =>
      i === index ? { ...row, value: { ...row.value, ...patch } } : row,
    );
    updateRows(rows);
  };

  const renameRow = (index: number, alias: string) => {
    const rows = modelRows.map((row, i) =>
      i === index ? { ...row, alias } : row,
    );
    updateRows(rows);
  };

  const addRow = () => {
    setModelRows([
      ...modelRows,
      { alias: "", value: { model: "", max_context_size: 200000 } },
    ]);
  };

  const removeRow = (index: number) => {
    updateRows(modelRows.filter((_, i) => i !== index));
  };

  const toggleCapability = (index: number, capability: string) => {
    const current = modelRows[index]?.value.capabilities ?? [];
    const next = current.includes(capability)
      ? current.filter((item) => item !== capability)
      : [...current, capability];
    updateRow(index, { capabilities: next });
  };

  const choosePreset = (id: string) => {
    setPresetId(id);
    const selected =
      id === "custom" ? undefined : kimiCodeProviderPresets[Number(id)];
    const next: KimiCodeSettingsConfig = selected
      ? JSON.parse(JSON.stringify(selected.settingsConfig))
      : { ...KIMI_CODE_DEFAULT_CONFIG, models: {} };
    update(next);
    setModelRows(toRows(next.models));
    form.reset({
      name: selected?.name ?? "",
      notes: "",
      websiteUrl: selected?.websiteUrl ?? "",
      icon: selected?.icon ?? "",
      iconColor: selected?.iconColor ?? "",
      settingsConfig: JSON.stringify(next),
    });
  };

  const speedTestEndpoints = useSpeedTestEndpoints({
    appId: "kimicode",
    selectedPresetId: presetId === "custom" ? null : presetId,
    presetEntries: PRESET_ENTRIES,
    baseUrl: config.base_url ?? "",
    codexBaseUrl: "",
    initialData,
  });

  return (
    <Form {...form}>
      <form
        id="provider-form"
        className="space-y-6 glass rounded-xl p-6 border border-white/10"
        onSubmit={form.handleSubmit(async (identity) => {
          if (!ready || busy) return;
          setBusy(true);
          onSubmittingChange?.(true);
          setError("");
          try {
            const models = Object.fromEntries(
              Object.entries(toModels(modelRows)).map(([alias, value]) => [
                alias,
                sanitizeModel(value),
              ]),
            );
            const customEndpoints: Record<string, CustomEndpoint> = {
              ...(initialData?.meta?.custom_endpoints ?? {}),
            };
            for (const url of draftCustomEndpoints) {
              if (!customEndpoints[url]) {
                customEndpoints[url] = { url, addedAt: Date.now() };
              }
            }
            await onSubmit({
              ...identity,
              name: identity.name.trim(),
              meta: {
                ...initialData?.meta,
                commonConfigEnabled: useCommonConfig,
                endpointAutoSelect,
                ...(Object.keys(customEndpoints).length
                  ? { custom_endpoints: customEndpoints }
                  : {}),
              },
              providerKey: providerId,
              presetId: presetId === "custom" ? undefined : presetId,
              presetCategory: category,
              settingsConfig: JSON.stringify({ ...config, models }, null, 2),
            });
          } catch (error) {
            setError(String(error));
          } finally {
            setBusy(false);
            onSubmittingChange?.(false);
          }
        })}
      >
        {!initialData && (
          <ProviderPresetSelector
            selectedPresetId={presetId}
            presetEntries={PRESET_ENTRIES}
            presetCategoryLabels={{
              custom: t("providerPreset.custom"),
              official: t("providerForm.categoryOfficial"),
              cn_official: t("providerForm.categoryCnOfficial"),
              aggregator: t("providerForm.categoryAggregation"),
              third_party: t("providerForm.categoryThirdParty"),
            }}
            onPresetChange={choosePreset}
            category={category}
          />
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        <fieldset
          disabled={busy || !jsonValid}
          className="min-w-0 space-y-6 border-0 p-0 disabled:opacity-50"
        >
          <BasicFormFields form={form} />

          <div className="space-y-2">
            <Label>{t("provider.form.kimicode.providerType")}</Label>
            <Select
              value={config.type}
              onValueChange={(type) =>
                update({ ...config, type: type as KimiCodeProviderType })
              }
            >
              <SelectTrigger
                aria-label={t("provider.form.kimicode.providerType")}
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {KIMI_CODE_PROVIDER_TYPES.map((type) => (
                  <SelectItem key={type} value={type}>
                    {PROVIDER_TYPE_LABELS[type]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <ApiKeySection
            label="API Key"
            value={config.api_key ?? ""}
            onChange={(api_key) => update({ ...config, api_key })}
            category={category}
            shouldShowLink={Boolean(preset?.apiKeyUrl)}
            websiteUrl={preset?.apiKeyUrl ?? ""}
            isPartner={preset?.isPartner}
            partnerPromotionKey={preset?.partnerPromotionKey}
          />

          <div className="space-y-2">
            <Label htmlFor="kimicode-api-key-env">
              {t("provider.form.kimicode.apiKeyEnv")}
            </Label>
            <Input
              id="kimicode-api-key-env"
              value={config.api_key_env ?? ""}
              onChange={(e) =>
                update({
                  ...config,
                  api_key_env: e.target.value.trim() || undefined,
                })
              }
              placeholder="KIMI_API_KEY"
            />
            <p className="text-xs text-muted-foreground">
              {t("provider.form.kimicode.apiKeyEnvHint")}
            </p>
          </div>

          <EndpointField
            id="kimicode-base-url"
            label={t("providerForm.apiEndpoint")}
            value={config.base_url ?? ""}
            onChange={(base_url) => update({ ...config, base_url })}
            placeholder="https://api.moonshot.ai/v1"
            onManageClick={() => setIsEndpointModalOpen(true)}
          />

          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <Label>{t("provider.form.kimicode.models")}</Label>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={addRow}
              >
                <Plus className="h-4 w-4 mr-1" />
                {t("provider.form.kimicode.addModel")}
              </Button>
            </div>
            {modelRows.map((row, index) => (
              <div
                key={index}
                className="space-y-3 rounded-lg border border-border-default p-3"
              >
                <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                  <div className="space-y-1">
                    <Label>{t("provider.form.kimicode.modelAlias")}</Label>
                    <Input
                      value={row.alias}
                      onChange={(event) => renameRow(index, event.target.value)}
                      placeholder="kimi-for-coding"
                    />
                  </div>
                  <div className="space-y-1">
                    <Label>{t("provider.form.kimicode.modelId")}</Label>
                    <Input
                      value={row.value.model}
                      onChange={(event) =>
                        updateRow(index, { model: event.target.value })
                      }
                      placeholder="kimi-for-coding"
                    />
                  </div>
                  <div className="space-y-1">
                    <Label>{t("provider.form.kimicode.maxContextSize")}</Label>
                    <Input
                      type="number"
                      min={1}
                      step={1}
                      value={String(row.value.max_context_size ?? "")}
                      onChange={(event) =>
                        updateRow(index, {
                          max_context_size: Number(event.target.value),
                        })
                      }
                      placeholder="262144"
                    />
                  </div>
                  <div className="space-y-1">
                    <Label>{t("provider.form.kimicode.displayName")}</Label>
                    <Input
                      value={row.value.display_name ?? ""}
                      onChange={(event) =>
                        updateRow(index, { display_name: event.target.value })
                      }
                    />
                  </div>
                </div>
                <div className="space-y-1">
                  <Label>{t("provider.form.kimicode.capabilities")}</Label>
                  <div className="flex flex-wrap gap-x-4 gap-y-2">
                    {KIMI_CODE_CAPABILITIES.map((capability) => (
                      <label
                        key={capability}
                        className="flex items-center gap-1.5 text-xs text-muted-foreground"
                      >
                        <Checkbox
                          checked={Boolean(
                            row.value.capabilities?.includes(capability),
                          )}
                          onCheckedChange={() =>
                            toggleCapability(index, capability)
                          }
                          aria-label={capability}
                        />
                        {t(`provider.form.kimicode.capability.${capability}`, {
                          defaultValue: capability,
                        })}
                      </label>
                    ))}
                  </div>
                </div>
                <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                  <div className="space-y-1">
                    <Label>{t("provider.form.kimicode.supportEfforts")}</Label>
                    <Input
                      value={(row.value.support_efforts ?? []).join(", ")}
                      onChange={(event) =>
                        updateRow(index, {
                          support_efforts: event.target.value
                            .split(",")
                            .map((item) => item.trim())
                            .filter(Boolean),
                        })
                      }
                      placeholder="low, medium, high"
                    />
                  </div>
                  <div className="space-y-1">
                    <Label>{t("provider.form.kimicode.defaultEffort")}</Label>
                    <Input
                      value={row.value.default_effort ?? ""}
                      onChange={(event) =>
                        updateRow(index, { default_effort: event.target.value })
                      }
                    />
                  </div>
                </div>
                <div className="flex justify-end">
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => removeRow(index)}
                    aria-label={t("provider.form.kimicode.removeModel")}
                  >
                    <X className="h-4 w-4 mr-1" />
                    {t("provider.form.kimicode.removeModel")}
                  </Button>
                </div>
              </div>
            ))}
          </div>
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <Switch
                id="kimicode-common-config"
                checked={useCommonConfig}
                onCheckedChange={setUseCommonConfig}
              />
              <Label htmlFor="kimicode-common-config">
                {t("provider.form.kimicode.commonConfig")}
              </Label>
            </div>
            {useCommonConfig && (
              <div className="space-y-1">
                <p className="text-xs text-muted-foreground">
                  {t("provider.form.kimicode.commonConfigHint")}
                </p>
                <textarea
                  className="w-full min-h-24 rounded-lg border border-border-default bg-background px-3 py-2 font-mono text-xs text-foreground focus:outline-none focus:ring-2 focus:ring-blue-500/20"
                  value={commonConfigSnippet}
                  onChange={(event) =>
                    persistCommonConfigSnippet(event.target.value)
                  }
                  placeholder={'# config.toml 片段\n# theme = "dark"'}
                  aria-label={t("provider.form.kimicode.commonConfig")}
                />
                {commonConfigError && (
                  <p role="alert" className="text-xs text-destructive">
                    {commonConfigError}
                  </p>
                )}
              </div>
            )}
          </div>
        </fieldset>

        {isEndpointModalOpen && (
          <EndpointSpeedTest
            appId="kimicode"
            providerId={providerId}
            value={config.base_url ?? ""}
            onChange={(base_url) => update({ ...config, base_url })}
            initialEndpoints={speedTestEndpoints}
            visible={isEndpointModalOpen}
            onClose={() => setIsEndpointModalOpen(false)}
            autoSelect={endpointAutoSelect}
            onAutoSelectChange={setEndpointAutoSelect}
            onCustomEndpointsChange={
              initialData ? undefined : setDraftCustomEndpoints
            }
          />
        )}

        <div className="space-y-2">
          <Label htmlFor="kimicode-settings-config">
            {t("provider.configJson")}
          </Label>
          <JsonEditor
            id="kimicode-settings-config"
            ariaLabel={t("provider.configJson")}
            value={jsonText}
            darkMode={isDarkMode}
            language="json"
            showValidation
            height={Math.max(1, jsonText.split("\n").length) * 20 + 20}
            onChange={(text) => {
              setJsonText(text);
              try {
                const parsed = configSchema.parse(JSON.parse(text));
                const next: KimiCodeSettingsConfig = {
                  base_url: "",
                  api_key: "",
                  ...parsed,
                };
                setConfig(next);
                setModelRows(toRows(next.models));
                setJsonValid(true);
              } catch {
                setJsonValid(false);
              }
            }}
          />
        </div>
        {showButtons && (
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="outline"
              onClick={onCancel}
              disabled={busy}
            >
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={busy || !ready}>
              {submitLabel}
            </Button>
          </div>
        )}
      </form>
    </Form>
  );
}
