<script lang="ts">
  import { ArrowDown, ArrowUp } from "@lucide/svelte";
  import { api, errorText, type DayTraffic, type HistoryPoint, type Snapshot } from "../lib/api";
  import { num, rate } from "../lib/format";
  import { locale, t } from "../lib/i18n.svelte";
  import LineChart from "../lib/LineChart.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";

  type Tab = "fps" | "thermal" | "traffic";
  type Range = "hour" | "day" | "week";
  const hashTab = location.hash.split("/")[1] as Tab;
  let tab = $state<Tab>(["fps", "thermal", "traffic"].includes(hashTab) ? hashTab : "thermal");
  let range = $state<Range>("day");
  let points = $state<HistoryPoint[]>([]);
  let days = $state<DayTraffic[]>([]);
  let live = $state<Snapshot | null>(null);
  let error = $state<string | null>(null);
  let loaded = $state(false);

  // Colours checked with the dataviz validator (see LineChart).
  const CPU = "#8f6af2";
  const GPU = "#c98500";

  function since(r: Range): number {
    if (r === "hour") return Date.now() - 3_600_000;
    if (r === "week") return Date.now() - 7 * 86_400_000;
    const d = new Date();
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  }

  // History changes once a minute: reload on range change and every minute
  // while the page is visible.
  $effect(() => {
    const r = range;
    const load = () => {
      if (document.hidden) return;
      api.history(since(r), Date.now(), r === "hour" ? 60 : 240)
        .then((p) => { points = p; loaded = true; error = null; })
        .catch((e) => (error = errorText(e)));
    };
    load();
    const id = setInterval(load, 60_000);
    return () => clearInterval(id);
  });

  // The traffic tab also shows live speeds: a snapshot every 2 s while open.
  $effect(() => {
    if (tab !== "traffic") return;
    api.traffic(Date.now() - 7 * 86_400_000).then((d) => (days = d)).catch((e) => (error = errorText(e)));
    const tick = () => !document.hidden && api.snapshot().then((s) => (live = s)).catch(() => {});
    tick();
    const id = setInterval(tick, 2000);
    return () => clearInterval(id);
  });

  const times = $derived(points.map((p) => p.ts_unix_ms / 1000));
  const hasGpu = $derived(points.some((p) => p.gpu_temp_c != null));
  function pair(cpu: (p: HistoryPoint) => number | null, gpu: (p: HistoryPoint) => number | null) {
    return [
      { label: t().mon.cpu, color: CPU, values: points.map(cpu) },
      ...(hasGpu ? [{ label: t().mon.gpu, color: GPU, values: points.map(gpu) }] : []),
    ];
  }
  const maxDay = $derived(Math.max(1, ...days.map((d) => d.rx_bytes + d.tx_bytes)));
  function gb(b: number): string {
    return b >= 1e9 ? `${num(b / 1e9, 1)} ${t().mon.gb}` : `${num(b / 1e6, 0)} ${t().mon.mb}`;
  }
  function dayName(date: string): string {
    return new Intl.DateTimeFormat(locale.current === "fa" ? "fa-IR" : "en-GB", { weekday: "short" }).format(new Date(date + "T12:00"));
  }
</script>

