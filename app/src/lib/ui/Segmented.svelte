<script lang="ts" generics="T extends string">
  // A segmented control (a small radio group). A disabled option carries its
  // reason as a tooltip; pages also print the reason next to the control.
  type Option = { value: T; label: string; disabled?: boolean; why?: string };
  let { options, value, label, onchange }: { options: Option[]; value: T; label: string; onchange: (v: T) => void } = $props();
</script>

<div class="seg" role="radiogroup" aria-label={label}>
  {#each options as o (o.value)}
    <button
      type="button"
      role="radio"
      aria-checked={o.value === value}
      disabled={o.disabled}
      title={o.why}
      onclick={() => onchange(o.value)}
    >{o.label}</button>
  {/each}
</div>

<style>
  .seg {
    display: inline-flex;
    flex-wrap: wrap;
    max-width: 100%;
    padding: 3px;
    gap: 2px;
    border-radius: 12px;
    background: var(--card-2);
  }
  button {
    all: unset;
    padding: 6px 14px;
    border-radius: 9px;
    font-size: 0.86rem;
    color: var(--text-2);
    cursor: pointer;
    transition: background 150ms ease-out, color 150ms ease-out;
  }
  button:hover:not(:disabled) { color: var(--text); }
  button[aria-checked="true"] { background: var(--accent); color: #fff; font-weight: 600; }
  button:disabled { color: var(--text-3); cursor: not-allowed; }
  button:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
</style>
