// Inline "Nitrate" buttons on the sites people grab clips from.
//
// None of these expose a stable hook — the class names are build hashes and the
// markup changes without notice. So every site gets a list of candidate anchors
// tried in order, then a structural search that looks for the action row by
// shape rather than by name, and only if both miss does a floating button
// appear. Selector rot then degrades to "the button moved" rather than "the
// feature vanished".

const MARK = "data-nitrate-button";
const LOG = "[Nitrate]";

const MARK_SVG = `
<svg class="nitrate-icon" viewBox="0 0 24 24" aria-hidden="true">
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
 * `anchors` are tried in order until one matches; each is either a selector or
 * `{ sel, parent, placement }`, where `parent` uses the match's parent (handy
 * when the stable name belongs to a child of the row we actually want).
 * `scope` limits the search to a repeating item — a tweet, a post — so feeds
 * get one button each. `resolve` works out which URL that button should send.
 * `label` is whether the word "Nitrate" is written next to the mark by default,
 * which follows whatever the site's own buttons do.
 */
const SITES = [
  {
    id: "youtube",
    matches: () => /(^|\.)youtube\.com$|(^|\.)youtu\.be$/.test(location.hostname),
    scope: null,
    label: true,
    anchors: [
      "ytd-watch-metadata #top-level-buttons-computed",
      // The like/dislike pill is the one element in that row with a name that
      // has survived several redesigns, so its parent is the row itself.
      { sel: "ytd-watch-metadata ytd-segmented-like-dislike-button-view-model", parent: true },
      { sel: "ytd-watch-metadata segmented-like-dislike-button-view-model", parent: true },
      "#top-level-buttons-computed",
      "ytd-menu-renderer #top-level-buttons-computed",
    ],
    resolve: () => location.href,
  },
  {
    id: "twitch",
    matches: () => /(^|\.)twitch\.tv$/.test(location.hostname),
    scope: null,
    label: true,
    anchors: [
      { sel: '[data-a-target="clips-share-button"]', placement: "after" },
      { sel: '[data-target="clips-page__content"] [data-a-target="share-button"]', placement: "after" },
      { sel: '[data-a-target="share-button"]', placement: "after" },
    ],
    resolve: () => location.href,
  },
  {
    id: "x",
    matches: () => /(^|\.)x\.com$|(^|\.)twitter\.com$/.test(location.hostname),
    scope: 'article[data-testid="tweet"]',
    // The whole row is icons and counts; a word would be the only text in it.
    label: false,
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
    label: true,
    // Deliberately not the credit bar: that's the byline above the title, which
    // is where the button ended up looking like a stray banner. These all sit
    // in the vote/comment/share row along the bottom, where it belongs.
    anchors: [
      { sel: "shreddit-post-share-button", placement: "after" },
      { sel: '[data-post-click-location="share"]', placement: "after" },
      { sel: 'shreddit-post [slot="full-post-link"] ~ div [data-testid="comments-action-button"]', placement: "after" },
    ],
    resolve: (scope) => {
      const permalink = scope?.getAttribute("permalink");
      return permalink ? new URL(permalink, location.origin).href : location.href;
    },
  },
  {
    id: "instagram",
    matches: () => /(^|\.)instagram\.com$/.test(location.hostname),
    // Feed posts live in `article`; reels don't, and their controls are a
    // vertical rail down the right-hand side. When there's no article to scope
    // to, search the page and let the structural pass find that rail.
    scope: () => {
      const articles = Array.from(document.querySelectorAll("article"));
      return articles.length ? articles : [document.body];
    },
    label: false,
    anchors: [],
    resolve: (scope) => {
      const link = scope?.querySelector?.('a[href*="/p/"], a[href*="/reel/"]');
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

// "auto" follows the site: written out where the native buttons carry words,
// mark-only where they're all icons. Overridable from the extension's options.
let labelMode = "auto";

// Startup lives at the very bottom of this file, after every declaration has
// been evaluated. Calling it from up here would work for the hoisted function
// declarations but not for the `let` bindings they close over.

// ---------------------------------------------------------------------------

function wantsLabel() {
  if (labelMode === "icon") return false;
  if (labelMode === "label") return true;
  return Boolean(site?.label);
}

function makeButton(getUrl) {
  const button = document.createElement("button");
  button.setAttribute(MARK, "1");
  button.className = "nitrate-send";
  button.dataset.site = site.id;
  button.type = "button";
  button.title = "Send to Nitrate";
  // The tooltip isn't announced, so the name has to be here too — otherwise
  // the mark-only button reads as an unlabelled button to a screen reader.
  button.setAttribute("aria-label", "Send to Nitrate");
  button.innerHTML = wantsLabel()
    ? `${MARK_SVG}<span class="nitrate-label">Nitrate</span>`
    : MARK_SVG;

  button.addEventListener("click", (event) => {
    // Feeds wrap posts in giant click targets; without this, sending a link
    // also opens the post.
    event.preventDefault();
    event.stopPropagation();

    const url = getUrl();
    let ok = false;

    // Fired from here rather than by asking the background to inject a script.
    //
    // Being listed in `content_scripts` does not grant host permissions, and
    // `activeTab` is only granted after a gesture on the extension's own UI —
    // a button injected into the page isn't that. So `scripting.executeScript`
    // against this tab is refused. Navigating is something the content script
    // can already do, and needs no permission at all.
    try {
      if (/^https?:\/\//i.test(url)) {
        window.location.href = `nitrate://add?url=${encodeURIComponent(url)}`;
        ok = true;
      }
    } catch {
      ok = false;
    }

    button.classList.add(ok ? "is-done" : "is-failed");
    setTimeout(() => button.classList.remove("is-done", "is-failed"), 1600);
  });

  return button;
}

