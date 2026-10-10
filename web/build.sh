#!/usr/bin/env sh
# Builds the browser version into dist/.
# Needs: rustup target add wasm32-unknown-unknown
#        cargo install wasm-bindgen-cli --version <wasm-bindgen version from Cargo.lock>
# Optional: wasm-opt (binaryen) for a smaller module.
set -eu
cd "$(dirname "$0")/.."

profile="${PROFILE:-web-release}"
# Cargo puts the `dev` profile in `debug/`.
out_dir="$profile"
if [ "$profile" = dev ]; then out_dir=debug; fi

cargo build --profile "$profile" --target wasm32-unknown-unknown
rm -rf dist
wasm-bindgen --target web --no-typescript --out-dir dist --out-name shadertoy \
  "target/wasm32-unknown-unknown/$out_dir/shadertoy.wasm"

if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -Os --enable-bulk-memory --enable-nontrapping-float-to-int \
    -o dist/shadertoy_bg.wasm dist/shadertoy_bg.wasm ||
    echo "wasm-opt failed; keeping the unoptimized module"
fi

cp web/index.html dist/
ls -la dist
