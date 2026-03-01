#!/bin/sh
set -e

echo "Building rssed-wasm..."
wasm-pack build --target web --out-dir pkg --release

echo ""
echo "Build complete. To serve locally:"
echo "  cd web && python3 -m http.server 8080"
echo ""
echo "Then open http://localhost:8080"
