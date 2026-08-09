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

// How long the page gets to build its action row before the corner button is
// accepted as the answer.
const START = Date.now();
const FALLBACK_DELAY = 4000;

// Site navigation looks exactly like an action row to a structural search —
// Instagram's sidebar is a tall column of labelled icons, same as a reel's
// rail — so it's ruled out by role rather than left to lose on size.
const CHROME = "nav, header, aside, [role='navigation'], [role='banner']";

const SVG_NS = "http://www.w3.org/2000/svg";

/**
 * The mark, built as elements rather than parsed from a string.
 *
 * `innerHTML` would be shorter, but Mozilla's reviewer flags every assignment
 * to it — it can't tell a constant from something built out of page data. It's
 * also the one path a page's Trusted Types policy could interfere with. Neither
 * is worth arguing about for markup this small.
 *
 * Drawn at 24px with 2px strokes, matching these sites' own icon sets: a
 * heavier mark reads as a logo dropped into the row rather than as a control
 * belonging to it.
 */
function markSvg() {
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("class", "nitrate-icon");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");

  const strokes = document.createElementNS(SVG_NS, "g");
  for (const [name, value] of [
    ["stroke", "currentColor"],
    ["stroke-width", "2"],
    ["stroke-linecap", "round"],
    ["stroke-linejoin", "round"],
    ["fill", "none"],
  ]) {
    strokes.setAttribute(name, value);
  }

  for (const points of ["7,5 12,9 17,5", "7,19 12,15 17,19"]) {
    const chevron = document.createElementNS(SVG_NS, "polyline");
    chevron.setAttribute("points", points);
    strokes.append(chevron);
  }
  svg.append(strokes);

  const bar = document.createElementNS(SVG_NS, "rect");
  for (const [name, value] of [
    ["x", "6"],
    ["y", "11"],
    ["width", "12"],
    ["height", "2"],
    ["rx", "1"],
    ["fill", "currentColor"],
  ]) {
    bar.setAttribute(name, value);
  }
  svg.append(bar);

  return svg;
}

/**
 * Per-site strategy.
 *
 * `anchors` are tried in order until one matches; each is either a selector or
 * `{ sel, up, placement }`, where `up` climbs that many parents from the match
 * (handy when the only stable name belongs to something inside the row we
 * actually want).
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
    // Take the real pills' measurements instead of hardcoding them; see
    // `adoptStyle`.
    copyStyle: true,
    anchors: [
      "ytd-watch-metadata #top-level-buttons-computed",
      // The like/dislike pill is the one element in that row with a name that
      // has survived several redesigns, so its parent is the row itself.
      { sel: "ytd-watch-metadata ytd-segmented-like-dislike-button-view-model", up: 1 },
      { sel: "ytd-watch-metadata segmented-like-dislike-button-view-model", up: 1 },
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
    copyStyle: true,
    anchors: [
      // The clip page's row of actions. Every `data-a-target` that used to be
      // on these buttons is gone, and the only labelled icon on the whole page
      // is elsewhere, so the structural pass finds nothing either — which is
      // why the button was landing in the corner. This is the one hook left:
      // the link two levels up from it is the row itself.
      {
        sel: '[data-test-selector="clips-watch-full-button"]',
        up: 2,
        placement: "penultimate",
      },
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
    // An opened post draws the same row larger than the timeline does, so the
    // size is taken from whatever is beside it rather than fixed here.
    copyStyle: true,
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
    // Its posts are custom elements that hide their controls in shadow roots.
    deep: true,
    // Deliberately not the credit bar: that's the byline above the title, which
    // is where the button ended up looking like a stray banner. These all sit
    // in the vote/comment/share row along the bottom, where it belongs.
    anchors: [
      // Ordered by how specific they are. The share control is the one that
      // sits at the end of the action row on every layout — feed, opened post
      // and community page alike — which is why the button appeared in three
      // different places depending on where you were.
      { sel: "shreddit-post-share-button", placement: "after" },
      { sel: '[data-post-click-location="share"]', placement: "after" },
      { sel: 'button[aria-label="Share" i]', placement: "after" },
      { sel: '[data-post-click-location="comments-button"]', placement: "after" },
      { sel: 'button[aria-label*="comment" i]', placement: "after" },
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
      if (articles.length) return articles;
      // A reel has no article, and searching the whole page finds the left
      // navigation first — Home/Reels/Search/Profile is also a tall column of
      // labelled icons. `main` holds the reel and not the sidebar.
      return [document.querySelector("main") ?? document.body];
    },
    label: false,
    anchors: [],
    // Climbs from the rail rather than searching the scope, because on a reel
    // feed the scope is `main` and holds several reels at once — searching it
    // would send whichever one happens to be first in the document, not the one
    // whose button was pressed.
    resolve: (scope, node) => {
      let element = node ?? scope;
      for (let depth = 0; element && depth < 8; depth += 1) {
        const link = element.querySelector?.('a[href*="/p/"], a[href*="/reel/"]');
        if (link) return new URL(link.getAttribute("href"), location.origin).href;
        element = element.parentElement;
      }
      return location.href;
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

  button.append(markSvg());
  if (wantsLabel()) {
    const label = document.createElement("span");
    label.className = "nitrate-label";
    label.textContent = "Nitrate";
    button.append(label);
  }

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
 * Finds action rows by shape instead of by name.
 *
 * Every one of these sites groups its controls into one small box holding
 * several labelled icons, so an element containing at least three of them is a
 * row — or, on a reel, the vertical rail. That survives a rename, which is what
 * the selectors above keep failing to do.
 *
 * Returns every such group rather than the best one: a reel feed keeps several
 * reels alive at once, each with its own rail, and stopping at the first meant
 * the button stayed behind on the reel you'd just scrolled past.
 */
