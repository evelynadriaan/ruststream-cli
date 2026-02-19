# RustStream CLI

Offline-first terminal audio library for YouTube (Rust + daemon + TUI).

## Install

```bash
sudo apt-get update
sudo apt-get install -y ffmpeg yt-dlp
curl -fsSL https://raw.githubusercontent.com/evelynadriaan/ruststream-cli/main/install.sh | sh
```

Detailed Linux install guide: `docs/install/linux.md`

## Usage

```bash
# Add a track
clistream add "https://youtube.com/watch?v=..."

# Open the player
clistream
```

**Keyboard shortcuts:**
- `↑↓` Navigate
- `←→` Seek
- `Space` Play/pause
- `/` Search
- `a` Add track
- `e` Rename track
- `q` Quit

For all CLI commands: `clistream help`
