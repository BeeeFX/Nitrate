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

/**
 * Hands a link to the desktop app.
 *
 * Assigning to `location` in the page is what actually triggers the protocol
 * handler. The page doesn't navigate — the browser recognises the scheme and
 * passes it to the OS — so this is invisible from the user's point of view.
 */
export async function sendToApp(url, tabId) {
  if (!isSendable(url)) {
    throw new Error("Only web links can be sent to Nitrate.");
  }

  const deepLink = buildDeepLink(url);

  if (tabId != null) {
    await chrome.scripting.executeScript({
      target: { tabId },
      // `world: MAIN` isn't needed; the isolated world can navigate too.
      func: (link) => {
        window.location.href = link;
      },
      args: [deepLink],
    });
    return;
  }

  // No tab to borrow — open and immediately close a throwaway one.
  const tab = await chrome.tabs.create({ url: deepLink, active: false });
  setTimeout(() => chrome.tabs.remove(tab.id).catch(() => {}), 800);
}

/** Brief confirmation on the toolbar icon, so a click never feels ignored. */
export async function flashBadge(text, colour) {
  try {
    await chrome.action.setBadgeBackgroundColor({ color: colour });
    await chrome.action.setBadgeText({ text });
    setTimeout(() => chrome.action.setBadgeText({ text: "" }), 1600);
  } catch {
    // Badges are cosmetic; never let one break the actual send.
  }
}
