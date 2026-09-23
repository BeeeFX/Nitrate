<script lang="ts">
  import { untrack } from "svelte";
  import { formatDuration, formatSize, linkTimestamp, parseClock } from "../format";
  import { app } from "../state.svelte";
  import type { Job } from "../types";

  interface Props {
    /** A long link that hasn't been downloaded yet. */
    job: Job;
  }

  let { job }: Props = $props();

  const length = $derived(job.knownDuration ?? 0);
  const settings = $derived(app.settingsFor(job));

  // Starts where the link was shared from, when it says — a YouTube `t=` or a
  // Twitch timestamp is nearly always the moment someone meant — and runs a
  // minute from there, which is a starting point to adjust rather than a guess
  // at the answer. Read once, when the card appears: after that the fields
  // are the user's, and a later update to the job mustn't write over them.
  const initial = untrack(() => {
    const duration = job.knownDuration ?? 0;
    const sharedAt = linkTimestamp(job.url ?? "");
    const start = Math.min(sharedAt ?? 0, Math.max(0, duration - 1));
    const end = Math.min(start + 60, duration || start + 60);
    return { sharedAt, start, end };
  });
  const sharedAt = initial.sharedAt;
  let fromText = $state(formatDuration(initial.start));
  let toText = $state(formatDuration(initial.end));

  const from = $derived(parseClock(fromText));
  const to = $derived(parseClock(toText));

  const problem = $derived.by(() => {
    if (from === null) return "The start isn't a time — try 1:23 or 1:02:03.";
    if (to === null) return "The end isn't a time — try 1:23 or 1:02:03.";
    if (to <= from) return "The end has to come after the start.";
    if (length > 0 && from >= length) return `It's only ${formatDuration(length)} long.`;
    return null;
  });

  // Clamped to the video, so an end typed past it just means "to the end".
  const part = $derived(
    from !== null && to !== null
      ? { start: from, end: length > 0 ? Math.min(to, length) : to }
      : null,
  );

  function downloadPart(event: SubmitEvent) {
    event.preventDefault();
    if (problem || !part) return;
    void app.startPart(job.id, part.start, part.end);
  }
</script>

<form class="part" onsubmit={downloadPart}>
  <p class="why">
    {formatDuration(length)} long — squeezing all of it into
    {formatSize(settings.targetBytes)} would look poor, and it's a big download.
    Fetch just the part you want:
  </p>

  <div class="range">
    <label>
      <span>From</span>
      <input
        bind:value={fromText}
        inputmode="numeric"
        autocomplete="off"
        spellcheck="false"
        aria-invalid={from === null}
      />
    </label>
    <label>
      <span>To</span>
      <input
        bind:value={toText}
        inputmode="numeric"
        autocomplete="off"
        spellcheck="false"
        aria-invalid={to === null || (from !== null && to <= from)}
      />
    </label>
    <button class="go" type="submit" disabled={problem !== null}>Download part</button>
  </div>

  {#if problem}
    <p class="problem" role="status">{problem}</p>
  {:else if part}
    <p class="sub">
      {formatDuration(part.end - part.start)} of it{#if sharedAt !== null}, from where the
        link was shared{/if}. Or <button type="button" class="link" onclick={() => app.start(job.id)}
        >download all of it</button
      > to trim in the editor.
    </p>
  {/if}
</form>

<style>
  .part {
    margin-top: 9px;
    padding: 8px 9px 9px;
    border-radius: var(--radius-sm);
    background: rgba(250, 168, 26, 0.1);
    font-size: 10.5px;
    line-height: 1.45;
    color: rgba(250, 200, 120, 0.92);
  }

  .why {
    margin: 0 0 7px;
  }

  .range {
    display: flex;
    align-items: flex-end;
    gap: 6px;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1 1 0;
  }

  label span {
    font-size: 9.5px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-dim);
  }

  input {
    width: 100%;
    box-sizing: border-box;
    padding: 5px 7px;
    border-radius: 6px;
    border: 1px solid var(--hairline-bright);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
  }

  input:focus-visible {
    outline: 2px solid var(--blurple-bright);
    outline-offset: 1px;
  }

  input[aria-invalid="true"] {
    border-color: rgba(237, 66, 69, 0.7);
  }

  .go {
    flex: none;
    padding: 6px 10px;
    border: none;
    border-radius: 6px;
    background: var(--blurple);
    color: #fff;
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
  }

  .go:hover:not(:disabled) {
    background: var(--blurple-bright);
  }

  .go:disabled {
    opacity: 0.45;
    cursor: default;
  }

  .sub,
  .problem {
    margin: 6px 0 0;
  }

  .problem {
    color: #ff8f91;
  }

  .link {
    padding: 0;
    border: none;
    background: none;
    color: inherit;
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }
</style>
