# rssed-wasm: A Code Walkthrough

This is a linear walkthrough of rssed-wasm — the browser port of [rssed](https://github.com/rgibbons-dev/rssed). The project has four source files: a Rust library (`src/lib.rs`, 507 lines), an HTML shell (`web/index.html`, 59 lines), a CSS stylesheet (`web/style.css`, 396 lines), and a JavaScript terminal emulator (`web/main.js`, 194 lines). We'll follow each file top-to-bottom, covering how the CLI was adapted for WebAssembly and how the browser terminal works.

## Build pipeline

The build does not use wasm-pack (which is sunset). It uses `cargo` and `wasm-bindgen` directly:

```bash
cat build.sh
```

```output
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
```

Step 1 compiles the Rust library crate to a `.wasm` binary using Cargo's built-in WASM target. Step 2 runs `wasm-bindgen`, which reads the `#[wasm_bindgen]` annotations in the compiled binary and generates the JavaScript glue code — the `pkg/rssed_wasm.js` module that the browser imports. The `--target web` flag produces ES module output (no bundler required).

## Dependencies

```bash
sed -n '9,21p' Cargo.toml
```

```output
[dependencies]
wasm-bindgen = "0.2"
wasm-bindgen-futures = "0.4"
js-sys = "0.3"
web-sys = { version = "0.3", features = ["Window", "Storage"] }
feed-rs = "2"
html2text = "0.12.5"
gloo-net = { version = "0.6", features = ["http"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
getrandom = { version = "0.2", features = ["js"] }
uuid = { version = "1", features = ["js"] }
```

The first four crates are the WASM interop layer: **wasm-bindgen** exposes Rust structs/functions to JavaScript, **wasm-bindgen-futures** bridges Rust futures to JS promises, **js-sys** provides bindings to core JS types (`Promise`), and **web-sys** gives access to browser APIs (here just `Window` and `Storage` for localStorage).

**feed-rs** and **html2text** are carried over unchanged from the CLI version — they're pure Rust and work in WASM without modification.

**gloo-net** replaces `reqwest`. reqwest pulls in tokio, which doesn't compile to `wasm32-unknown-unknown`. gloo-net uses the browser's `fetch()` API under the hood, so HTTP requests work natively in the browser.

**serde** and **serde_json** handle session serialization. The CLI version wrote plain-text URL lists to `~/.rssed`; the WASM version serializes to JSON and stores in localStorage.

**getrandom** and **uuid** need the `js` feature flag on `wasm32-unknown-unknown`. Without it, uuid (a transitive dependency of feed-rs) fails to compile because it can't find a randomness source. The `js` feature tells it to use `crypto.getRandomValues()`.

The crate type tells Cargo to produce a C-compatible dynamic library — which for the `wasm32-unknown-unknown` target means a `.wasm` binary:

```bash
sed -n '6,7p' Cargo.toml
```

```output
[lib]
crate-type = ["cdylib", "rlib"]
```

`cdylib` is for the WASM output. `rlib` is kept so the crate can also be used as a Rust library dependency if needed.

## The Rust library: src/lib.rs

### Imports

```bash
sed -n '1,7p' src/lib.rs
```

```output
use wasm_bindgen::prelude::*;
use feed_rs::parser;
use feed_rs::model::{Entry, Feed};
use html2text::from_read;
use js_sys::Promise;
use serde::{Deserialize, Serialize};
use web_sys::window;
```

Compared to the CLI's imports, `std::io`, `std::env`, `std::fs`, and `std::error::Error` are gone — there's no stdin, no environment variables, no filesystem in the browser. In their place: `wasm_bindgen::prelude` for the `#[wasm_bindgen]` macro, `js_sys::Promise` for returning async results to JavaScript, `serde` for session serialization, and `web_sys::window` for accessing `localStorage`.

The feed-rs and html2text imports are identical to the CLI version. That's the whole point — the feed parsing layer doesn't know or care that it's running in a browser.

### Data types

```bash
sed -n '9,26p' src/lib.rs
```

```output
// --- Types ---

#[derive(Clone)]
struct StoredFeed {
    url: String,
    feed: Feed,
}

#[derive(Copy, Clone)]
enum Addr {
    One(usize),
    Range(usize, usize),
}

#[derive(Serialize, Deserialize)]
struct SessionData {
    urls: Vec<String>,
}
```

`StoredFeed` and `Addr` are unchanged from the CLI. They're internal types not exposed to JavaScript, so they don't need `#[wasm_bindgen]`.

`SessionData` is new. The CLI version wrote bare URL-per-line text files; in the browser there's no filesystem, so sessions are stored in `localStorage` as JSON. `SessionData` is the serialization envelope — just a `{ "urls": [...] }` object.

### Feed operations

```bash
sed -n '30,43p' src/lib.rs
```

```output
async fn fetch_feed(url: &str) -> Result<Feed, String> {
    let resp = gloo_net::http::Request::get(url)
        .send()
        .await
        .map_err(|e| format!("fetch error: {}", e))?;

    let bytes = resp
        .binary()
        .await
        .map_err(|e| format!("read error: {}", e))?;

    let feed = parser::parse(&bytes[..]).map_err(|e| format!("parse error: {}", e))?;
    Ok(feed)
}
```

This is the WASM equivalent of the CLI's `fetch_feed`. The structure is the same — HTTP GET, then parse — but the implementation swaps reqwest for gloo-net. `gloo_net::http::Request::get(url).send()` compiles to the browser's `fetch()` API. The `.binary()` call reads the response body as `Vec<u8>`, which feeds into `parser::parse()` exactly like the CLI's `reqwest::get().bytes()`.

The error type changed from `Box<dyn Error>` to `String`. In WASM, `Box<dyn Error>` is awkward to send across the JS boundary; `String` errors are simpler and get converted to `JsValue::from_str()` by the caller.

The remaining feed helper functions — `feed_title`, `entry_title`, `entry_link`, and `format_entry` — are byte-for-byte identical to the CLI version:

```bash
sed -n '45,75p' src/lib.rs
```

```output
fn feed_title(feed: &Feed) -> String {
    feed.title
        .as_ref()
        .map(|t| t.content.clone())
        .unwrap_or_else(|| "(untitled)".to_string())
}

fn entry_title(entry: &Entry) -> String {
    entry
        .title
        .as_ref()
        .map(|t| t.content.clone())
        .unwrap_or_else(|| "(no title)".to_string())
}

fn entry_link(entry: &Entry) -> Option<String> {
    entry.links.first().map(|l| l.href.clone())
}

fn format_entry(entry: &Entry) -> String {
    let title = entry_title(entry);
    let body = entry
        .content
        .as_ref()
        .and_then(|c| c.body.clone())
        .or_else(|| entry.summary.as_ref().map(|s| s.content.clone()))
        .unwrap_or_default()
        .replace('\n', "");
    let fmt = from_read(body.as_bytes(), 80);
    format!("{}\n===============\n{}", title, fmt.trim())
}
```

These functions are pure data transformations — no I/O, no platform dependencies. They worked in the CLI and they work in WASM without changes. html2text's `from_read()` still wraps HTML at 80 columns, producing the same terminal-style output for the browser's monospace display.

### Address parsing

```bash
sed -n '77,113p' src/lib.rs
```

```output
// --- Address parsing ---

fn parse_one(s: &str, cur: usize, last: usize) -> Option<usize> {
    match s.trim() {
        "." => Some(cur),
        "$" => Some(last),
        n => n.parse::<usize>().ok().filter(|&v| v <= last),
    }
}

fn parse_addr(s: &str, cur: usize, last: usize) -> Option<Addr> {
    match s {
        "." => return Some(Addr::One(cur)),
        "$" => return Some(Addr::One(last)),
        "," => return Some(Addr::Range(0, last)),
        ";" => return Some(Addr::Range(cur, last)),
        _ => {}
    }
    if let Some(pos) = s.find(',') {
        let a = parse_one(&s[..pos], cur, last)?;
        let b = parse_one(&s[pos + 1..], cur, last)?;
        if a <= b {
            Some(Addr::Range(a, b))
        } else {
            None
        }
    } else {
        parse_one(s, cur, last).map(Addr::One)
    }
}

fn addr_indices(addr: Addr) -> Vec<usize> {
    match addr {
        Addr::One(n) => vec![n],
        Addr::Range(a, b) => (a..=b).collect(),
    }
}
```

These three functions are unchanged from the CLI. See the [rssed walkthrough](https://github.com/rgibbons-dev/rssed/blob/main/walkthrough.md) for a detailed explanation. The address parsing is pure logic — no I/O, no platform coupling — so it ports to WASM verbatim.

### Storage helpers

This is the biggest architectural change from the CLI. The CLI used `std::fs` to read/write a plain text file at `~/.rssed`. The browser has no filesystem. Instead, we use `localStorage`:

```bash
sed -n '146,173p' src/lib.rs
```

```output
// --- Storage helpers ---

fn get_storage() -> Option<web_sys::Storage> {
    window()?.local_storage().ok()?
}

fn save_session(store: &[StoredFeed]) -> Result<usize, String> {
    let storage = get_storage().ok_or_else(|| "no storage available".to_string())?;
    let data = SessionData {
        urls: store.iter().map(|f| f.url.clone()).collect(),
    };
    let json = serde_json::to_string(&data).map_err(|e| e.to_string())?;
    let bytes = json.len();
    storage
        .set_item("rssed_session", &json)
        .map_err(|_| "storage write failed".to_string())?;
    Ok(bytes)
}

fn load_session_urls() -> Result<Vec<String>, String> {
    let storage = get_storage().ok_or_else(|| "no storage available".to_string())?;
    let json = storage
        .get_item("rssed_session")
        .map_err(|_| "storage read failed".to_string())?
        .ok_or_else(|| "no saved session".to_string())?;
    let data: SessionData = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    Ok(data.urls)
}
```

`get_storage()` chains two `?` operators: `window()` returns `Option<Window>` (it's `None` in non-browser WASM environments like Node), and `.local_storage()` returns `Result<Option<Storage>>` (it can fail if storage is disabled). The double-`?` collapses both failure modes into `None`.

`save_session` mirrors the CLI's `w` command: it collects all URLs from the store, serializes them as JSON, and writes to `localStorage` under the key `"rssed_session"`. It returns the byte count, matching the CLI's convention of printing the number of bytes written.

`load_session_urls` mirrors the CLI's `e` command: it reads the JSON from `localStorage`, deserializes it, and returns the URL list. The caller then fetches each URL, exactly like the CLI reads a file and fetches each line.

The CLI also supported `#`-comments in session files and an explicit filename argument (`e myfile`, `w myfile`). These don't translate to `localStorage`, so they're dropped. The `f` command now just prints `"(browser localStorage)"`.

### The WASM-exposed REPL struct

In the CLI, program state lived as six `let mut` variables in `main()`. In the WASM version, they're fields on a struct that JavaScript holds a reference to:

```bash
sed -n '175,197p' src/lib.rs
```

```output
// --- WASM-exposed REPL ---

#[wasm_bindgen]
pub struct Rssed {
    store: Vec<StoredFeed>,
    cur: usize,
    item: usize,
    dirty: bool,
    quit_warned: bool,
}

#[wasm_bindgen]
impl Rssed {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Rssed {
        Rssed {
            store: Vec::new(),
            cur: 0,
            item: 0,
            dirty: false,
            quit_warned: false,
        }
    }
```

`#[wasm_bindgen]` on the struct makes it available to JavaScript as a class. `#[wasm_bindgen(constructor)]` marks `new()` as the JS constructor, so JavaScript can write `new Rssed()`. The five fields map directly to the CLI's five mutable variables (the sixth, `file`, is gone since there's no filesystem path to track).

JavaScript sees `Rssed` as an opaque handle — it can't access the fields directly. All interaction goes through the `exec()` method.

### The exec method: CLI loop → single-command dispatch

The CLI had a `loop { read_line(); match ... }` structure. In WASM, there's no blocking stdin loop. Instead, JavaScript calls `exec()` once per command and `await`s the result:

```bash
sed -n '199,209p' src/lib.rs
```

```output
    /// Process a single line of input, returns a Promise<String> with the output.
    pub fn exec(&mut self, input: &str) -> Promise {
        let input = input.trim().to_string();

        // We need to handle async commands differently
        // For sync commands, return resolved promise immediately
        // For async commands (a, e, g), return a proper future

        if input.is_empty() {
            return resolve_str("");
        }
```

The return type is `Promise` — every command returns a JS promise, even synchronous ones. This gives JavaScript a uniform `await rssed.exec(cmd)` interface. Sync commands return an already-resolved promise via `resolve_str()`. Async commands (`a`, `e`, `g`) return a real future wrapped by `wasm_bindgen_futures::future_to_promise()`.

The `resolve_str` helper at the bottom of the file is simple:

```bash
sed -n '504,507p' src/lib.rs
```

```output
fn resolve_str(s: &str) -> Promise {
    let val = JsValue::from_str(s);
    Promise::resolve(&val)
}
```

It wraps a Rust `&str` into a `JsValue`, then into an immediately-resolved `Promise`. This is the synchronous fast path.

### Item navigation and token splitting

```bash
sed -n '211,235p' src/lib.rs
```

```output
        // + / -
        if input == "+" {
            if self.store.is_empty() || self.store[self.cur].feed.entries.is_empty() {
                return resolve_str("?");
            }
            let max = self.store[self.cur].feed.entries.len() - 1;
            if self.item < max {
                self.item += 1;
            }
            return resolve_str(&format_entry(&self.store[self.cur].feed.entries[self.item]));
        }
        if input == "-" {
            if self.store.is_empty() || self.store[self.cur].feed.entries.is_empty() {
                return resolve_str("?");
            }
            if self.item > 0 {
                self.item -= 1;
            }
            return resolve_str(&format_entry(&self.store[self.cur].feed.entries[self.item]));
        }

        let (tok, arg) = match input.find(' ') {
            Some(i) => (input[..i].to_string(), Some(input[i + 1..].trim().to_string())),
            None => (input.clone(), None),
        };
```

This is the same dispatch structure as the CLI: check `+`/`-` first, then split the input into a command token and an optional argument. The only difference is `println!()` is replaced by `return resolve_str(...)` — instead of printing to stdout, the output is returned to JavaScript.

### Async commands: a, g, e

These three commands need to fetch URLs, which is async in the browser. They use `wasm_bindgen_futures::future_to_promise()` to return a real JS promise:

```bash
sed -n '237,263p' src/lib.rs
```

```output
        // --- Async commands ---
        match tok.as_str() {
            "a" => {
                let url = match arg {
                    Some(u) => u,
                    None => return resolve_str("?"),
                };
                // We need a raw pointer approach to mutate self from an async block.
                // Instead, we'll use a different pattern: return the data and apply it.
                let ptr = self as *mut Rssed;
                return wasm_bindgen_futures::future_to_promise(async move {
                    match fetch_feed(&url).await {
                        Ok(feed) => {
                            let n = feed.entries.len();
                            let rssed = unsafe { &mut *ptr };
                            rssed.store.push(StoredFeed {
                                url: url.to_string(),
                                feed,
                            });
                            rssed.cur = rssed.store.len() - 1;
                            rssed.item = 0;
                            rssed.dirty = true;
                            Ok(JsValue::from_str(&n.to_string()))
                        }
                        Err(_) => Ok(JsValue::from_str("?")),
                    }
                });
            }
```

The `let ptr = self as *mut Rssed` and `unsafe { &mut *ptr }` is the central trick. Rust's borrow checker won't let you move `&mut self` into an `async move` block — the block might outlive the borrow. But in WASM, there's only one thread, and the JavaScript side `await`s each `exec()` call before calling the next one, so the pointer is guaranteed valid for the duration of the future. The `unsafe` block documents this contract.

The `g` command (lines 265–288) follows the same pattern: take a raw pointer, spawn an async block, iterate feeds and re-fetch each URL. The `e` command (lines 290–322) reads URLs from `localStorage` synchronously (via `load_session_urls()`), then spawns the async block to fetch them.

### Sync command dispatch

```bash
sed -n '326,329p' src/lib.rs
```

```output
        // --- Sync commands ---
        let result = self.exec_sync(&tok, arg.as_deref());
        resolve_str(&result)
    }
```

If none of the async arms matched, the command is synchronous. `exec_sync` handles everything else and returns a `String`, which gets wrapped in a resolved promise. This split keeps the `exec()` method manageable — async commands return early with `future_to_promise()`, sync commands fall through to `exec_sync()`.

### exec_sync: the sync dispatch table

```bash
sed -n '332,367p' src/lib.rs
```

```output
impl Rssed {
    fn exec_sync(&mut self, tok: &str, _arg: Option<&str>) -> String {
        match tok {
            "w" => {
                match save_session(&self.store) {
                    Ok(bytes) => {
                        self.dirty = false;
                        self.quit_warned = false;
                        bytes.to_string()
                    }
                    Err(_) => "?".to_string(),
                }
            }
            "f" => {
                "(browser localStorage)".to_string()
            }
            "q" => {
                if self.dirty && !self.quit_warned {
                    self.quit_warned = true;
                    "?".to_string()
                } else {
                    self.store.clear();
                    self.cur = 0;
                    self.item = 0;
                    self.dirty = false;
                    self.quit_warned = false;
                    "__QUIT__".to_string()
                }
            }
            "Q" => {
                self.store.clear();
                self.cur = 0;
                self.item = 0;
                self.dirty = false;
                "__QUIT__".to_string()
            }
```

This is the same dispatch table as the CLI's second `match tok` block, with two differences:

1. **`w` calls `save_session()` instead of `fs::write()`.** The output is the same — a byte count.
2. **`q`/`Q` return `"__QUIT__"` instead of `break`ing.** There's no loop to break out of; the sentinel string is caught by JavaScript to display "session cleared." The `q` command still has the ed-style double-quit guard: first `q` with unsaved changes prints `?`, second `q` actually clears.

The remaining sync commands — `=`, `u`, `h`, `o`, `p`, `n`, `d`, bare numbers, and line-addressed commands — are structurally identical to the CLI. The only change throughout is replacing `println!()` with `return "...".to_string()`:

```bash
sed -n '414,448p' src/lib.rs
```

```output
            _ => {
                // Bare number: change current feed
                if let Ok(n) = tok.parse::<usize>() {
                    if !self.store.is_empty() && n < self.store.len() {
                        self.cur = n;
                        self.item = 0;
                        return self.store[self.cur].url.clone();
                    } else {
                        return "?".to_string();
                    }
                }

                // Line-addressed commands
                if tok.len() > 1 {
                    let (addr_s, cmd_s) = tok.split_at(tok.len() - 1);
                    let cmd_c = cmd_s.chars().next().unwrap();

                    match cmd_c {
                        'o' => {
                            if self.store.is_empty()
                                || self.store[self.cur].feed.entries.is_empty()
                            {
                                return "?".to_string();
                            }
                            let last = self.store[self.cur].feed.entries.len() - 1;
                            let Some(addr) = parse_addr(addr_s, self.item, last) else {
                                return "?".to_string();
                            };
                            let indices = addr_indices(addr);
                            let mut out = Vec::new();
                            for &i in &indices {
                                out.push(format_entry(&self.store[self.cur].feed.entries[i]));
                            }
                            self.item = *indices.last().unwrap();
                            out.join("\n")
                        }
```

One small difference from the CLI: where the CLI used `println!()` inside a loop (printing each line separately to stdout), the WASM version collects output into a `Vec<String>` and joins with `\n`. It has to — there's no stdout to print to line by line; the entire output must be returned as one string.

## The HTML shell: web/index.html

```bash
cat web/index.html
```

```output
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>rssed</title>
  <link rel="stylesheet" href="style.css">
</head>
<body>
  <div id="app">
    <!-- Top bar -->
    <header id="topbar">
      <button id="hamburger" aria-label="Menu" aria-expanded="false">
        <span></span>
        <span></span>
        <span></span>
      </button>
      <h1 id="title">rssed</h1>
      <nav id="nav-desktop">
        <a href="#" id="nav-home">Home</a>
        <a href="https://github.com/rgibbons-dev/rssed" target="_blank" rel="noopener">Source</a>
        <a href="#" id="nav-about">About</a>
      </nav>
    </header>

    <!-- Mobile slide-out menu -->
    <div id="mobile-menu-overlay" class="hidden"></div>
    <nav id="mobile-menu" class="hidden">
      <a href="#" class="mobile-nav-link" id="mobile-nav-home">Home</a>
      <a href="https://github.com/rgibbons-dev/rssed" target="_blank" rel="noopener" class="mobile-nav-link">Source</a>
      <a href="#" class="mobile-nav-link" id="mobile-nav-about">About</a>
    </nav>

    <!-- Terminal -->
    <main id="terminal-wrapper">
      <div id="terminal">
        <div id="output"></div>
        <div id="input-line">
          <span id="prompt">:</span>
          <input type="text" id="input" autocomplete="off" autocapitalize="off" spellcheck="false" autofocus>
        </div>
      </div>
    </main>

    <!-- About overlay -->
    <div id="about-overlay" class="hidden">
      <div id="about-panel">
        <button id="about-close" aria-label="Close">&times;</button>
        <h2>About rssed</h2>
        <p>An ed(1)-style RSS/Atom feed reader, compiled to WebAssembly and running entirely in your browser.</p>
        <p>Supports RSS 0.9/1.0/2.0, Atom 0.3/1.0, and JSON Feed 1.0.</p>
        <p>Type <code>h</code> for a full list of commands.</p>
        <p class="about-meta">Built with Rust + wasm-bindgen</p>
      </div>
    </div>
  </div>
  <script type="module" src="main.js"></script>
</body>
</html>
```

The HTML has four sections:

1. **Top bar** (lines 12–24): The hamburger button has three `<span>` children — CSS transforms them into an animated X when the menu opens. The title and desktop nav sit side by side; CSS repositions them per breakpoint.

2. **Mobile menu** (lines 27–32): A slide-down nav and a semi-transparent overlay behind it. Both start with `class="hidden"` and are toggled by JavaScript.

3. **Terminal** (lines 35–42): An output div and an input line. The output div holds a growing list of `<div class="line">` elements. The input is a plain `<input type="text">` with `autocomplete="off"` and `autocapitalize="off"` to prevent mobile keyboards from interfering. The `:` prompt span sits next to it.

4. **About overlay** (lines 46–55): A modal that appears when clicking "About" in either nav.

The `<script type="module">` at the bottom loads `main.js`, which imports the WASM module and wires everything together.

## The CSS: web/style.css

### Color system

```bash
sed -n '1,21p' web/style.css
```

```output
/*
 * rssed — browser terminal
 *
 * Color palette (Refactoring UI approach):
 *   - Define a base neutral scale + one accent color
 *   - Use darker shades for primary content, lighter for secondary
 *   - Limit the palette; let whitespace and type do the work
 *
 * Neutrals (slate):
 *   900  #0f172a   — terminal bg
 *   800  #1e293b   — panel surfaces
 *   700  #334155   — borders, subtle
 *   400  #94a3b8   — secondary text
 *   300  #cbd5e1   — primary text
 *   200  #e2e8f0   — bright text
 *   100  #f1f5f9   — headings
 *
 * Accent (amber):
 *   400  #fbbf24   — prompt, links, highlights
 *   300  #fcd34d   — hover states
 */
```

The palette follows Refactoring UI's approach: one neutral scale, one accent color. The slate scale provides seven shades from near-black (`#0f172a`) to near-white (`#f1f5f9`), creating depth through shade rather than color variety. Amber (`#fbbf24`) is the single accent — used for the prompt character, link hovers, and the input caret. Everything that's interactive or demands attention is amber; everything else is a shade of slate.

### Responsive layout

The desktop and mobile breakpoints are at 768px:

```bash
sed -n '341,367p' web/style.css
```

```output
@media (min-width: 768px) {
  #hamburger {
    display: none !important;
  }

  #title {
    position: absolute;
    left: 50%;
    transform: translateX(-50%);
  }

  #terminal-wrapper {
    padding: 24px 48px;
  }

  #terminal {
    max-width: 900px;
    border: 1px solid #334155;
    border-radius: 8px;
    overflow: hidden;
    background: #0f172a;
  }

  #input-line {
    border-radius: 0 0 8px 8px;
  }
}
```

On desktop: the hamburger is hidden, the title is absolutely centered, and the terminal gets padding, a max-width, and rounded corners — it floats as a contained panel. On mobile:

```bash
sed -n '371,396p' web/style.css
```

```output
@media (max-width: 767px) {
  #hamburger {
    display: flex;
  }

  #nav-desktop {
    display: none;
  }

  #topbar {
    padding: 0 12px;
  }

  #output {
    padding: 12px;
    font-size: 12px;
  }

  #input-line {
    padding: 8px 12px 10px;
  }

  #input {
    font-size: 16px; /* prevent iOS zoom */
  }
}
```

On mobile: the hamburger appears, the desktop nav hides, and the terminal fills the entire viewport edge-to-edge. The input font size is set to 16px specifically to prevent iOS Safari's auto-zoom behavior on focus (Safari zooms in on inputs smaller than 16px).

### Hamburger animation

```bash
sed -n '87,104p' web/style.css
```

```output
#hamburger span {
  display: block;
  width: 18px;
  height: 2px;
  background: #94a3b8;
  border-radius: 1px;
  transition: transform 0.2s, opacity 0.2s;
}

#hamburger[aria-expanded="true"] span:nth-child(1) {
  transform: translateY(6px) rotate(45deg);
}
#hamburger[aria-expanded="true"] span:nth-child(2) {
  opacity: 0;
}
#hamburger[aria-expanded="true"] span:nth-child(3) {
  transform: translateY(-6px) rotate(-45deg);
}
```

The three spans are the hamburger's three bars. When `aria-expanded="true"` (set by JavaScript on click), the top bar rotates 45 degrees down, the middle bar fades out, and the bottom bar rotates 45 degrees up — forming an X. The `transition` on transform and opacity makes it animate over 200ms.

### Terminal output styling

```bash
sed -n '196,214p' web/style.css
```

```output
#output .line {
  min-height: 1.6em;
}

#output .line-input {
  color: #fbbf24;
}

#output .line-output {
  color: #cbd5e1;
}

#output .line-error {
  color: #f87171;
}

#output .line-info {
  color: #94a3b8;
}
```

Four line types, four colors. Input lines (echoed commands) are amber — they visually separate what you typed from what the system responded. Output is the primary text color (slate 300). Errors are red (`#f87171`). Info messages (the boot banner, "session cleared") are subdued slate 400.

## The JavaScript: web/main.js

### Boot sequence

```bash
sed -n '1,14p' web/main.js
```

```output
import init, { Rssed } from "../pkg/rssed_wasm.js";

let rssed = null;
const output = document.getElementById("output");
const input = document.getElementById("input");
const hamburger = document.getElementById("hamburger");
const mobileMenu = document.getElementById("mobile-menu");
const overlay = document.getElementById("mobile-menu-overlay");
const aboutOverlay = document.getElementById("about-overlay");
const aboutClose = document.getElementById("about-close");

// --- History ---
const history = [];
let histIdx = -1;
```

The import pulls two things from the WASM glue code: `init` (an async function that loads and instantiates the `.wasm` binary) and `Rssed` (the JS class generated from the Rust struct). The `rssed` variable starts `null` and is set after init completes.

`history` and `histIdx` implement command history (arrow up/down). It's a simple array-and-index scheme — `histIdx` points at the currently recalled command, or equals `history.length` when you're at the bottom (typing new input).

```bash
sed -n '180,194p' web/main.js
```

```output
// --- Boot ---

async function boot() {
  await init();
  rssed = new Rssed();

  appendLine("rssed — ed(1)-style RSS reader", "line-info");
  appendLine('type "h" for help, "a <url>" to add a feed', "line-info");
  appendLine("", "line-info");
  input.focus();
}

boot().catch((err) => {
  appendLine(`fatal: failed to load WASM module: ${err}`, "line-error");
});
```

`boot()` loads the WASM module, creates the REPL instance, prints a welcome banner, and focuses the input. If WASM loading fails (network error, browser doesn't support WASM), the error is caught and displayed in red.

### Terminal output

```bash
sed -n '16,32p' web/main.js
```

```output
// --- Terminal helpers ---

function appendLine(text, cls = "line-output") {
  if (text === "") return;
  const lines = text.split("\n");
  for (const line of lines) {
    const div = document.createElement("div");
    div.className = `line ${cls}`;
    div.textContent = line;
    output.appendChild(div);
  }
  output.scrollTop = output.scrollHeight;
}

function appendInput(text) {
  appendLine(`: ${text}`, "line-input");
}
```

`appendLine` is the browser equivalent of `println!()`. It splits multi-line output (from commands like `o` that produce several lines) and creates a `<div>` per line. Using `textContent` rather than `innerHTML` prevents any HTML in feed content from being interpreted — this is a terminal, not a browser. After appending, it scrolls to the bottom.

`appendInput` echoes the user's command back, prefixed with `:` (the prompt character), in amber.

### CORS proxy

```bash
sed -n '34,51p' web/main.js
```

```output
// --- CORS proxy ---
// Many RSS feeds don't set CORS headers. We use a public CORS proxy
// so the WASM fetch can succeed from the browser.
const CORS_PROXY = "https://corsproxy.io/?url=";

function proxyUrl(cmd) {
  // If the command is 'a <url>', wrap the URL with the CORS proxy
  const match = cmd.match(/^a\s+(.+)$/);
  if (match) {
    let url = match[1].trim();
    // Don't double-proxy
    if (!url.startsWith(CORS_PROXY)) {
      url = CORS_PROXY + encodeURIComponent(url);
    }
    return `a ${url}`;
  }
  return cmd;
}
```

This is the one piece of the system that has no CLI equivalent. In the CLI, reqwest fetches URLs directly — there are no CORS restrictions. In the browser, most RSS feeds don't serve `Access-Control-Allow-Origin` headers, so `fetch()` would fail. The proxy rewrites the `a <url>` command to route through `corsproxy.io`, which fetches the URL server-side and returns it with permissive CORS headers.

The URL is `encodeURIComponent()`-encoded because it's passed as a query parameter. The double-proxy guard prevents encoding the URL twice if someone pastes a pre-proxied URL.

The `g` (refresh) and `e` (load session) commands also need the proxy, but they're handled automatically: the URLs stored in `rssed.store` are already proxied (they were added via `a`), so when `g` re-fetches them, the proxy URL is already baked in.

### Command execution

```bash
sed -n '53,88p' web/main.js
```

```output
// --- Command execution ---

let busy = false;

async function execute(raw) {
  if (!rssed || busy) return;

  const trimmed = raw.trim();
  if (!trimmed) return;

  // Record in history
  history.push(trimmed);
  histIdx = history.length;

  appendInput(trimmed);
  input.value = "";

  busy = true;
  input.placeholder = "working...";

  try {
    const proxied = proxyUrl(trimmed);
    const result = await rssed.exec(proxied);
    if (result === "__QUIT__") {
      appendLine("session cleared", "line-info");
    } else if (result) {
      appendLine(result);
    }
  } catch (err) {
    appendLine(`error: ${err}`, "line-error");
  }

  busy = false;
  input.placeholder = "";
  input.focus();
}
```

`execute` is the bridge between the browser UI and the Rust REPL. The `busy` flag prevents concurrent commands — since `exec()` returns a promise, a second Enter press during a slow fetch would otherwise trigger a second command. While busy, the input placeholder shows "working..." as a visual indicator.

The `"__QUIT__"` sentinel from the Rust side is caught here and translated to a human-readable message. In the CLI, `q`/`Q` broke the loop and the process exited. In the browser, there's no process to exit — "quit" just clears the in-memory session state.

### Input handling

```bash
sed -n '90,119p' web/main.js
```

```output
// --- Input handling ---

input.addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    e.preventDefault();
    execute(input.value);
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    if (histIdx > 0) {
      histIdx--;
      input.value = history[histIdx];
    }
  } else if (e.key === "ArrowDown") {
    e.preventDefault();
    if (histIdx < history.length - 1) {
      histIdx++;
      input.value = history[histIdx];
    } else {
      histIdx = history.length;
      input.value = "";
    }
  }
});

// Focus input when clicking on terminal
document.getElementById("terminal").addEventListener("click", (e) => {
  if (e.target.id !== "input") {
    input.focus();
  }
});
```

Enter submits the command. Arrow up/down navigate history, matching standard terminal behavior. The click-to-focus handler ensures clicking anywhere in the terminal area (not just the input field) activates the input — the terminal output is a passive display area, not interactive.

### Mobile menu

```bash
sed -n '121,148p' web/main.js
```

```output
// --- Mobile menu ---

function toggleMenu() {
  const open = !mobileMenu.classList.contains("hidden");
  if (open) {
    mobileMenu.classList.add("hidden");
    overlay.classList.add("hidden");
    hamburger.setAttribute("aria-expanded", "false");
  } else {
    mobileMenu.classList.remove("hidden");
    overlay.classList.remove("hidden");
    hamburger.setAttribute("aria-expanded", "true");
  }
}

function closeMenu() {
  mobileMenu.classList.add("hidden");
  overlay.classList.add("hidden");
  hamburger.setAttribute("aria-expanded", "false");
}

hamburger.addEventListener("click", toggleMenu);
overlay.addEventListener("click", closeMenu);

// Close menu on nav click
document.querySelectorAll(".mobile-nav-link").forEach((link) => {
  link.addEventListener("click", closeMenu);
});
```

The mobile menu toggles via `classList.add/remove("hidden")`. The `aria-expanded` attribute drives the CSS hamburger-to-X animation (the `#hamburger[aria-expanded="true"] span:nth-child(...)` rules). Clicking the overlay closes the menu. Clicking any nav link also closes the menu — the `forEach` on `.mobile-nav-link` handles this.

### About modal

```bash
sed -n '150,178p' web/main.js
```

```output
// --- About modal ---

function showAbout(e) {
  e.preventDefault();
  closeMenu();
  aboutOverlay.classList.remove("hidden");
}

function hideAbout() {
  aboutOverlay.classList.add("hidden");
}

document.getElementById("nav-about").addEventListener("click", showAbout);
document.getElementById("mobile-nav-about").addEventListener("click", showAbout);
aboutClose.addEventListener("click", hideAbout);
aboutOverlay.addEventListener("click", (e) => {
  if (e.target === aboutOverlay) hideAbout();
});

// Home links just focus the terminal
document.getElementById("nav-home").addEventListener("click", (e) => {
  e.preventDefault();
  input.focus();
});
document.getElementById("mobile-nav-home").addEventListener("click", (e) => {
  e.preventDefault();
  closeMenu();
  input.focus();
});
```

Both desktop and mobile About links trigger the same modal. Clicking the overlay background (not the panel itself) closes it — the `e.target === aboutOverlay` check ensures clicks inside the panel don't dismiss it. The Home links simply focus the terminal input.

## Line counts

```bash
wc -l src/lib.rs web/index.html web/style.css web/main.js
```

```output
  507 src/lib.rs
   59 web/index.html
  396 web/style.css
  194 web/main.js
 1156 total
```

1156 lines for a fully functional browser port of an ed(1)-style RSS reader — WASM compilation, responsive terminal UI, localStorage persistence, CORS proxying, command history, and mobile navigation. The Rust library is a near-direct port of the CLI; the feed parsing, address system, and dispatch logic carry over unchanged. The platform adaptation is concentrated in three places: `fetch_feed` (gloo-net instead of reqwest), the storage helpers (localStorage instead of `std::fs`), and the `exec` method (promise-returning dispatch instead of a stdin loop). The web layer is vanilla HTML/CSS/JS with no build tools, no framework, no npm.
