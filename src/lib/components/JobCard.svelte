<script lang="ts">
  import { Channel, invoke } from "@tauri-apps/api/core";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import {
    formatBitrate,
    formatDuration,
    formatEta,
    formatSize,
    truncateName,
  } from "../format";
  import { app, etaFor } from "../state.svelte";
  import { hasEdits, type Job } from "../types";

  interface Props {
    job: Job;
  }

  let { job }: Props = $props();

  let eta = $state<number | null>(null);

  // Recomputing on a timer rather than per progress event keeps the estimate
  // from flickering on every one of ffmpeg's frequent updates.
  $effect(() => {
    if (job.status !== "running") {
      eta = null;
      return;
    }
    const tick = () => (eta = etaFor(job));
    tick();
    const timer = setInterval(tick, 1000);
    return () => clearInterval(timer);
  });

  let saved = $derived(
    job.finalBytes && job.originalBytes
      ? Math.max(0, Math.round((1 - job.finalBytes / job.originalBytes) * 100))
      : null,
  );

  let percent = $derived(Math.round(job.progress * 100));

  /**
   * Plays the finished file.
   *
   * This went through the opener plugin's `openPath`, which is not in that
   * plugin's default permission set — so the call was rejected and the promise
   * rejected with it. Nothing was awaiting it, so the button did nothing at all
   * and said nothing about why. Now it goes through our own command, and a
   * failure reaches the card.
   */
  async function play() {
    if (!job.output) return;
    try {
      await invoke("open_video", { path: job.output });
    } catch (err) {
      app.say(String(err));
    }
  }

  // ---------------------------------------------------------------------
  // Dragging the finished file out
  // ---------------------------------------------------------------------

  const draggable = $derived(job.status === "done" && Boolean(job.output));

  /**
   * The picture that follows the cursor, as a PNG.
   *
   * Thumbnails are JPEG — cheaper to make and there are a lot of them — but the
   * drag plugin takes PNG only, so it's converted once and kept. Done ahead of
   * time because `dragstart` has no room to wait for an image to decode.
   */
  let dragImage = $state<string | null>(null);

  $effect(() => {
    if (!draggable || !job.thumbnail || dragImage) return;

    const image = new Image();
    image.onload = () => {
      const canvas = document.createElement("canvas");
      canvas.width = image.naturalWidth;
      canvas.height = image.naturalHeight;
      const context = canvas.getContext("2d");
      if (!context) return;
      context.drawImage(image, 0, 0);
      dragImage = canvas.toDataURL("image/png");
    };
    image.src = job.thumbnail;
  });

  /** A plain dark tile, for a video whose first frame never arrived. */
  function fallbackImage(): string {
    const canvas = document.createElement("canvas");
    canvas.width = 160;
    canvas.height = 90;
    const context = canvas.getContext("2d");
    if (context) {
      context.fillStyle = "#1b1e27";
      context.fillRect(0, 0, canvas.width, canvas.height);
      context.fillStyle = "#5865F2";
      context.fillRect(0, canvas.height - 4, canvas.width, 4);
    }
    return canvas.toDataURL("image/png");
  }

  /**
   * Hands the real file to whatever it's dropped on.
   *
   * The webview's own drag can only offer text, which is why dragging a card
   * into Discord used to do nothing useful. `preventDefault` calls that off and
   * the OS drag takes over with the actual file, so the receiving app sees what
   * it would see from Explorer.
   */
  async function beginFileDrag(event: DragEvent) {
    if (!draggable || !job.output) return;

    // A press that starts on a button belongs to the button. Without this,
    // twitching the mouse while clicking Remove would hand someone the file
    // instead of deleting it — and the click never lands, because the drag
    // swallows it. Cancelled outright rather than left to the webview, which
    // would otherwise drag the card's text.
    if ((event.target as HTMLElement | null)?.closest("button")) {
      event.preventDefault();
      return;
    }

    event.preventDefault();

    try {
      await invoke("plugin:drag|start_drag", {
        item: [job.output],
        image: dragImage ?? fallbackImage(),
        // Copy, not move: the file stays in the output folder afterwards.
        options: { mode: "copy" },
        onEvent: new Channel(),
      });
    } catch (err) {
      app.say(`Couldn't start the drag: ${err}`);
    }
  }

  let working = $derived(job.status === "running" || job.status === "queued");
  let idle = $derived(!working);
  // A link has no local file to edit until it has been fetched.
  let editable = $derived(idle && job.path !== "");
  let edited = $derived(hasEdits(job.edits));

  // Held back rather than started automatically, because it's very long.
  let waiting = $derived(job.status === "held");
  let settings = $derived(app.settingsFor(job));