function findActionClusters(root) {
  const scope = root === document ? document.body : root;
  if (!scope?.querySelectorAll) return [];

  const icons = Array.from(
    scope.querySelectorAll("svg[aria-label], svg[role='img']"),
  ).filter((icon) => {
    if (icon.closest?.(CHROME)) return false;

    // Ignore anything small. A post's own controls are drawn at 20-24px, while
    // the like button on each comment is nearer 12px — and a comment thread
    // holds far more of them, so the smallest cluster of three was often three
    // comments rather than the post. That's how the button ended up down in the
    // replies on Instagram.
    const box = icon.getBoundingClientRect?.();
    return !box || box.width === 0 || box.width >= 18;
  });
  if (icons.length < 3) return [];

  const counts = new Map();
  for (const icon of icons) {
    let node = icon.parentElement;
    for (let depth = 0; node && node !== scope.parentElement && depth < 10; depth += 1) {
      counts.set(node, (counts.get(node) || 0) + 1);
      node = node.parentElement;
    }
  }

  const candidates = [];
  for (const [node, count] of counts) {
    if (count < 3) continue;
    const rect = node.getBoundingClientRect?.();
    if (!rect || !rect.width || !rect.height) continue;
    candidates.push({ node, rect, area: rect.width * rect.height });
  }
  // Smallest first, so the row itself is preferred over everything wrapping it.
  candidates.sort((a, b) => a.area - b.area);

  const chosen = [];
  for (const candidate of candidates) {
    // One per group: anything containing an already-chosen row, or contained by
    // one, is the same set of buttons seen from a different level.
    const overlaps = chosen.some(
      (other) => other.node.contains(candidate.node) || candidate.node.contains(other.node),
    );
    if (overlaps) continue;
    // A rail is taller than it is wide; the button stacks rather than sits beside.
    candidate.node.dataset.nitrateOrientation =
      candidate.rect.height > candidate.rect.width * 1.5 ? "column" : "row";
    chosen.push(candidate);
  }

  return chosen.map((c) => c.node);
}

/**
 * `querySelector`, but able to see inside shadow roots.
 *
 * Reddit builds its posts from custom elements that keep their innards in a
 * shadow root, so `shreddit-post-share-button` is genuinely present and
 * genuinely invisible to an ordinary query. Every Reddit anchor missed for that
 * reason, which is why the button kept ending up wherever the structural search
 * happened to land — usually the post header.
 */
function deepQuery(root, selector) {
  if (!root?.querySelectorAll) return null;

  const direct = root.querySelector(selector);
  if (direct) return direct;

  // Breadth-first through open shadow roots. Closed ones stay invisible, but
  // nothing here uses them.
  const queue = Array.from(root.querySelectorAll("*"));
  while (queue.length) {
    const node = queue.shift();
    const shadow = node.shadowRoot;
    if (!shadow) continue;

    const found = shadow.querySelector(selector);
    if (found) return found;
    queue.push(...shadow.querySelectorAll("*"));
  }

  return null;
}

