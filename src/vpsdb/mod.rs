//! Virtual Pinball Spreadsheet (VPS DB) integration.
//!
//! Crowdsourced catalog at <https://virtualpinballspreadsheet.github.io/>
//! that tracks ~2500 games and 13 resource types per game (table files,
//! backglasses, ROMs, PUP packs, alt-sound, alt-color, POVs, etc.).
//!
//! At rescan time PinReady fingerprints each installed table and asks
//! VPS DB "is this table you?" via [`matcher::match_table`]. A
//! successful match anchors the table to a stable VPS game ID, which
//! we then re-use to:
//!   * fetch hover-preview media (backglass image + audio jingle) from
//!     the companion [VPinMediaDB](crate::mediadb)
//!   * surface "update available" badges in the launcher grid
//!     (compare local VPS-recorded `updatedAt` vs the catalog's current
//!     `updatedAt`)
//!
//! This whole pipeline is opt-in (config key `catalog_enrichment_enabled`)
//! because the first sync downloads ~6.7 MB and the mediadb sync downloads
//! a similar JSON plus per-table media files. Users on metered/offline
//! pincabs can keep PinReady purely local.

pub mod fetch;
pub mod matcher;
pub mod models;

pub use matcher::match_table_from_paths;
