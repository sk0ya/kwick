//! Inline answers: unit conversion ("10km to mi"), currency ("100 usd in
//! jpy"), and dates ("today+30d", "2026-12-25", "unix 1700000000"). Enter
//! copies the value.

use super::{Action, Item};
use eframe::egui;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Length,
    Mass,
    Volume,
    Area,
    Temperature,
    Data,
    Time,
    Speed,
}

/// (aliases, kind, factor to the kind's base unit). Temperatures use the
/// factor as a tag and are converted by `to_kelvin`/`from_kelvin`.
const UNITS: &[(&[&str], Kind, f64)] = &[
    (&["mm", "ミリ"], Kind::Length, 0.001),
    (&["cm", "センチ"], Kind::Length, 0.01),
    (&["m", "meter", "meters", "メートル"], Kind::Length, 1.0),
    (&["km", "キロメートル"], Kind::Length, 1000.0),
    (&["in", "inch", "inches", "インチ", "\""], Kind::Length, 0.0254),
    (&["ft", "feet", "foot", "フィート", "'"], Kind::Length, 0.3048),
    (&["yd", "yard", "yards", "ヤード"], Kind::Length, 0.9144),
    (&["mi", "mile", "miles", "マイル"], Kind::Length, 1609.344),
    (&["nmi", "海里"], Kind::Length, 1852.0),
    (&["mg"], Kind::Mass, 1e-6),
    (&["g", "gram", "grams", "グラム"], Kind::Mass, 0.001),
    (&["kg", "キログラム", "キロ"], Kind::Mass, 1.0),
    (&["t", "ton", "tonne", "トン"], Kind::Mass, 1000.0),
    (&["oz", "ounce", "オンス"], Kind::Mass, 0.028_349_523_125),
    (&["lb", "lbs", "pound", "pounds"], Kind::Mass, 0.453_592_37),
    (&["ml", "cc"], Kind::Volume, 0.001),
    (&["l", "liter", "litre", "リットル"], Kind::Volume, 1.0),
    (&["gal", "gallon", "ガロン"], Kind::Volume, 3.785_411_784),
    (&["qt", "quart"], Kind::Volume, 0.946_352_946),
    (&["pt", "pint", "パイント"], Kind::Volume, 0.473_176_473),
    (&["cup", "カップ"], Kind::Volume, 0.24),
    (&["floz"], Kind::Volume, 0.029_573_529_562_5),
    (&["m2", "m²", "平米", "平方メートル"], Kind::Area, 1.0),
    (&["km2", "km²"], Kind::Area, 1e6),
    (&["ha", "ヘクタール"], Kind::Area, 1e4),
    (&["a", "アール"], Kind::Area, 100.0),
    (&["acre", "acres", "エーカー"], Kind::Area, 4046.856_422_4),
    (&["ft2", "ft²", "sqft"], Kind::Area, 0.092_903_04),
    (&["坪", "tsubo"], Kind::Area, 400.0 / 121.0),
    (&["c", "°c", "℃", "celsius", "度", "摂氏"], Kind::Temperature, 1.0),
    (&["f", "°f", "℉", "fahrenheit", "華氏"], Kind::Temperature, 2.0),
    (&["k", "kelvin", "ケルビン"], Kind::Temperature, 3.0),
    (&["bit", "bits"], Kind::Data, 0.125),
    (&["b", "byte", "bytes", "バイト"], Kind::Data, 1.0),
    (&["kb"], Kind::Data, 1e3),
    (&["mb"], Kind::Data, 1e6),
    (&["gb"], Kind::Data, 1e9),
    (&["tb"], Kind::Data, 1e12),
    (&["kib"], Kind::Data, 1024.0),
    (&["mib"], Kind::Data, 1_048_576.0),
    (&["gib"], Kind::Data, 1_073_741_824.0),
    (&["tib"], Kind::Data, 1_099_511_627_776.0),
    (&["ms", "ミリ秒"], Kind::Time, 0.001),
    (&["s", "sec", "secs", "second", "seconds", "秒"], Kind::Time, 1.0),
    (&["min", "mins", "minute", "minutes", "分"], Kind::Time, 60.0),
    (&["h", "hr", "hrs", "hour", "hours", "時間"], Kind::Time, 3600.0),
    (&["d", "day", "days", "日"], Kind::Time, 86400.0),
    (&["wk", "week", "weeks", "週"], Kind::Time, 604_800.0),
    (&["yr", "year", "years", "年"], Kind::Time, 31_536_000.0),
    (&["m/s", "mps"], Kind::Speed, 1.0),
    (&["km/h", "kmh", "kph", "キロ毎時"], Kind::Speed, 1.0 / 3.6),
    (&["mph"], Kind::Speed, 0.447_04),
    (&["kn", "kt", "knot", "knots", "ノット"], Kind::Speed, 0.514_444),
];

