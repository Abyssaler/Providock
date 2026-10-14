//! HTML5 table normalization. Scripts never execute; spans are expanded before price parsing.
use super::{
    price_import::PriceQuote,
    pricing::{ContextTier, PeakWindows, RateSet},
};
use scraper::{ElementRef, Html, Node, Selector};

fn compact(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn walk(el: ElementRef<'_>, out: &mut String, in_cell: bool) {
    let name = el.value().name();
    if ["script", "style", "head", "noscript", "nav", "aside", "sup"].contains(&name) {
        return;
    }
    if name == "table" {
        out.push_str("\n[TABLE]\n");
        let selector = Selector::parse("tr").unwrap();
        let mut pending: Vec<(usize, String)> = vec![];
        for row in el.select(&selector).take(500) {
            let mut cells = vec![];
            let mut col = 0;
            for cell in row
                .child_elements()
                .filter(|e| matches!(e.value().name(), "td" | "th"))
            {
                while pending.get(col).is_some_and(|p| p.0 > 0) {
                    cells.push(pending[col].1.clone());
                    pending[col].0 -= 1;
                    col += 1;
                }
                let mut value = String::new();
                walk(cell, &mut value, true);
                let value = compact(&value).replace('|', "/");
                let span = |attr| {
                    cell.attr(attr)
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or(1)
                        .clamp(1, 32)
                };
                for _ in 0..span("colspan") {
                    if col >= pending.len() {
                        pending.push((0, String::new()));
                    }
                    cells.push(value.clone());
                    pending[col] = (span("rowspan") - 1, value.clone());
                    col += 1;
                }
            }
            while col < pending.len() {
                if pending[col].0 > 0 {
                    cells.push(pending[col].1.clone());
                    pending[col].0 -= 1;
                } else {
                    cells.push(String::new());
                }
                col += 1;
            }
            out.push_str(&cells.join("|"));
            out.push('\n');
        }
        out.push_str("[/TABLE]\n");
        return;
    }
    let block = [
        "tr", "p", "div", "h1", "h2", "h3", "h4", "br", "li", "section", "details",
    ]
    .contains(&name);
    if block {
        out.push(if in_cell { ' ' } else { '\n' });
    }
    for child in el.children() {
        match child.value() {
            Node::Text(t) => out.push_str(t),
            Node::Element(_) => {
                if let Some(e) = ElementRef::wrap(child) {
                    walk(e, out, in_cell)
                }
            }
            _ => {}
        }
    }
    if block {
        out.push(if in_cell { ' ' } else { '\n' });
    }
}
pub fn visible_text(html: &str) -> String {
    let doc = Html::parse_document(html);
    let mut out = String::new();
    walk(doc.root_element(), &mut out, false);
    out.lines()
        .map(compact)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
fn key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
pub(super) fn model_key(s: &str) -> String {
    s.rsplit('/')
        .next()
        .unwrap_or(s)
        .trim()
        .to_lowercase()
        .replace('_', "-")
}
fn matches_model(cell: &str, model: &str) -> bool {
    cell.split([' ', '、', ',', '（', '(', '\n'])
        .any(|s| model_key(s) == model_key(model))
}
fn kind(cell: &str) -> Option<usize> {
    let c = key(cell);
    if c.contains("小时")
        || c.contains("hour")
        || c.contains("存储")
        || c.contains("模态")
        || c.contains("modality")
        || c.contains("长度")
        || c.contains("length")
    {
        return None;
    }
    if c.contains("cachewrite")
        || c.contains("cachecreation")
        || c.contains("缓存创建")
        || c.contains("缓存写入")
    {
        Some(3)
    } else if (c.contains("cached")
        || c.contains("cachehit")
        || c.contains("cacheread")
        || c.contains("命中缓存")
        || c.contains("缓存命中")
        || c.contains("缓存读取"))
        && !c.contains("未命中")
        && !c.contains("miss")
    {
        Some(2)
    } else if c.contains("output") || c.contains("输出") {
        Some(1)
    } else if c.contains("input") || c.contains("输入") {
        Some(0)
    } else {
        None
    }
}
pub(super) fn decimal(raw: &str) -> Option<String> {
    let mut parts = raw.split('.');
    let a = parts.next()?;
    let b = parts.next();
    if a.is_empty()
        || a.len() > 9
        || !a.bytes().all(|c| c.is_ascii_digit())
        || parts.next().is_some()
        || b.is_some_and(|b| b.is_empty() || b.len() > 9 || !b.bytes().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    Some(raw.into())
}
pub(super) fn cell_rate(raw: &str) -> Option<String> {
    if ["免费", "Free", "free"].contains(&raw.trim()) {
        return Some("0".into());
    }
    if raw.contains(['%', '>', '<', '–', '-', '≥', '≤']) {
        return None;
    }
    let clean = raw.replace("1M", "").replace("1m", "");
    let ns: Vec<_> = clean
        .split(|c: char| !c.is_ascii_digit() && c != '.')
        .filter(|s| !s.is_empty())
        .collect();
    (ns.len() == 1).then(|| decimal(ns[0])).flatten()
}
fn rates(a: [Option<String>; 4]) -> RateSet {
    let [input, output, cache_read, cache_write] = a;
    RateSet {
        input,
        output,
        cache_read,
        cache_write,
    }
}
fn currency(s: &str) -> Option<&'static str> {
    let usd = s.contains('$') || s.contains("USD") || s.contains("美元");
    let cny = s.contains(['¥', '￥'])
        || s.contains("CNY")
        || s.contains("元/")
        || s.contains("元 /")
        || s.contains("元") && !usd;
    match (usd, cny) {
        (true, false) => Some("USD"),
        (false, true) => Some("CNY"),
        _ => None,
    }
}
fn bounds(s: &str) -> Option<(u64, Option<u64>)> {
    let norm = s
        .replace('，', ",")
        .replace('（', "(")
        .replace('）', ")")
        .replace(' ', "");
    fn number(s: &str) -> Option<u64> {
        let s = s.trim();
        if let Some(k) = s.strip_suffix(['K', 'k']) {
            let n = k.parse::<f64>().ok()?;
            let n = n * 1000.;
            (n.is_finite() && n >= 0. && n.fract() == 0.).then_some(n as u64)
        } else {
            s.parse().ok()
        }
    }
    if let Some((_, tail)) = norm.split_once('[') {
        let (a, b) = tail.split_once(',')?;
        let b = b.split([')', ']', '）']).next()?;
        return Some((number(a)?, Some(number(b)?)));
    }
    if let Some((_, tail)) = norm.split_once('≥') {
        return Some((number(tail)?, None));
    }
    None
}
pub fn parse(
    text: &str,
    agent: &str,
    model: &str,
    url: &str,
    wanted: Option<&str>,
    batch: bool,
) -> Result<PriceQuote, String> {
    let lower = text.to_lowercase();
    if !["million", "1m", "mtok"].iter().any(|s| lower.contains(s)) && !text.contains("百万") {
        return Err("页面未明确每百万 Token 单位".into());
    }
    let fallback = currency(text);
    let mut headers: Vec<Option<usize>> = vec![];
    let mut model_col = None;
    let mut horizontal = None;
    let mut found: Vec<(RateSet, String)> = vec![];
    let mut base: [Option<String>; 4] = Default::default();
    let mut peak: [Option<String>; 4] = Default::default();
    let mut horizontal_currency = None;
    for line in text.lines() {
        if line == "[TABLE]" || line == "[/TABLE]" {
            headers.clear();
            model_col = None;
            horizontal = None;
            continue;
        }
        let cells: Vec<_> = line.split('|').map(str::trim).collect();
        if cells.len() < 3 {
            continue;
        }
        let mapped: Vec<_> = cells.iter().map(|s| kind(s)).collect();
        if mapped.contains(&Some(0))
            && mapped.contains(&Some(1))
            && cells
                .iter()
                .any(|s| key(s).contains("model") || s.contains("模型"))
        {
            model_col = cells
                .iter()
                .position(|s| key(s).contains("model") || s.contains("模型"));
            headers = mapped;
            horizontal = None;
            continue;
        }
        if headers.is_empty() && ["模型", "Model", "Models"].contains(&cells[0]) {
            horizontal = cells.iter().position(|s| matches_model(s, model));
            continue;
        }
        if let Some(index) = horizontal {
            if index >= cells.len() {
                continue;
            }
            let prefix = cells[..index].join(" ");
            if let Some(k) = kind(&prefix) {
                let dest = if prefix.contains("高峰") {
                    &mut peak
                } else {
                    &mut base
                };
                dest[k] = cell_rate(cells[index]);
                horizontal_currency = currency(cells[index]).or(horizontal_currency);
            }
            continue;
        }
        let Some(index) = model_col else {
            continue;
        };
        if cells.len() != headers.len() || !matches_model(cells[index], model) {
            continue;
        }
        let is_batch = cells
            .iter()
            .any(|s| s.contains("批量") || s.to_lowercase().contains("batch"));
        // Pages with no separate batch rows must not fabricate a batch discount.
        if is_batch != batch {
            continue;
        }
        let row_currency = currency(line)
            .or_else(|| {
                currency(
                    &text[..text.find(line).unwrap_or(0)]
                        .lines()
                        .rev()
                        .take(4)
                        .collect::<Vec<_>>()
                        .join(" "),
                )
            })
            .or(fallback);
        let Some(cur) = row_currency else {
            continue;
        };
        if wanted.is_some_and(|w| w != cur) {
            continue;
        }
        let mut a: [Option<String>; 4] = Default::default();
        for (cell, k) in cells.iter().zip(&headers) {
            if let Some(k) = k {
                a[*k] = cell_rate(cell);
            }
        }
        if a[0].is_some() && a[1].is_some() {
            found.push((
                rates(a),
                cells
                    .iter()
                    .enumerate()
                    .filter(|(i, s)| {
                        *i != index
                            && headers[*i].is_none()
                            && (s.contains("输入") || s.contains("输出"))
                    })
                    .map(|(_, s)| *s)
                    .collect::<Vec<_>>()
                    .join(" "),
            ));
        }
    }
    let mut quote = PriceQuote {
        agent: agent.into(),
        model: model.into(),
        currency: wanted
            .or(horizontal_currency)
            .or(fallback)
            .unwrap_or("CNY")
            .into(),
        input: None,
        output: None,
        cache_read: None,
        cache_write: None,
        source_url: url.into(),
        method: "网页表格".into(),
        notes: vec![if batch {
            "批量推理价格；请确认日志对应 Batch API。".into()
        } else {
            "标准实时推理价格；地区/币种按所选值筛选，未计搜索、音频或按小时存储费用。".into()
        }],
        peak: None,
        peak_windows: None,
        context_tiers: None,
    };
    let r = if base[0].is_some() && base[1].is_some() {
        if batch || wanted.is_some_and(|w| Some(w) != horizontal_currency.or(fallback)) {
            return Err("未找到所选币种/调用方式的价格".into());
        }
        if peak.iter().any(Option::is_some) {
            if peak[0].is_none()
                || peak[1].is_none()
                || !text.contains("北京时间")
                || !text.contains("周一至周五")
                || !text.contains("9:00 - 12:00")
                || !text.contains("14:00 - 18:00")
            {
                return Err("分时段价格存在，但时区/日历/时间段无法完整确认，请手动配置".into());
            }
            quote.peak = Some(rates(peak));
            quote.peak_windows = Some(PeakWindows {
                timezone: "Asia/Shanghai".into(),
                weekdays_only: true,
                exclude_china_holidays: text.contains("不含中国法定节假日"),
                ranges: vec!["09:00-12:00".into(), "14:00-18:00".into()],
            });
            quote.notes.push("基础价为空闲时段，高峰价按北京时间周一至周五；排除中国节假日时当前日历覆盖 2026 年，其他年份估算保留未知。".into());
        }
        rates(base)
    } else {
        if found.is_empty() {
            return Err(
                "未找到目标模型的完整价格；请检查模型名、币种、调用方式或使用模型辅助解析".into(),
            );
        }
        if found.len() == 1 && bounds(&found[0].1).is_none() {
            found.remove(0).0
        } else {
            let mut tiers = vec![];
            for (r, label) in &found {
                let (input_min, input_max) =
                    bounds(label).ok_or("存在多行价格，无法确认分档条件，请手动核对")?;
                let output = label
                    .split_once("输出")
                    .map(|(_, s)| bounds(s).ok_or("输出分档条件不兼容"))
                    .transpose()?;
                tiers.push(ContextTier {
                    label: label.clone(),
                    input_min: Some(input_min),
                    input_max,
                    output_min: output.map(|v| v.0),
                    output_max: output.and_then(|v| v.1),
                    rates: r.clone(),
                });
            }
            quote.context_tiers = Some(tiers);
            quote.notes.push("整请求按输入/输出长度组合分档；区间下限包含、上限不含，缺少判定用量或超出已知范围时不估算。".into());
            found[0].0.clone()
        }
    };
    quote.input = r.input;
    quote.output = r.output;
    quote.cache_read = r.cache_read;
    quote.cache_write = r.cache_write;
    if quote.cache_write.is_none() && text.replace(' ', "").contains("缓存写入：限时免费")
    {
        quote.cache_write = Some("0".into());
        quote
            .notes
            .push("缓存写入按页面当前限时免费计为 0；请关注免费期结束公告并重新获取价格。".into());
    }
    super::price_page::validate_quote(&quote)?;
    Ok(quote)
}
