#!/bin/sh
set -e

REPO="${CLISTREAM_REPO:-evelynadriaan/ruststream-cli}"
INSTALL_DIR="${CLISTREAM_INSTALL_DIR:-/usr/local/bin}"

OS="$(uname -s)"
if [ "$OS" != "Linux" ]; then
    echo "This installer currently supports Linux only."
    exit 1
fi

ARCH="$(uname -m)"
case "$ARCH" in
    x86_64|amd64) ARTIFACT="clistream-linux-x86_64" ;;
    *)
        echo "Unsupported architecture: $ARCH"
        echo "Supported architectures: x86_64"
        exit 1
        ;;
esac

echo "Installing clistream for Linux/$ARCH..."

if ! command -v ffmpeg >/dev/null 2>&1; then
    echo "Missing dependency: ffmpeg"
    exit 1
fi

if ! command -v yt-dlp >/dev/null 2>&1; then
    echo "Missing dependency: yt-dlp"
    exit 1
fi

if ! command -v mpv >/dev/null 2>&1; then
    echo "Warning: mpv is not installed. Streaming commands (stream/save) will be unavailable."
fi

if command -v clistream >/dev/null 2>&1; then
    echo "Stopping existing daemon..."
    clistream daemon stop 2>/dev/null || true
fi

DOWNLOAD_URL="https://github.com/$REPO/releases/latest/download/$ARTIFACT"
CHECKSUM_URL="${DOWNLOAD_URL}.sha256"
TMP_FILE="$(mktemp)"
TMP_SHA="$(mktemp)"
trap 'rm -f "$TMP_FILE" "$TMP_SHA"' EXIT

curl -fsSL "$DOWNLOAD_URL" -o "$TMP_FILE"
curl -fsSL "$CHECKSUM_URL" -o "$TMP_SHA"

(
    cd "$(dirname "$TMP_FILE")"
    expected_file="$(basename "$TMP_FILE")"
    checksum_line="$(cat "$TMP_SHA")"
    checksum_value="$(echo "$checksum_line" | awk '{print $1}')"
    echo "${checksum_value}  ${expected_file}" | sha256sum -c -
)

chmod +x "$TMP_FILE"

if [ -w "$INSTALL_DIR" ]; then
    mv "$TMP_FILE" "$INSTALL_DIR/clistream"
else
    echo "Need sudo to install to $INSTALL_DIR"
    sudo mv "$TMP_FILE" "$INSTALL_DIR/clistream"
fi

echo "clistream installed to $INSTALL_DIR/clistream"
echo ""
echo "Get started:"
echo "  clistream add <youtube-url>"
echo "  clistream play <search>"
