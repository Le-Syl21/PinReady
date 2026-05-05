//! Match an installed table against the VPS catalog.
//!
//! Strategy chain (ordered by trust):
//!   1. **High** — exact ROM-name match (`cGameName` from the embedded
//!      VBS == `RomFile.version` of some `Game`). PinMAME romnames are
//!      stable across mods/forks of a given machine, so this is the
//!      strongest fingerprint we have.
//!   2. **Medium** — ROM-name fuzzy: try stripping common version
//!      suffixes (`_l7`, `_113b`, `_lh6`, …) so a table whose VBS asks
//!      for `mm_109c` still matches the canonical `mm_l5` entry.
//!   3. **Medium** — directb2s `<GameName>` value matches catalog
//!      `RomFile.version` for some `Game`. The B2S server is independent
//!      from VPM so this catches tables that don't use PinMAME but ship
//!      a B2S anyway.
//!   4. **High** — embedded `TableInfo.TableName` from the `.vpx` matches
//!      a `Game.name` exactly after normalisation (lowercase, alnum only).
//!      The author-set TableName is far more reliable than a folder name
//!      that may be localised ("2001 L'Odyssée de l'Espace" → "2001 - A
//!      Space Odyssey").
//!   5. **Medium** — TableName substring match (either direction).
//!   6. **Low** — basename fuzzy on the table folder name vs `Game.name`.
//!      Cheap last-resort, often catches non-localised cabs.
//!
//! The caller fingers a confidence level so the UI can hide low-confidence
//! suggestions until the user confirms (planned).

use crate::vpsdb::models::Game;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchConfidence {
    Low,
    Medium,
    High,
}

impl std::fmt::Display for MatchConfidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatchConfidence::Low => write!(f, "low"),
            MatchConfidence::Medium => write!(f, "medium"),
            MatchConfidence::High => write!(f, "high"),
        }
    }
}

/// What we resolved.
#[derive(Debug, Clone)]
pub struct MatchResult<'a> {
    pub game: &'a Game,
    pub confidence: MatchConfidence,
    pub strategy: &'static str,
}

/// Do the match. `rom_name` is the embedded `cGameName` from the VBS
/// (we already extract it for [`crate::vbs_patches`]). `b2s_game_name`
/// comes from [`directb2s::read`]. `table_name` is the author-set
/// `TableInfo.TableName` baked into the `.vpx`. `folder_name` is the
/// last-resort fallback.
///
/// Returns `None` only when every strategy strikes out — in practice
/// this is rare because most tables hit `High` via ROM name.
pub fn match_table<'a>(
    games: &'a [Game],
    rom_name: Option<&str>,
    b2s_game_name: Option<&str>,
    table_name: Option<&str>,
    folder_name: &str,
) -> Option<MatchResult<'a>> {
    // Pre-compute the TableName-exact candidate so the ROM strategies
    // can cross-check against it. Original/MOD tables routinely embed
    // a *fake* `cGameName` (lifted from a real PinMAME table they
    // forked from) — e.g. "Batman '66" carrying `flash_l1`, which then
    // wins ROM-exact against Williams Flash 1979 in High confidence.
    // The author-set `TableInfo.TableName` is more reliable; if it
    // points at a different game, we override the ROM match.
    let tablename_exact: Option<&Game> = table_name.and_then(|tn| find_by_name_exact(games, tn));

    // Strategy 1: exact ROM name.
    if let Some(rom) = rom_name {
        if let Some(g) = find_by_rom_exact(games, rom) {
            // Cross-check: TableName disagrees with ROM → trust TableName.
            if let Some(tn_g) = tablename_exact {
                if tn_g.id != g.id {
                    return Some(MatchResult {
                        game: tn_g,
                        confidence: MatchConfidence::High,
                        strategy: "tablename_overrides_rom",
                    });
                }
            }
            return Some(MatchResult {
                game: g,
                confidence: MatchConfidence::High,
                strategy: "rom_exact",
            });
        }
        // Strategy 2: rom name minus revision suffix
        if let Some(g) = find_by_rom_fuzzy(games, rom) {
            if let Some(tn_g) = tablename_exact {
                if tn_g.id != g.id {
                    return Some(MatchResult {
                        game: tn_g,
                        confidence: MatchConfidence::High,
                        strategy: "tablename_overrides_rom",
                    });
                }
            }
            return Some(MatchResult {
                game: g,
                confidence: MatchConfidence::Medium,
                strategy: "rom_fuzzy",
            });
        }
    }

    // Strategy 3: B2S declared GameName. Same cross-check as ROM —
    // .directb2s files are cargo-culted between author MODs as often
    // as cGameName.
    if let Some(b2s) = b2s_game_name {
        if let Some(g) = find_by_rom_exact(games, b2s) {
            if let Some(tn_g) = tablename_exact {
                if tn_g.id != g.id {
                    return Some(MatchResult {
                        game: tn_g,
                        confidence: MatchConfidence::High,
                        strategy: "tablename_overrides_b2s",
                    });
                }
            }
            return Some(MatchResult {
                game: g,
                confidence: MatchConfidence::Medium,
                strategy: "b2s_rom_exact",
            });
        }
    }

    // Strategy 4: embedded TableName, exact normalised match. Trust
    // the author's metadata over folder names (which are often
    // user-localised: "2001 L'Odyssée de l'Espace" hides the catalog
    // entry "2001 - A Space Odyssey").
    if let Some(tn) = table_name {
        if let Some(g) = find_by_name_exact(games, tn) {
            return Some(MatchResult {
                game: g,
                confidence: MatchConfidence::High,
                strategy: "tablename_exact",
            });
        }
        // Strategy 5: TableName substring (either direction).
        if let Some(g) = find_by_name_fuzzy(games, tn) {
            return Some(MatchResult {
                game: g,
                confidence: MatchConfidence::Medium,
                strategy: "tablename_fuzzy",
            });
        }
    }

    // Strategy 6: folder-name fuzzy.
    if let Some(g) = find_by_name_fuzzy(games, folder_name) {
        return Some(MatchResult {
            game: g,
            confidence: MatchConfidence::Low,
            strategy: "name_fuzzy",
        });
    }

    None
}

