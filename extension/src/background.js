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

// Inline buttons live in the page, which can't reach the protocol handler
// through a sandboxed content script reliably — so they ask us instead.
chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (message?.type !== "nitrate:send") return;
  deliver(message.url, sender.tab?.id).then(
    () => sendResponse({ ok: true }),
    (error) => sendResponse({ ok: false, error: String(error) }),
  );
  // Keeps the channel open for the async reply.
  return true;
});

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
