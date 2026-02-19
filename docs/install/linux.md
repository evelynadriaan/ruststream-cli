# Linux Install

## Binary install (recommended)

1. Install runtime dependencies:

```bash
sudo apt-get update
sudo apt-get install -y ffmpeg yt-dlp
```

2. Install latest release binary:

```bash
curl -fsSL https://raw.githubusercontent.com/SoItGoesIO/mixyt/main/install.sh | sh
```

Installer behavior:

1. Linux-only guard.
2. Verifies `ffmpeg` and `yt-dlp` are available.
3. Downloads `clistream-linux-x86_64` and matching `.sha256` file.
4. Verifies checksum before moving binary to `/usr/local/bin/clistream`.

## Build from source

Install build dependencies first:

```bash
sudo apt-get update
sudo apt-get install -y pkg-config libasound2-dev libdbus-1-dev ffmpeg yt-dlp
```

Then build:

```bash
cargo build --release
```

Run:

```bash
./target/release/clistream help
```

