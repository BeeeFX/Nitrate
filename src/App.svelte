<script lang="ts">
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { onMount } from "svelte";
  import AdvancedPanel from "./lib/components/AdvancedPanel.svelte";
  import DropZone from "./lib/components/DropZone.svelte";
  import JobCard from "./lib/components/JobCard.svelte";
  import TargetPicker from "./lib/components/TargetPicker.svelte";
  import TitleBar from "./lib/components/TitleBar.svelte";
  import { app } from "./lib/state.svelte";

  let hovering = $state(false);
  let settingsOpen = $state(false);

  let finished = $derived(app.jobs.filter((j) => j.status === "done").length);
  let hasJobs = $derived(app.jobs.length > 0);
  let hasFinished = $derived(
    app.jobs.some((j) => j.status !== "running" && j.status !== "queued"),
  );

  onMount(() => {
    void app.init();

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
      void pending.then((unlisten) => unlisten());
    };
  });
</script>

<main class="shell" class:busy={app.busy}>
  <div class="ambient" aria-hidden="true"></div>

  <TitleBar {settingsOpen} onToggleSettings={() => (settingsOpen = !settingsOpen)} />

  {#if !settingsOpen}
    <TargetPicker />
  {/if}

  <section class="content">
    {#if settingsOpen}
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

  {#if hasJobs && !settingsOpen}
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
