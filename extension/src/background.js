// Right-click entries and the toolbar button.
//
// The extension holds no permission to read page content on its own — every
// path here acts on a URL the browser hands us, or on the tab the user just
// clicked. Nothing is collected and nothing leaves the machine except a link
// passed to the local app.

import { flashBadge, isSendable, sendToApp } from "./shared.js";

const MENU_LINK = "nitrate-send-link";
const MENU_PAGE = "nitrate-send-page";
const MENU_VIDEO = "nitrate-send-video";

const UNANSWERED_TITLE = "Nitrate didn't answer — is the app installed and running?";

chrome.runtime.onInstalled.addListener(() => {
  if (!chrome.contextMenus) return;
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: MENU_LINK,
      title: "Send link to Nitrate",
      contexts: ["link"],
    });
    chrome.contextMenus.create({
      id: MENU_VIDEO,
      title: "Send video to Nitrate",
      contexts: ["video"],
    });
    chrome.contextMenus.create({
      id: MENU_PAGE,
      title: "Send this page to Nitrate",
      contexts: ["page", "frame"],
    });
  });
});

// Optional on purpose. Firefox for Android has no context menus at all, and
// an unguarded call here threw before the toolbar listener below was ever
// registered — so one missing API took the whole extension down with it. The
// extension no longer offers itself on Android, but a browser without menus
// should still get a working toolbar button.
chrome.contextMenus?.onClicked.addListener(async (info, tab) => {
  // A streamed video's source is a `blob:` address that only means something
  // inside that tab — which is every video on the five supported sites — so
  // it counts as no source at all, and the page (or the embed it sits in)
  // is sent instead. Passing it on only ever flashed the red "!".
  const videoSrc = info.srcUrl && isSendable(info.srcUrl) ? info.srcUrl : null;

  // Prefer whatever the user actually right-clicked over the page it sits on.
  const target =
    (info.menuItemId === MENU_LINK && info.linkUrl) ||
    (info.menuItemId === MENU_VIDEO && (videoSrc || info.frameUrl || info.pageUrl)) ||
    info.pageUrl ||
    tab?.url;

  await deliver(target, tab?.id);
});

chrome.action.onClicked.addListener(async (tab) => {
  await deliver(tab?.url, tab?.id);
});

// The inline buttons don't come through here — a content script can navigate
// to the protocol itself, and asking the background to inject a script would
// need host permissions that being listed in `content_scripts` doesn't grant.

async function deliver(url, tabId) {
  if (!isSendable(url)) {
    await flashBadge("!", "#ED4245");
    return;
  }
  // The verdict takes a couple of seconds to arrive, and the old instant tick
  // was only ever a guess. Something has to show in the meantime, or the click
  // looks ignored; whichever badge comes next replaces this one.
  await flashBadge("…", "#5865F2", { hold: 10000 });
  try {
    if (await sendToApp(url, tabId)) {
      await flashBadge("✓", "#43B581");
    } else {
      // Not an error — the link went out fine, and nothing took it. Held long
      // enough to hover over and read why.
      await flashBadge("?", "#FAA61A", { title: UNANSWERED_TITLE, hold: 8000 });
    }
  } catch {
    await flashBadge("!", "#ED4245");
  }
}
