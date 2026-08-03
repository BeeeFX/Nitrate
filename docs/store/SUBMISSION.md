# Store submission cheat sheet

Everything to paste into the Chrome Web Store and Firefox AMO forms, in roughly
the order each asks for it.

**Packages** — build them yourself, they aren't attached to releases:

```bash
npm run extension
```

That writes both into `extension/dist/`:

| Store | Upload |
| --- | --- |
| Chrome Web Store | `nitrate-extension-chrome-*.zip` |
| Firefox AMO | `nitrate-extension-firefox-*.zip` |

They are **not interchangeable** — Chrome wants a service worker, Firefox wants
background scripts and a fixed add-on id.

They're kept off the release page on purpose: a loose zip only invites people to
sideload a copy that never auto-updates, which is the problem the stores exist to
solve.

**Artwork** — in this folder:

| File | Used for |
| --- | --- |
| `icon-128.png` | Store icon, both stores |
| `promo-1280x800.png` | Screenshot slot (Chrome requires at least one) |
| `tile-440x280.png` | Chrome small promo tile |
| `marquee-1400x560.png` | Chrome marquee, shown across the top of the listing |

The three promo images are **24-bit PNG with no alpha channel**, which Chrome
insists on and silently rejects otherwise. If you regenerate them, keep the
`.flatten()` call in the render — without it sharp emits RGBA and the upload
fails with an unhelpful error. The icon may keep its alpha; only the promo slots
care.

---

# Chrome Web Store

## Store listing

**Item name** _(75 max)_

```
Send to Nitrate
```

**Summary** _(132 max — this is 92)_

```
Send a video link to the Nitrate desktop app in one click, instead of copying and pasting it.
```

**Description** _(16,000 max)_

```
Send to Nitrate adds a one-click button for handing a video link to the Nitrate desktop application, so you don't have to select the address bar, copy it, switch windows and paste.

WHAT IT DOES

• Adds a "Send to Nitrate" button to YouTube, X, Instagram, Reddit and Twitch
• Adds a right-click entry that works on any link, on any site
• Adds a toolbar button that sends whatever page you're currently on

The button is styled to match each site's own controls, and can be set to show
its mark alone if you'd rather it stayed quiet.

That's the whole extension. It hands an address to a program on your own computer and stops there.

REQUIRES THE DESKTOP APP

This extension does nothing on its own. It needs the free, open-source Nitrate application installed on the same computer:

https://github.com/BeeeFX/Nitrate

Nitrate compresses video to a chosen file size, for sharing somewhere with an upload limit.

PRIVACY

• No analytics, no telemetry, no account
• Reads no page content beyond the link address a button belongs to
• Stores nothing, locally or remotely
• Sends nothing over the internet — the address travels from your browser to a program on the same machine, and nowhere else

Full policy: https://github.com/BeeeFX/Nitrate/blob/main/PRIVACY.md

OPEN SOURCE

Every line is public and the shipped files are the repository files, unmodified:

https://github.com/BeeeFX/Nitrate/tree/main/extension
```

**Category**

```
Tools
```

**Language:** English (United Kingdom)

**Store icon:** `icon-128.png`

**Screenshots:** `promo-1280x800.png`
_See "Better screenshots" at the end — a real one is worth adding once you've
installed it._

**Small promo tile:** `tile-440x280.png`

**Marquee promo tile:** `marquee-1400x560.png`
_Optional, but it's what Chrome uses if the extension is ever featured, and it
fills the banner at the top of the listing page._

**Homepage URL / Support URL**

```
https://github.com/BeeeFX/Nitrate
```

## Privacy tab

This is the part that gets submissions rejected, so it's worth being precise.

**Single purpose**

```
Send the address of a video page or link to the Nitrate desktop application installed on the same computer, so the user does not have to copy and paste it by hand.
```

**Permission justifications**

`contextMenus`
```
Adds the "Send to Nitrate" entries to the right-click menu. This is the extension's main interface. It is used for nothing else.
```

`activeTab`
```
Reads the address of the tab the user is currently on, so that address can be handed to the desktop application when they click the toolbar button. Only the URL is read; no page content is accessed.
```

`scripting`
```
Used solely to open a "nitrate://" link in the current tab, which is the mechanism that passes the URL to the desktop application. The injected function does nothing but set the location to that link. No page data is read, stored or modified.
```

`storage`
```
Stores one setting, chosen by the user on the options page: whether the injected button shows its icon alone or its icon and name. No user data of any kind is stored.
```

