//! What a table says about itself, for the cover windows: its name, maker and
//! year, version, author, dates, and how often it was played.
//!
//! Read from the table information stored in the `.vpx` (a few small streams,
//! not the whole file) and from the `<table>.info` file Visual Pinball keeps
//! next to it. Reading happens on a worker thread (see `app::cover_info`):
//! the selection changes fast and a `.vpx` can be hundreds of megabytes.

use std::path::Path;

/// The text shown on a cover.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TableInfo {
    /// Name of the table.
    pub title: String,
    /// Maker and year, or empty.
    pub subtitle: String,
    /// One fact per line (author, version, dates, plays).
    pub lines: Vec<String>,
}

/// Labels of the facts, in the language of the interface.
#[derive(Debug, Clone)]
pub struct Labels {
    pub author: String,
    pub version: String,
    pub saved: String,
    pub released: String,
    pub played: String,
    pub last_played: String,
}

/// A date as `2025-02-09`, whichever way Visual Pinball wrote it: an ISO
/// timestamp (`2026-11-12T05:05:39Z`), a C-style date (`Sun Feb  9 22:54:51
/// 2025`), or `MM/DD/YYYY` when the day cannot be mistaken for a month.
/// Anything else is kept as it is.
fn day(text: &str) -> String {
    use regex::Regex;
    use std::sync::OnceLock;
    static C_STYLE: OnceLock<Regex> = OnceLock::new();
    static SLASHES: OnceLock<Regex> = OnceLock::new();
    let t = text.trim();
    if let Some((d, _)) = t.split_once('T')
        && d.len() == 10
    {
        return d.to_string();
    }
    let c_style = C_STYLE.get_or_init(|| {
        Regex::new(r"^[A-Za-z]{3} ([A-Za-z]{3}) +(\d{1,2}) \d\d:\d\d:\d\d (\d{4})$").unwrap()
    });
    if let Some(c) = c_style.captures(t) {
        const MONTHS: [&str; 12] = [
            "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
        ];
        if let Some(m) = MONTHS.iter().position(|m| m.eq_ignore_ascii_case(&c[1])) {
            let d: u32 = c[2].parse().unwrap_or(0);
            return format!("{}-{:02}-{:02}", &c[3], m + 1, d);
        }
    }
    let slashes = SLASHES.get_or_init(|| Regex::new(r"^(\d{1,2})/(\d{1,2})/(\d{4})$").unwrap());
    if let Some(c) = slashes.captures(t) {
        let (a, b): (u32, u32) = (c[1].parse().unwrap_or(0), c[2].parse().unwrap_or(0));
        // only when one of the two cannot be a month is the order certain
        if (1..=12).contains(&a) && b > 12 && b <= 31 {
            return format!("{}-{:02}-{:02}", &c[3], a, b);
        }
        if (1..=12).contains(&b) && a > 12 && a <= 31 {
            return format!("{}-{:02}-{:02}", &c[3], b, a);
        }
    }
    t.to_string()
}

/// `value` unless it is empty or only a placeholder.
fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim_matches('\0').trim().to_string())
        .filter(|v| !v.is_empty())
}

