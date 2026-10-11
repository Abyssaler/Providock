export default function SourcesPage() {
  return (
    <section aria-label="数据源页面">
      <div className="page-head">
        <h1>数据源</h1>
        <p className="muted">每个来源都显示格式、覆盖范围和最近同步。</p>
      </div>
      <div className="card">
        <p>
          <strong>尚未发现数据源。</strong>
        </p>
        <p className="muted small">
          目录发现与兼容状态将在 T04–T08 落地：Claude Code、Codex、Gemini CLI、Grok Build、
          OpenCode、Pi、MiniMax Code、Kimi Code，以及默认关闭的可选采集网关（仅覆盖经过它的 Claude
          Desktop 模型映射流量）。
        </p>
      </div>
    </section>
  );
}
