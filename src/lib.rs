use wasm_bindgen::prelude::*;
use feed_rs::parser;
use feed_rs::model::{Entry, Feed};
use html2text::from_read;
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
    // Only allow http(s) URLs to prevent SSRF via localStorage tampering
    let urls = data
        .urls
        .into_iter()
        .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
        .collect();
    Ok(urls)
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

    /// Process a single line of input. Returns the output string.
    pub async fn exec(&mut self, input: &str) -> String {
        let input = input.trim().to_string();

        if input.is_empty() {
            return String::new();
        }

        // + / -
        if input == "+" {
            if self.store.is_empty() || self.store[self.cur].feed.entries.is_empty() {
                return "?".to_string();
            }
            let max = self.store[self.cur].feed.entries.len() - 1;
            if self.item < max {
                self.item += 1;
            }
            return format_entry(&self.store[self.cur].feed.entries[self.item]);
        }
        if input == "-" {
            if self.store.is_empty() || self.store[self.cur].feed.entries.is_empty() {
                return "?".to_string();
            }
            if self.item > 0 {
                self.item -= 1;
            }
            return format_entry(&self.store[self.cur].feed.entries[self.item]);
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
                    None => return "?".to_string(),
                };
                match fetch_feed(&url).await {
                    Ok(feed) => {
                        let n = feed.entries.len();
                        self.store.push(StoredFeed {
                            url: url.to_string(),
                            feed,
                        });
                        self.cur = self.store.len() - 1;
                        self.item = 0;
                        self.dirty = true;
                        return n.to_string();
                    }
                    Err(_) => return "?".to_string(),
                }
            }
            "g" => {
                if self.store.is_empty() {
                    return "?".to_string();
                }
                for i in 0..self.store.len() {
                    let url = self.store[i].url.clone();
                    match fetch_feed(&url).await {
                        Ok(feed) => self.store[i].feed = feed,
                        Err(_) => return "?".to_string(),
                    }
                }
                return String::new();
            }
            "e" => {
                let urls = match load_session_urls() {
                    Ok(u) => u,
                    Err(_) => return "?".to_string(),
                };
                self.store.clear();
                let mut total = 0usize;
                let mut output = String::new();
                for url in &urls {
                    match fetch_feed(url).await {
                        Ok(feed) => {
                            total += feed.entries.len();
                            self.store.push(StoredFeed {
                                url: url.to_string(),
                                feed,
                            });
                        }
                        Err(_) => {
                            output.push_str("?\n");
                        }
                    }
                }
                self.cur = 0;
                self.item = 0;
                self.dirty = false;
                self.quit_warned = false;
                output.push_str(&total.to_string());
                return output;
            }
            _ => {}
        }

        // --- Sync commands ---
        self.exec_sync(&tok, arg.as_deref())
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

#[cfg(test)]
mod tests {
    use super::*;

    // === Address Parsing Tests ===

    // -- parse_one --

    #[test]
    fn parse_one_dot_returns_current() {
        assert_eq!(parse_one(".", 3, 10), Some(3));
    }

    #[test]
    fn parse_one_dollar_returns_last() {
        assert_eq!(parse_one("$", 3, 10), Some(10));
    }

    #[test]
    fn parse_one_valid_number() {
        assert_eq!(parse_one("5", 0, 10), Some(5));
    }

    #[test]
    fn parse_one_zero() {
        assert_eq!(parse_one("0", 0, 10), Some(0));
    }

    #[test]
    fn parse_one_at_last() {
        assert_eq!(parse_one("10", 0, 10), Some(10));
    }

    #[test]
    fn parse_one_beyond_last_returns_none() {
        assert_eq!(parse_one("11", 0, 10), None);
    }

    #[test]
    fn parse_one_non_numeric_returns_none() {
        assert_eq!(parse_one("abc", 0, 10), None);
    }

    #[test]
    fn parse_one_whitespace_trimmed() {
        assert_eq!(parse_one("  .  ", 5, 10), Some(5));
        assert_eq!(parse_one("  3  ", 0, 10), Some(3));
    }

