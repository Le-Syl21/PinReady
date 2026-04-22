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
}