/// Helpers — convenience overloads when the caller already has the
/// metadata loaded for a single table.
pub fn match_table_from_paths<'a>(
    games: &'a [Game],
    vpx_path: &Path,
    folder_name: &str,
) -> Option<MatchResult<'a>> {
    let (rom_name, table_name) = read_vpx_meta(vpx_path);
    let b2s_name = vpx_path
        .with_extension("directb2s")
        .canonicalize()
        .ok()
        .and_then(|p| read_b2s_game_name(&p));
    match_table(
        games,
        rom_name.as_deref(),
        b2s_name.as_deref(),
        table_name.as_deref(),
        folder_name,
    )
}

/// Open the `.vpx` once and pull both the romname (from the embedded
/// VBS `cGameName`) and the `TableInfo.TableName`. Cheap to bundle —
/// we'd otherwise re-open the OLE compound file twice.
pub(crate) fn read_vpx_meta(vpx_path: &Path) -> (Option<String>, Option<String>) {
    let Ok(mut vpx) = vpin::vpx::open(vpx_path) else {
        return (None, None);
    };
    let rom_name = vpx
        .read_gamedata()
        .ok()
        .and_then(|g| extract_cgamename(&g.code.string));
    let table_name = vpx
        .read_tableinfo()
        .ok()
        .and_then(|info| info.table_name)
        .filter(|s| !s.trim().is_empty());
    (rom_name, table_name)
}

/// Pull the first uncommented `Const cGameName = "..."` from a VBS
/// blob. Mirrors the heuristic used by [`crate::vbs_patches`] but
/// scoped to one regex.
pub(crate) fn extract_cgamename(vbs: &str) -> Option<String> {
    for line in vbs.lines() {
        let s = line.trim_start();
        if s.starts_with('\'') || s.to_ascii_lowercase().starts_with("rem ") {
            continue;
        }
        // Tolerate `Const cGameName="..."`, `cGameName = "..."`, etc.
        let lower = s.to_ascii_lowercase();
        if let Some(idx) = lower.find("cgamename") {
            // After cGameName, find the next '=' then '"…"'.
            let after = &s[idx + "cgamename".len()..];
            let eq = after.find('=')?;
            let q = after[eq + 1..].find('"')?;
            let start = eq + 1 + q + 1;
            let rest = &after[start..];
            let end = rest.find('"')?;
            let name = &rest[..end];
            // Sanity: only printable rom-name chars
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Read `<GameName>` from a `.directb2s` via the `directb2s` crate.
fn read_b2s_game_name(b2s_path: &Path) -> Option<String> {
    let f = std::fs::File::open(b2s_path).ok()?;
    let r = std::io::BufReader::new(f);
    let data = directb2s::read(r).ok()?;
    let g = data.game_name.value;
    if g.trim().is_empty() {
        None
    } else {
        Some(g)
    }
}

/// Scan every `Game.romFiles[*].version` for an exact match.
fn find_by_rom_exact<'a>(games: &'a [Game], rom: &str) -> Option<&'a Game> {
    let needle = rom.to_ascii_lowercase();
    games.iter().find(|g| {
        g.rom_files.iter().any(|rf| {
            rf.version
                .as_deref()
                .map(|v| v.to_ascii_lowercase() == needle)
                .unwrap_or(false)
        })
    })
}

/// Strip common revision suffixes and retry exact match. Catches:
/// `mm_109c` → `mm_l5`, `tz_94ch` → `tz_94h`, `im_185ve` → `im_183ve`,
/// etc. — not bullet-proof, but better than nothing.
fn find_by_rom_fuzzy<'a>(games: &'a [Game], rom: &str) -> Option<&'a Game> {
    // Split at first underscore: "mm_109c" → "mm".
    let prefix = rom.split_once('_').map(|(p, _)| p).unwrap_or(rom);
    if prefix.len() < 2 {
        return None;
    }
    let p_lower = prefix.to_ascii_lowercase();
    games.iter().find(|g| {
        g.rom_files.iter().any(|rf| {
            rf.version
                .as_deref()
                .map(|v| {
                    let v_lower = v.to_ascii_lowercase();
                    v_lower.starts_with(&p_lower)
                        && v_lower
                            .as_bytes()
                            .get(p_lower.len())
                            .map(|&b| b == b'_')
                            .unwrap_or(false)
                })
                .unwrap_or(false)
        })
    })
}

