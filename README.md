 # clistream

A CLI tool for saving and playing YouTube audio from the terminal on Linux.

![clistream TUI](screenshot.png)

## Install

```bash
sudo apt-get update
sudo apt-get install -y ffmpeg yt-dlp
curl -fsSL https://raw.githubusercontent.com/SoItGoesIO/mixyt/main/install.sh | sh
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
