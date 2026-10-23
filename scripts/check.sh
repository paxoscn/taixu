#!/usr/bin/env bash
set -euo pipefail

root="$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
cd "$root"

echo "==> bundled references are in sync with docs/"
"$root/scripts/sync-skill-docs.sh" --check

echo "==> cargo fmt --check"
cargo fmt --check

echo "==> cargo clippy --all-targets -- -D warnings"
cargo clippy --all-targets -- -D warnings

echo "==> cargo test --all-targets"
cargo test --all-targets

echo "==> cargo test --doc"
cargo test --doc

echo "==> cargo doc (deny warnings)"
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps

echo "==> examples"
for example in basic tasks trade_rollback npc_actions persistence region_matrix channels; do
    echo "--- $example"
    cargo run -q --example "$example"
done

echo "All checks passed."
