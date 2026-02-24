# Changelog

All notable changes to this project will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
Versioning follows [Semantic Versioning](https://semver.org/).

---

## [1.2.0] — 2026-02-24

### Added
- Optional Discord Rich Presence support — set `discord_rich_presence = true` in `~/.config/clistream/config.toml` to enable

---

## [1.1.0] — 2026-02-19

### Added
- YouTube streaming directly from TUI — press `s` to stream any URL or playlist
- Playlist queue in TUI — playlist tracks loaded into library panel, navigable with `←` / `→`
- YouTube search from TUI — press `?` to search by name, stream or add from results
- Lyrics panel — press `L` to show/hide synced lyrics fetched from lrclib.net
- Remove track from TUI — `d` or Delete key removes selected track
- Save stream to library from TUI — `S` (Shift+S) saves current stream as a download
- Local listen log — every play recorded to SQLite; `clistream history --limit N` to view
- `StreamQueueLoad` IPC command for pre-fetched playlist metadata

### Fixed
- Auto-advance: tracks now play the next item when finished instead of stopping silently
- Repeat One mode now correctly loops the current track (was stored but not enforced)

---

## [1.0.0] — 2026-02-19

Initial public release. Forked and significantly extended from [davidhariri/mixyt](https://github.com/davidhariri/mixyt).

### Core features
- Download YouTube audio via yt-dlp + ffmpeg, stored as MP3 in `~/.local/share/clistream/audio/`
- SQLite library with tracks, playlists, and schema migrations
- Background daemon with Unix socket IPC (JSON, protocol versioned)
- TUI player built with ratatui — now-playing, library, progress bar, keyboard controls
- CLI: add, remove, play, pause, resume, stop, seek, volume, next, prev, queue, shuffle, repeat
- Playlist management: create, add, remove, list, delete
- YouTube streaming via mpv subprocess with JSON IPC
- Linux media key integration via MPRIS/D-Bus (souvlaki)
- Fuzzy search with skim
- Export/import library as JSON
- GitHub Actions CI (lint, test, clippy, release build)
- Prebuilt Linux x86_64 binary attached to releases
