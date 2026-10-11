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
  claude_desktop: "Claude Desktop（网关）",
};

export default function SourcesPage() {
  const [sources, setSources] = useState<SourceInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showPaths, setShowPaths] = useState(false);

  const refresh = useCallback(() => {
    invoke<SourceInfo[]>("sources_list")
      .then(setSources)
      .catch((e) => setError(String(e)));
  }, []);

  useEffect(() => refresh(), [refresh]);

  return (
    <section aria-label="数据源页面">
      <div className="page-head">
        <div>
          <h1>数据源</h1>
          <p className="muted">每个来源都显示格式、覆盖范围和最近同步。</p>
        </div>
        <label className="small muted">
          <input
            type="checkbox"
            checked={showPaths}
            onChange={(e) => setShowPaths(e.target.checked)}
          />{" "}
          显示目录路径（默认隐藏）
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
          <p className="muted small">
            到「用量」页点击「同步记录」触发目录发现。当前发现规则：Kimi Code（KIMI_CODE_HOME 或
            ~/.kimi-code）、Codex（CODEX_HOME 或 ~/.codex）；其余六个 Agent
            的接入已在开发中。自定义目录添加与暂停开关在后续任务提供。
          </p>
        </div>
      )}

      {sources !== null && sources.length > 0 && (
        <div className="card">
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
                    <span className="badge">{s.compatStatus}</span>
                  </td>
                  <td className="number num">{s.events}</td>
                  <td className="muted small">{s.lastSyncAt ?? "—"}</td>
                  {showPaths && <td className="small">{s.rootPath}</td>}
                </tr>
              ))}
            </tbody>
          </table>
          <p className="muted small" style={{ marginTop: 12 }}>
            重新扫描是幂等操作，不会重复计数；停用/删除源在后续任务提供。网关默认关闭。
          </p>
        </div>
      )}
    </section>
  );
}
