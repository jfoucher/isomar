//! Translations through gettext. Catalogs live in `po/`, maintained with
//! `scripts/i18n.sh`.
//!
//! Messages with runtime values use named `{placeholders}` so translators can
//! reorder them: `gettext_f("Delete {name}?", &[("name", &route)])`.

use std::path::PathBuf;

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android")))]
pub use gettextrs::{gettext, ngettext, pgettext};
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "android"))]
pub use mo::{gettext, ngettext, pgettext};

pub const DOMAIN: &str = "isomar";

/// Sets up the locale and the message catalog. Call before building any UI.
#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android")))]
pub fn init() {
    // SAFETY: called first thing in main, before any other thread exists.
    unsafe {
        gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");
    }
    let _ = gettextrs::bindtextdomain(DOMAIN, locale_dir());
    let _ = gettextrs::bind_textdomain_codeset(DOMAIN, "UTF-8");
    let _ = gettextrs::textdomain(DOMAIN);
}

/// Loads the catalogue of the user's first language that has one (`LANGUAGE` is a colon-separated list).
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "android"))]
pub fn init() {
    let langs = std::env::var("LANGUAGE").or_else(|_| std::env::var("LANG")).unwrap_or_default();
    mo::load(&locale_dir(), &langs);
}

/// A reader of `.mo` catalogues, for macOS, iOS and Android: they have no libintl, so the app bundles no C library for it.
/// Plurals use the rule of most European languages (one form for 1, another otherwise).
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "android", test))]
// Elsewhere it is built for its tests alone, which read catalogues but look nothing up.
#[cfg_attr(not(any(target_os = "macos", target_os = "ios", target_os = "android")), allow(dead_code))]
mod mo {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::OnceLock;

    static CATALOG: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();

    /// The entries of a `.mo` file: the original (with its plural after a NUL, and a context before an EOT)
    /// and its translations.
    pub fn parse(data: &[u8]) -> Option<HashMap<String, Vec<String>>> {
        let word = |at: usize, big: bool| -> Option<u32> {
            let b: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
            Some(if big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
        };
        let big = match word(0, false)? {
            0x9504_12de => false,
            0xde12_0495 => true,
            _ => return None,
        };
        let (n, originals, translations) = (word(8, big)? as usize, word(12, big)? as usize, word(16, big)? as usize);
        let text = |table: usize, i: usize| -> Option<&[u8]> {
            let (len, off) = (word(table + 8 * i, big)? as usize, word(table + 8 * i + 4, big)? as usize);
            data.get(off..off + len)
        };
        let mut out = HashMap::new();
        for i in 0..n {
            let key = String::from_utf8_lossy(text(originals, i)?).into_owned();
            let value: Vec<String> = String::from_utf8_lossy(text(translations, i)?).split('\0').map(str::to_string).collect();
            // The first entry is the catalogue's header.
            if !key.is_empty() {
                out.insert(key, value);
            }
        }
        Some(out)
    }

    pub fn load(dir: &Path, languages: &str) {
        for lang in languages.split(':').map(|l| l.split('.').next().unwrap_or("")).filter(|l| !l.is_empty() && *l != "C") {
            // `fr_FR` also tries `fr`.
            for name in [lang, lang.split('_').next().unwrap_or(lang)] {
                if let Ok(data) = std::fs::read(dir.join(name).join("LC_MESSAGES").join(format!("{}.mo", super::DOMAIN)))
                    && let Some(c) = parse(&data)
                {
                    let _ = CATALOG.set(c);
                    return;
                }
            }
        }
    }

    fn lookup(key: &str, form: usize) -> Option<String> {
        CATALOG.get()?.get(key)?.get(form).filter(|s| !s.is_empty()).cloned()
    }

    pub fn gettext(msgid: &str) -> String {
        lookup(msgid, 0).unwrap_or_else(|| msgid.to_string())
    }

    pub fn ngettext(msgid: &str, plural: &str, n: u32) -> String {
        let form = usize::from(n != 1);
        lookup(&format!("{msgid}\0{plural}"), form).unwrap_or_else(|| if n == 1 { msgid } else { plural }.to_string())
    }

    pub fn pgettext(context: &str, msgid: &str) -> String {
        lookup(&format!("{context}\u{4}{msgid}"), 0).unwrap_or_else(|| msgid.to_string())
    }

    #[cfg(test)]
    pub fn mo_bytes(entries: &[(&str, &str)]) -> Vec<u8> {
        // Little-endian, no hash table: header, two tables, then the strings.
        let n = entries.len() as u32;
        let mut strings = Vec::new();
        let mut originals = Vec::new();
        let mut translations = Vec::new();
        let base = 28 + 16 * n;
        for (k, v) in entries {
            originals.push((k.len() as u32, base + strings.len() as u32));
            strings.extend_from_slice(k.as_bytes());
            strings.push(0);
            translations.push((v.len() as u32, base + strings.len() as u32));
            strings.extend_from_slice(v.as_bytes());
            strings.push(0);
        }
        let mut out = Vec::new();
        for w in [0x9504_12de_u32, 0, n, 28, 28 + 8 * n, 0, 0] {
            out.extend(w.to_le_bytes());
        }
        for (l, o) in originals.iter().chain(&translations) {
            out.extend(l.to_le_bytes());
            out.extend(o.to_le_bytes());
        }
        out.extend(strings);
        out
    }
}

/// The user's language as Natural Earth names it ("fr", "pt", "zht"…), "en" when none is set.
pub fn language() -> &'static str {
    static LANG: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    LANG.get_or_init(|| {
        // The first of these that is set and is a language: `LANGUAGE` is a list, the others one locale.
        let from_env = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|k| std::env::var(k).ok()).map(|v| language_from(&v)).find(|l| !l.is_empty());
        from_env.unwrap_or_else(|| "en".to_string())
    })
}

