use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// Default database location
fn default_db_path() -> PathBuf {
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
