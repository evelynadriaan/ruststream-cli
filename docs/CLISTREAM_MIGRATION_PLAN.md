# clistream Migration Plan (Canonical)

Updated: 2026-02-19
Status: Source of truth for implementation.

## 1. Non-Negotiables

1. Linux-first delivery. Linux is the only required platform gate.
2. macOS is best-effort compile only, never a release/test blocker.
3. Fresh-start product. No legacy `mixyt` user/data migration work.
4. PoC first, then full migration/optimization.
5. Use one canonical plan file only: this file.

## 2. Repository and Crate Structure (Decision Complete)

1. Keep current Git repo (`mixyt`) as implementation repo.
2. Build `clistream` as the root package in this same repo (not a sibling repo, not dual-repo).
3. Do not keep a dual runtime mode. Old `mixyt` behavior remains only in git history.
4. Target paths in the new app:
5. Config: `~/.config/clistream/config.toml` (`dirs::config_dir()/clistream/config.toml`).
6. Data: `~/.local/share/clistream` (`dirs::data_dir()/clistream`).
7. Audio: `<data>/audio`.
8. DB: `<data>/clistream.db`.
9. PID: `<data>/clistream.pid`.
10. Socket: `dirs::runtime_dir()/clistream/clistream.sock`, fallback `<data>/clistream.sock`.

## 3. Linux Build Dependencies (Must Be Explicit)

CI and installer must install/check at minimum:

1. `pkg-config`
2. `libasound2-dev` (ALSA compile dependency for `rodio/cpal`)
3. `libdbus-1-dev` (Linux D-Bus compile dependency for `souvlaki`)
4. Runtime dependencies: `ffmpeg`, `yt-dlp`

CI requirement for Ubuntu jobs:

```bash
sudo apt-get update
sudo apt-get install -y pkg-config libasound2-dev libdbus-1-dev ffmpeg yt-dlp
```

## 4. Contract Definitions (Phase 0 Deliverables)

## 4.1 IPC envelope

1. `protocol_version: 1` on every request/response.
2. Typed error code vocabulary (fixed now, not deferred):
3. `DAEMON_UNAVAILABLE`
4. `IPC_PROTOCOL_MISMATCH`
5. `TRACK_NOT_FOUND`
6. `TRACK_UNAVAILABLE`
7. `INVALID_TIME_FORMAT`
8. `DEPENDENCY_MISSING`
9. `DOWNLOAD_FAILED`
10. `AUDIO_INIT_FAILED`
11. `AUDIO_PLAY_FAILED`
12. `DB_ERROR`
13. `INTERNAL_ERROR`

## 4.2 DB schema bootstrap

1. Create `schema_migrations(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)`.
2. `version = 1` is baseline schema creation for tracks/playlists/playlist_tracks + indexes.
3. On DB init:
4. If `schema_migrations` empty: run baseline DDL in one transaction, insert version `1`.
5. Else: apply pending migrations sequentially in version order, one transaction per migration.
6. Any migration failure rolls back current transaction and aborts startup.

## 4.3 Config

1. Add `config_version = 1` under `[app]`.
2. `StorageConfig::default()` must be rewritten for new `data_dir` behavior (not just renamed).

## 5. PoC Scope (Phase 1)

Goal: prove Linux viability with reliability baseline, not just compile.

Required PoC deliverables:

1. Linux CI required job green (`build`, `test`, `clippy`, `fmt`).
2. End-to-end CLI smoke on Linux:
3. `daemon start`
4. `add`
5. `play`
6. `status`
7. `daemon stop`
8. One integration test for daemon lifecycle on Linux.
9. Minimal start-up race fix moved into PoC:
10. Replace socket-file existence polling with readiness handshake (`GetStatus` loop with timeout/jitter).

PoC acceptance:

1. Linux required jobs green.
2. No macOS gate requirement.
3. Smoke script reproducible from `scripts/smoke/linux_core.sh`.

PoC rollback/fallback rule:

1. If Linux audio backend proves unreliable after race fix, pivot immediately to backend abstraction:
2. `AudioBackend` trait with `rodio` implementation first.
3. Add `mpv --no-video` subprocess backend fallback for Linux.
4. Keep this pivot in Phase 1 if blocker persists > 1 working day.

## 6. CLI Behavior Decisions

1. V1 includes `remove <query>` explicitly.
2. Default command behavior:
3. If no subcommand and interactive TTY: launch TUI.
4. If no subcommand and non-TTY: do not launch TUI; print guidance and exit non-zero.
5. CI tests must always invoke explicit subcommands.

