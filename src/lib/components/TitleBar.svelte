<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { app } from "../state.svelte";

  interface Props {
    settingsOpen: boolean;
    onToggleSettings: () => void;
  }

  let { settingsOpen, onToggleSettings }: Props = $props();

  function togglePin() {
    void app.setPinned(!app.pinned);
  }

  /**
   * Tells the backend a drag may be starting.
   *
   * The window sees the same move event whether the user dragged it or the app
   * repositioned it, and it needs to tell them apart — one is worth dropping
   * the acrylic backdrop for, the other would make it blink on every open.
   *
   * Fires on the bar itself, so pressing a button up there doesn't count.
   */
  function noteDragStart(event: PointerEvent) {
    const target = event.target as HTMLElement | null;
    if (!target?.hasAttribute("data-tauri-drag-region")) return;
    void invoke("window_drag_started").catch(() => {});
  }
</script>

<!-- data-tauri-drag-region makes the bar behave like a native title bar,
     which the window needs since it's drawn without decorations. -->
<!-- Listened for on the window rather than the bar: a press anywhere in a drag
     region counts, and a header isn't an interactive element to hang a pointer
     handler off. -->
<svelte:window onpointerdown={noteDragStart} />

<header class="bar" data-tauri-drag-region>
  <div class="brand" data-tauri-drag-region>
    <svg class="mark" viewBox="0 0 24 24" aria-hidden="true">
      <g
        stroke="currentColor"
        stroke-width="2.4"
        stroke-linecap="round"
        stroke-linejoin="round"
        fill="none"
      >
        <polyline points="7,5 12,9 17,5" />
        <polyline points="7,19 12,15 17,19" />
      </g>
      <rect x="6" y="11" width="12" height="2.4" rx="1.2" fill="currentColor" />
    </svg>
    <span class="name" data-tauri-drag-region>Nitrate</span>
  </div>

  <div class="actions">
    <button
      class="icon"
      class:on={app.pinned}
      onclick={togglePin}
      title={app.pinned ? "Unpin (hides when it loses focus)" : "Pin (stay open)"}
      aria-label="Pin window"
      aria-pressed={app.pinned}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M15 3l6 6-2.5 1.2-3.2 3.2-.4 4.3-3.4-3.4L6 20l4.7-5.5-3.4-3.4 4.3-.4 3.2-3.2z"
          fill="currentColor"
        />
      </svg>
    </button>

    <button
      class="icon"
      class:on={settingsOpen}
      onclick={onToggleSettings}
      title="Advanced settings"
      aria-label="Advanced settings"
      aria-pressed={settingsOpen}
    >
      <!-- Sliders rather than a gear: it survives 15px, and "parameters" is
           what the panel behind it actually is. -->
      <svg
        viewBox="0 0 24 24"
        aria-hidden="true"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
      >
        <path d="M3 8h5" />
        <path d="M13 8h8" />
        <path d="M3 16h11" />
        <path d="M19 16h2" />
        <circle cx="10.5" cy="8" r="2.5" fill="currentColor" stroke="none" />
        <circle cx="16.5" cy="16" r="2.5" fill="currentColor" stroke="none" />
      </svg>
    </button>

    <button
      class="icon close"
      onclick={() => invoke("hide_window")}
      title="Close to tray"
      aria-label="Close to tray"
    >
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M6 6l12 12M18 6L6 18"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
        />
      </svg>
    </button>
  </div>
</header>

<style>
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 44px;
    padding: 0 8px 0 14px;
    flex-shrink: 0;
    border-bottom: 1px solid var(--hairline);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    pointer-events: none;
  }

  .mark {
    width: 17px;
    height: 17px;
    color: var(--blurple-bright);
    filter: drop-shadow(0 0 7px rgba(124, 136, 255, 0.55));
  }

  .name {
    font-size: 13px;
    font-weight: 650;
    letter-spacing: 0.2px;
  }

  .actions {
    display: flex;
    gap: 2px;
  }

  .icon {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: var(--radius-sm);
    color: var(--text-faint);
    transition: background 0.14s, color 0.14s;
  }

  .icon svg {
    width: 15px;
    height: 15px;
  }

  .icon:hover {
    background: var(--surface-hover);
    color: var(--text);
  }

  .icon.on {
    color: var(--blurple-bright);
    background: rgba(88, 101, 242, 0.16);
  }

  .close:hover {
    background: var(--danger);
    color: #fff;
  }
</style>
