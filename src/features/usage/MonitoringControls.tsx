import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { MonitoringSettings } from "../../contracts/usage";

export default function MonitoringControls({
  onChange,
}: {
  onChange?: (settings: MonitoringSettings) => void;
}) {
  const [settings, setSettings] = useState<MonitoringSettings | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let active = true;
    invoke<MonitoringSettings>("monitoring_settings")
      .then((value) => {
        if (active) {
          setSettings(value);
          onChange?.(value);
        }
      })
      .catch(() => {
        if (active) setError("采集设置仅在桌面应用中可用。");
      });
    return () => {
      active = false;
    };
  }, [onChange]);
  const update = async (
    key: "agentAutoSyncEnabled" | "accountAutoRefreshEnabled",
    enabled: boolean,
  ) => {
    setSaving(true);
    setError(null);
    try {
      const next = await invoke<MonitoringSettings>("monitoring_update", {
        patch: { [key]: enabled },
      });
      setSettings(next);
      onChange?.(next);
    } catch {
      setError("保存失败，原设置已保留，请重试。");
    } finally {
      setSaving(false);
    }
  };
  return (
    <div className="monitoring-controls">
      <label className="monitor-row">
        <span>
          <strong>Agent 自动同步</strong>
          <small>
            启动时同步，之后每 {settings?.agentIntervalSeconds ?? 60}{" "}
            秒读取本机日志；关闭后仍可手动同步。
          </small>
        </span>
        <input
          type="checkbox"
          role="switch"
          aria-label="Agent 自动同步"
          checked={settings?.agentAutoSyncEnabled ?? false}
          disabled={!settings || saving}
          onChange={(e) => void update("agentAutoSyncEnabled", e.target.checked)}
        />
      </label>
      <label className="monitor-row">
        <span>
          <strong>账户自动刷新</strong>
          <small>
            余额每 {Math.round((settings?.balanceIntervalSeconds ?? 900) / 60)} 分钟，计划额度每{" "}
            {Math.round((settings?.planIntervalSeconds ?? 300) / 60)}{" "}
            分钟；按账户到期刷新，含少量时间抖动。
          </small>
        </span>
        <input
          type="checkbox"
          role="switch"
          aria-label="账户自动刷新"
          checked={settings?.accountAutoRefreshEnabled ?? false}
          disabled={!settings || saving}
          onChange={(e) => void update("accountAutoRefreshEnabled", e.target.checked)}
        />
      </label>
      <p className="muted small">
        设置保存到本机，重启后继续生效。关闭不会中断正在执行的一次读取或请求，后续任务停止。
      </p>
      {error && (
        <p role="alert" className="monitor-error">
          {error}
        </p>
      )}
    </div>
  );
}