<header class="head">
  <h1>{t().mon.title}</h1>
  {#if api.preview}<span class="badge">{t().preview}</span>{/if}
</header>

<div class="tabs" role="tablist">
  {#each ["fps", "thermal", "traffic"] as const as id}
    <button role="tab" aria-selected={tab === id} class:active={tab === id} onclick={() => (tab = id)}>{t().mon.tabs[id]}</button>
  {/each}
</div>

{#if error}<p class="problem" role="alert">{error}</p>{/if}

{#if tab === "fps"}
  <section class="card"><p class="muted">{t().mon.fps}</p></section>
{:else if tab === "thermal"}
  <div class="filters">
    <Segmented
      label={t().mon.tabs.thermal}
      value={range}
      options={(["hour", "day", "week"] as const).map((r) => ({ value: r, label: t().mon.ranges[r] }))}
      onchange={(v) => (range = v)}
    />
  </div>
  {#if loaded && points.length < 2}
    <section class="card"><p class="muted">{t().mon.empty}</p></section>
  {:else}
    <div class="charts">
      <section class="card">
        <h2>{t().mon.temp}</h2>
        <LineChart {times} series={pair((p) => p.cpu_temp_c, (p) => p.gpu_temp_c)} waiting="" mode="history" height={170} timeStyle={range === "week" ? "weekday" : "clock"} />
      </section>
      <section class="card">
        <h2>{t().mon.load}</h2>
        <LineChart {times} series={pair((p) => p.cpu_load_pct, (p) => p.gpu_load_pct)} waiting="" unit="٪" range={[0, 100]} mode="history" height={170} timeStyle={range === "week" ? "weekday" : "clock"} />
      </section>
      <section class="card">
        <h2>{t().mon.power}</h2>
        <LineChart {times} series={pair((p) => p.cpu_w, (p) => p.gpu_w)} waiting="" unit={` ${t().unit.w}`} range={[0, 30]} mode="history" height={170} timeStyle={range === "week" ? "weekday" : "clock"} />
      </section>
    </div>
    <p class="small">{t().mon.note}</p>
  {/if}
{:else}
  <section class="card">
    <h2>{t().mon.now}</h2>
    {#if live}
      {@const d = rate(live.net.rx_bps)}
      {@const u = rate(live.net.tx_bps)}
      <div class="speeds">
        <span><ArrowDown size={18} aria-hidden="true" /> {t().mon.down} <b dir="ltr">{d.value} {d.unit}</b></span>
        <span><ArrowUp size={18} aria-hidden="true" /> {t().mon.up} <b dir="ltr">{u.value} {u.unit}</b></span>
        <span class="muted" dir="ltr">{live.net.iface}</span>
      </div>
    {/if}
  </section>
  <section class="card">
    <h2>{t().mon.perDay}</h2>
    {#if days.length}
      <ul class="legend">
        <li><span class="key" style:background={CPU}></span>{t().mon.down}</li>
        <li><span class="key" style:background={GPU}></span>{t().mon.up}</li>
      </ul>
      <div class="bars">
        {#each days as d (d.date)}
          <div class="bar-col" title={`${d.date}: ${t().mon.down} ${gb(d.rx_bytes)}, ${t().mon.up} ${gb(d.tx_bytes)}`}>
            <span class="bar-val">{gb(d.rx_bytes + d.tx_bytes)}</span>
            <span class="stack">
              <span class="seg up" style:--h={d.tx_bytes / maxDay}></span>
              <span class="seg down" style:--h={d.rx_bytes / maxDay}></span>
            </span>
            <span class="bar-day">{dayName(d.date)}</span>
          </div>
        {/each}
      </div>
    {:else}
      <p class="muted">{t().mon.empty}</p>
    {/if}
    <p class="small">{t().mon.perApp}</p>
  </section>
{/if}

<style>
  .head { display: flex; align-items: center; gap: 14px; margin-bottom: 16px; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 800; }
  h2 { margin: 0 0 10px; font-size: 0.98rem; font-weight: 700; }
  .badge { padding: 3px 12px; border-radius: 999px; font-size: 0.8rem; background: var(--card-2); color: var(--text-2); }
  .tabs { display: flex; gap: 4px; border-bottom: 1px solid var(--line); margin-bottom: 16px; }
  .tabs button {
    all: unset;
    padding: 9px 14px;
    font-size: 0.9rem;
    color: var(--text-2);
    cursor: pointer;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }
  .tabs button:hover { color: var(--text); }
  .tabs button.active { color: var(--text); border-bottom-color: var(--accent); font-weight: 600; }
  .tabs button:focus-visible { outline: 2px solid var(--accent-text); outline-offset: -2px; border-radius: 6px; }
  .problem { margin: 0 0 14px; padding: 12px 16px; border-radius: var(--radius-sm); background: color-mix(in oklab, var(--danger) 12%, var(--card)); color: var(--danger); font-size: 0.9rem; }
  .filters { margin-bottom: 14px; }
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; margin-bottom: 16px; min-width: 0; }
  .charts { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 16px; }
  .charts .card { margin: 0; }
  @media (max-width: 1150px) { .charts { grid-template-columns: 1fr; } }
  .muted { color: var(--text-2); margin: 0; font-size: 0.9rem; }
  .small { color: var(--text-3); font-size: 0.8rem; margin: 10px 0 0; }
  .speeds { display: flex; flex-wrap: wrap; gap: 12px 36px; align-items: center; font-size: 0.95rem; }
  .speeds span { display: inline-flex; align-items: center; gap: 8px; }
  .speeds b { font-size: 1.3rem; }
  .legend { list-style: none; margin: 0 0 8px; padding: 0; display: flex; gap: 16px; font-size: 0.82rem; color: var(--text-2); }
  .legend li { display: flex; align-items: center; gap: 6px; }
  .key { width: 12px; height: 12px; border-radius: 3px; }
  .bars { display: flex; align-items: flex-end; gap: 16px; height: 190px; }
  .bar-col { flex: 1; max-width: 64px; height: 100%; display: flex; flex-direction: column; justify-content: flex-end; align-items: center; gap: 6px; }
  .stack { display: flex; flex-direction: column; gap: 2px; align-items: center; }
  .seg { width: 24px; height: calc(var(--h) * 130px); min-height: 1px; }
  .seg.up { background: #c98500; border-radius: 4px 4px 0 0; }
  .seg.down { background: #8f6af2; }
  .bar-val { font-size: 0.72rem; color: var(--text-2); white-space: nowrap; }
  .bar-day { font-size: 0.75rem; color: var(--text-3); }
</style>
