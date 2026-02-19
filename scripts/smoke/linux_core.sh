#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${CLISTREAM_TEST_URL:-}" ]]; then
  echo "Set CLISTREAM_TEST_URL to a YouTube URL before running this smoke test."
  exit 1
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

export XDG_DATA_HOME="$tmp_dir/data"
export XDG_CONFIG_HOME="$tmp_dir/config"
export XDG_RUNTIME_DIR="$tmp_dir/run"
mkdir -p "$XDG_DATA_HOME" "$XDG_CONFIG_HOME" "$XDG_RUNTIME_DIR"

echo "[smoke] starting daemon"
cargo run -- daemon start

echo "[smoke] adding track"
cargo run -- add "$CLISTREAM_TEST_URL" --alias smoke-track

echo "[smoke] starting playback"
cargo run -- play smoke-track

echo "[smoke] checking status"
cargo run -- status

echo "[smoke] streaming fixed URL via mpv"
cargo run -- stream "https://www.youtube.com/watch?v=jNQXAC9IVRw"
sleep 5

echo "[smoke] stopping daemon"
cargo run -- daemon stop

echo "[smoke] done"
