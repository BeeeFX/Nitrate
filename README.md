<div align="center">

<img src="docs/banner.png" alt="Nitrate — drag, drop, done. Any video, under Discord's limit." width="100%">

[![CI](https://github.com/BeeeFX/Nitrate/actions/workflows/ci.yml/badge.svg)](https://github.com/BeeeFX/Nitrate/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/BeeeFX/Nitrate?display_name=tag&color=5865F2)](https://github.com/BeeeFX/Nitrate/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/BeeeFX/Nitrate/total?color=5865F2)](https://github.com/BeeeFX/Nitrate/releases)
[![Licence](https://img.shields.io/badge/licence-MIT-5865F2)](LICENSE)

**Discord won't take your video? Drop it here.**

Nitrate lives in your system tray. Click the icon, drop in a video, and it comes
back in your Downloads folder small enough to send — without paying for Nitro.

[**Download for Windows**](https://github.com/BeeeFX/Nitrate/releases/latest) · [How it works](#how-it-hits-an-exact-size) · [Build from source](#building-from-source)

</div>

---

## See it

<table>
<tr>
<td width="33%" valign="top">
<img src="docs/screenshot-empty.png" alt="The drop zone, with target size tabs for 10, 50, 100 and 500 MB" width="100%">
<p align="center"><sub><b>Pick a size, drop a file</b></sub></p>
</td>
<td width="33%" valign="top">
<img src="docs/screenshot-progress.png" alt="Three videos in the queue, two encoding with progress bars and time remaining" width="100%">
<p align="center"><sub><b>Every file, its own progress</b></sub></p>
</td>
<td width="33%" valign="top">
<img src="docs/screenshot-done.png" alt="Three finished videos showing 167 MB down to 9.3 MB, 68 MB to 9.7 MB and 46 MB to 9.9 MB" width="100%">
<p align="center"><sub><b>167 MB → 9.3 MB</b></sub></p>
</td>
</tr>
</table>

## What it does

- **Drop as many videos as you like.** Each gets its own thumbnail, progress bar
  and time remaining. Compression runs in the background while you carry on.
- **It actually hits the target.** A 10 MB limit produces a ~9.7 MB file — not an
  11 MB one that Discord bounces.
- **It protects the picture.** Rather than smearing a long 4K clip into an
  unwatchable 10 MB, it steps the resolution and frame rate down until the
  bitrate is genuinely enough, and tells you what it did.
- **Nothing is uploaded.** Every frame is encoded on your own machine.
- **It gets out of the way.** Files land in Downloads. Close the window and it
  waits in the tray.

Also handles **"Open with Nitrate"** — right-click any video, or drop one onto
the shortcut, and it starts straight away.

## Install

Grab the installer from [Releases](https://github.com/BeeeFX/Nitrate/releases/latest)
and run it. It installs for your user only, so there's no admin prompt.

> **Windows will warn you the first time.** The app isn't code-signed — a
> certificate costs a few hundred a year, which is hard to justify for a free
> tool. Click **More info → Run anyway**. Updates *are* cryptographically signed;
> that's a separate thing, covered below.

macOS and Linux builds come out of the same CI pipeline but are **untested** —
treat them as experimental.

### Updates take care of themselves

Nitrate checks for updates on launch and every six hours. When one appears you
get a banner at the top of the window: **Update** downloads it, **Restart**
applies it. If something is still encoding, the restart button waits until the
queue is clear so nothing in flight gets thrown away.

Every update is signed, and the app refuses any package that doesn't match the
public key it was built with.

## How it hits an exact size

File size is just bitrate multiplied by duration, so a target size maps straight
onto a bitrate budget:

```
usable_bits  = target_bytes × 8 × safety_margin
total_kbps   = usable_bits ÷ 1000 ÷ duration
video_kbps   = total_kbps − audio_kbps − muxing_overhead
```

Three things turn that arithmetic into a result you can rely on:

1. **A safety margin** (3% by default) absorbs container overhead and rate
   control drift, so you land *under* the limit rather than exactly on it.
2. **A quality floor.** Below roughly 0.045 bits per pixel per frame, H.264 falls
   apart. If the budget can't sustain the source resolution, Nitrate drops the
   frame rate to 30 first, then walks the resolution down a ladder until the
   picture has enough bits to hold together.
3. **Verify, then retry.** Rate control aims, it doesn't promise. If the result
   overshoots, Nitrate rescales the bitrate by exactly how much it missed by and
   re-encodes, up to three attempts.

Newer codecs hold up at lower bitrates, so switching codec also changes when
downscaling kicks in — H.265 and AV1 stay sharp well below where H.264 gives up.

## Settings

<table>
<tr>
<td width="46%" valign="top">
<img src="docs/screenshot-settings.png" alt="Advanced settings: codec, container, resolution cap, frame rate, encoder speed, audio, two-pass and hardware encoding toggles, and a safety margin slider" width="100%">
</td>
<td valign="top">

Sensible defaults, with everything exposed if you want it:

| | |
| --- | --- |
| **Codec** | H.264, H.265, VP9, AV1 |
| **Container** | MP4, WebM, MKV |
| **Resolution / frame rate** | Auto, or capped by hand |
| **Audio** | AAC, Opus, copy, or none |
| **Two-pass** | Much closer to the target. ~1.6× slower |
| **Hardware encoding** | Uses your GPU. Far faster, less precise |
| **Safety margin** | How much headroom to leave |
| **Output folder** | Downloads by default |
| **Start with the computer** | Launch to the tray at sign-in |

Encoder speed options follow the codec you pick, because they don't share a
scale: H.264 takes named presets, VP9 a number that counts *backwards*, AV1 a
0–13 scale, and every GPU vendor another one again. Nitrate shows the right
choices and translates them behind the scenes.

</td>
</tr>
</table>

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
| `npm run bump 0.2.0` | Set the version everywhere it has to match |
| `npm run check` | Typecheck the frontend |
| `cargo test` (in `src-tauri/`) | Run the encoder tests |

### How it's put together

```
src/                    Svelte 5 frontend
  lib/state.svelte.ts   Job queue and settings, bridged to Rust
  lib/components/       UI
src-tauri/src/
  encode.rs             Bitrate planning, downscale ladder, two-pass, retry
  ffmpeg.rs             Binary discovery, ffprobe parsing, progress streaming
  tray.rs               Tray icon and popup positioning
  lib.rs                Commands, worker pool, events
scripts/                ffmpeg fetch, icon generation, version bumping
```

The tests in `src-tauri/tests/encode_test.rs` build throwaway clips with ffmpeg
and assert the output really does land under the target — including a
deliberately harsh target that forces a downscale, and a check that a preset left
over from a different codec gets sanitised instead of reaching ffmpeg.

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

> **Back up the signing key.** It lives in the repo secrets
> `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, with a
> local copy in `~/.tauri/`. Lose it and every installed copy will reject all
> future updates, because they only trust the public key they shipped with — the
> only way back is asking everyone to reinstall by hand.

## Licence

Nitrate's own source is MIT. Released binaries bundle GPL v3 ffmpeg builds, which
carries obligations if you redistribute them — see
[THIRD-PARTY.md](THIRD-PARTY.md).

Not affiliated with Discord. "Discord" and "Nitro" are trademarks of Discord Inc.
