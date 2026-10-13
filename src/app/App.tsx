import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import MonitoringControls from "../features/usage/MonitoringControls";
import AgentVisibilitySettings from "../features/usage/AgentVisibilitySettings";
import UsagePage from "../features/usage/UsagePage";
import AccountsPage from "../features/accounts/AccountsPage";
import SessionsPage from "../features/sessions/SessionsPage";
import SourcesPage from "../features/sources/SourcesPage";
import PageBoundary from "./PageBoundary";
import AppUpdates from "../features/updates/AppUpdates";
import Hint from "../components/Hint";
import "./app.css";

type PageId = "usage" | "accounts" | "sessions" | "sources" | "settings";

const PAGES: { id: PageId; label: string }[] = [
  { id: "usage", label: "用量" },
  { id: "accounts", label: "账户" },
  { id: "sessions", label: "会话" },
  { id: "sources", label: "数据源" },
];

interface AppInfo {
  version: string;
  identifier: string;
}

export default function App() {
  const [page, setPage] = useState<PageId>("usage");
  const [settingsTab, setSettingsTab] = useState("monitoring");
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    // 纯浏览器预览（pnpm dev）时没有 Tauri 运行时，属正常情况
    invoke<AppInfo>("app_info")
      .then(setAppInfo)
      .catch(() => setAppInfo(null));
  }, []);

  useEffect(() => {
    // macOS 隐藏式标题栏：红绿灯悬浮在顶栏上，顶栏整体可拖拽并给红绿灯让位
    if (appInfo && /Mac OS X/.test(navigator.userAgent)) {
      document.documentElement.classList.add("mac-overlay");
      return () => document.documentElement.classList.remove("mac-overlay");
    }
  }, [appInfo]);

  return (
    <div>
      <header className="appbar" data-tauri-drag-region>
        <div className="brand">
          <svg className="mark" viewBox="0 0 64 64" aria-hidden="true">
            <path
              d="M17 48V15h18a13 13 0 0 1 0 26H27"
              fill="none"
              stroke="currentColor"
              strokeWidth="7"
              strokeLinecap="round"
            />
            <path d="M28 49h22" stroke="#d4ff5f" strokeWidth="7" strokeLinecap="round" />
            <circle cx="50" cy="14" r="4" fill="#d4ff5f" />
          </svg>
          Providock
        </div>
        <nav aria-label="主导航">
          {PAGES.map((p) => (
            <button
              key={p.id}
              className={page === p.id ? "active" : ""}
              aria-pressed={page === p.id}
              onClick={() => setPage(p.id)}
            >
              {p.label}
            </button>
          ))}
        </nav>
        <div className="tools">
          <span
            className="status small muted"
            title={appInfo ? `v${appInfo.version}` : "浏览器预览"}
          >
            <i className="dot" aria-hidden="true" />
            {appInfo ? `v${appInfo.version}` : "预览"}
          </span>
          <button
            onClick={() => setPage("settings")}
            className={page === "settings" ? "active" : ""}
          >
            设置
          </button>
        </div>
      </header>
      <main>
        {page === "settings" && (
          <div className="page-head">
            <h1>设置</h1>
          </div>
        )}
        <div className={page === "settings" ? "settings-shell" : undefined}>
          {page === "settings" && (
            <nav className="settings-nav" aria-label="设置分组">
              {[
                ["monitoring", "采集与刷新"],
                ["display", "Agent 显示"],
                ["updates", "软件更新"],
                ["about", "关于与隐私"],
              ].map(([id, label]) => (
                <button
                  key={id}
                  className={settingsTab === id ? "active" : ""}
                  aria-pressed={settingsTab === id}
                  onClick={() => setSettingsTab(id)}
                >
                  {label}
                </button>
              ))}
            </nav>
          )}
          <div className={page === "settings" ? "settings-content" : undefined}>
            <AppUpdates
              version={appInfo?.version}
              settings={page === "settings" && settingsTab === "updates"}
            />
            <PageBoundary key={page}>
              {page === "usage" && <UsagePage />}
              {page === "accounts" && <AccountsPage />}
              {page === "sessions" && <SessionsPage />}
              {page === "sources" && <SourcesPage />}
              {page === "settings" && settingsTab === "monitoring" && (
                <section className="card">
                  <h2>
                    采集与刷新{" "}
                    <Hint>
                      两类自动任务独立控制。更改立即生效并保存到本机；关闭后仍可手动刷新。
                    </Hint>
                  </h2>
                  <MonitoringControls />
                </section>
              )}
              {page === "settings" && settingsTab === "display" && <AgentVisibilitySettings />}
              {page === "settings" && settingsTab === "about" && (
                <section className="card settings-about">
                  <h2>
                    Providock{" "}
                    <span className="muted small">
                      {appInfo ? `v${appInfo.version}` : "开发预览"}
                    </span>
                  </h2>
                  <p>本地用量与账户管理</p>
                  <dl>
                    <div>
                      <dt>运行方式</dt>
                      <dd>关闭窗口后驻留菜单栏或托盘；右击托盘选择退出。macOS 也可 ⌘Q 退出。</dd>
                    </div>
                    <div>
                      <dt>采集</dt>
                      <dd>只读 Agent 日志；默认不保存对话正文。</dd>
                    </div>
                    <div>
                      <dt>凭据</dt>
                      <dd>API Key 明文保存在独立本机文件，受本地文件权限保护。</dd>
                    </div>
                    <div>
                      <dt>价格解析</dt>
                      <dd>程序优先；模型辅助仅在你主动点击后发送公开价格页内容。</dd>
                    </div>
                  </dl>
                </section>
              )}
            </PageBoundary>
          </div>
        </div>
      </main>
    </div>
  );
}
