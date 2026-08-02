# Privacy Policy

_Last updated: 2 August 2026_

This covers the **Nitrate** desktop application and the **Send to Nitrate**
browser extension.

## The short version

Neither collects anything. There is no analytics, no telemetry, no account, and
no server belonging to this project that your data passes through.

## The browser extension

The extension does one thing: when you click its button or choose it from the
right-click menu, it hands the address of the page or link you chose to the
Nitrate desktop application on the same computer, using a `nitrate://` link.

- It does not read page content.
- It does not track which pages you visit.
- It does not store anything, locally or remotely.
- It does not send anything over the internet.

The URL you select never leaves your machine — it travels from the browser to a
program on the same computer, and nowhere else.

## The desktop application

Video files are read, encoded and written entirely on your own computer. No file
or frame is ever uploaded.

The application makes network requests in exactly three situations:

1. **You paste or send a link.** It contacts that site to fetch the video you
   asked for, using [yt-dlp](https://github.com/yt-dlp/yt-dlp). Ordinary traffic
   with that site, no different from opening it in a browser.
2. **Fetching the downloader.** yt-dlp is downloaded from its GitHub releases the
   first time you use a link, and refreshed periodically.
3. **Checking for updates.** The application asks GitHub whether a newer version
   exists. This request contains no identifier beyond what any HTTP request
   necessarily reveals to the server it contacts.

Settings and preferences are stored in a file on your own computer and are never
transmitted.

## Third parties

GitHub serves the update check and the yt-dlp download, so GitHub sees those
requests as it would any other download. Sites you send links to see a request
for the video, as they would from a browser.

## Contact

Questions or concerns: open an issue at
<https://github.com/BeeeFX/Nitrate/issues>.
