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

chrome.runtime.onInstalled.addListener(() => {
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

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  // Prefer whatever the user actually right-clicked over the page it sits on.
  const target =
    (info.menuItemId === MENU_LINK && info.linkUrl) ||
    (info.menuItemId === MENU_VIDEO && (info.srcUrl || info.pageUrl)) ||
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
  try {
    await sendToApp(url, tabId);
    await flashBadge("✓", "#43B581");
  } catch {
    await flashBadge("!", "#ED4245");
  }
}
