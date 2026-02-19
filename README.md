# RustStream CLI

**Offline-first terminal audio library for YouTube — Rust + daemon + TUI.**

[![CI](https://github.com/evelynadriaan/ruststream-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/evelynadriaan/ruststream-cli/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/evelynadriaan/ruststream-cli)](https://github.com/evelynadriaan/ruststream-cli/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Platform: Linux](https://img.shields.io/badge/platform-Linux%20x86__64-lightgrey)](https://github.com/evelynadriaan/ruststream-cli/releases)

---

**Problem:** YouTube is the world's largest music library but accessing it requires a browser, ads, and an active connection.
**For:** Developers and power users who live in the terminal.
**Does:** Downloads YouTube audio to a local library and plays it from a keyboard-driven TUI with a background daemon.
**Why better:** Offline after download. No account. No Electron. Everything in one terminal window.
**Status:** v1.1.0 — stable. CI-tested on Linux x86_64. Prebuilt binary available.

---

## Why this exists

Browser tabs accumulate. Streaming services shuffle catalogues. This tool solves a specific problem: build a personal audio library from YouTube, own it locally, and control it entirely from the terminal. It is built for people who live in the terminal and want their music workflow to match.

---

## Features

- **Download & store** — saves YouTube audio via yt-dlp + ffmpeg, stores metadata in SQLite
- **Full library management** — add, remove, rename, search (fuzzy), import/export JSON
- **Playlists** — create, populate, and navigate named playlists
- **Queue, shuffle, repeat** — runtime queue with shuffle and repeat Off/One/All modes
- **YouTube streaming** — stream any URL or playlist via mpv without downloading first
- **YouTube search in TUI** — search by name, stream or add without leaving the player
- **Lyrics panel** — fetches synced lyrics from lrclib.net, shown in-player with `L`
- **Daemon architecture** — background playback engine with Unix socket IPC; survives terminal close
- **Linux media key integration** — MPRIS/D-Bus support for system media keys and lock screen controls
- **Local listen history** — every play logged to SQLite; view with `clistream history`

---

## Install

**Prerequisites:**
```bash
sudo apt-get update
sudo apt-get install -y ffmpeg yt-dlp mpv
```

**Install clistream:**
```bash
curl -fsSL https://raw.githubusercontent.com/evelynadriaan/ruststream-cli/main/install.sh | sh
```

Or download a prebuilt binary from [Releases](https://github.com/evelynadriaan/ruststream-cli/releases).

---

## Quickstart

```bash
# Add a track from YouTube
clistream add "https://youtube.com/watch?v=..."

# Open the interactive player
clistream
```

---

## How it works

```
┌─────────────────────────────────────────────────────┐
│                  User interface                      │
│                                                     │
│   CLI commands          TUI (ratatui)               │
│   clistream play        clistream  (or clistream tui)│
└────────────────┬────────────────────────────────────┘
                 │ Unix socket IPC (JSON, versioned)
                 ▼
┌─────────────────────────────────────────────────────┐
│                    Daemon                            │
│                                                     │
│   Playback engine    Queue + shuffle/repeat         │
│   rodio (local)      mpv subprocess (streaming)     │
│   Media keys (MPRIS/D-Bus)                          │
└────────────────┬────────────────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────────────────┐
│              Storage (~/.local/share/clistream)      │
│                                                     │
│   clistream.db   tracks, playlists, listen_log      │
│   audio/         <uuid>.mp3 per downloaded track    │
└─────────────────────────────────────────────────────┘
```

The daemon starts automatically on first use and runs in the background. The CLI and TUI are thin IPC clients — multiple terminals can connect to the same daemon simultaneously.

---

## Keyboard shortcuts

### Navigation
| Key | Action |
|-----|--------|
| `↑` / `k` | Select previous track |
| `↓` / `j` | Select next track |
| `←` / `→` | Previous / next track (or seek ±10s when not streaming) |

### Playback
| Key | Action |
|-----|--------|
| `Space` | Play / pause |
| `Enter` | Play selected track |
| `+` / `=` | Volume up |
| `-` | Volume down |
| `L` | Toggle lyrics panel |

### Library & streaming
| Key | Action |
|-----|--------|
| `a` | Add track by URL |
| `s` | Stream a YouTube URL or playlist |
| `S` | Save current stream to library |
| `?` | Search YouTube by name |
| `d` / Delete | Remove selected track |
| `e` | Rename / set alias |
| `/` | Search local library |
| `l` | Switch to local library view |
| `q` | Quit |

---

## CLI reference

```bash
clistream add <url>           # Download and add a track
clistream play <query>        # Play by title (fuzzy match)
clistream stream <url>        # Stream without downloading
clistream search <query>      # Search local library
clistream history --limit 20  # View listening history
clistream playlist create <name>
clistream queue list
clistream status
clistream help                # Full command list
```

---

## Product decisions & non-goals

**Offline-first is intentional.** Downloaded audio plays without a network connection, survives rate-limits, and does not depend on YouTube availability. Streaming is an additional mode, not the primary one.

**Non-goals for now:**
- Playlist import from Spotify / Apple Music
- Audio normalisation (replaygain / loudness)
- ID3 tag editing
- Remote control / multi-device sync
- macOS as a supported release target (best-effort compile only)
- A GUI or web interface

---

## Platform support

| Platform | Status |
|----------|--------|
| Linux x86_64 | Supported — prebuilt binaries, CI-tested |
| macOS | Best-effort compile only — not a release target |
| Windows | Not supported |

---

## License

MIT — see [LICENSE](LICENSE).
Original codebase by David (2025). Modifications by Nicolai (2026).
