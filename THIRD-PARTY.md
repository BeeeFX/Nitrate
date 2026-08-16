# Third-party components

## ffmpeg / ffprobe — GPL v3

Nitrate does not implement video encoding itself. It drives **ffmpeg**, which
released builds bundle as a sidecar binary.

The bundled builds come from
[BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds) and are the
**GPL v3** variants. They have to be: the encoders that make this app useful —
`libx264` and `libx265` — are themselves GPL-licensed, and an LGPL ffmpeg build
cannot encode H.264 or H.265 at all.

### What that means

Nitrate's own source code is MIT (see `LICENSE`). But **a distributed build that
bundles GPL ffmpeg is a combined work, and the combination must be distributed
under the GPL v3.** In practice, if you publish binaries:

- Ship this notice alongside them.
- Point users to the exact ffmpeg build you bundled, and to its source. BtbN's
  releases link their source revision; the pinned version is in
  `scripts/fetch-ffmpeg.mjs`.
- Keep Nitrate's own source available — it already is, which satisfies the
  bulk of the obligation.

This is the same arrangement used by HandBrake, Shotcut and OBS. It is not a
problem for an open-source project; it *would* be a problem for a closed-source
commercial one.

### Avoiding the GPL

If you need a permissively-licensed build, swap the download URLs in
`scripts/fetch-ffmpeg.mjs` for LGPL builds and restrict the codec list to VP9,
AV1 and Opus. You lose H.264 and H.265 — which for a Discord-focused tool is a
significant loss, since H.264 is what plays inline everywhere.

## yt-dlp — Unlicense (public domain)

Pasting a link uses [yt-dlp](https://github.com/yt-dlp/yt-dlp), which is released
into the public domain. It carries no licensing obligations, and unlike ffmpeg it
is **not** bundled — it's fetched into your app data folder the first time you
paste a link, and refreshed weekly.

That's deliberate. Sites change their players constantly and yt-dlp ships
frequent releases to keep up; a copy frozen at Nitrate's release date would stop
working within weeks. Since a link can only be fetched online anyway, downloading
the tool on demand costs nothing.

yt-dlp refuses DRM-protected content, and downloading from some sites is against
their terms of service — that's between you and the site.

## QuickJS — MIT

YouTube requires a JavaScript engine to answer its player challenge, and yt-dlp
has deprecated extraction without one. Nitrate fetches
[quickjs-ng](https://github.com/quickjs-ng/quickjs) for the purpose — two
megabytes against Deno's thirty-eight, which is the only reason it was chosen
over yt-dlp's own recommendation.

Like yt-dlp it is fetched into your app data folder rather than bundled, but
unlike yt-dlp it is **pinned** to a version whose checksum is recorded in
`src-tauri/src/download.rs`. It carries no obligations beyond keeping its notice.

## gallery-dl — GPL v2

Photo posts that yt-dlp cannot reach fall back to
[gallery-dl](https://codeberg.org/mikf/gallery-dl). It is fetched on demand and
pinned, exactly like QuickJS.

It is GPL-licensed, but nothing here links against it: Nitrate runs it as a
separate program, which is the arrangement the GPL is happy with. No combined
work is distributed, because the binary is never shipped with Nitrate.

## Others

| Component | License |
| --- | --- |
| [Tauri](https://tauri.app) | MIT / Apache-2.0 |
| [Svelte](https://svelte.dev) | MIT |
| [Vite](https://vite.dev) | MIT |
| [tauri-plugin-drag](https://crates.io/crates/tauri-plugin-drag) | MIT / Apache-2.0 |
| [sharp](https://sharp.pixelplumbing.com) (build-time only) | Apache-2.0 |
| [addons-linter](https://github.com/mozilla/addons-linter) (build-time only) | MPL-2.0 |
