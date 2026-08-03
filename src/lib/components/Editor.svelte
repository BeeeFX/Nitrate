<script lang="ts">
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { formatBitrate, formatDuration, formatSize } from "../format";
  import { ASPECTS, VIDEO_CODECS } from "../presets";
  import { app } from "../state.svelte";
  import type { CropRect, Job, VideoCodec } from "../types";
  import TargetPicker from "./TargetPicker.svelte";

  interface Props {
    job: Job;
  }

  let { job }: Props = $props();

  const duration = $derived(job.info?.duration ?? 0);
  const sourceW = $derived(job.info?.width ?? 0);
  const sourceH = $derived(job.info?.height ?? 0);

  // Playback is preferred, but the webview can only decode some formats — MKV
  // and ProRes it simply won't touch. Those fall back to stills pulled with
  // ffmpeg, which is slower to scrub but works for everything.
  let videoSrc = $state<string | null>(null);
  let videoEl = $state<HTMLVideoElement | null>(null);
  let canPlay = $state(false);
  let playing = $state(false);

  let frame = $state<string | null>(null);
  let strip = $state<string[]>([]);
  let loadingFrame = $state(false);

  // Seeded from the job on purpose — App remounts this component per job with
  // a keyed block, so there's no stale-prop hazard to guard against.
  /* svelte-ignore state_referenced_locally */
  let playhead = $state(job.edits.start ?? 0);
  /* svelte-ignore state_referenced_locally */
  let inPoint = $state(job.edits.start ?? 0);
  /* svelte-ignore state_referenced_locally */
  let outPoint = $state<number | null>(job.edits.end);
  /* svelte-ignore state_referenced_locally */
  let crop = $state<CropRect | null>(job.edits.crop);
  let aspect = $state("free");

  let previewEl = $state<HTMLElement | null>(null);
  let stripEl = $state<HTMLElement | null>(null);

  const effectiveOut = $derived(outPoint ?? duration);
  const trimmed = $derived(Math.max(0.1, effectiveOut - inPoint));
  const isTrimmed = $derived(inPoint > 0.01 || effectiveOut < duration - 0.01);

  // What the crop leaves behind, in real pixels — the number that decides
  // whether the target can hold the resolution.
  const croppedW = $derived(crop ? Math.round(sourceW * crop.width) : sourceW);
  const croppedH = $derived(crop ? Math.round(sourceH * crop.height) : sourceH);

  /**
   * The preview box is given the video's own shape, as a plain number so the
   * stylesheet can do arithmetic with it.
   *
   * It can't be left to the media's intrinsic size: a percentage max-height
   * resolves to nothing against a parent whose height is set by its own
   * content, so a tall video kept its full height and the box cut the bottom
   * off. See the `.preview` rule for why the fit is computed rather than
   * clamped.
   */
  const mediaRatio = $derived(
    sourceW > 0 && sourceH > 0 ? sourceW / sourceH : 16 / 9,
  );

  // This video's own compression settings, which start as whatever the main
  // interface says and can be changed here without affecting anything else.
  const settings = $derived(app.settingsFor(job));
  const sizeMode = $derived(settings.mode === "size");

  const alreadyFits = $derived(
    sizeMode &&
      (job.info?.sizeBytes ?? 0) > 0 &&
      (job.info?.sizeBytes ?? 0) <= settings.targetBytes,
  );
  const dirty = $derived(isTrimmed || crop !== null);
  // Nothing to do only when it already fits *and* hasn't been edited.
  const canCompress = $derived(dirty || !alreadyFits);

  // ---------------------------------------------------------------------
  // Preview source
  // ---------------------------------------------------------------------

  $effect(() => {
    if (!job.path) return;
    void invoke<string>("allow_preview", { path: job.path })
      .then((p) => (videoSrc = convertFileSrc(p)))
      .catch(() => (videoSrc = null));
  });

  async function loadFrame(at: number) {
    if (!job.path || canPlay) return;
    loadingFrame = true;
    try {
      frame = await invoke<string>("frame_at", { path: job.path, time: at });
    } catch {
      // Leave the previous frame up rather than flashing an empty box.
    } finally {
      loadingFrame = false;
    }
  }

  // Scrubbing stills would otherwise fire an ffmpeg call per pixel of movement.
  let frameTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const at = playhead;
    if (canPlay) return;
    clearTimeout(frameTimer);
    frameTimer = setTimeout(() => void loadFrame(at), 110);
    return () => clearTimeout(frameTimer);
  });

  $effect(() => {
    if (!job.path) return;
    void invoke<string[]>("filmstrip", { path: job.path, count: 12 })
      .then((f) => (strip = f))
      .catch(() => (strip = []));
  });

  // Push edits back to the store so the plan readout recalculates.
  let commitTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const next = {
      start: inPoint > 0.01 ? inPoint : null,
      end: effectiveOut < duration - 0.01 ? effectiveOut : null,
      crop: crop ? { ...crop } : null,
    };
    clearTimeout(commitTimer);
    commitTimer = setTimeout(() => app.setEdits(job.id, next), 180);
    return () => clearTimeout(commitTimer);
  });

  // ---------------------------------------------------------------------
  // Playback
  // ---------------------------------------------------------------------

  let raf = 0;

  function follow() {
    if (!videoEl || !playing) return;
    playhead = videoEl.currentTime;
    // Looping the selection is what makes a trim easy to judge.
    if (videoEl.currentTime >= effectiveOut) {
      videoEl.currentTime = inPoint;
    }
    raf = requestAnimationFrame(follow);
  }

  function togglePlay() {
    if (!videoEl || !canPlay) return;
    if (playing) {
      videoEl.pause();
      return;
    }
    if (videoEl.currentTime < inPoint || videoEl.currentTime >= effectiveOut) {
      videoEl.currentTime = inPoint;
    }
    void videoEl.play();
  }

  function onPlay() {
    playing = true;
    raf = requestAnimationFrame(follow);
  }

  function onPause() {
    playing = false;
    cancelAnimationFrame(raf);
  }

  function seek(to: number) {
    playhead = to;
    if (videoEl && canPlay) videoEl.currentTime = to;
  }

  $effect(() => () => cancelAnimationFrame(raf));

  // ---------------------------------------------------------------------
  // Crop
  // ---------------------------------------------------------------------

  function ratioFor(id: string): number | null {
    return ASPECTS.find((a) => a.id === id)?.ratio ?? null;
  }

  /** Builds a starting rectangle that fills as much of the frame as the
   *  chosen aspect allows, centred. */
  function applyAspect(id: string) {
    aspect = id;
    const ratio = ratioFor(id);
    if (ratio === null) {
      if (!crop) crop = { x: 0.1, y: 0.1, width: 0.8, height: 0.8 };
      return;
    }
    if (sourceW === 0 || sourceH === 0) return;

    const sourceRatio = sourceW / sourceH;
    let w = 1;
    let h = 1;
    if (ratio > sourceRatio) {
      h = sourceRatio / ratio;
    } else {
      w = ratio / sourceRatio;
    }
    crop = { x: (1 - w) / 2, y: (1 - h) / 2, width: w, height: h };
  }

  function clearCrop() {
    crop = null;
    aspect = "free";
  }

  type DragMode = "move" | "nw" | "ne" | "sw" | "se";
  let drag: { mode: DragMode; startX: number; startY: number; rect: CropRect } | null =
    null;

  function beginDrag(event: PointerEvent, mode: DragMode) {
    if (!crop) return;
    event.stopPropagation();
    event.preventDefault();
    drag = { mode, startX: event.clientX, startY: event.clientY, rect: { ...crop } };
  }

  function onDrag(event: PointerEvent) {
    if (!drag || !crop || !previewEl) return;
    const box = previewEl.getBoundingClientRect();
    const dx = (event.clientX - drag.startX) / box.width;
    const dy = (event.clientY - drag.startY) / box.height;
    const start = drag.rect;
    const ratio = ratioFor(aspect);

    if (drag.mode === "move") {
      crop = {
        ...start,
        x: clamp(start.x + dx, 0, 1 - start.width),
        y: clamp(start.y + dy, 0, 1 - start.height),
      };
      return;
    }

    // Corner resize: anchor the opposite corner and move this one.
    const right = start.x + start.width;
    const bottom = start.y + start.height;
    let x = start.x;
    let y = start.y;
    let w = start.width;
    let h = start.height;

    if (drag.mode === "se") {
      w = clamp(start.width + dx, 0.05, 1 - start.x);
      h = clamp(start.height + dy, 0.05, 1 - start.y);
    } else if (drag.mode === "sw") {
      x = clamp(start.x + dx, 0, right - 0.05);
      w = right - x;
      h = clamp(start.height + dy, 0.05, 1 - start.y);
    } else if (drag.mode === "ne") {
      w = clamp(start.width + dx, 0.05, 1 - start.x);
      y = clamp(start.y + dy, 0, bottom - 0.05);
      h = bottom - y;
    } else {
      x = clamp(start.x + dx, 0, right - 0.05);
      w = right - x;
      y = clamp(start.y + dy, 0, bottom - 0.05);
      h = bottom - y;
    }

    if (ratio !== null && sourceW > 0 && sourceH > 0) {
      const sourceRatio = sourceW / sourceH;
      h = (w * sourceRatio) / ratio;
      if (h > 1) {
        h = 1;
        w = (h * ratio) / sourceRatio;
      }
      if (drag.mode === "nw" || drag.mode === "ne") y = bottom - h;
      if (drag.mode === "nw" || drag.mode === "sw") x = right - w;
      y = clamp(y, 0, 1 - h);
      x = clamp(x, 0, 1 - w);
    }

    crop = { x, y, width: w, height: h };
  }

  function clamp(v: number, lo: number, hi: number) {
    return Math.min(Math.max(v, lo), hi);
  }

  // Tracking on the window rather than the elements means a drag keeps working
  // when the pointer leaves the preview, which is exactly when you're pulling a
  // crop edge outwards.
  function onWindowMove(event: PointerEvent) {
    onDrag(event);
    onScrub(event);
  }

  function onWindowUp() {
    drag = null;
    scrub = null;
  }

  // ---------------------------------------------------------------------
  // Timeline
  // ---------------------------------------------------------------------

  type Handle = "in" | "out" | "playhead";
  let scrub: Handle | null = null;

  function timeFromEvent(event: PointerEvent): number {
    if (!stripEl) return 0;
    const box = stripEl.getBoundingClientRect();
    return clamp((event.clientX - box.left) / box.width, 0, 1) * duration;
  }

  function beginScrub(event: PointerEvent, handle: Handle) {
    event.stopPropagation();
    event.preventDefault();
    scrub = handle;
    if (playing) videoEl?.pause();
    onScrub(event);
  }

  function onScrub(event: PointerEvent) {
    if (!scrub) return;
    const t = timeFromEvent(event);
    if (scrub === "in") {
      inPoint = Math.min(t, effectiveOut - 0.2);
      seek(inPoint);
    } else if (scrub === "out") {
      outPoint = Math.max(t, inPoint + 0.2);
      seek(outPoint);
    } else {
      seek(t);
    }
  }

  function resetTrim() {
    inPoint = 0;
    outPoint = null;
    seek(0);
  }

  const pct = (t: number) => (duration > 0 ? (t / duration) * 100 : 0);

  async function compress() {
    await app.closeEditor();
    await app.start(job.id);
  }
