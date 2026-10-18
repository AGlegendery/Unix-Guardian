<script lang="ts">
  // A toggle switch: role="switch" so screen readers announce on/off, and the
  // knob moves toward the reading direction's end when on.
  let {
    checked,
    disabled = false,
    label,
    onchange,
  }: { checked: boolean; disabled?: boolean; label: string; onchange: (v: boolean) => void } = $props();
</script>

<button
  type="button"
  class="switch"
  role="switch"
  aria-checked={checked}
  aria-label={label}
  {disabled}
  onclick={() => onchange(!checked)}
>
  <span class="knob" aria-hidden="true"></span>
</button>

<style>
  .switch {
    all: unset;
    box-sizing: border-box;
    width: 42px;
    height: 24px;
    flex: none;
    border-radius: 999px;
    /* Off reads as off, not as a missing control: a visible outline and
       a light knob (the user found grey on dark grey hard to see). */
    background: #2c2c36;
    border: 1px solid #5a5a6a;
    padding: 2px;
    cursor: pointer;
    display: flex;
    transition: background 180ms ease-out, border-color 180ms ease-out;
  }
  .knob {
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: #c9c7d6;
    transition: translate 180ms ease-out, background 180ms ease-out;
  }
  .switch[aria-checked="true"] { background: var(--accent); border-color: var(--accent); }
  .switch[aria-checked="true"] .knob { background: #fff; translate: calc(18px * var(--dir, 1)); }
  :global([dir="rtl"]) .switch { --dir: -1; }
  .switch:disabled { cursor: not-allowed; opacity: 0.45; }
  .switch:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
</style>
