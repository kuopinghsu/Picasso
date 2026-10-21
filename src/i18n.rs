//! Interface language. Text is written in English in the code and looked up
//! in each language's `TABLE` of (English, translation) pairs; `{}` marks a
//! value filled in by `trf`.

mod de;
mod es;
mod fr;
mod it;
mod ja;
mod ko;
mod pt_br;
mod zh_hans;
mod zh_hant;

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    English,
    TraditionalChinese,
    SimplifiedChinese,
    Japanese,
    Korean,
    German,
    Spanish,
    French,
    Italian,
    Portuguese,
}

impl Lang {
    pub const ALL: [Lang; 10] = [
        Lang::English,
        Lang::TraditionalChinese,
        Lang::SimplifiedChinese,
        Lang::Japanese,
        Lang::Korean,
        Lang::German,
        Lang::Spanish,
        Lang::French,
        Lang::Italian,
        Lang::Portuguese,
    ];

    /// Name of the language written in that language, for the picker.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::English => "English",
            Lang::TraditionalChinese => "繁體中文",
            Lang::SimplifiedChinese => "简体中文",
            Lang::Japanese => "日本語",
            Lang::Korean => "한국어",
            Lang::German => "Deutsch",
            Lang::Spanish => "Español",
            Lang::French => "Français",
            Lang::Italian => "Italiano",
            Lang::Portuguese => "Português (Brasil)",
        }
    }

    /// Value stored in the `language` setting.
    pub fn code(self) -> &'static str {
        match self {
            Lang::English => "en",
            Lang::TraditionalChinese => "zh-Hant",
            Lang::SimplifiedChinese => "zh-Hans",
            Lang::Japanese => "ja",
            Lang::Korean => "ko",
            Lang::German => "de",
            Lang::Spanish => "es",
            Lang::French => "fr",
            Lang::Italian => "it",
            Lang::Portuguese => "pt-BR",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|lang| lang.code() == code)
    }

    /// Reads a system locale such as `zh-TW`, `zh-Hant-HK`, `zh_CN.UTF-8`,
    /// or `fr-CA`. Chinese without a script or region reads as Simplified,
    /// and every Portuguese locale gets the Brazilian text.
    pub fn from_locale(locale: &str) -> Option<Lang> {
        let tag = locale.split(['.', '@']).next()?.replace('_', "-").to_ascii_lowercase();
        let mut parts = tag.split('-');
        match parts.next()? {
            "en" => Some(Lang::English),
            "ja" => Some(Lang::Japanese),
            "ko" => Some(Lang::Korean),
            "de" => Some(Lang::German),
            "es" => Some(Lang::Spanish),
            "fr" => Some(Lang::French),
            "it" => Some(Lang::Italian),
            "pt" => Some(Lang::Portuguese),
            "zh" => {
                let traditional = parts.any(|part| matches!(part, "hant" | "tw" | "hk" | "mo"));
                Some(if traditional { Lang::TraditionalChinese } else { Lang::SimplifiedChinese })
            }
            _ => None,
        }
    }

    /// (English, translation) pairs. English itself has none.
    fn strings(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Lang::English => &[],
            Lang::TraditionalChinese => zh_hant::TABLE,
            Lang::SimplifiedChinese => zh_hans::TABLE,
            Lang::Japanese => ja::TABLE,
            Lang::Korean => ko::TABLE,
            Lang::German => de::TABLE,
            Lang::Spanish => es::TABLE,
            Lang::French => fr::TABLE,
            Lang::Italian => it::TABLE,
            Lang::Portuguese => pt_br::TABLE,
        }
    }

    fn index(self) -> usize {
        Lang::ALL.iter().position(|item| *item == self).unwrap_or(0)
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn current() -> Lang {
    Lang::ALL[CURRENT.load(Ordering::Relaxed) as usize % Lang::ALL.len()]
}

pub fn set(lang: Lang) {
    CURRENT.store(lang.index() as u8, Ordering::Relaxed);
}

/// The first preferred system language Picasso has text for, else English.
pub fn system() -> Lang {
    sys_locale::get_locales().find_map(|locale| Lang::from_locale(&locale)).unwrap_or(Lang::English)
}

/// `text` in the current language. Text without a translation stays English.
pub fn tr(text: &'static str) -> &'static str {
    let lang = current();
    if lang == Lang::English {
        return text;
    }
    table(lang).get(text).copied().unwrap_or(text)
}

/// `tr(text)` with each `{}` replaced by the next value in `args`.
pub fn trf(text: &'static str, args: &[&dyn Display]) -> String {
    let mut out = String::new();
    let mut args = args.iter();
    let mut parts = tr(text).split("{}").peekable();
    while let Some(part) = parts.next() {
        out.push_str(part);
        if parts.peek().is_some() {
            if let Some(arg) = args.next() {
                out.push_str(&arg.to_string());
            }
        }
    }
    out
}

/// Lookup map for `lang`, built the first time the language is used.
fn table(lang: Lang) -> &'static HashMap<&'static str, &'static str> {
    static MAPS: [OnceLock<HashMap<&'static str, &'static str>>; Lang::ALL.len()] =
        [const { OnceLock::new() }; Lang::ALL.len()];
    MAPS[lang.index()].get_or_init(|| lang.strings().iter().copied().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_system_locales() {
        assert_eq!(Lang::from_locale("zh-TW"), Some(Lang::TraditionalChinese));
        assert_eq!(Lang::from_locale("zh-Hant-HK"), Some(Lang::TraditionalChinese));
        assert_eq!(Lang::from_locale("zh_CN.UTF-8"), Some(Lang::SimplifiedChinese));
        assert_eq!(Lang::from_locale("zh-Hans-SG"), Some(Lang::SimplifiedChinese));
        assert_eq!(Lang::from_locale("ja-JP"), Some(Lang::Japanese));
        assert_eq!(Lang::from_locale("ko-KR"), Some(Lang::Korean));
        assert_eq!(Lang::from_locale("de_AT.UTF-8"), Some(Lang::German));
        assert_eq!(Lang::from_locale("es-419"), Some(Lang::Spanish));
        assert_eq!(Lang::from_locale("fr-CA"), Some(Lang::French));
        assert_eq!(Lang::from_locale("it-CH"), Some(Lang::Italian));
        assert_eq!(Lang::from_locale("pt-PT"), Some(Lang::Portuguese));
        assert_eq!(Lang::from_locale("en-US"), Some(Lang::English));
        assert_eq!(Lang::from_locale("nl-NL"), None);
        assert_eq!(Lang::from_code("zh-Hant"), Some(Lang::TraditionalChinese));
        assert_eq!(Lang::from_code("pt-BR"), Some(Lang::Portuguese));
    }

    #[test]
    fn every_language_translates_the_same_text() {
        let reference: Vec<&str> = zh_hant::TABLE.iter().map(|(en, _)| *en).collect();
        let unique: std::collections::HashSet<&str> = reference.iter().copied().collect();
        assert_eq!(unique.len(), reference.len(), "duplicate English text");
        for lang in Lang::ALL.into_iter().filter(|lang| *lang != Lang::English) {
            let keys: Vec<&str> = lang.strings().iter().map(|(en, _)| *en).collect();
            for en in &reference {
                assert!(keys.contains(en), "{lang:?} is missing {en:?}");
            }
            for en in &keys {
                assert!(unique.contains(en), "{lang:?} has unknown text {en:?}");
            }
            assert_eq!(keys.len(), reference.len(), "{lang:?} has duplicate rows");
        }
    }

    #[test]
    fn translations_keep_placeholders() {
        for lang in Lang::ALL {
            for (en, text) in lang.strings() {
                assert_eq!(text.matches("{}").count(), en.matches("{}").count(), "{lang:?}: {text:?}");
                assert!(!text.trim().is_empty(), "{lang:?} leaves {en:?} empty");
            }
        }
    }

    #[test]
    fn fills_placeholders_in_order() {
        assert_eq!(trf("Can't watch {}: {}", &[&"D:\\Photos", &"denied"]), "Can't watch D:\\Photos: denied");
        assert_eq!(trf("{} photos", &[&3]), "3 photos");
    }
}
