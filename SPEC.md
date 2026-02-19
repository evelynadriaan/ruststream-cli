# clistream

A command-line tool for saving, managing, and playing YouTube audio.

## Overview

**clistream** is a CLI tool that allows developers to build a personal library of audio from YouTube videos. It downloads and stores audio locally, provides organization through playlists, and offers full playback controls including background play and media key integration.

## Target Platforms

- Linux
- macOS (best-effort compile only, not a release target)

## Core Features

### 1. Audio Management

#### Save Audio
- Download audio from YouTube URLs
- Extract and store audio in a standard format (e.g., opus, mp3)
- Auto-fetch metadata: title, duration, channel
- Optional custom alias for quick reference

#### Library Storage
- Local filesystem storage in a dedicated directory (e.g., `~/.local/share/clistream/`)
- SQLite database for metadata and library state
- Audio files stored alongside database

#### Organization
- **Flat list**: All tracks accessible as a single collection
- **Playlists**: User-created named collections of tracks
- Tracks can belong to multiple playlists

### 2. Search & Discovery

- Search by YouTube title (auto-fetched)
- Search by custom alias (user-assigned)
- Fuzzy matching for approximate searches
- List all tracks or filter by playlist

### 3. Playback

#### Modes
- **Single track**: Play one track
- **Queue/Playlist**: Play multiple tracks in sequence
- **Shuffle**: Randomize playback order
- **Repeat**: Loop single track or entire queue

#### Controls
- Play / Pause / Stop
- Next / Previous track
- Seek forward/backward
- Volume control

#### Background Playback
- Daemon process handles audio playback
- Continues playing while terminal is used for other tasks
- Survives terminal closure (optional)

### 4. Media Key Integration

- Respond to system media keys (play/pause, next, previous)
- Works on Linux (via MPRIS/D-Bus). macOS is best-effort compile only.
- Requires background daemon to be running

### 5. User Interface

#### Subcommand CLI (Primary)
Standard command-line interface with subcommands:

```
clistream add <url> [--alias <name>]      # Add track to library
clistream remove <query>                   # Remove track from library
clistream play <query>                     # Play a track
clistream pause                            # Pause playback
clistream resume                           # Resume playback
clistream stop                             # Stop playback
clistream next                             # Skip to next track
clistream prev                             # Go to previous track
clistream seek <time>                      # Seek to position
clistream volume <0-100>                   # Set volume
clistream list [--playlist <name>]         # List tracks
clistream search <query>                   # Fuzzy search library
clistream playlist create <name>           # Create playlist
clistream playlist add <playlist> <query>  # Add track to playlist
clistream playlist remove <playlist> <query>
clistream playlist list                    # List all playlists
clistream queue add <query>                # Add to current queue
clistream queue list                       # Show current queue
clistream queue clear                      # Clear queue
clistream shuffle [on|off]                 # Toggle shuffle
clistream repeat [off|one|all]             # Set repeat mode
clistream status                           # Show current playback status
clistream daemon start                     # Start background daemon
clistream daemon stop                      # Stop background daemon
clistream daemon status                    # Check daemon status
clistream export [--file <path>]           # Export library to JSON
clistream import <file>                    # Import library from JSON
```

#### Interactive TUI (Secondary)
Full-screen terminal interface with:
- Track/playlist browsing
- Playback controls
- Queue management
- Keyboard navigation

Launched via: `clistream tui` or `clistream -i`

### 6. Export & Backup

- Export library metadata to JSON
- Import from JSON backup
- Does not include audio files (re-downloads on import)

## Data Model

### Track
| Field       | Type     | Description                    |
|-------------|----------|--------------------------------|
| id          | UUID     | Unique identifier              |
| url         | string   | YouTube URL                    |
| title       | string   | Video title (auto-fetched)     |
| alias       | string?  | Optional custom name           |
| duration    | integer  | Duration in seconds            |
| added_at    | datetime | When track was added           |
| file_path   | string   | Path to local audio file       |
| available   | boolean  | Whether source is still valid  |

### Playlist
| Field       | Type     | Description                    |
|-------------|----------|--------------------------------|
| id          | UUID     | Unique identifier              |
| name        | string   | Playlist name                  |
| created_at  | datetime | When playlist was created      |

### PlaylistTrack
| Field       | Type     | Description                    |
|-------------|----------|--------------------------------|
| playlist_id | UUID     | Reference to playlist          |
| track_id    | UUID     | Reference to track             |
| position    | integer  | Order in playlist              |

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    clistream CLI                        │
│  (subcommands, TUI, communicates with daemon via IPC)   │
└─────────────────────┬───────────────────────────────────┘
                      │ IPC (Unix socket)
┌─────────────────────▼───────────────────────────────────┐
│                 clistream daemon                        │
│  - Audio playback engine                                │
│  - Queue management                                     │
│  - Media key listener                                   │
│  - Playback state                                       │
└─────────────────────┬───────────────────────────────────┘
                      │
┌─────────────────────▼───────────────────────────────────┐
│                    Storage                              │
│  ~/.local/share/clistream/                              │
│  ├── clistream.db    (SQLite database)                  │
│  ├── audio/          (downloaded audio files)           │
│  └── clistream.sock  (daemon socket)                    │
└─────────────────────────────────────────────────────────┘
```

## Error Handling

### Unavailable Videos
- When a YouTube video becomes unavailable (deleted, private, etc.)
- Track remains in library but marked as `available: false`
- User notified when attempting to play unavailable track
- Periodic health check command: `clistream check`

### Network Errors
- Graceful failure with clear error messages
- Retry logic for transient failures
- Offline mode: can play already-downloaded tracks

## Dependencies

External tools:
- **yt-dlp**: YouTube audio extraction (must be installed)
- **ffmpeg**: Audio processing (must be installed)

## Configuration

Config file: `~/.config/clistream/config.toml`

```toml
[storage]
path = "~/.local/share/clistream" # Library location

[audio]
format = "opus"             # Preferred audio format
quality = "best"            # Audio quality

[daemon]
auto_start = true           # Start daemon automatically

[playback]
default_volume = 80         # Default volume (0-100)
```

## Future Considerations (Out of Scope for v1)

- YouTube playlist import
- Tag/category system
- Audio normalization
- Discord Rich Presence integration
- Remote control via web interface
- Sync across devices
