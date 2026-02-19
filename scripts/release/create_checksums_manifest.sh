#!/usr/bin/env bash
set -euo pipefail

dist_dir="${1:-dist}"
manifest="${2:-$dist_dir/SHA256SUMS}"

if [[ ! -d "$dist_dir" ]]; then
  echo "dist directory not found: $dist_dir" >&2
  exit 1
fi

tmp_manifest="$(mktemp)"
trap 'rm -f "$tmp_manifest"' EXIT

find "$dist_dir" -maxdepth 1 -type f ! -name '*.sha256' ! -name 'SHA256SUMS' -print0 \
  | sort -z \
  | while IFS= read -r -d '' file; do
      sha256sum "$file"
    done > "$tmp_manifest"

mv "$tmp_manifest" "$manifest"
echo "Wrote checksum manifest: $manifest"

