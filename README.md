<div align="center">

# Nitrate

**Drag, drop, done.** Compress any video to fit Discord's upload limits —
without paying for Nitro.

Lives in your system tray. Click the icon, drop in videos, get them back in
your Downloads folder at exactly the size you need.

</div>

---

## What it does

Discord caps free uploads at 10 MB. Nitrate takes a video of any size and lands
it just under that ceiling — or under 50 MB, 100 MB, 500 MB, or any custom size
you type in.

- **Drop several files at once.** Each one gets its own progress and ETA.
- **Actually hits the target.** Two-pass encoding plus a verification step, so a
  10 MB limit produces a ~9.6 MB file, not an 11 MB one that Discord rejects.
- **Protects the picture.** Rather than smearing a long 4K clip into an
  unwatchable 10 MB, it steps the resolution and frame rate down until the
  bitrate is actually enough.
- **Everything is local.** Nothing is uploaded anywhere.
- **Advanced settings when you want them** — codec, container, audio, hardware
  encoding, resolution and frame-rate caps, safety margin, output folder, and
  whether to start with the computer.

Encoder speed options follow whichever codec is selected, since they don't share
a scale: H.264 and H.265 take named presets, VP9 a `cpu-used` number that counts
backwards, AV1 a 0–13 scale, and each GPU vendor another one again. Nitrate
shows the right choices and translates them per encoder.

## Install

Download the Windows installer (`Nitrate_x.y.z_x64-setup.exe`) from
[Releases](https://github.com/BeeeFX/Nitrate/releases) and run it. It installs
per-user, so it doesn't ask for admin rights.

macOS and Linux builds come out of the same CI matrix but are currently
unverified — treat them as untested.

On first launch Nitrate puts an icon in your system tray. Click it to open the
popup; click away to dismiss it, or hit the pin to keep it open.

### Updates

Nitrate checks for updates on launch and every six hours after. When one is
available a banner appears at the top of the popup — click **Update** to
download it, then **Restart** to apply.

Updates are signed, and the app refuses any package that doesn't match its
built-in public key. If an encode is still running, the restart button stays
disabled until the queue is clear so work in progress isn't thrown away.

## How it hits an exact size

File size is just bitrate times duration, so the target maps directly onto a
bitrate budget:

```
usable_bits  = target_bytes × 8 × safety_margin
total_kbps   = usable_bits ÷ 1000 ÷ duration
video_kbps   = total_kbps − audio_kbps − muxing_overhead
```

Three things turn that arithmetic into a reliable result:

1. **A safety margin** (3% by default) absorbs container overhead and rate
   control drift, so you land *under* the limit rather than on it.
2. **A quality floor.** Below roughly 0.045 bits per pixel per frame, H.264
   falls apart. If the budget can't sustain the source resolution, Nitrate drops
   the frame rate to 30 first, then walks the resolution down a ladder until the
   picture has enough bits to look right.
3. **Verify and retry.** Rate control aims, it doesn't promise. If the result
   overshoots, Nitrate rescales the bitrate by the overshoot ratio and re-encodes,
   up to three attempts.

The codecs have different floors — H.265 and AV1 hold up at lower bitrates than
H.264 — so switching codec in Advanced settings also changes when downscaling
kicks in.

## Building from source

Requires [Node](https://nodejs.org) 20+ and [Rust](https://rustup.rs).
On Windows you also need the MSVC build tools; on Linux, the
[Tauri system dependencies](https://tauri.app/start/prerequisites/).

```bash
npm install
```

`npm install` runs a postinstall step that downloads the ffmpeg and ffprobe
sidecars (~260 MB) into `src-tauri/binaries/`. They're deliberately not in git.

```bash
npm run app
```

That starts the app in development mode, with the window shown on launch rather
than hidden in the tray.

| Command | What it does |
| --- | --- |
| `npm run app` | Run in development |
| `npm run app:build` | Build installers into `src-tauri/target/release/bundle/` |
| `npm run ffmpeg` | Re-fetch the ffmpeg sidecars |
| `npm run icons` | Regenerate icons from `assets/icon.svg` |
| `npm run bump 0.2.0` | Set the version everywhere it has to match |
| `npm run check` | Typecheck the frontend |
| `cargo test` (in `src-tauri/`) | Run the encoder tests |

## Cutting a release

```bash
npm run bump 0.2.0
```

Then commit, tag and push:

```bash
git commit -am "Release v0.2.0" && git tag v0.2.0 && git push && git push --tags
```

The tag triggers `.github/workflows/release.yml`, which builds every platform,
signs the updater artifacts and opens a **draft** release.

**Publish the draft.** The updater endpoint points at
`releases/latest/download/latest.json`, and GitHub only resolves `latest` to a
published, non-prerelease release — a draft is invisible to it.

### The signing key

Updates are signed with a keypair generated by `tauri signer generate`. The
public half lives in `src-tauri/tauri.conf.json`; the private half is stored as
the repo secrets `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, with a local backup in `~/.tauri/`.

> **Keep that backup.** If the private key is lost, already-installed copies of
> Nitrate will reject every future update, because they only trust the public
> key they were built with. Recovering means asking every user to reinstall by
> hand. Back the key up somewhere durable.

### Layout

```
src/                    Svelte 5 frontend
  lib/state.svelte.ts   Job queue and settings, bridged to Rust
  lib/components/       UI
src-tauri/src/
  encode.rs             Bitrate planning, downscale ladder, two-pass, retry
  ffmpeg.rs             Binary discovery, ffprobe parsing, progress streaming
  tray.rs               Tray icon and popup positioning
  lib.rs                Commands, worker pool, events
scripts/                ffmpeg fetch and icon generation
```

The encoder tests in `src-tauri/tests/encode_test.rs` build throwaway clips with
ffmpeg and assert the output really does land under the target — including a
deliberately harsh target that forces a downscale.

## Licence

Nitrate's source is MIT. Released binaries bundle GPL v3 ffmpeg builds, which
carries obligations if you redistribute them — see
[THIRD-PARTY.md](THIRD-PARTY.md).

Not affiliated with Discord. "Discord" and "Nitro" are trademarks of Discord Inc.
