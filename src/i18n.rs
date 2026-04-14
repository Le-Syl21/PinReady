// Internationalization — language detection, selection, and locale management.
// Uses rust-i18n with TOML locale files in locales/ directory.

/// All supported UI languages with their display names.
pub const LANGUAGE_OPTIONS: &[(&str, &str)] = &[
    ("en", "English"),
    ("fr", "Français"),
    ("es", "Español"),
    ("pt", "Português"),
    ("it", "Italiano"),
    ("de", "Deutsch"),
    ("nl", "Nederlands"),
    ("sv", "Svenska"),
    ("fi", "Suomi"),
    ("pl", "Polski"),
    ("cs", "Čeština"),
    ("sk", "Slovenčina"),
    ("tr", "Türkçe"),
    ("ru", "Русский"),
    ("ar", "العربية"),
    ("hi", "हिन्दी"),
    ("bn", "বাংলা"),
    ("zh", "中文"),
    ("zh_tw", "繁體中文"),
    ("ja", "日本語"),
    ("ko", "한국어"),
    ("id", "Bahasa Indonesia"),
    ("ur", "اردو"),
    ("sw", "Kiswahili"),
    ("vi", "Tiếng Việt"),
    ("th", "ไทย"),
];

/// Detect the system language. Cross-platform:
/// - Linux: reads LANG / LC_ALL / LC_MESSAGES env vars
/// - macOS: reads AppleLocale / AppleLanguages via `defaults read`
/// - Windows: reads GetUserDefaultUILanguage via PowerShell
///
/// Returns the index into LANGUAGE_OPTIONS (default: 0 = English).
pub fn detect_system_language() -> usize {
    let lang_raw = detect_system_language_string();
    match_language_code(&lang_raw)
}

/// Get the raw system language string from the OS.
fn detect_system_language_string() -> String {
    // Try env vars first (works on Linux, sometimes set on macOS)
    if let Ok(lang) = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .or_else(|_| std::env::var("LC_MESSAGES"))
    {
        if !lang.is_empty() && lang != "C" && lang != "POSIX" {
            return lang.to_lowercase();
        }
    }

    // macOS: defaults read -g AppleLocale (returns e.g. "fr_FR")
    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleLocale"])
            .output()
        {
            if output.status.success() {
                let locale = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .to_lowercase();
                if !locale.is_empty() {
                    return locale;
                }
            }
        }
    }

    // Windows: PowerShell to get UI language
    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "(Get-Culture).TwoLetterISOLanguageName",
            ])
            .output()
        {
            if output.status.success() {
                let lang = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .to_lowercase();
                if !lang.is_empty() {
                    return lang;
                }
            }
        }
    }

    String::new()
}

/// Match a raw language string (e.g. "fr_fr.utf-8", "zh_tw", "en") to a
/// LANGUAGE_OPTIONS index.
fn match_language_code(raw: &str) -> usize {
    // Normalize: replace hyphens with underscores
    let normalized = raw.replace('-', "_");

    // Check zh_tw first before falling back to 2-letter code
    if normalized.starts_with("zh_tw") {
        return LANGUAGE_OPTIONS
            .iter()
            .position(|(c, _)| *c == "zh_tw")
            .unwrap_or(0);
    }

    let code = normalized.get(..2).unwrap_or("");
    LANGUAGE_OPTIONS
        .iter()
        .position(|(c, _)| *c == code)
        .unwrap_or(0)
}

/// Set the active locale.
pub fn set_locale(lang: &str) {
    rust_i18n::set_locale(lang);
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- LANGUAGE_OPTIONS ---

    #[test]
    fn language_options_not_empty() {
        assert!(!LANGUAGE_OPTIONS.is_empty());
    }

    #[test]
    fn language_options_starts_with_english() {
        assert_eq!(LANGUAGE_OPTIONS[0], ("en", "English"));
    }

    #[test]
    fn language_options_codes_unique() {
        let mut codes: Vec<&str> = LANGUAGE_OPTIONS.iter().map(|(c, _)| *c).collect();
        let len_before = codes.len();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), len_before, "duplicate language codes found");
    }

    // --- match_language_code ---

    #[test]
    fn match_english() {
        assert_eq!(match_language_code("en"), 0);
        assert_eq!(match_language_code("en_us.utf-8"), 0);
        assert_eq!(match_language_code("en_GB"), 0);
    }

    #[test]
    fn match_french() {
        let idx = LANGUAGE_OPTIONS
            .iter()
            .position(|(c, _)| *c == "fr")
            .unwrap();
        assert_eq!(match_language_code("fr"), idx);
        assert_eq!(match_language_code("fr_FR.UTF-8"), idx);
        assert_eq!(match_language_code("fr_ca"), idx);
    }

    #[test]
    fn match_chinese_simplified() {
        let idx = LANGUAGE_OPTIONS
            .iter()
            .position(|(c, _)| *c == "zh")
            .unwrap();
        assert_eq!(match_language_code("zh_cn"), idx);
        assert_eq!(match_language_code("zh"), idx);
    }

    #[test]
    fn match_chinese_traditional() {
        let idx = LANGUAGE_OPTIONS
            .iter()
            .position(|(c, _)| *c == "zh_tw")
            .unwrap();
        assert_eq!(match_language_code("zh_tw"), idx);
        // detect_system_language_string() lowercases before calling match_language_code
        assert_eq!(match_language_code("zh_tw.utf-8"), idx);
    }

    #[test]
    fn match_hyphenated_locale() {
        let idx = LANGUAGE_OPTIONS
            .iter()
            .position(|(c, _)| *c == "zh_tw")
            .unwrap();
        // Hyphens are replaced with underscores, but input must be lowercase
        // (detect_system_language_string lowercases before calling match_language_code)
        assert_eq!(match_language_code("zh-tw"), idx);
    }

    #[test]
    fn match_unknown_defaults_to_english() {
        assert_eq!(match_language_code("xx"), 0);
        assert_eq!(match_language_code(""), 0);
        assert_eq!(match_language_code("zz_ZZ"), 0);
    }

    #[test]
    fn match_japanese() {
        let idx = LANGUAGE_OPTIONS
            .iter()
            .position(|(c, _)| *c == "ja")
            .unwrap();
        assert_eq!(match_language_code("ja_JP.UTF-8"), idx);
    }

    #[test]
    fn match_arabic() {
        let idx = LANGUAGE_OPTIONS
            .iter()
            .position(|(c, _)| *c == "ar")
            .unwrap();
        assert_eq!(match_language_code("ar_SA"), idx);
    }

    #[test]
    fn match_case_insensitive_input() {
        // detect_system_language_string lowercases, so match_language_code receives lowercase
        let fr_idx = LANGUAGE_OPTIONS
            .iter()
            .position(|(c, _)| *c == "fr")
            .unwrap();
        assert_eq!(match_language_code("fr_fr.utf-8"), fr_idx);
    }
}
