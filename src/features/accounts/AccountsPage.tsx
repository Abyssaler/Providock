export default function AccountsPage() {
  return (
    <section aria-label="账户页面">
      <div className="page-head">
        <h1>账户与额度</h1>
        <p className="muted">API 余额与 Coding Plan 独立管理，无需绑定 Agent。</p>
      </div>
      <div className="card">
        <p>
          <strong>还没有账户。</strong>
        </p>
        <p className="muted small">
          账户模块将在 T09–T11 实现：平台与区域 → 账户类型 → 凭据（仅存系统凭据库）→ 只读验证 →
          快照展示。平台没有官方查询能力时只保留管理字段，不显示虚构余额。
        </p>
      </div>
    </section>
  );
}
