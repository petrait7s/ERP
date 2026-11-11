//! Number, money and date formatting for Bangladesh: lakh/crore grouping, optional Bangla digits.
use serde_json::Value;

pub fn num(v: &Value) -> f64 {
    match v { Value::Number(n) => n.as_f64().unwrap_or(0.0), Value::String(s) => s.parse().unwrap_or(0.0), _ => 0.0 }
}

/// 1234567.5 -> "12,34,567.50" (South Asian grouping).
pub fn group_in(x: f64, decimals: usize) -> String {
    let neg = x < 0.0;
    let s = format!("{:.*}", decimals, x.abs());
    let (int, frac) = s.split_once('.').map(|(a, b)| (a.to_string(), Some(b.to_string()))).unwrap_or((s.clone(), None));
    let mut out = String::new();
    let b: Vec<char> = int.chars().collect();
    for (i, c) in b.iter().enumerate() {
        let from_end = b.len() - i;
        if i > 0 && (from_end == 3 || (from_end > 3 && (from_end - 3) % 2 == 0)) { out.push(','); }
        out.push(*c);
    }
    if let Some(f) = frac { out.push('.'); out.push_str(&f); }
    if neg { format!("-{out}") } else { out }
}

pub fn bn_digits(s: &str) -> String {
    s.chars().map(|c| match c { '0'..='9' => char::from_u32('০' as u32 + (c as u32 - '0' as u32)).unwrap(), _ => c }).collect()
}
pub fn digits(s: String) -> String { if crate::state::is_bn() { bn_digits(&s) } else { s } }

pub fn money(v: &Value) -> String { digits(format!("৳ {}", group_in(num(v), 2))) }
pub fn money_f(x: f64) -> String { digits(format!("৳ {}", group_in(x, 2))) }
/// Compact money for tiles: 1.2 Cr, 34.5 L, 12.3 K.
pub fn money_short(x: f64) -> String {
    let a = x.abs(); let sign = if x < 0.0 { "-" } else { "" };
    let s = if a >= 1.0e7 { format!("{:.2} Cr", a / 1.0e7) } else if a >= 1.0e5 { format!("{:.2} L", a / 1.0e5) } else if a >= 1.0e3 { format!("{:.1} K", a / 1.0e3) } else { format!("{:.0}", a) };
    digits(format!("{sign}৳ {s}"))
}
pub fn qty(v: &Value) -> String { let x = num(v); digits(if x.fract().abs() < 1e-9 { group_in(x, 0) } else { group_in(x, 2).trim_end_matches('0').trim_end_matches('.').to_string() }) }
pub fn fc(v: &Value, cur: &str) -> String { digits(format!("{} {}", cur, group_in(num(v), 2))) }
pub fn pct(v: &Value) -> String { digits(format!("{:.1}%", num(v))) }

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTHS_BN: [&str; 12] = ["জানু", "ফেব্র", "মার্চ", "এপ্রিল", "মে", "জুন", "জুলাই", "আগস্ট", "সেপ্টে", "অক্টো", "নভে", "ডিসে"];
/// "2026-12-01" -> "01 Dec 2026"; timestamps are shown in Bangladesh Standard Time.
pub fn date(v: &Value) -> String {
    let s = v.as_str().unwrap_or("");
    if s.len() < 10 { return if s.is_empty() { "—".into() } else { s.into() }; }
    let (y, m, d) = (&s[0..4], s[5..7].parse::<usize>().unwrap_or(1), &s[8..10]);
    let mon = if crate::state::is_bn() { MONTHS_BN[(m.max(1) - 1).min(11)] } else { MONTHS[(m.max(1) - 1).min(11)] };
    digits(format!("{d} {mon} {y}"))
}
pub fn datetime(v: &Value) -> String {
    let s = v.as_str().unwrap_or("");
    if s.len() < 16 { return date(v); }
    // timestamps arrive in UTC; BST is UTC+6
    let h: u32 = s[11..13].parse().unwrap_or(0); let mi = &s[14..16];
    let total = h + 6; let day_shift = total >= 24; let hh = total % 24;
    let base = date(&Value::String(s[0..10].to_string()));
    digits(format!("{base}{} {:02}:{mi}", if day_shift { " (+1)" } else { "" }, hh))
}
pub fn today() -> String { let d = js_sys::Date::new_0(); format!("{:04}-{:02}-{:02}", d.get_full_year(), d.get_month() + 1, d.get_date()) }
pub fn add_days(date: &str, days: i32) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(&format!("{date}T00:00:00")));
    d.set_date((d.get_date() as i32 + days) as u32);
    format!("{:04}-{:02}-{:02}", d.get_full_year(), d.get_month() + 1, d.get_date())
}
pub fn month_start() -> String { let d = js_sys::Date::new_0(); format!("{:04}-{:02}-01", d.get_full_year(), d.get_month() + 1) }
pub fn text(v: &Value) -> String { match v { Value::String(s) => s.clone(), Value::Null => "—".into(), Value::Bool(b) => if *b { "Yes".into() } else { "No".into() }, Value::Number(n) => n.to_string(), other => other.to_string() } }
/// "partially_received" -> "Partially received"
pub fn nice(s: &str) -> String { let t = s.replace('_', " "); let mut c = t.chars(); match c.next() { Some(f) => f.to_uppercase().collect::<String>() + c.as_str(), None => String::new() } }
