//! Serde structs decoded from `vpsdb.json`.
//!
//! VPS DB is verbose — each game can carry 13 different resource lists
//! (tables, backglasses, ROMs, PUP packs, alt-sound, alt-color, wheel
//! art, toppers, POVs, media packs, rules, sounds, tutorials). For our
//! "catalog enrichment" use case we only really need:
//!   * `name`, `manufacturer`, `year`, `id`, `updatedAt` for matching +
//!     update-checking
//!   * `tableFiles` to know which formats exist (filter VPX-only)
//!   * `romFiles` to know the canonical ROM name(s) for the game
//!
//! Everything else is `Option<Vec<…>>` and `serde(default)` so unknown
//! future fields don't break the deserializer.

// Many fields here are deserialized but only consumed by v0.9 hover-UI
// + update-notifier code; allow until we wire those in.
#![allow(dead_code)]

use serde::Deserialize;

/// One game entry in the spreadsheet.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    /// Stable VPS-assigned ID (e.g. `"--xQTsPe33"`). Used as the join
    /// key against VPinMediaDB and as our DB persistence anchor.
    pub id: String,
    /// Human-readable game name.
    pub name: String,
    /// Manufacturer (`"Williams"`, `"Stern"`, `"Original"`, ...).
    #[serde(default)]
    pub manufacturer: Option<String>,
    /// Release year. Optional because some "Original" tables omit it.
    #[serde(default)]
    pub year: Option<u16>,
    /// Game type: `"SS"` (solid-state), `"EM"` (electromechanical),
    /// `"PM"` (pure mechanical), `"DG"` (digital). Mostly informational.
    #[serde(default, rename = "type")]
    pub game_type: Option<String>,
    /// Themes: `["Music", "Movie"]` etc.
    #[serde(default)]
    pub theme: Vec<String>,
    /// VPS unix-millis timestamp; bumps on any catalog edit (new file
    /// added, metadata corrected, etc.). We compare local-vs-remote to
    /// decide whether to refresh.
    #[serde(default)]
    pub updated_at: Option<i64>,

    /// `.vpx`/`.fp`/`.fpt`/etc. table files contributed for this game.
    #[serde(default)]
    pub table_files: Vec<TableFile>,
    /// PinMAME ROM zips known to the catalog.
    #[serde(default)]
    pub rom_files: Vec<RomFile>,
    /// `.directb2s` backglass files.
    #[serde(default, rename = "b2sFiles")]
    pub b2s_files: Vec<ResourceFile>,
    /// PinUP Player video packs.
    #[serde(default, rename = "pupPackFiles")]
    pub pup_pack_files: Vec<ResourceFile>,
    /// Alt-sound (alternative music/SFX packs).
    #[serde(default)]
    pub alt_sound_files: Vec<ResourceFile>,
    /// Alt-color (DMD colorizations: PAL+VNI / cROMc / cRZ).
    #[serde(default)]
    pub alt_color_files: Vec<ResourceFile>,
    /// Wheel art (used by frontends, cabinet wheel/marquee).
    #[serde(default)]
    pub wheel_art_files: Vec<ResourceFile>,
    /// Topper video/LED content.
    #[serde(default)]
    pub topper_files: Vec<ResourceFile>,
    /// POV (camera angle) presets.
    #[serde(default)]
    pub pov_files: Vec<ResourceFile>,
    /// Bundled media packs.
    #[serde(default)]
    pub media_pack_files: Vec<ResourceFile>,
    /// Rule sheets / docs.
    #[serde(default)]
    pub rule_files: Vec<ResourceFile>,
    /// Sound packs.
    #[serde(default)]
    pub sound_files: Vec<ResourceFile>,
    /// Tutorials / how-tos.
    #[serde(default)]
    pub tutorial_files: Vec<ResourceFile>,
}

/// Common shape for most resource categories: id, version, authors,
/// URLs, and timestamps. The catalog adds extra fields per category
/// (`tableFormat`, `version`, etc.) which we capture in specialized
/// types like [`TableFile`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceFile {
    /// VPS-assigned resource ID. Unique across the whole DB.
    pub id: String,
    /// Free-form version string ("1.0.2", "v1.1", etc.).
    #[serde(default)]
    pub version: Option<String>,
    /// Contributor handles.
    #[serde(default)]
    pub authors: Vec<String>,
    /// Download URLs (may include vpuniverse.com, vpforums.org, mega
    /// links, GitHub releases, …). The first one is generally the
    /// preferred source.
    #[serde(default)]
    pub urls: Vec<ResourceUrl>,
    /// Free-form feature tags ("HD", "VR", "Custom", …).
    #[serde(default)]
    pub features: Vec<String>,
    /// Last edit timestamp (unix-millis).
    #[serde(default)]
    pub updated_at: Option<i64>,
    /// Free-form comment.
    #[serde(default)]
    pub comment: Option<String>,
}

/// A table-file entry. Adds `tableFormat` ("VPX" / "VP9" / "FP" / ...)
/// over [`ResourceFile`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableFile {
    pub id: String,
    /// `"VPX"`, `"VP9"`, `"FP"`, `"FX"`, `"FX3"`, ...
    pub table_format: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub urls: Vec<ResourceUrl>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub updated_at: Option<i64>,
    #[serde(default)]
    pub comment: Option<String>,
}

/// A ROM file entry. The `version` field traditionally encodes the ROM
/// name (e.g. `"sttng_l7"`). VPS sometimes also pins a more canonical
/// name in `id` — we look at both.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RomFile {
    pub id: String,
    /// PinMAME romset name (`"afm_113b"`, `"sttng_l7"`, ...). May also
    /// hold a longer "v1.7 home edition" string for some entries.
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub urls: Vec<ResourceUrl>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub updated_at: Option<i64>,
    #[serde(default)]
    pub comment: Option<String>,
}

/// One download URL (`{ "url": "..." }`); the schema is structured so
/// future fields like `auth_required` or `mirror_priority` can be added
/// without breaking us.
#[derive(Debug, Clone, Deserialize)]
pub struct ResourceUrl {
    pub url: String,
    #[serde(default)]
    pub broken: Option<bool>,
}
