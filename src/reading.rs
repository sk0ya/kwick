//! Readings (読み) for Japanese titles, so "しかく" / "shikaku" find
//! 資格情報マネージャー without typing the kanji. The reading comes from the
//! system IME's reverse conversion (IFELanguage, "MSIME.Japan").

use crate::providers::Item;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// title → reading, kept across rescans (each IME lookup costs a few ms).
fn cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Append the hiragana and romaji reading of every Japanese title to its key.
/// Silently does nothing when the Japanese IME isn't installed.
pub fn annotate(items: &mut [Item]) {
    let mut readings = cache().lock().unwrap();
    let mut missing: Vec<String> = items
        .iter()
        .filter(|it| has_japanese(&it.title) && !readings.contains_key(&it.title))
        .map(|it| it.title.clone())
        .collect();
    missing.sort();
    missing.dedup();
    if !missing.is_empty() {
        // A fresh thread gets its own STA, whatever the caller's COM state is.
        let found: Vec<(String, String)> = std::thread::spawn(move || {
            let Some(ime) = Ime::open() else {
                return Vec::new();
            };
            missing
                .into_iter()
                .filter_map(|t| ime.phonetic(&t).map(|r| (t, r)))
                .collect()
        })
        .join()
        .unwrap_or_default();
        readings.extend(found);
    }

    for item in items.iter_mut() {
        if !has_japanese(&item.title) {
            continue;
        }
        // Without an IME reading, the title's own kana still gets a hiragana
        // form, since queries are folded to hiragana (see matcher).
        let kana = readings.get(&item.title).unwrap_or(&item.title);
        let hira = to_hiragana(kana);
        let romaji = to_romaji(&hira);
        item.key = format!("{} {hira} {romaji}", item.key);
    }
}

/// Cheap variant for short-lived results (window titles, processes): only
/// fold the title's own katakana, without asking the IME for readings.
pub fn annotate_kana(items: &mut [Item]) {
    for item in items.iter_mut() {
        if item.title.chars().any(|c| ('ァ'..='ヶ').contains(&c)) {
            item.key = format!("{} {}", item.key, to_hiragana(&item.title));
        }
    }
}

/// Katakana → hiragana, other characters unchanged. Used for queries too, so
/// "シカク" matches the hiragana reading.
pub fn to_hiragana(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'ァ'..='ヶ' => char::from_u32(c as u32 - 0x60).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// The IME hands back ASCII as full-width (Ｗｉｎｄｏｗｓ); fold it back.
fn to_halfwidth(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{3000}' => ' ',
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            _ => c,
        })
        .collect()
}

fn has_japanese(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(c, '\u{3040}'..='\u{30FF}' | '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}')
    })
}

/// Hiragana → Hepburn romaji (shi, tsu, ja...). Non-kana passes through.
pub fn to_romaji(hira: &str) -> String {
    const SMALL: &[(char, &str)] = &[('ゃ', "ya"), ('ゅ', "yu"), ('ょ', "yo")];
    let chars: Vec<char> = hira.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // っ doubles the next consonant.
        if c == 'っ' {
            if let Some(next) = chars.get(i + 1).and_then(|&n| kana_romaji(n)) {
                out.push_str(&next[..1]);
            }
            i += 1;
            continue;
        }
        let Some(base) = kana_romaji(c) else {
            if c != 'ー' {
                out.push(c);
            }
            i += 1;
            continue;
        };
        // ふぁ → fa, てぃ → ti, うぉ → wo, しぇ → she, ゔぁ → va
        if let Some(v) = chars.get(i + 1).and_then(|&n| small_vowel(n)) {
            let stem = match base {
                "u" => "w",
                "shi" | "chi" | "ji" => &base[..base.len() - 1],
                _ => base.trim_end_matches(['a', 'i', 'u', 'e', 'o']),
            };
            if !stem.is_empty() {
                out.push_str(stem);
                out.push(v);
                i += 2;
                continue;
            }
        }
        // きゃ → kya, しゃ → sha, じゃ → ja, ちゃ → cha
        if let Some(&(_, y)) = chars.get(i + 1).and_then(|n| SMALL.iter().find(|(s, _)| s == n)) {
            if base.len() >= 2 && base.ends_with('i') {
                let stem = &base[..base.len() - 1];
                match stem {
                    "sh" | "ch" | "j" => {
                        out.push_str(stem);
                        out.push_str(&y[1..]);
                    }
                    _ => {
                        out.push_str(stem);
                        out.push_str(y);
                    }
                }
                i += 2;
                continue;
            }
        }
        out.push_str(base);
        i += 1;
    }
    out
}

