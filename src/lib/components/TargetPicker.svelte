<script lang="ts">
  import { TIERS } from "../presets";
  import { app } from "../state.svelte";

  // Whichever tier matches the current target is highlighted; anything else
  // means the user has typed their own number.
  let matched = $derived(TIERS.find((t) => t.bytes === app.settings.targetBytes));
  let custom = $state(false);
  let customMb = $state(25);

  // Opening custom mode should start from whatever's currently set.
  $effect(() => {
    if (!matched && !custom) {
      custom = true;
      customMb = Math.round(app.settings.targetBytes / 1_000_000);
    }
  });

  function pickTier(bytes: number) {
    custom = false;
    app.update({ targetBytes: bytes });
  }

  function openCustom() {
    custom = true;
    customMb = Math.round(app.settings.targetBytes / 1_000_000) || 25;
    app.update({ targetBytes: Math.max(1, customMb) * 1_000_000 });
  }

  function onCustomInput(event: Event) {
    const raw = Number((event.currentTarget as HTMLInputElement).value);
    if (!Number.isFinite(raw)) return;
    customMb = raw;
    const clamped = Math.min(Math.max(raw, 0.5), 10_000);
    app.update({ targetBytes: Math.round(clamped * 1_000_000) });
  }
</script>

<section class="target">
  <div class="label">Target size</div>

  <div class="tiers">
    {#each TIERS as tier (tier.id)}
      <button
        class="tier"
        class:active={!custom && matched?.id === tier.id}
        onclick={() => pickTier(tier.bytes)}
      >
        <span class="tier-name">{tier.label}</span>
        <span class="tier-size tnum">{tier.sub}</span>
      </button>
    {/each}
  </div>

  <div class="custom-row">
    <button class="tier custom-toggle" class:active={custom} onclick={openCustom}>
      Custom
    </button>

    {#if custom}
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
  </div>
</section>

<style>
  .target {
    padding: 12px 14px 14px;
    border-bottom: 1px solid var(--hairline);
    flex-shrink: 0;
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
    grid-template-columns: repeat(4, 1fr);
    gap: 6px;
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

  .tier:hover {
    background: var(--surface-hover);
  }

  .tier:active {
    transform: scale(0.97);
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

  .custom-toggle {
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
</style>
