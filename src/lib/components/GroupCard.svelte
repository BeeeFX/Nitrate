<script lang="ts">
  import { slide } from "svelte/transition";
  import { formatSize } from "../format";
  import { app } from "../state.svelte";
  import type { Job } from "../types";
  import JobCard from "./JobCard.svelte";

  interface Props {
    /** Every item that came out of one post, in the order the post listed them. */
    items: Job[];
  }

  let { items }: Props = $props();

  // Collapsed by default: the whole point is that a four-image post takes one
  // row rather than four. Opening it is a deliberate act.
  let open = $state(false);

  const title = $derived(items[0]?.groupTitle ?? "Post");
  const done = $derived(items.filter((j) => j.status === "done").length);
  const working = $derived(
    items.some((j) => j.status === "running" || j.status === "queued"),
  );
  const failed = $derived(items.filter((j) => j.status === "failed").length);

  /**
   * A post of nothing but photos has nothing to compress.
   *
   * They are handed over as they arrive, so a Compress button would promise
   * work that never happens and then report "done" for having done nothing.
   */
  const anythingToCompress = $derived(items.some((j) => j.mediaKind !== "photo"));

  /** Only meaningful once everything has finished; partial totals mislead. */
  const totalBytes = $derived(
    done === items.length
      ? items.reduce((sum, j) => sum + (j.finalBytes ?? 0), 0)
      : null,
  );

  /** A short "2 photos, 1 GIF" rather than a bare count. */
  const summary = $derived.by(() => {
    const counts = { photo: 0, gif: 0, video: 0 } as Record<string, number>;
    for (const job of items) counts[job.mediaKind ?? "video"] += 1;

    const label = (n: number, one: string, many: string) =>
      `${n} ${n === 1 ? one : many}`;

    return [
      counts.photo && label(counts.photo, "photo", "photos"),
      counts.gif && label(counts.gif, "GIF", "GIFs"),
      counts.video && label(counts.video, "video", "videos"),
    ]
      .filter(Boolean)
      .join(" · ");
  });

  /** The first few posters, so the row shows what's inside while shut. */
  const previews = $derived(items.filter((j) => j.thumbnail).slice(0, 4));

  /**
   * One bar for the whole post, averaged across its items.
   *
   * Each item has its own bar once the group is open, but collapsed — which is
   * how it spends most of its life — there was nothing at all to say work was
   * happening.
   */
  const overall = $derived(
    items.reduce((sum, j) => sum + (j.status === "done" ? 1 : j.progress), 0) /
      Math.max(1, items.length),
  );

  function compressAll() {
    for (const job of items) {
      if (job.status === "queued" || job.status === "held") void app.start(job.id);
    }
  }

  function removeAll() {
    for (const job of items) app.remove(job.id);
  }
</script>

<article class="group" class:open>
  <div class="head">
    <button
      class="disclose"
      onclick={() => (open = !open)}
      aria-expanded={open}
      title={open ? "Collapse" : "Show what's inside"}
    >
      <span class="stack">
        {#each previews as job (job.id)}
          <img src={job.thumbnail} alt="" draggable="false" />
        {/each}
        {#if previews.length === 0}
          <span class="stack-empty">{items.length}</span>
        {/if}
      </span>

      <span class="text">
        <span class="name" title={title}>{title}</span>
        <span class="sub">
          {summary}
          {#if failed > 0}
            <span class="warn">· {failed} failed</span>
          {:else if working}
            <span class="dim">· {done} of {items.length} done</span>
          {:else if totalBytes !== null}
            <span class="dim">· {formatSize(totalBytes)}</span>
          {/if}
        </span>
      </span>

      <svg class="chevron" viewBox="0 0 24 24" aria-hidden="true">
        <path
          d="M8 10l4 4 4-4"
          stroke="currentColor"
          stroke-width="2"
          fill="none"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </svg>
    </button>

    <div class="acts">
      {#if anythingToCompress && items.some((j) => j.status === "queued" || j.status === "held")}
        <button class="act" onclick={compressAll} title="Compress all of them">
          <svg viewBox="0 0 24 24"><path d="M8 5l11 7-11 7z" fill="currentColor" /></svg>
        </button>
      {/if}
      <button class="act dim-act" onclick={removeAll} title="Remove all of them">
        <svg viewBox="0 0 24 24"
          ><path
            d="M6 7h12M9 7V5h6v2m-8 0 1 12h8l1-12"
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

  {#if working}
    <div class="bar" aria-hidden="true">
      <span style:width="{Math.round(overall * 100)}%"></span>
    </div>
  {/if}

  {#if open}
    <!-- `slide` rather than a height animation: the contents are a variable
         number of cards, so there's no height to animate towards. -->
    <div class="inner" transition:slide={{ duration: 220 }}>
      {#each items as job (job.id)}
        <JobCard {job} />
      {/each}
    </div>
  {/if}
</article>

<style>
  .group {
    border-radius: var(--radius);
    background: rgba(255, 255, 255, 0.03);
    border: 1px solid var(--hairline);
    overflow: hidden;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
  }

  .disclose {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: 1;
    min-width: 0;
    text-align: left;
    border-radius: var(--radius-sm);
  }

  /* Overlapping posters, so a glance says what's in there. */
  .stack {
    display: flex;
    flex-shrink: 0;
    align-items: center;
  }

  .stack img {
    width: 34px;
    height: 34px;
    object-fit: cover;
    border-radius: var(--radius-sm);
    border: 1.5px solid var(--surface);
    background: rgba(0, 0, 0, 0.3);
  }

  .stack img:not(:first-child) {
    margin-left: -14px;
  }

  .stack-empty {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: var(--radius-sm);
    background: rgba(0, 0, 0, 0.28);
    font-size: 12px;
    font-weight: 700;
    color: var(--text-faint);
  }

  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .name {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sub {
    font-size: 11px;
    color: var(--text-faint);
  }

  .warn {
    color: var(--danger);
  }

  .dim {
    color: var(--text-faint);
  }

  .chevron {
    width: 18px;
    height: 18px;
    flex-shrink: 0;
    margin-left: auto;
    color: var(--text-faint);
    transition: transform 0.2s var(--ease-spring);
  }

  .group.open .chevron {
    transform: rotate(180deg);
  }

  .acts {
    display: flex;
    gap: 2px;
    flex-shrink: 0;
  }

  .act {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: var(--radius-sm);
    color: var(--text-dim);
  }

  .act:hover {
    background: var(--surface-hover);
    color: var(--text);
  }

  .act svg {
    width: 15px;
    height: 15px;
  }

  .dim-act {
    color: var(--text-faint);
  }

  .bar {
    height: 2px;
    background: rgba(255, 255, 255, 0.07);
    overflow: hidden;
  }

  .bar span {
    display: block;
    height: 100%;
    background: var(--blurple-bright);
    transition: width 0.25s ease;
  }

  .inner {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 0 8px 8px;
  }
</style>