fn unit(name: &str) -> Option<(Kind, f64)> {
    let name = name.to_lowercase();
    UNITS
        .iter()
        .find(|(aliases, _, _)| aliases.contains(&name.as_str()))
        .map(|&(_, kind, factor)| (kind, factor))
}

fn to_kelvin(v: f64, tag: f64) -> f64 {
    match tag as u8 {
        1 => v + 273.15,
        2 => (v - 32.0) * 5.0 / 9.0 + 273.15,
        _ => v,
    }
}

fn from_kelvin(k: f64, tag: f64) -> f64 {
    match tag as u8 {
        1 => k - 273.15,
        2 => (k - 273.15) * 9.0 / 5.0 + 32.0,
        _ => k,
    }
}

/// Up to 10 significant digits, no trailing zeros, scientific when huge or tiny.
pub fn format_number(x: f64) -> String {
    if !x.is_finite() {
        return x.to_string();
    }
    let a = x.abs();
    if a != 0.0 && !(1e-6..1e15).contains(&a) {
        return format!("{x:e}");
    }
    let decimals = if a == 0.0 {
        0
    } else {
        (9 - a.log10().floor() as i32).clamp(0, 12) as usize
    };
    let s = format!("{x:.decimals$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };
    if s == "-0" {
        "0".into()
    } else {
        s
    }
}

/// "10km to mi" → (10, "km", "mi"). The keyword (to / in / -> / =) is
/// required so that ordinary searches are never mistaken for conversions.
fn parse_conversion(q: &str) -> Option<(f64, String, String)> {
    let q = q.replace('→', " to ").replace("->", " to ").replace("=>", " to ");
    let tokens: Vec<&str> = q.split_whitespace().collect();
    // The last keyword, so "10 in to cm" reads "in" as inches.
    let kw = tokens
        .iter()
        .rposition(|t| matches!(t.to_lowercase().as_str(), "to" | "in" | "=" | "は"))?;
    if kw == 0 || kw + 2 != tokens.len() {
        return None;
    }
    let left: String = tokens[..kw].concat();
    let split = left
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || c == ',' || (i == 0 && c == '-')))
        .map(|(i, _)| i)?;
    let value: f64 = left[..split].replace(',', "").parse().ok()?;
    let from = left[split..].to_string();
    Some((value, from, tokens[kw + 1].to_string()))
}

fn copy_item(value: String, subtitle: String) -> Item {
    Item::new(value.clone(), subtitle, Action::Copy(value)).transient()
}

/// Physical unit conversion.
fn convert_units(q: &str) -> Option<Item> {
    let (value, from, to) = parse_conversion(q)?;
    let (k1, f1) = unit(&from)?;
    let (k2, f2) = unit(&to)?;
    if k1 != k2 {
        return None;
    }
    let result = if k1 == Kind::Temperature {
        from_kelvin(to_kelvin(value, f1), f2)
    } else {
        value * f1 / f2
    };
    let shown = format_number(result);
    let mut item = copy_item(
        format!("{shown} {to}"),
        format!("{} {from} = {shown} {to}  (Enter でコピー)", format_number(value)),
    );
    item.action = Action::Copy(shown);
    Some(item)
}

// ---------------------------------------------------------------- currency

#[derive(Default)]
enum RateState {
    #[default]
    Unknown,
    Fetching,
    /// units of each currency per 1 USD
    Ready(HashMap<String, f64>),
    Failed(String),
}

/// Exchange rates, fetched on the first currency query (never otherwise)
/// and cached on disk for 12 hours.
#[derive(Clone, Default)]
pub struct Currency {
    state: Arc<Mutex<RateState>>,
}

