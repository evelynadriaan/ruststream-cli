# Architecture

## Runtime model

```
┌────────────────────────────────────────────────────────┐
│                    User interface                       │
│                                                        │
│   CLI (one-shot)            TUI (interactive)          │
│   clistream play <q>        clistream  (or tui)        │
│   clistream add <url>                                  │
│   clistream history                                    │
└───────────────────┬────────────────────────────────────┘
                    │ JSON over Unix socket
                    │ $XDG_RUNTIME_DIR/clistream/clistream.sock
                    │ Protocol version header on every message
                    ▼
┌────────────────────────────────────────────────────────┐
│                      Daemon                             │
│                                                        │
│  ┌──────────────┐   ┌──────────────────────────────┐  │
│  │ IPC listener │   │     Playback engine          │  │
│  │ (Unix socket)│──▶│                              │  │
│  └──────────────┘   │  RodioBackend (local files)  │  │
│                     │  MpvBackend (streaming)       │  │
│  ┌──────────────┐   │                              │  │
│  │ Media keys   │   │  Queue + shuffle + repeat    │  │
│  │ MPRIS/D-Bus  │──▶│  PlaybackState (Arc<Mutex>)  │  │
│  └──────────────┘   └──────────────────────────────┘  │
│                                  │                      │
│                          mpsc channels                  │
│                          AudioCommand / Response        │
└──────────────────────────────────┬─────────────────────┘
                                   │
                                   ▼
┌────────────────────────────────────────────────────────┐
│              Storage (~/.local/share/clistream)         │
│                                                        │
│  clistream.db                                          │
│    tracks          — id, title, url, file_path, dur   │
│    playlists       — id, name, created_at             │
│    playlist_tracks — playlist_id, track_id, position  │
│    listen_log      — track_title, source, started_at  │
│    schema_migrations                                   │
│                                                        │
│  audio/                                               │
│    <uuid>.mp3  (one file per downloaded track)        │
└────────────────────────────────────────────────────────┘
```

## Key modules

| Module | Responsibility |
|--------|---------------|
| `src/main.rs` | Entry point. TTY detection, daemon auto-start, subcommand dispatch |
| `src/cli/mod.rs` | All CLI commands via clap. Thin — delegates to daemon via IPC |
| `src/tui/mod.rs` | ratatui TUI. AppMode state machine, all rendering, keyboard handler |
| `src/daemon/mod.rs` | Daemon process. IPC listener, playback state machine, media keys thread |
| `src/audio/mod.rs` | PlaybackBackend trait. RodioBackend (local) + MpvBackend (stream) |
| `src/ipc/mod.rs` | DaemonCommand / DaemonResponse enums. JSON serialisation. DaemonClient |
| `src/db/mod.rs` | SQLite via rusqlite. Schema migrations, all queries |
| `src/models/mod.rs` | Track, Playlist, StreamEntry, ListenEvent, PlaybackState |
| `src/download/mod.rs` | yt-dlp wrapper. Download, metadata fetch, retry with backoff |
| `src/config/mod.rs` | AppConfig. XDG path resolution for config/data/runtime dirs |
| `src/lyrics/mod.rs` | lrclib.net HTTP fetch. Returns plaintext lyrics by title |

## Design decisions

See [decisions.md](decisions.md) for documented tradeoffs.

## Threading model

- **Main thread:** IPC accept loop
- **Audio thread:** Receives `AudioCommand` via mpsc, drives RodioBackend or MpvBackend
- **Playback monitor thread:** Polls `is_finished()` every 1s, triggers auto-advance
- **Media controls thread:** D-Bus event loop (souvlaki)
- **TUI:** Single-threaded event loop, 250ms tick, polls daemon via IPC for status
