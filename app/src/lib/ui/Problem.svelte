<script lang="ts">
  // An error the user should read, with a way to report it as a bug. The
  // report carries the message, so the user does not have to copy it.
  import { api } from "../api";
  import { t } from "../i18n.svelte";
  let { text }: { text: string } = $props();
  let failed = $state<string | null>(null);
</script>

<div class="problem" role="alert">
  <p>{text}</p>
  <button onclick={() => api.reportBug(`Error shown: ${text}`).catch((e) => (failed = String(e)))}>
    {t().bug.inError} {t().bug.link}
  </button>
  {#if failed}<p class="failed">{failed}</p>{/if}
</div>

<style>
  .problem {
    margin: 0 0 14px;
    padding: 12px 16px;
    border-radius: var(--radius-sm);
    background: color-mix(in oklab, var(--danger) 12%, var(--card));
    color: var(--danger);
    font-size: 0.9rem;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 6px 16px;
  }
  p { margin: 0; }
  button { all: unset; cursor: pointer; font-size: 0.8rem; color: var(--text-2); text-decoration: underline; text-underline-offset: 3px; }
  button:hover { color: var(--text); }
  button:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; border-radius: 4px; }
  .failed { flex-basis: 100%; color: var(--text-2); font-size: 0.8rem; }
</style>