const RATES_URL: &str = "https://open.er-api.com/v6/latest/USD";
const RATES_MAX_AGE: Duration = Duration::from_secs(12 * 3600);

fn currency_code(name: &str) -> Option<String> {
    let code = match name {
        "円" | "えん" => "JPY",
        "ドル" | "$" => "USD",
        "ユーロ" | "€" => "EUR",
        "元" | "人民元" => "CNY",
        "ウォン" => "KRW",
        "ポンド" | "£" => "GBP",
        _ if name.len() == 3 && name.chars().all(|c| c.is_ascii_alphabetic()) => {
            return Some(name.to_ascii_uppercase())
        }
        _ => return None,
    };
    Some(code.into())
}

/// Pull `"XYZ": number` pairs out of the "rates" object (the API's JSON is
/// flat enough that a JSON parser would be dead weight).
pub fn parse_rates(json: &str) -> Option<HashMap<String, f64>> {
    let start = json.find("\"rates\"")?;
    let body = &json[start..];
    let open = body.find('{')?;
    let close = body[open..].find('}')? + open;
    let mut out = HashMap::new();
    for pair in body[open + 1..close].split(',') {
        let (k, v) = pair.split_once(':')?;
        let k = k.trim().trim_matches('"');
        let v: f64 = v.trim().parse().ok()?;
        out.insert(k.to_string(), v);
    }
    (!out.is_empty()).then_some(out)
}

impl Currency {
    fn cache_path() -> std::path::PathBuf {
        crate::config::config_dir().join("rates.json")
    }

    fn load_cached() -> Option<HashMap<String, f64>> {
        let path = Self::cache_path();
        let age = std::fs::metadata(&path).ok()?.modified().ok()?.elapsed().ok()?;
        if age > RATES_MAX_AGE {
            return None;
        }
        parse_rates(&std::fs::read_to_string(path).ok()?)
    }

    fn ensure(&self, ctx: &egui::Context) {
        let mut state = self.state.lock().unwrap();
        if !matches!(*state, RateState::Unknown) {
            return;
        }
        if let Some(rates) = Self::load_cached() {
            *state = RateState::Ready(rates);
            return;
        }
        *state = RateState::Fetching;
        let shared = self.state.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = crate::http::get(RATES_URL).and_then(|r| {
                let text = String::from_utf8_lossy(&r.body).into_owned();
                let rates = parse_rates(&text).ok_or("為替レートを解釈できません")?;
                let _ = std::fs::write(Self::cache_path(), &text);
                Ok(rates)
            });
            *shared.lock().unwrap() = match result {
                Ok(rates) => RateState::Ready(rates),
                Err(e) => RateState::Failed(e),
            };
            ctx.request_repaint();
        });
    }

    /// A download is in flight (the launcher re-searches when it ends).
    pub fn is_fetching(&self) -> bool {
        matches!(*self.state.lock().unwrap(), RateState::Fetching)
    }

    fn convert(&self, q: &str, ctx: &egui::Context) -> Option<Item> {
        let (value, from, to) = parse_conversion(q)?;
        let from = currency_code(&from)?;
        let to = currency_code(&to)?;
        self.ensure(ctx);
        let state = self.state.lock().unwrap();
        match &*state {
            RateState::Ready(rates) => {
                let (a, b) = (rates.get(&from)?, rates.get(&to)?);
                let result = value / a * b;
                let decimals = if to == "JPY" || to == "KRW" { 0 } else { 2 };
                let shown = format!("{result:.decimals$}");
                let mut item = copy_item(
                    format!("{shown} {to}"),
                    format!(
                        "{} {from} = {shown} {to}  (open.er-api.com のレート、Enter でコピー)",
                        format_number(value)
                    ),
                );
                item.action = Action::Copy(shown);
                Some(item)
            }
            RateState::Fetching | RateState::Unknown => Some(
                Item::new("為替レートを取得中…", format!("{from} → {to}"), Action::Copy(String::new()))
                    .transient(),
            ),
            RateState::Failed(e) => Some(
                Item::new("為替レートを取得できませんでした", e.clone(), Action::Copy(String::new()))
                    .transient(),
            ),
        }
    }
}

