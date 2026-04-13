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
