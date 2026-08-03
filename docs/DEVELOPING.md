# Developing Nitrate

Everything the README deliberately leaves out, so the front page can stay a
front page.

## Building from source

You'll need [Node](https://nodejs.org) 20+ and [Rust](https://rustup.rs). On
Windows, the MSVC build tools; on Linux, the
[Tauri system dependencies](https://tauri.app/start/prerequisites/).

```bash
npm install
npm run app
```

`npm install` also downloads the ffmpeg and ffprobe sidecars (~260 MB) into
`src-tauri/binaries/`. They're deliberately kept out of git.

| Command | What it does |
| --- | --- |
| `npm run app` | Run in development |
| `npm run app:build` | Build installers into `src-tauri/target/release/bundle/` |
| `npm run ffmpeg` | Re-fetch the ffmpeg sidecars |
| `npm run icons` | Regenerate icons from `assets/icon.svg` |
| `npm run extension` | Package the browser extension for both stores |
| `npm run extension:check` | Run the extension's scripts against a stub DOM |
| `npm run store:shots` | Rebuild the store screenshots at 1280×800 |
| `npm run bump 0.2.0` | Set the version everywhere it has to match |
| `npm run check` | Typecheck the frontend |
| `cargo test` (in `src-tauri/`) | Run the encoder tests |

## How it's put together

```
src/                    Svelte 5 frontend
  lib/state.svelte.ts   Job queue and settings, bridged to Rust
  lib/components/       UI
src-tauri/src/
  encode.rs             Bitrate planning, downscale ladder, two-pass, retry
  ffmpeg.rs             Binary discovery, ffprobe parsing, progress streaming
  download.rs           yt-dlp fetching and link probing
  deeplink.rs           nitrate:// parsing and validation
  tray.rs               Tray icon and popup positioning
  lib.rs                Commands, worker pool, events
extension/src/          Browser extension (one source tree, two manifests)
scripts/                ffmpeg fetch, icon generation, version bumping, checks
```

The tests in `src-tauri/tests/encode_test.rs` build throwaway clips with ffmpeg
and assert the output really does land under the target — including a
deliberately harsh target that forces a downscale, and a check that a preset
left over from a different codec gets sanitised instead of reaching ffmpeg.

`scripts/check-extension.mjs` runs the extension's content script against a stub
DOM, because the browsers' own selectors rot and a parse check catches none of
it. It covers all five sites, a reel feed with two rails, and a Twitch clip row.

## How it hits an exact size

File size is bitrate multiplied by duration, so a target size maps straight onto
a bitrate budget:

```
usable_bits  = target_bytes × 8 × safety_margin
total_kbps   = usable_bits ÷ 1000 ÷ duration
video_kbps   = total_kbps − audio_kbps − muxing_overhead
```

Three things turn that arithmetic into a result you can rely on:

1. **A safety margin** (3% by default) absorbs container overhead and rate
   control drift, so you land *under* the limit rather than exactly on it.
2. **A quality floor.** Below roughly 0.045 bits per pixel per frame, H.264
   falls apart. If the budget can't sustain the source resolution, Nitrate drops
   the frame rate to 30 first, then walks the resolution down a ladder until the
   picture has enough bits to hold together.
3. **Verify, then retry.** Rate control aims, it doesn't promise. If the result
   overshoots, Nitrate rescales the bitrate by exactly how much it missed by and
   re-encodes, up to three attempts.

Newer codecs hold up at lower bitrates, so switching codec also changes when
downscaling kicks in — H.265 and AV1 stay sharp well below where H.264 gives up.

In quality mode there's no budget to work back from, so the estimated size comes
from a bits-per-pixel model: an anchor CRF per encoder family, scaled by the
rule that six points of CRF halves the bitrate. It's an estimate and is labelled
as one — identical footage aside, a still scene and a busy one at the same CRF
differ several times over.

## Encoder speed presets

These don't share a scale: H.264 takes named presets, VP9 a number that counts
*backwards*, AV1 a 0–13 scale, and every GPU vendor another one again. Nitrate
shows the right choices per codec and translates them behind the scenes, and
sanitises a preset left over from a previous codec rather than passing it on.

## Why the `nitrate://` link doesn't trust its caller

Registering a URL scheme means **any web page can fire it**, not only our
extension — the OS routes by scheme, not by sender, and a secret baked into an
extension is readable by anyone who unzips it. There's no way around that.

So the app treats every incoming link as hostile until checked. It:

- accepts only `http` and `https`, never `file:`, `data:` or anything else
- **refuses loopback, LAN and link-local addresses**, so a hostile page can't
  use the downloader to probe your router or a service on localhost
- rate-limits arrivals, so nothing can flood the queue
- always shows the window, so nothing is ever queued out of sight
- **waits for a click by default.** There's a setting to start links
  automatically, and it's off deliberately

`src-tauri/src/deeplink.rs` holds the parsing and the checks, with unit tests
for each rejection.

## Platform support

Windows only for now. The code is cross-platform and the release workflow
already has macOS and Linux entries, but they're commented out until someone has
actually run the result — macOS in particular needs an arm64 ffmpeg build before
it can ship anything trustworthy. See `.github/workflows/release.yml`.

## Releasing

```bash
npm run bump 0.2.0
git commit -am "Release v0.2.0" && git tag v0.2.0 && git push && git push --tags
```

The tag triggers the release workflow, which builds every platform, signs the
updater artifacts and opens a **draft** release.

**Publish the draft** — the updater looks at
`releases/latest/download/latest.json`, and GitHub never resolves `latest` to a
draft.

Updates are signed with minisign, and the app refuses any package that doesn't
match the public key it was built with. This is not Windows code signing and
won't stop the SmartScreen prompt on first install. To verify an installer by
hand, its signature is published in the release's `latest.json`, and the
matching public key is in `src-tauri/tauri.conf.json`.

> **Back up the signing key.** It lives in the repo secrets
> `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, with a
> local copy in `~/.tauri/`. Lose it and every installed copy will reject all
> future updates, because they only trust the public key they shipped with — the
> only way back is asking everyone to reinstall by hand.

## Browser extension

See [docs/store/SUBMISSION.md](store/SUBMISSION.md) for the store listings, and
`extension/src/content.js` for the per-site button placement, which is the part
most likely to need maintenance as those sites change their markup.

Before submitting to Firefox, run Mozilla's own linter over the package — AMO
runs it on upload and rejects the file rather than explaining what to change:

```bash
npx addons-linter extension/dist/nitrate-extension-firefox-*.zip
```

Zero errors and zero warnings is the bar, and it's currently met. Two things in
the manifest exist purely to satisfy it: `data_collection_permissions` set to
`["none"]`, which Mozilla now requires of every add-on, and a minimum Firefox
version of 140 (142 on Android), because that key doesn't exist before then.
