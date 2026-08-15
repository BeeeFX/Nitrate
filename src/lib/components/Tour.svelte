<script lang="ts">
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { app } from "../state.svelte";

  const EXTENSION_URL = "https://github.com/BeeeFX/Nitrate#browser-extension";

  const steps = [
    {
      title: "It lives in your tray",
      body: "Nitrate stays out of the way. Click its icon in the system tray whenever you need it, and close the window to send it back there.",
      icon: "tray",
    },
    {
      title: "Pick a size, drop a video",
      body: "Choose the limit you're up against — 20 MB free, or a Nitro tier — then drop files in. They land in your Downloads folder small enough to send.",
      icon: "drop",
    },
    {
      title: "Or paste a link",
      body: "Press Ctrl+V with a YouTube, X, Instagram, Reddit or Twitch link copied. It downloads, then compresses.",
      icon: "link",
    },
    {
      title: "Take it wherever you need it",
      body: "When a video is done, drag its row straight into Discord or a folder — anywhere along the row works. Right-click it instead to copy the file, then paste.",
      icon: "grab",
      toggle: {
        label: "Copy every finished video automatically",
        hint: "Paste a link here, paste the video into Discord. It does replace whatever you had copied.",
        get: () => app.copyWhenDone,
        set: (on: boolean) => {
          app.copyWhenDone = on;
          void app.persist();
        },
      },
    },
    {
      title: "Skip the copying, soon",
      body: "A browser extension is on the way that puts a Send to Nitrate button straight on those sites. It's waiting to be approved by the Chrome and Firefox stores — the link will go up on GitHub as soon as it is.",
      icon: "puzzle",
      action: { label: "Follow along on GitHub", url: EXTENSION_URL },
      toggle: {
        label: "Start browser links without asking",
        hint: "Left off, they wait in the queue for a click — any page can send one, not just the extension.",
        get: () => app.browserLinksAutoStart,
        set: (on: boolean) => void app.setBrowserLinksAutoStart(on),
      },
    },
  ];

  let index = $state(0);
  const step = $derived(steps[index]);
  const last = $derived(index === steps.length - 1);

  function next() {
    if (last) app.dismissTour();
    else index += 1;
  }
</script>

