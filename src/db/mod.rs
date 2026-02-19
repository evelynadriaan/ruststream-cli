use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, Row, Transaction, params};
use std::path::Path;
use uuid::Uuid;

use crate::models::{ListenEvent, Playlist, PlaylistTrack, Track};

const CURRENT_SCHEMA_VERSION: i64 = 2;

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open database at {}", path.display()))?;

        let db = Self { conn };
        db.init()?;
        Ok(db)
    }

    #[allow(dead_code)]
    pub fn open_in_memory() -> Result<Self> {
        let conn =
            Connection::open_in_memory().with_context(|| "Failed to open in-memory database")?;

        let db = Self { conn };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<()> {
        self.conn
            .execute("PRAGMA foreign_keys = ON", [])
            .with_context(|| "Failed to enable foreign keys")?;

        self.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS schema_migrations (
                    version INTEGER PRIMARY KEY,
                    applied_at TEXT NOT NULL
                )",
                [],
            )
            .with_context(|| "Failed to initialize migration tracking table")?;

        let current_version: Option<i64> =
            self.conn
                .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                    row.get(0)
                })?;

        match current_version {
            None => {
                let tx = self
                    .conn
                    .unchecked_transaction()
                    .with_context(|| "Failed to start schema bootstrap transaction")?;
                apply_migration_1(&tx)?;
                record_migration(&tx, 1)?;
                if CURRENT_SCHEMA_VERSION >= 2 {
                    apply_migration_2(&tx)?;
                    record_migration(&tx, 2)?;
                }
                tx.commit()
                    .with_context(|| "Failed to commit schema bootstrap transaction")?;
            }
            Some(version) if version == CURRENT_SCHEMA_VERSION => {}
            Some(version) if version > CURRENT_SCHEMA_VERSION => {
                anyhow::bail!(
                    "Database schema version {} is newer than supported version {}",
                    version,
                    CURRENT_SCHEMA_VERSION
                );
            }
            Some(1) => {
                let tx = self
                    .conn
                    .unchecked_transaction()
                    .with_context(|| "Failed to start schema migration transaction")?;
                apply_migration_2(&tx)?;
                record_migration(&tx, 2)?;
                tx.commit()
                    .with_context(|| "Failed to commit schema migration transaction")?;
            }
            Some(version) => anyhow::bail!(
                "Unsupported schema version {}. Only {} is currently supported.",
                version,
                CURRENT_SCHEMA_VERSION
            ),
        }

        Ok(())
    }

    fn row_to_track(row: &Row) -> rusqlite::Result<Track> {
        Ok(Track {
            id: row.get::<_, String>(0)?.parse().unwrap_or_default(),
            url: row.get(1)?,
            title: row.get(2)?,
            alias: row.get(3)?,
            duration: row.get::<_, i64>(4)? as u64,
            added_at: row
                .get::<_, String>(5)?
                .parse::<DateTime<Utc>>()
                .unwrap_or_default(),
            file_path: row.get(6)?,
            available: row.get::<_, i64>(7)? != 0,
        })
    }

    fn row_to_playlist(row: &Row) -> rusqlite::Result<Playlist> {
        Ok(Playlist {
            id: row.get::<_, String>(0)?.parse().unwrap_or_default(),
            name: row.get(1)?,
            created_at: row
                .get::<_, String>(2)?
                .parse::<DateTime<Utc>>()
                .unwrap_or_default(),
        })
    }

    // Track operations
    pub fn insert_track(&self, track: &Track) -> Result<()> {
        self.conn.execute(
            "INSERT INTO tracks (id, url, title, alias, duration, added_at, file_path, available)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                track.id.to_string(),
                track.url,
                track.title,
                track.alias,
                track.duration as i64,
                track.added_at.to_rfc3339(),
                track.file_path,
                track.available as i64,
            ],
        ).with_context(|| "Failed to insert track")?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn get_track(&self, id: &Uuid) -> Result<Option<Track>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, url, title, alias, duration, added_at, file_path, available
             FROM tracks WHERE id = ?1",
        )?;

        let track = stmt.query_row([id.to_string()], Self::row_to_track).ok();
        Ok(track)
    }

    pub fn get_track_by_url(&self, url: &str) -> Result<Option<Track>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, url, title, alias, duration, added_at, file_path, available
             FROM tracks WHERE url = ?1",
        )?;

        let track = stmt.query_row([url], Self::row_to_track).ok();
        Ok(track)
    }

    pub fn get_all_tracks(&self) -> Result<Vec<Track>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, url, title, alias, duration, added_at, file_path, available
             FROM tracks ORDER BY added_at DESC",
        )?;

        let tracks = stmt
            .query_map([], Self::row_to_track)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(tracks)
    }

    #[allow(dead_code)]
    pub fn search_tracks(&self, query: &str) -> Result<Vec<Track>> {
        let pattern = format!("%{query}%");
        let mut stmt = self.conn.prepare(
            "SELECT id, url, title, alias, duration, added_at, file_path, available
             FROM tracks
             WHERE title LIKE ?1 OR alias LIKE ?1
             ORDER BY added_at DESC",
        )?;

        let tracks = stmt
            .query_map([&pattern], Self::row_to_track)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(tracks)
    }

    #[allow(dead_code)]
    pub fn update_track_alias(&self, id: &Uuid, alias: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE tracks SET alias = ?1 WHERE id = ?2",
            params![alias, id.to_string()],
        )?;
        Ok(())
    }

    pub fn update_track_availability(&self, id: &Uuid, available: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE tracks SET available = ?1 WHERE id = ?2",
            params![available as i64, id.to_string()],
        )?;
        Ok(())
    }

    pub fn delete_track(&self, id: &Uuid) -> Result<()> {
        self.conn
            .execute("DELETE FROM tracks WHERE id = ?1", [id.to_string()])?;
        Ok(())
    }

    // Playlist operations
    pub fn insert_playlist(&self, playlist: &Playlist) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO playlists (id, name, created_at) VALUES (?1, ?2, ?3)",
                params![
                    playlist.id.to_string(),
                    playlist.name,
                    playlist.created_at.to_rfc3339(),
                ],
            )
            .with_context(|| "Failed to insert playlist")?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn get_playlist(&self, id: &Uuid) -> Result<Option<Playlist>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, created_at FROM playlists WHERE id = ?1")?;

        let playlist = stmt.query_row([id.to_string()], Self::row_to_playlist).ok();
        Ok(playlist)
    }

    pub fn get_playlist_by_name(&self, name: &str) -> Result<Option<Playlist>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, created_at FROM playlists WHERE name = ?1")?;

        let playlist = stmt.query_row([name], Self::row_to_playlist).ok();
        Ok(playlist)
    }

    pub fn get_all_playlists(&self) -> Result<Vec<Playlist>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, created_at FROM playlists ORDER BY name")?;

        let playlists = stmt
            .query_map([], Self::row_to_playlist)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(playlists)
    }

    #[allow(dead_code)]
    pub fn delete_playlist(&self, id: &Uuid) -> Result<()> {
        self.conn
            .execute("DELETE FROM playlists WHERE id = ?1", [id.to_string()])?;
        Ok(())
    }

    // Playlist track operations
    #[allow(dead_code)]
    pub fn add_track_to_playlist(&self, playlist_id: &Uuid, track_id: &Uuid) -> Result<()> {
        let position: i32 = self.conn.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_tracks WHERE playlist_id = ?1",
            [playlist_id.to_string()],
            |row| row.get(0),
        )?;

        self.conn.execute(
            "INSERT OR IGNORE INTO playlist_tracks (playlist_id, track_id, position)
             VALUES (?1, ?2, ?3)",
            params![playlist_id.to_string(), track_id.to_string(), position],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn remove_track_from_playlist(&self, playlist_id: &Uuid, track_id: &Uuid) -> Result<()> {
        self.conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND track_id = ?2",
            params![playlist_id.to_string(), track_id.to_string()],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn get_playlist_tracks(&self, playlist_id: &Uuid) -> Result<Vec<Track>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.url, t.title, t.alias, t.duration, t.added_at, t.file_path, t.available
             FROM tracks t
             INNER JOIN playlist_tracks pt ON t.id = pt.track_id
             WHERE pt.playlist_id = ?1
             ORDER BY pt.position",
        )?;

        let tracks = stmt
            .query_map([playlist_id.to_string()], Self::row_to_track)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(tracks)
    }

    pub fn get_all_playlist_tracks(&self) -> Result<Vec<PlaylistTrack>> {
        let mut stmt = self
            .conn
            .prepare("SELECT playlist_id, track_id, position FROM playlist_tracks")?;

        let entries = stmt
            .query_map([], |row| {
                Ok(PlaylistTrack {
                    playlist_id: row.get::<_, String>(0)?.parse().unwrap_or_default(),
                    track_id: row.get::<_, String>(1)?.parse().unwrap_or_default(),
                    position: row.get(2)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(entries)
    }

    #[allow(dead_code)]
    pub fn get_track_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    #[allow(dead_code)]
    pub fn get_playlist_track_count(&self, playlist_id: &Uuid) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM playlist_tracks WHERE playlist_id = ?1",
            [playlist_id.to_string()],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn insert_listen_event(
        &self,
        track_title: &str,
        source: &str,
        started_at: &str,
        duration_played: u64,
    ) -> Result<()> {
        if source != "local" && source != "stream" {
            anyhow::bail!("Invalid source '{}'. Expected 'local' or 'stream'", source);
        }

        self.conn.execute(
            "INSERT INTO listen_log (id, track_id, track_title, source, started_at, duration_played)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5)",
            params![
                Uuid::new_v4().to_string(),
                track_title,
                source,
                started_at,
                duration_played as i64
            ],
        )?;
        Ok(())
    }

    pub fn get_listen_history(&self, limit: usize) -> Result<Vec<ListenEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, track_title, source, started_at, duration_played
             FROM listen_log
             ORDER BY started_at DESC
             LIMIT ?1",
        )?;

        let events = stmt
            .query_map([limit as i64], |row| {
                Ok(ListenEvent {
                    id: row.get(0)?,
                    track_title: row.get(1)?,
                    source: row.get(2)?,
                    started_at: row.get(3)?,
                    duration_played: row.get::<_, i64>(4)? as u64,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(events)
    }
}

fn apply_migration_1(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS tracks (
            id TEXT PRIMARY KEY,
            url TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            alias TEXT,
            duration INTEGER NOT NULL,
            added_at TEXT NOT NULL,
            file_path TEXT NOT NULL,
            available INTEGER NOT NULL DEFAULT 1
        );

        CREATE TABLE IF NOT EXISTS playlists (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS playlist_tracks (
            playlist_id TEXT NOT NULL,
            track_id TEXT NOT NULL,
            position INTEGER NOT NULL,
            PRIMARY KEY (playlist_id, track_id),
            FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE,
            FOREIGN KEY (track_id) REFERENCES tracks(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_tracks_title ON tracks(title);
        CREATE INDEX IF NOT EXISTS idx_tracks_alias ON tracks(alias);
        CREATE INDEX IF NOT EXISTS idx_playlist_tracks_position ON playlist_tracks(playlist_id, position);
        "#,
    )
    .with_context(|| "Failed to apply schema migration v1")?;
    Ok(())
}

fn apply_migration_2(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS listen_log (
            id TEXT PRIMARY KEY,
            track_id TEXT,
            track_title TEXT NOT NULL,
            source TEXT NOT NULL CHECK(source IN ('local','stream')),
            started_at TEXT NOT NULL,
            duration_played INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .with_context(|| "Failed to apply schema migration v2")?;
    Ok(())
}

fn record_migration(tx: &Transaction<'_>, version: i64) -> Result<()> {
    tx.execute(
        "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
        params![version, Utc::now().to_rfc3339()],
    )
    .with_context(|| format!("Failed to record schema migration version {}", version))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_track_crud() {
        let db = Database::open_in_memory().unwrap();

        let track = Track::new(
            "https://youtube.com/watch?v=test".to_string(),
            "Test Track".to_string(),
            180,
            "/path/to/audio.opus".to_string(),
        );

        db.insert_track(&track).unwrap();

        let retrieved = db.get_track(&track.id).unwrap().unwrap();
        assert_eq!(retrieved.title, "Test Track");
        assert_eq!(retrieved.url, "https://youtube.com/watch?v=test");

        db.update_track_alias(&track.id, Some("my-track")).unwrap();
        let updated = db.get_track(&track.id).unwrap().unwrap();
        assert_eq!(updated.alias, Some("my-track".to_string()));

        db.delete_track(&track.id).unwrap();
        assert!(db.get_track(&track.id).unwrap().is_none());
    }

    #[test]
    fn test_playlist_operations() {
        let db = Database::open_in_memory().unwrap();

        let playlist = Playlist::new("My Playlist".to_string());
        db.insert_playlist(&playlist).unwrap();

        let track1 = Track::new(
            "https://youtube.com/watch?v=1".to_string(),
            "Track 1".to_string(),
            120,
            "/path/1.opus".to_string(),
        );
        let track2 = Track::new(
            "https://youtube.com/watch?v=2".to_string(),
            "Track 2".to_string(),
            180,
            "/path/2.opus".to_string(),
        );

        db.insert_track(&track1).unwrap();
        db.insert_track(&track2).unwrap();

        db.add_track_to_playlist(&playlist.id, &track1.id).unwrap();
        db.add_track_to_playlist(&playlist.id, &track2.id).unwrap();

        let tracks = db.get_playlist_tracks(&playlist.id).unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].title, "Track 1");
        assert_eq!(tracks[1].title, "Track 2");
    }
}
