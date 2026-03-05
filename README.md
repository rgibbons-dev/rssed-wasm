# rssed-wasm

A browser-based terminal interface for [rssed](https://github.com/rgibbons-dev/rssed), an ed(1)-style RSS/Atom feed reader. Compiled from Rust to WebAssembly.

## Prerequisites

- [Rust](https://rustup.rs/) (with the `wasm32-unknown-unknown` target)
- [wasm-bindgen-cli](https://crates.io/crates/wasm-bindgen-cli)

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli
```

## Building

```sh
./build.sh
```

This runs two steps:

1. `cargo build --target wasm32-unknown-unknown --release` — compiles the Rust library to a `.wasm` binary
2. `wasm-bindgen --target web --out-dir web/pkg ...` — generates the JS glue code and typed wrapper that the browser loads

Output lands in `web/pkg/`. After building, the `web/` directory is entirely self-contained and can be deployed to any static file host as-is.

## Running locally

After building:

```sh
cd web
python3 -m http.server 8080
```

Open `http://localhost:8080`.

## Usage

Type `h` in the terminal for the full command reference. Quick start:

```
a <url>    add a feed
,p         list all feeds
0           select first feed
o           view current item
+/-         next/prev item
w           save session (localStorage)
e           load saved session
```

Sessions are persisted to browser `localStorage`.

## CORS proxy

Most RSS feeds don't serve CORS headers, so the browser's `fetch()` would reject them. By default, feed URLs are routed through [corsproxy.io](https://corsproxy.io), a free third-party proxy. Be aware:

- **Availability** — it's a free service; it may go down, rate-limit, or disappear
- **Privacy** — every URL you fetch is visible to the proxy operator
- **Security** — not suitable for private or authenticated feeds

To use your own proxy (e.g. [cors-anywhere](https://github.com/Rob--W/cors-anywhere)), change `CORS_PROXY` at the top of `web/main.js`. Set it to `""` to disable proxying entirely (useful if your feeds already allow cross-origin requests).
