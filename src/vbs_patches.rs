//! VBS patching pipeline for VPX Standalone compatibility.
//!
//! Some VPX tables use VBScript idioms that work under Windows's native
//! vbscript.dll but break on Wine's reimplementation used by VPX
//! Standalone (Linux / macOS). The community project
//! [jsm174/vpx-standalone-scripts](https://github.com/jsm174/vpx-standalone-scripts)
//! curates patched `.vbs` files for affected tables. At scan time we
//! fingerprint each table's embedded VBS, consult the catalog, and —
//! when a patched version exists — install it as a sidecar `.vbs`
//! alongside the `.vpx`. VPX auto-loads sidecar scripts in preference
//! to the embedded one.
//!
//! See also: src/db.rs (`vbs_catalog`, `vbs_patches` tables).
//!
//! This module only concerns itself with the catalog fetcher. The
//! extraction + classification logic lives in a later commit.

use anyhow::{Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// GitHub repo coordinates. Kept as constants so tests can substitute
/// fixtures if we ever add integration tests against a mock server.
const REPO: &str = "jsm174/vpx-standalone-scripts";
const BRANCH: &str = "master";

/// A single entry in jsm174's `hashes.json`. We only decode the fields
/// we actually use — `serde(default)` keeps extra fields from breaking
/// the deserializer if upstream adds new ones.
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogEntry {
    /// The ORIGINAL (unpatched) script's filename. Not strictly needed
    /// for matching (we key on sha256), but kept so log output can
    /// reference the human-readable name when we classify a table.
    #[allow(dead_code)]
    pub file: String,
    /// SHA256 of the original script — matched against embedded VBS.
    pub sha256: String,
    /// The patched script we'd install as a sidecar.
    pub patched: CatalogPatchedInfo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogPatchedInfo {
    /// Filename on jsm174's repo. Informational — we don't use it for
    /// lookup. Kept for parity with the upstream JSON shape.
    #[allow(dead_code)]
    pub file: String,
    /// SHA256 of the patched file — verified post-download.
    pub sha256: String,
    /// Raw URL on github.com. Downloaded as-is and written to the
    /// table's sidecar path after SHA verification.
    pub url: String,
}

/// Fetch the SHA of master's HEAD commit. Cheap call (~200 bytes of
/// JSON) so we can gate `hashes.json` re-download on it.
pub fn fetch_latest_commit_sha() -> Result<String> {
    let url = format!("https://api.github.com/repos/{REPO}/commits/{BRANCH}");
    let response = ureq::get(&url)
        .header("User-Agent", "PinReady")
        .header("Accept", "application/vnd.github.v3+json")
        .call()
        .context("Failed to query jsm174 latest commit")?;
    let body = response.into_body().read_to_string()?;
    let json: serde_json::Value =
        serde_json::from_str(&body).context("Failed to parse commit JSON")?;
    json["sha"]
        .as_str()
        .map(|s| s.to_string())
        .context("Missing 'sha' in commit response")
}

/// Fetch `hashes.json` from master. Returns the raw JSON string; the
/// caller stores it verbatim in `vbs_catalog.hashes_json` so future
/// parse tweaks don't require a re-fetch.
pub fn fetch_hashes_json() -> Result<String> {
    let url = format!("https://raw.githubusercontent.com/{REPO}/refs/heads/{BRANCH}/hashes.json");
    let response = ureq::get(&url)
        .header("User-Agent", "PinReady")
        .call()
        .context("Failed to fetch jsm174 hashes.json")?;
    let body = response
        .into_body()
        .read_to_string()
        .context("Failed to read hashes.json body")?;
    // Validate it parses as JSON before caching — avoids storing a
    // corrupt/redirected response.
    let _: serde_json::Value =
        serde_json::from_str(&body).context("hashes.json is not valid JSON")?;
    Ok(body)
}

/// Parse a cached `hashes_json` blob into the typed catalog list.
/// Kept separate from the fetch so offline-booted PinReady can still
/// drive classification off the cached payload.
pub fn parse_catalog(hashes_json: &str) -> Result<Vec<CatalogEntry>> {
    serde_json::from_str(hashes_json).context("Failed to parse cached hashes.json")
}

/// Lowercase hex SHA256 of arbitrary bytes. jsm174's `hashes.json`
/// uses 64-char lowercase hex, so we match that convention directly.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Read the VBScript embedded inside a `.vpx` file. Uses the `vpin`
/// crate (same one we use for backglass image extraction) — no
/// external tool required. Returns the script as a UTF-8 `String`;
/// `vpin` stores it that way internally, and jsm174's hashes are
/// computed over the same UTF-8 byte stream that `-extractvbs` writes
/// to disk, so our hash can be compared directly without any
/// normalization step.
pub fn extract_embedded_vbs(vpx_path: &Path) -> Result<String> {
    let mut vpx = vpin::vpx::open(vpx_path)
        .with_context(|| format!("Failed to open .vpx: {}", vpx_path.display()))?;
    let gamedata = vpx
        .read_gamedata()
        .with_context(|| format!("Failed to read GameData from {}", vpx_path.display()))?;
    Ok(gamedata.code.string)
}

/// The scanner's verdict for a single table. Bound tightly to the
/// action the patch-applier will take — we classify once, act once,
/// record once in `vbs_patches`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchDecision {
    /// Embedded VBS hash is unknown to jsm174 — either the table is
    /// fine as-is, or it has a standalone incompat that hasn't been
    /// catalogued. Either way, we do nothing. DB status: `NotInCatalog`.
    NotInCatalog,

    /// A sidecar `.vbs` already exists and matches the catalog's
    /// `patched.sha256`. Nothing to do. DB status: `AlreadyPatched`.
    AlreadyPatched,

    /// No sidecar yet — download and install the patched version.
    /// DB status (on success): `Applied`.
    NoSidecar {
        patched_url: String,
        patched_sha: String,
    },

    /// Sidecar exists but matches the embedded (= catalog's original)
    /// hash — redundant copy, user did `-extractvbs` but never patched.
    /// Safe to delete before installing the patched version, since the
    /// bytes live inside the `.vpx` and can be re-extracted any time.
    /// DB status (on success): `Applied`.
    SidecarIsRedundant {
        patched_url: String,
        patched_sha: String,
    },

    /// Sidecar exists with content that matches neither the original
    /// nor the patched version — the user has customized it. We rename
    /// it to `<table>.pre_standalone.vbs` (VPX ignores that name) and
    /// install the patched version, so the user gets a working table
    /// and keeps their backup. DB status (on success): `CustomPreserved`.
    SidecarIsCustom {
        patched_url: String,
        patched_sha: String,
    },
}

/// Classification output passed through the scan pipeline. Holds the
/// raw SHAs so the caller can persist them in `vbs_patches` regardless
/// of whether any file operation happens.
#[derive(Debug, Clone)]
pub struct Classification {
    pub embedded_sha: String,
    pub sidecar_sha: Option<String>,
    pub decision: PatchDecision,
}

/// Classify a single table's VBS state against the catalog. Pure
/// function over inputs (reads the `.vpx` + optional sidecar from disk,
/// but makes no network calls and does not mutate anything). Split
/// from the apply step so unit tests can drive the decision matrix
/// with synthetic hashes.
pub fn classify(vpx_path: &Path, catalog: &[CatalogEntry]) -> Result<Classification> {
    let embedded = extract_embedded_vbs(vpx_path)?;
    let embedded_sha = sha256_hex(embedded.as_bytes());
    let sidecar_path = vpx_path.with_extension("vbs");
    let sidecar_sha = if sidecar_path.is_file() {
        let bytes = std::fs::read(&sidecar_path)
            .with_context(|| format!("Failed to read sidecar {}", sidecar_path.display()))?;
        Some(sha256_hex(&bytes))
    } else {
        None
    };
    let decision = decide(&embedded_sha, sidecar_sha.as_deref(), catalog);
    Ok(Classification {
        embedded_sha,
        sidecar_sha,
        decision,
    })
}

/// Pure decision function — no I/O. Extracted so we can unit-test the
/// decision matrix exhaustively without faking `.vpx` files on disk.
pub fn decide(
    embedded_sha: &str,
    sidecar_sha: Option<&str>,
    catalog: &[CatalogEntry],
) -> PatchDecision {
    let Some(entry) = catalog.iter().find(|e| e.sha256 == embedded_sha) else {
        return PatchDecision::NotInCatalog;
    };
    let patched_url = entry.patched.url.clone();
    let patched_sha = entry.patched.sha256.clone();
    match sidecar_sha {
        None => PatchDecision::NoSidecar {
            patched_url,
            patched_sha,
        },
        Some(sha) if sha == entry.patched.sha256 => PatchDecision::AlreadyPatched,
        Some(sha) if sha == entry.sha256 => PatchDecision::SidecarIsRedundant {
            patched_url,
            patched_sha,
        },
        Some(_) => PatchDecision::SidecarIsCustom {
            patched_url,
            patched_sha,
        },
    }
}

/// DB `status` strings — kept in sync with the `PatchDecision` variants
/// and documented in the `vbs_patches` schema comment. Use these
/// constants rather than stringifying enum names so we have a single
/// source of truth if we rename anything.
pub mod status {
    pub const NOT_IN_CATALOG: &str = "NotInCatalog";
    pub const ALREADY_PATCHED: &str = "AlreadyPatched";
    pub const APPLIED: &str = "Applied";
    pub const CUSTOM_PRESERVED: &str = "CustomPreserved";
    pub const FAILED: &str = "Failed";
}

/// DB status string for a given decision — before considering apply
/// outcome. Use `status::FAILED` explicitly when `apply_patch` errors.
pub fn decision_status(decision: &PatchDecision) -> &'static str {
    match decision {
        PatchDecision::NotInCatalog => status::NOT_IN_CATALOG,
        PatchDecision::AlreadyPatched => status::ALREADY_PATCHED,
        PatchDecision::NoSidecar { .. } | PatchDecision::SidecarIsRedundant { .. } => {
            status::APPLIED
        }
        PatchDecision::SidecarIsCustom { .. } => status::CUSTOM_PRESERVED,
    }
}

