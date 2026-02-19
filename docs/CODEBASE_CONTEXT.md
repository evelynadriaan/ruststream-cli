# mixyt Codebase Context

Generated: 2026-02-19
Repo snapshot: fork of `davidhariri/mixyt`

## 1) Executive Summary

- The runtime is mostly Unix-portable Rust code; the project is currently *packaging and messaging macOS-only*, not truly code-only macOS.
- Hard macOS gating is in docs/install/release/CI, not in core playback/database/TUI logic.
- There is a partially implemented macOS-native media key module (`src/daemon/mediakeys/macos.rs`) that is currently placeholder-only and not wired into runtime flow.
- Linux support is plausible with targeted distribution and CI changes, plus optional media integration hardening.

## 2) Analysis Method (Multi-pass "agents")

This context was built using parallel passes:

1. Platform pass: OS-specific logic, build targets, installer/release paths.
2. Runtime pass: daemon/audio/ipc architecture and playback behavior.
3. Data pass: config, DB schema, models, export/import semantics.
4. Product pass: CLI/TUI behavior vs `SPEC.md`.
5. Delivery pass: CI/release/testing posture and operational risks.

## 3) What Is Actually macOS Native Today

### 3.1 macOS-native in product surface and delivery

1. README says "macOS only" (`README.md:5`).
2. Installer exits on non-Darwin (`install.sh:16` to `install.sh:21`).
3. Installer fetches only mac artifacts (`install.sh:10` to `install.sh:13`, `install.sh:32`).
4. Release workflow builds only Darwin targets (`.github/workflows/release.yml:16` to `.github/workflows/release.yml:21`).
5. CI runs only on `macos-latest` (`.github/workflows/ci.yml:14`, `.github/workflows/ci.yml:28`, `.github/workflows/ci.yml:41`).

### 3.2 macOS-native in code

1. `src/daemon/mediakeys/macos.rs` is a placeholder for `MPRemoteCommandCenter` / `MPNowPlayingInfoCenter` integration and currently loops forever (`src/daemon/mediakeys/macos.rs:6` to `src/daemon/mediakeys/macos.rs:23`).
2. Core daemon media keys currently use `souvlaki` abstraction (`src/daemon/mod.rs:186` onward), not the native mac module above.

### 3.3 What is *not* macOS-only in core runtime

1. IPC is Unix socket based via `interprocess` (`src/ipc/mod.rs:65` onward), workable on Linux.
2. Playback uses `rodio` (`src/audio/mod.rs:19` onward), cross-platform backend abstraction.
3. Storage/db/config logic is OS-agnostic (`src/config/mod.rs`, `src/db/mod.rs`, `src/models/mod.rs`).
4. TUI is crossterm/ratatui based (`src/tui/mod.rs`), portable terminal stack.

## 4) What To Change It To (Linux-capable replacements)

### 4.1 Immediate, low-risk changes

1. CI matrix: add Ubuntu jobs alongside macOS in `.github/workflows/ci.yml`.
2. Release matrix: add Linux target(s) (at least `x86_64-unknown-linux-gnu`; consider musl).
3. Installer: split by OS and artifact names; do not hard-exit for Linux.
4. README: replace "macOS only" with "macOS + Linux (status by feature)" and explicit dependency install instructions for both.

### 4.2 Medium changes for Linux quality

1. Media key strategy:
   - Keep souvlaki baseline.
   - Validate Linux MPRIS naming/behavior (dbus name currently `mixyt`, `src/daemon/mod.rs:199`).
2. Daemon lifecycle:
   - Add systemd user service template for Linux.
   - Keep detached spawn fallback for non-systemd setups.
3. Packaging:
   - Add tarball + checksums for Linux binaries.
   - Optional: deb/rpm/homebrew tap/aarch64 Linux artifact.

### 4.3 Longer-term quality improvements

1. Optional native adapters:
   - macOS: real MediaPlayer framework bridge.
   - Linux: explicit MPRIS conformance tests.
2. Gate media integrations behind Cargo features if needed (`media-keys-macos`, `media-keys-linux`).

## 5) What Is Better on macOS Than Linux (keep mac-native paths)

1. Now Playing integration quality:
   - macOS native MediaPlayer APIs can provide tighter lock-screen/control-center integration than generic abstraction.
2. Audio backend consistency:
   - CoreAudio is typically less fragmented than Linux desktop audio stacks.
3. Daemon process management UX:
   - `launchd` integration can feel more native than ad-hoc detached processes on macOS.

Recommendation: keep a mac-native integration path for these, but add Linux-first equivalents rather than forcing one abstraction for everything.

