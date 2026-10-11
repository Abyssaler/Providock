//! 十进制字符串的精确加法（金额/单位合计用）。
//! 不用二进制浮点；按小数位对齐后以 i128 整数相加，保留源精度。

/// `a + b`，两者均为合法十进制（可选负号）。非法输入返回 None。
pub fn decimal_add(a: &str, b: &str) -> Option<String> {
    let (sa, ia, fa) = split(a)?;
    let (sb, ib, fb) = split(b)?;
    let scale = fa.len().max(fb.len());
    let va = to_scaled(sa, ia, fa, scale)?;
    let vb = to_scaled(sb, ib, fb, scale)?;
    let sum = va.checked_add(vb)?;
    Some(from_scaled(sum, scale))
}

/// 比例 num/den → 0..1 的 4 位小数字符串（截断不四舍五入）。分母为 0 返回 None。
pub fn ratio_decimal(num: u128, den: u128) -> Option<String> {
    if den == 0 {
        return None;
    }
    let scaled = num.checked_mul(10000)? / den;
    Some(from_scaled(i128::try_from(scaled).ok()?, 4))
}

/// (负号, 整数段, 小数段)
fn split(s: &str) -> Option<(bool, &str, &str)> {
    let (neg, t) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let mut parts = t.splitn(2, '.');
    let int_part = parts.next().unwrap_or("");
    let frac = parts.next().unwrap_or("");
    if int_part.is_empty() && frac.is_empty() {
        return None;
    }
    if !int_part.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((neg, if int_part.is_empty() { "0" } else { int_part }, frac))
}

fn to_scaled(neg: bool, int_part: &str, frac: &str, scale: usize) -> Option<i128> {
    let mut digits = String::with_capacity(int_part.len() + scale);
    digits.push_str(int_part);
    digits.push_str(frac);
    for _ in frac.len()..scale {
        digits.push('0');
    }
    let v: i128 = digits.parse().ok()?;
    Some(if neg { -v } else { v })
}

fn from_scaled(v: i128, scale: usize) -> String {
    let neg = v < 0;
    let digits = v.unsigned_abs().to_string();
    let padded = if digits.len() <= scale {
        format!("{}{}", "0".repeat(scale + 1 - digits.len()), digits)
    } else {
        digits
    };
    let (int_part, frac) = padded.split_at(padded.len() - scale);
    // 去掉多余的尾零，但保留至少整数
    let frac = frac.trim_end_matches('0');
    let body = if frac.is_empty() {
        int_part.trim_start_matches('0').to_string()
    } else {
        format!("{}.{}", int_part.trim_start_matches('0'), frac)
    };
    let body = if body.is_empty() || body.starts_with('.') {
        format!("0{body}")
    } else {
        body
    };
    if neg && body != "0" {
        format!("-{body}")
    } else {
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_without_float() {
        assert_eq!(decimal_add("0.1", "0.2"), Some("0.3".into()));
        assert_eq!(decimal_add("10.50", "0.05"), Some("10.55".into()));
        assert_eq!(decimal_add("-3", "1.5"), Some("-1.5".into()));
        assert_eq!(decimal_add("1.005", "2"), Some("3.005".into()));
        assert_eq!(decimal_add("0", "-0.00"), Some("0".into()));
        // 超过 f64 精度也安全
        assert_eq!(
            decimal_add("9007199254740993.5", "0.5"),
            Some("9007199254740994".into())
        );
        assert_eq!(decimal_add("abc", "1"), None);
    }

    #[test]
    fn ratio_is_truncated_decimal() {
        assert_eq!(ratio_decimal(800, 2000), Some("0.4".into()));
        assert_eq!(ratio_decimal(1, 3), Some("0.3333".into()));
        assert_eq!(ratio_decimal(5, 0), None);
    }
}