/// The facts of the table at `vpx`; `fallback_name` is shown when the file
/// carries no name.
pub fn read(vpx: &Path, fallback_name: &str, labels: &Labels) -> TableInfo {
    let mut info = TableInfo {
        title: fallback_name.to_string(),
        ..TableInfo::default()
    };
    let mut maker = String::new();
    let mut year = String::new();
    if let Ok(t) = vpin::vpx::open(vpx).and_then(|mut v| v.read_tableinfo()) {
        let prop = |name: &str| {
            clean(
                t.properties
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(name))
                    .map(|(_, v)| v.clone()),
            )
            .unwrap_or_default()
        };
        if let Some(name) = clean(t.table_name.clone()) {
            info.title = name;
        }
        maker = prop("CompanyName");
        year = prop("CompanyYear");
        if let Some(a) = clean(t.author_name.clone()) {
            info.lines.push(format!("{}: {a}", labels.author));
        }
        if let Some(v) = clean(t.table_version.clone()) {
            info.lines.push(format!("{}: {v}", labels.version));
        }
        if let Some(d) = clean(t.table_save_date.clone()) {
            info.lines.push(format!("{}: {}", labels.saved, day(&d)));
        }
        if let Some(d) = clean(t.release_date.clone()) {
            info.lines.push(format!("{}: {}", labels.released, day(&d)));
        }
    }
    info.subtitle = match (maker.is_empty(), year.is_empty()) {
        (false, false) => format!("{maker} · {year}"),
        (false, true) => maker,
        (true, false) => year,
        (true, true) => String::new(),
    };
    // What Visual Pinball counted: how many times, when last.
    if let Ok(text) = std::fs::read_to_string(vpx.with_extension("info"))
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
    {
        let run = &v["VPX"];
        if let Some(n) = run["RunCount"].as_u64().filter(|n| *n > 0) {
            info.lines.push(format!("{}: {n}", labels.played));
        }
        if let Some(last) = run["LastRun"].as_str().filter(|s| !s.is_empty()) {
            info.lines
                .push(format!("{}: {}", labels.last_played, day(last)));
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels() -> Labels {
        Labels {
            author: "Author".into(),
            version: "Version".into(),
            saved: "Saved".into(),
            released: "Released".into(),
            played: "Played".into(),
            last_played: "Last played".into(),
        }
    }

    #[test]
    fn a_timestamp_is_cut_to_its_day() {
        assert_eq!(day("2026-11-12T05:05:39Z"), "2026-11-12");
        assert_eq!(day(" 2022-03-17 "), "2022-03-17");
        // as Visual Pinball writes the dates of a table
        assert_eq!(day("Sun Feb  9 22:54:51 2025"), "2025-02-09");
        assert_eq!(day("02/26/2021"), "2021-02-26");
        // a date that could be read both ways is left alone
        assert_eq!(day("03/04/2021"), "03/04/2021");
        assert_eq!(day("Thu Mar 17 2022"), "Thu Mar 17 2022");
    }

    #[test]
    fn a_missing_file_still_gives_the_name() {
        let info = read(
            Path::new("/nonexistent/Some Table.vpx"),
            "Some Table",
            &labels(),
        );
        assert_eq!(info.title, "Some Table");
        assert!(info.lines.is_empty());
        assert!(info.subtitle.is_empty());
    }

    #[test]
    fn the_info_file_gives_the_plays() {
        let dir = tempfile::tempdir().unwrap();
        let vpx = dir.path().join("T.vpx");
        std::fs::write(
            vpx.with_extension("info"),
            r#"{"VPX":{"Version":"1.1","RunCount":11,"LastRun":"2026-11-12T05:05:39Z","RunTime":4007}}"#,
        )
        .unwrap();
        let info = read(&vpx, "T", &labels());
        assert!(info.lines.contains(&"Played: 11".to_string()));
        assert!(info.lines.contains(&"Last played: 2026-11-12".to_string()));
    }

    #[test]
    fn every_language_has_the_labels_of_the_covers_and_the_sort() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");
        let keys = [
            "tiles_progress",
            "tiles_clearing",
            "info_author",
            "info_version",
            "info_saved",
            "info_released",
            "info_played",
            "info_last_played",
            "info_favorite",
            "info_my_rating",
            "info_community",
            "info_votes",
            "info_no_backglass",
            "sort_favorites",
            "sort_rating",
            "sort_community",
            "sort_last_played",
            "sort_most_played",
        ];
        let mut files = 0;
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            files += 1;
            let text = std::fs::read_to_string(&path).unwrap();
            for key in keys {
                assert!(
                    text.lines().any(|l| l.starts_with(&format!("{key} = \""))),
                    "{} lacks {key}",
                    path.display()
                );
            }
        }
        assert!(files >= 26, "{files} language files");
    }
}
