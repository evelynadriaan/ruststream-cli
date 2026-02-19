# Decisions

Key technical and product tradeoffs made during development.

---

## Daemon architecture over in-process playback

**Decision:** Background daemon with Unix socket IPC rather than running the audio engine in the CLI process.

**Why:** Playback must survive terminal close and support multiple concurrent clients (CLI + TUI simultaneously). In-process audio stops when the process exits.

**Tradeoff:** More complex startup (auto-start logic, PID file, socket cleanup). Mitigated by transparent auto-start on first command.

---

## mpv subprocess for streaming (not in-process)

**Decision:** Stream via mpv subprocess controlled over JSON IPC socket, rather than piping yt-dlp audio into rodio.

**Why:** mpv handles codec negotiation, buffering, and network errors. A pipe-based approach cannot seek and breaks on network hiccups. mpv has been solving this problem for years.

**Tradeoff:** mpv is a runtime dependency for streaming. Non-fatal — tool degrades gracefully if mpv is absent (streaming commands return a clear error, local playback unaffected).

---

## rodio for local playback (not mpv for everything)

**Decision:** Use rodio for local file playback and mpv only for streaming.

**Why:** rodio is pure Rust, embeds cleanly, and has zero external dependencies for local MP3/OGG playback. Using mpv for everything would make mpv a hard dependency even for users who only want a local library.

---

## Offline-first download model as default

**Decision:** Default workflow is download-then-play, not stream-then-optionally-save.

**Why:** Local files are instant to start, work without network, and survive YouTube URL changes. Streaming is an additional mode for discovery and playlists, not the primary flow.

---

## SQLite over flat files

**Decision:** SQLite for all persistent state (tracks, playlists, listen log, migrations).

**Why:** Relational queries, atomic transactions, schema migrations with version tracking. A flat JSON file breaks on concurrent writes from CLI + daemon.

---

## XDG base directory spec

**Decision:** All paths follow XDG: config in `$XDG_CONFIG_HOME/clistream`, data in `$XDG_DATA_HOME/clistream`, socket in `$XDG_RUNTIME_DIR/clistream`.

**Why:** Correct Linux behaviour. Allows per-user isolation and works cleanly in containers and CI environments with isolated XDG dirs.

---

## Linux-only release target

**Decision:** Ship prebuilt binaries for Linux x86_64 only. macOS is best-effort compile.

**Why:** MPRIS/D-Bus media key integration is Linux-specific. The audio stack (ALSA/PulseAudio/PipeWire) is Linux-native. Maintaining macOS CI and binaries for an untested platform is noise.