</script>

<!-- The whole card is the handle, not just the thumbnail: it's one row
     representing one file, so anywhere along it should pick the file up.
     Presses that land on a button are excluded in `beginFileDrag`.

     No role and no tab stop, deliberately. Dragging has no keyboard equivalent
     to expose, and everything it achieves is already reachable from the buttons
     on the right — play the file, or show it in its folder. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<article
  class="card"
  class:done={job.status === "done"}
  class:failed={job.status === "failed"}
  class:grabbable={draggable}
  draggable={draggable}
  ondragstart={beginFileDrag}
  title={draggable ? "Drag this into Discord, a folder, anywhere" : undefined}
>
  <div class="top">
    <div class="thumb" class:empty={!job.thumbnail}>
      {#if job.thumbnail}
        <img src={job.thumbnail} alt="" />
      {:else}
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path
            d="M4 5h16a1 1 0 011 1v12a1 1 0 01-1 1H4a1 1 0 01-1-1V6a1 1 0 011-1zm6 3.5v7l6-3.5z"
            fill="currentColor"
          />
        </svg>
      {/if}

      {#if job.status === "done"}
        <span class="badge ok" aria-hidden="true">
          <svg viewBox="0 0 24 24">
            <path
              d="M5 13l4 4L19 7"
              stroke="currentColor"
              stroke-width="3"
              fill="none"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
        </span>
      {/if}
    </div>

    <div class="info">
      <div class="name" title={job.name}>
        {truncateName(job.name)}
        {#if edited}
          <span class="edits">
            {#if job.edits.start !== null || job.edits.end !== null}
              <span class="tag">✂ {formatDuration(
                (job.edits.end ?? job.info?.duration ?? 0) - (job.edits.start ?? 0),
              )}</span>
            {/if}
            {#if job.edits.crop}<span class="tag">crop</span>{/if}
          </span>
        {/if}
      </div>

      <div class="meta tnum">
        {#if job.status === "failed"}
          <span class="err">{job.error}</span>
        {:else if job.status === "done" && job.finalBytes}
          <span class="from">{formatSize(job.originalBytes ?? 0)}</span>
          <span class="arrow">→</span>
          <span class="to">{formatSize(job.finalBytes)}</span>
          {#if saved !== null}<span class="saved">−{saved}%</span>{/if}
        {:else if job.info}
          <span>{formatSize(job.info.sizeBytes)}</span>
          <span class="dot">·</span>
          <span>{formatDuration(job.info.duration)}</span>
          {#if job.plan}
            <span class="dot">·</span>
            <span class:downscale={job.plan.downscaled}>
              {job.plan.height}p
            </span>
            <span class="dot">·</span>
            {#if job.plan.mode === "keep"}
              <span>{job.plan.copyStreams ? "no re-encoding" : "near-lossless"}</span>
              {#if job.plan.estimatedBytes}
                <span class="dot">·</span>
                <span>≈{formatSize(job.plan.estimatedBytes)}</span>
              {/if}
            {:else if job.plan.mode === "quality"}
              <!-- No bitrate budget exists in quality mode, so quoting one
                   would just be a misleading zero. The estimate stands in for
                   it: without one there's nothing here to suggest the size. -->
              <span>{settings.quality} quality</span>
              {#if job.plan.estimatedBytes}
                <span class="dot">·</span>
                <span class="estimate" title="A rough guess from the quality setting and the frame size. Busy footage runs larger, still footage smaller.">
                  ≈{formatSize(job.plan.estimatedBytes)}
                </span>
              {/if}
            {:else}
              <span>{formatBitrate(job.plan.videoKbps)}</span>
            {/if}
          {/if}
        {:else}
          <span class="dim">{job.stage}</span>
        {/if}
      </div>
    </div>

    <div class="actions">
      {#if waiting}
        <button class="act go" onclick={() => app.start(job.id)} title="Compress it anyway">
          <svg viewBox="0 0 24 24"><path d="M8 5l11 7-11 7z" fill="currentColor" /></svg>
        </button>
      {:else if working}
        <button class="act" onclick={() => app.cancel(job.id)} title="Cancel">
          <svg viewBox="0 0 24 24"
            ><path
              d="M6 6l12 12M18 6L6 18"
              stroke="currentColor"
              stroke-width="2.2"
              stroke-linecap="round"
            /></svg
          >
        </button>
      {:else if job.status === "done"}
        <button
          class="act"
          onclick={play}
          title="Play"
        >
          <svg viewBox="0 0 24 24"><path d="M8 5l12 7-12 7z" fill="currentColor" /></svg>
        </button>
        <button
          class="act"
          onclick={() => job.output && revealItemInDir(job.output)}
          title="Show in folder"
        >
          <svg viewBox="0 0 24 24"
            ><path
              d="M3 6a1 1 0 011-1h5l2 2h9a1 1 0 011 1v11a1 1 0 01-1 1H4a1 1 0 01-1-1z"
              fill="currentColor"
            /></svg
          >
        </button>
      {:else}
        <button class="act" onclick={() => app.start(job.id)} title="Retry">
          <svg viewBox="0 0 24 24"
            ><path
              d="M4 12a8 8 0 1 1 2.5 5.8M4 19v-5h5"
              stroke="currentColor"
              stroke-width="2.2"
              fill="none"
              stroke-linecap="round"
              stroke-linejoin="round"
            /></svg
          >
        </button>
      {/if}

      {#if editable}
        <button
          class="act"
          class:on={edited}
          onclick={() => app.openEditor(job.id)}
          title="Crop and trim"
        >
          <!-- Plain scissors: two rings and crossed blades. Anything more
               detailed turns to mush at 14px. -->
          <svg
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.7"
            stroke-linecap="round"
          >
            <circle cx="6.5" cy="6.5" r="2.6" />
            <circle cx="6.5" cy="17.5" r="2.6" />
            <path d="M20 4 8.7 15.3" />
            <path d="M13.8 14 20 20" />
            <path d="M8.7 8.7 11.5 11.5" />
          </svg>
        </button>
      {/if}

      <button class="act dim-act" onclick={() => app.remove(job.id)} title="Remove">
        <svg viewBox="0 0 24 24"
          ><path
            d="M5 7h14M10 11v6M14 11v6M6 7l1 12a1 1 0 001 1h8a1 1 0 001-1l1-12M9 7V5a1 1 0 011-1h4a1 1 0 011 1v2"
            stroke="currentColor"
            stroke-width="1.8"
            fill="none"
            stroke-linecap="round"
            stroke-linejoin="round"
          /></svg
        >
      </button>
    </div>
  </div>

  {#if waiting && job.heldReason === "browser"}
    <!-- Held for safety, not because of length: any web page can fire the
         protocol, so nothing from outside starts on its own by default. -->
    <p class="hold calm">
      <svg viewBox="0 0 24 24" aria-hidden="true"
        ><path
          d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"
          stroke="currentColor"
          stroke-width="1.7"
          fill="none"
          stroke-linejoin="round"
        /></svg
      >
      <span>
        Sent from your browser. Links from outside wait for you — press play to
        start, or turn it on in settings.
      </span>
    </p>
  {:else if waiting}
    <p class="hold">
      <svg viewBox="0 0 24 24" aria-hidden="true"
        ><path
          d="M12 9v5M12 17.5v.01M10.3 4.3 2.5 18a2 2 0 0 0 1.7 3h15.6a2 2 0 0 0 1.7-3L13.7 4.3a2 2 0 0 0-3.4 0z"
          stroke="currentColor"
          stroke-width="1.8"
          fill="none"
          stroke-linecap="round"
          stroke-linejoin="round"
        /></svg
      >
      <span>
        {formatDuration(job.knownDuration ?? 0)} long — squeezing all of it into
        {formatSize(settings.targetBytes)} would look poor. Trim a section first,
        pick <strong>No limit</strong>, or start it anyway.
      </span>
    </p>
  {:else if working}
    <div class="progress">
      <div class="track">
        <div
          class="fill"
          class:indeterminate={job.status === "queued"}
          style:width="{job.status === 'queued' ? 100 : percent}%"
        ></div>
      </div>
      <div class="status">
        <span class="stage">{job.stage}</span>
        <span class="right tnum">
          {#if eta}<span class="eta">{formatEta(eta)}</span>{/if}
          {#if job.status === "running"}<span class="pct">{percent}%</span>{/if}
        </span>
      </div>
    </div>
  {/if}

  {#if job.status === "done" && job.notes.length > 0}
    <ul class="notes">
      {#each job.notes as note}<li>{note}</li>{/each}
    </ul>
  {/if}
</article>

<style>
  .card {
    padding: 10px;
    border-radius: var(--radius);
    background: var(--surface);
    border: 1px solid var(--hairline);
    /* A faint top highlight is what sells the surface as glass rather than fill. */
    background-image: linear-gradient(
      to bottom,
      rgba(255, 255, 255, 0.045),
      transparent 40%
    );
    animation: rise 0.32s var(--ease-spring);
  }

  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(6px) scale(0.985);
    }
  }

  .card.done {
    border-color: rgba(67, 181, 129, 0.3);
  }

  .card.failed {
    border-color: rgba(237, 66, 69, 0.35);
  }

  .top {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .thumb {
    position: relative;
    flex-shrink: 0;
    width: 46px;
    height: 46px;
    border-radius: var(--radius-sm);
    overflow: hidden;
    background: rgba(0, 0, 0, 0.28);
  }

  /* The only hint that the row can be picked up, short of writing it on the
     card. The buttons keep their own cursor, since they aren't handles. */
  .card.grabbable {
    cursor: grab;
  }

  .card.grabbable:active {
    cursor: grabbing;
  }

  .card.grabbable button {
    cursor: pointer;
  }

  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .thumb.empty {
    display: grid;
    place-items: center;
    color: var(--text-faint);
  }

  .thumb.empty svg {
    width: 20px;
    height: 20px;
  }

  .badge {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgba(10, 12, 18, 0.62);
    color: var(--success);
    animation: pop 0.34s var(--ease-spring);
  }

  .badge svg {
    width: 22px;
    height: 22px;
  }

  @keyframes pop {
    from {
      opacity: 0;
      transform: scale(0.6);
    }
  }

  .info {
    flex: 1;
    min-width: 0;
  }

  .name {
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-size: 12.5px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .edits {
    display: flex;
    gap: 4px;
    flex-shrink: 0;
  }

  .tag {
    padding: 1px 5px;
    border-radius: 5px;
    font-size: 9.5px;
    font-weight: 700;
    color: var(--blurple-bright);
    background: rgba(88, 101, 242, 0.16);
  }

  .act.on {
    color: var(--blurple-bright);
    background: rgba(88, 101, 242, 0.16);
  }

  .act.go {
    color: var(--warn);
    background: rgba(250, 168, 26, 0.14);
  }

  .act.go:hover {
    background: var(--warn);
    color: #1a1206;
  }

  .hold {
    display: flex;
    align-items: flex-start;
    gap: 7px;
    margin-top: 9px;
    padding: 8px 9px;
    border-radius: var(--radius-sm);
    background: rgba(250, 168, 26, 0.1);
    font-size: 10.5px;
    line-height: 1.45;
    color: rgba(250, 200, 120, 0.92);
  }

  .hold svg {
    width: 13px;
    height: 13px;
    flex-shrink: 0;
    margin-top: 1px;
    color: var(--warn);
  }

  /* Not a warning — just an explanation of why it's waiting. */
  .hold.calm {
    background: rgba(88, 101, 242, 0.1);
    color: rgba(180, 190, 255, 0.92);
  }

  .hold.calm svg {
    color: var(--blurple-bright);
  }

  .meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 3px;
    font-size: 11px;
    color: var(--text-dim);
  }

  .dot {
    color: var(--text-faint);
  }

  .dim {
    color: var(--text-faint);
  }

  .arrow {
    color: var(--text-faint);
  }

  .to {
    color: var(--success);
    font-weight: 650;
  }

  /* Dotted underline: the number is a guess, and it should look like one. */
  .estimate {
    text-decoration: underline dotted;
    text-underline-offset: 3px;
    cursor: help;
  }

  .saved {
    padding: 1px 5px;
    border-radius: 5px;
    font-size: 10px;
    font-weight: 700;
    color: var(--success);
    background: rgba(67, 181, 129, 0.15);
  }

  .downscale {
    color: var(--warn);
  }

  .err {
    color: var(--danger);
    line-height: 1.35;
  }

  .actions {
    display: flex;
    gap: 1px;
    flex-shrink: 0;
  }

  .act {
    display: grid;
    place-items: center;
    width: 27px;
    height: 27px;
    border-radius: 7px;
    color: var(--text-dim);
    transition: background 0.14s, color 0.14s;
  }

  .act svg {
    width: 14px;
    height: 14px;
  }

  .act:hover {
    background: var(--surface-strong);
    color: var(--text);
  }

  .dim-act {
    color: var(--text-faint);
  }

  .dim-act:hover {
    color: var(--danger);
    background: rgba(237, 66, 69, 0.14);
  }

  .progress {
    margin-top: 9px;
  }

  .track {
    height: 4px;
    border-radius: 99px;
    background: rgba(255, 255, 255, 0.07);
    overflow: hidden;
  }

  .fill {
    position: relative;
    height: 100%;
    border-radius: 99px;
    background: linear-gradient(90deg, var(--blurple), var(--blurple-bright));
    box-shadow: 0 0 12px rgba(88, 101, 242, 0.75);
    transition: width 0.25s ease-out;
  }

  /* A travelling sheen makes it obvious work is happening even when the
     bar itself is barely moving. */
  .fill::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(
      90deg,
      transparent,
      rgba(255, 255, 255, 0.55),
      transparent
    );
    animation: sheen 1.5s linear infinite;
  }

  @keyframes sheen {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(100%);
    }
  }

  .fill.indeterminate {
    opacity: 0.35;
    transition: none;
  }

  .status {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 5px;
    font-size: 10.5px;
    color: var(--text-dim);
  }

  .right {
    display: flex;
    gap: 7px;
    align-items: baseline;
  }

  .eta {
    color: var(--text-faint);
  }

  .pct {
    font-weight: 700;
    color: var(--blurple-bright);
  }

  .notes {
    margin-top: 8px;
    padding-left: 14px;
    font-size: 10.5px;
    line-height: 1.5;
    color: var(--text-faint);
  }
</style>
