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

## 11. Definition of Done

1. `clistream` Linux V1 flow is stable (`add/remove/play/status/daemon/tui`).
2. Linux CI, smoke tests, and release pipeline are green.
3. Runtime race and stale socket/pid handling are hardened.
4. Phase 5 advanced commands are Linux-tested and documented.
5. No extra engineering work is allocated to mac user optimization.

