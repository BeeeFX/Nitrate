// Runs the extension's scripts against a stub DOM before they're packaged.
//
// A parse check isn't enough: the bug that shipped in 0.4.0 was a temporal dead
// zone — startup called a function that read a `let` declared further down, so
// the file parsed perfectly and threw on the first line it executed. Nothing
// short of running it would have caught that.
//
// Two passes:
//   1. An empty page, where nothing matches, so every site walks its whole
//      fallback path and must end up on the floating button.
//   2. A page shaped like a reel feed, where the script has to place one button
//      per rail — the case that kept regressing, because a single placed button
//      used to end the search for the entire scope.
//
// Then what a button does once it's pressed — whether it waits to hear back
// from the app before claiming success — and the same for the toolbar and the
// right-click menu, which live in the background script.

import { readFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const MARK = "data-nitrate-button";

const HOSTS = [
  ["youtube", "www.youtube.com", "/watch?v=abc"],
  ["twitch", "www.twitch.tv", "/videos/1"],
  ["x", "x.com", "/user/status/1"],
  ["reddit", "www.reddit.com", "/r/a/comments/b/c/"],
  ["instagram", "www.instagram.com", "/reels/abc/"],
  ["unsupported", "example.com", "/"],
];

// ---------------------------------------------------------------------------
// A DOM small enough to read and real enough to place a button in.
//
// The selector matcher only understands the handful of selectors the content
// script actually uses. That's deliberate: a general one would be a browser,
// and jsdom is no help here — it has no layout, so getBoundingClientRect comes
// back as zeroes and every candidate row would be discarded before it was
// considered.
// ---------------------------------------------------------------------------

/**
 * Just enough of EventTarget to press a button and blur a window.
 *
 * Listeners are kept in a set per type, as a browser does, so adding the same
 * function twice is one listener — and a count that climbs with every click
 * means fresh closures are piling up.
 */
function eventTarget(target) {
  const listeners = new Map();
  target.addEventListener = (type, listener) => {
    if (!listeners.has(type)) listeners.set(type, new Set());
    listeners.get(type).add(listener);
  };
  target.removeEventListener = (type, listener) => {
    listeners.get(type)?.delete(listener);
  };
  target.listenerCount = (type) => listeners.get(type)?.size ?? 0;
  target.dispatch = (type) => {
    const event = { type, preventDefault() {}, stopPropagation() {} };
    for (const listener of [...(listeners.get(type) ?? [])]) listener(event);
  };
  return target;
}

function makeElement(tag = "div", attrs = {}) {
  const element = eventTarget({
    tagName: tag.toUpperCase(),
    attrs: { ...attrs },
    dataset: {},
    style: { setProperty() {}, removeProperty() {} },
    children: [],
    parentElement: null,
    innerHTML: "",
    textContent: "",
    className: "",
    rect: { width: 0, height: 0, top: 0, left: 0 },
    classList: {
      add(name) {
        element.className = `${element.className} ${name}`.trim();
      },
      remove(...names) {
        element.className = element.className
          .split(" ")
          .filter((c) => c && !names.includes(c))
          .join(" ");
      },
      contains: (name) => element.className.split(" ").includes(name),
    },
    setAttribute(name, value) {
      element.attrs[name] = String(value);
    },
    getAttribute: (name) => element.attrs[name] ?? null,
    hasAttribute: (name) => name in element.attrs,
    appendChild(child) {
      child.parentElement = element;
      element.children.push(child);
      return child;
    },
    append(...kids) {
      for (const child of kids) {
        child.parentElement = element;
        element.children.push(child);
      }
    },
    prepend(child) {
      child.parentElement = element;
      element.children.unshift(child);
      return child;
    },
    insertBefore(child, reference) {
      const at = element.children.indexOf(reference);
      child.parentElement = element;
      element.children.splice(at === -1 ? element.children.length : at, 0, child);
      return child;
    },
    get lastElementChild() {
      return element.children[element.children.length - 1] ?? null;
    },
    after(sibling) {
      const parent = element.parentElement;
      if (!parent) return;
      sibling.parentElement = parent;
      parent.children.splice(parent.children.indexOf(element) + 1, 0, sibling);
    },
    remove() {
      const parent = element.parentElement;
      if (!parent) return;
      parent.children.splice(parent.children.indexOf(element), 1);
      element.parentElement = null;
    },
    contains(other) {
      for (let node = other; node; node = node.parentElement) {
        if (node === element) return true;
      }
      return false;
    },
    closest(selector) {
      for (let node = element; node; node = node.parentElement) {
        if (matches(node, selector)) return node;
      }
      return null;
    },
    // Present so the shadow-piercing search has something to ask about. No
    // fixture uses one, which is itself worth noting: the Reddit fix this
    // supports can't be proven here.
    shadowRoot: null,
    isConnected: true,
    querySelector: (selector) => descendants(element).find((n) => matches(n, selector)) ?? null,
    querySelectorAll: (selector) => descendants(element).filter((n) => matches(n, selector)),
    getBoundingClientRect: () => element.rect,
  });
  return element;
}

function descendants(node, out = []) {
  for (const child of node.children) {
    out.push(child);
    descendants(child, out);
  }
  return out;
}

/** Handles only the selector forms the content script uses. */
function matches(node, selector) {
  return selector
    .split(",")
    .map((part) => part.trim())
    .some((part) => matchesOne(node, part));
}

function matchesOne(node, selector) {
  if (selector.startsWith(`[${MARK}]`)) {
    if (!node.hasAttribute(MARK)) return false;
    if (selector.includes(":not(.nitrate-floating)")) {
      return !node.classList.contains("nitrate-floating");
    }
    return true;
  }
  if (selector === "svg[aria-label]") {
    return node.tagName === "SVG" && node.hasAttribute("aria-label");
  }
  if (selector === "svg[role='img']") {
    return node.tagName === "SVG" && node.getAttribute("role") === "img";
  }
  if (selector === "button") return node.tagName === "BUTTON";
  if (selector === "main") return node.tagName === "MAIN";
  if (selector === "article") return node.tagName === "ARTICLE";
  if (selector === "video") return node.tagName === "VIDEO";
  if (selector === "nav" || selector === "header" || selector === "aside") {
    return node.tagName === selector.toUpperCase();
  }
  if (selector.startsWith("[role=")) {
    const wanted = selector.slice(7, -2);
    return node.getAttribute("role") === wanted;
  }
  // An [attr="value"] selector, optionally qualified by tag and optionally
  // case-insensitive — how both Twitch rows are anchored. The VOD's Share
  // button carries no data attribute at all, so `button[aria-label="Share" i]`
  // is the only thing left to name it by.
  const attribute = selector.match(/^([a-z]+)?\[([\w-]+)="([^"]+)"( i)?\]$/i);
  if (attribute) {
    const [, tag, name, value, insensitive] = attribute;
    if (tag && node.tagName !== tag.toUpperCase()) return false;
    const actual = node.getAttribute(name);
    if (actual === null) return false;
    return insensitive ? actual.toLowerCase() === value.toLowerCase() : actual === value;
  }
  // Anything else is a site-specific selector that this fixture doesn't model,
  // which is the point — the structural pass is what's under test.
  return false;
}

/** A reel feed: a sidebar to be ignored, and two reels each with their own rail. */
function reelFixture(body) {
  const nav = makeElement("nav");
  nav.rect = { width: 220, height: 900, top: 0, left: 0 };
  for (const label of ["Home", "Reels", "Search", "Profile"]) {
    const icon = makeElement("svg", { "aria-label": label });
    nav.appendChild(icon);
  }
  body.appendChild(nav);

  const main = makeElement("main");
  main.rect = { width: 1200, height: 1800, top: 0, left: 220 };
  body.appendChild(main);

  const rails = [];
  for (let index = 0; index < 2; index += 1) {
    const reel = makeElement("div");
    reel.rect = { width: 700, height: 900, top: index * 900, left: 500 };
    const link = makeElement("a", { href: `/reel/${index}/` });
    reel.appendChild(link);

    const rail = makeElement("div");
    rail.rect = { width: 60, height: 600, top: index * 900 + 150, left: 1150 };
    for (const label of ["Like", "Comment", "Share", "Save"]) {
      // Wrapped, as Instagram"s own are: the structural search only counts an
      // icon that belongs to something clickable, so a bare svg here would
      // model a decoration rather than a control.
      const control = makeElement("div", { role: "button" });
      control.appendChild(makeElement("svg", { "aria-label": label }));
      rail.appendChild(control);
    }
    reel.appendChild(rail);
    main.appendChild(reel);
    rails.push(rail);
  }

  return rails;
}

/**
 * A Twitch clip page's row of actions.
 *
 * Each control sits in a wrapper div, and the only named element is the
 * "Watch Full Video" link — so the anchor has to climb two levels to reach the
 * row, and land second from the end rather than past the overflow menu.
 */
function clipFixture(body) {
  const row = makeElement("div");
  row.rect = { width: 400, height: 32, top: 500, left: 500 };

  const wrap = (child) => {
    const wrapper = makeElement("div");
    wrapper.appendChild(child);
    row.appendChild(wrapper);
    return wrapper;
  };

  const watch = makeElement("a", { "data-test-selector": "clips-watch-full-button" });
  watch.textContent = "Watch Full Video";
  wrap(watch);

  for (const text of ["Edit", "Share"]) {
    const button = makeElement("button");
    button.textContent = text;
    wrap(button);
  }

  const overflow = makeElement("button", { "aria-label": "Clip Options" });
  wrap(overflow);

  body.appendChild(row);
  return row;
}

/**
 * A Twitch VOD's action row.
 *
 * Nothing in here carries a `data-a-target` or a `data-test-selector` — the
 * real page has none on these buttons — so the Share button's label is the
 * only hook, and it sits under four single-child wrappers rather than in the
 * row itself. The row holds two controls, one short of the three the
 * structural pass needs, so nothing catches this if the anchor misses: it went
 * straight to the corner, which is the bug.
 */
function vodFixture(body) {
  const row = makeElement("div");
  row.rect = { width: 129, height: 32, top: 284, left: 800 };

  const share = makeElement("button", { "aria-label": "Share" });
  share.textContent = "Share";
  // Buried, as the real one is. The depth is deliberately not the number the
  // anchor climbs — it stops at the first ancestor holding a row, whatever
  // that turns out to be.
  let wrapped = share;
  for (let depth = 0; depth < 4; depth += 1) {
    const wrapper = makeElement("div");
    wrapper.appendChild(wrapped);
    wrapped = wrapper;
  }
  row.appendChild(wrapped);

  const overflow = makeElement("button", { "aria-label": "More" });
  row.appendChild(overflow);

  body.appendChild(row);
  return row;
}

/**
 * An Instagram DM thread.
 *
 * Its header (call, video, info) and its composer (mic, photo, sticker) are
 * both rows of labelled icons inside buttons — the exact shape the structural
 * pass hunts for — so it used to hang a button off each of them, pointing at
 * the conversation rather than at any video. With `withViewer`, a shared clip
 * has been opened into the modal, which is the one case there's something to
 * send.
 */
function dmFixture(body, withViewer) {
  const main = makeElement("main");
  main.rect = { width: 1100, height: 900, top: 0, left: 220 };

  const iconRow = (labels, rect) => {
    const row = makeElement("div");
    row.rect = rect;
    for (const label of labels) {
      const control = makeElement("button");
      control.appendChild(makeElement("svg", { "aria-label": label }));
      row.appendChild(control);
    }
    main.appendChild(row);
  };

  iconRow(["Audio call", "Video call", "Conversation information"], {
    width: 300, height: 44, top: 0, left: 800,
  });
  iconRow(["Voice clip", "Add photo or video", "Choose a sticker"], {
    width: 300, height: 44, top: 850, left: 800,
  });
  body.appendChild(main);

  if (!withViewer) return { main, viewer: null };

  const viewer = makeElement("div", { role: "dialog" });
  viewer.rect = { width: 900, height: 800, top: 40, left: 300 };
  viewer.appendChild(makeElement("video"));
  viewer.appendChild(makeElement("a", { href: "/reel/xyz/" }));

  const actions = makeElement("div");
  actions.rect = { width: 200, height: 40, top: 780, left: 320 };
  for (const label of ["Like", "Comment", "Share"]) {
    const control = makeElement("button");
    control.appendChild(makeElement("svg", { "aria-label": label }));
    actions.appendChild(control);
  }
  viewer.appendChild(actions);
  body.appendChild(viewer);

  return { main, viewer };
}

/** A live channel page: the same share button a VOD has, nothing recorded. */
function liveFixture(body) {
  const row = makeElement("div");
  row.rect = { width: 400, height: 32, top: 500, left: 500 };

  const share = makeElement("button", { "data-a-target": "share-button" });
  share.textContent = "Share";
  row.appendChild(share);

  body.appendChild(row);
  return row;
}

/**
 * The page-level objects a send touches.
 *
 * Every assignment to `window.location.href` is kept, which is how a check
 * tells a send from a click that did something else; so is every
 * `window.open`. The document starts visible, and a check can hide it.
 */
function makePage(documentProps = {}) {
  const navigations = [];
  const opened = [];
  const window = eventTarget({
    location: {
      get href() {
        return navigations.at(-1) ?? "";
      },
      set href(value) {
        navigations.push(value);
      },
    },
    open: (...args) => {
      opened.push(args);
      return null;
    },
    // Deliberately useless: the style-adoption path has to cope with a
    // reference button it can't measure.
    getComputedStyle: () => ({
      getPropertyValue: () => "",
      backgroundColor: "",
      borderRadius: "",
    }),
  });
  const document = eventTarget({ visibilityState: "visible", ...documentProps });
  return { window, document, navigations, opened };
}

function makeSandbox(hostname, pathname, fixture) {
  const body = makeElement("body");
  const head = makeElement("head");
  const documentElement = makeElement("html");
  documentElement.appendChild(head);
  documentElement.appendChild(body);

  const extras = fixture ? fixture(body) : null;
  const logs = [];
  const { window, document, navigations, opened } = makePage({
    body,
    head,
    documentElement,
    createElement: (tag) => makeElement(tag),
    createElementNS: (_ns, tag) => makeElement(tag),
    querySelector: (selector) => body.querySelector(selector),
    querySelectorAll: (selector) => body.querySelectorAll(selector),
    getElementById: () => makeElement(),
  });

  const observers = [];

  return {
    logs,
    extras,
    observers,
    navigations,
    opened,
    sandbox: {
      document,
      location: {
        hostname,
        pathname,
        href: `https://${hostname}${pathname}`,
        origin: `https://${hostname}`,
      },
      window,
      console: {
        info: (...args) => logs.push(args.join(" ")),
        warn: (...args) => logs.push(args.join(" ")),
        error: (...args) => logs.push(args.join(" ")),
        log: (...args) => logs.push(args.join(" ")),
      },
      chrome: {
        runtime: { lastError: null },
        storage: {
          sync: { get: (_defaults, cb) => cb?.({}), set: (_v, cb) => cb?.() },
          onChanged: { addListener() {} },
        },
      },
      // Kept rather than ignored, so a check can play a page that never stops
      // changing.
      MutationObserver: class {
        constructor(callback) {
          observers.push(callback);
        }
        observe() {}
        disconnect() {}
      },
      setTimeout,
      clearTimeout,
      URL,
    },
  };
}

// Long enough to clear the 400ms scan debounce *and* the 4s grace period the
// script gives a page before falling back to the corner button — otherwise the
// fallback path, which is the one an empty page always ends on, never runs.
const settle = () => new Promise((r) => setTimeout(r, 5000));

/** Starts a script in a fresh sandbox, without waiting for it to finish. */
async function load(file, hostname, pathname, fixture) {
  const code = await readFile(join(root, "extension", "src", file), "utf8");
  const context = makeSandbox(hostname, pathname, fixture);
  vm.createContext(context.sandbox);
  new vm.Script(code, { filename: file }).runInContext(context.sandbox);
  return context;
}

async function run(file, hostname, pathname, fixture, { busy = false } = {}) {
  const context = await load(file, hostname, pathname, fixture);
  // A page that never goes quiet, the way chat replay on a Twitch VOD doesn't:
  // a mutation every 100ms, far inside the scan's 400ms debounce.
  const churn = busy
    ? setInterval(() => context.observers.forEach((callback) => callback([])), 100)
    : null;
  await settle();
  clearInterval(churn);
  return context;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function expect(condition, message) {
  if (!condition) throw new Error(message);
}

/** Polls until `probe` finds something, rather than guessing a delay. */
async function until(probe, what, timeout = 3000) {
  for (const start = Date.now(); Date.now() - start < timeout; await sleep(25)) {
    const found = probe();
    if (found) return found;
  }
  throw new Error(`gave up waiting for ${what}`);
}

/**
 * Reads a timing constant out of the script under test.
 *
 * Copying the number in here would let the two drift apart, and a check that
 * waits less than the script does fails for a reason that has nothing to do
 * with the code.
 */
async function constant(file, name) {
  const code = await readFile(join(root, "extension", "src", file), "utf8");
  const match = code.match(new RegExp(`const ${name} = (\\d+);`));
  if (!match) throw new Error(`${file} no longer declares ${name}`);
  return Number(match[1]);
}

// ---------------------------------------------------------------------------
// Pressing the button.
//
// A send only counts once the app has answered, and the only answer a page can
// see is its window coming forward — to the page, a blur. Before that, the
// button turned green whether or not anything was installed to hear it.
// ---------------------------------------------------------------------------

const RELEASES = "https://github.com/BeeeFX/Nitrate/releases/latest";
const STATES = ["is-sending", "is-done", "is-failed", "is-unanswered"];

const stateOf = (button) => STATES.find((name) => button.classList.contains(name)) ?? "at rest";

/** A clip page with its inline button in place, ready to be pressed. */
async function clipPage() {
  const page = await load("content.js", "www.twitch.tv", "/ludwig/clip/Something", clipFixture);
  const button = await until(
    () => page.extras.children.find((child) => child.hasAttribute(MARK)),
    "the inline button",
  );
  return { ...page, button, window: page.sandbox.window, document: page.sandbox.document };
}

const PRESSING = {
  async "button: a blur after sending counts as the app answering"() {
    const { button, window, document, navigations } = await clipPage();

    // Twice, because a second press mustn't leave the first one's listeners
    // behind — each would flip the button on the same blur.
    button.dispatch("click");
    button.dispatch("click");
    expect(
      navigations.length === 2 && navigations.every((link) => link.startsWith("nitrate://add?url=")),
      `expected two sends, got ${JSON.stringify(navigations)}`,
    );
    expect(stateOf(button) === "is-sending", `straight after the click it was ${stateOf(button)}`);
    expect(
      window.listenerCount("blur") === 1 && document.listenerCount("visibilitychange") === 1,
      `two presses left ${window.listenerCount("blur")} blur listener(s) behind`,
    );

    window.dispatch("blur");
    expect(stateOf(button) === "is-done", `after the blur it was ${stateOf(button)}`);
    expect(window.listenerCount("blur") === 0, "still listening after the answer arrived");
  },

  async "button: silence reads as unanswered, and the next press offers the download"() {
    const wait = await constant("content.js", "ANSWER_WINDOW");
    const { button, window, navigations, opened } = await clipPage();

    button.dispatch("click");
    await sleep(wait + 150);
    expect(stateOf(button) === "is-unanswered", `after ${wait}ms of silence it was ${stateOf(button)}`);
    expect(
      /didn't answer/.test(button.getAttribute("aria-label")),
      `its name doesn't say what happened: ${button.getAttribute("aria-label")}`,
    );

    button.dispatch("click");
    expect(navigations.length === 1, "the second press sent the link again");
    expect(
      opened.length === 1 && opened[0][0] === RELEASES && opened[0][2] === "noopener",
      `wanted the releases page opened, got ${JSON.stringify(opened)}`,
    );
    expect(stateOf(button) === "at rest", `after offering the download it was ${stateOf(button)}`);
    // Opening a tab hides this one, which would otherwise read as an answer.
    expect(window.listenerCount("blur") === 0, "still listening after offering the download");
  },

  async "button: an answer after the verdict still counts"() {
    const wait = await constant("content.js", "ANSWER_WINDOW");
    const { button, document } = await clipPage();

    button.dispatch("click");
    await sleep(wait + 150);
    expect(stateOf(button) === "is-unanswered", `after ${wait}ms of silence it was ${stateOf(button)}`);

    // Hidden rather than blurred, so both routes are covered.
    document.visibilityState = "hidden";
    document.dispatch("visibilitychange");
    expect(stateOf(button) === "is-done", `a late answer left it ${stateOf(button)}`);
  },
};

// ---------------------------------------------------------------------------
// The toolbar and the right-click menu.
//
// background.js and shared.js are ES modules, which `vm` only runs behind an
// experimental flag. So they're joined into one script with the import line
// and the `export` keywords taken out; nothing else is touched, and the names
// don't collide.
// ---------------------------------------------------------------------------

const TAB = { id: 7, url: "https://www.youtube.com/watch?v=abc" };
const THROWAWAY = 99;

function chromeEvent() {
  const listeners = [];
  return {
    listeners,
    addListener: (listener) => listeners.push(listener),
    removeListener: (listener) => {
      const at = listeners.indexOf(listener);
      if (at !== -1) listeners.splice(at, 1);
    },
  };
}

/**
 * Loads the background script against a stub browser.
 *
 * `scriptable: false` refuses every injection, as the PDF viewer and store
 * pages do. `contextMenus: false` leaves the API out entirely, as Firefox for
 * Android did.
 */
async function loadBackground({ contextMenus = true, scriptable = true } = {}) {
  const read = (file) => readFile(join(root, "extension", "src", file), "utf8");
  const code = [
    (await read("shared.js")).replace(/^export /gm, ""),
    (await read("background.js")).replace(/^import .*$/m, ""),
  ].join("\n");

  // The page the toolbar was pressed on, which the injected function runs in.
  const page = makePage();
  const record = { badges: [], titles: [], created: [], activated: [], removed: [] };

  const chrome = {
    runtime: {
      onInstalled: chromeEvent(),
      getManifest: () => ({ action: { default_title: "Send this page to Nitrate" } }),
    },
    action: {
      onClicked: chromeEvent(),
      setBadgeBackgroundColor: async () => {},
      setBadgeText: async ({ text }) => void record.badges.push(text),
      setTitle: async ({ title }) => void record.titles.push(title),
    },
    scripting: {
      executeScript: async ({ func, args = [] }) => {
        if (!scriptable) throw new Error("Cannot access contents of the page.");
        return [{ result: await func(...args) }];
      },
    },
    tabs: {
      create: async (props) => {
        record.created.push(props);
        return { id: THROWAWAY, ...props };
      },
      get: async (id) => ({ id, active: true }),
      update: async (id) => void record.activated.push(id),
      remove: async (id) => void record.removed.push(id),
    },
    windows: { WINDOW_ID_NONE: -1, onFocusChanged: chromeEvent() },
  };
  if (contextMenus) {
    chrome.contextMenus = { onClicked: chromeEvent(), removeAll: (done) => done?.(), create() {} };
  }

  const sandbox = {
    chrome,
    window: page.window,
    document: page.document,
    setTimeout,
    clearTimeout,
    URL,
  };
  vm.createContext(sandbox);
  new vm.Script(code, { filename: "background.js" }).runInContext(sandbox);
  return { chrome, record, ...page };
}

const press = (chrome) => chrome.action.onClicked.listeners[0](TAB);

const TOOLBAR = {
  async "background: a browser without context menus still gets the toolbar"() {
    const { chrome } = await loadBackground({ contextMenus: false });
    expect(chrome.action.onClicked.listeners.length === 1, "the toolbar listener was never registered");
    for (const listener of chrome.runtime.onInstalled.listeners) listener();
  },

  async "toolbar: an answer shows a tick"() {
    const { chrome, record, window, navigations } = await loadBackground();
    const pending = press(chrome);
    await until(() => navigations.length, "the link to be sent");
    window.dispatch("blur");
    await pending;
    expect(record.badges.at(-1) === "✓", `the badge ended on ${JSON.stringify(record.badges.at(-1))}`);
    expect(window.listenerCount("blur") === 0, "the injected function kept listening");
  },

  async "toolbar: silence shows a question mark, and says why"() {
    const { chrome, record, navigations } = await loadBackground();
    await press(chrome);
    expect(navigations.length === 1, `expected one send, got ${navigations.length}`);
    expect(record.badges.at(-1) === "?", `the badge ended on ${JSON.stringify(record.badges.at(-1))}`);
    expect(
      /didn't answer/.test(record.titles.at(-1)),
      `the tooltip doesn't say why: ${JSON.stringify(record.titles.at(-1))}`,
    );
  },

  async "toolbar: a page that refuses scripts is sent through a tab in front"() {
    const { chrome, record } = await loadBackground({ scriptable: false });
    const pending = press(chrome);
    const [created] = await until(() => record.created.length && record.created, "a tab to open");
    expect(
      created.active === true && created.url.startsWith("nitrate://add?url="),
      `wanted the link opened in front, got ${JSON.stringify(created)}`,
    );
    expect(record.removed.length === 0, "the tab closed before anyone could answer its prompt");

    for (const listener of [...chrome.windows.onFocusChanged.listeners]) {
      listener(chrome.windows.WINDOW_ID_NONE);
    }
    await pending;
    expect(record.badges.at(-1) === "✓", `the badge ended on ${JSON.stringify(record.badges.at(-1))}`);
    await until(() => record.removed.includes(THROWAWAY), "the throwaway tab to close");
    expect(record.activated.includes(TAB.id), "the user wasn't handed back to their own tab");
  },
};

/** Runs named checks side by side, collecting rather than throwing. */
function runChecks(checks) {
  return Promise.all(
    Object.entries(checks).map(async ([name, check]) => {
      try {
        await check();
        return { name };
      } catch (error) {
        return { name, error };
      }
    }),
  );
}

async function main() {
  let failures = 0;
  const fail = (message) => {
    console.error(`  FAIL  ${message}`);
    failures += 1;
  };

  // Started first and reported after the site passes: they wait on real
  // timers too, and share nothing with anything else.
  const pressing = runChecks({ ...PRESSING, ...TOOLBAR });

  // Concurrent because each one waits out the fallback delay, and the sandboxes
  // share nothing.
  const results = await Promise.all(
    HOSTS.map(async ([name, hostname, pathname]) => {
      try {
        return { name, hostname, logs: (await run("content.js", hostname, pathname)).logs };
      } catch (error) {
        return { name, hostname, error };
      }
    }),
  );

  for (const { name, hostname, logs, error } of results) {
    if (error) {
      fail(`content.js on ${hostname}: ${error.message}`);
      continue;
    }
    const expected = name === "unsupported" ? "isn't a supported site" : `active on ${name}`;
    if (!logs.some((line) => line.includes(expected))) {
      fail(`content.js on ${hostname}: expected "${expected}", got ${JSON.stringify(logs)}`);
      continue;
    }
    // An empty page matches nothing, so every supported site must reach the
    // corner button. If it doesn't, the fallback has stopped working.
    if (name !== "unsupported" && !logs.some((line) => line.includes("floating button"))) {
      fail(`content.js on ${hostname}: never fell back to the floating button`);
      continue;
    }
    console.log(`  ok    content.js on ${hostname}`);
  }

  for (const { name, error } of await pressing) {
    if (error) fail(`${name}: ${error.message}`);
    else console.log(`  ok    ${name}`);
  }

  // A reel feed keeps several reels alive at once. Every rail gets a button, or
  // scrolling to the next reel leaves it behind — which it did, twice.
  try {
    const { extras: rails, logs } = await run(
      "content.js",
      "www.instagram.com",
      "/reels/abc/",
      reelFixture,
    );
    const placed = rails.filter((rail) => rail.querySelector(`[${MARK}]`));
    if (placed.length !== rails.length) {
      fail(`reel feed: ${placed.length} of ${rails.length} rails got a button`);
    } else if (logs.some((line) => line.includes("floating button"))) {
      fail("reel feed: fell back to the corner button despite finding the rails");
    } else {
      console.log(`  ok    reel feed: a button on each of ${rails.length} rails`);
    }
  } catch (error) {
    fail(`reel feed: ${error.message}`);
  }

  // The clip row: the anchor has to climb two levels to reach it and land
  // before the overflow menu, not after it.
  try {
    const { extras: row, logs } = await run(
      "content.js",
      "www.twitch.tv",
      "/ludwig/clip/Something",
      clipFixture,
    );
    const index = row.children.findIndex((child) => child.hasAttribute(MARK));
    if (index === -1) {
      fail("clip row: no button was placed");
    } else if (index !== row.children.length - 2) {
      fail(`clip row: button landed at ${index} of ${row.children.length}, wanted second from the end`);
    } else if (logs.some((line) => line.includes("floating button"))) {
      fail("clip row: fell back to the corner button despite finding the row");
    } else {
      console.log("  ok    clip row: button sits before the overflow menu");
    }
  } catch (error) {
    fail(`clip row: ${error.message}`);
  }

  // A VOD is not a clip page, and none of the clip anchors exist on it. The
  // button has to reach the row anyway, and land before the overflow menu.
  try {
    const { extras: row, logs } = await run(
      "content.js",
      "www.twitch.tv",
      "/videos/2833641956",
      vodFixture,
    );
    const index = row.children.findIndex((child) => child.hasAttribute(MARK));
    if (index === -1) {
      fail("VOD row: no button was placed");
    } else if (index !== row.children.length - 2) {
      fail(`VOD row: button landed at ${index} of ${row.children.length}, wanted second from the end`);
    } else if (logs.some((line) => line.includes("floating button"))) {
      fail("VOD row: fell back to the corner button despite finding the row");
    } else {
      console.log("  ok    VOD row: button sits before the overflow menu");
    }
  } catch (error) {
    fail(`VOD row: ${error.message}`);
  }

  // A DM thread on its own offers nothing to compress, and its chrome is shaped
  // exactly like an action row — so this is the check that the page is being
  // judged rather than the markup. The corner button counts as a failure too:
  // it would send the conversation.
  try {
    const { sandbox } = await run(
      "content.js",
      "www.instagram.com",
      "/direct/t/17999",
      (body) => dmFixture(body, false),
    );
    const placed = descendants(sandbox.document.body).filter((node) =>
      node.hasAttribute?.(MARK),
    );
    if (placed.length > 0) {
      fail(`instagram DMs: ${placed.length} button(s) placed in a conversation`);
    } else {
      console.log("  ok    instagram DMs: no button in the conversation");
    }
  } catch (error) {
    fail(`instagram DMs: ${error.message}`);
  }

  // Open a shared clip, though, and there's exactly one video to point at.
  try {
    const { sandbox, extras } = await run(
      "content.js",
      "www.instagram.com",
      "/direct/t/17999",
      (body) => dmFixture(body, true),
    );
    const placed = descendants(sandbox.document.body).filter((node) =>
      node.hasAttribute?.(MARK),
    );
    if (placed.length !== 1) {
      fail(`instagram DM viewer: ${placed.length} button(s), wanted exactly 1`);
    } else if (!extras.viewer.contains(placed[0])) {
      fail("instagram DM viewer: the button landed outside the opened video");
    } else {
      console.log("  ok    instagram DM viewer: one button, inside the video");
    }
  } catch (error) {
    fail(`instagram DM viewer: ${error.message}`);
  }

  // A live channel has nothing finished behind it. The share button is right
  // there and would otherwise be taken as an anchor, so this is the check that
  // the page itself is being judged and not just the markup on it.
  for (const [what, pathname] of [
    ["a live channel", "/somestreamer"],
    ["the directory", "/directory/game/Chess"],
    ["a channel's video list", "/somestreamer/videos"],
  ]) {
    try {
      const { sandbox } = await run("content.js", "www.twitch.tv", pathname, liveFixture);
      const placed = descendants(sandbox.document.body).filter((node) =>
        node.hasAttribute?.(MARK),
      );
      if (placed.length > 0) {
        fail(`${what}: ${placed.length} button(s) placed where there's nothing to download`);
      } else {
        console.log(`  ok    ${what}: no button`);
      }
    } catch (error) {
      fail(`${what}: ${error.message}`);
    }
  }

  // A debounce with no ceiling waits for a quiet moment that a busy page never
  // has, so the scan was postponed for as long as the page stayed busy —
  // neither the inline button nor the corner one ever arrived.
  try {
    const { logs } = await run("content.js", "www.twitch.tv", "/videos/1", undefined, {
      busy: true,
    });
    if (!logs.some((line) => line.includes("floating button"))) {
      fail("a page that never goes quiet: the scan never ran");
    } else {
      console.log("  ok    a page that never goes quiet still gets scanned");
    }
  } catch (error) {
    fail(`a page that never goes quiet: ${error.message}`);
  }

  try {
    await run("options.js", "example.com", "/");
    console.log("  ok    options.js");
  } catch (error) {
    fail(`options.js: ${error.message}`);
  }

  if (failures) {
    console.error(`\n${failures} check(s) failed — not packaging.`);
    process.exit(1);
  }
}

main();