/// Download the patched `.vbs` from jsm174 + verify its SHA256 against
/// `expected_sha`. Fails loudly (returns `Err`) on network failure or
/// hash mismatch — callers must not write anything to disk in either
/// case. Files on jsm174 are small (a few KB each), so we buffer the
/// whole body in memory before hashing.
pub fn download_and_verify(url: &str, expected_sha: &str) -> Result<Vec<u8>> {
    let response = ureq::get(url)
        .header("User-Agent", "PinReady")
        .call()
        .with_context(|| format!("Failed to GET {url}"))?;
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .read_to_end(&mut bytes)
        .context("Failed to read patched .vbs body")?;
    let got = sha256_hex(&bytes);
    if got != expected_sha {
        anyhow::bail!("Patched VBS SHA mismatch: expected {expected_sha}, got {got} (url: {url})");
    }
    Ok(bytes)
}

/// Apply the classifier's verdict to disk. Atomic for the file that
/// actually matters (the sidecar): we download + verify in memory
/// first, so disk state is never half-written. The `SidecarIsCustom`
/// branch preserves the user's original as `<table>.pre_standalone.vbs`
/// (a filename VPX does NOT auto-load) before installing the patch —
/// the user keeps their backup and gets a working table.
///
/// Network + hash verification happen before any mutation, so a failed
/// download leaves the table's existing state untouched.
pub fn apply_patch(vpx_path: &Path, decision: &PatchDecision) -> Result<()> {
    // Fast path: nothing to do.
    let (url, expected_sha) = match decision {
        PatchDecision::NotInCatalog | PatchDecision::AlreadyPatched => return Ok(()),
        PatchDecision::NoSidecar {
            patched_url,
            patched_sha,
        }
        | PatchDecision::SidecarIsRedundant {
            patched_url,
            patched_sha,
        }
        | PatchDecision::SidecarIsCustom {
            patched_url,
            patched_sha,
        } => (patched_url.as_str(), patched_sha.as_str()),
    };

    // Step 1: fetch + verify. No disk ops until this succeeds.
    let bytes = download_and_verify(url, expected_sha)?;

    let sidecar = vpx_path.with_extension("vbs");

    // Step 2: if the user has a CUSTOM sidecar, preserve it. Redundant
    // sidecars (= embedded) don't need preservation — the bytes live
    // inside the .vpx and can be recovered via `-extractvbs`.
    if matches!(decision, PatchDecision::SidecarIsCustom { .. }) && sidecar.is_file() {
        let backup = vpx_path.with_extension("pre_standalone.vbs");
        std::fs::rename(&sidecar, &backup).with_context(|| {
            format!(
                "Failed to preserve existing sidecar as {}",
                backup.display()
            )
        })?;
        log::info!(
            "vbs_patches: preserved custom sidecar as {}",
            backup.display()
        );
    }

    // Step 3: write patched bytes. For SidecarIsRedundant, this
    // overwrites the redundant copy in a single syscall — no explicit
    // delete needed.
    std::fs::write(&sidecar, &bytes)
        .with_context(|| format!("Failed to write patched sidecar {}", sidecar.display()))?;
    log::info!(
        "vbs_patches: installed patched sidecar {}",
        sidecar.display()
    );
    Ok(())
}

