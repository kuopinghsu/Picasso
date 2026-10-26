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
    fn asian_languages_leave_only_names_in_english() {
        let names = ["HDR", "Reinhard", "Drago", "Kodachrome"];
        for lang in [Lang::TraditionalChinese, Lang::SimplifiedChinese, Lang::Japanese, Lang::Korean] {
            for (en, text) in lang.strings() {
                let words = en.split(|c: char| !c.is_alphabetic()).filter(|word| word.len() > 1);
                if en == text && words.clone().count() > 0 {
                    assert!(words.into_iter().all(|word| names.contains(&word)), "{lang:?} leaves {en:?} in English");
                }
            }
        }
    }

    /// Words with more than one Traditional Chinese form, and the one Picasso
    /// uses in the window and in README_zh-TW.md. The others are mostly the
    /// words of mainland China, and appear when text is copied from the
    /// Simplified Chinese table.
    const ZH_HANT_TERMS: &[(&str, &str)] = &[
        ("相片庫", "圖庫"),
        ("照片庫", "圖庫"),
        ("媒體庫", "圖庫"),
        ("照片", "相片"),
        ("縮略圖", "縮圖"),
        ("略縮圖", "縮圖"),
        ("文件夾", "資料夾"),
        ("元數據", "中繼資料"),
        ("元資料", "中繼資料"),
        ("無損編輯", "非破壞性編輯"),
        ("航跡", "軌跡"),
        ("回收站", "垃圾桶"),
        ("屏幕", "螢幕"),
        ("視頻", "影片"),
        ("默認", "預設"),
        ("菜單", "選單"),
        ("鼠標", "滑鼠"),
        ("軟件", "軟體"),
        ("信息", "訊息"),
        ("網絡", "網路"),
        ("用戶", "使用者"),
        ("數據", "資料"),
        ("質量", "品質"),
        ("打印", "列印"),
        ("搜索", "搜尋"),
        ("導出", "匯出"),
        ("導入", "匯入"),
        ("支持", "支援"),
        ("設置", "設定"),
        ("窗口", "視窗"),
        ("緩存", "快取"),
        ("字體", "字型"),
        ("分辨率", "解析度"),
    ];
    /// Simplified characters that have a different Traditional form.
    const SIMPLIFIED_ONLY: &str = "图张设选时个这们为动无开关删编辑档夹缩览页键录视频显据库标";

    fn zh_hant_wording(source: &str, text: &str) -> Vec<String> {
        let mut found = Vec::new();
        for (word, standard) in ZH_HANT_TERMS {
            if text.contains(word) {
                found.push(format!("{source}: {word} should be {standard}: {text}"));
            }
        }
        if let Some(c) = text.chars().find(|c| SIMPLIFIED_ONLY.contains(*c)) {
            found.push(format!("{source}: Simplified {c}: {text}"));
        }
        found
    }

    #[test]
    fn traditional_chinese_uses_one_word_for_each_thing() {
        let mut found: Vec<String> = zh_hant::TABLE.iter().flat_map(|(_, text)| zh_hant_wording("window", text)).collect();
        let readme = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("README_zh-TW.md")).unwrap();
        for (line, text) in readme.lines().enumerate() {
            found.extend(zh_hant_wording(&format!("README_zh-TW.md:{}", line + 1), text));
        }
        assert!(found.is_empty(), "{}", found.join("\n"));
    }

    /// The headings, list items, table rows, and code blocks of a Markdown
    /// file in order, and every `code` span in it, sorted.
    fn markdown_outline(name: &str) -> (Vec<String>, Vec<String>) {
        let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(name)).unwrap();
        let mut outline = Vec::new();
        let mut spans = Vec::new();
        let mut in_code = false;
        for line in text.lines() {
            if line.starts_with("```") {
                in_code = !in_code;
                outline.push("code block".to_string());
                continue;
            }
            if in_code {
                continue;
            }
            if line.starts_with('#') {
                outline.push(line.split(' ').next().unwrap_or_default().to_string());
            } else if line.starts_with("- ") {
                outline.push("list item".into());
            } else if line.starts_with('|') {
                outline.push(format!("table row of {} cells", line.matches('|').count() - 1));
            }
            spans.extend(line.split('`').skip(1).step_by(2).map(str::to_string));
        }
        spans.sort();
        (outline, spans)
    }

    #[test]
    fn the_two_readmes_say_the_same_things() {
        let (en_outline, en_spans) = markdown_outline("README.md");
        let (zh_outline, zh_spans) = markdown_outline("README_zh-TW.md");
        assert_eq!(en_outline, zh_outline, "README.md and README_zh-TW.md have different sections, lists, or tables");
        let only = |a: &[String], b: &[String]| -> Vec<String> {
            let mut rest = b.to_vec();
            a.iter()
                .filter(|span| match rest.iter().position(|other| other == *span) {
                    Some(at) => {
                        rest.remove(at);
                        false
                    }
                    None => true,
                })
                .cloned()
                .collect()
        };
        assert!(
            en_spans == zh_spans,
            "code spans differ\nonly in README.md: {:?}\nonly in README_zh-TW.md: {:?}",
            only(&en_spans, &zh_spans),
            only(&zh_spans, &en_spans)
        );
    }

    /// Text in each `tr("…")` and `trf("…", …)` call of the source files.
    fn source_texts() -> Vec<(String, String)> {
        fn walk(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|name| name != "i18n") {
                        walk(&path, files);
                    }
                } else if path.extension().is_some_and(|ext| ext == "rs") && !path.ends_with("src/i18n.rs") {
                    files.push(path);
                }
            }
        }
        let mut files = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        let mut found = Vec::new();
        for file in files {
            let code = std::fs::read_to_string(&file).unwrap();
            let name = file.file_name().unwrap().to_string_lossy().into_owned();
            for call in ["tr(", "trf("] {
                for (at, _) in code.match_indices(call) {
                    let before = code[..at].chars().next_back();
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ':') && !code[..at].ends_with("i18n::") {
                        continue;
                    }
                    let rest = code[at + call.len()..].trim_start();
                    let Some(literal) = rest.strip_prefix('"') else { continue };
                    let mut text = String::new();
                    let mut chars = literal.chars();
                    while let Some(c) = chars.next() {
                        match c {
                            '"' => break,
                            '\\' => match chars.next() {
                                Some('n') => text.push('\n'),
                                Some(other) => text.push(other),
                                None => break,
                            },
                            other => text.push(other),
                        }
                    }
                    found.push((name.clone(), text));
                }
            }
        }
        found
    }

    #[test]
    fn every_text_in_the_source_is_translated() {
        let known: std::collections::HashSet<&str> = zh_hant::TABLE.iter().map(|(en, _)| *en).collect();
        let texts = source_texts();
        assert!(texts.len() > 300, "the source scan found only {} texts", texts.len());
        let missing: Vec<String> = texts
            .into_iter()
            .filter(|(_, text)| text.replace("{} px", "").chars().any(char::is_alphabetic) && !known.contains(text.as_str()))
            .map(|(file, text)| format!("{file}: {text:?}"))
            .collect();
        assert!(missing.is_empty(), "not in the translation tables:\n{}", missing.join("\n"));
    }

    #[test]
    fn fills_placeholders_in_order() {
        assert_eq!(trf("Can't watch {}: {}", &[&"D:\\Photos", &"denied"]), "Can't watch D:\\Photos: denied");
        assert_eq!(trf("{} photos", &[&3]), "3 photos");
    }
}
