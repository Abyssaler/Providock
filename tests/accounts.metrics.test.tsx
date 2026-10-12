import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import type { AccountSummaryDto, Snapshot } from "../src/contracts/accounts";
import AccountMetrics from "../src/features/accounts/AccountMetrics";

/**
 * 指标区展示规则（ACCOUNT_ADAPTERS 第 6 节）的渲染验证：
 * 无能力隐藏指标、过期保留旧值、只有比例只显示比例、401 引导更新凭据、币种分行。
 */

const NOW = new Date("2026-10-11T10:00:00Z");

function snapshot(partial: Partial<Snapshot>): Snapshot {
  return {
    snapshotId: 1,
    accountId: "acc-1",
    credentialId: null,
    queriedAt: "2026-10-11T09:30:00Z",
    providerAsOf: null,
    status: "ok",
    evidence: "official:v1",
    errorCode: null,
    expiresAt: null,
    balances: [],
    quotaWindows: [],
    usageMetrics: [],
    ...partial,
  };
}

function account(partial: Partial<AccountSummaryDto>): AccountSummaryDto {
  return {
    accountId: "acc-1",
    platform: "deepseek",
    region: "cn",
    displayName: "示例账户",
    configKind: "api_payg",
    createdAt: "2026-10-11T09:00:00Z",
    credentials: [],
    snapshot: null,
    stale: false,
    latestStatus: null,
    credentialReady: true,
    ...partial,
  };
}

function text(html: string): string {
  return html.replace(/<[^>]*>/g, "");
}

function render(acc: AccountSummaryDto, mode: "unsupported" | "auth" | "values"): string {
  return renderToStaticMarkup(
    <AccountMetrics account={acc} mode={mode} now={NOW} onRotateCredential={() => {}} />,
  );
}

describe("AccountMetrics 展示规则", () => {
  it("无官方查询能力时不显示余额数字", () => {
    const html = render(
      account({
        snapshot: snapshot({
          balances: [
            {
              kind: "available",
              amount: "110.00",
              currency: "CNY",
              walletScope: null,
              expiresAt: null,
              formula: null,
              quality: "reported",
            },
          ],
        }),
      }),
      "unsupported",
    );
    expect(text(html)).toContain("暂未接入官方查询能力");
    expect(text(html)).not.toContain("110.00");
    expect(text(html)).not.toContain("可用余额");
  });

  it("过期保留旧值并标快照已过期", () => {
    const html = render(
      account({
        stale: true,
        latestStatus: "unavailable",
        snapshot: snapshot({
          balances: [
            {
              kind: "available",
              amount: "18.00",
              currency: "CNY",
              walletScope: null,
              expiresAt: null,
              formula: null,
              quality: "reported",
            },
          ],
        }),
      }),
      "values",
    );
    expect(text(html)).toContain("18.00");
    expect(text(html)).toContain("快照已过期");
    expect(text(html)).toContain("原快照时间");
  });

  it("401/权限不足引导更新凭据且不显示余额数字", () => {
    const html = render(
      account({ latestStatus: "auth_required", stale: true, snapshot: null }),
      "auth",
    );
    const body = text(html);
    expect(body).toContain("需要更新凭据");
    expect(body).toContain("不显示余额数字");
    expect(body).not.toContain("可用余额");
  });

  it("多窗口并排显示比例与重置时间，币种分行不合并", () => {
    const html = render(
      account({
        configKind: "both",
        snapshot: snapshot({
          balances: [
            {
              kind: "available",
              amount: "110.00",
              currency: "CNY",
              walletScope: null,
              expiresAt: null,
              formula: null,
              quality: "reported",
            },
            {
              kind: "available",
              amount: "5.00",
              currency: "USD",
              walletScope: null,
              expiresAt: null,
              formula: null,
              quality: "reported",
            },
          ],
          quotaWindows: [
            {
              windowId: "glm:plan:5h:tokens",
              label: "5 小时",
              resource: null,
              scope: null,
              unit: "tokens",
              periodKind: null,
              durationSeconds: 18000,
              startsAt: null,
              resetsAt: "2026-10-11T12:00:00Z",
              used: null,
              limit: null,
              remaining: null,
              usedRatio: "0.58",
              remainingRatio: "0.42",
              rawRatio: null,
              overLimit: null,
              quality: "reported",
              evidenceRef: null,
            },
          ],
        }),
      }),
      "values",
    );
    const body = text(html);
    expect(body).toContain("5 小时剩余 42% · 2 小时 0 分钟后重置");
    expect(body).toContain("110.00");
    expect(body).toContain("CNY");
    expect(body).toContain("5.00");
    expect(body).toContain("USD");
  });
});
