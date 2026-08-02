// Inline "Nitrate" buttons on the sites people grab clips from.
//
// None of these expose a stable hook — the class names are build hashes and the
// markup changes without notice. So every site gets a list of candidate anchors
// tried in order, and if all of them miss, a floating button appears instead.
// Selector rot then degrades to "the button moved to the corner" rather than
// "the feature vanished".

const MARK = "data-nitrate-button";

const MARK_SVG = `
<svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true">
  <g stroke="currentColor" stroke-width="2.4" stroke-linecap="round"
     stroke-linejoin="round" fill="none">
    <polyline points="7,5 12,9 17,5"/>
    <polyline points="7,19 12,15 17,19"/>
  </g>
  <rect x="6" y="11" width="12" height="2.4" rx="1.2" fill="currentColor"/>
</svg>`;

/**
 * Per-site strategy.
 *
 * `anchors` are tried in order until one matches. `scope` limits the search to
 * a repeating item (a tweet, a post) so feeds get one button each. `resolve`
 * works out which URL that particular button should send.
 */
const SITES = [
  {
    id: "youtube",
    matches: () => /(^|\.)youtube\.com$|(^|\.)youtu\.be$/.test(location.hostname),
    scope: null,
    anchors: [
      "ytd-watch-metadata #top-level-buttons-computed",
      "#top-level-buttons-computed",
      "ytd-menu-renderer #top-level-buttons-computed",
    ],
    resolve: () => location.href,
  },
  {
    id: "twitch",
    matches: () => /(^|\.)twitch\.tv$/.test(location.hostname),
    scope: null,
    anchors: [
      '[data-a-target="clips-share-button"]',
      '[data-target="clips-page__content"] [data-a-target="share-button"]',
      '[data-a-target="share-button"]',
    ],
    anchorPlacement: "after",
    resolve: () => location.href,
  },
  {
    id: "x",
    matches: () => /(^|\.)x\.com$|(^|\.)twitter\.com$/.test(location.hostname),
    scope: 'article[data-testid="tweet"]',
    anchors: ['[role="group"]'],
    resolve: (scope) => {
      const link = scope?.querySelector('a[href*="/status/"]');
      return link ? new URL(link.getAttribute("href"), location.origin).href : location.href;
    },
  },
  {
    id: "reddit",
    matches: () => /(^|\.)reddit\.com$/.test(location.hostname),
    scope: "shreddit-post",
    anchors: ['[slot="credit-bar"]', "shreddit-post-share-button", '[data-post-click-location="share"]'],
    resolve: (scope) => {
      const permalink = scope?.getAttribute("permalink");
      return permalink ? new URL(permalink, location.origin).href : location.href;
    },
  },
  {
    id: "instagram",
    matches: () => /(^|\.)instagram\.com$/.test(location.hostname),
    scope: "article",
    anchors: ["section:has(svg[aria-label])", "section"],
    resolve: (scope) => {
      const link = scope?.querySelector('a[href*="/p/"], a[href*="/reel/"]');
      return link ? new URL(link.getAttribute("href"), location.origin).href : location.href;
    },
  },
];

const site = SITES.find((s) => {
  try {
    return s.matches();
  } catch {
    return false;
  }
});

if (site) {
  injectStyles();
  scheduleScan();
  observe();
}

// ---------------------------------------------------------------------------

function makeButton(getUrl) {
  const button = document.createElement("button");
  button.setAttribute(MARK, "1");
  button.className = "nitrate-send";
  button.type = "button";
  button.title = "Send to Nitrate";
  button.innerHTML = `${MARK_SVG}<span>Nitrate</span>`;

  button.addEventListener("click", (event) => {
    // Feeds wrap posts in giant click targets; without this, sending a link
    // also opens the post.
    event.preventDefault();
    event.stopPropagation();

    const url = getUrl();
    button.classList.add("is-busy");

    chrome.runtime.sendMessage({ type: "nitrate:send", url }, (reply) => {
      button.classList.remove("is-busy");
      const ok = !chrome.runtime.lastError && reply?.ok;
      button.classList.add(ok ? "is-done" : "is-failed");
      setTimeout(() => button.classList.remove("is-done", "is-failed"), 1600);
    });
  });

  return button;
}

function place(anchor, button, placement) {
  if (placement === "after") anchor.after(button);
  else anchor.appendChild(button);
}

function scan() {
  const scopes = site.scope
    ? Array.from(document.querySelectorAll(site.scope))
    : [document];

  for (const scope of scopes) {
    const root = scope === document ? document : scope;
    if (root.querySelector?.(`[${MARK}]`)) continue;

    let anchor = null;
    for (const selector of site.anchors) {
      try {
        anchor = root.querySelector(selector);
      } catch {
        // `:has()` and custom elements aren't universally supported; a bad
        // selector shouldn't take the whole scan down.
        anchor = null;
      }
      if (anchor) break;
    }

    if (!anchor) continue;
    const button = makeButton(() => site.resolve(scope === document ? null : scope));
    place(anchor, button, site.anchorPlacement);
  }

  ensureFallback();
}

/**
 * When no anchor matched anywhere, put a button in the corner so the feature
 * still works on a page whose markup has moved on.
 */
function ensureFallback() {
  const hasInline = document.querySelector(`[${MARK}]:not(.nitrate-floating)`);
  const existing = document.querySelector(".nitrate-floating");

  if (hasInline) {
    existing?.remove();
    return;
  }
  if (existing) return;

  const button = makeButton(() => location.href);
  button.classList.add("nitrate-floating");
  document.body?.appendChild(button);
}

// SPA navigation and infinite feeds both mean the DOM never settles, so the
// scan is debounced rather than run per mutation.
let pending = 0;
function scheduleScan() {
  clearTimeout(pending);
  pending = setTimeout(() => {
    try {
      scan();
    } catch {
      // Never let a site's markup break the page it's injected into.
    }
  }, 400);
}

function observe() {
  const observer = new MutationObserver(scheduleScan);
  observer.observe(document.documentElement, { childList: true, subtree: true });
  window.addEventListener("popstate", scheduleScan);
  // YouTube fires this on every in-app navigation.
  window.addEventListener("yt-navigate-finish", scheduleScan);
}

function injectStyles() {
  const style = document.createElement("style");
  style.textContent = `
    .nitrate-send {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      padding: 6px 12px;
      margin: 0 4px;
      border: none;
      border-radius: 999px;
      background: #5865F2;
      color: #fff;
      font: 600 13px/1 system-ui, -apple-system, "Segoe UI", sans-serif;
      cursor: pointer;
      vertical-align: middle;
      transition: filter .15s, opacity .15s;
    }
    .nitrate-send:hover { filter: brightness(1.12); }
    .nitrate-send.is-busy { opacity: .6; cursor: default; }
    .nitrate-send.is-done { background: #43B581; }
    .nitrate-send.is-failed { background: #ED4245; }
    .nitrate-send.nitrate-floating {
      position: fixed;
      right: 18px;
      bottom: 18px;
      z-index: 2147483000;
      box-shadow: 0 6px 20px rgba(0,0,0,.35);
    }
  `;
  (document.head || document.documentElement).appendChild(style);
}
