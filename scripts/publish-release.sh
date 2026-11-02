#!/usr/bin/env bash
# Called only by the tag workflow after all release builds have succeeded.
set -euo pipefail

asset_dir="$(cd "${1:?release asset directory required}" && pwd)"
[[ "$RELEASE_TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "Invalid release tag" >&2; exit 1; }
[[ "$RELEASE_COMMIT" =~ ^[0-9a-f]{40}$ ]] || { echo "Invalid release commit" >&2; exit 1; }

# Validate the complete local set before making any release API writes.
python3 - "$asset_dir" <<'PY'
import hashlib
from pathlib import Path
import sys

directory = Path(sys.argv[1])
binaries = {
    "acp-traces-linux-amd64",
    "acp-traces-linux-arm64",
    "acp-traces-macos-amd64",
    "acp-traces-macos-arm64",
    "acp-traces-windows-amd64.exe",
    "acp-traces-windows-arm64.exe",
}
if {path.name for path in directory.iterdir()} != binaries | {"SHA256SUMS"}:
    raise SystemExit("Release requires exactly six binaries and SHA256SUMS")
for path in directory.iterdir():
    if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
        raise SystemExit("Release assets must be nonempty regular files")
expected = {
    f"{hashlib.sha256((directory / name).read_bytes()).hexdigest()}  {name}"
    for name in binaries
}
lines = (directory / "SHA256SUMS").read_text().splitlines()
if len(lines) != len(binaries) or set(lines) != expected:
    raise SystemExit("Release checksum manifest does not match the binaries")
PY

# Include drafts in the authenticated check. Any failed page stops publication.
existing="$(gh api --paginate "repos/$GH_REPO/releases" \
  --jq ".[] | select(.tag_name == \"$RELEASE_TAG\") | .id")"
[[ -z "$existing" ]] || { echo "Release already exists" >&2; exit 1; }
current="$(gh api "repos/$GH_REPO/commits/$RELEASE_TAG" --jq .sha)"
[[ "$current" == "$RELEASE_COMMIT" ]] || { echo "Release tag moved" >&2; exit 1; }

release_id="$(gh api --method POST "repos/$GH_REPO/releases" \
  -f tag_name="$RELEASE_TAG" -f name="$RELEASE_TAG" \
  -f body="$(cat CHANGELOG.md)" -F draft=true -F prerelease=false --jq .id)"
[[ "$release_id" =~ ^[0-9]+$ ]] || { echo "Invalid release ID" >&2; exit 1; }
for asset in "$asset_dir"/*; do
  gh api --method POST \
    "https://uploads.github.com/repos/$GH_REPO/releases/$release_id/assets?name=${asset##*/}" \
    --header 'Content-Type: application/octet-stream' \
    --input "$asset" > /dev/null
done

current="$(gh api "repos/$GH_REPO/commits/$RELEASE_TAG" --jq .sha)"
[[ "$current" == "$RELEASE_COMMIT" ]] || { echo "Release tag moved" >&2; exit 1; }
# Use the created draft's ID; never resolve a draft by its potentially reused tag.
# Upload failures leave this draft unpublished for a maintainer to inspect.
gh api --method PATCH "repos/$GH_REPO/releases/$release_id" \
  -F draft=false -f make_latest=true > /dev/null
