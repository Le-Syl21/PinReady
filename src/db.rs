use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// Default database location, following OS conventions:
/// - Linux:   ~/.local/share/pinready/pinready.db
/// - macOS:   ~/Library/Application Support/pinready/pinready.db
/// - Windows: %APPDATA%\pinready\pinready.db
pub fn default_db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pinready")
        .join("pinready.db")
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: Option<&Path>) -> Result<Self> {
        let path = path.map(PathBuf::from).unwrap_or_else(default_db_path);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create db directory: {}", parent.display()))?;
        }

        let conn = Connection::open(&path)
            .with_context(|| format!("Failed to open database: {}", path.display()))?;

        let db = Self { conn };
        db.migrate()?;
        db.init_schema()?;
        Ok(db)
    }

    /// Apply destructive schema migrations before `init_schema` creates
    /// the current-shape tables. Only the `backglass` cache table has
    /// ever been reshaped; its contents are always regeneratable from
    /// the `.vpx`/`.directb2s` files on disk, so dropping the table on
    /// schema mismatch is safe.
    fn migrate(&self) -> Result<()> {
        // v1 of the backglass table used columns (path, image, source,
        // extracted_at); v2 uses (rel_path, image). Detect the old shape
        // by the presence of a `path` column and drop it so the
        // subsequent CREATE installs the new schema.
        let has_old_backglass: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM pragma_table_info('backglass')
                 WHERE name = 'path' LIMIT 1",
                [],
                |row| row.get::<_, i32>(0),
            )
            .map(|_| true)
            .unwrap_or(false);
        if has_old_backglass {
            log::info!("Dropping v1 backglass cache (schema upgrade to v2)");
            self.conn
                .execute("DROP TABLE backglass", [])
                .context("Failed to drop v1 backglass table")?;
        }

        // v2 → v3: add cached_at_mtime column (Unix seconds of the source
        // file we extracted from). Existing rows get 0 → considered stale
        // on next scan and re-extracted once, from then on mtime-based
        // invalidation kicks in.
        let has_mtime_col: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM pragma_table_info('backglass')
                 WHERE name = 'cached_at_mtime' LIMIT 1",
                [],
                |row| row.get::<_, i32>(0),
            )
            .map(|_| true)
            .unwrap_or(false);
        let has_v2_backglass: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM pragma_table_info('backglass')
                 WHERE name = 'rel_path' LIMIT 1",
                [],
                |row| row.get::<_, i32>(0),
            )
            .map(|_| true)
            .unwrap_or(false);
        if has_v2_backglass && !has_mtime_col {
            log::info!("Adding cached_at_mtime column (schema upgrade to v3)");
            self.conn
                .execute(
                    "ALTER TABLE backglass ADD COLUMN cached_at_mtime INTEGER NOT NULL DEFAULT 0",
                    [],
                )
                .context("Failed to add cached_at_mtime column")?;
        }
        Ok(())
    }

    fn init_schema(&self) -> Result<()> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS config (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS tables (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                path         TEXT NOT NULL UNIQUE,
                name         TEXT NOT NULL,
                manufacturer TEXT,
                year         INTEGER,
                rom_name     TEXT,
                last_scanned TEXT NOT NULL
            );

            -- Per-table backglass cache. Keyed by the .vpx path RELATIVE
            -- to the configured tables directory, so moving the tables
            -- folder to another disk (and updating `tables_dir` in config)
            -- doesn't invalidate the cache. `image` holds JPEG bytes at
            -- quality 85 (~5× smaller than PNG, visually lossless on the
            -- photographic backglass content at 1280×1024); PNG/WebP bytes
            -- stored as-is when the source was a user override at
            -- `<table_dir>/media/launcher.(png|webp|jpg|jpeg)`.
            -- `cached_at_mtime` is the Unix-seconds mtime of the source
            -- file at extraction time; the scanner re-extracts when any
            -- candidate source file on disk is newer.
            CREATE TABLE IF NOT EXISTS backglass (
                rel_path        TEXT    PRIMARY KEY,
                image           BLOB    NOT NULL,
                cached_at_mtime INTEGER NOT NULL DEFAULT 0
            );

            -- jsm174/vpx-standalone-scripts catalog cache. Single-row
            -- table (keyed on last_commit_sha). At startup we hit
            -- api.github.com to read the master branch's latest commit
            -- SHA and compare — if different, we re-fetch hashes.json.
            -- Saves bandwidth (hashes.json is ~150 KB) and lets us work
            -- offline with the last-known catalog.
            CREATE TABLE IF NOT EXISTS vbs_catalog (
                last_commit_sha TEXT    PRIMARY KEY,
                hashes_json     TEXT    NOT NULL,
                fetched_at      INTEGER NOT NULL
            );

            -- Per-table VBS patch state. Keyed on `rel_path` (same
            -- semantics as the backglass cache — the .vpx path relative
            -- to tables_dir). `embedded_sha256` is the SHA256 of the VBS
            -- embedded inside the .vpx; `sidecar_sha256` is the SHA of
            -- the companion .vbs file next to the .vpx, if any.
            -- `status` records what the scanner did: NotInCatalog /
            -- AlreadyPatched / Applied / CustomPreserved (the user's
            -- sidecar was renamed to .pre_standalone.vbs before applying
            -- the patched version from the catalog) / Failed.
            -- `last_checked_mtime` is max(mtime(.vpx), mtime(sidecar))
            -- at classification time — scanner re-evaluates when any
            -- source file is newer than this.
            CREATE TABLE IF NOT EXISTS vbs_patches (
                rel_path           TEXT    PRIMARY KEY,
                embedded_sha256    TEXT    NOT NULL,
                sidecar_sha256     TEXT,
                status             TEXT    NOT NULL,
                last_checked_mtime INTEGER NOT NULL DEFAULT 0
            );",
            )
            .context("Failed to initialize database schema")?;
        Ok(())
    }

    /// Lookup the cached backglass image for a table by its `.vpx` path
    /// relative to the configured `tables_dir`. Returns `(image_bytes,
    /// cached_at_mtime)` so the scanner can compare against source-file
    /// mtimes and re-extract when stale. `None` if no entry exists.
    pub fn get_backglass(&self, rel_path: &str) -> Option<(Vec<u8>, i64)> {
        self.conn
            .query_row(
                "SELECT image, cached_at_mtime FROM backglass WHERE rel_path = ?1",
                [rel_path],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?)),
            )
            .ok()
    }

    /// Upsert an encoded backglass image (JPEG/PNG/WebP depending on
    /// source) for a table's relative path. `source_mtime` is the Unix
    /// seconds mtime of the file we extracted from — stored so the next
    /// scan can detect user-dropped launcher.* overrides without a
    /// manual rescan.
    pub fn set_backglass(&self, rel_path: &str, image: &[u8], source_mtime: i64) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO backglass (rel_path, image, cached_at_mtime)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(rel_path) DO UPDATE
                 SET image = excluded.image,
                     cached_at_mtime = excluded.cached_at_mtime",
                rusqlite::params![rel_path, image, source_mtime],
            )
            .context("Failed to insert backglass row")?;
        Ok(())
    }

    /// Wipe every cached backglass. Called by the long-press rescan so the
    /// next scan re-extracts all images from scratch.
    pub fn clear_backglass(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM backglass", [])
            .context("Failed to clear backglass cache")?;
        Ok(())
    }

    // --- vbs_catalog (jsm174/vpx-standalone-scripts cache) ---

    /// Read the cached jsm174 catalog. Returns `(last_commit_sha,
    /// hashes_json)`. At most one row ever exists — the SHA is the
    /// primary key and we just overwrite on each refresh.
    pub fn get_vbs_catalog(&self) -> Option<(String, String)> {
        self.conn
            .query_row(
                "SELECT last_commit_sha, hashes_json FROM vbs_catalog LIMIT 1",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .ok()
    }

    /// Replace the cached catalog with a fresh (sha, hashes_json). The
    /// previous row is wiped first so only one entry ever lives here.
    pub fn set_vbs_catalog(&self, last_commit_sha: &str, hashes_json: &str) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.conn
            .execute("DELETE FROM vbs_catalog", [])
            .context("Failed to clear vbs_catalog")?;
        self.conn
            .execute(
                "INSERT INTO vbs_catalog (last_commit_sha, hashes_json, fetched_at)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![last_commit_sha, hashes_json, now],
            )
            .context("Failed to insert vbs_catalog row")?;
        Ok(())
    }

    // --- vbs_patches (per-table state) ---

    /// Lookup the recorded VBS-patch state for a table. Returns
    /// `(embedded_sha, sidecar_sha, status, last_checked_mtime)`.
    /// `None` if no entry exists (never classified).
    pub fn get_vbs_patch(&self, rel_path: &str) -> Option<(String, Option<String>, String, i64)> {
        self.conn
            .query_row(
                "SELECT embedded_sha256, sidecar_sha256, status, last_checked_mtime
                 FROM vbs_patches WHERE rel_path = ?1",
                [rel_path],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .ok()
    }

    /// Upsert the VBS-patch state for a table after the scanner has
    /// classified + acted on it.
    pub fn set_vbs_patch(
        &self,
        rel_path: &str,
        embedded_sha: &str,
        sidecar_sha: Option<&str>,
        status: &str,
        last_checked_mtime: i64,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO vbs_patches
                 (rel_path, embedded_sha256, sidecar_sha256, status, last_checked_mtime)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(rel_path) DO UPDATE SET
                   embedded_sha256 = excluded.embedded_sha256,
                   sidecar_sha256  = excluded.sidecar_sha256,
                   status          = excluded.status,
                   last_checked_mtime = excluded.last_checked_mtime",
                rusqlite::params![
                    rel_path,
                    embedded_sha,
                    sidecar_sha,
                    status,
                    last_checked_mtime
                ],
            )
            .context("Failed to upsert vbs_patches row")?;
        Ok(())
    }

    /// Wipe every recorded VBS-patch state. Called by the Rebuild
    /// action so the next scan re-classifies all tables from scratch.
    // Wired in by the Rebuild button (task #9). Drop once consumed.
    #[allow(dead_code)]
    pub fn clear_vbs_patches(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM vbs_patches", [])
            .context("Failed to clear vbs_patches")?;
        Ok(())
    }

    /// Mark the wizard as completed.
    pub fn set_configured(&self) -> Result<()> {
        self.set_config("wizard_completed", "true")
    }

    /// Get a config value by key.
    pub fn get_config(&self, key: &str) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM config WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .ok()
    }

    /// Set a config value.
    pub fn set_config(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO config (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [key, value],
            )
            .context("Failed to set config value")?;
        Ok(())
    }

    /// Get the tables root directory.
    pub fn get_tables_dir(&self) -> Option<String> {
        self.get_config("tables_dir")
    }

    /// Set the tables root directory.
    pub fn set_tables_dir(&self, path: &str) -> Result<()> {
        self.set_config("tables_dir", path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> Database {
        // Use in-memory-like temp file for isolation
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let db = Database::open(Some(&path)).unwrap();
        // Keep dir alive by leaking — tests are short-lived
        std::mem::forget(dir);
        db
    }

    #[test]
    fn open_creates_schema() {
        let db = temp_db();
        // Tables should exist — insert should work
        db.set_config("test_key", "test_value").unwrap();
    }

    #[test]
    fn get_config_missing_returns_none() {
        let db = temp_db();
        assert_eq!(db.get_config("nonexistent"), None);
    }

    #[test]
    fn set_and_get_config() {
        let db = temp_db();
        db.set_config("my_key", "my_value").unwrap();
        assert_eq!(db.get_config("my_key"), Some("my_value".to_string()));
    }

    #[test]
    fn set_config_upserts() {
        let db = temp_db();
        db.set_config("key", "v1").unwrap();
        db.set_config("key", "v2").unwrap();
        assert_eq!(db.get_config("key"), Some("v2".to_string()));
    }

    #[test]
    fn set_configured() {
        let db = temp_db();
        db.set_configured().unwrap();
        assert_eq!(db.get_config("wizard_completed"), Some("true".to_string()));
    }

    #[test]
    fn tables_dir_roundtrip() {
        let db = temp_db();
        assert_eq!(db.get_tables_dir(), None);
        db.set_tables_dir("/home/user/tables").unwrap();
        assert_eq!(db.get_tables_dir(), Some("/home/user/tables".to_string()));
    }

    #[test]
    fn multiple_config_keys_independent() {
        let db = temp_db();
        db.set_config("a", "1").unwrap();
        db.set_config("b", "2").unwrap();
        assert_eq!(db.get_config("a"), Some("1".to_string()));
        assert_eq!(db.get_config("b"), Some("2".to_string()));
    }

    #[test]
    fn vbs_catalog_roundtrip() {
        let db = temp_db();
        assert!(db.get_vbs_catalog().is_none());
        db.set_vbs_catalog("abc123", "{\"files\": []}").unwrap();
        let (sha, json) = db.get_vbs_catalog().unwrap();
        assert_eq!(sha, "abc123");
        assert_eq!(json, "{\"files\": []}");
    }

    #[test]
    fn vbs_catalog_replaces_single_row() {
        let db = temp_db();
        db.set_vbs_catalog("sha1", "{\"v\":1}").unwrap();
        db.set_vbs_catalog("sha2", "{\"v\":2}").unwrap();
        let (sha, json) = db.get_vbs_catalog().unwrap();
        assert_eq!(sha, "sha2");
        assert_eq!(json, "{\"v\":2}");
        // Ensure only one row exists
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM vbs_catalog", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn vbs_patch_roundtrip_with_sidecar() {
        let db = temp_db();
        db.set_vbs_patch(
            "Totan/Totan.vpx",
            "embed_sha_hex",
            Some("sidecar_sha_hex"),
            "Applied",
            1_700_000_000,
        )
        .unwrap();
        let (emb, side, status, mtime) = db.get_vbs_patch("Totan/Totan.vpx").unwrap();
        assert_eq!(emb, "embed_sha_hex");
        assert_eq!(side, Some("sidecar_sha_hex".to_string()));
        assert_eq!(status, "Applied");
        assert_eq!(mtime, 1_700_000_000);
    }

    #[test]
    fn vbs_patch_roundtrip_no_sidecar() {
        let db = temp_db();
        db.set_vbs_patch("Foo/Foo.vpx", "emb", None, "NotInCatalog", 0)
            .unwrap();
        let (_, side, status, _) = db.get_vbs_patch("Foo/Foo.vpx").unwrap();
        assert_eq!(side, None);
        assert_eq!(status, "NotInCatalog");
    }

    #[test]
    fn vbs_patch_upserts_on_rescan() {
        let db = temp_db();
        db.set_vbs_patch("t/t.vpx", "emb1", None, "NotInCatalog", 100)
            .unwrap();
        db.set_vbs_patch("t/t.vpx", "emb2", Some("sc"), "Applied", 200)
            .unwrap();
        let (emb, side, status, mtime) = db.get_vbs_patch("t/t.vpx").unwrap();
        assert_eq!(emb, "emb2");
        assert_eq!(side, Some("sc".to_string()));
        assert_eq!(status, "Applied");
        assert_eq!(mtime, 200);
    }

    #[test]
    fn clear_vbs_patches_wipes_all() {
        let db = temp_db();
        db.set_vbs_patch("a.vpx", "e", None, "s", 1).unwrap();
        db.set_vbs_patch("b.vpx", "e", None, "s", 1).unwrap();
        db.clear_vbs_patches().unwrap();
        assert!(db.get_vbs_patch("a.vpx").is_none());
        assert!(db.get_vbs_patch("b.vpx").is_none());
    }
}
