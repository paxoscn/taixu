#!/usr/bin/env bash
set -euo pipefail

skill_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
root="$(git -C "$skill_dir" rev-parse --show-toplevel)"
src="$root/docs"
dst="$skill_dir/references"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
cp "$src"/*.md "$tmp"/
cp "$root/CHANGELOG.md" "$tmp/CHANGELOG.md"

if [[ "${1:-}" == "--check" ]]; then
    diff -ru "$dst" "$tmp"
else
    mkdir -p "$dst"
    cp "$src"/*.md "$dst"/
    cp "$root/CHANGELOG.md" "$dst/CHANGELOG.md"
    echo "Synced docs -> $dst"
fi