**Host permission justification** _(youtube.com, x.com, twitter.com, instagram.com, reddit.com, twitch.tv)_
```
A content script adds a "Send to Nitrate" button into the page on these five sites, next to the existing share controls. To send the correct link, it reads the address of the specific post or video the button belongs to — for example the permalink of a tweet in a timeline. Nothing else on the page is read, and no page content is stored or transmitted anywhere.
```

**Are you using remote code?**

```
No, I am not using remote code
```
_(All JavaScript is contained in the package. Nothing is fetched or evaluated at
runtime.)_

**Data usage** — tick **nothing**. The extension collects no data of any kind.
The one thing it stores is a display preference the user chose themselves, which
is not user data and doesn't belong in any of these categories.

Then tick all three certifications:
- I do not sell or transfer user data to third parties, outside of approved use cases
- I do not use or transfer user data for purposes that are unrelated to my item's single purpose
- I do not use or transfer user data to determine creditworthiness or for lending purposes

**Privacy policy URL**

```
https://github.com/BeeeFX/Nitrate/blob/main/PRIVACY.md
```

## Distribution

- **Visibility:** Public
- **Regions:** All
- **Pricing:** Free

---

# Firefox AMO

<https://addons.mozilla.org/developers/addon/submit/distribution>

Choose **"On this site"** for listed distribution.

**Name**

```
Send to Nitrate
```

**Summary** _(250 max)_

```
Adds a Send to Nitrate button to YouTube, X, Instagram, Reddit and Twitch, plus a right-click entry that works anywhere. Hands the link to the Nitrate desktop app so you don't have to copy and paste it. Requires the free Nitrate application.
```

**Description** — the same text as the Chrome description above works as-is.

**Categories** _(pick up to two)_

```
Photos, Music & Videos
Other
```

**Support email:** your address
**Support site**

```
https://github.com/BeeeFX/Nitrate/issues
```

**License**

```
MIT License
```

**Privacy policy**

```
https://github.com/BeeeFX/Nitrate/blob/main/PRIVACY.md
```

**Tags**

```
video, compress, download, discord, share
```

**Release notes for this version**

```
First release. Adds a "Send to Nitrate" button on YouTube, X, Instagram, Reddit and Twitch, a right-click entry that works on any link, and a toolbar button for the current page. The button is styled to match each site's own controls, and an options page chooses whether it shows its icon alone or its icon and name.
```

## Notes to reviewer

AMO reviews source, so this saves a round trip:

```
This extension is a thin bridge to a desktop application. Its whole interface is one injected button, one context-menu entry, and an options page with a single setting. It performs no network activity whatsoever.

HOW IT WORKS
Clicking the button (or the context-menu entry) opens a "nitrate://" link in the current tab. That is a custom URL scheme registered by the Nitrate desktop application, which is what actually fetches and compresses the video. The extension's entire job is constructing that link and opening it.

SOURCE CODE
Not minified, obfuscated or transpiled. The files in the package are the files in the repository, byte for byte:

  https://github.com/BeeeFX/Nitrate/tree/main/extension

The build step (scripts/build-extension.mjs) only copies src/ and selects the correct manifest — Chrome needs a service worker, Firefox needs background scripts. Reproduce with:

  git clone https://github.com/BeeeFX/Nitrate
  npm install
  npm run extension

REMOTE CODE
None. No eval, no remotely hosted scripts, no dynamic imports.

DATA
Nothing is collected or transmitted. The content script reads only the href of the post a button was added to, so the correct link is sent rather than the page's own address.

The only use of storage is one preference, set by the user on the options page: whether the injected button shows its icon alone or its icon and name. It is written to storage.sync as a single string ("auto", "icon" or "label"). No user data of any kind is stored.

COMPANION APPLICATION
The desktop app is open source and MIT licensed: https://github.com/BeeeFX/Nitrate
```

**Does your extension use minified, concatenated or machine-generated code?**

```
No
```

---

# Better screenshots

Both listings currently use a promotional graphic. It satisfies the requirement,
but a genuine screenshot is far more persuasive and reassures reviewers the
extension does what it says.

Once you've installed it, capture:

1. **A YouTube watch page** with the blurple "Nitrate" button visible in the row
   next to Share — the single most convincing image
2. **The right-click menu** open on a link, showing "Send link to Nitrate"
3. Optionally **the app receiving it**, with the video queued

Chrome wants 1280×800 or 640×400. Firefox is relaxed about size. Drop them in
`docs/store/` and they'll be part of the repo.

---

# After it's live

Update these two places with the real listing URLs:

| Where | What |
| --- | --- |
| `README.md` | The "Browser extension" section, which currently sends people to the release zips |
| `src/lib/components/Tour.svelte` | `EXTENSION_URL`, used by the final tour step and the settings link |

Both currently point at the README anchor, which stays correct either way — so
this is an improvement rather than a fix.
