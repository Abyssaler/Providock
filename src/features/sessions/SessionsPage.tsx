import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type {
  SessionFilter,
  SessionItem,
  SessionsPage as SessionsPageDto,
} from "../../contracts/sessions";
import { formatTokenCompact, formatTokenFull } from "../../lib/format";
import { agentLabel } from "./contentView";
import SessionDrawer from "./SessionDrawer";
import "./sessions.css";

type Period = "all" | "today" | "7d" | "30d";

const PAGE_LIMIT = 50;

const PERIODS: { id: Period; label: string }[] = [
  { id: "all", label: "全部时间" },
  { id: "today", label: "今天" },
  { id: "7d", label: "最近 7 天" },
  { id: "30d", label: "最近 30 天" },
];

const AGENT_OPTIONS = [
  "claude_code",
  "codex",
  "gemini_cli",
  "grok_build",
  "opencode",
  "pi",
  "mcode",
  "kimi_code",
];

function periodRange(period: Period): { from?: string; to?: string } {
  if (period === "all") return {};
  const now = new Date();
  const from = new Date(now);
  if (period === "today") {
    from.setUTCHours(0, 0, 0, 0);
  } else {
    from.setUTCDate(from.getUTCDate() - (period === "7d" ? 7 : 30));
  }
  return { from: from.toISOString(), to: now.toISOString() };
}

export default function SessionsPage() {
  const [period, setPeriod] = useState<Period>("all");
  const [agent, setAgent] = useState("");
  const [queryInput, setQueryInput] = useState("");
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<SessionItem[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<SessionItem | null>(null);
  const [hidePaths, setHidePaths] = useState(false);

  const triggerRef = useRef<HTMLButtonElement | null>(null);

  const fetchPage = useCallback(
    (cursor: string | null, append: boolean) => {
      const filter: SessionFilter = {
        agent: agent || undefined,
        query: query || undefined,
        ...periodRange(period),
      };
      setLoading(true);
      setError(null);
      invoke<SessionsPageDto>("sessions_page", { filter, cursor, limit: PAGE_LIMIT })
        .then((page) => {
          setItems((prev) => (append ? [...prev, ...page.items] : page.items));
          setNextCursor(page.nextCursor);
          setHasMore(page.hasMore);
        })
        .catch((e) => setError(String(e)))
        .finally(() => setLoading(false));
    },
    [agent, query, period],
  );

  useEffect(() => {
    fetchPage(null, false);
  }, [fetchPage]);

  const closeDrawer = useCallback(() => {
    setSelected(null);
    triggerRef.current?.focus();
  }, []);

  const submitSearch = (event: React.FormEvent) => {
    event.preventDefault();
    setQuery(queryInput.trim());
  };

  const resetFilters = () => {
    setAgent("");
    setPeriod("all");
    setQueryInput("");
    setQuery("");
  };

  const filtered = agent !== "" || query !== "" || period !== "all";

  return (
    <section aria-label="会话页面">
      <div className="page-head">
        <div>
          <h1>会话</h1>
          <p className="muted">统计先呈现；对话正文由你选择是否按需查看。</p>
        </div>
        <label className="small muted">
          <input
            type="checkbox"
            checked={hidePaths}
            onChange={(e) => setHidePaths(e.target.checked)}
          />{" "}
          隐私显示（隐藏标题与项目路径）
        </label>
      </div>

      <form className="card sess-filters" onSubmit={submitSearch} aria-label="会话筛选">
        <label>
          <span className="muted small">搜索元数据</span>
          <input
            type="search"
            value={queryInput}
            placeholder="会话 ID 或标题（不搜索正文）"
            onChange={(e) => setQueryInput(e.target.value)}
          />
        </label>
        <label>
          <span className="muted small">Agent</span>
          <select value={agent} onChange={(e) => setAgent(e.target.value)}>
            <option value="">全部 Agent</option>
            {AGENT_OPTIONS.map((id) => (
              <option key={id} value={id}>
                {agentLabel(id)}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span className="muted small">最后活跃</span>
          <select value={period} onChange={(e) => setPeriod(e.target.value as Period)}>
            {PERIODS.map((p) => (
              <option key={p.id} value={p.id}>
                {p.label}
              </option>
            ))}
          </select>
        </label>
        <div className="actions">
          <button type="submit">搜索</button>
          <button type="button" disabled={!filtered} onClick={resetFilters}>
            重置
          </button>
        </div>
      </form>

      <p className="muted small" style={{ marginTop: 10 }}>
        搜索只查索引元数据（会话 ID 与标题），不在后台扫描完整正文；时间按 UTC 半开区间筛选。
      </p>

      {error && (
        <div className="card" role="alert">
          <p className="muted">读取失败：{error}</p>
        </div>
      )}

      {loading && items.length === 0 && !error && (
        <div className="card">
          <p className="muted">读取中…</p>
        </div>
      )}

      {!loading && !error && items.length === 0 && (
        <div className="card">
          <p>
            <strong>{filtered ? "所选范围内没有会话。" : "暂无会话索引。"}</strong>
          </p>
          <p className="muted small">
            {filtered
              ? "可重置筛选，或到「用量」页点击「同步记录」重新扫描本机日志。"
              : "先在「用量」页点击「同步记录」扫描本机 Agent 日志。发现会话但没有可计量记录时，这里仍会列出会话并显示“—”，未知不会当作 0。"}
          </p>
        </div>
      )}

      {items.length > 0 && (
        <div className="card sess-list" role="list">
          {items.map((item) => (
            <button
              type="button"
              role="listitem"
              className="sess-row"
              key={item.sessionKey}
              onClick={(event) => {
                triggerRef.current = event.currentTarget;
                setSelected(item);
              }}
            >
              <span className="sess-row-main">
                <strong>
                  {hidePaths && item.title ? "（标题已隐藏）" : (item.title ?? "（源未提供标题）")}
                </strong>
                <span className="muted small">
                  {agentLabel(item.agent)} · {item.sessionId}
                </span>
                <span className="sess-tags">
                  {item.hasParent && <span className="badge">子会话</span>}
                  {item.hasSubagents && <span className="badge">含子 Agent</span>}
                  {item.archived && <span className="badge">已归档</span>}
                  {item.hasParent && item.parentSessionId && (
                    <span className="muted small">父：{item.parentSessionId}</span>
                  )}
                </span>
              </span>
              <span className="sess-row-metrics">
                <span className="num" title={formatTokenFull(item.tokenTotal)}>
                  {formatTokenCompact(item.tokenTotal)}
                </span>
                <span className="muted small">
                  Token 小计 · 已知 {item.tokenKnownEvents}/{item.eventCount} 条
                </span>
                <span className="muted small">{item.lastActiveAt ?? "时间未知"}</span>
              </span>
            </button>
          ))}
        </div>
      )}

      {hasMore && (
        <div className="actions" style={{ marginTop: 14 }}>
          <button disabled={loading} onClick={() => fetchPage(nextCursor, true)}>
            {loading ? "加载中…" : "加载更多"}
          </button>
        </div>
      )}

      {selected && <SessionDrawer session={selected} hidePaths={hidePaths} onClose={closeDrawer} />}
    </section>
  );
}
