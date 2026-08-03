// Runs the extension's scripts against a stub DOM before they're packaged.
//
// A parse check isn't enough: the bug that shipped in 0.4.0 was a temporal dead
// zone — startup called a function that read a `let` declared further down, so
// the file parsed perfectly and threw on the first line it executed. Nothing
// short of running it would have caught that.
//
// The stub answers "no" to every query, which is the worst case anyway: no
// anchors match, so the code walks its whole fallback path.

import { readFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

const HOSTS = [
  ["youtube", "www.youtube.com", "/watch?v=abc"],
  ["twitch", "www.twitch.tv", "/videos/1"],
  ["x", "x.com", "/user/status/1"],
  ["reddit", "www.reddit.com", "/r/a/comments/b/c/"],
  ["instagram", "www.instagram.com", "/reels/abc/"],
  ["unsupported", "example.com", "/"],
];

function makeElement(tag = "div") {
  const element = {
    tagName: tag.toUpperCase(),
    dataset: {},
    style: {},
    children: [],
    parentElement: null,
    innerHTML: "",
    textContent: "",
    className: "",
    classList: {
      add() {},
      remove() {},
      contains: () => false,
    },
    setAttribute() {},
    getAttribute: () => null,
    addEventListener() {},
    removeEventListener() {},
    appendChild(child) {
      element.children.push(child);
      return child;
    },
    prepend(child) {
      element.children.unshift(child);
      return child;
    },
    after() {},
    remove() {},
    querySelector: () => null,
    querySelectorAll: () => [],
    getBoundingClientRect: () => ({ width: 0, height: 0, top: 0, left: 0 }),
  };
  return element;
}

function makeSandbox(hostname, pathname) {
  const body = makeElement("body");
  const head = makeElement("head");
  const documentElement = makeElement("html");

  const logs = [];

  const document = {
    body,
    head,
    documentElement,
    createElement: (tag) => makeElement(tag),
    querySelector: () => null,
    querySelectorAll: () => [],
    getElementById: () => makeElement(),
    addEventListener() {},
  };

  return {
    logs,
    sandbox: {
      document,
      location: { hostname, pathname, href: `https://${hostname}${pathname}`, origin: `https://${hostname}` },
      window: { addEventListener() {}, location: { href: "" } },
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
// fallback path, which is the one a stub DOM always ends up on, never runs.
const settle = () => new Promise((r) => setTimeout(r, 5000));

async function run(file, hostname, pathname) {
  const code = await readFile(join(root, "extension", "src", file), "utf8");
  const { sandbox, logs } = makeSandbox(hostname, pathname);
  vm.createContext(sandbox);
  new vm.Script(code, { filename: file }).runInContext(sandbox);
  await settle();
  return logs;
}

async function main() {
  let failures = 0;

  // Concurrent because each one waits out the fallback delay, and the sandboxes
  // share nothing.
  const results = await Promise.all(
    HOSTS.map(async ([name, hostname, pathname]) => {
      try {
        return { name, hostname, logs: await run("content.js", hostname, pathname) };
      } catch (err) {
        return { name, hostname, error: err };
      }
    }),
  );

  for (const { name, hostname, logs, error } of results) {
    if (error) {
      console.error(`  FAIL  content.js on ${hostname}: ${error.message}`);
      failures += 1;
      continue;
    }
    const expected = name === "unsupported" ? "isn't a supported site" : `active on ${name}`;
    if (!logs.some((line) => line.includes(expected))) {
      console.error(`  FAIL  content.js on ${hostname}: expected "${expected}", got ${JSON.stringify(logs)}`);
      failures += 1;
      continue;
    }
    // The stub matches nothing, so every supported site must reach the corner
    // button. If it doesn't, the fallback has stopped working.
    if (name !== "unsupported" && !logs.some((line) => line.includes("floating button"))) {
      console.error(`  FAIL  content.js on ${hostname}: never fell back to the floating button`);
      failures += 1;
      continue;
    }
    console.log(`  ok    content.js on ${hostname}`);
  }

  try {
    await run("options.js", "example.com", "/");
    console.log("  ok    options.js");
  } catch (err) {
    console.error(`  FAIL  options.js: ${err.message}`);
    failures += 1;
  }

  if (failures) {
    console.error(`\n${failures} check(s) failed — not packaging.`);
    process.exit(1);
  }
}

main();
