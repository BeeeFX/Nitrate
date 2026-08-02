<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { onMount } from "svelte";
  import AdvancedPanel from "./lib/components/AdvancedPanel.svelte";
  import DropZone from "./lib/components/DropZone.svelte";
  import Editor from "./lib/components/Editor.svelte";
  import JobCard from "./lib/components/JobCard.svelte";
  import TargetPicker from "./lib/components/TargetPicker.svelte";
  import TitleBar from "./lib/components/TitleBar.svelte";
  import Tour from "./lib/components/Tour.svelte";
  import UpdateBanner from "./lib/components/UpdateBanner.svelte";
  import { app } from "./lib/state.svelte";
  import { updater } from "./lib/updater.svelte";

  let hovering = $state(false);
  let settingsOpen = $state(false);
  let shell = $state<HTMLElement | null>(null);

  /**
   * Replayed every time the tray reveals the window. The Web Animations API is
   * used rather than a CSS class because it can be re-triggered on demand — a
   * CSS animation only runs once unless the element is torn down and rebuilt,
   * which would throw away the job queue.
   */
  function playEntrance() {
    if (!shell) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    shell.animate(
      [
        { opacity: 0, transform: "scale(0.96) translateY(10px)" },
        { opacity: 1, transform: "scale(1) translateY(0)" },
      ],
      { duration: 240, easing: "cubic-bezier(0.22, 1, 0.36, 1)" },
    );
  }

  let finished = $derived(app.jobs.filter((j) => j.status === "done").length);
  let hasJobs = $derived(app.jobs.length > 0);
  let hasFinished = $derived(
    app.jobs.some((j) => j.status !== "running" && j.status !== "queued"),
  );

  onMount(() => {
    void app.init();
    updater.start();
    playEntrance();

    const shown = listen("popup://shown", () => playEntrance());

    // Pasting a link is the whole "add from the internet" interface — there's
    // no URL box, because the drop zone already is the place things go.
    const onPaste = (event: ClipboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (target && /^(INPUT|TEXTAREA)$/.test(target.tagName)) return;

      const text = event.clipboardData?.getData("text")?.trim();
      if (!text || !/^https?:\/\/\S+$/i.test(text)) return;

      event.preventDefault();
      void app.addUrl(text);
    };
    document.addEventListener("paste", onPaste);

    // Tauri's own drag-drop gives real filesystem paths, which the HTML5
    // drop event can't provide inside a webview.
    const pending = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === "over") {
        hovering = true;
      } else if (event.payload.type === "drop") {
        hovering = false;
        void app.addFiles(event.payload.paths);
      } else {
        hovering = false;
      }
    });

    return () => {
      updater.stop();
      document.removeEventListener("paste", onPaste);
      void pending.then((unlisten) => unlisten());
      void shown.then((unlisten) => unlisten());
    };
  });
</script>

<main class="shell" class:busy={app.busy} bind:this={shell}>
  <div class="ambient" aria-hidden="true"></div>

  {#if app.ready && app.showTour}
    <Tour />
  {/if}

  <TitleBar {settingsOpen} onToggleSettings={() => (settingsOpen = !settingsOpen)} />

  <UpdateBanner />

  {#if !settingsOpen && !app.editing}
    <TargetPicker settings={app.settings} onChange={(patch) => app.update(patch)} />
  {/if}

  {#if app.notice}
    <div class="notice">{app.notice}</div>
  {/if}

  <section class="content">
    {#if app.editing}
      {@const editing = app.editing}
      <!-- Keyed so switching jobs rebuilds the editor rather than leaving the
           previous clip's crop and trim in place. -->
      {#key editing.id}
        <Editor job={editing} />
      {/key}
    {:else if settingsOpen}
      <AdvancedPanel />
    {:else}
      <div class="body" class:padded={hasJobs}>
        <DropZone {hovering} compact={hasJobs} />

        {#if hasJobs}
          <div class="jobs">
            {#each app.jobs as job (job.id)}
              <JobCard {job} />
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </section>

  {#if hasJobs && !settingsOpen && !app.editing}
    <footer class="footer">
      <span class="summary tnum">
        {#if app.busy}
          <span class="pulse"></span>
          {app.active + app.queued} in progress
        {:else}
          {finished} of {app.jobs.length} done
        {/if}
      </span>

      {#if hasFinished}
        <button class="clear" onclick={() => app.clearFinished()}>Clear</button>
      {/if}
    </footer>
  {/if}
</main>

<style>
  .shell {
    position: relative;
    display: flex;
    flex-direction: column;
    height: 100vh;
    /* Sits over the OS acrylic, dark enough for text to hold up on any wallpaper. */
    background: var(--scrim);
    border-radius: 10px;
    border: 1px solid rgba(255, 255, 255, 0.07);
    overflow: hidden;
  }

  /* A blurple wash behind the header that wakes up while work is running. */
  .ambient {
    position: absolute;
    top: -160px;
    left: 50%;
    width: 420px;
    height: 300px;
    transform: translateX(-50%);
    background: radial-gradient(
      ellipse at center,
      rgba(88, 101, 242, 0.32),
      transparent 68%
    );
    opacity: 0.5;
    pointer-events: none;
    transition: opacity 0.7s ease;
  }

  .shell.busy .ambient {
    opacity: 1;
    animation: pulse-glow 3s ease-in-out infinite;
  }

  @keyframes pulse-glow {
    50% {
      opacity: 0.62;
    }
  }

  .notice {
    flex-shrink: 0;
    padding: 7px 14px;
    font-size: 11px;
    color: rgba(250, 200, 120, 0.95);
    background: rgba(250, 168, 26, 0.12);
    border-bottom: 1px solid rgba(250, 168, 26, 0.22);
    animation: drop 0.22s var(--ease-spring);
  }

  @keyframes drop {
    from {
      opacity: 0;
      transform: translateY(-6px);
    }
  }

  .content {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  /* When there are no jobs the drop target should own the whole space. */
  .body:not(.padded) {
    justify-content: center;
  }

  .jobs {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 14px;
    flex-shrink: 0;
    border-top: 1px solid var(--hairline);
    font-size: 11px;
    color: var(--text-dim);
  }

  .summary {
    display: flex;
    align-items: center;
    gap: 7px;
  }

  .pulse {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--blurple-bright);
    box-shadow: 0 0 8px var(--blurple-bright);
    animation: blink 1.4s ease-in-out infinite;
  }

  @keyframes blink {
    50% {
      opacity: 0.25;
    }
  }

  .clear {
    font-size: 11px;
    color: var(--text-faint);
    padding: 3px 8px;
    border-radius: 6px;
    transition: background 0.14s, color 0.14s;
  }

  .clear:hover {
    background: var(--surface-hover);
    color: var(--text);
  }
</style>
