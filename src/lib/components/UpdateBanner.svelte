<script lang="ts">
  import { app } from "../state.svelte";
  import { updater } from "../updater.svelte";

  // Restarting mid-encode would throw away the work in flight.
  let blockedByWork = $derived(updater.status === "ready" && app.busy);
</script>

{#if updater.visible}
  <div class="banner" class:ready={updater.status === "ready"}>
    <span class="dot"></span>

    <div class="text">
      {#if updater.status === "available"}
        <span class="head">Version {updater.version} is available</span>
      {:else if updater.status === "downloading"}
        <span class="head">
          Downloading{#if updater.progress !== null}
            <span class="tnum"> {Math.round(updater.progress * 100)}%</span>
          {/if}
        </span>
      {:else if blockedByWork}
        <span class="head">Update ready — finish encoding first</span>
      {:else}
        <span class="head">Update ready</span>
      {/if}
    </div>

    {#if updater.status === "available"}
      <button class="action" onclick={() => updater.install()}>Update</button>
      <button class="dismiss" onclick={() => updater.dismiss()} aria-label="Dismiss">
        <svg viewBox="0 0 24 24"
          ><path
            d="M6 6l12 12M18 6L6 18"
            stroke="currentColor"
            stroke-width="2.2"
            stroke-linecap="round"
          /></svg
        >
      </button>
    {:else if updater.status === "ready"}
      <button class="action" disabled={blockedByWork} onclick={() => updater.restart()}>
        Restart
      </button>
    {/if}

    {#if updater.status === "downloading" && updater.progress !== null}
      <div class="track" style:--p="{updater.progress * 100}%"></div>
    {/if}
  </div>
{/if}

<style>
  .banner {
    position: relative;
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 9px 10px 9px 13px;
    flex-shrink: 0;
    background: rgba(88, 101, 242, 0.16);
    border-bottom: 1px solid rgba(124, 136, 255, 0.28);
    overflow: hidden;
    animation: drop 0.3s var(--ease-spring);
  }

  @keyframes drop {
    from {
      opacity: 0;
      transform: translateY(-100%);
    }
  }

  .banner.ready {
    background: rgba(67, 181, 129, 0.16);
    border-bottom-color: rgba(67, 181, 129, 0.32);
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--blurple-bright);
    box-shadow: 0 0 8px var(--blurple-bright);
    animation: blink 1.8s ease-in-out infinite;
  }

  .banner.ready .dot {
    background: var(--success);
    box-shadow: 0 0 8px var(--success);
    animation: none;
  }

  @keyframes blink {
    50% {
      opacity: 0.3;
    }
  }

  .text {
    flex: 1;
    min-width: 0;
  }

  .head {
    font-size: 11.5px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .action {
    flex-shrink: 0;
    padding: 4px 11px;
    border-radius: 7px;
    font-size: 11px;
    font-weight: 650;
    background: var(--blurple);
    color: #fff;
    transition: background 0.14s, opacity 0.14s;
  }

  .banner.ready .action {
    background: var(--success);
  }

  .action:hover:not(:disabled) {
    filter: brightness(1.12);
  }

  .action:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .dismiss {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    flex-shrink: 0;
    border-radius: 6px;
    color: var(--text-faint);
  }

  .dismiss svg {
    width: 12px;
    height: 12px;
  }

  .dismiss:hover {
    background: var(--surface-hover);
    color: var(--text);
  }

  /* Download progress reads along the bottom edge of the banner. */
  .track {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 2px;
    width: var(--p);
    background: var(--blurple-bright);
    box-shadow: 0 0 8px var(--blurple-bright);
    transition: width 0.25s ease-out;
  }
</style>
