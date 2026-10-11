import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import UsagePage from "../features/usage/UsagePage";
import AccountsPage from "../features/accounts/AccountsPage";
import SessionsPage from "../features/sessions/SessionsPage";
import SourcesPage from "../features/sources/SourcesPage";
import "./app.css";

type PageId = "usage" | "accounts" | "sessions" | "sources";

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
  const [theme, setTheme] = useState<"light" | "dark">("light");
  const [appInfo, setAppInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    document.documentElement.dataset.theme = theme === "dark" ? "dark" : "";
  }, [theme]);

  useEffect(() => {
    // 纯浏览器预览（pnpm dev）时没有 Tauri 运行时，属正常情况
    invoke<AppInfo>("app_info")
      .then(setAppInfo)
      .catch(() => setAppInfo(null));
  }, []);

  return (
    <div>
      <header className="appbar">
        <div className="brand">
          <span className="mark" aria-hidden="true">
            P
          </span>
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
            aria-label="切换深浅主题"
            onClick={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
          >
            ◐
          </button>
          <button disabled title="设置将在后续任务中实现">
            设置
          </button>
        </div>
      </header>
      <main>
        {page === "usage" && <UsagePage />}
        {page === "accounts" && <AccountsPage />}
        {page === "sessions" && <SessionsPage />}
        {page === "sources" && <SourcesPage />}
        <footer className="foot">
          <span>Providock · 工程骨架（T01）</span>
          <span>本机记录 ≠ 平台全部消费 · 未提供 ≠ 0</span>
        </footer>
      </main>
    </div>
  );
}
