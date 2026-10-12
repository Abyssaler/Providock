import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { SourceInfo } from "../../contracts/usage";

const AGENT_LABELS: Record<string, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  gemini_cli: "Gemini CLI",
  grok_build: "Grok Build",
  opencode: "OpenCode",
  pi: "Pi",
  mcode: "MiniMax Code",
  kimi_code: "Kimi Code",
  deepseek_harness: "DeepSeek Harness",
  claude_desktop: "Claude Desktop（网关）",
};
const COMPAT_LABELS: Record<string, string> = {
  compatible: "兼容",
  partial: "部分兼容",
  incompatible: "不兼容",
  unknown: "待识别",
};

export default function SourcesPage() {
  const [sources, setSources] = useState<SourceInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showPaths, setShowPaths] = useState(false);

  const refresh = useCallback(() => {
    invoke<SourceInfo[]>("sources_list")
      .then((value) => {
        setSources(value);
        setError(null);
      })
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => refresh(), [refresh]);

  return (
    <section aria-label="数据源页面">
      <div className="page-head">
        <div>
          <h1>数据源</h1>
        </div>
        <label className="small muted">
          <input
            type="checkbox"
            checked={showPaths}
            onChange={(e) => setShowPaths(e.target.checked)}
          />{" "}
          显示路径
        </label>
      </div>

      {error && (
        <div className="card" role="alert">
          <p className="muted">读取失败：{error}</p>
        </div>
      )}

      {sources !== null && sources.length === 0 && !error && (
        <div className="card">
          <p>
            <strong>尚未发现数据源。</strong>
          </p>
          <p className="muted">在用量页同步以发现本机日志。</p>
        </div>
      )}

      {sources !== null && sources.length > 0 && (
        <div className="card">
          <div className="source-table-scroll">
            <table className="tablewrap">
              <thead>
                <tr>
                  <th>Agent</th>
                  <th>解析器</th>
                  <th>状态</th>
                  <th className="number">计量记录</th>
                  <th>最近同步</th>
                  {showPaths && <th>目录</th>}
                </tr>
              </thead>
              <tbody>
                {sources.map((s) => (
                  <tr key={s.sourceId}>
                    <td>{AGENT_LABELS[s.agent] ?? s.agent}</td>
                    <td className="muted small">{s.parserVersion}</td>
                    <td>
                      <span className="badge">
                        {COMPAT_LABELS[s.compatStatus] ?? s.compatStatus}
                      </span>
                    </td>
                    <td className="number num">{s.events}</td>
                    <td className="muted small">
                      {s.lastSyncAt
                        ? new Date(s.lastSyncAt).toLocaleString("zh-CN", {
                            month: "2-digit",
                            day: "2-digit",
                            hour: "2-digit",
                            minute: "2-digit",
                          })
                        : "—"}
                    </td>
                    {showPaths && <td className="small source-path">{s.rootPath}</td>}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </section>
  );
}
