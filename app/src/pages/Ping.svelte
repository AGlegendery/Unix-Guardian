<script lang="ts">
  import Problem from "../lib/ui/Problem.svelte";
  import { Ban, Gauge, ShieldCheck } from "@lucide/svelte";
  import {
    api, errorText,
    type AppTraffic, type NetControlStatus, type NetRule, type PingComparison, type PingReport, type PingTarget,
  } from "../lib/api";
  import { num, rate } from "../lib/format";
  import { t } from "../lib/i18n.svelte";
  import LineChart from "../lib/LineChart.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";

  type Tab = "ping" | "control";
  const hashTab = location.hash.split("/")[1] as Tab;
  let tab = $state<Tab>(hashTab === "control" ? "control" : "ping");
  let report = $state<PingReport | null>(null);
  let comparison = $state<PingComparison | null>(null);
  let apps = $state<AppTraffic[]>([]);
  let rules = $state<NetRule[]>([]);
  let status = $state<NetControlStatus | null>(null);
  let error = $state<string | null>(null);
  let showAll = $state(false);
  let busy = $state(false);
  // The app whose limit form is open, with its choices.
  let editing = $state<{ app: string; up: number; down: number; when: "always" | "gaming" } | null>(null);

  // Colours checked with the dataviz validator (see LineChart).
  const FIRST = "#8f6af2";
  const SECOND = "#c98500";
  const WINDOW_MS = 5 * 60_000;

  // Asking keeps the agent measuring; it stops a minute after the page closes.
  $effect(() => {
    if (tab !== "ping") return;
    const tick = () => {
      if (document.hidden) return;
      api.pingRecent(Date.now() - WINDOW_MS)
        .then((r) => { report = r; error = null; })
        .catch((e) => (error = errorText(e)));
    };
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  });

  // Apps every 2 s (the service reads counters that often while asked);
  // rules and status with them, cheap.
  $effect(() => {
    const tick = () => {
      if (document.hidden) return;
      Promise.all([api.netApps(), api.netRules(), api.netStatus()])
        .then(([a, r, s]) => { apps = a; rules = r; status = s; })
        .catch((e) => (error = errorText(e)));
    };
    tick();
    const id = setInterval(tick, 2000);
    return () => clearInterval(id);
  });

  const lastMark = $derived(report?.marks.filter((m) => m.kind === "gamemode-on").at(-1) ?? null);
  const game = $derived(report?.targets.find((x) => x.id === "game" && x.addr) ?? null);
  // The chart compares the router with the far end: the game's server while
  // one is known, else the DNS server.
  const near = $derived(report?.targets.find((x) => x.id === "gateway") ?? null);
  const far = $derived(game ?? report?.targets.find((x) => x.id === "internet") ?? null);

  $effect(() => {
    const m = lastMark;
    const target = far?.id;
    if (!m || !target) {
      comparison = null;
      return;
    }
    api.pingCompare(target, m.ts_unix_ms, 60).then((c) => (comparison = c)).catch(() => {});
  });

  // One time axis: whole seconds, since every target is pinged in the same round.
  const chart = $derived.by(() => {
    const secs = new Map<number, [number | null, number | null]>();
    const put = (tg: PingTarget | null, i: 0 | 1) => {
      for (const s of tg?.samples ?? []) {
        const k = Math.round(s.ts_unix_ms / 1000);
        const row = secs.get(k) ?? [null, null];
        row[i] = s.rtt_ms;
        secs.set(k, row);
      }
    };
    put(near, 0);
    put(far, 1);
    const keys = [...secs.keys()].sort((a, b) => a - b);
    return { times: keys, a: keys.map((k) => secs.get(k)![0]), b: keys.map((k) => secs.get(k)![1]) };
  });

  function targetName(tg: PingTarget): string {
    return tg.id === "game" ? `${t().ping.targets.game}: ${tg.label}` : t().ping.targets[tg.id];
  }

  const ruleFor = (name: string) => rules.find((r) => r.app.toLowerCase() === name.toLowerCase()) ?? null;
  const background = $derived(
    apps.filter((a) => !a.has_game && a.kind !== "session" && a.category !== "vpn" && (a.state !== "" || (a.category && a.rx_bps + a.tx_bps >= 50_000) || a.tx_bps >= 125_000)),
  );
  // A VPN carries the other apps' traffic: it gets one line of its own.
  const vpnApps = $derived(apps.filter((a) => a.category === "vpn"));
  const others = $derived(apps.filter((a) => a.category !== "vpn"));
  const listed = $derived(showAll ? others : others.filter((a) => a.rx_bytes + a.tx_bytes > 0 || a.state !== ""));
  const maxRate = $derived(Math.max(1, ...background.map((a) => a.rx_bps + a.tx_bps)));

  async function act(f: () => Promise<unknown>) {
    busy = true;
    try {
      await f();
      [apps, rules, status] = await Promise.all([api.netApps(), api.netRules(), api.netStatus()]);
      error = null;
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
  const block = (a: AppTraffic) => act(() => api.netSetRule(a.name, true, 0, 0, "always"));
  const allow = (a: AppTraffic) => act(() => api.netRemoveRule(a.name));
  function openLimit(a: AppTraffic) {
    const r = ruleFor(a.name);
    editing = { app: a.name, up: r?.up_kbps || 1000, down: r?.down_kbps || 8000, when: r?.when ?? "gaming" };
  }
  async function saveLimit() {
    const e = editing;
    if (!e) return;
    await act(() => api.netSetRule(e.app, false, e.up, e.down, e.when));
    if (!error) editing = null;
  }

  const UP = [0, 256, 1000, 4000];
  const DOWN = [0, 2000, 8000, 32000];
  const kb = (k: number) => (k === 0 ? "—" : k >= 1000 ? `${num(k / 1000, k % 1000 ? 1 : 0)} Mbit/s` : `${k} kbit/s`);
  const ms = (v: number | null) => (v == null ? "—" : num(v, v < 10 ? 1 : 0));
</script>

<header class="head">
  <h1>{t().ping.title}</h1>
  {#if api.preview}<span class="badge">{t().preview}</span>{/if}
</header>

<div class="tabs" role="tablist">
  {#each ["ping", "control"] as const as id}
    <button role="tab" aria-selected={tab === id} class:active={tab === id} onclick={() => (tab = id)}>{t().ping.tabs[id]}</button>
  {/each}
</div>

{#if error}<Problem text={error} />{/if}
{#if status?.simulated}<p class="note">{t().ping.simulated}</p>{/if}
{#if status && !status.available}<p class="problem">{t().ping.unavailable} {status.problem}</p>{/if}
{#each status?.shaping_problems ?? [] as p}<p class="problem">{p}</p>{/each}

{#snippet vpnLine()}
  {#each vpnApps as v (v.id)}
    {@const d = rate(v.rx_bps)}
    {@const u = rate(v.tx_bps)}
    <p class="vpnline">
      <b dir="auto">{v.name}</b>
      <span dir="ltr">↓ {d.value} {d.unit} · ↑ {u.value} {u.unit}</span>
      {status?.vpn_tun ? t().ping.vpnTun : t().ping.vpnProxy}
    </p>
  {/each}
{/snippet}

{#snippet appActions(a: AppTraffic)}
  {#if a.state !== ""}
    <span class="state {a.state}">{a.state === "blocked" ? t().ping.blocked : t().ping.limited}</span>
    <button class="btn" disabled={busy} onclick={() => allow(a)}>{t().ping.allow}</button>
  {:else}
    <button class="btn" disabled={busy} onclick={() => openLimit(a)}><Gauge size={15} aria-hidden="true" /> {t().ping.limit}</button>
    <button class="btn danger" disabled={busy} onclick={() => block(a)}><Ban size={15} aria-hidden="true" /> {t().ping.block}</button>
  {/if}
{/snippet}

{#snippet limitForm()}
  {#if editing}
    <div class="form">
      <div class="field">
        <span>{t().ping.upCap}</span>
        <Segmented label={t().ping.upCap} value={String(editing.up)} options={UP.map((k) => ({ value: String(k), label: kb(k) }))} onchange={(v) => editing && (editing.up = Number(v))} />
      </div>
      <div class="field">
        <span>{t().ping.downCap}</span>
        <Segmented label={t().ping.downCap} value={String(editing.down)} options={DOWN.map((k) => ({ value: String(k), label: kb(k) }))} onchange={(v) => editing && (editing.down = Number(v))} />
      </div>
      <div class="field">
        <Segmented label={t().ping.gaming} value={editing.when} options={[{ value: "gaming", label: t().ping.gaming }, { value: "always", label: t().ping.always }]} onchange={(v) => editing && (editing.when = v)} />
      </div>
      <div class="field buttons">
        <button class="btn primary" disabled={busy || (editing.up === 0 && editing.down === 0)} onclick={saveLimit}>{t().ping.save}</button>
        <button class="btn" onclick={() => (editing = null)}>{t().ping.cancel}</button>
      </div>
    </div>
  {/if}
{/snippet}

{#if tab === "ping"}
  <div class="stats">
    {#each report?.targets ?? [] as tg (tg.id)}
      <section class="card stat">
        <span class="label">{targetName(tg)} <span class="addr" dir="ltr">{tg.addr}</span></span>
        {#if tg.via}<span class="route" class:vpn={tg.through_vpn}>{tg.through_vpn ? t().ping.throughVpn : t().ping.direct} <span dir="ltr">{tg.via}</span></span>{/if}
        {#if tg.problem}
          <p class="muted">{tg.problem}</p>
        {:else}
          <b>{ms(tg.stats.median_ms)} <small>{t().ping.ms}</small></b>
          <span class="sub">
            {t().ping.jitter} <b dir="ltr">{ms(tg.stats.jitter_ms)}</b> ·
            {t().ping.p95} <b dir="ltr">{ms(tg.stats.p95_ms)}</b> ·
            {t().ping.loss} <b dir="ltr">{num(tg.stats.loss_pct)}٪</b>
          </span>
        {/if}
      </section>
    {/each}
  </div>

  <section class="card">
    <h2>{t().ping.chart} <small class="muted">{t().ping.ms}</small></h2>
    <LineChart
      times={chart.times}
      series={[
        ...(near ? [{ label: t().ping.targets.gateway, color: FIRST, values: chart.a }] : []),
        ...(far ? [{ label: far.id === "game" ? far.label : t().ping.targets.internet, color: SECOND, values: chart.b }] : []),
      ]}
      waiting={t().ping.waiting}
      unit=""
      range={[0, 50]}
      height={220}
    />
  </section>

  <section class="card">
    <h2>{t().ping.compare}</h2>
    {#if comparison}
      <div class="compare">
        {#each [["before", comparison.before], ["after", comparison.after]] as const as [k, s]}
          <div>
            <span class="label">{t().ping[k]}</span>
            <span dir="ltr">{t().ping.median} {ms(s.median_ms)} · {t().ping.jitter} {ms(s.jitter_ms)} · {t().ping.p95} {ms(s.p95_ms)}</span>
          </div>
        {/each}
      </div>
      <p class="verdict {comparison.verdict}">{t().ping.verdict[comparison.verdict]}</p>
    {:else}
      <p class="muted">{t().ping.noMark}</p>
    {/if}
  </section>

  <section class="card">
    <h2>{t().ping.background}</h2>
    {#if background.length === 0}
      <p class="muted">{t().ping.backgroundEmpty}</p>
      {@render vpnLine()}
    {:else}
      <ul class="list">
        {#each background as a (a.id)}
          {@const d = rate(a.rx_bps)}
          {@const u = rate(a.tx_bps)}
          <li>
            <div class="who">
              <b dir="auto">{a.name}</b>
              {#if a.category}<span class="chip">{t().ping.categories[a.category] ?? a.category}</span>{/if}
              <span class="bar"><span style:--w={(a.rx_bps + a.tx_bps) / maxRate}></span></span>
              <span class="rates" dir="ltr">↓ {d.value} {d.unit} · ↑ {u.value} {u.unit}</span>
            </div>
            <div class="actions">{@render appActions(a)}</div>
          </li>
          {#if editing?.app === a.name}<li class="editing">{@render limitForm()}</li>{/if}
        {/each}
      </ul>
      {@render vpnLine()}
    {/if}
  </section>
{:else}
  <section class="card">
    <div class="row-head">
      <h2>{t().ping.apps}</h2>
      <label class="check"><input type="checkbox" bind:checked={showAll} /> {t().ping.showAll}</label>
    </div>
    <table>
      <thead>
        <tr><th>{t().ping.apps}</th><th>{t().ping.down}</th><th>{t().ping.up}</th><th>{t().ping.total}</th><th></th></tr>
      </thead>
      <tbody>
        {#each listed as a (a.id)}
          {@const d = rate(a.rx_bps)}
          {@const u = rate(a.tx_bps)}
          <tr>
            <td>
              <b dir="auto">{a.name}</b>
              <span class="kind">{t().ping.kinds[a.kind] ?? a.kind}</span>
              {#if a.category}<span class="chip">{t().ping.categories[a.category] ?? a.category}</span>{/if}
              {#if a.has_game}<span class="warn" title={t().ping.hasGame}>⚠</span>{/if}
            </td>
            <td dir="ltr">{d.value} {d.unit}</td>
            <td dir="ltr">{u.value} {u.unit}</td>
            <td dir="ltr">{num((a.rx_bytes + a.tx_bytes) / 1e6, 1)} MB</td>
            <td class="actions">{@render appActions(a)}</td>
          </tr>
          {#if editing?.app === a.name}<tr><td colspan="5">{@render limitForm()}</td></tr>{/if}
        {/each}
      </tbody>
    </table>
    {@render vpnLine()}
  </section>

  <section class="card">
    <h2><ShieldCheck size={17} aria-hidden="true" /> {t().ping.rules}</h2>
    {#if rules.length === 0}
      <p class="muted">{t().ping.noRules}</p>
    {:else}
      <ul class="list">
        {#each rules as r (r.app)}
          <li>
            <div class="who">
              <b dir="auto">{r.app}</b>
              <span dir="ltr">{r.block ? t().ping.blocked : `↑ ${kb(r.up_kbps)} · ↓ ${kb(r.down_kbps)}`}</span>
              <span class="muted">{r.when === "gaming" ? t().ping.gaming : t().ping.always}</span>
              <span class="state {r.active ? 'on' : ''}">{r.active ? t().ping.active : t().ping.waitingGame}</span>
            </div>
            <div class="actions"><button class="btn" disabled={busy} onclick={() => act(() => api.netRemoveRule(r.app))}>{t().ping.remove}</button></div>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<style>
  .head { display: flex; align-items: center; gap: 14px; margin-bottom: 16px; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 800; }
  h2 { margin: 0 0 10px; font-size: 0.98rem; font-weight: 700; display: flex; align-items: center; gap: 8px; }
  h2 small { font-weight: 500; font-size: 0.8rem; }
  .badge { padding: 3px 12px; border-radius: 999px; font-size: 0.8rem; background: var(--card-2); color: var(--text-2); }
  .tabs { display: flex; gap: 4px; border-bottom: 1px solid var(--line); margin-bottom: 16px; }
  .tabs button { all: unset; padding: 9px 14px; font-size: 0.9rem; color: var(--text-2); cursor: pointer; border-bottom: 2px solid transparent; margin-bottom: -1px; }
  .tabs button:hover { color: var(--text); }
  .tabs button.active { color: var(--text); border-bottom-color: var(--accent); font-weight: 600; }
  .tabs button:focus-visible { outline: 2px solid var(--accent-text); outline-offset: -2px; border-radius: 6px; }
  .problem { margin: 0 0 14px; padding: 12px 16px; border-radius: var(--radius-sm); background: color-mix(in oklab, var(--danger) 12%, var(--card)); color: var(--danger); font-size: 0.9rem; }
  .note { margin: 0 0 14px; padding: 10px 16px; border-radius: var(--radius-sm); background: var(--card-2); color: var(--text-2); font-size: 0.88rem; }
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; margin-bottom: 16px; min-width: 0; }
  .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 16px; margin-bottom: 16px; }
  .stats .card { margin: 0; }
  .stat { display: grid; gap: 6px; }
  .stat .label { color: var(--text-2); font-size: 0.85rem; }
  .addr { color: var(--text-3); font-size: 0.78rem; margin-inline-start: 6px; }
  .route { font-size: 0.75rem; color: var(--text-3); }
  .route.vpn { color: var(--accent-text); }
  .vpnline { margin: 12px 0 0; font-size: 0.82rem; color: var(--text-2); display: flex; gap: 10px; flex-wrap: wrap; align-items: baseline; }
  .stat > b { font-size: 1.7rem; font-weight: 800; }
  .stat small { font-size: 0.8rem; font-weight: 500; color: var(--text-2); }
  .sub { color: var(--text-2); font-size: 0.82rem; }
  .sub b { color: var(--text); font-weight: 600; }
  .muted { color: var(--text-2); margin: 0; font-size: 0.9rem; }
  .small { color: var(--text-3); font-size: 0.8rem; margin: 10px 0 0; }
  .compare { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 10px 24px; font-size: 0.9rem; }
  .compare .label { display: block; color: var(--text-2); font-size: 0.8rem; margin-bottom: 2px; }
  .verdict { margin: 12px 0 0; font-weight: 600; }
  .verdict.flatter { color: var(--ok, #34d399); }
  .verdict.worse { color: var(--danger); }
  .list { list-style: none; margin: 0; padding: 0; display: grid; gap: 4px; }
  .list li { display: flex; align-items: center; justify-content: space-between; gap: 14px; padding: 10px 0; border-bottom: 1px solid var(--line); flex-wrap: wrap; }
  .list li:last-child { border-bottom: 0; }
  .list li.editing { display: block; }
  .who { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; min-width: 0; }
  .chip { padding: 2px 9px; border-radius: 999px; background: var(--card-2); color: var(--text-2); font-size: 0.75rem; }
  .kind { color: var(--text-3); font-size: 0.75rem; margin-inline-start: 6px; }
  .bar { width: 120px; height: 6px; border-radius: 3px; background: var(--card-2); overflow: hidden; }
  .bar span { display: block; height: 100%; width: calc(var(--w) * 100%); background: #8f6af2; }
  .rates { color: var(--text-2); font-size: 0.82rem; }
  .actions { display: flex; gap: 8px; align-items: center; justify-content: flex-end; }
  .btn { all: unset; display: inline-flex; align-items: center; gap: 6px; padding: 6px 12px; border-radius: 9px; background: var(--card-2); color: var(--text); font-size: 0.85rem; cursor: pointer; }
  .btn:hover:not(:disabled) { background: color-mix(in oklab, var(--card-2) 80%, var(--text) 8%); }
  .btn.primary { background: var(--accent); color: white; font-weight: 600; }
  .btn.danger { color: var(--danger); }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .state { padding: 2px 9px; border-radius: 999px; font-size: 0.75rem; background: var(--card-2); color: var(--text-2); }
  .state.blocked { color: var(--danger); }
  .state.limited, .state.on { color: var(--accent-text); }
  .warn { color: var(--warn, #fbbf24); margin-inline-start: 6px; cursor: help; }
  .form { display: flex; flex-wrap: wrap; gap: 12px 20px; align-items: flex-end; padding: 12px; border-radius: var(--radius-sm); background: color-mix(in oklab, var(--card-2) 50%, var(--card)); }
  .field { display: grid; gap: 6px; font-size: 0.82rem; color: var(--text-2); }
  .field.buttons { display: flex; gap: 8px; }
  .row-head { display: flex; justify-content: space-between; align-items: center; gap: 12px; }
  .check { font-size: 0.85rem; color: var(--text-2); display: flex; gap: 6px; align-items: center; }
  table { width: 100%; border-collapse: collapse; font-size: 0.88rem; }
  th { text-align: start; color: var(--text-3); font-weight: 500; font-size: 0.78rem; padding: 6px 8px; border-bottom: 1px solid var(--line); }
  td { padding: 8px; border-bottom: 1px solid var(--line); vertical-align: middle; }
  tbody tr:last-child td { border-bottom: 0; }
</style>
