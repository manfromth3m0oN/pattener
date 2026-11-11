#!/usr/bin/env sh
# Build the WebAssembly module and place it next to index.html.
set -e
cd "$(dirname "$0")/.."
cargo build --release -p latexpat-wasm --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/latexpat_wasm.wasm web/latexpat.wasm
echo "built web/latexpat.wasm; serve the folder, e.g.: python3 -m http.server -d web 8000"
