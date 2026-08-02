<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { VIDEO_EXTENSIONS } from "../presets";
  import { app } from "../state.svelte";

  interface Props {
    hovering: boolean;
    compact: boolean;
  }

  let { hovering, compact }: Props = $props();

  async function browse() {
    // The native picker steals focus, which would otherwise dismiss the popup.
    const picked = await app.withDialog(() =>
      open({
        multiple: true,
        // The same list the drop handler enforces, so the picker and a drag
        // can't disagree about what counts as a video.
        filters: [{ name: "Video", extensions: VIDEO_EXTENSIONS }],
      }),
    );

    if (!picked) return;
    await app.addFiles(Array.isArray(picked) ? picked : [picked]);
  }
</script>

<button
  class="zone"
  class:hovering
  class:compact
  onclick={browse}
  aria-label="Drop videos here or click to browse"
>
  <div class="glow"></div>

  <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
    <g
      stroke="currentColor"
      stroke-width="1.8"
      fill="none"
      stroke-linecap="round"
      stroke-linejoin="round"
    >
      <path d="M12 15V4m0 0L8 8m4-4l4 4" />
      <path d="M4 15v3a2 2 0 002 2h12a2 2 0 002-2v-3" />
    </g>
  </svg>

  <div class="text">
    <span class="head">
      {hovering ? "Release to compress" : "Drop videos or paste a link"}
    </span>
    {#if !compact}
      <span class="sub">
        Click to browse, or press Ctrl+V with a link copied
      </span>
      <span class="sites">YouTube · X · Instagram · Reddit · Twitch</span>
    {/if}
  </div>
</button>

<style>
  .zone {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 12px;
    width: 100%;
    padding: 30px 18px;
    border-radius: var(--radius-lg);
    border: 1.5px dashed var(--hairline-bright);
    background: var(--surface);
    color: var(--text-dim);
    overflow: hidden;
    transition: border-color 0.2s, background 0.2s, padding 0.25s var(--ease-spring),
      color 0.2s;
  }

  .zone.compact {
    flex-direction: row;
    padding: 13px 18px;
  }

  .zone:not(.compact) {
    flex-direction: column;
    gap: 14px;
  }

  .zone:hover {
    background: var(--surface-hover);
    border-color: rgba(124, 136, 255, 0.5);
    color: var(--text);
  }

  .zone.hovering {
    border-color: var(--blurple-bright);
    border-style: solid;
    background: rgba(88, 101, 242, 0.13);
    color: var(--text);
  }

  /* Bloom that only shows during a drag, so the target reads as live. */
  .glow {
    position: absolute;
    inset: -40%;
    background: radial-gradient(
      circle at center,
      rgba(88, 101, 242, 0.45),
      transparent 62%
    );
    opacity: 0;
    transition: opacity 0.25s;
    pointer-events: none;
  }

  .zone.hovering .glow {
    opacity: 1;
    animation: breathe 2.2s ease-in-out infinite;
  }

  @keyframes breathe {
    50% {
      opacity: 0.62;
    }
  }

  .icon {
    position: relative;
    width: 26px;
    height: 26px;
    flex-shrink: 0;
    transition: transform 0.3s var(--ease-spring);
  }

  .zone.compact .icon {
    width: 19px;
    height: 19px;
  }

  .zone.hovering .icon {
    transform: translateY(-3px) scale(1.08);
    color: var(--blurple-bright);
  }

  .text {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    text-align: center;
  }

  .zone.compact .text {
    align-items: flex-start;
  }

  .head {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }

  .zone.compact .head {
    font-size: 12px;
    font-weight: 500;
    color: var(--text-dim);
  }

  .sub {
    font-size: 11px;
    color: var(--text-faint);
  }

  .sites {
    margin-top: 4px;
    font-size: 10px;
    letter-spacing: 0.03em;
    color: rgba(155, 160, 180, 0.55);
  }
</style>
