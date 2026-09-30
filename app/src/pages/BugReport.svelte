<script lang="ts">
  import { Bug, Copy, Mail } from "@lucide/svelte";
  import { api, errorText } from "../lib/api";
  import { t } from "../lib/i18n.svelte";
  let copied = $state(false);
  let error = $state<string | null>(null);
  async function copy() {
    try {
      await navigator.clipboard.writeText(t().bug.email);
      copied = true;
      setTimeout(() => (copied = false), 2000);
    } catch (e) {
      error = errorText(e);
    }
  }
</script>

<header class="head"><h1>{t().bug.title}</h1></header>
<section class="card">
  <Bug size={28} aria-hidden="true" />
  <p>{t().bug.body}</p>
  <p class="email" dir="ltr">{t().bug.email}</p>
  <div class="buttons">
    <button class="btn primary" onclick={() => api.reportBug("From the Bug report page.").catch((e) => (error = errorText(e)))}>
      <Mail size={16} aria-hidden="true" /> {t().bug.open}
    </button>
    <button class="btn" onclick={copy}><Copy size={16} aria-hidden="true" /> {copied ? t().bug.copied : t().bug.copy}</button>
  </div>
  {#if error}<p class="muted">{error}</p>{/if}
  <p class="muted">{t().bug.include}</p>
</section>

<style>
  .head { margin-bottom: 16px; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 800; }
  .card { background: var(--card); border-radius: var(--radius); padding: 22px 24px; display: grid; gap: 12px; max-width: 680px; color: var(--text); }
  .card p { margin: 0; line-height: 1.7; }
  .email { font-size: 1.2rem; font-weight: 700; color: var(--accent-text); user-select: all; }
  .buttons { display: flex; gap: 10px; flex-wrap: wrap; }
  .btn { all: unset; display: inline-flex; align-items: center; gap: 8px; padding: 9px 16px; border-radius: 10px; background: var(--card-2); cursor: pointer; font-size: 0.9rem; }
  .btn.primary { background: var(--accent); color: white; font-weight: 600; }
  .btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .muted { color: var(--text-2); font-size: 0.88rem; }
</style>