function findAnchor(root) {
  for (const entry of site.anchors) {
    const { sel, up = 0, placement } = typeof entry === "string" ? { sel: entry } : entry;
    let match = null;
    try {
      match = (site.deep ? deepQuery(root, sel) : root.querySelector?.(sel)) ?? null;
    } catch {
      // `:has()` and custom elements aren't universally supported; a bad
      // selector shouldn't take the whole scan down.
      match = null;
    }
    if (!match) continue;

    let node = match;
    for (let step = 0; step < up && node; step += 1) node = node.parentElement;
    if (node) return { node, placement: placement ?? "append" };
  }

  return null;
}

/**
 * Copies a neighbouring button's own computed style onto ours.
 *
 * Every hardcoded guess at YouTube's pill — height, radius, padding, the exact
 * grey — is a guess that goes stale the next time they adjust it, and it can't
 * be right for both themes at once anyway. Reading it off the Share button
 * sitting next to us is exact by construction, and stays exact.
 *
 * The CSS rules remain as the fallback for when no reference button is found.
 */
const COPIED = [
  "height",
  "border-radius",
  "font-family",
  "font-size",
  "font-weight",
  "line-height",
  "letter-spacing",
  "color",
];

/**
 * Padding is copied only when there is some.
 *
 * Twitch's buttons compute to zero — their spacing lives on an inner element —
 * and copying that would print the label hard against both edges. A zero here
 * means "the padding is somewhere else", not "there isn't any", so the site
 * rule's own value stands.
 */
const COPIED_IF_SET = ["padding-left", "padding-right"];

function adoptStyle(button, row) {
  let read;
  try {
    read = window.getComputedStyle;
    if (typeof read !== "function") return;
  } catch {
    return;
  }

  // Links count as well as buttons: Twitch's "Watch Full Video" is an anchor
  // styled identically to the buttons beside it.
  const candidates = Array.from(row.querySelectorAll?.("button, a") ?? []).filter(
    (candidate) => !candidate.hasAttribute(MARK),
  );
  if (!candidates.length) return;

  // Not just any button in the row. The like/dislike pair is a single segmented
  // control, so each half computes a border-radius rounded on one side only —
  // "20px 0px 0px 20px" — and copying that would give a button flat down one
  // edge. A uniform radius has no spaces in it, which rules them both out.
  // Preferring one with a label then gets the padding and font from a button
  // shaped like ours rather than from the round overflow menu.
  const uniform = (candidate) => {
    const radius = window.getComputedStyle(candidate).borderRadius || "";
    return !radius.trim().includes(" ");
  };
  const reference =
    candidates.find((c) => c.textContent?.trim() && uniform(c)) ??
    candidates.find(uniform) ??
    candidates[0];

  const style = window.getComputedStyle(reference);
  for (const property of COPIED) {
    const value = style.getPropertyValue(property);
    if (value) button.style.setProperty(property, value);
  }
  for (const property of COPIED_IF_SET) {
    const value = style.getPropertyValue(property);
    if (value && parseFloat(value) > 0) button.style.setProperty(property, value);
  }

  // Match the neighbouring icon's size.
  //
  // X draws a bigger row on an opened post than in the timeline, so any fixed
  // size is wrong in one of the two places — which is what made the mark look
  // undersized once a post was opened. Measuring a sibling gets both, and any
  // future resize, without knowing which view this is.
  const mark = button.querySelector(".nitrate-icon");
  const sibling = reference.querySelector("svg");
  if (mark && sibling) {
    const rect = sibling.getBoundingClientRect?.();
    if (rect?.width > 0 && rect?.height > 0) {
      mark.style.width = `${rect.width}px`;
      mark.style.height = `${rect.height}px`;
    }
  }

  // Only the reference's own background, never an ancestor's. X's action
  // buttons are transparent on purpose; climbing past them finds the page
  // behind and paints the button with it, which is either invisible or a solid
  // block depending on the theme. Transparent here means transparent.
  const background = style.backgroundColor;
  if (background && !/^(transparent|rgba\(0, 0, 0, 0\))$/.test(background)) {
    button.style.setProperty("background-color", background);
  }

  button.dataset.nitrateAdopted = "1";
}

