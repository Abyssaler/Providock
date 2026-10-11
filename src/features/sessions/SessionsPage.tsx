export default function SessionsPage() {
  return (
    <section aria-label="会话页面">
      <div className="page-head">
        <h1>会话</h1>
        <p className="muted">统计先呈现；对话正文由你选择是否查看。</p>
      </div>
      <div className="card">
        <p>
          <strong>暂无会话索引。</strong>
        </p>
        <p className="muted small">
          会话列表与可选正文查看将在 T13 完成。默认只保存统计元数据，正文按需从源文件读取，
          不复制进应用数据库。
        </p>
      </div>
    </section>
  );
}
