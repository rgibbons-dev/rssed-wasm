#!/bin/sh
set -e

echo "Building rssed-wasm..."

# 1. Compile Rust to WASM
cargo build --target wasm32-unknown-unknown --release

# 2. Generate JS bindings
wasm-bindgen --target web --out-dir pkg \
  target/wasm32-unknown-unknown/release/rssed_wasm.wasm

echo ""
echo "Build complete. To serve locally:"
echo "  cd web && python3 -m http.server 8080"
echo ""
echo "Then open http://localhost:8080"
