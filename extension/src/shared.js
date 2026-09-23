// Shared helpers. Chrome and Firefox both expose `chrome.*` in Manifest V3,
// so one namespace covers both without a polyfill.

export const SCHEME = "nitrate";

/** Sites where an inline button is injected. */
export const SUPPORTED_HOSTS = [
  "youtube.com",
  "youtu.be",
  "twitter.com",
  "x.com",
  "instagram.com",
  "reddit.com",
  "redd.it",
  "twitch.tv",
];

/**
 * Only ordinary web links are ever handed over.
 *
 * The app validates independently — it has to, since any page can fire the
 * protocol — but refusing here too means an obviously wrong link never leaves
 * the browser.
 */
export function isSendable(url) {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}

export function buildDeepLink(url) {
  return `${SCHEME}://add?url=${encodeURIComponent(url)}`;
}

// How long the app gets to take focus before a send is called unanswered, and
// how long a throwaway tab waits for the browser's prompt to be dealt with.
// The inline button in `content.js` uses the same 2.5 seconds.
const ANSWER_WINDOW = 2500;
const LATE_ANSWER = 15000;

/**
 * Hands a link to the desktop app, and says whether anything answered.
 *
 * Assigning to `location` in the page is what actually triggers the protocol
 * handler. The page doesn't navigate — the browser recognises the scheme and
 * passes it to the OS — so this is invisible from the user's point of view.
 * It's also silent when nothing is registered, which is why the result is
 * judged by whether the app took focus rather than assumed; see
 * `fireAndWait`.
 */
export async function sendToApp(url, tabId) {
  if (!isSendable(url)) {
    throw new Error("Only web links can be sent to Nitrate.");
  }

  const deepLink = buildDeepLink(url);

  if (tabId != null && (await acceptsScripts(tabId))) {
    try {
      const [injection] = await chrome.scripting.executeScript({
        target: { tabId },
        // `world: MAIN` isn't needed; the isolated world can navigate too.
        func: fireAndWait,
        args: [deepLink, ANSWER_WINDOW],
      });
      return injection?.result === true;
    } catch {
      // The page took a script a moment ago, so this is almost always the tab
      // closing or navigating while we waited — after the link had gone.
      // Trying again through a tab of our own would send the same video twice.
      return false;
    }
  }

  // Nowhere to run a script: no tab at all, or one that refuses them, like the
  // PDF viewer or a store page. The address is fine, so it goes out through a
  // tab of our own instead.
  return sendThroughTab(deepLink, tabId);
}

/**
 * Whether a script can be injected into this tab at all.
 *
 * Asked separately, before anything is sent, because afterwards the two kinds
 * of failure look the same: a page that refused the script, and one that took
 * it, sent the link, and then closed while we waited to hear back. The first
 * wants another route; the second has already been sent.
 */
async function acceptsScripts(tabId) {
  try {
    await chrome.scripting.executeScript({ target: { tabId }, func: () => true });
    return true;
  } catch {
    return false;
  }
}

/**
 * Runs in the page: fires the link, then waits to see if anything took it.
 *
 * Nitrate brings its window forward whenever it's handed a link, so the page
 * losing focus — or going hidden under it — soon after is the app answering.
 * Nothing within `wait` means nothing did: Chrome logs "no registered
 * handler" where nobody will see it, and Firefox blocks the navigation, both
 * without a word to the page.
 *
 * Serialised into the tab by `executeScript`, so it can't reach anything else
 * in this module — the wait arrives as an argument. The browser awaits the
 * promise and hands back what it resolves to.
 */
function fireAndWait(link, wait) {
  return new Promise((resolve) => {
    const settle = (answered) => {
      window.removeEventListener("blur", onBlur);
      document.removeEventListener("visibilitychange", onHidden);
      clearTimeout(timer);
      resolve(answered);
    };
    const onBlur = () => settle(true);
    const onHidden = () => {
      if (document.visibilityState === "hidden") settle(true);
    };
    const timer = setTimeout(() => settle(false), wait);

    window.addEventListener("blur", onBlur);
    document.addEventListener("visibilitychange", onHidden);
    window.location.href = link;
  });
}

/**
 * Opens the link in a throwaway tab, for when there's no page to borrow.
 *
 * It used to open in the background and close 800ms later — which is before
 * anyone could answer the browser's "Open Nitrate?" prompt, and a background
 * tab's prompt can't be seen anyway. So it opens in front, and stays until the
 * browser loses focus to the app or the prompt has plainly been left alone.
 * Closing it hands the user back to the tab they were on, if it's still
 * showing ours; if they've moved on, they're left where they went.
 */
async function sendThroughTab(deepLink, returnTo) {
  const leaving = browserLosesFocus(LATE_ANSWER);
  const tab = await chrome.tabs.create({
    url: deepLink,
    active: true,
    ...(returnTo != null && { openerTabId: returnTo }),
  });

  leaving.then(async () => {
    try {
      const current = await chrome.tabs.get(tab.id);
      if (current.active && returnTo != null) {
        await chrome.tabs.update(returnTo, { active: true });
      }
      await chrome.tabs.remove(tab.id);
    } catch {
      // Already closed by hand, or the tab to go back to has gone.
    }
  });

  // `leaving` only ever settles on `false` long after this, so the race is
  // really "did the app answer in time".
  const tooLate = new Promise((resolve) => setTimeout(() => resolve(false), ANSWER_WINDOW));
  return Promise.race([leaving, tooLate]);
}

/**
 * Resolves `true` when the browser loses focus to another program, or `false`
 * after `wait`.
 *
 * A throwaway tab can't be asked about blur the way a borrowed page can — the
 * extension has no access to it — so the question goes to the browser's
 * windows instead. None of them having focus means something outside the
 * browser took it.
 */
function browserLosesFocus(wait) {
  return new Promise((resolve) => {
    const changed = (windowId) => {
      if (windowId === chrome.windows.WINDOW_ID_NONE) settle(true);
    };
    const settle = (answered) => {
      chrome.windows.onFocusChanged.removeListener(changed);
      clearTimeout(timer);
      resolve(answered);
    };
    const timer = setTimeout(() => settle(false), wait);
    chrome.windows.onFocusChanged.addListener(changed);
  });
}

let badgeTimer = 0;

/**
 * Brief confirmation on the toolbar icon, so a click never feels ignored.
 *
 * With a `title`, the icon's tooltip says what the badge means for as long as
 * it shows — a "?" alone doesn't explain itself — and goes back to the
 * manifest's own afterwards. A new flash replaces the last one outright, so an
 * old timer can't clear a badge that's only just gone up.
 */
export async function flashBadge(text, colour, { title, hold = 1600 } = {}) {
  clearTimeout(badgeTimer);
  try {
    await chrome.action.setBadgeBackgroundColor({ color: colour });
    await chrome.action.setBadgeText({ text });
    await chrome.action.setTitle({ title: title ?? defaultTitle() });
    // Again, since another flash may have started while this one was awaiting.
    clearTimeout(badgeTimer);
    badgeTimer = setTimeout(() => {
      chrome.action.setBadgeText({ text: "" });
      chrome.action.setTitle({ title: defaultTitle() });
    }, hold);
  } catch {
    // Badges are cosmetic; never let one break the actual send.
  }
}

function defaultTitle() {
  return chrome.runtime.getManifest?.().action?.default_title ?? "Send to Nitrate";
}
