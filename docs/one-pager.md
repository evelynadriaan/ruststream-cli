# RustStream CLI — One Pager

## Problem

YouTube is the world's largest music library but accessing it requires a browser, an active connection, and tolerance for ads, autoplay, and tab sprawl. Streaming services own your queue. There is no good terminal-native option that lets you build a personal, portable, offline audio library from YouTube.

## User

Developers and power users who live in the terminal, use tiling window managers or remote servers, and want their music workflow to match their development workflow. Someone who would rather press `j/k` than click.

## Solution

A Rust CLI/TUI tool that downloads YouTube audio to a local library, manages it with a keyboard-driven interface, and plays it through a background daemon. No browser. No account. No internet required after download.

## Differentiation

| | clistream | mpv + yt-dlp scripts | spotify-player | cmus |
|---|---|---|---|---|
| YouTube native | ✅ | manual | ❌ | ❌ |
| Offline library | ✅ | ❌ | ❌ | ✅ |
| TUI player | ✅ | ❌ | ✅ | ✅ |
| Streaming mode | ✅ | ✅ | ✅ | ❌ |
| Daemon + IPC | ✅ | ❌ | ✅ | ✅ |
| Lyrics | ✅ | ❌ | ✅ | ❌ |

## Status

**v1.1.0 — stable.** Core library management, TUI player, streaming, lyrics, and listen history are all shipped and tested. CI green on Linux x86_64. Prebuilt binary available.

## Key metrics (qualitative)

- One command to install
- One command to add a track
- Zero config required to get started
- All data stays local