<div class="scrim">
  <div class="card">
    <div class="art" data-icon={step.icon}>
      {#if step.icon === "tray"}
        <svg viewBox="0 0 24 24"
          ><path
            d="M3 15h18M5 19h14a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2z"
            stroke="currentColor"
            stroke-width="1.7"
            fill="none"
            stroke-linecap="round"
            stroke-linejoin="round"
          /></svg
        >
      {:else if step.icon === "drop"}
        <svg viewBox="0 0 24 24"
          ><g
            stroke="currentColor"
            stroke-width="1.7"
            fill="none"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M12 15V4m0 0L8 8m4-4 4 4" />
            <path d="M4 15v3a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3" />
          </g></svg
        >
      {:else if step.icon === "link"}
        <svg viewBox="0 0 24 24"
          ><g
            stroke="currentColor"
            stroke-width="1.7"
            fill="none"
            stroke-linecap="round"
          >
            <path d="M10 13a5 5 0 0 0 7 0l3-3a5 5 0 0 0-7-7l-1 1" />
            <path d="M14 11a5 5 0 0 0-7 0l-3 3a5 5 0 0 0 7 7l1-1" />
          </g></svg
        >
      {:else if step.icon === "grab"}
        <svg viewBox="0 0 24 24"
          ><g
            stroke="currentColor"
            stroke-width="1.7"
            fill="none"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M9 11V5.5a1.5 1.5 0 0 1 3 0V11" />
            <path d="M12 11V4.5a1.5 1.5 0 0 1 3 0V11" />
            <path d="M15 11V6.5a1.5 1.5 0 0 1 3 0V14a6 6 0 0 1-6 6h-1a6 6 0 0 1-6-6v-2a1.5 1.5 0 0 1 3 0" />
          </g></svg
        >
      {:else}
        <svg viewBox="0 0 24 24"
          ><path
            d="M10 3h4v3a2 2 0 1 0 4 0V3h3v4a2 2 0 1 1 0 4v6a2 2 0 0 1-2 2h-5v-3a2 2 0 1 0-4 0v3H5a2 2 0 0 1-2-2v-5h1a2 2 0 1 0 0-4H3V5a2 2 0 0 1 2-2h5z"
            stroke="currentColor"
            stroke-width="1.6"
            fill="none"
            stroke-linejoin="round"
          /></svg
        >
      {/if}
    </div>

    <h2>{step.title}</h2>
    <p>{step.body}</p>

    <!-- Settable here rather than described. A tour that says "there's an
         option for that in settings" has just given someone homework. -->
    {#if step.toggle}
      <label class="opt">
        <input
          type="checkbox"
          checked={step.toggle.get()}
          onchange={(e) => step.toggle?.set(e.currentTarget.checked)}
        />
        <span class="opt-text">
          <span class="opt-label">{step.toggle.label}</span>
          <span class="opt-hint">{step.toggle.hint}</span>
        </span>
      </label>
    {/if}

    {#if step.action}
      <button class="link" onclick={() => openUrl(step.action.url)}>
        {step.action.label}
      </button>
    {/if}

    <div class="foot">
      <div class="dots">
        {#each steps as _, i (i)}
          <span class="dot" class:on={i === index}></span>
        {/each}
      </div>
      <div class="buttons">
        {#if !last}
          <button class="skip" onclick={() => app.dismissTour()}>Skip</button>
        {/if}
        <button class="next" onclick={next}>{last ? "Get started" : "Next"}</button>
      </div>
    </div>
  </div>
</div>

<style>
  .scrim {
    position: absolute;
    inset: 0;
    z-index: 20;
    display: grid;
    place-items: center;
    padding: 18px;
    background: rgba(10, 11, 17, 0.82);
    backdrop-filter: blur(10px);
    animation: fade 0.25s var(--ease-spring);
  }

  @keyframes fade {
    from {
      opacity: 0;
    }
  }

  .card {
    width: 100%;
    max-width: 340px;
    /* A step carrying both a paragraph and a toggle is the tallest thing here,
       and the window is only 660px. Scroll rather than run off the bottom. */
    max-height: 100%;
    overflow-y: auto;
    padding: 22px 20px 16px;
    border-radius: var(--radius-lg);
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid var(--hairline-bright);
    text-align: center;
    animation: rise 0.3s var(--ease-spring);
  }

  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(10px) scale(0.98);
    }
  }

  .art {
    display: grid;
    place-items: center;
    width: 52px;
    height: 52px;
    margin: 0 auto 14px;
    border-radius: 15px;
    background: rgba(88, 101, 242, 0.18);
    color: var(--blurple-bright);
  }

  .art svg {
    width: 26px;
    height: 26px;
  }

  h2 {
    font-size: 15px;
    font-weight: 650;
    margin-bottom: 7px;
  }

  p {
    font-size: 12px;
    line-height: 1.55;
    color: var(--text-dim);
  }

  .opt {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    width: 100%;
    margin-top: 14px;
    padding: 10px 12px;
    text-align: left;
    border-radius: var(--radius);
    background: var(--surface);
    border: 1px solid var(--hairline);
    cursor: pointer;
  }

  .opt:hover {
    background: var(--surface-hover);
  }

  .opt input {
    flex-shrink: 0;
    margin-top: 1px;
    accent-color: var(--blurple);
  }

  .opt-text {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .opt-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
  }

  .opt-hint {
    font-size: 11px;
    line-height: 1.45;
    color: var(--text-faint);
  }

  .link {
    margin-top: 12px;
    font-size: 11.5px;
    font-weight: 600;
    color: var(--blurple-bright);
  }

  .link:hover {
    text-decoration: underline;
  }

  .foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-top: 20px;
  }

  .dots {
    display: flex;
    gap: 5px;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: rgba(255, 255, 255, 0.18);
    transition: background 0.2s, transform 0.2s;
  }

  .dot.on {
    background: var(--blurple-bright);
    transform: scale(1.25);
  }

  .buttons {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .skip {
    padding: 6px 10px;
    border-radius: var(--radius-sm);
    font-size: 11.5px;
    color: var(--text-faint);
  }

  .skip:hover {
    color: var(--text);
  }

  .next {
    padding: 7px 16px;
    border-radius: var(--radius);
    background: var(--blurple);
    color: #fff;
    font-size: 11.5px;
    font-weight: 650;
    box-shadow: 0 4px 14px -6px rgba(88, 101, 242, 0.95);
  }

  .next:hover {
    filter: brightness(1.12);
  }
</style>
