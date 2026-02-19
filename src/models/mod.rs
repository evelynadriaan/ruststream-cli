use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: Uuid,
    pub url: String,
    pub title: String,
    pub alias: Option<String>,
    pub duration: u64,
    pub added_at: DateTime<Utc>,
    pub file_path: String,
    pub available: bool,
}

impl Track {
    pub fn new(url: String, title: String, duration: u64, file_path: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            url,
            title,
            alias: None,
            duration,
            added_at: Utc::now(),
            file_path,
            available: true,
        }
    }

    pub fn display_name(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.title)
    }

    pub fn format_duration(&self) -> String {
        let minutes = self.duration / 60;
        let seconds = self.duration % 60;
        format!("{minutes}:{seconds:02}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Playlist {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

impl Playlist {
    #[allow(dead_code)]
    pub fn new(name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistTrack {
    pub playlist_id: Uuid,
    pub track_id: Uuid,
    pub position: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEntry {
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    One,
    All,
}

impl std::fmt::Display for RepeatMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepeatMode::Off => write!(f, "off"),
            RepeatMode::One => write!(f, "one"),
            RepeatMode::All => write!(f, "all"),
        }
    }
}

impl std::str::FromStr for RepeatMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "off" => Ok(RepeatMode::Off),
            "one" => Ok(RepeatMode::One),
            "all" => Ok(RepeatMode::All),
            _ => Err(format!("Invalid repeat mode: {s}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlaybackState {
    pub current_track: Option<Track>,
    pub queue: Vec<Track>,
    pub queue_index: usize,
    pub stream_queue: Vec<StreamEntry>,
    pub stream_queue_index: usize,
    pub is_streaming: bool,
    pub is_playing: bool,
    pub volume: u8,
    pub position: u64,
    pub shuffle: bool,
    pub repeat: RepeatMode,
}

impl PlaybackState {
    pub fn new() -> Self {
        Self {
            volume: 80,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryExport {
    pub version: String,
    pub exported_at: DateTime<Utc>,
    pub tracks: Vec<Track>,
    pub playlists: Vec<Playlist>,
    pub playlist_tracks: Vec<PlaylistTrack>,
}

impl LibraryExport {
    pub fn new(
        tracks: Vec<Track>,
        playlists: Vec<Playlist>,
        playlist_tracks: Vec<PlaylistTrack>,
    ) -> Self {
        Self {
            version: "1.0".to_string(),
            exported_at: Utc::now(),
            tracks,
            playlists,
            playlist_tracks,
        }
    }
}