/**
 * Finds the action row by shape instead of by name.
 *
 * Every one of these sites groups its controls into one small box holding
 * several labelled icons, so the smallest element containing at least three of
 * them is the row — or, on a reel, the vertical rail. That survives a rename,
 * which is what the selectors above keep failing to do.
 */
function findActionCluster(root) {
  const scope = root === document ? document.body : root;
  if (!scope?.querySelectorAll) return null;

  const icons = Array.from(scope.querySelectorAll("svg[aria-label], svg[role='img']"));
  if (icons.length < 3) return null;

  const counts = new Map();
  for (const icon of icons) {
    let node = icon.parentElement;
    for (let depth = 0; node && node !== scope.parentElement && depth < 10; depth += 1) {
      counts.set(node, (counts.get(node) || 0) + 1);
      node = node.parentElement;
    }
  }

  let best = null;
  for (const [node, count] of counts) {
    if (count < 3) continue;
    if (node.querySelector(`[${MARK}]`)) continue;
    const rect = node.getBoundingClientRect?.();
    if (!rect || !rect.width || !rect.height) continue;
    const area = rect.width * rect.height;
    if (!best || area < best.area) best = { node, area, rect };
  }

  if (!best) return null;
  // A rail is taller than it is wide; the button wants to stack, not sit beside.
  best.node.dataset.nitrateOrientation =
    best.rect.height > best.rect.width * 1.5 ? "column" : "row";
  return best.node;
}

function findAnchor(root) {
  for (const entry of site.anchors) {
    const { sel, parent = false, placement } = typeof entry === "string" ? { sel: entry } : entry;
    let match = null;
    try {
      match = root.querySelector?.(sel) ?? null;
    } catch {
      // `:has()` and custom elements aren't universally supported; a bad
      // selector shouldn't take the whole scan down.
      match = null;
    }
    if (!match) continue;
    const node = parent ? match.parentElement : match;
    if (node) return { node, placement: placement ?? "append" };
  }

  const cluster = findActionCluster(root);
  return cluster ? { node: cluster, placement: "append" } : null;
}

function place(anchor, button, placement) {
  if (placement === "after") anchor.after(button);
  else anchor.appendChild(button);
}

function scopes() {
  if (typeof site.scope === "function") return site.scope();
  if (site.scope) return Array.from(document.querySelectorAll(site.scope));
  return [document];
}

function scan() {
  for (const scope of scopes()) {
    const root = scope === document ? document : scope;
    if (root.querySelector?.(`[${MARK}]`)) continue;

    const anchor = findAnchor(root);
    if (!anchor) continue;

    const button = makeButton(() => site.resolve(scope === document ? null : scope));
    if (anchor.node.dataset?.nitrateOrientation === "column") {
      button.classList.add("nitrate-stacked");
    }
    place(anchor.node, button, anchor.placement);
  }

  ensureFallback();
}

