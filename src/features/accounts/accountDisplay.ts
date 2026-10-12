/**
 * 账户卡片纯展示逻辑（无副作用、可单测）。
 *
 * 规则依据 docs/specs/ACCOUNT_ADAPTERS.md 第 6 节与 docs/specs/UI_UX.md 第 5/8 节：
 * - 无官方查询能力 → 隐藏指标，不显示余额数字；
 * - 暂时失败 → 保留旧值并标过期；
 * - 币种不合并；只有比例就只显示比例（绝不反推绝对量）；
 * - 多窗口并排；倒计时归零显示「等待平台刷新」，不本地补满。
 *
 * 金额/比例是契约里的十进制字符串。除进度条宽度外，这里不把额度字符串转成 Number
 * 参与运算：百分比换算用字符串移位，大小比较用十进制字符串比较。
 */

import type {
  AccountSummaryDto,
  BalanceEntry,
  ConfigKind,
  CredentialDto,
  CredentialType,
  Quality,
  QuotaUnit,
  QuotaWindow,
  SnapshotStatus,
} from "../../contracts/accounts";

// ---------------------------------------------------------------------------
// 文案
// ---------------------------------------------------------------------------

export const CONFIG_KIND_LABELS: Record<ConfigKind, string> = {
  api_payg: "API 付费",
  coding_plan: "Coding Plan",
  both: "API 付费 + Coding Plan",
};

export const CREDENTIAL_TYPE_LABELS: Record<CredentialType, string> = {
  api_key: "API Key",
  management_key: "管理密钥",
  oauth_handle: "OAuth 凭据",
  local_server_token: "本机服务令牌",
  ak_sk: "AK/SK",
};

export const BALANCE_KIND_LABELS: Record<BalanceEntry["kind"], string> = {
  available: "可用余额",
  paid: "充值",
  grant: "赠金",
  credit: "授信 / 额度",
};

export const STATUS_LABELS: Record<SnapshotStatus, string> = {
  ok: "正常",
  stale: "快照已过期",
  refreshing: "刷新中",
  auth_required: "需要更新凭据",
  rate_limited: "平台限流",
  unsupported: "暂未接入官方查询能力",
  incompatible: "接口不兼容",
  unavailable: "暂时不可用",
  no_plan: "无 Coding Plan 套餐",
};

export const QUOTA_UNIT_LABELS: Record<QuotaUnit, string> = {
  tokens: "Token",
  requests: "请求",
  credits: "Credits",
  currency: "金额",
  percent: "%",
  vendor_unit: "平台单位",
};

const UNSUPPORTED_TEXT = "暂未接入官方查询能力";

export function unsupportedNote(): string {
  return UNSUPPORTED_TEXT;
}

export function qualityNote(quality: Quality): string | null {
  switch (quality) {
    case "reported":
      return null;
    case "derived":
      return "推导值";
    case "estimated":
      return "估算值";
    case "unknown":
      return "质量未知";
  }
}

// ---------------------------------------------------------------------------
// 十进制字符串工具（不经过浮点运算）
// ---------------------------------------------------------------------------

interface ParsedDecimal {
  neg: boolean;
  int: string;
  frac: string;
}

function parseDecimal(value: string): ParsedDecimal | null {
  const m = /^([+-]?)(\d+)(?:\.(\d*))?$/.exec(value.trim());
  if (!m) return null;
  return {
    neg: m[1] === "-",
    int: m[2].replace(/^0+(?=\d)/, ""),
    frac: (m[3] ?? "").replace(/0+$/, ""),
  };
}

function compareMagnitude(a: ParsedDecimal, b: ParsedDecimal): number {
  if (a.int.length !== b.int.length) return a.int.length < b.int.length ? -1 : 1;
  if (a.int !== b.int) return a.int < b.int ? -1 : 1;
  const width = Math.max(a.frac.length, b.frac.length);
  const fa = a.frac.padEnd(width, "0");
  const fb = b.frac.padEnd(width, "0");
  if (fa === fb) return 0;
  return fa < fb ? -1 : 1;
}

