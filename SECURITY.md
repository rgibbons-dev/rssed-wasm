# rssed-wasm: Security Audit

Read-only assessment of the codebase at commit `e7851ce`.

## Scope

| File | Lines | Role |
|---|---|---|
| `src/lib.rs` | 502 | Rust WASM library — REPL engine, feed fetching, localStorage |
| `web/main.js` | 194 | JS terminal emulator, CORS proxy, DOM output |
| `web/index.html` | 59 | HTML shell |
| `web/style.css` | 396 | Stylesheet |
| `build.sh` | 20 | Build script |
| `Cargo.toml` | 25 | Dependency manifest |

## Findings

### 1. SSRF via CORS proxy — user-supplied URLs fetched server-side

**Severity:** MEDIUM
**File:** `web/main.js:44-55`, `src/lib.rs:29-42`
**Class:** SSRF

```bash
sed -n '44,55p' web/main.js
```

```output
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

The user types `a <url>`, the URL is proxied through `corsproxy.io`, which fetches it server-side. An attacker (or the user themselves) could supply internal/private network URLs (`http://169.254.169.254/latest/meta-data/`, `http://localhost:8080/admin`, etc.) and the CORS proxy would fetch them, potentially exposing internal services. This is SSRF-by-proxy — the proxy operator's infrastructure is the target, not ours, but if someone self-hosts the proxy (as the README recommends), it becomes a direct SSRF against their own network.

**SUGGESTION:** For self-hosted proxies, recommend allowlisting URL schemes (`https://` only) and blocking private IP ranges. In `proxyUrl()`, reject non-http(s) schemes and RFC1918/link-local IPs client-side:

```js
const parsed = new URL(url);
if (!["http:", "https:"].includes(parsed.protocol)) return cmd;
```

### 2. localStorage poisoning — deserialized without URL validation

**Severity:** LOW
**File:** `src/lib.rs:164-171`
**Class:** Insecure deserialization / localStorage tampering

```bash
sed -n '164,171p' src/lib.rs
```

```output
fn load_session_urls() -> Result<Vec<String>, String> {
    let storage = get_storage().ok_or_else(|| "no storage available".to_string())?;
    let json = storage
        .get_item("rssed_session")
        .map_err(|_| "storage read failed".to_string())?
        .ok_or_else(|| "no saved session".to_string())?;
    let data: SessionData = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    Ok(data.urls)
```

`localStorage` is writable by any JS on the same origin (e.g. via XSS on any co-hosted page, or browser devtools). A tampered `rssed_session` key could contain arbitrary URLs. When the user runs `e` (load session), each URL is fetched through the CORS proxy. Combined with finding #1, localStorage poisoning could trigger SSRF without the user typing the malicious URL.

The `SessionData` struct is simple (`Vec<String>`) so there's no deserialization gadget chain risk — serde_json into a flat struct is safe. The risk is purely the URL content.

**SUGGESTION:** Validate loaded URLs before fetching:

```rust
let urls: Vec<String> = data.urls.into_iter()
    .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
    .collect();
```

### 3. className injection in appendLine (latent)

**Severity:** LOW (not currently exploitable)
**File:** `web/main.js:23`
**Class:** DOM-based injection

```bash
sed -n '22,24p' web/main.js
```

```output
    const div = document.createElement("div");
    div.className = `line ${cls}`;
    div.textContent = line;
```

The `cls` parameter is interpolated into `className`. If `cls` ever came from external input, an attacker could inject arbitrary class names. **Currently not exploitable** — all callers pass hardcoded string literals. Latent footgun if the code evolves.

**SUGGESTION:** No action needed now. If `cls` becomes dynamic, validate against an allowlist.

### 4. Error messages may leak internal state

**Severity:** LOW
**File:** `web/main.js:87, 198`
**Class:** Information disclosure

```bash
sed -n '85,89p' web/main.js
```

```output
  busy = false;
  input.placeholder = "";
  input.focus();
```

```bash
sed -n '86,88p' web/main.js
```

```output
    appendLine(`error: ${err}`, "line-error");
```

JS error objects can contain stack traces, internal paths, and system details. These are displayed directly in the terminal UI. For a local tool this is fine; for a hosted version it could leak browser/system internals.

**SUGGESTION:** For production, sanitize: `err.message || String(err)`.

## Negative results (not vulnerable)

| Class | Status |
|---|---|
| XSS (reflected, stored, DOM-based) | **Not vulnerable** — all DOM output uses `textContent`, never `innerHTML` |
| Command injection | N/A — no shell calls, no `exec`, no `child_process` |
| SQL injection | N/A — no database |
| Path traversal | N/A — no filesystem access (browser WASM) |
| XXE | **Not vulnerable** — feed-rs uses quick-xml, which doesn't process DTDs/external entities |
| SSTI | N/A — no server-side templates |
| Prototype pollution | **Not vulnerable** — no object merging or dynamic property access |
| Hardcoded secrets | None found |
| JWT / auth misconfig | N/A — no authentication |
| Prompt injection | N/A — no LLM in the loop |
| unsafe Rust | **None** — eliminated in the async fn refactor |
| Race conditions | **Low risk** — `busy` flag prevents concurrent `exec()` calls; WASM is single-threaded |
| build.sh injection | **Not vulnerable** — no user input flows into the build script |

## Summary

| # | Severity | Finding | File | Lines |
|---|---|---|---|---|
| 1 | **MEDIUM** | SSRF via CORS proxy | `web/main.js` | 44–55 |
| 2 | **LOW** | localStorage poisoning feeds unvalidated URLs | `src/lib.rs` | 164–171 |
| 3 | **LOW** | Latent className injection | `web/main.js` | 23 |
| 4 | **LOW** | Error messages may leak internal state | `web/main.js` | 87, 198 |

**Overall posture: Good.** Small codebase, no auth surface, consistent use of `textContent` (no XSS), no `unsafe` Rust, XXE-safe XML parser. The only actionable finding is SSRF-via-proxy, which is inherent to the CORS proxy architecture and already documented in the README. Adding client-side URL scheme validation would reduce the risk for self-hosters.
