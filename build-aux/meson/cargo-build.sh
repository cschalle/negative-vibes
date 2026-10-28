#!/usr/bin/env bash
# Usage: cargo-build.sh OUTPUT BUILT_BINARY CARGO [cargo args...]
# Runs cargo build and copies the resulting binary to OUTPUT.
set -euo pipefail

OUTPUT="$1"; shift
BUILT_BINARY="$1"; shift
CARGO="$1"; shift

# A local, git-ignored file may give the Google OAuth client this build signs in with.
CLIENT_ENV="$(dirname "${BASH_SOURCE[0]}")/../google-client.env"
if [ -f "$CLIENT_ENV" ]; then
    set -a; . "$CLIENT_ENV"; set +a
fi

"$CARGO" build "$@"
cp -a "$BUILT_BINARY" "$OUTPUT"
