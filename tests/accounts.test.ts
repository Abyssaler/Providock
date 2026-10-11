import { describe, expect, it } from "vitest";
import type {
  AccountSummaryDto,
  BalanceEntry,
  CredentialDto,
  QuotaWindow,
} from "../src/contracts/accounts";
import {
  compareDecimal,
  credentialSummary,
  formatCountdown,
  groupBalances,
  isOverLimit,
  metricMode,
  ratioToPercent,
  sortWindows,
  statusBucket,
  windowDisplay,
  windowLabel,
} from "../src/features/accounts/accountDisplay";

function balance(partial: Partial<BalanceEntry>): BalanceEntry {
  return {
    kind: "available",
    amount: "0",
    currency: "USD",
    walletScope: null,
    expiresAt: null,
    formula: null,
    quality: "reported",
    ...partial,
  };
}

function window_(partial: Partial<QuotaWindow>): QuotaWindow {
  return {
    windowId: "glm:plan:5h:tokens",
    label: null,
    resource: null,
    scope: null,
    unit: "tokens",
    periodKind: null,
    durationSeconds: null,
    startsAt: null,
    resetsAt: null,
    used: null,
    limit: null,
    remaining: null,
    usedRatio: null,
    remainingRatio: null,
    rawRatio: null,
    overLimit: null,
    quality: "reported",
    evidenceRef: null,
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
    ...partial,
  };
}

describe("ratioToPercent：比例换算不经浮点运算", () => {
  it("常见比例", () => {
    expect(ratioToPercent("0.42")).toBe("42");
    expect(ratioToPercent("0.425")).toBe("42.5");
    expect(ratioToPercent("1.2")).toBe("120");
    expect(ratioToPercent("1")).toBe("100");
    expect(ratioToPercent("0")).toBe("0");
    expect(ratioToPercent("-0.05")).toBe("-5");
  });

  it("未知值保持 null，不当作 0", () => {
    expect(ratioToPercent(null)).toBe(null);
    expect(ratioToPercent("abc")).toBe(null);
    expect(ratioToPercent("1e5")).toBe(null);
  });
});

describe("compareDecimal：十进制字符串比较，不经过 Number", () => {
  it("大小与相等", () => {
    expect(compareDecimal("1.02", "1")).toBe(1);
    expect(compareDecimal("1", "1.00")).toBe(0);
    expect(compareDecimal("-1", "0")).toBe(-1);
    expect(compareDecimal("-1.5", "-1.4")).toBe(-1);
  });

  it("超出 2^53 也不失真", () => {
    expect(compareDecimal("9007199254740993123", "9007199254740993122")).toBe(1);
    expect(compareDecimal("9007199254740993123", "9007199254740993123")).toBe(0);
  });

  it("非法输入返回 null", () => {
    expect(compareDecimal("abc", "1")).toBe(null);
    expect(compareDecimal("1", "1.2.3")).toBe(null);
  });
});

describe("isOverLimit：used_ratio 超 1 才显示超限", () => {
  it("按比例判断", () => {
    expect(isOverLimit(window_({ usedRatio: "1.02" }))).toBe(true);
    expect(isOverLimit(window_({ usedRatio: "1" }))).toBe(false);
    expect(isOverLimit(window_({ usedRatio: "0.99" }))).toBe(false);
  });

  it("平台显式 over_limit 优先", () => {
    expect(isOverLimit(window_({ overLimit: true }))).toBe(true);
    expect(isOverLimit(window_({ overLimit: false, usedRatio: "1.5" }))).toBe(true);
  });
});

describe("formatCountdown：倒计时归零显示等待平台刷新", () => {
  const now = new Date("2026-10-11T10:00:00Z");

  it("按剩余时间分档", () => {
    expect(formatCountdown("2026-10-11T12:00:00Z", now)).toBe("2 小时 0 分钟后重置");
    expect(formatCountdown("2026-10-13T16:00:00Z", now)).toBe("2 天 6 小时后重置");
    expect(formatCountdown("2026-10-11T10:30:00Z", now)).toBe("30 分钟后重置");
  });

  it("已到重置时间不本地补满", () => {
    expect(formatCountdown("2026-10-11T10:00:00Z", now)).toBe("等待平台刷新");
    expect(formatCountdown("2026-10-11T09:00:00Z", now)).toBe("等待平台刷新");
  });

  it("没有重置时间或无法解析时返回 null", () => {
    expect(formatCountdown(null, now)).toBe(null);
    expect(formatCountdown("not-a-date", now)).toBe(null);
  });
});

describe("groupBalances：币种不合并，可用余额为主", () => {
  it("按币种分行，同币种内取总值行", () => {
    const groups = groupBalances([
      balance({ kind: "available", amount: "110.00", currency: "CNY" }),
      balance({ kind: "grant", amount: "10.00", currency: "CNY" }),
      balance({ kind: "paid", amount: "100.00", currency: "CNY" }),
      balance({ kind: "available", amount: "5.00", currency: "USD" }),
    ]);
    expect(groups.map((g) => g.currency)).toEqual(["CNY", "USD"]);

    const cny = groups[0];
    expect(cny.primary?.amount).toBe("110.00");
    expect(cny.components.map((c) => c.kind)).toEqual(["grant", "paid"]);

    const usd = groups[1];
    expect(usd.primary?.amount).toBe("5.00");
    expect(usd.components).toEqual([]);
    // 没有跨币种相加：每组只含该币种条目
    expect(cny.entries.every((e) => e.currency === "CNY")).toBe(true);
  });

  it("公式声明的总值行优先于组件，不重复相加", () => {
    const [group] = groupBalances([
      balance({
        kind: "paid",
        amount: "8.00",
        formula: "total = granted + topped_up",
      }),
      balance({ kind: "grant", amount: "2.00" }),
    ]);
    expect(group.primary?.amount).toBe("8.00");
    expect(group.components.map((c) => c.kind)).toEqual(["grant"]);
  });

  it("负余额原样显示", () => {
    const [group] = groupBalances([balance({ amount: "-3.50" })]);
    expect(group.primary?.amount).toBe("-3.50");
  });

  it("没有总值行且有多条时不臆造总额", () => {
    const [group] = groupBalances([
      balance({ kind: "grant", amount: "2.00" }),
      balance({ kind: "credit", amount: "300" }),
    ]);
    expect(group.currency).toBe("USD");
    expect(group.primary).toBe(null);
    expect(group.components.map((c) => c.kind)).toEqual(["grant", "credit"]);
  });
});