/// Exact name match after normalisation (alnum-lowercase). Used with
/// the embedded `TableInfo.TableName` to skip both fuzzy strategies
/// when the author's title and the catalog title differ only in
/// punctuation/casing (e.g. `"2001: A Space Odyssey"` vs
/// `"2001 - A Space Odyssey"`).
fn find_by_name_exact<'a>(games: &'a [Game], name: &str) -> Option<&'a Game> {
    let needle = normalize(name);
    if needle.len() < 3 {
        return None;
    }
    games.iter().find(|g| normalize(&g.name) == needle)
}

/// Folder-name fuzzy: lowercase, strip non-alphanumeric, substring
/// match in either direction. Only fires when nothing else worked, so
/// false positives just downgrade confidence to `Low`.
fn find_by_name_fuzzy<'a>(games: &'a [Game], folder: &str) -> Option<&'a Game> {
    let needle = normalize(folder);
    if needle.len() < 3 {
        return None;
    }
    games.iter().find(|g| {
        let n = normalize(&g.name);
        n.len() >= 3 && (n.contains(&needle) || needle.contains(&n))
    })
}

fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vpsdb::models::{Game, RomFile};

    fn make_game(id: &str, name: &str, rom_versions: &[&str]) -> Game {
        Game {
            id: id.into(),
            name: name.into(),
            manufacturer: None,
            year: None,
            game_type: None,
            theme: vec![],
            updated_at: None,
            table_files: vec![],
            rom_files: rom_versions
                .iter()
                .map(|v| RomFile {
                    id: format!("rid-{v}"),
                    version: Some((*v).to_string()),
                    authors: vec![],
                    urls: vec![],
                    features: vec![],
                    updated_at: None,
                    comment: None,
                })
                .collect(),
            b2s_files: vec![],
            pup_pack_files: vec![],
            alt_sound_files: vec![],
            alt_color_files: vec![],
            wheel_art_files: vec![],
            topper_files: vec![],
            pov_files: vec![],
            media_pack_files: vec![],
            rule_files: vec![],
            sound_files: vec![],
            tutorial_files: vec![],
        }
    }

    #[test]
    fn exact_rom_match_wins_first() {
        let games = vec![
            make_game("g1", "Apollo 13", &["apollo13"]),
            make_game("g2", "Attack from Mars", &["afm_113b", "afm_10"]),
        ];
        let m = match_table(&games, Some("apollo13"), None, None, "anything").unwrap();
        assert_eq!(m.game.id, "g1");
        assert_eq!(m.confidence, MatchConfidence::High);
    }

    #[test]
    fn fuzzy_falls_back_to_prefix_underscore() {
        let games = vec![make_game("g1", "Medieval Madness", &["mm_l5", "mm_10"])];
        // mm_109c is a hacked variant; mm_l5 is canonical
        let m = match_table(&games, Some("mm_109c"), None, None, "anything").unwrap();
        assert_eq!(m.game.id, "g1");
        assert_eq!(m.confidence, MatchConfidence::Medium);
    }

    #[test]
    fn b2s_match_is_medium() {
        let games = vec![make_game("g1", "Iron Man", &["im_183ve"])];
        let m = match_table(&games, None, Some("im_183ve"), None, "Iron Man").unwrap();
        assert_eq!(m.confidence, MatchConfidence::Medium);
    }

    #[test]
    fn folder_fuzzy_is_low_confidence() {
        let games = vec![make_game("g1", "Tron Legacy LE", &[])];
        let m = match_table(&games, None, None, None, "Tron Legacy").unwrap();
        assert_eq!(m.confidence, MatchConfidence::Low);
    }

    #[test]
    fn no_match_when_nothing_lines_up() {
        let games = vec![make_game("g1", "Apollo 13", &["apollo13"])];
        assert!(match_table(&games, Some("zzz"), None, None, "Mystery").is_none());
    }

    #[test]
    fn extract_cgamename_strips_comments() {
        let vbs = r#"' Const cGameName = "ss_01" 'commented out
Const cGameName = "ss_15"
"#;
        assert_eq!(extract_cgamename(vbs).as_deref(), Some("ss_15"));
    }

    #[test]
    fn extract_cgamename_handles_no_spaces() {
        let vbs = r#"Const cGameName="apollo13",UseSolenoids=2"#;
        assert_eq!(extract_cgamename(vbs).as_deref(), Some("apollo13"));
    }
}
