/**
 * 显示格式化工具。注意：统计计算全部在 Rust 侧完成（docs/ARCHITECTURE.md 第 6 节），
 * 前端只做展示格式化。十进制字符串是契约传输格式，不能转 Number 做运算。
 */

/** 把十进制整数字符串格式化为紧凑显示（如 1.2M）；null 表示源未提供。 */
export function formatTokenCompact(value: string | null): string {
  if (value === null) return "—";
  if (!/^\d+$/.test(value)) return "—";
  const digits = value.replace(/^0+(?=\d)/, "");
  const len = digits.length;
  if (len <= 3) return digits;
  if (len <= 6) return `${trimDecimal(digits.slice(0, -3), digits.slice(-3, -2))}K`;
  if (len <= 9) return `${trimDecimal(digits.slice(0, -6), digits.slice(-6, -5))}M`;
  return `${trimDecimal(digits.slice(0, -9), digits.slice(-9, -8))}B`;
}

function trimDecimal(intPart: string, firstFracDigit: string): string {
  return firstFracDigit === "0" ? intPart : `${intPart}.${firstFracDigit}`;
}

/** 千分位完整值，用于详情与 tooltip；不做四舍五入。 */
export function formatTokenFull(value: string | null): string {
  if (value === null || !/^\d+$/.test(value)) return "未提供";
  return value.replace(/^0+(?=\d)/, "").replace(/\B(?=(\d{3})+(?!\d))/g, ",");
}

/** Round decimal text for display only; preserve exact backend amounts, including >2^53. */
export function formatMoneyAmount(value: string): string {
  const match = /^(-?)(\d+)(?:\.(\d+))?$/.exec(value);
  if (!match) return "—";
  const fraction = (match[3] ?? "").padEnd(3, "0");
  const cents =
    BigInt(match[2]) * 100n + BigInt(fraction.slice(0, 2)) + (fraction[2] >= "5" ? 1n : 0n);
  return `${match[1] && cents ? "-" : ""}${cents / 100n}.${(cents % 100n).toString().padStart(2, "0")}`;
}
