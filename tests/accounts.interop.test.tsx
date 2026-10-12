import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import fixture from "../fixtures/interop/account-summary.json";
import type { AccountSummaryDto } from "../src/contracts/accounts";
import AccountMetrics from "../src/features/accounts/AccountMetrics";
import { GroupTable } from "../src/features/usage/UsageBreakdown";

describe("Rust IPC 与页面互通回归", () => {
  it("真实后端契约的已存快照可显示余额、窗口和指标", () => {
    const html = renderToStaticMarkup(
      <AccountMetrics
        account={fixture as AccountSummaryDto}
        mode="values"
        now={new Date("2026-10-12T01:00:00Z")}
        onRotateCredential={() => {}}
      />,
    );
    expect(html).toContain("18");
    expect(html).toContain("CNY");
    expect(html).toContain("42%");
    expect(html).toContain("123");
  });
  it("模型表保留未知字段和未定价，并显示完整估算覆盖", () => {
    const html = renderToStaticMarkup(
      <GroupTable
        models
        onPrice={() => {}}
        groups={[
          {
            name: "synthetic-model",
            agent: "codex",
            events: 2,
            tokens: {
              inputFresh: "1000",
              inputCacheRead: null,
              inputCacheWrite: null,
              inputTotal: null,
              outputTotal: "100",
              total: null,
            },
            currentCost: [],
            pricedEvents: 0,
            completeCostEvents: 0,
          },
        ]}
      />,
    );
    expect(html).toContain("synthetic-model");
    expect(html).toContain("Codex");
    expect(html).toContain("未提供");
    expect(html).toContain("未定价");
    expect(html).toContain("0/2");
    expect(html).toContain("设置价格");
  });
});
