//! SQLite persistence. A game is stored as its settings plus an append-only
//! log of the events that moved it forward; seats and scoring marks are
//! small mutable state saved alongside.

use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use igo_core::{Color, Move, Vertex};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::protocol::Settings;

/// Something that changed the game itself, replayed in order on load.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Move {
        color: Color,
        #[serde(rename = "move")]
        mv: Move,
    },
    Resume,
    Finish {
        dead: Vec<Vertex>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeatRecord {
    pub token: String,
    pub name: String,
}

/// Seats and scoring marks: everything about a room that is not in the log.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Table {
    pub seats: [Option<SeatRecord>; 2],
    pub dead: Vec<Vertex>,
    pub accepted: Vec<Color>,
}

pub struct Saved {
    pub settings: Settings,
    pub table: Table,
    pub events: Vec<Event>,
}

pub struct Store {
    conn: Mutex<Connection>,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening database at {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS rooms (
                 id         TEXT PRIMARY KEY,
                 settings   TEXT NOT NULL,
                 tbl        TEXT NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS events (
                 room  TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
                 seq   INTEGER NOT NULL,
                 event TEXT NOT NULL,
                 PRIMARY KEY (room, seq)
             );",
        )?;
        Ok(Store {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn create(&self, id: &str, settings: &Settings) -> Result<()> {
        self.conn().execute(
            "INSERT INTO rooms (id, settings, tbl, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                id,
                serde_json::to_string(settings)?,
                serde_json::to_string(&Table::default())?,
                now()
            ],
        )?;
        Ok(())
    }

    pub fn exists(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn()
            .query_row("SELECT 1 FROM rooms WHERE id = ?1", [id], |_| Ok(()))
            .optional()?
            .is_some())
    }

    pub fn load(&self, id: &str) -> Result<Option<Saved>> {
        let conn = self.conn();
        let Some((settings, table)) = conn
            .query_row(
                "SELECT settings, tbl FROM rooms WHERE id = ?1",
                [id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        else {
            return Ok(None);
        };

        let mut stmt = conn.prepare("SELECT event FROM events WHERE room = ?1 ORDER BY seq")?;
        let events = stmt
            .query_map([id], |row| row.get::<_, String>(0))?
            .map(|text| Ok(serde_json::from_str(&text?)?))
            .collect::<Result<Vec<Event>>>()?;

        Ok(Some(Saved {
            settings: serde_json::from_str(&settings)?,
            table: serde_json::from_str(&table)?,
            events,
        }))
    }

    pub fn append(&self, id: &str, seq: usize, event: &Event) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO events (room, seq, event) VALUES (?1, ?2, ?3)",
            params![id, seq as i64, serde_json::to_string(event)?],
        )?;
        conn.execute(
            "UPDATE rooms SET updated_at = ?2 WHERE id = ?1",
            params![id, now()],
        )?;
        Ok(())
    }

    pub fn save_table(&self, id: &str, table: &Table) -> Result<()> {
        self.conn().execute(
            "UPDATE rooms SET tbl = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, serde_json::to_string(table)?, now()],
        )?;
        Ok(())
    }

    pub fn touch(&self, id: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE rooms SET updated_at = ?2 WHERE id = ?1",
            params![id, now()],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM rooms WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Rooms not written to for at least `max_idle_secs`.
    pub fn stale(&self, max_idle_secs: u64) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id FROM rooms WHERE updated_at < ?1")?;
        let ids = stmt
            .query_map([now() - max_idle_secs as i64], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }
}
