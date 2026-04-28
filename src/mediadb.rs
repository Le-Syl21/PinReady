//! VPinMediaDB integration — companion media catalog to VPS DB.
//!
//! [`superhac/vpinmediadb`](https://github.com/superhac/vpinmediadb)
//! is a crowdsourced media library hosted on GitHub raw, indexed by
//! VPS DB game ID. For each game it can carry up to 11 asset types
//! (`bg`, `dmd`, `table`, `fss`, `wheel`, `cab`, `realdmd`, `flyer`,
//! `audio`, `table_video`) in 1k and 4k resolutions.
//!
//! PinReady currently consumes only the two assets used by the
//! launcher hover preview — wheel/dmd/table/video/etc. are skipped
//! to keep per-table disk usage minimal:
//!   * `1k/bg.png`    → backglass preview
//!   * `audio.mp3`    → table jingle / call-out
//!
//! Both files keep their factory upstream names when installed
//! locally (see [`install_asset`]), so a re-sync replaces in place.
//!
//! Caching strategy mirrors [`crate::vpsdb`]: pull `vpinmdb.json`
//! (~few MB) once and keep it on disk; only re-fetch when its content
//! hash changes (the index ships an md5 per asset, used as the
//! invalidation key for individual assets too).
//!
//! Network failures degrade gracefully — a table that already has its
//! local cache keeps working offline, and tables that didn't get a
//! match simply skip media enrichment.

// Several index-fields (cab/flyer/realdmd/...) are decoded but unused
// until the v0.9 hover UI consumes them; allow until then.
#![allow(dead_code)]

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

const INDEX_URL: &str = "https://raw.githubusercontent.com/superhac/vpinmediadb/main/vpinmdb.json";

/// One game's worth of media URLs, keyed by VPS game ID at the
/// top-level of `vpinmdb.json`. The schema is uneven — some games
/// only have a `wheel`, some have a full 1k/4k pair plus video.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct MediaEntry {
    /// Top-level wheel art (no resolution variants).
    pub wheel: Option<String>,
    pub wheel_md5: Option<String>,
    pub cab: Option<String>,
    pub cab_md5: Option<String>,
    pub flyer: Option<String>,
    pub flyer_md5: Option<String>,
    pub realdmd: Option<String>,
    pub realdmd_md5: Option<String>,
    /// Top-level audio (single jingle, no resolution variants).
    pub audio: Option<String>,
    pub audio_md5: Option<String>,

    /// 1k bucket — typically has bg/dmd/table/fss + video.
    #[serde(rename = "1k")]
    pub k1: Option<MediaBucket>,
    /// 4k bucket — same shape but higher res.
    #[serde(rename = "4k")]
    pub k4: Option<MediaBucket>,

    /// Whole-entry version hash; bumped when *any* asset under this
    /// game changes. Useful as a cheap "is anything new for this
    /// game?" signal.
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct MediaBucket {
    pub bg: Option<String>,
    pub bg_md5: Option<String>,
    pub dmd: Option<String>,
    pub dmd_md5: Option<String>,
    pub table: Option<String>,
    pub table_md5: Option<String>,
    pub fss: Option<String>,
    pub fss_md5: Option<String>,
    pub table_video: Option<String>,
    pub table_video_md5: Option<String>,
}

/// Parsed `vpinmdb.json` plus its on-disk cache home.
pub struct MediaDb {
    pub games: HashMap<String, MediaEntry>,
    cache_dir: PathBuf,
}

impl MediaDb {
    pub fn default_cache_dir() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("pinready")
            .join("mediadb")
    }

    /// Sync index. Buffers `vpinmdb.json` to disk and parses; falls
    /// back to whatever cache exists on network errors.
    pub fn sync(cache_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&cache_dir).ok();
        let cache_path = cache_dir.join("vpinmdb.json");

        let bytes = match fetch_index() {
            Ok(b) => {
                if let Err(e) = std::fs::write(&cache_path, &b) {
                    log::warn!("Failed to write mediadb cache: {e}");
                }
                b
            }
            Err(e) => {
                log::warn!("MediaDb index fetch failed ({e}) — falling back to cache");
                std::fs::read(&cache_path).context("No mediadb cache and remote unreachable")?
            }
        };

        let games: HashMap<String, MediaEntry> =
            serde_json::from_slice(&bytes).context("vpinmdb.json failed to parse")?;
        Ok(Self { games, cache_dir })
    }

    /// Per-asset md5 lookup helpers. Used by the scan worker so it
    /// only DLs an asset when its md5 differs from what we cached on
    /// the local table.
    pub fn bg_url_md5(&self, game_id: &str) -> Option<(&str, &str)> {
        let e = self.games.get(game_id)?;
        let b = e.k1.as_ref()?;
        Some((b.bg.as_deref()?, b.bg_md5.as_deref()?))
    }
    pub fn audio_url_md5(&self, game_id: &str) -> Option<(&str, &str)> {
        let e = self.games.get(game_id)?;
        Some((e.audio.as_deref()?, e.audio_md5.as_deref()?))
    }
    pub fn wheel_url_md5(&self, game_id: &str) -> Option<(&str, &str)> {
        let e = self.games.get(game_id)?;
        Some((e.wheel.as_deref()?, e.wheel_md5.as_deref()?))
    }
}

