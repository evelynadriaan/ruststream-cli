use clap::{Parser, Subcommand};

mod commands;
pub use commands::*;

#[derive(Parser)]
#[command(name = "clistream")]
#[command(about = "A CLI tool for saving, managing, and playing YouTube audio")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Add a track to the library from a YouTube URL
    Add {
        /// YouTube URL
        url: String,
        /// Optional alias for quick reference
        #[arg(short, long)]
        alias: Option<String>,
    },

    /// Remove a track from the library
    Remove {
        /// Track name, alias, or search query
        query: String,
    },

    /// Play a track
    Play {
        /// Track name, alias, or search query
        query: String,
    },

    /// Pause playback
    Pause,

    /// Resume playback
    Resume,

    /// Stop playback
    Stop,

    /// Seek to a position (e.g., "1:30" or "90")
    Seek {
        /// Position in seconds or MM:SS format
        position: String,
    },

    /// Set or show volume
    Volume {
        /// Volume level (0-100)
        level: Option<u8>,
    },

    /// List tracks in the library
    List,

    /// Search the library
    Search {
        /// Search query
        query: String,
    },

    /// Show current playback status
    Status,

    /// Stream a YouTube URL or playlist via mpv without saving
    Stream {
        /// URL to stream
        url: String,
    },

    /// Save the currently streaming track to the library in background
    Save,

    /// Skip to the next track in the queue
    Next,

    /// Return to the previous track in the queue
    Prev,

    /// Queue management
    Queue {
        #[command(subcommand)]
        command: QueueCommands,
    },

    /// Set or show shuffle mode
    Shuffle {
        /// Shuffle mode (on/off)
        #[arg(value_parser = ["on", "off"])]
        mode: Option<String>,
    },

    /// Set or show repeat mode
    Repeat {
        /// Repeat mode (off/one/all)
        #[arg(value_parser = ["off", "one", "all"])]
        mode: Option<String>,
    },

    /// Playlist management
    Playlist {
        #[command(subcommand)]
        command: PlaylistCommands,
    },

    /// Daemon management
    Daemon {
        #[command(subcommand)]
        command: DaemonCommands,
    },

    /// Export library to JSON
    Export {
        /// Output file path
        #[arg(short, long)]
        file: Option<String>,
    },

    /// Import library from JSON
    Import {
        /// Input file path
        file: String,
    },

    /// Check track availability
    Check,

    /// Launch interactive TUI
    #[command(name = "tui")]
    Tui,
}

#[derive(Subcommand)]
pub enum DaemonCommands {
    /// Start the daemon
    Start,
    /// Stop the daemon
    Stop,
    /// Show daemon status
    Status,
    /// Run daemon in foreground (internal use)
    Run,
}

#[derive(Subcommand)]
pub enum QueueCommands {
    /// Add a track to the queue
    Add {
        /// Track name, alias, or search query
        query: String,
    },
    /// List queued tracks
    List,
    /// Clear queued tracks
    Clear,
}

#[derive(Subcommand)]
pub enum PlaylistCommands {
    /// Create a playlist
    Create {
        /// Playlist name
        name: String,
    },
    /// Add a track to a playlist
    Add {
        /// Playlist name
        playlist: String,
        /// Track name, alias, or search query
        query: String,
    },
    /// Remove a track from a playlist
    Remove {
        /// Playlist name
        playlist: String,
        /// Track name, alias, or search query
        query: String,
    },
    /// List playlists
    List,
    /// Delete a playlist
    Delete {
        /// Playlist name
        name: String,
    },
}