/// Refresh the cached catalog if the upstream commit has moved. Strategy:
///
/// 1. Pull current master HEAD SHA (~200 bytes).
/// 2. Compare with the cached `last_commit_sha`.
/// 3. If they differ (or no cache exists), fetch + persist `hashes.json`.
/// 4. Otherwise, leave the cache intact.
///
/// Network failures during step 1 or 3 are logged but non-fatal — the
/// caller keeps using the last-known catalog, which is the right
/// behavior for an offline pincab.
///
/// Returns `Ok(())` on any outcome where the cache is usable (either
/// fresh or previously-populated). Returns `Err` only on DB errors.
pub fn refresh_catalog_if_stale(db: &crate::db::Database) -> Result<()> {
    let cached_sha = db.get_vbs_catalog().map(|(sha, _)| sha);
    let remote_sha = match fetch_latest_commit_sha() {
        Ok(sha) => sha,
        Err(e) => {
            log::warn!("vbs_patches: offline or GitHub unreachable ({e}); using cached catalog");
            return Ok(());
        }
    };
    if cached_sha.as_deref() == Some(remote_sha.as_str()) {
        log::debug!("vbs_patches: catalog up-to-date (commit {remote_sha})");
        return Ok(());
    }
    log::info!(
        "vbs_patches: refreshing catalog (cached={:?}, remote={})",
        cached_sha.as_deref().unwrap_or("<none>"),
        &remote_sha[..8.min(remote_sha.len())]
    );
    let json = match fetch_hashes_json() {
        Ok(j) => j,
        Err(e) => {
            log::warn!("vbs_patches: failed to fetch hashes.json ({e}); keeping old cache");
            return Ok(());
        }
    };
    db.set_vbs_catalog(&remote_sha, &json)?;
    log::info!(
        "vbs_patches: catalog refreshed, {} bytes stored",
        json.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_catalog_happy_path() {
        let sample = r#"[
            {
                "file": "Power Play (Bally 1977).vbs.original",
                "sha256": "ef9b7f6c10256d74ce8928c056ff138807a7859c8a27b70e9e2f70f57fb61dfd",
                "url": "https://raw.example/original.vbs",
                "patched": {
                    "file": "Power Play (Bally 1977).vbs",
                    "sha256": "7fb40259669996f5bc14e64306aa9609e669a71bb459dee0a65f8bd8e9dd4d00",
                    "url": "https://raw.example/patched.vbs"
                }
            }
        ]"#;
        let catalog = parse_catalog(sample).unwrap();
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].sha256.len(), 64);
        assert_eq!(catalog[0].patched.sha256.len(), 64);
        assert!(catalog[0].patched.url.starts_with("https://"));
    }

    #[test]
    fn parse_catalog_tolerates_unknown_fields() {
        // jsm174 might add fields later — we must not reject the whole
        // catalog because of one unrecognized key.
        let sample = r#"[
            {
                "file": "Foo.vbs.original",
                "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                "url": "https://x/y",
                "unknown_future_field": 42,
                "patched": {
                    "file": "Foo.vbs",
                    "sha256": "1111111111111111111111111111111111111111111111111111111111111111",
                    "url": "https://x/y"
                }
            }
        ]"#;
        let catalog = parse_catalog(sample).unwrap();
        assert_eq!(catalog.len(), 1);
    }

    #[test]
    fn parse_catalog_rejects_malformed() {
        assert!(parse_catalog("not json").is_err());
        assert!(parse_catalog("{}").is_err());
    }

    #[test]
    fn sha256_matches_known_vector() {
        // "abc" → SHA-256 from the test vector appendix of FIPS 180-4.
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn fake_entry(orig_sha: &str, patched_sha: &str) -> CatalogEntry {
        CatalogEntry {
            file: "Foo.vbs.original".into(),
            sha256: orig_sha.into(),
            patched: CatalogPatchedInfo {
                file: "Foo.vbs".into(),
                sha256: patched_sha.into(),
                url: "https://example.test/Foo.vbs".into(),
            },
        }
    }

    #[test]
    fn decide_not_in_catalog() {
        let catalog = vec![fake_entry("aaaa", "bbbb")];
        assert_eq!(decide("9999", None, &catalog), PatchDecision::NotInCatalog);
        assert_eq!(
            decide("9999", Some("anything"), &catalog),
            PatchDecision::NotInCatalog
        );
    }

    #[test]
    fn decide_no_sidecar_triggers_download() {
        let catalog = vec![fake_entry("aaaa", "bbbb")];
        match decide("aaaa", None, &catalog) {
            PatchDecision::NoSidecar {
                patched_url,
                patched_sha,
            } => {
                assert_eq!(patched_sha, "bbbb");
                assert!(patched_url.ends_with("/Foo.vbs"));
            }
            other => panic!("expected NoSidecar, got {other:?}"),
        }
    }

    #[test]
    fn decide_already_patched_skips() {
        let catalog = vec![fake_entry("aaaa", "bbbb")];
        assert_eq!(
            decide("aaaa", Some("bbbb"), &catalog),
            PatchDecision::AlreadyPatched
        );
    }

    #[test]
    fn decide_sidecar_is_redundant_copy() {
        // Sidecar is a byte-identical copy of the embedded (= original)
        // VBS. Safe to delete because the bytes are preserved in the .vpx.
        let catalog = vec![fake_entry("aaaa", "bbbb")];
        match decide("aaaa", Some("aaaa"), &catalog) {
            PatchDecision::SidecarIsRedundant { patched_sha, .. } => {
                assert_eq!(patched_sha, "bbbb");
            }
            other => panic!("expected SidecarIsRedundant, got {other:?}"),
        }
    }

    #[test]
    fn decide_custom_sidecar_gets_preserved() {
        // Sidecar matches neither original nor patched — user has
        // customized it. We patch and rename their backup so the table
        // works instead of staying broken.
        let catalog = vec![fake_entry("aaaa", "bbbb")];
        match decide("aaaa", Some("custom_sha"), &catalog) {
            PatchDecision::SidecarIsCustom { patched_sha, .. } => {
                assert_eq!(patched_sha, "bbbb");
            }
            other => panic!("expected SidecarIsCustom, got {other:?}"),
        }
    }

    #[test]
    fn decide_scan_finds_first_matching_entry() {
        // Sanity: if two entries share the same `sha256` (shouldn't
        // happen but let's not explode), we pick the first one.
        let catalog = vec![
            fake_entry("dup", "first_patch"),
            fake_entry("dup", "second_patch"),
        ];
        match decide("dup", None, &catalog) {
            PatchDecision::NoSidecar { patched_sha, .. } => {
                assert_eq!(patched_sha, "first_patch");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn decision_status_strings_match_schema() {
        let catalog = vec![fake_entry("a", "b")];
        assert_eq!(
            decision_status(&decide("x", None, &catalog)),
            status::NOT_IN_CATALOG
        );
        assert_eq!(
            decision_status(&decide("a", Some("b"), &catalog)),
            status::ALREADY_PATCHED
        );
        assert_eq!(
            decision_status(&decide("a", None, &catalog)),
            status::APPLIED
        );
        assert_eq!(
            decision_status(&decide("a", Some("a"), &catalog)),
            status::APPLIED
        );
        assert_eq!(
            decision_status(&decide("a", Some("custom"), &catalog)),
            status::CUSTOM_PRESERVED
        );
    }

    #[test]
    fn apply_patch_noop_for_not_in_catalog() {
        // No network, no file I/O — must return Ok immediately.
        let tmp = tempfile::tempdir().unwrap();
        let vpx = tmp.path().join("Foo.vpx");
        // apply_patch only touches `vpx.with_extension("vbs")` — we
        // don't need a real .vpx for this branch.
        apply_patch(&vpx, &PatchDecision::NotInCatalog).unwrap();
        assert!(!vpx.with_extension("vbs").exists());
    }

    #[test]
    fn apply_patch_noop_for_already_patched() {
        let tmp = tempfile::tempdir().unwrap();
        let vpx = tmp.path().join("Foo.vpx");
        let sidecar = vpx.with_extension("vbs");
        std::fs::write(&sidecar, b"already-patched-body").unwrap();
        apply_patch(&vpx, &PatchDecision::AlreadyPatched).unwrap();
        // Untouched.
        let after = std::fs::read(&sidecar).unwrap();
        assert_eq!(after, b"already-patched-body");
    }

    #[test]
    fn apply_patch_preserves_custom_sidecar_on_rename_step() {
        // We can't easily unit-test the full download path without
        // standing up a local HTTP server. What we CAN verify is that
        // the rename step happens BEFORE any write — so if we craft a
        // decision whose download will fail (bogus URL), the sidecar
        // must remain untouched.
        let tmp = tempfile::tempdir().unwrap();
        let vpx = tmp.path().join("Foo.vpx");
        let sidecar = vpx.with_extension("vbs");
        std::fs::write(&sidecar, b"user-custom-content").unwrap();

        let decision = PatchDecision::SidecarIsCustom {
            patched_url: "http://127.0.0.1:1/nope".to_string(),
            patched_sha: "0".repeat(64),
        };
        let err = apply_patch(&vpx, &decision).unwrap_err();
        // Error from the download step.
        assert!(
            format!("{err:#}").contains("Failed to GET") || format!("{err:#}").contains("nope"),
            "unexpected error: {err:#}"
        );
        // CRITICAL: sidecar is untouched because we short-circuit
        // before mutating anything.
        let after = std::fs::read(&sidecar).unwrap();
        assert_eq!(after, b"user-custom-content");
        assert!(!vpx.with_extension("pre_standalone.vbs").exists());
    }
}