</script>

<svelte:window onpointermove={onWindowMove} onpointerup={onWindowUp} />

<div class="editor">
  <header class="bar">
    <button class="back" onclick={() => app.closeEditor()}>
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M15 5l-7 7 7 7"
          stroke="currentColor"
          stroke-width="2.2"
          fill="none"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
      Back
    </button>
    <span class="filename" title={job.name}>{job.name}</span>
  </header>

  <div class="stage">
    <div
      class="preview"
      bind:this={previewEl}
      style:--media-ratio={mediaRatio}
    >
      {#if videoSrc}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video
          bind:this={videoEl}
          src={videoSrc}
          class:hidden={!canPlay}
          muted
          playsinline
          preload="auto"
          oncanplay={() => (canPlay = true)}
          onerror={() => {
            canPlay = false;
            videoSrc = null;
          }}
          onplay={onPlay}
          onpause={onPause}
        ></video>
      {/if}

      {#if !canPlay}
        {#if frame}
          <img src={frame} alt="" draggable="false" />
        {:else}
          <div class="frame-loading">Reading frame…</div>
        {/if}
      {/if}

      {#if crop}
        <div
          class="crop"
          role="application"
          aria-label="Crop area"
          style:left="{crop.x * 100}%"
          style:top="{crop.y * 100}%"
          style:width="{crop.width * 100}%"
          style:height="{crop.height * 100}%"
          onpointerdown={(e) => beginDrag(e, "move")}
        >
          <button
            class="handle nw"
            aria-label="Resize from top left"
            onpointerdown={(e) => beginDrag(e, "nw")}
          ></button>
          <button
            class="handle ne"
            aria-label="Resize from top right"
            onpointerdown={(e) => beginDrag(e, "ne")}
          ></button>
          <button
            class="handle sw"
            aria-label="Resize from bottom left"
            onpointerdown={(e) => beginDrag(e, "sw")}
          ></button>
          <button
            class="handle se"
            aria-label="Resize from bottom right"
            onpointerdown={(e) => beginDrag(e, "se")}
          ></button>
        </div>
      {/if}

      {#if loadingFrame && !canPlay}<div class="spinner"></div>{/if}
    </div>
  </div>

  <div class="controls">
    <div class="row">
      <span class="label">Crop</span>
      <div class="chips">
        {#each ASPECTS as a (a.id)}
          <button
            class="chip"
            class:active={crop !== null && aspect === a.id}
            onclick={() => applyAspect(a.id)}
          >
            {a.label}
          </button>
        {/each}
        {#if crop}
          <button class="chip ghost" onclick={clearCrop}>Clear</button>
        {/if}
      </div>
    </div>

    <div class="timeline">
      <div
        class="strip"
        bind:this={stripEl}
        role="slider"
        tabindex="0"
        aria-label="Playhead"
        aria-valuemin={0}
        aria-valuemax={duration}
        aria-valuenow={playhead}
        onpointerdown={(e) => beginScrub(e, "playhead")}
      >
        {#each strip as src, i (i)}
          <img src={src} alt="" draggable="false" />
        {/each}
        {#if strip.length === 0}
          <div class="strip-empty"></div>
        {/if}

        <div class="mask left" style:width="{pct(inPoint)}%"></div>
        <div class="mask right" style:width="{100 - pct(effectiveOut)}%"></div>

        <div class="playhead" style:left="{pct(playhead)}%"></div>

        <button
          class="handle-bar in"
          style:left="{pct(inPoint)}%"
          aria-label="Trim start"
          onpointerdown={(e) => beginScrub(e, "in")}
        ></button>
        <button
          class="handle-bar out"
          style:left="{pct(effectiveOut)}%"
          aria-label="Trim end"
          onpointerdown={(e) => beginScrub(e, "out")}
        ></button>
      </div>

      <div class="times tnum">
        <div class="left-group">
          <button
            class="play"
            onclick={togglePlay}
            disabled={!canPlay}
            title={canPlay
              ? "Play the selection"
              : "This format can't be played here — scrub the timeline instead"}
            aria-label={playing ? "Pause" : "Play"}
          >
            {#if playing}
              <svg viewBox="0 0 24 24"
                ><path d="M8 5h3v14H8zM13 5h3v14h-3z" fill="currentColor" /></svg
              >
            {:else}
              <svg viewBox="0 0 24 24"
                ><path d="M8 5l11 7-11 7z" fill="currentColor" /></svg
              >
            {/if}
          </button>
          <span>{formatDuration(inPoint)}</span>
        </div>

        <span class="mid">
          {formatDuration(trimmed)} selected
          {#if isTrimmed}
            <button class="reset" onclick={resetTrim}>reset</button>
          {/if}
        </span>

        <span>{formatDuration(effectiveOut)}</span>
      </div>
    </div>

    <!-- The same target control as the main window, but writing to this video
         only — so "Compress" is never a mystery about what it will produce. -->
    <div class="compression">
      <div class="row">
        <span class="label">Compress to</span>
        {#if job.settings}
          <button class="linkish" onclick={() => app.clearJobSettings(job.id)}>
            reset to default
          </button>
        {/if}
      </div>

      <TargetPicker
        compact
        settings={settings}
        onChange={(patch) => app.updateJobSettings(job.id, patch)}
      />

      <div class="row codec-row">
        <span class="label">Codec</span>
        <div class="chips">
          {#each VIDEO_CODECS as codec (codec.id)}
            <button
              class="chip"
              class:active={settings.videoCodec === codec.id}
              title={codec.hint}
              onclick={() =>
                app.updateJobSettings(job.id, {
                  videoCodec: codec.id as VideoCodec,
                })}
            >
              {codec.label}
            </button>
          {/each}
        </div>
      </div>
    </div>

    <div class="summary">
      <div class="readout tnum">
        {#if job.plan}
          <span class="dim">{croppedW}×{croppedH}</span>
          <span class="sep">→</span>
          <strong class:warn={job.plan.height < croppedH}>
            {job.plan.width}×{job.plan.height}
          </strong>
          <span class="sep">·</span>
          {#if job.plan.mode === "quality"}
            <span>quality {settings.quality}</span>
            <span class="sep">·</span>
            {#if job.plan.estimatedBytes}
              <span
                class="target estimate"
                title="A rough guess from the quality setting and the frame size. Busy footage runs larger, still footage smaller."
              >
                ≈{formatSize(job.plan.estimatedBytes)}
              </span>
            {:else}
              <span class="target">size not fixed</span>
            {/if}
          {:else}
            <span>{formatBitrate(job.plan.videoKbps)}</span>
            <span class="sep">·</span>
            <span class="target">{formatSize(settings.targetBytes)}</span>
          {/if}
        {:else if alreadyFits && !dirty}
          <span class="dim">Already under {formatSize(settings.targetBytes)}</span>
        {:else}
          <span class="dim">Working out the plan…</span>
        {/if}
      </div>

      <button class="compress" disabled={!canCompress} onclick={compress}>
        {dirty ? "Apply & compress" : "Compress"}
      </button>
    </div>

    {#if job.plan?.notes.length}
      <ul class="notes">
        {#each job.plan.notes as note}<li>{note}</li>{/each}
      </ul>
    {/if}
  </div>
</div>

<style>
  .editor {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    background: rgba(16, 17, 25, 0.94);
    backdrop-filter: blur(18px);
    z-index: 6;
    animation: fade 0.2s var(--ease-spring);
  }

  @keyframes fade {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    flex-shrink: 0;
    border-bottom: 1px solid var(--hairline);
  }

  .back {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 5px 10px 5px 6px;
    border-radius: var(--radius-sm);
    font-size: 12px;
    font-weight: 600;
    color: var(--text-dim);
  }

  .back svg {
    width: 15px;
    height: 15px;
  }

  .back:hover {
    background: var(--surface-hover);
    color: var(--text);
  }

  .filename {
    font-size: 12px;
    color: var(--text-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .stage {
    flex: 1;
    min-height: 0;
    display: grid;
    place-items: center;
    padding: 12px 14px;
    /* Makes the stage a query container so the preview can size itself from
       both of its dimensions. Safe here because the stage's own size comes from
       the flex row above it, never from what's inside it. */
    container-type: size;
  }

  /*
   * Sized to the media exactly, because the crop rectangle is positioned
   * against it — any padding here would offset the crop.
   *
   * The fit is computed rather than clamped. max-width/max-height don't manage
   * it: once a width is stated, clamping the height doesn't send the constraint
   * back through the ratio, so the box stretches and the video is cut off —
   * measured in this same engine, where every aspect came back the identical
   * stretched size. The container-query units give both of the stage's
   * dimensions, so the width can be the smaller of "all of it" and "as much as
   * the height allows", which is the fit itself rather than an approximation.
   */
  .preview {
    position: relative;
    aspect-ratio: var(--media-ratio, 1.7778);
    width: min(100cqw, calc(100cqh * var(--media-ratio, 1.7778)));
    line-height: 0;
    border-radius: var(--radius);
    overflow: hidden;
    background: rgba(0, 0, 0, 0.4);
    touch-action: none;
  }

  .preview img,
  .preview video {
    display: block;
    width: 100%;
    height: 100%;
    /* The box already has the media's shape, so this only ever matters if the
       dimensions were unreadable — letterboxing beats stretching. */
    object-fit: contain;
    user-select: none;
  }

  .preview video.hidden {
    display: none;
  }

  /* Fills the box rather than setting its own size, which would fight the
     aspect ratio and overflow the stage while the first frame is read. */
  .frame-loading {
    display: grid;
    place-items: center;
    width: 100%;
    height: 100%;
    font-size: 12px;
    color: var(--text-faint);
  }

  /* One enormous shadow rather than four bands: exact by construction, and
     it can't leave seams or double-darkened corners. */
  .crop {
    position: absolute;
    border: 1.5px solid var(--blurple-bright);
    box-shadow: 0 0 0 4000px rgba(0, 0, 0, 0.55);
    cursor: move;
    touch-action: none;
  }

  .handle {
    position: absolute;
    width: 13px;
    height: 13px;
    padding: 0;
    background: var(--blurple-bright);
    border-radius: 3px;
    border: 2px solid rgba(10, 12, 18, 0.85);
    touch-action: none;
  }

  .handle.nw {
    top: -7px;
    left: -7px;
    cursor: nwse-resize;
  }
  .handle.ne {
    top: -7px;
    right: -7px;
    cursor: nesw-resize;
  }
  .handle.sw {
    bottom: -7px;
    left: -7px;
    cursor: nesw-resize;
  }
  .handle.se {
    bottom: -7px;
    right: -7px;
    cursor: nwse-resize;
  }

  .spinner {
    position: absolute;
    top: 8px;
    right: 8px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 2px solid rgba(255, 255, 255, 0.25);
    border-top-color: var(--blurple-bright);
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .controls {
    flex-shrink: 0;
    padding: 0 14px 14px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--text-faint);
  }

  .chips {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }

  .chip {
    padding: 5px 11px;
    border-radius: var(--radius-sm);
    background: var(--surface);
    font-size: 11px;
    font-weight: 600;
    color: var(--text-dim);
    transition: background 0.14s, color 0.14s;
  }

  .chip:hover {
    background: var(--surface-hover);
    color: var(--text);
  }

  .chip.active {
    background: var(--blurple);
    color: #fff;
  }

  .chip.ghost {
    color: var(--text-faint);
    background: none;
  }

  .chip.ghost:hover {
    color: var(--danger);
  }

  .timeline {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .strip {
    position: relative;
    display: flex;
    height: 54px;
    border-radius: var(--radius-sm);
    overflow: hidden;
    background: rgba(0, 0, 0, 0.35);
    cursor: pointer;
    touch-action: none;
  }

  .strip img {
    flex: 1;
    min-width: 0;
    height: 100%;
    object-fit: cover;
    user-select: none;
    pointer-events: none;
  }

  .strip-empty {
    flex: 1;
  }

  .mask {
    position: absolute;
    top: 0;
    bottom: 0;
    background: rgba(8, 9, 14, 0.72);
    pointer-events: none;
  }

  .mask.left {
    left: 0;
  }
  .mask.right {
    right: 0;
  }

  .playhead {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    margin-left: -1px;
    background: #fff;
    box-shadow: 0 0 6px rgba(0, 0, 0, 0.8);
    pointer-events: none;
  }

  .handle-bar {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 12px;
    margin-left: -6px;
    padding: 0;
    background: var(--blurple-bright);
    border-radius: 3px;
    box-shadow: 0 0 10px rgba(88, 101, 242, 0.7);
    cursor: ew-resize;
    touch-action: none;
  }

  .times {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 11px;
    color: var(--text-dim);
  }

  .left-group {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .play {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 50%;
    background: var(--surface-strong);
    color: var(--text);
    transition: background 0.14s, opacity 0.14s;
  }

  .play svg {
    width: 13px;
    height: 13px;
  }

  .play:hover:not(:disabled) {
    background: var(--blurple);
  }

  .play:disabled {
    opacity: 0.35;
    cursor: default;
  }

  .mid {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text);
    font-weight: 600;
  }

  .reset {
    font-size: 10.5px;
    font-weight: 500;
    color: var(--blurple-bright);
  }

  .reset:hover {
    text-decoration: underline;
  }

  .compression {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 11px;
    border-radius: var(--radius);
    background: rgba(255, 255, 255, 0.028);
    border: 1px solid var(--hairline);
  }

  .codec-row {
    gap: 10px;
  }

  .linkish {
    font-size: 10.5px;
    color: var(--blurple-bright);
  }

  .linkish:hover {
    text-decoration: underline;
  }

  .summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding-top: 2px;
  }

  .readout {
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-size: 12px;
    flex-wrap: wrap;
  }

  .readout strong {
    font-weight: 700;
    color: var(--success);
  }

  .readout strong.warn {
    color: var(--warn);
  }

  .sep,
  .dim {
    color: var(--text-faint);
  }

  .target {
    color: var(--text-dim);
  }

  /* Dotted underline: the number is a guess, and it should look like one. */
  .estimate {
    text-decoration: underline dotted;
    text-underline-offset: 3px;
    cursor: help;
  }

  .compress {
    flex-shrink: 0;
    padding: 8px 16px;
    border-radius: var(--radius);
    background: var(--blurple);
    color: #fff;
    font-size: 12px;
    font-weight: 650;
    box-shadow: 0 4px 16px -6px rgba(88, 101, 242, 0.9);
    transition: filter 0.14s, opacity 0.14s;
  }

  .compress:hover:not(:disabled) {
    filter: brightness(1.12);
  }

  .compress:disabled {
    opacity: 0.4;
    cursor: default;
    box-shadow: none;
  }

  .notes {
    padding-left: 16px;
    font-size: 10.5px;
    line-height: 1.5;
    color: var(--text-faint);
  }
</style>