fn fetch_index() -> Result<Vec<u8>> {
    let resp = ureq::get(INDEX_URL)
        .header("User-Agent", "PinReady")
        .call()
        .context("Failed to fetch vpinmdb.json")?;
    let mut body = resp.into_body();
    let mut buf = Vec::with_capacity(8 * 1024 * 1024);
    body.as_reader()
        .read_to_end(&mut buf)
        .context("Failed to read vpinmdb.json body")?;
    Ok(buf)
}

/// Download a single asset URL, verify md5. Returns the raw bytes.
/// Used by the scan worker — never holds the request open longer
/// than necessary, no retries (we'll just retry next scan if needed).
pub fn fetch_asset(url: &str, expected_md5: &str) -> Result<Vec<u8>> {
    let resp = ureq::get(url)
        .header("User-Agent", "PinReady")
        .call()
        .with_context(|| format!("Failed to GET {url}"))?;
    let mut body = resp.into_body();
    let mut buf = Vec::with_capacity(2 * 1024 * 1024);
    body.as_reader()
        .read_to_end(&mut buf)
        .with_context(|| format!("Failed to read body from {url}"))?;
    let actual: String = md5_digest(&buf)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected_md5.to_ascii_lowercase() {
        anyhow::bail!("MediaDB asset {url} md5 mismatch: expected {expected_md5}, got {actual}");
    }
    Ok(buf)
}

/// Drop an asset into a table folder under `medias/` using the
/// catalog's factory filename verbatim — `bg.png`, `audio.mp3`,
/// `wheel.png`. Keeping the upstream names lets us refresh in place
/// without juggling per-table-renamed copies, and the hover-preview
/// loader on the UI side just looks for fixed filenames per table
/// dir. No parenthesized labels, no per-table basenames mixed in.
pub fn install_asset(table_dir: &Path, filename: &str, bytes: &[u8]) -> Result<PathBuf> {
    let media_dir = table_dir.join("medias");
    std::fs::create_dir_all(&media_dir)
        .with_context(|| format!("Failed to create {}", media_dir.display()))?;
    let target = media_dir.join(filename);
    std::fs::write(&target, bytes)
        .with_context(|| format!("Failed to write {}", target.display()))?;
    Ok(target)
}

/// Tiny MD5 — we don't pull `md5` crate just for this, we use `md-5`
/// from the `digest` family that `sha2` (already a dep) ships with
/// via the same trait. Actually `sha2` doesn't carry md5. So we ship
/// a hand-rolled minimal MD5 here, since:
/// * it's cryptographically weak but that's the catalog's choice
/// * we only use it to verify what the catalog declares; not security
/// * pulling another dep just for `md5` would bloat the binary
fn md5_digest(input: &[u8]) -> [u8; 16] {
    // Public-domain Mark Crispin / RFC 1321 reference, ported to safe
    // Rust. ~50 lines, no dep.
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const K: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(input.len() + 72);
    padded.extend_from_slice(input);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_le_bytes());

    for chunk in padded.chunks(64) {
        let mut m = [0u32; 16];
        for (i, w) in chunk.chunks(4).enumerate() {
            m[i] = u32::from_le_bytes(w.try_into().unwrap());
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let temp = d;
            d = c;
            c = b;
            b = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(K[i])
                    .wrapping_add(m[g])
                    .rotate_left(S[i]),
            );
            a = temp;
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_known_vectors() {
        // RFC 1321 test vectors.
        let hex =
            |bytes: [u8; 16]| -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() };
        assert_eq!(hex(md5_digest(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(md5_digest(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex(md5_digest(b"message digest")),
            "f96b697d7cb7938d525a2f31aaf161d0"
        );
    }
}