V1 commands:

1. `add`, `remove`, `play`, `pause`, `resume`, `stop`, `seek`, `volume`, `status`, `list`, `search`, `daemon start|stop|status|run`, `tui`.

Phase 5 commands:

1. `next`, `prev`, queue, shuffle, repeat, playlist surfaces.

## 7. Linux Renaming Tasks That Must Be Explicit

1. Replace hardcoded service identity strings:
2. `dbus_name: "mixyt"` -> `dbus_name: "clistream"`.
3. `display_name: "mixyt"` -> `display_name: "clistream"`.
4. Audit user-facing names in logs/errors/help text for `mixyt` leftovers.

## 8. Multi-Agent File Ownership Map

This is mandatory to enable parallelism safely.

1. Agent 1 (bootstrap/toolchain): `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, root crate identity.
2. Agent 2 (CI): `.github/workflows/*`.
3. Agent 3 (release artifacts): `scripts/release/*`, release workflow packaging sections.
4. Agent 4 (installer/docs): `install.sh`, `docs/install/*`, README install sections.
5. Agent 5 (CLI): `src/main.rs`, `src/cli/*`.
6. Agent 6 (daemon/runtime/audio/ipc): `src/daemon/*`, `src/audio/*`, `src/ipc/*`.
7. Agent 7 (data/config/models): `src/db/*`, `src/config/*`, `src/models/*`, `src/migrations/*`.
8. Agent 8 (tests): `tests/*`, `scripts/smoke/*`, `src/**/tests` only for test additions.

Shared-file protocol:

1. If a change touches another agent's owned file, submit contract PR first and rebase dependent branches.
2. Agents 5 and 6 coordinate via IPC contract only, not ad-hoc.

## 9. Phase Plan With Real Parallelism

## Phase 0 (serial-critical, short)

1. Agent 1 + Agent 7 define contracts and baseline schema/config.
2. Agent 2 prepares Linux CI scaffolding in parallel (blocked only on toolchain pin).
3. Agents 3/4/8 prepare scripts/tests/docs stubs in parallel.

## Phase 1 (PoC)

1. Agents 2/5/6/8 execute Linux PoC and handshake race fix.
2. Agent 4 finalizes Linux dependency checks in installer/docs.

## Phase 2 (V1 stabilization)

1. Agents 5/6/7 complete V1 command/runtime/data behavior.
2. Agent 8 expands Linux integration coverage.

## Phase 3 (delivery)

1. Agents 2/3/4 finalize Linux CI/release/installer docs.
2. Linux artifacts and checksums become release gate.

## Phase 4 (reliability optimization)

1. Agent 6 state-machine and daemon resilience hardening.
2. Agent 8 fault-injection/restart/recovery tests.

## Phase 5 (advanced features)

1. Agent 5 adds advanced commands.
2. Agent 6/7 add runtime/data support as needed.
3. Agent 8 adds coverage for each new command family.

## 10. Testing and Gates

Linux required gates:

1. `cargo fmt --check`
2. `cargo clippy -- -D warnings`
3. `cargo test` (unit + integration)
4. Linux smoke script
5. Release-target build for Linux artifacts

macOS:

1. Optional compile-only check.
2. Never blocks merge or release.

## 11. Phase 6 — YouTube Streaming via mpv (Plan Amendment 2026-02-19)

### 11.1 Decision: mpv subprocess as streaming backend

Streaming backend is **mpv spawned as a subprocess**, controlled via its JSON IPC socket.

Rationale:
- `yt-dlp --get-url` CDN URLs contain `expire=<unix_timestamp>` and go stale (~6 hours) — unsuitable for playlists or long sessions.
- yt-dlp pipe → rodio cannot seek without re-spawning the process.
- mpv has yt-dlp integration built in, handles buffering/seeking/format internally, and exposes a clean JSON IPC socket that mirrors clistream's existing daemon IPC pattern.
- No compile-time dependency (no `libmpv` crate) — subprocess only, keeping build simple.

### 11.2 Architecture

Add `PlaybackBackend` trait to `src/audio/mod.rs`:

```
PlaybackBackend (trait)
  ├── RodioBackend  — local file playback (existing, unchanged)
  └── MpvBackend    — stream URL playback (new, Phase 6)
