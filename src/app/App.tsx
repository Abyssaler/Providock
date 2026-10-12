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
            {appInfo ? `本地优先 · v${appInfo.version}` : "浏览器预览模式"}
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
        <AppUpdates version={appInfo?.version} settings={page === "settings"} />
        <PageBoundary key={page}>
          {page === "usage" && <UsagePage />}
          {page === "accounts" && <AccountsPage />}
          {page === "sessions" && <SessionsPage />}
          {page === "sources" && <SourcesPage />}
          {page === "settings" && (
            <section aria-label="采集设置">
              <div className="page-head">
                <h1>自动监测设置</h1>
                <p className="muted">Agent 用量与平台账户分别调度，关闭自动任务不影响手动刷新。</p>
              </div>
              <MonitoringControls />
              <AgentVisibilitySettings />
              <div className="card">
                <h2>桌面运行</h2>
                <p className="muted">
                  macOS 关闭窗口后继续在菜单栏运行，自动任务遵循上方开关。点击菜单栏图标可查看最近
                  24 小时用量、同步或重新打开窗口；使用 ⌘Q 或菜单栏「退出 Providock」结束程序。
                </p>
              </div>
            </section>
          )}
        </PageBoundary>
        <footer className="foot">
          <span>Providock · 本地用量与账户管理</span>
          <span>本机记录 ≠ 平台全部消费 · 未提供 ≠ 0</span>
        </footer>
      </main>
    </div>
  );
}
