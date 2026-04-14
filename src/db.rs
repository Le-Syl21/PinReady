use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// Default database location
pub fn default_db_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share/pinready/pinready.db")
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
        db.init_schema()?;
        Ok(db)
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
            );",
            )
            .context("Failed to initialize database schema")?;
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
