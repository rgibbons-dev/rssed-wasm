use wasm_bindgen::prelude::*;
use feed_rs::parser;
use feed_rs::model::{Entry, Feed};
use html2text::from_read;
use js_sys::Promise;
use serde::{Deserialize, Serialize};
use web_sys::window;

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

// --- Feed operations ---

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

// --- Help ---

fn help_text() -> &'static str {
    r#"rssed - an ed(1)-style RSS/Atom feed reader

line addressing:
  .      current line
  $      last line
  ,      first through last line
  ;      current through last line
  n      the nth line
  x,y    xth through yth line

commands:
  a <url>    add a feed
  [.,.]d     delete feed(s)
  e          load session from storage
  g          refresh all feeds
  h          show this help
  [.,.]n     print feed title(s) with line numbers
  [.,.]o     print item(s) from current feed
  [.,.]p     print feed title(s)
  q          quit (clear session state)
  u          print current item URL
  w          save session to storage
  =          print number of feeds
  [num]      set current feed
  +          next item
  -          previous item"#
}

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

    /// Process a single line of input, returns a Promise<String> with the output.
    pub fn exec(&mut self, input: &str) -> Promise {
        let input = input.trim().to_string();

        // We need to handle async commands differently
        // For sync commands, return resolved promise immediately
        // For async commands (a, e, g), return a proper future

        if input.is_empty() {
            return resolve_str("");
        }

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
            "g" => {
                if self.store.is_empty() {
                    return resolve_str("?");
                }
                let ptr = self as *mut Rssed;
                return wasm_bindgen_futures::future_to_promise(async move {
                    let rssed = unsafe { &mut *ptr };
                    let mut err = false;
                    for i in 0..rssed.store.len() {
                        let url = rssed.store[i].url.clone();
                        match fetch_feed(&url).await {
                            Ok(feed) => rssed.store[i].feed = feed,
                            Err(_) => {
                                err = true;
                                break;
                            }
                        }
                    }
                    if err {
                        Ok(JsValue::from_str("?"))
                    } else {
                        Ok(JsValue::from_str(""))
                    }
                });
            }
            "e" => {
                let urls = match load_session_urls() {
                    Ok(u) => u,
                    Err(_) => return resolve_str("?"),
                };
                let ptr = self as *mut Rssed;
                return wasm_bindgen_futures::future_to_promise(async move {
                    let rssed = unsafe { &mut *ptr };
                    rssed.store.clear();
                    let mut total = 0usize;
                    let mut output = String::new();
                    for url in &urls {
                        match fetch_feed(url).await {
                            Ok(feed) => {
                                total += feed.entries.len();
                                rssed.store.push(StoredFeed {
                                    url: url.to_string(),
                                    feed,
                                });
                            }
                            Err(_) => {
                                output.push_str("?\n");
                            }
                        }
                    }
                    rssed.cur = 0;
                    rssed.item = 0;
                    rssed.dirty = false;
                    rssed.quit_warned = false;
                    output.push_str(&total.to_string());
                    Ok(JsValue::from_str(&output))
                });
            }
            _ => {}
        }

        // --- Sync commands ---
        let result = self.exec_sync(&tok, arg.as_deref());
        resolve_str(&result)
    }
}

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
            "=" => self.store.len().to_string(),
            "u" => {
                if self.store.is_empty() || self.store[self.cur].feed.entries.is_empty() {
                    return "?".to_string();
                }
                let clamped = self.item.min(self.store[self.cur].feed.entries.len() - 1);
                match entry_link(&self.store[self.cur].feed.entries[clamped]) {
                    Some(link) => link,
                    None => "?".to_string(),
                }
            }
            "h" => help_text().to_string(),
            "o" => {
                if self.store.is_empty() || self.store[self.cur].feed.entries.is_empty() {
                    return "?".to_string();
                }
                let clamped = self.item.min(self.store[self.cur].feed.entries.len() - 1);
                self.item = clamped;
                format_entry(&self.store[self.cur].feed.entries[self.item])
            }
            "p" => {
                if self.store.is_empty() {
                    return "?".to_string();
                }
                feed_title(&self.store[self.cur].feed)
            }
            "n" => {
                if self.store.is_empty() {
                    return "?".to_string();
                }
                format!("{}\t{}", self.cur, feed_title(&self.store[self.cur].feed))
            }
            "d" => {
                if self.store.is_empty() {
                    return "?".to_string();
                }
                self.store.remove(self.cur);
                self.dirty = true;
                if self.store.is_empty() {
                    self.cur = 0;
                } else if self.cur >= self.store.len() {
                    self.cur = self.store.len() - 1;
                }
                self.item = 0;
                String::new()
            }
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
                        'p' | 'n' | 'd' => {
                            if self.store.is_empty() {
                                return "?".to_string();
                            }
                            let last = self.store.len() - 1;
                            let Some(addr) = parse_addr(addr_s, self.cur, last) else {
                                return "?".to_string();
                            };
                            let indices = addr_indices(addr);
                            match cmd_c {
                                'p' => {
                                    let mut out = Vec::new();
                                    for &i in &indices {
                                        out.push(feed_title(&self.store[i].feed));
                                    }
                                    out.join("\n")
                                }
                                'n' => {
                                    let mut out = Vec::new();
                                    for &i in &indices {
                                        out.push(format!(
                                            "{}\t{}",
                                            i,
                                            feed_title(&self.store[i].feed)
                                        ));
                                    }
                                    out.join("\n")
                                }
                                'd' => {
                                    for &i in indices.iter().rev() {
                                        self.store.remove(i);
                                    }
                                    self.dirty = true;
                                    if self.store.is_empty() {
                                        self.cur = 0;
                                    } else if self.cur >= self.store.len() {
                                        self.cur = self.store.len() - 1;
                                    }
                                    self.item = 0;
                                    String::new()
                                }
                                _ => unreachable!(),
                            }
                        }
                        _ => "?".to_string(),
                    }
                } else {
                    "?".to_string()
                }
            }
        }
    }
}

fn resolve_str(s: &str) -> Promise {
    let val = JsValue::from_str(s);
    Promise::resolve(&val)
}
