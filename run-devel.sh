#!/usr/bin/env bash
# Development launcher for Negative Vibes.
# Sets up SANE to use the custom plustek8200i backend from ~/projects/scanner.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKEND_DIR="$HOME/projects/scanner/backends/backend"

exec env \
  GSETTINGS_SCHEMA_DIR="$SCRIPT_DIR/_build/schemas" \
  APP_ID="io.github.cschalle.NegativeVibes.Devel" \
  PKGDATADIR="$SCRIPT_DIR/data" \
  SANE_CONFIG_DIR="$HOME/.sane" \
  LD_LIBRARY_PATH="${BACKEND_DIR}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
  cargo run --manifest-path "$SCRIPT_DIR/Cargo.toml" --bin negative-vibes "$@"
