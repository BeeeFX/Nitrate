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

function makeElement(tag = "div", attrs = {}) {
  const element = {
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
      remove(name) {
        element.className = element.className
          .split(" ")
          .filter((c) => c && c !== name)
          .join(" ");
      },
      contains: (name) => element.className.split(" ").includes(name),
    },
    setAttribute(name, value) {
      element.attrs[name] = String(value);
    },
    getAttribute: (name) => element.attrs[name] ?? null,
    hasAttribute: (name) => name in element.attrs,
    addEventListener() {},
    removeEventListener() {},
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
  };
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

function makeSandbox(hostname, pathname, fixture) {
  const body = makeElement("body");
  const head = makeElement("head");
  const documentElement = makeElement("html");
  documentElement.appendChild(head);
  documentElement.appendChild(body);

  const extras = fixture ? fixture(body) : null;
  const logs = [];

  const document = {
    body,
    head,
    documentElement,
    createElement: (tag) => makeElement(tag),
    createElementNS: (_ns, tag) => makeElement(tag),
    querySelector: (selector) => body.querySelector(selector),
    querySelectorAll: (selector) => body.querySelectorAll(selector),
    getElementById: () => makeElement(),
    addEventListener() {},
  };

  return {
    logs,
    extras,
    sandbox: {
      document,
      location: {
        hostname,
        pathname,
        href: `https://${hostname}${pathname}`,
        origin: `https://${hostname}`,
      },
      window: {
        addEventListener() {},
        location: { href: "" },
        // Deliberately useless: the style-adoption path has to cope with a
        // reference button it can't measure.
        getComputedStyle: () => ({
          getPropertyValue: () => "",
          backgroundColor: "",
          borderRadius: "",
        }),
      },
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
      MutationObserver: class {
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

async function run(file, hostname, pathname, fixture) {
  const code = await readFile(join(root, "extension", "src", file), "utf8");
  const context = makeSandbox(hostname, pathname, fixture);
  vm.createContext(context.sandbox);
  new vm.Script(code, { filename: file }).runInContext(context.sandbox);
  await settle();
  return context;
}

async function main() {
  let failures = 0;
  const fail = (message) => {
    console.error(`  FAIL  ${message}`);
    failures += 1;
  };

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
