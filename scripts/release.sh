#!/usr/bin/env bash
# Releases qbrs's four publishable crates (qbrs-core, qbrs-macros, qbrs,
# qbrs-sqlx) together, in dependency order, via cargo-release. See
# release.toml at the workspace root for the shared config: one version
# across all four crates, one tag, crates.io publishing included.
#
# Prerequisites:
#   - `cargo login` already run (or CARGO_REGISTRY_TOKEN set) with publish
#     rights on all four crates.
#   - On `main`, clean working tree, up to date with origin/main.
#
# Usage:
#   ./scripts/release.sh <patch|minor|major|<exact-version>>
set -euo pipefail

usage() {
    echo "Usage: $0 <patch|minor|major|<exact-version>>" >&2
    exit 1
}

[ $# -eq 1 ] || usage
LEVEL="$1"

cd "$(git rev-parse --show-toplevel)"

if ! command -v cargo-release >/dev/null 2>&1; then
    echo "==> cargo-release not found; installing with 'cargo install cargo-release --locked'"
    cargo install cargo-release --locked
fi

if [ -z "${CARGO_REGISTRY_TOKEN:-}" ] && ! grep -q '^\[registries\.\|^\[registry\]' "${CARGO_HOME:-$HOME/.cargo}/credentials.toml" 2>/dev/null; then
    echo "error: no crates.io credentials found. Run 'cargo login' first, or set CARGO_REGISTRY_TOKEN." >&2
    exit 1
fi

BRANCH="$(git branch --show-current)"
if [ "$BRANCH" != "main" ]; then
    echo "error: release from 'main', not '$BRANCH'." >&2
    exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
    echo "error: working tree isn't clean. Commit or stash first." >&2
    exit 1
fi

echo "==> Checking main is up to date with origin/main"
git fetch origin main --quiet
if [ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]; then
    echo "error: local main has diverged from origin/main. Pull or push first." >&2
    exit 1
fi

echo "==> Running the same checks CI gates on"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked --all-features

# `--no-verify`, and only here: a dry run leaves the version numbers alone,
# so it packages each crate at the version already on crates.io, and a
# dependent's `qbrs-core = "<current>"` then resolves to that published copy
# instead of the sibling being packaged beside it, failing on every API added
# since. The real run below bumps first, so the new version exists nowhere but
# locally and the four verify against each other.
echo
echo "==> cargo-release dry run for '$LEVEL' (nothing is touched yet):"
cargo release "$LEVEL" --no-verify

echo
read -r -p "Proceed with the release plan above (bump, tag, push, publish to crates.io)? [y/N] " reply
case "$reply" in
    y|Y|yes|YES) ;;
    *)
        echo "Aborted. Nothing was changed."
        exit 1
        ;;
esac

cargo release "$LEVEL" --execute
