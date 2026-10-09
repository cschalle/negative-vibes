#!/usr/bin/env bash
# Usage: cargo-build.sh OUTPUT BUILT_BINARY CARGO [cargo args...]
# Runs cargo build and copies the resulting binary to OUTPUT.
set -euo pipefail

OUTPUT="$1"; shift
BUILT_BINARY="$1"; shift
CARGO="$1"; shift

"$CARGO" build "$@"
cp -a "$BUILT_BINARY" "$OUTPUT"