/** 十进制字符串比较；任一侧不是合法十进制时返回 null（不猜）。 */
export function compareDecimal(a: string, b: string): number | null {
  const pa = parseDecimal(a);
  const pb = parseDecimal(b);
  if (!pa || !pb) return null;
  const magnitude = compareMagnitude(pa, pb);
  if (magnitude === 0) return 0;
  if (pa.neg && pb.neg) return -magnitude;
  if (pa.neg) return -1;
  if (pb.neg) return 1;
  return magnitude;
}

/**
 * 比例字符串 → 百分比字符串（`0.42` → `42`，`1.2` → `120`）。
 * 用小数点右移两位实现，不经过浮点乘除，避免精度误差。
 */
export function ratioToPercent(ratio: string | null): string | null {
  if (ratio === null) return null;
  const parsed = parseDecimal(ratio);
  if (!parsed) return null;
  const digits = parsed.int + parsed.frac;
  const pointAt = parsed.int.length + 2;
  const padded = digits.padEnd(pointAt + 1, "0");
  const intPart = padded.slice(0, pointAt).replace(/^0+(?=\d)/, "");
  const fracPart = padded.slice(pointAt).replace(/0+$/, "");
  const body = fracPart.length > 0 ? `${intPart}.${fracPart}` : intPart;
  if (body === "0") return "0";
  return parsed.neg ? `-${body}` : body;
}

/** 进度条宽度（展示用；只在此处把比例转成 0–100 的数值）。 */
export function percentWidth(percent: string | null): number | null {
  if (percent === null) return null;
  const value = Number(percent);
  if (!Number.isFinite(value)) return null;
  return Math.min(100, Math.max(0, value));
}

// ---------------------------------------------------------------------------
// 凭据
// ---------------------------------------------------------------------------

/** 当前（未退休）凭据；旧引用按契约保留行，不参与卡片展示。 */
export function activeCredentials(credentials: CredentialDto[]): CredentialDto[] {
  return credentials.filter((c) => c.retiredAt === null);
}

/** 「API Key · ********mnop」；多个当前凭据只提示数量，不并列明文。 */
export function credentialSummary(credentials: CredentialDto[]): string {
  const active = activeCredentials(credentials);
  if (active.length === 0) return "未保存凭据";
  const first = active[0];
  const label = CREDENTIAL_TYPE_LABELS[first.credentialType];
  const head = first.mask ? `${label} ${first.mask}` : label;
  return active.length === 1 ? head : `${head} 等 ${active.length} 个`;
}

// ---------------------------------------------------------------------------
// 余额
// ---------------------------------------------------------------------------

export interface BalanceGroup {
  currency: string;
  walletScope: string | null;
  /** 主显示行：可用余额或公式声明的总值行；没有总值且多于一条时为 null。 */
  primary: BalanceEntry | null;
  /** 展开项（充值 / 赠金 / 授信等）。 */
  components: BalanceEntry[];
  entries: BalanceEntry[];
}

/** 与 Rust `accounts::formula_declares_total` 一致：公式左边是 total/available 即总值行。 */
function declaresTotal(formula: string | null): boolean {
  if (!formula) return false;
  const lhs = formula.split("=")[0]?.trim().toLowerCase();
  return lhs === "total" || lhs === "total_balance" || lhs === "available";
}

/**
 * 按 (币种, 钱包范围) 分组；**绝不跨币种/单位合并或比较**。
 * 组内优先取总值行作为主显示，其余作为展开项；没有总值行时若只有一条则直接作主值。
 */
export function groupBalances(balances: BalanceEntry[]): BalanceGroup[] {
  const groups = new Map<string, BalanceGroup>();
  for (const entry of balances) {
    const key = `${entry.currency}\u0000${entry.walletScope ?? ""}`;
    let group = groups.get(key);
    if (!group) {
      group = {
        currency: entry.currency,
        walletScope: entry.walletScope,
        primary: null,
        components: [],
        entries: [],
      };
      groups.set(key, group);
    }
    group.entries.push(entry);
  }
  for (const group of groups.values()) {
    const total = group.entries.find((e) => e.kind === "available" || declaresTotal(e.formula));
    if (total) {
      group.primary = total;
      group.components = group.entries.filter((e) => e !== total);
    } else if (group.entries.length === 1) {
      group.primary = group.entries[0];
      group.components = [];
    } else {
      group.primary = null;
      group.components = [...group.entries];
    }
  }
  // 只按币种/钱包范围分组排序；同组内保持平台返回顺序。
  return [...groups.values()].sort(
    (a, b) =>
      a.currency.localeCompare(b.currency) ||
      (a.walletScope ?? "").localeCompare(b.walletScope ?? ""),
  );
}

