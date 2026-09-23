<script lang="ts">
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { formatBitrate, formatDuration, formatSize } from "../format";
  import { ASPECTS, VIDEO_CODECS } from "../presets";
  import { app } from "../state.svelte";
  import type { CropRect, Edits, Job, VideoCodec } from "../types";
  import TargetPicker from "./TargetPicker.svelte";

  interface Props {
    job: Job;
  }

  let { job }: Props = $props();
  /** Ties controls to the hidden notes that explain their keys. */
  const uid = $props.id();

  // Kept apart from the prop because edits are written on the way out, and by
  // then the editor is being torn down. The keyed block in App means the job
  // can't change underneath it.
  /* svelte-ignore state_referenced_locally */
  const jobId = job.id;

  const duration = $derived(job.info?.duration ?? 0);
  const sourceW = $derived(job.info?.width ?? 0);
  const sourceH = $derived(job.info?.height ?? 0);

  /**
   * How far a single frame moves the playhead. A probe can come back without
   * a usable rate, and stepping by a common one beats not stepping at all.
   */
  const frameStep = $derived.by(() => {
    const fps = job.info?.fps ?? 0;
    return Number.isFinite(fps) && fps > 0 ? 1 / fps : 1 / 30;
  });

  // Playback is preferred, and the file itself is tried first. What a webview
  // can decode is not the same list as what this app can produce, though — AV1
  // and HEVC depend on the runtime and the machine, and some containers it
  // won't touch at all. Anything it refuses gets re-encoded to plain H.264 and
  // played from that instead, so every codec here is playable.
  let videoSrc = $state<string | null>(null);
  let videoEl = $state<HTMLVideoElement | null>(null);
  let canPlay = $state(false);
  let playing = $state(false);
  /** Building the stand-in copy. Only ever true briefly, and only once. */
  let proxying = $state(false);
  let proxyTried = $state(false);

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
  /** A photo out of a post: croppable, but there's nothing to trim or play. */
  const isStill = $derived(job.mediaKind === "photo");
  /** Croppable and trimmable like a video, but it comes back out as a GIF. */
  const isGif = $derived(job.mediaKind === "gif");

  const dirty = $derived(isStill ? crop !== null : isTrimmed || crop !== null);
  // Nothing to do only when it already fits *and* hasn't been edited. In keep
  // mode there's no compressing to do at all, so an unedited video is a no-op.
  const canCompress = $derived(
    settings.mode === "keep" ? dirty : dirty || !alreadyFits,
  );

  /**
   * The button says what's about to happen.
   *
   * "Compress" on a job that only trims and copies would be a lie, and the
   * difference matters: one takes a second and changes nothing, the other takes
   * minutes and re-encodes every frame.
   */
  const actionLabel = $derived.by(() => {
    if (settings.mode !== "keep") return dirty ? "Apply & compress" : "Compress";
    if (crop !== null) return "Apply & re-encode";
    return "Apply edits";
  });

  // ---------------------------------------------------------------------
  // Preview source
  // ---------------------------------------------------------------------

  $effect(() => {
    if (!job.path) return;
    proxyTried = false;
    void invoke<string>("allow_preview", { path: job.path })
      .then((p) => (videoSrc = convertFileSrc(p)))
      .catch(() => (videoSrc = null));
  });

  /**
   * Re-encodes to something the webview will definitely play.
   *
   * Tried once per file, and only after the real one has failed — there's no
   * point spending an encode on a video that plays fine as it is.
   */
  async function useProxy() {
    if (proxyTried || !job.path) return;
    proxyTried = true;
    proxying = true;
    try {
      const stand_in = await invoke<string>("preview_proxy", { path: job.path });
      videoSrc = convertFileSrc(stand_in);
    } catch {
      // Nothing more to try. Scrubbing through frames still works, which is
      // what the editor did for these files before there was a proxy at all.
      videoSrc = null;
    } finally {
      proxying = false;
    }
  }

  /**
   * A decoder it lacks doesn't always announce itself.
   *
   * Some files error outright, which is handled on the element. Others load
   * their metadata, report a duration, and then never become playable — no
   * error, no `canplay`, just a dead play button. Waiting a moment and giving
   * up covers both without re-encoding anything that was going to work.
   */
  $effect(() => {
    if (!videoSrc || canPlay || proxyTried) return;
    const timer = setTimeout(() => void useProxy(), 4000);
    return () => clearTimeout(timer);
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

  /** The edits as the job stores them, where an untouched end is null. */
  function currentEdits(): Edits {
    return {
      start: inPoint > 0.01 ? inPoint : null,
      end: effectiveOut < duration - 0.01 ? effectiveOut : null,
      crop: crop ? { ...crop } : null,
    };
  }

  // Push edits back to the store so the plan readout recalculates.
  //
  // Debounced, because a drag or a held key changes them many times a second
  // and every write asks for a new plan. What's waiting is held on to rather
  // than only timed, so it can be written the moment the editor goes: a nudge
  // made just before Back or Compress used to vanish with the timer.
  let commitTimer: ReturnType<typeof setTimeout> | undefined;
  let pendingEdits: Edits | null = null;

  function flushEdits() {
    clearTimeout(commitTimer);
    if (!pendingEdits) return;
    app.setEdits(jobId, pendingEdits);
    pendingEdits = null;
  }

  $effect(() => {
    pendingEdits = currentEdits();
    clearTimeout(commitTimer);
    commitTimer = setTimeout(flushEdits, 180);
  });

  // However the editor closes — Back, Compress, another clip — nothing waiting
  // is left behind.
  $effect(() => () => flushEdits());

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
    void videoEl.play().catch(() => {
      // Audible playback can be refused outright. A click on our own button
      // satisfies the autoplay policy in every webview tested, but the rule is
      // the runtime's to make and it costs nothing to survive a no: silence is
      // recoverable, a play button that does nothing isn't.
      if (!videoEl) return;
      app.previewMuted = true;
      videoEl.muted = true;
      void videoEl.play().catch(() => {});
    });
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
  // Sound
  // ---------------------------------------------------------------------

  /**
   * Whether there is anything to hear.
   *
   * A photo has no timeline at all, and a GIF has no audio track by
   * definition, so both would get a control that does nothing. A video whose
   * probe found no audio stream is the same case.
   */
  const hasAudio = $derived(!isStill && !isGif && Boolean(job.info?.audioCodec));

  /**
   * Volume is applied to the element rather than set in the markup: it's a
   * property with no attribute behind it, and the element is rebuilt when the
   * proxy takes over, so this re-runs and the replacement starts out matching
   * instead of reverting to full volume.
   */
  $effect(() => {
    if (!videoEl) return;
    videoEl.muted = app.previewMuted || !hasAudio;
    videoEl.volume = app.previewVolume;
  });

  function toggleMute() {
    app.previewMuted = !app.previewMuted;
    void app.persist();
  }

  function onVolumeInput(event: Event) {
    const raw = Number((event.currentTarget as HTMLInputElement).value);
    if (!Number.isFinite(raw)) return;
    app.previewVolume = Math.min(Math.max(raw, 0), 1);
    // Pulling the slider up is itself a request to hear it.
    if (app.previewVolume > 0) app.previewMuted = false;
  }

  // Written on release rather than per pixel of travel — the store is a file.
  function saveVolume() {
    void app.persist();
  }

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

  /**
   * Moves the rectangle, or resizes it from one corner, by a distance given
   * in fractions of the frame.
   *
   * A drag and a key press both come through here, so a nudge is clamped and
   * held to the aspect exactly as the pointer is.
   */
  function reshape(start: CropRect, mode: DragMode, dx: number, dy: number): CropRect {
    const ratio = ratioFor(aspect);

    if (mode === "move") {
      return {
        ...start,
        x: clamp(start.x + dx, 0, 1 - start.width),
        y: clamp(start.y + dy, 0, 1 - start.height),
      };
    }

    // Corner resize: anchor the opposite corner and move this one.
    const right = start.x + start.width;
    const bottom = start.y + start.height;
    let x = start.x;
    let y = start.y;
    let w = start.width;
    let h = start.height;

    if (mode === "se") {
      w = clamp(start.width + dx, 0.05, 1 - start.x);
      h = clamp(start.height + dy, 0.05, 1 - start.y);
    } else if (mode === "sw") {
      x = clamp(start.x + dx, 0, right - 0.05);
      w = right - x;
      h = clamp(start.height + dy, 0.05, 1 - start.y);
    } else if (mode === "ne") {
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
      if (mode === "nw" || mode === "ne") y = bottom - h;
      if (mode === "nw" || mode === "sw") x = right - w;
      y = clamp(y, 0, 1 - h);
      x = clamp(x, 0, 1 - w);
    }

    return { x, y, width: w, height: h };
  }

  function onDrag(event: PointerEvent) {
    if (!drag || !crop || !previewEl) return;
    const box = previewEl.getBoundingClientRect();
    crop = reshape(
      drag.rect,
      drag.mode,
      (event.clientX - drag.startX) / box.width,
      (event.clientY - drag.startY) / box.height,
    );
  }

  /**
   * Arrow keys nudge the crop a hundredth of the frame, or a tenth with Shift.
   * On the box they move it; on a corner they resize from that corner, which
   * under a fixed aspect means the width leads and the height follows, as it
   * does with the pointer.
   */
  function onCropKey(event: KeyboardEvent, mode: DragMode) {
    if (!crop || event.altKey || event.ctrlKey || event.metaKey) return;
    const step = event.shiftKey ? 0.1 : 0.01;
    let dx = 0;
    let dy = 0;
    if (event.key === "ArrowLeft") dx = -step;
    else if (event.key === "ArrowRight") dx = step;
    else if (event.key === "ArrowUp") dy = -step;
    else if (event.key === "ArrowDown") dy = step;
    else return;
    event.preventDefault();
    crop = reshape(crop, mode, dx, dy);
  }

  /** What the crop box is called aloud: its size, then where it sits. */
  const cropLabel = $derived(
    crop
      ? `Crop area, ${croppedW} by ${croppedH} pixels, ` +
          `${Math.round(sourceW * crop.x)} from the left, ` +
          `${Math.round(sourceH * crop.y)} from the top`
      : "",
  );

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

  /** The shortest clip a trim can leave. */
  const MIN_CLIP = 0.2;

  // Each end stays inside the file and a clip's length from the other, and the
  // preview jumps to it so the cut can be seen. The pointer can't leave the
  // strip, but a key pressed at the limit can ask to, so both ends are held.
  function setIn(t: number) {
    inPoint = clamp(t, 0, Math.max(0, effectiveOut - MIN_CLIP));
    seek(inPoint);
  }

  function setOut(t: number) {
    outPoint = clamp(t, Math.min(inPoint + MIN_CLIP, duration), duration);
    seek(outPoint);
  }

  function onScrub(event: PointerEvent) {
    if (!scrub) return;
    const t = timeFromEvent(event);
    if (scrub === "in") {
      setIn(t);
    } else if (scrub === "out") {
      setOut(t);
    } else {
      seek(t);
    }
  }

  /**
   * The timeline's three sliders, from the keyboard.
   *
   * All take the standard slider keys — Right and Up step forward, Left and
   * Down back, Page Up and Page Down jump ten seconds, Home and End go to the
   * limit — which is also what a screen reader sends to change one. The
   * playhead steps in seconds, or a frame at a time with Alt or Ctrl, since
   * finding a frame is what it's for; the trim ends step in tenths, for
   * placing a cut. I and O mark the clip at the playhead, as most editors do.
   */
  function onTimelineKey(event: KeyboardEvent, handle: Handle) {
    if (duration <= 0 || event.metaKey) return;
    const onPlayhead = handle === "playhead";

    if (onPlayhead && !event.altKey && !event.ctrlKey) {
      const letter = event.key.toLowerCase();
      if (letter === "i" || letter === "o") {
        event.preventDefault();
        if (letter === "i") setIn(playhead);
        else setOut(playhead);
        return;
      }
    }
    if (!onPlayhead && (event.altKey || event.ctrlKey)) return;

    let value = playhead;
    let min = 0;
    let max = duration;
    let step = event.altKey || event.ctrlKey ? frameStep : event.shiftKey ? 5 : 1;
    if (handle === "in") {
      value = inPoint;
      max = effectiveOut - MIN_CLIP;
    } else if (handle === "out") {
      value = effectiveOut;
      min = inPoint + MIN_CLIP;
    }
    if (!onPlayhead) step = event.shiftKey ? 1 : 0.1;

    let to: number;
    switch (event.key) {
      case "ArrowRight":
      case "ArrowUp":
        to = value + step;
        break;
      case "ArrowLeft":
      case "ArrowDown":
        to = value - step;
        break;
      case "PageUp":
        to = value + 10;
        break;
      case "PageDown":
        to = value - 10;
        break;
      case "Home":
        to = min;
        break;
      case "End":
        to = max;
        break;
      default:
        return;
    }
    event.preventDefault();
    // A drag pauses too: the frame on screen is the point of moving these.
    if (playing) videoEl?.pause();
    if (handle === "in") setIn(to);
    else if (handle === "out") setOut(to);
    else seek(clamp(to, 0, duration));
  }

  /**
   * A time to the tenth, for what a screen reader announces. The readouts
   * round to whole seconds, so a trim end moving a tenth at a time would be
   * read out as the same number ten presses running.
   */
  function spokenTime(t: number): string {
    const tenths = Math.round(t * 10);
    return `${formatDuration(Math.floor(tenths / 10))}.${tenths % 10}`;
  }

  function resetTrim() {
    inPoint = 0;
    outPoint = null;
    seek(0);
  }

  const pct = (t: number) => (duration > 0 ? (t / duration) * 100 : 0);

  async function compress() {
    // The job starts from the edits it holds, so the last nudge can't be left
    // sitting in the timer.
    flushEdits();
    // Pressing this is the whole permission slip for re-encoding a GIF, which
    // is left alone anywhere else in the app. Without it a GIF asked to shrink
    // would be copied instead and report itself done having changed nothing.
    if (isGif) {
      app.setEdits(jobId, { ...currentEdits(), forceEncode: true });
    }
    await app.closeEditor();
    // The editor is gone by now, and the prop with it.
    await app.start(jobId);
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
          playsinline
          preload="auto"
          oncanplay={() => (canPlay = true)}
          onerror={() => {
            canPlay = false;
            void useProxy();
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
        <!-- Scrubbing already works by this point, so this says why playback
             isn't there yet rather than blocking the frame underneath. -->
        {#if proxying}
          <div class="proxy-note">Preparing playback…</div>
        {/if}
      {/if}

      {#if crop}
        <!-- An application rather than a group: a screen reader in its
             reading mode keeps the arrow keys for itself inside a group, and
             moving the box is what they're for here. The linter counts it as
             static all the same. -->
        <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
        <div
          class="crop"
          role="application"
          tabindex="0"
          aria-label={cropLabel}
          aria-describedby="{uid}-crop-keys"
          style:left="{crop.x * 100}%"
          style:top="{crop.y * 100}%"
          style:width="{crop.width * 100}%"
          style:height="{crop.height * 100}%"
          onpointerdown={(e) => beginDrag(e, "move")}
          onkeydown={(e) => {
            // A corner's keys are that corner's, not a move of the whole box.
            if (e.target === e.currentTarget) onCropKey(e, "move");
          }}
        >
          <button
            class="handle nw"
            aria-label="Resize crop from top-left corner"
            onpointerdown={(e) => beginDrag(e, "nw")}
            onkeydown={(e) => onCropKey(e, "nw")}
          ></button>
          <button
            class="handle ne"
            aria-label="Resize crop from top-right corner"
            onpointerdown={(e) => beginDrag(e, "ne")}
            onkeydown={(e) => onCropKey(e, "ne")}
          ></button>
          <button
            class="handle sw"
            aria-label="Resize crop from bottom-left corner"
            onpointerdown={(e) => beginDrag(e, "sw")}
            onkeydown={(e) => onCropKey(e, "sw")}
          ></button>
          <button
            class="handle se"
            aria-label="Resize crop from bottom-right corner"
            onpointerdown={(e) => beginDrag(e, "se")}
            onkeydown={(e) => onCropKey(e, "se")}
          ></button>
        </div>
        <span id="{uid}-crop-keys" hidden>
          Arrow keys move it, and resize it from a corner. Hold Shift for
          bigger steps.
        </span>
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

    <!-- A still has nothing to trim, so the whole timeline goes rather than
         sitting there disabled. Crop is the only edit that means anything. -->
    {#if !isStill}
    <div class="timeline">
      <div class="track">
        <div
          class="strip"
          bind:this={stripEl}
          role="slider"
          tabindex="0"
          aria-label="Playhead"
          aria-valuemin={0}
          aria-valuemax={duration}
          aria-valuenow={playhead}
          aria-valuetext="{spokenTime(playhead)} of {formatDuration(duration)}"
          aria-describedby="{uid}-playhead-keys"
          onpointerdown={(e) => beginScrub(e, "playhead")}
          onkeydown={(e) => onTimelineKey(e, "playhead")}
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
        </div>

        <!-- The trim ends sit over the strip rather than inside it. A
             slider's contents don't count as controls of their own, so inside
             the playhead they'd be hidden from a screen reader; this layer
             clips them exactly as the strip did. -->
        <div class="ends">
          <div
            class="handle-bar in"
            style:left="{pct(inPoint)}%"
            role="slider"
            tabindex="0"
            aria-label="Clip start"
            aria-valuemin={0}
            aria-valuemax={Math.max(0, effectiveOut - MIN_CLIP)}
            aria-valuenow={inPoint}
            aria-valuetext="{spokenTime(inPoint)}, {formatDuration(trimmed)} selected"
            onpointerdown={(e) => beginScrub(e, "in")}
            onkeydown={(e) => onTimelineKey(e, "in")}
          ></div>
          <div
            class="handle-bar out"
            style:left="{pct(effectiveOut)}%"
            role="slider"
            tabindex="0"
            aria-label="Clip end"
            aria-valuemin={Math.min(inPoint + MIN_CLIP, duration)}
            aria-valuemax={duration}
            aria-valuenow={effectiveOut}
            aria-valuetext="{spokenTime(effectiveOut)}, {formatDuration(trimmed)} selected"
            onpointerdown={(e) => beginScrub(e, "out")}
            onkeydown={(e) => onTimelineKey(e, "out")}
          ></div>
        </div>
      </div>
      <span id="{uid}-playhead-keys" hidden>
        I and O start and end the clip here.
      </span>

      <div class="times tnum">
        <div class="left-group">
          <button
            class="play"
            onclick={togglePlay}
            disabled={!canPlay}
            title={canPlay
              ? "Play the selection"
              : proxying
                ? "Preparing playback for this format…"
                : "Scrub the timeline to preview this one"}
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

          <!-- Only where there's a track to play. A dead volume knob on a
               silent clip is worse than no knob at all. -->
          {#if hasAudio}
            <div class="volume" class:off={app.previewMuted}>
              <button
                class="mute"
                onclick={toggleMute}
                disabled={!canPlay}
                title={app.previewMuted ? "Unmute" : "Mute"}
                aria-label={app.previewMuted ? "Unmute" : "Mute"}
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path d="M4 9v6h4l5 4V5L8 9H4z" fill="currentColor" />
                  {#if app.previewMuted}
                    <path
                      d="M16.5 9.5l5 5M21.5 9.5l-5 5"
                      stroke="currentColor"
                      stroke-width="2"
                      stroke-linecap="round"
                      fill="none"
                    />
                  {:else}
                    <path
                      d="M16 8.6a4.2 4.2 0 0 1 0 6.8"
                      stroke="currentColor"
                      stroke-width="2"
                      stroke-linecap="round"
                      fill="none"
                    />
                  {/if}
                </svg>
              </button>
              <input
                type="range"
                min="0"
                max="1"
                step="0.01"
                value={app.previewMuted ? 0 : app.previewVolume}
                oninput={onVolumeInput}
                onchange={saveVolume}
                disabled={!canPlay}
                title="Preview volume"
                aria-label="Preview volume"
              />
            </div>
          {/if}

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
    {/if}

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
        sourceBytes={job.info?.sizeBytes ?? null}
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
          {#if job.plan.mode === "keep"}
            <span>{job.plan.copyStreams ? "no re-encoding" : "near-lossless"}</span>
            {#if job.plan.estimatedBytes}
              <span class="sep">·</span>
              <span class="target">≈{formatSize(job.plan.estimatedBytes)}</span>
            {/if}
          {:else if job.plan.mode === "quality"}
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
        {:else if settings.mode === "keep" && !dirty}
          <span class="dim">Crop or trim to apply — nothing else will change</span>
        {:else if alreadyFits && !dirty}
          <span class="dim">Already under {formatSize(settings.targetBytes)}</span>
        {:else}
          <span class="dim">Working out the plan…</span>
        {/if}
      </div>

      <button class="compress" disabled={!canCompress} onclick={compress}>
        {actionLabel}
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

  /* Sits over the frame rather than replacing it — the frame is the useful
     thing while this is happening, and scrubbing already works. */
  .proxy-note {
    position: absolute;
    left: 50%;
    bottom: 10px;
    transform: translateX(-50%);
    padding: 4px 10px;
    border-radius: 999px;
    background: rgba(0, 0, 0, 0.7);
    font-size: 11px;
    color: var(--text-dim);
    pointer-events: none;
    white-space: nowrap;
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

  /* The preview clips everything outside the frame, and a crop that fills it
     would take an outside ring with it, so this one is drawn inside the edge.
     Square, because the edge is the crop line itself. */
  .crop:focus-visible {
    outline-offset: -4px;
    border-radius: 0;
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

  /* A corner on the frame's edge is mostly cut off, ring and all, so the part
     that's left lights up too. */
  .handle:focus-visible {
    background: var(--text);
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

  .track {
    position: relative;
  }

  /* The strip's own box and clipping, so the trim ends look as they did when
     they lived inside it. Clicks fall through to the strip except on the ends
     themselves. */
  .ends {
    position: absolute;
    inset: 0;
    border-radius: var(--radius-sm);
    overflow: hidden;
    pointer-events: none;
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
    pointer-events: auto;
  }

  /* Lit up like a focused crop corner, with the ring drawn inside: the strip
     clips a handle's top and bottom, and half of it at either end, so a ring
     outside would barely show — and an accent ring on an accent handle
     wouldn't show at all. */
  .handle-bar:focus-visible {
    background: var(--text);
    outline-offset: -2px;
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

  .volume {
    display: flex;
    align-items: center;
    gap: 5px;
  }

  .mute {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    color: var(--text-dim);
    transition: background 0.14s, color 0.14s;
  }

  .mute svg {
    width: 15px;
    height: 15px;
  }

  .mute:hover:not(:disabled) {
    background: var(--surface-hover);
    color: var(--text);
  }

  .mute:disabled {
    opacity: 0.35;
    cursor: default;
  }

  /* Silenced is worth seeing without reading the icon closely. */
  .volume.off .mute {
    color: var(--text-faint);
  }

  .volume input[type="range"] {
    -webkit-appearance: none;
    appearance: none;
    width: 50px;
    height: 3px;
    border-radius: 999px;
    background: var(--surface-strong);
    cursor: pointer;
  }

  .volume input[type="range"]:disabled {
    opacity: 0.35;
    cursor: default;
  }

  .volume input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: none;
    background: var(--text);
    cursor: pointer;
  }

  .volume.off input[type="range"]::-webkit-slider-thumb {
    background: var(--text-faint);
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
