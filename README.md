# rssed-wasm

A browser-based terminal interface for [rssed](https://github.com/rgibbons-dev/rssed), an ed(1)-style RSS/Atom feed reader. Compiled from Rust to WebAssembly.

## Building

Requires [Rust](https://rustup.rs/) and [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/):

```sh
./build.sh
```

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