// ---------------------------------------------------------------------------
// 额度窗口
// ---------------------------------------------------------------------------

/** 窗口顺序：按已知周期长度升序（5 小时 → 7 天），未知长度排最后，再按窗口标识。 */
export function sortWindows(windows: QuotaWindow[]): QuotaWindow[] {
  return [...windows].sort((a, b) => {
    const da = a.durationSeconds;
    const db = b.durationSeconds;
    if (da !== null && db !== null && da !== db) return da - db;
    if (da === null && db !== null) return 1;
    if (da !== null && db === null) return -1;
    return a.windowId.localeCompare(b.windowId);
  });
}

function humanizeDurationToken(segment: string): string | null {
  const m = /^(\d+)([hdwm])$/.exec(segment.toLowerCase());
  if (!m) return null;
  const n = m[1];
  switch (m[2]) {
    case "h":
      return `${n} 小时`;
    case "d":
      return `${n} 天`;
    case "w":
      return `${n} 周`;
    default:
      return `${n} 分钟`;
  }
}

function humanizeSeconds(seconds: number): string | null {
  if (seconds <= 0) return null;
  if (seconds < 3600) return `${Math.max(1, Math.round(seconds / 60))} 分钟`;
  if (seconds < 86400) return `${Math.round(seconds / 3600)} 小时`;
  return `${Math.round(seconds / 86400)} 天`;
}

/** 窗口标题：平台 label 优先，其次从 windowId/周期长度推断，最后回退 windowId。 */
export function windowLabel(window: QuotaWindow): string {
  if (window.label) return window.label;
  for (const segment of window.windowId.split(":")) {
    const human = humanizeDurationToken(segment);
    if (human) return human;
  }
  if (window.durationSeconds !== null) {
    const human = humanizeSeconds(window.durationSeconds);
    if (human) return human;
  }
  return window.windowId;
}

/** `used_ratio` 超 1（或平台显式标 over_limit）即视为超限。 */
export function isOverLimit(window: QuotaWindow): boolean {
  if (window.overLimit === true) return true;
  if (window.usedRatio !== null) {
    const cmp = compareDecimal(window.usedRatio, "1");
    if (cmp !== null && cmp > 0) return true;
  }
  return false;
}

/**
 * 重置倒计时。已到重置时间时显示「等待平台刷新」——倒计时只是展示平台时间，
 * **不**在本地把额度补满。`now` 由调用方传入，便于测试。
 */
export function formatCountdown(resetsAt: string | null, now: Date): string | null {
  if (!resetsAt) return null;
  const target = new Date(resetsAt).getTime();
  if (Number.isNaN(target)) return null;
  const diff = target - now.getTime();
  if (diff <= 0) return "等待平台刷新";
  const totalMinutes = Math.floor(diff / 60_000);
  const days = Math.floor(totalMinutes / 1440);
  const hours = Math.floor((totalMinutes % 1440) / 60);
  const minutes = totalMinutes % 60;
  if (days > 0) return `${days} 天 ${hours} 小时后重置`;
  if (hours > 0) return `${hours} 小时 ${minutes} 分钟后重置`;
  if (minutes > 0) return `${minutes} 分钟后重置`;
  return "不到 1 分钟后重置";
}

export interface WindowDisplay {
  title: string;
  /** 百分比文本：优先剩余比例，只有已用比例时改用已用。 */
  percent: string | null;
  percentKind: "remaining" | "used";
  /** 进度条宽度；无比例时为 null。 */
  barPercent: number | null;
  resetText: string | null;
  overLimit: boolean;
  /** 只有绝对剩余量时回退显示的数值（不反推比例）。 */
  absoluteRemaining: string | null;
  unitLabel: string;
  quality: Quality;
}