    // -- parse_addr --

    #[test]
    fn parse_addr_dot() {
        let addr = parse_addr(".", 3, 10).unwrap();
        assert!(matches!(addr, Addr::One(3)));
    }

    #[test]
    fn parse_addr_dollar() {
        let addr = parse_addr("$", 3, 10).unwrap();
        assert!(matches!(addr, Addr::One(10)));
    }

    #[test]
    fn parse_addr_comma_full_range() {
        let addr = parse_addr(",", 3, 10).unwrap();
        assert!(matches!(addr, Addr::Range(0, 10)));
    }

    #[test]
    fn parse_addr_semicolon_from_current() {
        let addr = parse_addr(";", 3, 10).unwrap();
        assert!(matches!(addr, Addr::Range(3, 10)));
    }

    #[test]
    fn parse_addr_explicit_range() {
        let addr = parse_addr("2,5", 0, 10).unwrap();
        assert!(matches!(addr, Addr::Range(2, 5)));
    }

    #[test]
    fn parse_addr_inverted_range_returns_none() {
        assert!(parse_addr("5,2", 0, 10).is_none());
    }

    #[test]
    fn parse_addr_same_start_end() {
        let addr = parse_addr("3,3", 0, 10).unwrap();
        assert!(matches!(addr, Addr::Range(3, 3)));
    }

    #[test]
    fn parse_addr_single_number() {
        let addr = parse_addr("7", 0, 10).unwrap();
        assert!(matches!(addr, Addr::One(7)));
    }

    #[test]
    fn parse_addr_out_of_bounds_returns_none() {
        assert!(parse_addr("99", 0, 10).is_none());
    }

    #[test]
    fn parse_addr_range_with_dot() {
        let addr = parse_addr(".,5", 2, 10).unwrap();
        assert!(matches!(addr, Addr::Range(2, 5)));
    }

    #[test]
    fn parse_addr_range_with_dollar() {
        let addr = parse_addr("2,$", 0, 10).unwrap();
        assert!(matches!(addr, Addr::Range(2, 10)));
    }

    // -- addr_indices --

    #[test]
    fn addr_indices_single() {
        assert_eq!(addr_indices(Addr::One(5)), vec![5]);
    }

    #[test]
    fn addr_indices_range() {
        assert_eq!(addr_indices(Addr::Range(2, 5)), vec![2, 3, 4, 5]);
    }

    #[test]
    fn addr_indices_range_same() {
        assert_eq!(addr_indices(Addr::Range(3, 3)), vec![3]);
    }

    #[test]
    fn addr_indices_range_from_zero() {
        assert_eq!(addr_indices(Addr::Range(0, 2)), vec![0, 1, 2]);
    }

    // === Test Feed Helpers ===

    /// Build a minimal RSS feed from XML for testing.
    fn make_feed(xml: &str) -> Feed {
        feed_rs::parser::parse(xml.as_bytes()).unwrap()
    }