## 6) What Linux Is Better For (or needs Linux-native handling)

1. Service management:
   - systemd user units are standard and robust for background daemons.
2. Desktop media interoperability:
   - MPRIS over D-Bus is standard across major Linux desktops.
3. Distribution channels:
   - Distros and containerized formats can simplify install/upgrade compared to single-script curl installers.

## 7) Current Architecture Context (for fast re-entry)

## 7.1 Module map

- `src/main.rs` (98 lines): command routing and default-to-TUI behavior.
- `src/cli/mod.rs` (108): clap command definitions.
- `src/cli/commands.rs` (501): command implementations.
- `src/daemon/mod.rs` (571): daemon server, audio thread, monitor thread, media controls.
- `src/tui/mod.rs` (657): full interactive terminal UI and background add-download flow.
- `src/audio/mod.rs` (136): `rodio` wrapper.
- `src/ipc/mod.rs` (172): daemon client protocol over local socket.
- `src/download/mod.rs` (230): yt-dlp integration, metadata/progress parsing.
- `src/db/mod.rs` (389): SQLite schema + CRUD.
- `src/config/mod.rs` (148): config defaults and pathing.
- `src/models/mod.rs` (142): domain models and serialization.

Total Rust LOC: ~3152.

## 7.2 Runtime flow

1. Start app:
   - Parse CLI and build `App` (`src/main.rs`, `src/cli/commands.rs`).
2. No command:
   - defaults to TUI (`src/main.rs:22`).
3. TUI path:
   - ensures daemon is running then starts terminal loop (`src/main.rs:88` onward).
4. Daemon:
   - creates Unix socket, manages shared `PlaybackState`, handles JSON IPC commands (`src/daemon/mod.rs`).
5. Audio:
   - isolated on dedicated thread using `AudioPlayer`.

## 7.3 Data/storage

- Storage root default: `~/.mixyt` (`src/config/mod.rs:27`).
- DB path: `~/.mixyt/mixyt.db` (`src/config/mod.rs:121`).
- Socket path: `~/.mixyt/mixyt.sock` (`src/config/mod.rs:125`).
- PID path: `~/.mixyt/mixyt.pid` (`src/config/mod.rs:129`).
- Audio files: `~/.mixyt/audio` (`src/config/mod.rs:117`).

## 8) Implementation vs SPEC Gaps

`SPEC.md` lists wider command surface than implemented CLI.

Notable gaps:

1. `next`, `prev`, queue, shuffle, repeat commands are in spec (`SPEC.md:78` to `SPEC.md:92`) but not in CLI enum (`src/cli/mod.rs:16` onward).
2. Playlist command surface is spec'd but not exposed in CLI despite DB support existing.
3. TUI is implemented and launched by default, but there is no `-i` short flag despite spec note.

## 9) Risks and Constraints

1. Toolchain mismatch:
   - `edition = "2024"` (`Cargo.toml:4`) failed with local Cargo 1.84.0 in this environment.
   - Build/test could not run here until newer toolchain is installed.
2. Linux dependency surface:
   - `souvlaki` on Linux can require D-Bus development/runtime libs.
3. Install UX mismatch:
   - docs/install currently hardcode Homebrew and mac-only flow.
4. Metadata/doc drift:
   - `SPEC.md` says Linux+macOS target; README and workflows enforce mac-only.

## 10) Prioritized Migration Plan (if goal is true macOS+Linux)

### Phase 1 (1 day, low risk)

1. Update README platform messaging and dependency instructions.
2. Add Ubuntu CI jobs.
3. Add Linux release artifact(s).
4. Split installer by OS.

### Phase 2 (2-4 days, medium risk)

1. Add `next/prev/queue/shuffle/repeat` CLI plumbing to existing daemon protocol.
2. Expose playlist commands already backed by DB methods.
3. Add Linux setup docs for D-Bus/media keys and audio backend expectations.

### Phase 3 (optional, higher complexity)

1. Implement native macOS MediaPlayer adapter.
2. Add systemd user service integration for daemon management on Linux.
3. Add end-to-end smoke tests per OS in CI.

## 11) Fast Re-entry Checklist (next session)

1. Read this file first.
2. Confirm target objective:
   - "keep mac-first" vs "ship Linux parity".
3. If Linux parity:
   - start with Phase 1 (CI/release/install/docs) before deeper runtime changes.
4. If mac polish:
   - prioritize native MediaPlayer integration and launchd service.
5. Re-validate with real toolchain:
   - upgrade cargo/rustc, then run `cargo test`, `cargo clippy`, and smoke run TUI/daemon commands.

