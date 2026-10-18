<script lang="ts">
  // Monitoring → Ping stability: the router, the DNS server and the running
  // game's server, the last five minutes, before/after game mode, and the
  // background tasks using the internet (with Limit and Block).
  import { api, errorText, type PingComparison, type PingReport, type PingTarget } from "./api";
  import AppRuleActions from "./AppRuleActions.svelte";
  import { num, rate } from "./format";
  import { t } from "./i18n.svelte";
  import LineChart from "./LineChart.svelte";
  import type { NetControl } from "./netcontrol.svelte";
  import Problem from "./ui/Problem.svelte";
  import Segmented from "./ui/Segmented.svelte";
  import ModeSwitch from "./ui/ModeSwitch.svelte";

  let { ctl }: { ctl: NetControl } = $props();
  let report = $state<PingReport | null>(null);
  let comparison = $state<PingComparison | null>(null);
  let error = $state<string | null>(null);

  // Colours checked with the dataviz validator (see LineChart).
  const FIRST = "#8f6af2";
  const SECOND = "#c98500";
  const WINDOW_MS = 5 * 60_000;

  // Asking keeps the agent measuring; it stops a minute after the tab closes.
  $effect(() => {
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

  const background = $derived(
    ctl.apps.filter((a) => !a.has_game && a.kind !== "session" && a.category !== "vpn" && ((a.state !== "" && a.state !== "background") || (a.category && a.rx_bps + a.tx_bps >= 50_000) || a.tx_bps >= 125_000)),
  );
  const vpnApps = $derived(ctl.apps.filter((a) => a.category === "vpn"));
  const maxRate = $derived(Math.max(1, ...background.map((a) => a.rx_bps + a.tx_bps)));
  // "Slow down background apps".
  $effect(() => {
    ctl.loadBackground();
  });
  const UP = [256, 512, 1000, 2000];
  const DOWN = [2000, 4000, 8000, 16000];
  const kb = (k: number) => (k >= 1000 ? `${num(k / 1000, k % 1000 ? 1 : 0)} Mbit/s` : `${num(k)} kbit/s`);
  const slowed = $derived(ctl.apps.filter((a) => a.state === "background"));
  const spared = $derived(ctl.apps.filter((a) => a.state === "" && (a.has_game || a.category === "vpn" || a.category === "dns")));
  function setBg(patch: Partial<{ on: boolean; up: number; down: number; when: "always" | "gaming" }>) {
    const b = ctl.background;
    if (!b) return;
    const next = { on: b[0], up: b[1], down: b[2], when: b[3], ...patch };
    ctl.setBackground(next.on, next.up, next.down, next.when);
  }
  const ms = (v: number | null) => (v == null ? "—" : num(v, v < 10 ? 1 : 0));
</script>

{#if error}<Problem text={error} />{/if}

<!-- The most useful thing for ping while playing: every app that is not
     the game, a VPN or a DNS manager shares one small cap. -->
<section class="card bg mode-card" data-cat="net" class:is-on={ctl.background?.[0]}>
  <div class="bg-head">
    <div>
      <h2>{t().ping.bg.title}</h2>
      <p class="muted">{t().ping.bg.explain}</p>
    </div>
    {#if ctl.background}
      <ModeSwitch
        checked={ctl.background[0]}
        waiting={ctl.background[0] && ctl.background[3] === "gaming" && slowed.length === 0}
        disabled={ctl.busy || !ctl.status?.available}
        label={t().ping.bg.title}
        onchange={(v) => setBg({ on: v })}
      />
    {/if}
  </div>
  {#if !ctl.background}
    <p class="muted">{t().ping.bg.oldService}</p>
  {:else}
    {#if !ctl.background[0]}<p class="mode-off">{t().modeOff}</p>{/if}
    <div class="bg-fields depends">
      <div class="field">
        <span>{t().ping.upCap}</span>
        <Segmented label={t().ping.upCap} value={String(ctl.background[1])} options={UP.map((k) => ({ value: String(k), label: kb(k) }))} onchange={(v) => setBg({ up: Number(v) })} />
      </div>
      <div class="field">
        <span>{t().ping.downCap}</span>
        <Segmented label={t().ping.downCap} value={String(ctl.background[2])} options={DOWN.map((k) => ({ value: String(k), label: kb(k) }))} onchange={(v) => setBg({ down: Number(v) })} />
      </div>
      <div class="field">
        <span>{t().ping.bg.when}</span>
        <Segmented label={t().ping.bg.when} value={ctl.background[3]} options={[{ value: "always", label: t().ping.bg.always }, { value: "gaming", label: t().ping.gaming }]} onchange={(v) => setBg({ when: v })} />
      </div>
    </div>
    <p class="small">{t().ping.bg.shared}</p>
    {#if ctl.background[0]}
      {#if slowed.length}
        <p class="who-line"><b>{t().ping.bg.slowedNow(slowed.length)}</b> {#each slowed.slice(0, 12) as a (a.id)}<span class="chip" dir="auto">{a.name}</span>{/each}{#if slowed.length > 12}<span class="chip">+{num(slowed.length - 12)}</span>{/if}</p>
      {:else if ctl.background[3] === "gaming"}
        <p class="small">{t().ping.bg.waiting}</p>
      {/if}
      {#if spared.length}
        <p class="who-line"><b>{t().ping.bg.spared}</b> {#each spared as a (a.id)}<span class="chip spared" dir="auto">{a.name}</span>{/each}</p>
      {/if}
    {/if}
    {#if !ctl.status?.available}<p class="small">{t().ping.unavailable} {ctl.status?.problem ?? ""}</p>{/if}
  {/if}
</section>
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
    {#each vpnApps as v (v.id)}
    {@const d = rate(v.rx_bps)}
    {@const u = rate(v.tx_bps)}
    <p class="vpnline"><b dir="auto">{v.name}</b> <span dir="ltr">↓ {d.value} {d.unit} · ↑ {u.value} {u.unit}</span> {ctl.status?.vpn_tun ? t().ping.vpnTun : t().ping.vpnProxy}</p>
  {/each}
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
          <AppRuleActions {ctl} name={a.name} />
        </li>
        {#if ctl.editing?.app === a.name}<li class="editing"><AppRuleActions {ctl} name={a.name} part="form" /></li>{/if}
      {/each}
    </ul>
    {#each vpnApps as v (v.id)}
    {@const d = rate(v.rx_bps)}
    {@const u = rate(v.tx_bps)}
    <p class="vpnline"><b dir="auto">{v.name}</b> <span dir="ltr">↓ {d.value} {d.unit} · ↑ {u.value} {u.unit}</span> {ctl.status?.vpn_tun ? t().ping.vpnTun : t().ping.vpnProxy}</p>
  {/each}
  {/if}
</section>
<style>
  h2 { margin: 0 0 10px; font-size: 0.98rem; font-weight: 700; display: flex; align-items: center; gap: 8px; }
  h2 small { font-weight: 500; font-size: 0.8rem; }
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
  .bar { width: 120px; height: 6px; border-radius: 3px; background: var(--card-2); overflow: hidden; }
  .bar span { display: block; height: 100%; width: calc(var(--w) * 100%); background: var(--c-net); }
  .rates { color: var(--text-2); font-size: 0.82rem; }
  .small { color: var(--text-3); font-size: 0.8rem; margin: 10px 0 0; }
  .mode-off { margin: 12px 0 0; padding: 7px 12px; border-radius: 10px; font-size: 0.84rem; color: var(--text-2); background: var(--card-2); border-inline-start: 3px solid var(--text-3); }
  .bg-head { display: flex; justify-content: space-between; align-items: flex-start; gap: 16px; }
  .bg-head h2 { margin-bottom: 4px; }
  .bg-fields { display: flex; flex-wrap: wrap; gap: 14px 22px; margin-top: 14px; }
  .field { display: grid; gap: 6px; font-size: 0.82rem; color: var(--text-2); }
  .who-line { margin: 12px 0 0; display: flex; flex-wrap: wrap; gap: 6px; align-items: center; font-size: 0.84rem; }
  .chip.spared { color: var(--ok, #34d399); }
</style>
