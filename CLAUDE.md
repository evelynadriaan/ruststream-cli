# CLAUDE.md

## Project Overview

clistream is a Linux-first CLI tool for saving, managing, and playing YouTube audio from the terminal. It downloads audio via yt-dlp and plays it through a background daemon. macOS is best-effort compile only — never a release or test gate.

**Canonical plan:** `docs/CLISTREAM_MIGRATION_PLAN.md` — read this before making any architectural decisions.

## Architecture

- **Daemon** (`src/daemon/mod.rs`): Background process that handles audio playback. Communicates via Unix socket IPC with protocol versioning.
- **TUI** (`src/tui/mod.rs`): Terminal UI built with ratatui. Connects to daemon as a client.
- **Audio** (`src/audio/mod.rs`): Wrapper around rodio for local file playback.
- **CLI** (`src/cli/`): clap-based commands. Running `clistream` with no args and a TTY opens TUI; non-TTY exits with guidance.
- **DB** (`src/db/mod.rs`): SQLite with schema_migrations table. Version-tracked from day one.
- **IPC** (`src/ipc/mod.rs`): Versioned envelope (protocol_version=1) with typed error codes over Unix socket.
- **Downloader** (`src/download/mod.rs`): Wraps yt-dlp to fetch video info and download audio.

## Paths

- Config: `~/.config/clistream/config.toml`
- Data: `~/.local/share/clistream/`
- Audio files: `~/.local/share/clistream/audio/`
- DB: `~/.local/share/clistream/clistream.db`
- Daemon socket: `$XDG_RUNTIME_DIR/clistream/clistream.sock` (fallback: data dir)

## Development

```bash
# Build
cargo build

# Run TUI (starts daemon automatically if not running)
cargo run

# Run with specific command
cargo run -- add "https://youtube.com/watch?v=..."
cargo run -- help

# Tests
cargo test

# Lint
cargo clippy -- -D warnings
cargo fmt --check
```

## External Dependencies (Linux)

Runtime: `yt-dlp`, `ffmpeg`
Build: `pkg-config`, `libasound2-dev`, `libdbus-1-dev`

```bash
sudo apt-get install -y pkg-config libasound2-dev libdbus-1-dev ffmpeg yt-dlp
```

## Key Patterns

- Daemon owns the audio player. TUI/CLI are IPC clients.
- All IPC messages use `DaemonRequestEnvelope` / `DaemonResponseEnvelope` with `protocol_version`.
- DB migrations run at startup via `schema_migrations` table — never run raw DDL outside of migration system.
- Socket and PID paths resolved via config methods — never hardcoded elsewhere.

## Phase Status

- Phase 0 (contracts/bootstrap): complete
- Phase 1 (Linux PoC): complete
- Phase 2 (V1 stabilization): complete
- Phase 3 (delivery/CI/release): complete
- Phase 4 (reliability optimization): complete
- Phase 5 (advanced features: next/prev/queue/shuffle/repeat/playlist): complete
- Phase 6 (YouTube streaming via mpv): complete
