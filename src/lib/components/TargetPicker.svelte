<script lang="ts">
  import { QUALITY_LEVELS, TIERS } from "../presets";
  import type { Settings } from "../types";

  interface Props {
    settings: Settings;
    onChange: (patch: Partial<Settings>) => void;
    /** The editor uses a tighter version of the same control. */
    compact?: boolean;
    /**
     * Size of the video this applies to, when it applies to just one.
     *
     * The main window's picker is a default for whatever gets dropped next, so
     * it has no file to measure against and leaves every tier available.
     */
    sourceBytes?: number | null;
  }

  let { settings, onChange, compact = false, sourceBytes = null }: Props = $props();

  const sizeMode = $derived(settings.mode === "size");
  const matched = $derived(TIERS.find((t) => t.bytes === settings.targetBytes));

  /**
   * A tier at or above the file's own size can't do anything.
   *
   * Asking to fit a 12 MB video into 500 MB either passes it through untouched
   * or, once it's been cropped, re-encodes it to a target it was never near —
   * so the option is offered but not selectable, and says why on hover.
   */
  function tierIsPointless(bytes: number): boolean {
    return sourceBytes !== null && sourceBytes > 0 && sourceBytes <= bytes;
  }

  let custom = $state(false);
  let customMb = $state(25);

  // Opening custom mode should start from whatever's currently set.
  $effect(() => {
    if (sizeMode && !matched && !custom) {
      custom = true;
      customMb = Math.round(settings.targetBytes / 1_000_000);
    }
  });

  function pickTier(bytes: number) {
    custom = false;
    onChange({ mode: "size", targetBytes: bytes });
  }

  function openCustom() {
    custom = true;
    customMb = Math.round(settings.targetBytes / 1_000_000) || 25;
    onChange({ mode: "size", targetBytes: Math.max(1, customMb) * 1_000_000 });
  }

  function onCustomInput(event: Event) {
    const raw = Number((event.currentTarget as HTMLInputElement).value);
    if (!Number.isFinite(raw)) return;
    customMb = raw;
    const clamped = Math.min(Math.max(raw, 0.5), 10_000);
    onChange({ mode: "size", targetBytes: Math.round(clamped * 1_000_000) });
  }

  const activeQuality = $derived(
    QUALITY_LEVELS.find((q) => q.id === settings.quality),
  );
</script>

<section class="target" class:compact>
  {#if !compact}
    <div class="label">Target size</div>
  {/if}

  <div class="tiers">
    {#each TIERS as tier (tier.id)}
      {@const pointless = tierIsPointless(tier.bytes)}
      <button
        class="tier"
        class:active={sizeMode && !custom && matched?.id === tier.id}
        disabled={pointless}
        title={pointless
          ? `This video is already under ${tier.sub}`
          : `Fit under ${tier.sub}`}
        onclick={() => pickTier(tier.bytes)}
      >
        <span class="tier-name">{tier.label}</span>
        <span class="tier-size tnum">{tier.sub}</span>
      </button>
    {/each}
  </div>

  <div class="custom-row">
    <button
      class="tier flat"
      class:active={sizeMode && custom}
      onclick={openCustom}
    >
      Custom
    </button>

    {#if sizeMode && custom}
      <div class="custom-input">
        <input
          type="number"
          min="0.5"
          max="10000"
          step="0.5"
          class="tnum"
          value={customMb}
          oninput={onCustomInput}
          aria-label="Custom target size in megabytes"
        />
        <span class="unit">MB</span>
      </div>
    {/if}

    <!-- The escape hatch for long recordings, where no megabyte figure is
         the obviously right answer. -->
    <button
      class="tier flat"
      class:active={settings.mode === "quality"}
      onclick={() => onChange({ mode: "quality" })}
      title="Compress well and accept whatever size results"
    >
      No limit
    </button>

    <!-- For material that's already been compressed once, and for anyone who
         only came here to crop or trim. -->
    <button
      class="tier flat"
      class:active={settings.mode === "keep"}
      onclick={() => onChange({ mode: "keep" })}
      title="Apply the edits and leave the quality alone"
    >
      Don't compress
    </button>
  </div>

  {#if settings.mode === "keep"}
    <p class="hint">
      Trims without re-encoding, so it's instant and loses nothing. Cropping
      still has to re-encode, at near-lossless quality.
    </p>
  {/if}

  {#if settings.mode === "quality"}
    <div class="quality">
      <div class="quality-row">
        {#each QUALITY_LEVELS as level (level.id)}
          <button
            class="tier flat"
            class:active={settings.quality === level.id}
            onclick={() => onChange({ quality: level.id })}
          >
            {level.label}
          </button>
        {/each}
      </div>
      {#if activeQuality}
        <p class="hint">{activeQuality.hint}</p>
      {/if}
    </div>
  {/if}
</section>

<style>
  .target {
    padding: 12px 14px 14px;
    border-bottom: 1px solid var(--hairline);
    flex-shrink: 0;
  }

  .target.compact {
    padding: 0;
    border-bottom: none;
  }

  .label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.09em;
    text-transform: uppercase;
    color: var(--text-faint);
    margin-bottom: 9px;
  }

  .tiers {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 5px;
  }

  .tier {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 8px 4px;
    border-radius: var(--radius);
    background: var(--surface);
    border: 1px solid transparent;
    transition: background 0.15s, border-color 0.15s, transform 0.15s var(--ease-spring);
  }

  .compact .tier {
    padding: 6px 4px;
  }

  .tier:hover:not(:disabled) {
    background: var(--surface-hover);
  }

  .tier:active:not(:disabled) {
    transform: scale(0.97);
  }

  /* Still legible, so it's clear the option exists and why it's unavailable —
     the tooltip does the explaining. */
  .tier:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }

  .tier.active {
    background: rgba(88, 101, 242, 0.2);
    border-color: rgba(124, 136, 255, 0.55);
    box-shadow: 0 0 0 1px rgba(124, 136, 255, 0.18),
      0 4px 16px -6px rgba(88, 101, 242, 0.9);
  }

  .tier-name {
    font-size: 11px;
    font-weight: 600;
    color: var(--text);
  }

  .tier-size {
    font-size: 10px;
    color: var(--text-faint);
  }

  .tier.active .tier-size {
    color: var(--blurple-bright);
  }

  .custom-row {
    display: flex;
    align-items: stretch;
    gap: 6px;
    margin-top: 6px;
  }

  .flat {
    flex: 0 0 auto;
    padding: 7px 14px;
    font-size: 11px;
    font-weight: 600;
    justify-content: center;
  }

  .custom-input {
    display: flex;
    align-items: center;
    flex: 1;
    min-width: 0;
    padding: 0 10px 0 12px;
    border-radius: var(--radius);
    background: var(--surface);
    border: 1px solid var(--hairline);
  }

  .custom-input:focus-within {
    border-color: rgba(124, 136, 255, 0.6);
    background: var(--surface-hover);
  }

  .custom-input input {
    flex: 1;
    width: 100%;
    min-width: 0;
    background: none;
    border: none;
    outline: none;
    font-size: 13px;
    font-weight: 600;
    padding: 7px 0;
  }

  /* The spinner arrows crowd a field this small. */
  .custom-input input::-webkit-outer-spin-button,
  .custom-input input::-webkit-inner-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }

  .unit {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-faint);
  }

  .quality {
    margin-top: 6px;
  }

  .quality-row {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 6px;
  }

  .hint {
    margin-top: 6px;
    font-size: 10.5px;
    line-height: 1.45;
    color: var(--text-faint);
  }
</style>
