import init, { Rssed } from "./pkg/rssed_wasm.js";

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

// --- CORS proxy ---
// Most RSS feeds don't set CORS headers, so browser fetch() fails.
// This proxies requests through a third-party service. Caveats:
//   - It's a free service: may go down, rate-limit, or disappear
//   - All fetched URLs are visible to the proxy operator
//   - Not suitable for private/authenticated feeds
// To self-host, run your own CORS proxy (e.g. cors-anywhere) and
// change CORS_PROXY below. Set to "" to disable proxying entirely.
const CORS_PROXY = "https://corsproxy.io/?url=";

function proxyUrl(cmd) {
  // If the command is 'a <url>', wrap the URL with the CORS proxy
  const match = cmd.match(/^a\s+(.+)$/);
  if (match) {
    let url = match[1].trim();
    // Reject non-http(s) schemes to prevent SSRF against internal services
    try {
      const parsed = new URL(url);
      if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
        return cmd; // pass through unchanged; Rust side will fail on fetch
      }
    } catch {
      return cmd; // malformed URL, let it fail naturally
    }
    // Don't double-proxy
    if (CORS_PROXY && !url.startsWith(CORS_PROXY)) {
      url = CORS_PROXY + encodeURIComponent(url);
    }
    return `a ${url}`;
  }
  return cmd;
}

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
    appendLine(`error: ${err instanceof Error ? err.message : String(err)}`, "line-error");
  }

  busy = false;
  input.placeholder = "";
  input.focus();
}

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
  appendLine(`fatal: failed to load WASM module: ${err instanceof Error ? err.message : String(err)}`, "line-error");
});