fn small_vowel(c: char) -> Option<char> {
    Some(match c {
        'ぁ' => 'a',
        'ぃ' => 'i',
        'ぅ' => 'u',
        'ぇ' => 'e',
        'ぉ' => 'o',
        _ => return None,
    })
}

fn kana_romaji(c: char) -> Option<&'static str> {
    Some(match c {
        'あ' => "a", 'い' => "i", 'う' => "u", 'え' => "e", 'お' => "o",
        'か' => "ka", 'き' => "ki", 'く' => "ku", 'け' => "ke", 'こ' => "ko",
        'さ' => "sa", 'し' => "shi", 'す' => "su", 'せ' => "se", 'そ' => "so",
        'た' => "ta", 'ち' => "chi", 'つ' => "tsu", 'て' => "te", 'と' => "to",
        'な' => "na", 'に' => "ni", 'ぬ' => "nu", 'ね' => "ne", 'の' => "no",
        'は' => "ha", 'ひ' => "hi", 'ふ' => "fu", 'へ' => "he", 'ほ' => "ho",
        'ま' => "ma", 'み' => "mi", 'む' => "mu", 'め' => "me", 'も' => "mo",
        'や' => "ya", 'ゆ' => "yu", 'よ' => "yo",
        'ら' => "ra", 'り' => "ri", 'る' => "ru", 'れ' => "re", 'ろ' => "ro",
        'わ' => "wa", 'を' => "wo", 'ん' => "n",
        'が' => "ga", 'ぎ' => "gi", 'ぐ' => "gu", 'げ' => "ge", 'ご' => "go",
        'ざ' => "za", 'じ' => "ji", 'ず' => "zu", 'ぜ' => "ze", 'ぞ' => "zo",
        'だ' => "da", 'ぢ' => "ji", 'づ' => "zu", 'で' => "de", 'ど' => "do",
        'ば' => "ba", 'び' => "bi", 'ぶ' => "bu", 'べ' => "be", 'ぼ' => "bo",
        'ぱ' => "pa", 'ぴ' => "pi", 'ぷ' => "pu", 'ぺ' => "pe", 'ぽ' => "po",
        'ぁ' => "a", 'ぃ' => "i", 'ぅ' => "u", 'ぇ' => "e", 'ぉ' => "o",
        'ゃ' => "ya", 'ゅ' => "yu", 'ょ' => "yo", 'ゔ' => "vu",
        _ => return None,
    })
}

struct Ime {
    lang: windows::Win32::UI::Input::Ime::IFELanguage,
}

impl Ime {
    fn open() -> Option<Self> {
        use windows::core::w;
        use windows::Win32::System::Com::{
            CLSIDFromProgID, CoCreateInstance, CoInitializeEx, CLSCTX_ALL,
            COINIT_APARTMENTTHREADED,
        };
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let clsid = CLSIDFromProgID(w!("MSIME.Japan")).ok()?;
            let lang: windows::Win32::UI::Input::Ime::IFELanguage =
                CoCreateInstance(&clsid, None, CLSCTX_ALL).ok()?;
            lang.Open().ok()?;
            Some(Self { lang })
        }
    }

    /// Katakana reading of `text` (ASCII parts come back as-is).
    fn phonetic(&self, text: &str) -> Option<String> {
        let input = windows::core::BSTR::from(text);
        let mut out = windows::core::BSTR::new();
        unsafe {
            self.lang.GetPhonetic(&input, 1, -1, &mut out).ok()?;
        }
        let s = to_halfwidth(&out.to_string());
        (!s.is_empty()).then_some(s)
    }
}

impl Drop for Ime {
    fn drop(&mut self) {
        unsafe {
            let _ = self.lang.Close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_kana_to_romaji() {
        assert_eq!(to_romaji("しかくじょうほう"), "shikakujouhou");
        assert_eq!(to_romaji("きっぷ"), "kippu");
        assert_eq!(to_romaji("ちゃ"), "cha");
        assert_eq!(to_romaji("きゃ"), "kya");
        assert_eq!(to_romaji("ふぁいあうぉーる"), "faiaworu");
        assert_eq!(to_romaji("しぇる"), "sheru");
        assert_eq!(to_romaji("でぃすく"), "disuku");
        assert_eq!(to_hiragana("マネージャー"), "まねーじゃー");
    }

    #[test]
    fn ime_reading_when_available() {
        let Some(ime) = Ime::open() else {
            return; // no Japanese IME on this machine
        };
        let r = ime.phonetic("資格情報マネージャー").unwrap();
        assert_eq!(to_hiragana(&r), "しかくじょうほうまねーじゃー");
    }
}
