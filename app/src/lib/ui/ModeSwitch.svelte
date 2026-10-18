<script lang="ts">
  // The switch for a mode (game mode, replay buffer, overlay, slowing
  // background apps): larger than a setting's switch, with its state in
  // words beside it, so on and off read at a glance. `waiting`: nothing to
  // act on until a game runs (game mode following the games; slowing
  // background apps only while playing).
  import { t } from "../i18n.svelte";

  let {
    checked,
    waiting = false,
    disabled = false,
    label,
    onchange,
  }: { checked: boolean; waiting?: boolean; disabled?: boolean; label: string; onchange: (v: boolean) => void } = $props();
  const state = $derived(waiting ? "waiting" : checked ? "on" : "off");
</script>

<span class="mode-switch">
  <span class="badge {state}">{t().modeState[state]}</span>
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
</span>

<style>
  .mode-switch { display: inline-flex; align-items: center; gap: 10px; flex: none; }
  .badge {
    padding: 3px 12px;
    border-radius: 999px;
    font-size: 0.8rem;
    font-weight: 700;
    border: 1px solid transparent;
  }
  .badge.on { color: #062a1c; background: var(--ok); }
  .badge.off { color: var(--text-2); background: var(--card-2); border-color: var(--line); }
  .badge.waiting { color: #fff; background: var(--accent); }
  .switch {
    all: unset;
    box-sizing: border-box;
    width: 58px;
    height: 32px;
    border-radius: 999px;
    padding: 3px;
    cursor: pointer;
    display: flex;
    /* Off is a clear outline with a light knob, not grey on grey. */
    background: #2c2c36;
    border: 1.5px solid #5a5a6a;
    transition: background 180ms ease-out, border-color 180ms ease-out;
  }
  .knob {
    width: 23px;
    height: 23px;
    border-radius: 50%;
    background: #d6d4e2;
    box-shadow: 0 1px 3px rgb(0 0 0 / 0.5);
    transition: translate 180ms ease-out;
  }
  .switch[aria-checked="true"] { background: var(--cat, var(--accent)); border-color: var(--cat, var(--accent)); }
  .switch[aria-checked="true"] .knob { background: #fff; translate: calc(26px * var(--dir, 1)); }
  :global([dir="rtl"]) .switch { --dir: -1; }
  .switch:disabled { cursor: not-allowed; opacity: 0.45; }
  .switch:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 3px; }
</style>
