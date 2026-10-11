export default function UsagePage() {
  return (
    <section aria-label="用量页面">
      <div className="page-head">
        <h1>每一份用量，都有来源</h1>
        <p className="muted">查看本机 Agent 消耗，追踪输入、输出与缓存。</p>
      </div>
      <div className="card">
        <p>
          <strong>尚未接入数据源。</strong>
        </p>
        <p className="muted small">
          采集管线（解析 → 标准化 → 去重 → 账本 → 聚合）将在 T02–T07 实现。
          本页不会展示全零仪表盘或演示数据；完成采集后，摘要卡将显示总 Token、输入、输出、
          读缓存占比（含覆盖率）与计量记录数，费用按“已报告 / 估算”分列。
        </p>
      </div>
    </section>
  );
}
