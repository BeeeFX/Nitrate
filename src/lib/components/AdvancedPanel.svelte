<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import {
    AUDIO_BITRATES,
    AUDIO_CODECS,
    CONTAINERS,
    DOWNLOAD_HEIGHTS,
    FPS_CAPS,
    RESOLUTION_CAPS,
    VIDEO_CODECS,
    audioCodecsFor,
    containersFor,
    presetsFor,
  } from "../presets";
  import { app } from "../state.svelte";
  import type { AudioCodec, Container, VideoCodec } from "../types";

  let s = $derived(app.settings);
  let allowedContainers = $derived(containersFor(s.videoCodec));
  let allowedAudio = $derived(audioCodecsFor(s.container));
  let hwAvailable = $derived((app.caps?.hardwareEncoders.length ?? 0) > 0);
  // Each encoder has its own effort scale, so the options change with it.
  let presets = $derived(presetsFor(s.videoCodec, s.hardware));

  let activeCodec = $derived(VIDEO_CODECS.find((c) => c.id === s.videoCodec));

  async function pickFolder() {
    const dir = await app.withDialog(() => open({ directory: true }));
    if (typeof dir === "string") app.update({ outputDir: dir });
  }
</script>

<div class="panel">
  <div class="scroll">
    <!-- Video -->
    <div class="group">
      <div class="group-label">Video</div>

      <div class="seg" role="group" aria-label="Video codec">
        {#each VIDEO_CODECS as codec (codec.id)}
          <button
            class="seg-btn"
            class:active={s.videoCodec === codec.id}
            onclick={() => app.update({ videoCodec: codec.id as VideoCodec })}
          >
            {codec.label}
          </button>
        {/each}
      </div>
      {#if activeCodec}
        <p class="hint">{activeCodec.hint}</p>
      {/if}

      <div class="row">
        <span class="row-label">Container</span>
        <select
          value={s.container}
          onchange={(e) =>
            app.update({ container: e.currentTarget.value as Container })}
        >
          {#each CONTAINERS.filter((c) => allowedContainers.includes(c.id)) as c (c.id)}
            <option value={c.id}>{c.label}</option>
          {/each}
        </select>
      </div>

      <div class="row">
        <span class="row-label">Resolution cap</span>
        <select
          value={String(s.maxHeight)}
          onchange={(e) =>
            app.update({
              maxHeight:
                e.currentTarget.value === "null"
                  ? null
                  : Number(e.currentTarget.value),
            })}
        >
          {#each RESOLUTION_CAPS as cap}
            <option value={String(cap.value)}>{cap.label}</option>
          {/each}
        </select>
      </div>

      <div class="row">
        <span class="row-label">Frame rate</span>
        <select
          value={String(s.maxFps)}
          onchange={(e) =>
            app.update({
              maxFps:
                e.currentTarget.value === "null"
                  ? null
                  : Number(e.currentTarget.value),
            })}
        >
          {#each FPS_CAPS as cap}
            <option value={String(cap.value)}>{cap.label}</option>
          {/each}
        </select>
      </div>

      <div class="row">
        <span class="row-label">Encoder speed</span>
        <select
          value={s.preset}
          onchange={(e) => app.update({ preset: e.currentTarget.value })}
        >
          {#each presets as preset (preset.value)}
            <option value={preset.value}>{preset.label}</option>
          {/each}
        </select>
      </div>
    </div>

    <!-- Audio -->
    <div class="group">
      <div class="group-label">Audio</div>

      <div class="row">
        <span class="row-label">Codec</span>
        <select
          value={s.audioCodec}
          onchange={(e) =>
            app.update({ audioCodec: e.currentTarget.value as AudioCodec })}
        >
          {#each AUDIO_CODECS.filter((a) => allowedAudio.includes(a.id)) as a (a.id)}
            <option value={a.id}>{a.label}</option>
          {/each}
        </select>
      </div>

      <div class="row" class:disabled={s.audioCodec === "none" || s.audioCodec === "copy"}>
        <span class="row-label">Bitrate</span>
        <select
          value={String(s.audioBitrateKbps)}
          disabled={s.audioCodec === "none" || s.audioCodec === "copy"}
          onchange={(e) =>
            app.update({ audioBitrateKbps: Number(e.currentTarget.value) })}
        >
          {#each AUDIO_BITRATES as rate}
            <option value={String(rate)}>{rate} kbps</option>
          {/each}
        </select>
      </div>
    </div>

    <!-- Accuracy and speed -->
    <div class="group">
      <div class="group-label">Quality &amp; speed</div>

      <label class="toggle-row">
        <span class="toggle-text">
          <span class="row-label">Two-pass encoding</span>
          <span class="sub">Much closer to the target size. Roughly 1.6× slower.</span>
        </span>
        <input
          type="checkbox"
          checked={s.twoPass}
          onchange={(e) => app.update({ twoPass: e.currentTarget.checked })}
        />
      </label>

      <label class="toggle-row" class:disabled={!hwAvailable}>
        <span class="toggle-text">
          <span class="row-label">Hardware encoding</span>
          <span class="sub">
            {#if hwAvailable}
              Far faster, but size accuracy is looser.
            {:else}
              No GPU encoder detected on this machine.
            {/if}
          </span>
        </span>
        <input
          type="checkbox"
          checked={s.hardware}
          disabled={!hwAvailable}
          onchange={(e) => app.update({ hardware: e.currentTarget.checked })}
        />
      </label>

      <div class="slider-row">
        <div class="slider-head">
          <span class="row-label">Safety margin</span>
          <span class="value tnum">{Math.round((1 - s.safetyMargin) * 100)}%</span>
        </div>
        <input
          type="range"
          min="1"
          max="15"
          step="1"
          value={Math.round((1 - s.safetyMargin) * 100)}
          oninput={(e) =>
            app.update({ safetyMargin: 1 - Number(e.currentTarget.value) / 100 })}
        />
        <p class="hint">
          Headroom left below the limit. Lower means better quality; higher is
          safer against overshoot.
        </p>
      </div>
    </div>

    <!-- Output -->
    <div class="group">
      <div class="group-label">Output</div>

      <div class="row">
        <span class="row-label">Save to</span>
        <button class="path" onclick={pickFolder} title={s.outputDir ?? "Downloads"}>
          {s.outputDir ? s.outputDir.split(/[\\/]/).pop() : "Downloads"}
        </button>
      </div>

      {#if s.outputDir}
        <button class="reset" onclick={() => app.update({ outputDir: null })}>
          Reset to Downloads
        </button>
      {/if}

      <label class="toggle-row">
        <span class="toggle-text">
          <span class="row-label">Start automatically</span>
          <span class="sub">Begin compressing as soon as files are dropped.</span>
        </span>
        <input
          type="checkbox"
          checked={app.autoStart}
          onchange={(e) => {
            app.autoStart = e.currentTarget.checked;
            void app.persist();
          }}
        />
      </label>
    </div>

    <!-- Links -->
    <div class="group">
      <div class="group-label">Pasted links</div>

      <label class="toggle-row">
        <span class="toggle-text">
          <span class="row-label">Compress automatically</span>
          <span class="sub">
            Off keeps the download as it arrived, ready to crop, trim and
            compress by hand.
          </span>
        </span>
        <input
          type="checkbox"
          checked={s.autoCompressDownloads}
          onchange={(e) =>
            app.update({ autoCompressDownloads: e.currentTarget.checked })}
        />
      </label>

      <div class="row">
        <span class="row-label">Download quality</span>
        <select
          value={String(s.maxDownloadHeight)}
          onchange={(e) =>
            app.update({ maxDownloadHeight: Number(e.currentTarget.value) })}
        >
          {#each DOWNLOAD_HEIGHTS as h (h.value)}
            <option value={String(h.value)}>{h.label}</option>
          {/each}
        </select>
      </div>
      <p class="hint">
        The ceiling on what gets fetched. Pulling 4K only to squeeze it into a
        few megabytes wastes time and bandwidth.
      </p>
    </div>

    <!-- Behaviour -->
    <div class="group">
      <div class="group-label">Behaviour</div>

      <label class="toggle-row">
        <span class="toggle-text">
          <span class="row-label">Start with the computer</span>
          <span class="sub">Launch to the tray when you sign in.</span>
        </span>
        <input
          type="checkbox"
          checked={app.launchAtLogin}
          onchange={(e) => app.setLaunchAtLogin(e.currentTarget.checked)}
        />
      </label>

      <label class="toggle-row">
        <span class="toggle-text">
          <span class="row-label">Keep the window open</span>
          <span class="sub">Stay visible when you click elsewhere.</span>
        </span>
        <input
          type="checkbox"
          checked={app.pinned}
          onchange={(e) => app.setPinned(e.currentTarget.checked)}
        />
      </label>
    </div>
  </div>
</div>

<style>
  .panel {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    background: rgba(16, 17, 25, 0.9);
    backdrop-filter: blur(18px);
    animation: slide 0.24s var(--ease-spring);
    z-index: 5;
  }

  @keyframes slide {
    from {
      opacity: 0;
      transform: translateY(10px);
    }
  }

  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 12px 14px 18px;
  }

  .group {
    margin-bottom: 18px;
  }

  .group-label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--text-faint);
    margin-bottom: 9px;
  }

  .seg {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 3px;
    padding: 3px;
    border-radius: var(--radius);
    background: rgba(0, 0, 0, 0.26);
  }

  .seg-btn {
    padding: 6px 4px;
    border-radius: var(--radius-sm);
    font-size: 11px;
    font-weight: 600;
    color: var(--text-dim);
    transition: background 0.15s, color 0.15s;
  }

  .seg-btn:hover {
    color: var(--text);
    background: var(--surface-hover);
  }

  .seg-btn.active {
    background: var(--blurple);
    color: #fff;
    box-shadow: 0 3px 12px -4px rgba(88, 101, 242, 0.95);
  }

  .hint {
    margin-top: 6px;
    font-size: 10.5px;
    line-height: 1.45;
    color: var(--text-faint);
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 7px 0;
  }

  .row.disabled {
    opacity: 0.42;
  }

  .row-label {
    font-size: 12px;
    font-weight: 550;
  }

  select {
    min-width: 108px;
    padding: 5px 8px;
    border-radius: var(--radius-sm);
    background: var(--surface);
    border: 1px solid var(--hairline);
    font-size: 11.5px;
    outline: none;
    cursor: pointer;
  }

  select:hover:not(:disabled) {
    background: var(--surface-hover);
  }

  select:disabled {
    cursor: default;
  }

  /* The dropdown list itself is OS-rendered, so it needs an opaque background. */
  select option {
    background: #1a1c26;
    color: var(--text);
  }

  .toggle-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 0;
    cursor: pointer;
  }

  .toggle-row.disabled {
    opacity: 0.45;
    cursor: default;
  }

  .toggle-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .sub {
    font-size: 10.5px;
    line-height: 1.4;
    color: var(--text-faint);
  }

  input[type="checkbox"] {
    appearance: none;
    position: relative;
    flex-shrink: 0;
    width: 36px;
    height: 20px;
    border-radius: 99px;
    background: rgba(255, 255, 255, 0.13);
    cursor: pointer;
    transition: background 0.18s;
  }

  input[type="checkbox"]::after {
    content: "";
    position: absolute;
    top: 3px;
    left: 3px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    transition: transform 0.2s var(--ease-spring);
  }

  input[type="checkbox"]:checked {
    background: var(--blurple);
    box-shadow: 0 0 12px -2px rgba(88, 101, 242, 0.85);
  }

  input[type="checkbox"]:checked::after {
    transform: translateX(16px);
  }

  input[type="checkbox"]:disabled {
    cursor: default;
  }

  .slider-row {
    padding: 8px 0 2px;
  }

  .slider-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-bottom: 8px;
  }

  .value {
    font-size: 11.5px;
    font-weight: 700;
    color: var(--blurple-bright);
  }

  input[type="range"] {
    appearance: none;
    width: 100%;
    height: 4px;
    border-radius: 99px;
    background: rgba(255, 255, 255, 0.12);
    outline: none;
    cursor: pointer;
  }

  input[type="range"]::-webkit-slider-thumb {
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--blurple-bright);
    box-shadow: 0 0 10px rgba(88, 101, 242, 0.9);
    cursor: pointer;
  }

  .path {
    max-width: 150px;
    padding: 5px 9px;
    border-radius: var(--radius-sm);
    background: var(--surface);
    border: 1px solid var(--hairline);
    font-size: 11.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .path:hover {
    background: var(--surface-hover);
    border-color: var(--hairline-bright);
  }

  .reset {
    font-size: 10.5px;
    color: var(--blurple-bright);
    padding: 2px 0 6px;
  }

  .reset:hover {
    text-decoration: underline;
  }
</style>
