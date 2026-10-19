#!/usr/bin/env bash
# rustex1 dev server — Rust backend, Rust→wasm frontend, m-js hot reload.
#
#   ./dev.sh              build if needed, serve on :3001
#   ./dev.sh 4000         serve on another port
#   ./dev.sh --rebuild    force a fresh wasm build on start
#
set -euo pipefail
cd "$(dirname "$0")"

PORT=""           # empty → whatever rustex1.json says
EXTRA=""
for arg in "$@"; do
  case "$arg" in
    --*) EXTRA="$EXTRA $arg" ;;
    *) PORT="$arg" ;;
  esac
done

# The wasm-bindgen CLI is pinned to match the crate version and lives in tools/.
if ! ls tools/*/wasm-bindgen >/dev/null 2>&1; then
  echo "rustex1: installing wasm-bindgen CLI into tools/"
  mkdir -p tools
  V="$(cargo tree -p rustex1-kit -i wasm-bindgen 2>/dev/null | head -1 | grep -o '[0-9]*\.[0-9]*\.[0-9]*' | head -1)"
  V="${V:-0.2.129}"
  ARCH="$(uname -m)-apple-darwin"
  curl -sL "https://github.com/rustwasm/wasm-bindgen/releases/download/${V}/wasm-bindgen-${V}-${ARCH}.tar.gz" \
    | tar xz -C tools/
  echo "rustex1: got wasm-bindgen ${V} (${ARCH})"
fi

if ! rustup target list --installed | grep -q wasm32-unknown-unknown; then
  echo "rustex1: adding the wasm32-unknown-unknown target"
  rustup target add wasm32-unknown-unknown
fi

cargo build -q -p rustex1-server
ARGS=""
[ -n "$PORT" ] && ARGS="--port $PORT"
# shellcheck disable=SC2086
exec ./target/debug/rustex1-server $ARGS $EXTRA
