#!/usr/bin/env bash
# Development launcher for Negative Vibes.
# Uses the system SANE setup: the plustek8200i backend is installed in
# /usr/lib64/sane and listed in /etc/sane.d/dll.d. Set SANE_CONFIG_DIR or
# LD_LIBRARY_PATH yourself to test another backend build.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

exec env \
  GSETTINGS_SCHEMA_DIR="$SCRIPT_DIR/_build/schemas" \
  APP_ID="io.github.cschalle.NegativeVibes.Devel" \
  PKGDATADIR="$SCRIPT_DIR/data" \
  cargo run --manifest-path "$SCRIPT_DIR/Cargo.toml" --bin negative-vibes "$@"
