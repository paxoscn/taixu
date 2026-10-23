#!/usr/bin/env bash
set -euo pipefail

root="$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
skill_dir="$root/skills/taixu"
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
