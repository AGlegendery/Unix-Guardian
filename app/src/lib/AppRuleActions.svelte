<script lang="ts">
  // Limit / Block for one app (or its state and Allow when it has a rule),
  // and the limit form under it when open. Used by every app list on
  // Monitoring's Traffic and Ping tabs.
  import { Ban, Gauge } from "@lucide/svelte";
  import { num } from "./format";
  import { t } from "./i18n.svelte";
  import type { NetControl } from "./netcontrol.svelte";
  import Segmented from "./ui/Segmented.svelte";

  let { ctl, name, part = "buttons" }: { ctl: NetControl; name: string; part?: "buttons" | "form" } = $props();

  const UP = [0, 256, 1000, 4000];
  const DOWN = [0, 2000, 8000, 32000];
  const kb = (k: number) => (k === 0 ? "—" : k >= 1000 ? `${num(k / 1000, k % 1000 ? 1 : 0)} Mbit/s` : `${k} kbit/s`);
  const state = $derived(ctl.stateOf(name));
  const offline = $derived(!ctl.status?.available);
</script>

{#if part === "buttons"}
  <div class="actions">
    {#if state !== ""}
      <span class="state {state}">{state === "blocked" ? t().ping.blocked : t().ping.limited}</span>
      <button class="btn" disabled={ctl.busy} onclick={() => ctl.allow(name)}>{t().ping.allow}</button>
    {:else}
      <button class="btn" disabled={ctl.busy || offline} onclick={() => ctl.openLimit(name)}><Gauge size={15} aria-hidden="true" /> {t().ping.limit}</button>
      <button class="btn danger" disabled={ctl.busy || offline} onclick={() => ctl.block(name)}><Ban size={15} aria-hidden="true" /> {t().ping.block}</button>
    {/if}
  </div>
{:else if ctl.editing?.app.toLowerCase() === name.toLowerCase()}
  {@const e = ctl.editing}
  <div class="form">
    <div class="field">
      <span>{t().ping.upCap}</span>
      <Segmented label={t().ping.upCap} value={String(e.up)} options={UP.map((k) => ({ value: String(k), label: kb(k) }))} onchange={(v) => (e.up = Number(v))} />
    </div>
    <div class="field">
      <span>{t().ping.downCap}</span>
      <Segmented label={t().ping.downCap} value={String(e.down)} options={DOWN.map((k) => ({ value: String(k), label: kb(k) }))} onchange={(v) => (e.down = Number(v))} />
    </div>
    <div class="field">
      <Segmented label={t().ping.gaming} value={e.when} options={[{ value: "gaming", label: t().ping.gaming }, { value: "always", label: t().ping.always }]} onchange={(v) => (e.when = v)} />
    </div>
    <div class="field buttons">
      <button class="btn primary" disabled={ctl.busy || (e.up === 0 && e.down === 0)} onclick={() => ctl.saveLimit()}>{t().ping.save}</button>
      <button class="btn" onclick={() => (ctl.editing = null)}>{t().ping.cancel}</button>
    </div>
  </div>
{/if}

<style>
  .actions { display: flex; gap: 8px; align-items: center; justify-content: flex-end; flex-wrap: wrap; }
  .btn { all: unset; display: inline-flex; align-items: center; gap: 6px; padding: 6px 12px; border-radius: 9px; background: var(--card-2); color: var(--text); font-size: 0.85rem; cursor: pointer; white-space: nowrap; }
  .btn:hover:not(:disabled) { background: color-mix(in oklab, var(--card-2) 80%, var(--text) 8%); }
  .btn.primary { background: var(--accent); color: white; font-weight: 600; }
  .btn.danger { color: var(--danger); }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .state { padding: 2px 9px; border-radius: 999px; font-size: 0.75rem; background: var(--card-2); color: var(--text-2); }
  .state.blocked { color: var(--danger); }
  .state.limited { color: var(--accent-text); }
  .form { display: flex; flex-wrap: wrap; gap: 12px 20px; align-items: flex-end; padding: 12px; margin-top: 8px; border-radius: var(--radius-sm); background: color-mix(in oklab, var(--card-2) 50%, var(--card)); }
  .field { display: grid; gap: 6px; font-size: 0.82rem; color: var(--text-2); }
  .field.buttons { display: flex; gap: 8px; }
</style>
