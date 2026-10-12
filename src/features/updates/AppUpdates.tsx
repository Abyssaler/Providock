import { useEffect, useState } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { AUTO_CHECK_KEY, CHECK_INTERVAL_MS, UpdateController } from "./updateController";
import "./updates.css";

function readAutoCheck(): boolean {
  try {
    return localStorage.getItem(AUTO_CHECK_KEY) !== "false";
  } catch {
    return true;
  }
}

export default function AppUpdates({ version, settings }: { version?: string; settings: boolean }) {
  const [controller, setController] = useState<UpdateController | null>(null);
  const [state, setState] = useState(() => new UpdateController({ check, relaunch }).state);
  const [autoCheck, setAutoCheck] = useState(readAutoCheck);
  const [dismissed, setDismissed] = useState<string | null>(null);

  useEffect(() => {
    if (!version) return;
    const client = new UpdateController({ check: () => check({ timeout: 15000 }), relaunch });
    setController(client);
    const stop = client.subscribe(setState);
    return () => {
      stop();
      client.dispose();
    };
  }, [version]);

  useEffect(() => {
    if (!controller || !autoCheck) return;
    void controller.check();
    const timer = window.setInterval(() => void controller.check(), CHECK_INTERVAL_MS);
    const onVisible = () => {
      if (!document.hidden && Date.now() - (controller.state.checkedAt ?? 0) >= CHECK_INTERVAL_MS)
        void controller.check();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [autoCheck, controller]);

  const changeAutoCheck = (enabled: boolean) => {
    setAutoCheck(enabled);
    try {
      localStorage.setItem(AUTO_CHECK_KEY, String(enabled));
    } catch {
      // 本机存储被禁用时仍允许本次会话切换。
    }
  };
  const busy = ["checking", "downloading", "installing"].includes(state.phase);
  const status =
    state.phase === "checking"
      ? "正在检查 GitHub Release…"
      : state.phase === "downloading"
        ? `正在下载更新${state.progress === null ? "…" : ` · ${Math.floor(state.progress)}%`}`
        : state.phase === "installing"
          ? "正在校验并安装更新…"
          : state.phase === "installed"
            ? "更新已安装，正在重启…"
            : state.version
              ? `发现新版本 v${state.version}`
              : state.checkedAt && !state.error
                ? "当前已是最新版本"
                : "从 GitHub 正式 Release 获取更新";
  const action =
    state.phase === "installed" ? (
      <button className="primary" onClick={() => void controller?.restart()}>
        重启完成更新
      </button>
    ) : state.version ? (
      <button className="primary" disabled={busy} onClick={() => void controller?.install()}>
        下载、安装并重启
      </button>
    ) : null;
  const showBanner = state.version && dismissed !== state.version && !settings;

  return (
    <>
      {showBanner && (
        <aside className="update-banner" aria-label="版本更新通知" aria-live="polite">
          <div>
            <strong>{status}</strong>
            <span className="small muted">更新保留本机数据与设置。</span>
          </div>
          <div className="actions">
            {action}
            <button disabled={busy} onClick={() => setDismissed(state.version)}>
              稍后
            </button>
          </div>
          {state.error && <p role="alert">{state.error}</p>}
        </aside>
      )}
      {settings && (
        <section className="card app-updates" aria-label="软件更新">
          <h2>软件更新{version ? ` · v${version}` : ""}</h2>
          <label title="启动及每 6 小时检查；发现新版后由你点击下载与安装">
            <input
              type="checkbox"
              checked={autoCheck}
              disabled={!version}
              onChange={(event) => changeAutoCheck(event.target.checked)}
            />
            自动检查正式版更新
          </label>

          <p role="status">{version ? status : "浏览器预览不支持桌面更新。"}</p>
          <div className="actions">
            <button
              disabled={!controller || busy || state.phase === "installed"}
              onClick={() => void controller?.check()}
            >
              检查更新
            </button>
            {action}
          </div>
          {state.error && <p role="alert">{state.error}</p>}
          {state.notes && (
            <details>
              <summary>新版说明</summary>
              <pre>{state.notes}</pre>
            </details>
          )}
        </section>
      )}
    </>
  );
}