// ------------------------------------------------------------------- dates

const WEEKDAYS: [&str; 7] = ["日", "月", "火", "水", "木", "金", "土"];

/// Days since 1970-01-01 for a proleptic Gregorian date (H. Hinnant).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn weekday(days: i64) -> &'static str {
    WEEKDAYS[(days + 4).rem_euclid(7) as usize]
}

fn days_in_month(y: i64, m: u32) -> u32 {
    let next = if m == 12 { days_from_civil(y + 1, 1, 1) } else { days_from_civil(y, m + 1, 1) };
    (next - days_from_civil(y, m, 1)) as u32
}

/// Local time zone offset in seconds (from the system clock pair).
fn local_offset() -> i64 {
    use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
    let (local, utc) = unsafe { (GetLocalTime(), GetSystemTime()) };
    let secs = |t: &windows::Win32::Foundation::SYSTEMTIME| {
        days_from_civil(t.wYear as i64, t.wMonth as u32, t.wDay as u32) * 86400
            + t.wHour as i64 * 3600
            + t.wMinute as i64 * 60
    };
    // Round to the quarter hour: the two reads may straddle a minute.
    let diff = secs(&local) - secs(&utc);
    (diff as f64 / 900.0).round() as i64 * 900
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn format_date(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} ({})", weekday(days))
}

fn format_datetime(unix: i64, offset: i64) -> String {
    let local = unix + offset;
    let days = local.div_euclid(86400);
    let secs = local.rem_euclid(86400);
    format!(
        "{} {:02}:{:02}:{:02}",
        format_date(days),
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

fn relative(days: i64, today: i64) -> String {
    match days - today {
        0 => "今日".into(),
        n if n > 0 => format!("あと {n} 日"),
        n => format!("{} 日前", -n),
    }
}

fn parse_date(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.split(['-', '/', '.']).collect();
    if parts.len() != 3 {
        return None;
    }
    let y: i64 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    let d: u32 = parts[2].parse().ok()?;
    if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) || !(1..=9999).contains(&y) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// Add months, clamping the day ("01-31 + 1 month" → end of February).
fn add_months(days: i64, months: i64) -> i64 {
    let (y, m, d) = civil_from_days(days);
    let total = y * 12 + (m as i64 - 1) + months;
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) as u32 + 1);
    days_from_civil(ny, nm, d.min(days_in_month(ny, nm)))
}

fn date_items(q: &str, now: i64, offset: i64) -> Vec<Item> {
    let q = q.trim().to_lowercase();
    let today = (now + offset).div_euclid(86400);
    let date_item = |days: i64, note: &str| {
        let text = format_date(days);
        let (y, m, d) = civil_from_days(days);
        let mut item = copy_item(text, format!("{note}{}  (Enter でコピー)", relative(days, today)));
        item.action = Action::Copy(format!("{y:04}-{m:02}-{d:02}"));
        item
    };

    if matches!(q.as_str(), "now" | "today" | "今日" | "きょう" | "今" | "いま" | "date" | "time") {
        let mut items = vec![copy_item(format_datetime(now, offset), "現在の日時 (Enter でコピー)".into())];
        items.push(copy_item(now.to_string(), "Unix 時刻 (秒)".into()));
        return items;
    }
    if let Some(rest) = q.strip_prefix("unix").or_else(|| q.strip_prefix("epoch")) {
        let Ok(mut n) = rest.trim().parse::<i64>() else {
            return Vec::new();
        };
        if n.abs() >= 100_000_000_000 {
            n /= 1000; // milliseconds
        }
        return vec![copy_item(
            format_datetime(n, offset),
            format!("Unix {n} のローカル日時 (Enter でコピー)"),
        )];
    }

    let compact: String = q.chars().filter(|c| !c.is_whitespace()).collect();
    if let Some(days) = parse_date(&compact) {
        return vec![date_item(days, "")];
    }
    // today+30d / now - 3h / 2026-01-31 + 1m
    if let Some(pos) = compact.rfind(['+', '-']).filter(|&p| p > 0) {
        let (base, delta) = compact.split_at(pos);
        let sign = if delta.starts_with('-') { -1 } else { 1 };
        let delta = &delta[1..];
        let split = delta.find(|c: char| !c.is_ascii_digit()).unwrap_or(delta.len());
        let Ok(n) = delta[..split].parse::<i64>() else {
            return Vec::new();
        };
        let unit = &delta[split..];
        let n = n * sign;
        let base_days = match base {
            "today" | "今日" | "now" => Some(today),
            other => parse_date(other),
        };
        let Some(base_days) = base_days else {
            return Vec::new();
        };
        if base == "now" && matches!(unit, "h" | "min" | "m" | "s") {
            let secs = match unit {
                "h" => 3600,
                "s" => 1,
                _ => 60,
            };
            return vec![copy_item(
                format_datetime(now + n * secs, offset),
                format!("{q} (Enter でコピー)"),
            )];
        }
        let days = match unit {
            "d" | "" | "日" => base_days + n,
            "w" | "週" => base_days + n * 7,
            "m" | "mo" | "ヶ月" | "か月" => add_months(base_days, n),
            "y" | "年" => add_months(base_days, n * 12),
            _ => return Vec::new(),
        };
        return vec![date_item(days, &format!("{q} → "))];
    }
    Vec::new()
}