/// The language of a locale or of a `LANGUAGE` list (its first item): `fr_FR.UTF-8` is `fr`, `zh_TW` is
/// `zht`; empty for none, `C` and `POSIX`.
fn language_from(value: &str) -> String {
    let first = value.split(':').next().unwrap_or("").trim();
    let locale = first.split(['.', '@']).next().unwrap_or("");
    if matches!(locale, "" | "C" | "POSIX") {
        return String::new();
    }
    if matches!(locale, "zh_TW" | "zh_HK") {
        return "zht".to_string();
    }
    locale.split('_').next().unwrap_or("").to_lowercase()
}

/// `ISOMAR_LOCALEDIR`, else `<prefix>/share/locale` next to the executable's
/// `bin/` (covers ~/.local, /usr, /app in Flatpak and `target/share/locale`
/// for development builds).
fn locale_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ISOMAR_LOCALEDIR") {
        return dir.into();
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.parent()?.join("share/locale")))
        .unwrap_or_else(|| "/usr/share/locale".into())
}

fn substitute(mut s: String, args: &[(&str, &str)]) -> String {
    for (key, value) in args {
        s = s.replace(&format!("{{{key}}}"), value);
    }
    s
}

/// Translates `msgid` and fills in its `{name}` placeholders.
pub fn gettext_f(msgid: &str, args: &[(&str, &str)]) -> String {
    substitute(gettext(msgid), args)
}

/// Plural form of [`gettext_f`]; `{n}` is replaced by `n`.
pub fn ngettext_f(msgid: &str, msgid_plural: &str, n: u32, args: &[(&str, &str)]) -> String {
    let n_str = n.to_string();
    let mut all = vec![("n", n_str.as_str())];
    all.extend_from_slice(args);
    substitute(ngettext(msgid, msgid_plural, n), &all)
}

/// Marks a string for extraction without translating it, for tables that are
/// translated where they are displayed.
#[allow(non_snake_case)]
pub const fn N_(msgid: &str) -> &str {
    msgid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_catalogue_is_read_with_plurals_and_contexts() {
        let data = mo::mo_bytes(&[("", "Content-Type: text/plain; charset=UTF-8\n"), ("Hello", "Bonjour"), ("{n} point\0{n} points", "{n} point\0{n} points (fr)"), ("menu\u{4}Open", "Ouvrir")]);
        let c = mo::parse(&data).unwrap();
        assert_eq!(c["Hello"], ["Bonjour"]);
        assert_eq!(c["{n} point\0{n} points"], ["{n} point", "{n} points (fr)"]);
        assert_eq!(c["menu\u{4}Open"], ["Ouvrir"]);
        assert!(!c.contains_key(""), "the header is not a message");
        assert!(mo::parse(b"not a catalogue").is_none());
    }

    #[test]
    fn the_language_is_read_from_a_locale() {
        assert_eq!(language_from("fr_FR.UTF-8"), "fr");
        assert_eq!(language_from("pt_BR"), "pt");
        assert_eq!(language_from("de_DE@euro"), "de");
        assert_eq!(language_from("zh_TW.UTF-8"), "zht");
        assert_eq!(language_from("zh_CN"), "zh");
        assert_eq!(language_from("es:fr"), "es");
        assert_eq!(language_from("C"), "");
        assert_eq!(language_from("POSIX"), "");
        assert_eq!(language_from(""), "");
        assert_eq!(language_from("C.UTF-8"), "");
        assert!(!language().is_empty());
    }

    #[test]
    fn placeholders_are_substituted() {
        assert_eq!(
            substitute("Go from {a} to {b}".into(), &[("a", "x"), ("b", "y")]),
            "Go from x to y"
        );
        assert_eq!(ngettext_f("{n} waypoint", "{n} waypoints", 3, &[]), "3 waypoints");
    }
}
