#!/usr/bin/env bash
# Cuts a release of qbrs's four publishable crates (qbrs-core, qbrs-macros,
# qbrs, qbrs-sqlx): one version across all four, one tag, pushed. See
# release.toml at the workspace root for the shared config.
#
# It does not publish. `.github/workflows/release.yaml` reacts to the tag and
# does that, authenticating to crates.io by OIDC, so no crates.io credential
# has to exist on a laptop.
#
# Prerequisite: on `main`, clean working tree, up to date with origin/main.
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

echo
echo "==> cargo-release dry run for '$LEVEL' (nothing is touched yet):"
cargo release "$LEVEL"

echo
read -r -p "Proceed with the release plan above (bump, tag, push)? [y/N] " reply
case "$reply" in
    y|Y|yes|YES) ;;
    *)
        echo "Aborted. Nothing was changed."
        exit 1
        ;;
esac

cargo release "$LEVEL" --execute

echo
echo "==> Tag pushed. crates.io publishing runs from .github/workflows/release.yaml:"
echo "    https://github.com/RyutaroYako/qbrs/actions/workflows/release.yaml"