function place(anchor, button, placement) {
  if (placement === "after") anchor.after(button);
  else if (placement === "prepend") anchor.prepend(button);
  else if (placement === "penultimate" && anchor.lastElementChild) {
    // Second from the end: these rows finish with an overflow menu, and sitting
    // past it reads as belonging to something else.
    anchor.insertBefore(button, anchor.lastElementChild);
  } else anchor.appendChild(button);
}

function scopes() {
  if (typeof site.scope === "function") return site.scope();
  if (site.scope) return Array.from(document.querySelectorAll(site.scope));
  return [document];
}

function scan() {
  // A button placed before the page finished rendering can land in the wrong
  // group, and because a placed button stops the search it would stay there for
  // the life of the tab. Dropping the misplaced one lets the next pass — by
  // which time the real row exists — put it where it belongs.
  for (const button of document.querySelectorAll(`[${MARK}]:not(.nitrate-floating)`)) {
    if (button.closest?.(CHROME)) button.remove();
  }

  for (const scope of scopes()) {
    const root = scope === document ? document : scope;
    const anchor = findAnchor(root);

    if (anchor) {
      // Re-home a button the page has moved on from.
      //
      // These are all single-page apps: navigating swaps the content without
      // reloading, and a button placed against the old page either sits in the
      // wrong row or hangs off a detached node. It looked like "the button
      // doesn't appear until you refresh" on YouTube, and like a stray corner
      // button after going back on Reddit. Anything no longer sitting in the
      // anchor we'd choose now is dropped so this pass can place it properly.
      for (const stray of root.querySelectorAll?.(`[${MARK}]:not(.nitrate-floating)`) ?? []) {
        const home = anchor.placement === "after" ? anchor.node.parentElement : anchor.node;
        if (!stray.isConnected || stray.parentElement !== home) stray.remove();
      }

      // The floating button has to be excluded here. It lives on `body`, so on
      // a page with no scope it counts as "already done" and every later scan
      // short-circuits — meaning if the fallback ever appears before the action
      // row has rendered, it wins permanently and the inline button never
      // arrives. That's timing-dependent, which is why it came and went.
      if (root.querySelector?.(`[${MARK}]:not(.nitrate-floating)`)) {
        // Re-read on every pass: the theme switch changes the pills without
        // replacing them, and a button placed under the old colours would
        // otherwise keep them.
        if (site.copyStyle) refreshStyles(root);
        continue;
      }
      attach(anchor.node, scope, anchor.placement);
      continue;
    }

    // No named anchor, so fall back to shape. Each group is handled on its own
    // — checking the scope as a whole would leave every reel after the first
    // one bare, since they share `main`.
    for (const cluster of findActionClusters(root)) {
      if (cluster.querySelector?.(`[${MARK}]`)) continue;
      attach(cluster, scope);
    }
  }

  ensureFallback();
}

/** Builds a button for one anchor and puts it in place. */
function attach(node, scope, placement) {
  const button = makeButton(() => site.resolve(scope === document ? null : scope, node));

  // On a rail the button goes at the top, above the like control. Appending
  // would drop it under the account avatar at the foot of the column, which is
  // both the least reachable spot and visually part of a different group.
  let where = placement ?? "append";
  if (node.dataset?.nitrateOrientation === "column") {
    button.classList.add("nitrate-stacked");
    where = "prepend";
  }

  place(node, button, where);
  // After placing, so the reference button is measured in its final context.
  if (site.copyStyle) adoptStyle(button, button.parentElement ?? node);
}