/// All inline answers for the query (usually none).
pub fn query(q: &str, currency: &Currency, ctx: &egui::Context) -> Vec<Item> {
    if let Some(item) = convert_units(q) {
        return vec![item];
    }
    if let Some(item) = currency.convert(q, ctx) {
        return vec![item];
    }
    date_items(q, now_unix(), local_offset())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(q: &str) -> Option<String> {
        convert_units(q).map(|i| i.title)
    }

    #[test]
    fn converts_units() {
        assert_eq!(title("10km to mi").unwrap(), "6.213711922 mi");
        assert_eq!(title("100 c in f").unwrap(), "212 f");
        assert_eq!(title("1 GiB to MB").unwrap(), "1073.741824 MB");
        assert_eq!(title("1,000 g -> kg").unwrap(), "1 kg");
        assert_eq!(title("20坪 to m2").unwrap(), "66.11570248 m2");
        assert!(title("10km to kg").is_none()); // different kinds
        assert!(title("vscode").is_none());
        assert!(title("google chrome in").is_none());
    }

    #[test]
    fn formats_numbers() {
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(-0.0), "0");
        assert_eq!(format_number(1e20), "1e20");
        assert_eq!(format_number(123456.0), "123456");
    }

    #[test]
    fn parses_rates() {
        let json = r#"{"result":"success","base_code":"USD","rates":{"USD":1,"JPY":149.5,"EUR":0.92}}"#;
        let rates = parse_rates(json).unwrap();
        assert_eq!(rates["JPY"], 149.5);
        assert_eq!(rates.len(), 3);
        assert_eq!(currency_code("usd").as_deref(), Some("USD"));
        assert_eq!(currency_code("円").as_deref(), Some("JPY"));
        assert!(currency_code("chrome").is_none());
    }

    #[test]
    fn civil_round_trip() {
        for days in [-1000, 0, 19_000, 20_500, 2_932_896] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(weekday(days_from_civil(2026, 10, 4)), "日");
        assert_eq!(add_months(days_from_civil(2026, 1, 31), 1), days_from_civil(2026, 2, 28));
    }

    #[test]
    fn date_queries() {
        // 2026-10-04 12:00 UTC, JST (+9h)
        let now = days_from_civil(2026, 10, 4) * 86400 + 12 * 3600;
        let jst = 9 * 3600;
        let t = |q: &str| date_items(q, now, jst).first().map(|i| i.title.clone());
        assert_eq!(t("today+30d").unwrap(), "2026-11-03 (火)");
        assert_eq!(t("today - 1w").unwrap(), "2026-09-27 (日)");
        assert_eq!(t("2026-12-25").unwrap(), "2026-12-25 (金)");
        assert!(date_items("2026-12-25", now, jst)[0].subtitle.contains("あと 82 日"));
        assert_eq!(t("unix 0").unwrap(), "1970-01-01 (木) 09:00:00");
        assert_eq!(t("now").unwrap(), "2026-10-04 (日) 21:00:00");
        assert!(t("2026-13-01").is_none());
        assert!(t("notepad").is_none());
        assert!(t("c-3po").is_none());
    }
}