    fn sample_rss() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <rss version="2.0">
          <channel>
            <title>Test Feed</title>
            <item>
              <title>First Post</title>
              <link>https://example.com/1</link>
              <description>&lt;p&gt;Hello &lt;b&gt;world&lt;/b&gt;&lt;/p&gt;</description>
            </item>
            <item>
              <title>Second Post</title>
              <link>https://example.com/2</link>
              <description>Plain text summary</description>
            </item>
          </channel>
        </rss>"#
    }

    fn untitled_rss() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <rss version="2.0">
          <channel>
            <item>
              <description>No title here</description>
            </item>
          </channel>
        </rss>"#
    }

    // -- feed_title --

    #[test]
    fn feed_title_returns_title() {
        let feed = make_feed(sample_rss());
        assert_eq!(feed_title(&feed), "Test Feed");
    }

    #[test]
    fn feed_title_untitled_fallback() {
        let feed = make_feed(untitled_rss());
        assert_eq!(feed_title(&feed), "(untitled)");
    }

    // -- entry_title --

    #[test]
    fn entry_title_returns_title() {
        let feed = make_feed(sample_rss());
        assert_eq!(entry_title(&feed.entries[0]), "First Post");
    }

    #[test]
    fn entry_title_no_title_fallback() {
        let feed = make_feed(untitled_rss());
        assert_eq!(entry_title(&feed.entries[0]), "(no title)");
    }

    // -- entry_link --

    #[test]
    fn entry_link_returns_href() {
        let feed = make_feed(sample_rss());
        assert_eq!(
            entry_link(&feed.entries[0]),
            Some("https://example.com/1".to_string())
        );
    }

    #[test]
    fn entry_link_none_when_missing() {
        let feed = make_feed(untitled_rss());
        assert_eq!(entry_link(&feed.entries[0]), None);
    }

    // -- format_entry --

    #[test]
    fn format_entry_contains_title_and_separator() {
        let feed = make_feed(sample_rss());
        let out = format_entry(&feed.entries[0]);
        assert!(out.starts_with("First Post\n===============\n"));
    }

    #[test]
    fn format_entry_strips_html() {
        let feed = make_feed(sample_rss());
        let out = format_entry(&feed.entries[0]);
        // html2text should render <b>world</b> as plaintext
        assert!(out.contains("world"));
        assert!(!out.contains("<b>"));
    }

    #[test]
    fn format_entry_with_plain_text_summary() {
        let feed = make_feed(sample_rss());
        let out = format_entry(&feed.entries[1]);
        assert!(out.contains("Second Post"));
        assert!(out.contains("Plain text summary"));
    }

    // -- help_text --

    #[test]
    fn help_text_contains_commands() {
        let h = help_text();
        assert!(h.contains("a <url>"));
        assert!(h.contains("q "));
        assert!(h.contains("h "));
    }

    // === SessionData Serde Tests ===

    #[test]
    fn session_data_round_trip() {
        let data = SessionData {
            urls: vec![
                "https://example.com/feed.xml".to_string(),
                "https://other.com/rss".to_string(),
            ],
        };
        let json = serde_json::to_string(&data).unwrap();
        let restored: SessionData = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.urls, data.urls);
    }

    #[test]
    fn session_data_empty_urls() {
        let data = SessionData { urls: vec![] };
        let json = serde_json::to_string(&data).unwrap();
        let restored: SessionData = serde_json::from_str(&json).unwrap();
        assert!(restored.urls.is_empty());
    }

    #[test]
    fn session_data_deserialize_known_json() {
        let json = r#"{"urls":["https://example.com/feed"]}"#;
        let data: SessionData = serde_json::from_str(json).unwrap();
        assert_eq!(data.urls, vec!["https://example.com/feed"]);
    }

    #[test]
    fn session_data_invalid_json_fails() {
        let result = serde_json::from_str::<SessionData>("not json");
        assert!(result.is_err());
    }

    // === REPL exec_sync Tests ===

    /// Helper to build a Rssed instance with feeds loaded for testing.
    fn make_rssed_with_feeds() -> Rssed {
        let feed1 = make_feed(sample_rss());
        let feed2 = make_feed(
            r#"<?xml version="1.0" encoding="UTF-8"?>
            <rss version="2.0">
              <channel>
                <title>Second Feed</title>
                <item>
                  <title>Only Entry</title>
                  <link>https://example.com/only</link>
                  <description>The only entry</description>
                </item>
              </channel>
            </rss>"#,
        );
        Rssed {
            store: vec![
                StoredFeed {
                    url: "https://example.com/feed1".to_string(),
                    feed: feed1,
                },
                StoredFeed {
                    url: "https://example.com/feed2".to_string(),
                    feed: feed2,
                },
            ],
            cur: 0,
            item: 0,
            dirty: false,
            quit_warned: false,
        }
    }

    // -- = (count) --

    #[test]
    fn exec_sync_equals_returns_count() {
        let mut r = make_rssed_with_feeds();
        assert_eq!(r.exec_sync("=", None), "2");
    }

    #[test]
    fn exec_sync_equals_empty() {
        let mut r = Rssed::new();
        assert_eq!(r.exec_sync("=", None), "0");
    }

    // -- f (storage info) --

    #[test]
    fn exec_sync_f_returns_storage_label() {
        let mut r = Rssed::new();
        assert_eq!(r.exec_sync("f", None), "(browser localStorage)");
    }

    // -- h (help) --

    #[test]
    fn exec_sync_h_returns_help() {
        let mut r = Rssed::new();
        let out = r.exec_sync("h", None);
        assert!(out.contains("rssed"));
        assert!(out.contains("commands:"));
    }

    // -- p (print feed title) --

    #[test]
    fn exec_sync_p_prints_current_feed_title() {
        let mut r = make_rssed_with_feeds();
        assert_eq!(r.exec_sync("p", None), "Test Feed");
    }

    #[test]
    fn exec_sync_p_empty_store_returns_error() {
        let mut r = Rssed::new();
        assert_eq!(r.exec_sync("p", None), "?");
    }

    // -- n (print with line number) --

    #[test]
    fn exec_sync_n_prints_numbered() {
        let mut r = make_rssed_with_feeds();
        assert_eq!(r.exec_sync("n", None), "0\tTest Feed");
    }

    #[test]
    fn exec_sync_n_empty_returns_error() {
        let mut r = Rssed::new();
        assert_eq!(r.exec_sync("n", None), "?");
    }

    // -- u (print url) --

    #[test]
    fn exec_sync_u_prints_entry_url() {
        let mut r = make_rssed_with_feeds();
        assert_eq!(r.exec_sync("u", None), "https://example.com/1");
    }

    #[test]
    fn exec_sync_u_empty_returns_error() {
        let mut r = Rssed::new();
        assert_eq!(r.exec_sync("u", None), "?");
    }

    // -- o (print entry) --

    #[test]
    fn exec_sync_o_prints_entry() {
        let mut r = make_rssed_with_feeds();
        let out = r.exec_sync("o", None);
        assert!(out.starts_with("First Post\n===============\n"));
    }

    #[test]
    fn exec_sync_o_empty_returns_error() {
        let mut r = Rssed::new();
        assert_eq!(r.exec_sync("o", None), "?");
    }

    // -- bare number (select feed) --

    #[test]
    fn exec_sync_number_selects_feed() {
        let mut r = make_rssed_with_feeds();
        let out = r.exec_sync("1", None);
        assert_eq!(out, "https://example.com/feed2");
        assert_eq!(r.cur, 1);
        assert_eq!(r.item, 0);
    }

    #[test]
    fn exec_sync_number_out_of_range() {
        let mut r = make_rssed_with_feeds();
        assert_eq!(r.exec_sync("99", None), "?");
    }

    #[test]
    fn exec_sync_number_on_empty_store() {
        let mut r = Rssed::new();
        assert_eq!(r.exec_sync("0", None), "?");
    }

    // -- d (delete) --

    #[test]
    fn exec_sync_d_deletes_current_feed() {
        let mut r = make_rssed_with_feeds();
        r.exec_sync("d", None);
        assert_eq!(r.store.len(), 1);
        assert_eq!(feed_title(&r.store[0].feed), "Second Feed");
        assert!(r.dirty);
    }

    #[test]
    fn exec_sync_d_empty_returns_error() {
        let mut r = Rssed::new();
        assert_eq!(r.exec_sync("d", None), "?");
    }

    #[test]
    fn exec_sync_d_adjusts_cur_when_at_end() {
        let mut r = make_rssed_with_feeds();
        r.cur = 1; // point to last feed
        r.exec_sync("d", None);
        assert_eq!(r.cur, 0); // should adjust back
    }

    // -- q / Q (quit) --

    #[test]
    fn exec_sync_q_clean_returns_quit() {
        let mut r = make_rssed_with_feeds();
        assert_eq!(r.exec_sync("q", None), "__QUIT__");
        assert!(r.store.is_empty());
    }

    #[test]
    fn exec_sync_q_dirty_warns_first() {
        let mut r = make_rssed_with_feeds();
        r.dirty = true;
        assert_eq!(r.exec_sync("q", None), "?");
        assert!(r.quit_warned);
        // Second q should quit
        assert_eq!(r.exec_sync("q", None), "__QUIT__");
    }

    #[test]
    fn exec_sync_big_q_force_quits() {
        let mut r = make_rssed_with_feeds();
        r.dirty = true;
        assert_eq!(r.exec_sync("Q", None), "__QUIT__");
        assert!(r.store.is_empty());
    }

    // -- addressed commands --

    #[test]
    fn exec_sync_comma_p_prints_all_feeds() {
        let mut r = make_rssed_with_feeds();
        let out = r.exec_sync(",p", None);
        assert!(out.contains("Test Feed"));
        assert!(out.contains("Second Feed"));
    }

    #[test]
    fn exec_sync_comma_n_prints_all_numbered() {
        let mut r = make_rssed_with_feeds();
        let out = r.exec_sync(",n", None);
        assert!(out.contains("0\tTest Feed"));
        assert!(out.contains("1\tSecond Feed"));
    }

    #[test]
    fn exec_sync_addressed_d_deletes_range() {
        let mut r = make_rssed_with_feeds();
        r.exec_sync(",d", None);
        assert!(r.store.is_empty());
        assert!(r.dirty);
    }

    #[test]
    fn exec_sync_single_addr_p() {
        let mut r = make_rssed_with_feeds();
        let out = r.exec_sync("1p", None);
        assert_eq!(out, "Second Feed");
    }

    #[test]
    fn exec_sync_addressed_o_prints_items() {
        let mut r = make_rssed_with_feeds();
        let out = r.exec_sync("0o", None);
        assert!(out.contains("First Post"));
    }

    #[test]
    fn exec_sync_dollar_o_prints_last_item() {
        let mut r = make_rssed_with_feeds();
        let out = r.exec_sync("$o", None);
        assert!(out.contains("Second Post"));
    }

    #[test]
    fn exec_sync_unknown_command_returns_error() {
        let mut r = Rssed::new();
        assert_eq!(r.exec_sync("z", None), "?");
    }

    #[test]
    fn exec_sync_unknown_addressed_command_returns_error() {
        let mut r = make_rssed_with_feeds();
        assert_eq!(r.exec_sync("0x", None), "?");
    }

    // === Atom Feed Tests ===

    fn sample_atom() -> &'static str {
        r#"<?xml version="1.0" encoding="utf-8"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
          <title>Atom Feed</title>
          <entry>
            <title>Atom Entry</title>
            <link href="https://atom.example.com/1"/>
            <content type="html">&lt;p&gt;Atom content&lt;/p&gt;</content>
          </entry>
        </feed>"#
    }

    #[test]
    fn atom_feed_title() {
        let feed = make_feed(sample_atom());
        assert_eq!(feed_title(&feed), "Atom Feed");
    }

    #[test]
    fn atom_entry_title() {
        let feed = make_feed(sample_atom());
        assert_eq!(entry_title(&feed.entries[0]), "Atom Entry");
    }

    #[test]
    fn atom_entry_link() {
        let feed = make_feed(sample_atom());
        assert_eq!(
            entry_link(&feed.entries[0]),
            Some("https://atom.example.com/1".to_string())
        );
    }

    #[test]
    fn atom_format_entry() {
        let feed = make_feed(sample_atom());
        let out = format_entry(&feed.entries[0]);
        assert!(out.starts_with("Atom Entry\n===============\n"));
        assert!(out.contains("Atom content"));
    }

    // === Edge Case / Boundary Tests ===

    #[test]
    fn parse_addr_empty_string() {
        assert!(parse_addr("", 0, 10).is_none());
    }

    #[test]
    fn parse_one_negative_string() {
        assert_eq!(parse_one("-1", 0, 10), None);
    }

    #[test]
    fn parse_addr_comma_with_zero_last() {
        // Edge: only one feed (index 0)
        let addr = parse_addr(",", 0, 0).unwrap();
        assert!(matches!(addr, Addr::Range(0, 0)));
    }

    #[test]
    fn exec_sync_delete_last_remaining_feed() {
        let feed = make_feed(sample_rss());
        let mut r = Rssed {
            store: vec![StoredFeed {
                url: "https://example.com/only".to_string(),
                feed,
            }],
            cur: 0,
            item: 0,
            dirty: false,
            quit_warned: false,
        };
        r.exec_sync("d", None);
        assert!(r.store.is_empty());
        assert_eq!(r.cur, 0);
        assert_eq!(r.item, 0);
    }

    #[test]
    fn exec_sync_select_feed_resets_item() {
        let mut r = make_rssed_with_feeds();
        r.item = 1; // browsing second entry in feed 0
        r.exec_sync("1", None); // switch to feed 1
        assert_eq!(r.item, 0); // item should reset
    }

    #[test]
    fn exec_sync_addressed_o_updates_item_index() {
        let mut r = make_rssed_with_feeds();
        assert_eq!(r.item, 0);
        r.exec_sync("1o", None); // view second entry
        assert_eq!(r.item, 1); // item should advance to last viewed
    }

    #[test]
    fn exec_sync_semicolon_p_from_current() {
        let mut r = make_rssed_with_feeds();
        r.cur = 1;
        let out = r.exec_sync(";p", None);
        // semicolon = current through last, cur=1, last=1
        assert_eq!(out, "Second Feed");
    }

    #[test]
    fn exec_sync_dot_p_prints_current() {
        let mut r = make_rssed_with_feeds();
        r.cur = 1;
        let out = r.exec_sync(".p", None);
        assert_eq!(out, "Second Feed");
    }

    #[test]
    fn exec_sync_dollar_p_prints_last() {
        let mut r = make_rssed_with_feeds();
        let out = r.exec_sync("$p", None);
        assert_eq!(out, "Second Feed");
    }

    #[test]
    fn new_rssed_defaults() {
        let r = Rssed::new();
        assert!(r.store.is_empty());
        assert_eq!(r.cur, 0);
        assert_eq!(r.item, 0);
        assert!(!r.dirty);
        assert!(!r.quit_warned);
    }

    #[test]
    fn exec_sync_q_resets_all_state() {
        let mut r = make_rssed_with_feeds();
        r.cur = 1;
        r.item = 1;
        r.exec_sync("q", None);
        assert_eq!(r.cur, 0);
        assert_eq!(r.item, 0);
        assert!(!r.dirty);
        assert!(!r.quit_warned);
    }

    #[test]
    fn exec_sync_range_delete_middle() {
        // Build 3 feeds, delete the middle one via addressed d
        let feed1 = make_feed(sample_rss());
        let feed2 = make_feed(sample_atom());
        let feed3 = make_feed(
            r#"<?xml version="1.0" encoding="UTF-8"?>
            <rss version="2.0">
              <channel><title>Third Feed</title>
                <item><title>T</title><description>t</description></item>
              </channel>
            </rss>"#,
        );
        let mut r = Rssed {
            store: vec![
                StoredFeed { url: "u1".to_string(), feed: feed1 },
                StoredFeed { url: "u2".to_string(), feed: feed2 },
                StoredFeed { url: "u3".to_string(), feed: feed3 },
            ],
            cur: 0,
            item: 0,
            dirty: false,
            quit_warned: false,
        };
        r.exec_sync("1d", None);
        assert_eq!(r.store.len(), 2);
        assert_eq!(feed_title(&r.store[0].feed), "Test Feed");
        assert_eq!(feed_title(&r.store[1].feed), "Third Feed");
    }
}
