# clistream Codebase Context

Generated: 2026-02-19  
Repository: `evelynadriaan/ruststream-cli`

## Executive Summary

- `clistream` is a Linux-first CLI tool for managing and playing YouTube audio.
- Runtime playback uses a daemon with IPC over a Unix socket.
- Local file playback uses rodio; streaming playback uses mpv JSON IPC.
- macOS is best-effort compile only, not a release target.

## Current Architecture

1. CLI entrypoint: `src/main.rs`
2. CLI command layer: `src/cli/mod.rs`, `src/cli/commands.rs`
3. Daemon/runtime: `src/daemon/mod.rs`
4. Audio backends: `src/audio/mod.rs`
5. IPC contracts/client: `src/ipc/mod.rs`
6. Database layer: `src/db/mod.rs`
7. Config/path policy: `src/config/mod.rs`
8. Downloader integration: `src/download/mod.rs`
9. TUI: `src/tui/mod.rs`

## Path Policy

1. Config: `~/.config/clistream/config.toml`
2. Data: `~/.local/share/clistream/`
3. Database: `~/.local/share/clistream/clistream.db`
4. Audio files: `~/.local/share/clistream/audio/`
5. Daemon socket: `$XDG_RUNTIME_DIR/clistream/clistream.sock` (fallback to data dir)
6. mpv IPC socket: `$XDG_RUNTIME_DIR/clistream/clistream-mpv.sock` (fallback to data dir)

## Command Surface (High Level)

1. Core library: `add`, `remove`, `play`, `pause`, `resume`, `stop`, `seek`, `volume`, `status`, `list`, `search`
2. Daemon: `daemon start|stop|status|run`
3. Advanced playback: `next`, `prev`, `queue`, `shuffle`, `repeat`
4. Playlists: `playlist create|add|remove|list|delete`
5. Streaming: `stream <url>`, `stream <playlist-url>`, `save`
6. UI and data: `tui`, `export`, `import`, `check`

## Delivery and CI

1. Linux build/test/lint/format/release checks are the required gates.
2. Installer and docs are Linux-focused.
3. Release artifacts target Linux.
4. macOS checks are compile-only and non-blocking.

## Quick Re-entry Checklist

1. Read `CLAUDE.md`
2. Read `docs/CLISTREAM_MIGRATION_PLAN.md`
3. Run:
   - `cargo clippy -- -D warnings`
   - `cargo fmt --check`
   - `cargo test`
4. Run smoke script:
   - `scripts/smoke/linux_core.sh`
