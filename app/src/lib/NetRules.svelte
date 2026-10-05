<script lang="ts">
  // The block and limit rules in force (or waiting for a game), each with
  // Remove; and the VPN line when one carries the other apps' traffic.
  import { ShieldCheck } from "@lucide/svelte";
  import { api } from "./api";
  import { num, rate } from "./format";
  import { t } from "./i18n.svelte";
  import type { NetControl } from "./netcontrol.svelte";

  let { ctl }: { ctl: NetControl } = $props();
  const kb = (k: number) => (k === 0 ? "—" : k >= 1000 ? `${num(k / 1000, k % 1000 ? 1 : 0)} Mbit/s` : `${k} kbit/s`);
  const vpnApps = $derived(ctl.apps.filter((a) => a.category === "vpn"));
</script>

<section class="card">
  <h2><ShieldCheck size={17} aria-hidden="true" /> {t().ping.rules}</h2>
  {#if ctl.rules.length === 0}
    <p class="muted">{t().ping.noRules}</p>
  {:else}
    <ul class="list">
      {#each ctl.rules as r (r.app)}
        <li>
          <div class="who">
            <b dir="auto">{r.app}</b>
            <span dir="ltr">{r.block ? t().ping.blocked : `↑ ${kb(r.up_kbps)} · ↓ ${kb(r.down_kbps)}`}</span>
            <span class="muted">{r.when === "gaming" ? t().ping.gaming : t().ping.always}</span>
            <span class="state" class:on={r.active}>{r.active ? t().ping.active : t().ping.waitingGame}</span>
          </div>
          <button class="btn" disabled={ctl.busy} onclick={() => ctl.act(() => api.netRemoveRule(r.app))}>{t().ping.remove}</button>
        </li>
      {/each}
    </ul>
  {/if}
  {#each vpnApps as v (v.id)}
    {@const d = rate(v.rx_bps)}
    {@const u = rate(v.tx_bps)}
    <p class="vpnline"><b dir="auto">{v.name}</b> <span dir="ltr">↓ {d.value} {d.unit} · ↑ {u.value} {u.unit}</span> {ctl.status?.vpn_tun ? t().ping.vpnTun : t().ping.vpnProxy}</p>
  {/each}
  {#each ctl.status?.shaping_problems ?? [] as p}<p class="problem">{p}</p>{/each}
</section>

<style>
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; margin-bottom: 16px; min-width: 0; }
  h2 { margin: 0 0 10px; font-size: 0.98rem; font-weight: 700; display: flex; align-items: center; gap: 8px; }
  .muted { color: var(--text-2); margin: 0; font-size: 0.9rem; }
  .list { list-style: none; margin: 0; padding: 0; display: grid; gap: 4px; }
  .list li { display: flex; align-items: center; justify-content: space-between; gap: 14px; padding: 10px 0; border-bottom: 1px solid var(--line); flex-wrap: wrap; }
  .list li:last-child { border-bottom: 0; }
  .who { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; min-width: 0; }
  .state { padding: 2px 9px; border-radius: 999px; font-size: 0.75rem; background: var(--card-2); color: var(--text-2); }
  .state.on { color: var(--accent-text); }
  .btn { all: unset; display: inline-flex; align-items: center; gap: 6px; padding: 6px 12px; border-radius: 9px; background: var(--card-2); color: var(--text); font-size: 0.85rem; cursor: pointer; }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .vpnline { margin: 12px 0 0; font-size: 0.82rem; color: var(--text-2); display: flex; gap: 10px; flex-wrap: wrap; align-items: baseline; }
  .problem { margin: 12px 0 0; padding: 10px 14px; border-radius: var(--radius-sm); background: color-mix(in oklab, var(--danger) 12%, var(--card)); color: var(--danger); font-size: 0.85rem; }
</style>