function refreshStyles(root) {
  const buttons = root.querySelectorAll?.(`[${MARK}]:not(.nitrate-floating)`) ?? [];
  for (const button of buttons) {
    if (button.parentElement) adoptStyle(button, button.parentElement);
  }
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

  // These pages build their action rows well after first paint, so an
  // immediate fallback would routinely beat the real thing. Give the page a
  // few seconds to finish before concluding there's nothing to attach to.
  if (Date.now() - START < FALLBACK_DELAY) {
    scheduleScan();
    return;
  }

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
      position: relative;
      /* Never shrink. These rows are flex containers whose children are free to
         compress, so an extra button gets squeezed and its background is cut
         short — the label overflows past a pill that stops early. */
      flex: 0 0 auto;
      white-space: nowrap;
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

    /* A rail rather than a row: sits above the like control, spaced like one. */
    .nitrate-send.nitrate-stacked { display: flex; margin: 0 auto 16px; }

    /*
     * YouTube: not "close to" its pills but the same pills.
     *
     * The colours come from YouTube's own custom properties rather than fixed
     * values, so the button follows the theme switch for free — including the
     * pages that force one theme regardless of the account setting. The
     * fallbacks are the dark-theme values, since that's what these are set to
     * when the variables have been renamed out from under us.
     */
    .nitrate-send[data-site="youtube"] {
      height: 40px;
      padding: 0 16px;
      margin: 0 0 0 8px;
      gap: 0;
      border-radius: 20px;
      background: var(--yt-spec-badge-chip-background, rgba(255,255,255,.1));
      color: var(--yt-spec-text-primary, #f1f1f1);
      font-family: "Roboto", "Arial", sans-serif;
      font-size: 14px;
      font-weight: 500;
      letter-spacing: normal;
    }
    /*
     * The rim light. YouTube draws it with a <yt-light-shape> child element
     * whose ::before carries this gradient — a white wash across the top that's
     * gone three-quarters of the way down. It reads as a lit top edge, and its
     * absence is why the button looked flat next to Share.
     */
    .nitrate-send[data-site="youtube"]::before {
      content: "";
      position: absolute;
      inset: 0;
      border-radius: inherit;
      background-image: linear-gradient(rgba(255,255,255,.1), rgba(0,0,0,0) 75%);
      pointer-events: none;
    }
    .nitrate-send[data-site="youtube"]:hover {
      background: var(--yt-spec-button-chip-background-hover, rgba(255,255,255,.2));
    }
    /*
     * An adopted background is an inline style, which no rule here can outrank
     * — so the hover lightens what's already there instead of replacing it.
     * That also means it works whichever theme the copied colour came from.
     */
    .nitrate-send[data-nitrate-adopted]:hover { filter: brightness(1.4); }
    .nitrate-send[data-site="youtube"]:not(:has(.nitrate-label)) { width: 40px; padding: 0; }
    .nitrate-send[data-site="youtube"]:not(:has(.nitrate-label)) .nitrate-icon { margin: 0; }
    /*
     * Monochrome, like Share and Save beside it — the word carries the brand.
     * The negative left margin is YouTube's: it pulls the icon back out of the
     * 16px padding so a leading icon sits at 10px while the text keeps its 16.
     */
    .nitrate-send[data-site="youtube"] .nitrate-icon {
      width: 24px;
      height: 24px;
      color: currentColor;
      margin: 0 6px 0 -6px;
    }
    .nitrate-send[data-site="youtube"].is-done .nitrate-icon { color: #43B581; }
    .nitrate-send[data-site="youtube"].is-failed .nitrate-icon { color: #ED4245; }

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

    /*
     * X packs its row tightly, and the row is only as tall as its tallest
     * child. A fixed 34px button was taller than everything beside it, so it
     * stretched the whole action row — which on a timeline pushed every tweet
     * below it down and made the feed jump as posts loaded. Sized by its
     * padding instead, so it can never be the tallest thing in the row.
     */
    .nitrate-send[data-site="x"] {
      height: auto;
      align-self: center;
      padding: 4px;
      background: none;
      margin: 0;
    }
    .nitrate-send[data-site="x"]:not(:has(.nitrate-label)) { width: auto; }
    .nitrate-send[data-site="x"]:hover { background: rgba(88,101,242,.14); }
    .nitrate-send[data-site="x"] .nitrate-icon { width: 18px; height: 18px; }

    /* Twitch's clip actions are 32px fully round pills with 14px Inter. The
       adoption pass overwrites most of this from the Share button itself; these
       are the values it lands on, kept for when there's nothing to copy. */
    .nitrate-send[data-site="twitch"] {
      height: 32px;
      border-radius: 9000px;
      padding: 0 12px;
      font-family: Inter, "Helvetica Neue", Helvetica, Arial, sans-serif;
      font-size: 14px;
      font-weight: 600;
    }
    .nitrate-send[data-site="twitch"]:not(:has(.nitrate-label)) { width: 32px; padding: 0; }

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
