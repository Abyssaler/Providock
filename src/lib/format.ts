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