/**
 * When nothing matched anywhere, put a button in the corner so the feature
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

  // Worth saying out loud: it means both the selectors and the structural pass
  // came up empty, and knowing which site is most of the work of fixing it.
  console.info(
    `${LOG} nothing matched on ${site.id} — using the floating button. ` +
      `Tried: ${site.anchors.map((a) => (typeof a === "string" ? a : a.sel)).join(", ") || "(structural only)"}`,
  );

  const button = makeButton(() => location.href);
  button.classList.add("nitrate-floating");
  document.body?.appendChild(button);
}

/** Drops every button so the next scan rebuilds them in the new shape. */
function rerender() {
  for (const button of document.querySelectorAll(`[${MARK}]`)) button.remove();
  scheduleScan();
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

function watchSettings() {
  try {
    chrome.storage?.sync?.get({ labelMode: "auto" }, (stored) => {
      if (chrome.runtime?.lastError) return;
      if (stored?.labelMode && stored.labelMode !== labelMode) {
        labelMode = stored.labelMode;
        rerender();
      }
    });
    chrome.storage?.onChanged?.addListener((changes, area) => {
      if (area !== "sync" || !changes.labelMode) return;
      labelMode = changes.labelMode.newValue || "auto";
      rerender();
    });
  } catch {
    // No storage access is survivable — "auto" is the sensible default anyway.
  }
}

/**
 * Styling aims at "one of the site's own buttons", not at brand presence.
 *
 * The chip is a translucent grey rather than blurple, which reads correctly on
 * both a light and a dark page without having to detect which one this is; only
 * the mark keeps the brand colour. Sizes are matched per site because a 36px
 * YouTube pill next to Instagram's bare 24px icons would look wrong either way
 * round.
 */
function injectStyles() {
  const style = document.createElement("style");
  style.textContent = `
    .nitrate-send {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      gap: 6px;
      box-sizing: border-box;
      border: none;
      background: rgba(128,128,128,.16);
      color: inherit;
      cursor: pointer;
      vertical-align: middle;
      border-radius: 999px;
      height: 32px;
      padding: 0 12px;
      margin: 0 4px;
      /* Longhand on purpose: "inherit" is only valid as a whole value, so
         "font: 600 13px/1 inherit" silently drops the entire declaration. */
      font-family: inherit;
      font-weight: 600;
      font-size: 13px;
      line-height: 1;
      transition: background-color .15s, opacity .15s;
    }
    .nitrate-send:hover { background: rgba(128,128,128,.3); }
    .nitrate-send .nitrate-icon { width: 16px; height: 16px; color: #5865F2; flex: none; }
    .nitrate-send .nitrate-label { color: inherit; white-space: nowrap; }
    .nitrate-send.is-done { background: rgba(67,181,129,.25); }
    .nitrate-send.is-done .nitrate-icon { color: #43B581; }
    .nitrate-send.is-failed { background: rgba(237,66,69,.25); }
    .nitrate-send.is-failed .nitrate-icon { color: #ED4245; }

    /* No label means a circle, not a stubby pill. */
    .nitrate-send:not(:has(.nitrate-label)) { width: 32px; padding: 0; }

    /* A rail rather than a row: full width of the column, stacked under it. */
    .nitrate-send.nitrate-stacked { margin: 12px auto 0; display: flex; }

    /* YouTube's action row is 36px pills with 14px text. */
    .nitrate-send[data-site="youtube"] {
      height: 36px;
      padding: 0 14px;
      margin-left: 8px;
      font: 500 14px/1 "Roboto", Arial, sans-serif;
    }
    .nitrate-send[data-site="youtube"]:not(:has(.nitrate-label)) { width: 36px; padding: 0; }
    .nitrate-send[data-site="youtube"] .nitrate-icon { width: 18px; height: 18px; }

    /* Instagram's controls are bare outline icons on the page background. */
    .nitrate-send[data-site="instagram"] {
      background: none;
      height: 40px;
      width: 40px;
      padding: 0;
      margin: 0;
    }
    .nitrate-send[data-site="instagram"]:hover { background: none; opacity: .6; }
    .nitrate-send[data-site="instagram"] .nitrate-icon { width: 24px; height: 24px; }

    /* X packs its row tightly and sizes icons at 18px. */
    .nitrate-send[data-site="x"] {
      height: 34px;
      background: none;
      margin: 0;
    }
    .nitrate-send[data-site="x"]:not(:has(.nitrate-label)) { width: 34px; }
    .nitrate-send[data-site="x"]:hover { background: rgba(88,101,242,.14); }
    .nitrate-send[data-site="x"] .nitrate-icon { width: 18px; height: 18px; }

    /* Twitch uses square-ish corners and small, heavy text. */
    .nitrate-send[data-site="twitch"] {
      border-radius: 4px;
      height: 30px;
      font-size: 12px;
    }
    .nitrate-send[data-site="twitch"]:not(:has(.nitrate-label)) { width: 30px; }

    /* Reddit's action row is 32px pills with 12px text. */
    .nitrate-send[data-site="reddit"] { font-size: 12px; }

    .nitrate-send.nitrate-floating {
      position: fixed;
      right: 18px;
      bottom: 18px;
      z-index: 2147483000;
      width: auto;
      height: 36px;
      padding: 0 14px;
      background: #5865F2;
      color: #fff;
      font: 600 13px/1 system-ui, -apple-system, "Segoe UI", sans-serif;
      box-shadow: 0 6px 20px rgba(0,0,0,.35);
    }
    .nitrate-send.nitrate-floating .nitrate-icon { color: #fff; }
    .nitrate-send.nitrate-floating:hover { background: #6b76f5; }
  `;
  (document.head || document.documentElement).appendChild(style);
}

// ---------------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------------

if (site) {
  console.info(`${LOG} active on ${site.id}`);
  injectStyles();
  watchSettings();
  scheduleScan();
  observe();
} else {
  console.info(`${LOG} loaded, but ${location.hostname} isn't a supported site`);
}
