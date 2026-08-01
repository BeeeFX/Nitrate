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

## Others

| Component | License |
| --- | --- |
| [Tauri](https://tauri.app) | MIT / Apache-2.0 |
| [Svelte](https://svelte.dev) | MIT |
| [Vite](https://vite.dev) | MIT |
| [sharp](https://sharp.pixelplumbing.com) (build-time only) | Apache-2.0 |