```

Both backends expose: `play`, `pause`, `resume`, `stop`, `seek`, `get_position`, `set_volume`, `is_finished`.

The daemon's audio thread holds whichever backend is active. Switching backends happens on the next play/stream command — no mid-session switch.

`MpvBackend` spawns:
```
mpv --no-video --no-terminal --input-ipc-server=<mpv_socket_path> <url>
```

mpv socket path policy: `dirs::runtime_dir()/clistream/clistream-mpv.sock`, fallback `<data>/clistream-mpv.sock`.
Same runtime dir fallback policy as main daemon socket.

### 11.3 mpv JSON IPC commands used

```json
{"command": ["loadfile", "<url>"]}
{"command": ["set_property", "pause", true]}
{"command": ["set_property", "pause", false]}
{"command": ["seek", <seconds>, "absolute"]}
{"command": ["get_property", "playback-time"]}
{"command": ["set_property", "volume", <0-100>]}
{"command": ["quit"]}
```

mpv responds with `{"data": <value>, "error": "success"}` or `{"error": "<message>"}`.

### 11.4 New CLI commands

```
clistream stream <url>              # stream single URL via mpv without saving
clistream stream <playlist-url>     # stream entire YouTube playlist via mpv
clistream save                      # while streaming: download current track to library in background
```

`clistream save` behavior:
- Sends `SaveCurrentStream` IPC command to daemon.
- Daemon captures the current stream URL and spawns a background `yt-dlp` download thread.
- Playback continues uninterrupted from mpv while download runs.
- On download completion, track is added to DB silently.
- Does NOT switch playback from mpv to rodio after save — stream continues until it ends or user stops.
- Code must include: `// TODO(streaming-v2): switch to local rodio playback after save completes, with position handoff`

### 11.5 New IPC contract additions

New `DaemonCommand` variants:
```
Stream { url: String }
StreamPlaylist { url: String }
SaveCurrentStream
```

New `DaemonErrorCode` variants:
```
MPV_UNAVAILABLE       // mpv binary not found at runtime
MPV_IPC_ERROR         // mpv socket communication failure
MPV_LOAD_FAILED       // mpv failed to load the URL
```

`protocol_version` remains `1` — these are additive variants.

### 11.6 Dependency changes

mpv is an **optional runtime dependency** — required only for `stream` commands.

Installer (`install.sh`) dependency checks:
- Add mpv check after existing yt-dlp/ffmpeg checks.
- If mpv absent: warn user that streaming is unavailable, do not fail install.

CI (`.github/workflows/ci.yml`):
- Add `mpv` to Linux apt install block.
- Streaming integration test gated behind `mpv` presence check.

No new compile-time dependencies. No changes to `Cargo.toml`.

### 11.7 File ownership (Phase 6 agents)

Follows existing ownership map:
- Agent 5 (CLI): adds `stream`, `save` subcommands in `src/cli/`.
- Agent 6 (daemon/audio): adds `PlaybackBackend` trait, `MpvBackend`, `SaveCurrentStream` handler in `src/audio/`, `src/daemon/`.
- Agent 7 (IPC/config): adds new `DaemonCommand` variants and error codes to `src/ipc/mod.rs`.
- Agent 2 (CI): adds mpv to CI apt install.
- Agent 4 (installer/docs): adds mpv to installer dependency check and docs.
- Agent 8 (tests): adds streaming integration tests.

### 11.8 Phase sequencing

Phase 6 executes after Phase 5. Reason: Phase 5 builds queue/playlist infrastructure that `stream <playlist-url>` depends on.

Phase 6 acceptance criteria:
1. `clistream stream <url>` plays audio on Linux via mpv.
2. `clistream stream <playlist-url>` streams a YouTube playlist sequentially.
3. `clistream save` downloads current stream track to library without interrupting playback.
4. If mpv not installed, all three commands fail with `MPV_UNAVAILABLE` and a clear install message.
5. `clistream play` (library) and all existing commands unaffected.
6. Linux CI streaming test green.
7. Smoke script updated with streaming scenario.

### 11.9 Future intent (not in Phase 6 scope)

- Auto-switch from mpv to rodio after `clistream save` completes, with seek position handoff.
- `clistream stream` query matching library first, falling back to streaming if not found.
- Last.fm / ListenBrainz scrobbling for streamed tracks.

---

## 12. Definition of Done

1. `clistream` Linux V1 flow is stable (`add/remove/play/status/daemon/tui`).
2. Linux CI, smoke tests, and release pipeline are green.
3. Runtime race and stale socket/pid handling are hardened.
4. Phase 5 advanced commands are Linux-tested and documented.
5. Phase 6 streaming (`stream`, `save`, playlist streaming) works on Linux via mpv.
6. No extra engineering work is allocated to mac user optimization.