/** 单个窗口的展示模型。超限时改显示已用比例，进度条按 100% 封顶。 */
export function windowDisplay(window: QuotaWindow, now: Date): WindowDisplay {
  const remainingPercent = ratioToPercent(window.remainingRatio);
  const usedPercent = ratioToPercent(window.usedRatio);
  const overLimit = isOverLimit(window);
  let percent: string | null = null;
  let percentKind: "remaining" | "used" = "remaining";
  if (overLimit && usedPercent !== null) {
    percent = usedPercent;
    percentKind = "used";
  } else if (remainingPercent !== null) {
    percent = remainingPercent;
    percentKind = "remaining";
  } else if (usedPercent !== null) {
    percent = usedPercent;
    percentKind = "used";
  }
  return {
    title: windowLabel(window),
    percent,
    percentKind,
    barPercent: percentWidth(percent),
    resetText: formatCountdown(window.resetsAt, now),
    overLimit,
    absoluteRemaining: percent === null ? window.remaining : null,
    unitLabel: QUOTA_UNIT_LABELS[window.unit],
    quality: window.quality,
  };
}

// ---------------------------------------------------------------------------
// 时间
// ---------------------------------------------------------------------------

const pad = (n: number): string => String(n).padStart(2, "0");

/** 本地时间 `YYYY-MM-DD HH:mm`；解析失败返回 null。 */
export function formatTime(iso: string | null): string | null {
  if (!iso) return null;
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return null;
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(
    d.getMinutes(),
  )}`;
}

/** 相对时间；未来或解析失败返回 null。 */
export function formatRelative(iso: string | null, now: Date): string | null {
  if (!iso) return null;
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return null;
  const diff = now.getTime() - t;
  if (diff < 0) return null;
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 1) return "刚刚";
  if (minutes < 60) return `${minutes} 分钟前`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} 小时前`;
  return `${Math.floor(hours / 24)} 天前`;
}

// ---------------------------------------------------------------------------
// 账户状态
// ---------------------------------------------------------------------------

export type MetricMode = "unsupported" | "auth" | "values";

/**
 * 指标显示模式。`unsupportedHint` 用于平台无官方查询能力、且后端尚未记录状态
 * （`account_refresh` 直接以「unsupported platform」拒绝）的场景。
 */
export function metricMode(account: AccountSummaryDto, unsupportedHint = false): MetricMode {
  const status = account.latestStatus ?? account.snapshot?.status ?? null;
  if (unsupportedHint || status === "unsupported") return "unsupported";
  if (status === "auth_required") return "auth";
  return "values";
}

export type StatusBucket = "ok" | "stale" | "auth" | "unsupported" | "no_plan" | "failed";

export function statusBucket(account: AccountSummaryDto, unsupportedHint = false): StatusBucket {
  if (metricMode(account, unsupportedHint) === "unsupported") return "unsupported";
  if (account.latestStatus === "auth_required") return "auth";
  if (account.latestStatus === "no_plan") return "no_plan";
  if (account.stale) return "stale";
  const status = account.latestStatus;
  if (status && status !== "ok" && status !== "refreshing") return "failed";
  return "ok";
}

/** 平台图标占位：取别名/平台名首字符（含代理对）。 */
export function accountInitial(account: AccountSummaryDto): string {
  const source = account.displayName.trim() || account.platform.trim();
  const first = Array.from(source)[0];
  return first ? first.toUpperCase() : "?";
}

/** 查询来源徽标文案（evidence 是后端标注的来源，如 `official:v1`）。 */
export function evidenceLabel(evidence: string | null): string | null {
  if (!evidence) return null;
  const lower = evidence.toLowerCase();
  if (lower.includes("not_wired") || lower.includes("unsupported")) return "进程协议未接入";
  if (lower.startsWith("kimi:coding_direct:")) return "官方 Kimi Code 接口";
  if (lower.includes("plugin")) return "官方插件接口";
  if (lower.includes("experimental") || lower.includes("oauth")) return "官方实验接口";
  if (lower.startsWith("official")) return "官方公开接口";
  return "查询来源";
}
