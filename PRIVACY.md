# Privacy Policy

_Last updated: 23 September 2026_

This covers the **Nitrate** desktop application and the **Send to Nitrate**
browser extension.

## The short version

Neither collects anything. There is no analytics, no telemetry, no account, and
no server belonging to this project that your data passes through.

## The browser extension

The extension does one thing: when you click its button or choose it from the
right-click menu, it hands the address of the page or link you chose to the
Nitrate desktop application on the same computer, using a `nitrate://` link.

What it reads:

- **On YouTube, X, Instagram, Reddit and Twitch**, it reads the page's markup
  to work out where its button goes and which post or video each button
  belongs to — so that, in a feed, it sends the address of the post you
  clicked rather than the feed's. It also reads how the site's own buttons are
  styled, so that its button can match them. This happens inside your browser,
  is used on the spot, and is never kept or sent anywhere.
- **On any other site**, it reads nothing unless you use its right-click entry
  or its toolbar button, and then only the address of the page or link you
  chose.

What it stores:

- **One display preference**: whether its button shows the Nitrate mark alone
  or the mark and the name. It's kept in the browser's extension storage. If
  you have browser sync turned on, your browser copies it to your other
  signed-in devices through its own sync service, as it does your other
  settings; nothing belonging to this project is involved.
- Nothing else — no history of what you've sent, and no record of the pages
  you visit.

What it sends:

- The address you chose, to the Nitrate application. It travels from the
  browser to a program on the same computer, and nowhere else.
- Nothing of its own over the internet, and no analytics or telemetry of any
  kind.

If the application doesn't answer — usually because it isn't installed — the
button offers to open the application's download page. That's an ordinary
visit to GitHub, made only if you click, and it carries nothing from the page
you were on.

## The desktop application

Video files are read, encoded and written entirely on your own computer. No file
or frame is ever uploaded.

The application makes network requests in exactly three situations:

1. **You paste or send a link.** It contacts that site to fetch the video you
   asked for, using [yt-dlp](https://github.com/yt-dlp/yt-dlp). Ordinary traffic
   with that site, no different from opening it in a browser.
2. **Fetching the helper tools.** yt-dlp is downloaded from its GitHub releases
   the first time you use a link, and refreshed periodically. Alongside it,
   QuickJS is fetched from GitHub (YouTube will not serve a video without a
   JavaScript engine to answer its player challenge) and gallery-dl from
   Codeberg (for photo posts yt-dlp cannot reach). Each is downloaded once and
   only when a link needs it.
3. **Checking for updates.** The application asks GitHub whether a newer version
   exists. This request contains no identifier beyond what any HTTP request
   necessarily reveals to the server it contacts.

Settings and preferences are stored in a file on your own computer and are never
transmitted.

## Third parties

GitHub serves the update check and the yt-dlp and QuickJS downloads, and
Codeberg serves gallery-dl, so each sees those requests as it would any other
download. Sites you send links to see a request for the video, as they would
from a browser.

## Contact

Questions or concerns: open an issue at
<https://github.com/BeeeFX/Nitrate/issues>.
