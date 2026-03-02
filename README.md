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
2. `wasm-bindgen --target web --out-dir pkg ...` — generates the JS glue code and typed wrapper that the browser loads

Output lands in `pkg/`.

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

Sessions are persisted to browser `localStorage`. Feed URLs are fetched through a CORS proxy.