describe("sortWindows：多窗口并排按周期长度", () => {
  it("5 小时排在 7 天前，未知长度最后", () => {
    const sorted = sortWindows([
      window_({ windowId: "b:7d", durationSeconds: 604800 }),
      window_({ windowId: "c:unknown" }),
      window_({ windowId: "a:5h", durationSeconds: 18000 }),
    ]);
    expect(sorted.map((w) => w.windowId)).toEqual(["a:5h", "b:7d", "c:unknown"]);
  });
});

describe("windowLabel：窗口标题", () => {
  it("平台 label 优先，其次从 windowId 推断", () => {
    expect(windowLabel(window_({ label: "月度 Credits" }))).toBe("月度 Credits");
    expect(windowLabel(window_({ windowId: "glm:plan:5h:tokens" }))).toBe("5 小时");
    expect(windowLabel(window_({ windowId: "kimi:coding_plan:7d:requests" }))).toBe("7 天");
    expect(windowLabel(window_({ windowId: "custom", durationSeconds: 86400 }))).toBe("1 天");
    expect(windowLabel(window_({ windowId: "custom" }))).toBe("custom");
  });
});

describe("windowDisplay：只有比例就只显示比例", () => {
  const now = new Date("2026-10-11T10:00:00Z");

  it("剩余比例 + 重置时间", () => {
    const display = windowDisplay(
      window_({
        label: "5 小时",
        usedRatio: "0.58",
        remainingRatio: "0.42",
        resetsAt: "2026-10-11T12:00:00Z",
      }),
      now,
    );
    expect(display.title).toBe("5 小时");
    expect(display.percent).toBe("42");
    expect(display.percentKind).toBe("remaining");
    expect(display.barPercent).toBe(42);
    expect(display.resetText).toBe("2 小时 0 分钟后重置");
    expect(display.overLimit).toBe(false);
  });

  it("只有已用比例时改显示已用，不反推绝对量", () => {
    const display = windowDisplay(window_({ label: "7 天", usedRatio: "0.25" }), now);
    expect(display.percent).toBe("25");
    expect(display.percentKind).toBe("used");
    expect(display.absoluteRemaining).toBe(null);
  });

  it("超限时改显示已用比例，进度条封顶", () => {
    const display = windowDisplay(window_({ usedRatio: "1.2", remainingRatio: "-0.2" }), now);
    expect(display.overLimit).toBe(true);
    expect(display.percent).toBe("120");
    expect(display.percentKind).toBe("used");
    expect(display.barPercent).toBe(100);
  });

  it("只有绝对剩余量时回退显示数值", () => {
    const display = windowDisplay(window_({ unit: "requests", remaining: "42" }), now);
    expect(display.percent).toBe(null);
    expect(display.absoluteRemaining).toBe("42");
    expect(display.unitLabel).toBe("请求");
  });
});

describe("credentialSummary：掩码展示，退休凭据不参与", () => {
  const credentials: CredentialDto[] = [
    {
      credentialId: "c1",
      credentialType: "api_key",
      mask: "********mnop",
      fingerprint: "f",
      capabilities: null,
      createdAt: "2026-10-11T09:00:00Z",
      retiredAt: null,
    },
    {
      credentialId: "c0",
      credentialType: "api_key",
      mask: "********old",
      fingerprint: "f",
      capabilities: null,
      createdAt: "2026-10-10T09:00:00Z",
      retiredAt: "2026-10-11T09:00:00Z",
    },
  ];

  it("只显示当前凭据掩码", () => {
    expect(credentialSummary(credentials)).toBe("API Key ********mnop");
    expect(credentialSummary([])).toBe("未保存凭据");
  });
});

describe("metricMode / statusBucket：无能力与需更新凭据", () => {
  it("unsupported 隐藏指标", () => {
    const acc = account({ latestStatus: "unsupported" });
    expect(metricMode(acc)).toBe("unsupported");
    expect(metricMode(acc, true)).toBe("unsupported");
    expect(statusBucket(acc)).toBe("unsupported");
  });

  it("auth_required 走更新凭据引导", () => {
    const acc = account({ latestStatus: "auth_required" });
    expect(metricMode(acc)).toBe("auth");
    expect(statusBucket(acc)).toBe("auth");
  });

  it("失败后保留旧值记为过期", () => {
    const acc = account({ latestStatus: "unavailable", stale: true });
    expect(metricMode(acc)).toBe("values");
    expect(statusBucket(acc)).toBe("stale");
  });

  it("还没有查询状态时正常", () => {
    const acc = account({});
    expect(metricMode(acc)).toBe("values");
    expect(statusBucket(acc)).toBe("ok");
    expect(statusBucket(acc, true)).toBe("unsupported");
  });
});
