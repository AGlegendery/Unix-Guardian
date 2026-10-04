<script lang="ts">
  import { ChevronLeft, ChevronRight } from "@lucide/svelte";
  import { api, errorText, type DayTraffic, type TrafficBucket } from "./api";
  import { num } from "./format";
  import { locale, t } from "./i18n.svelte";

  // How much the internet was used: the last hour, today, 7 and 30 days,
  // then any day of the last 30 hour by hour. From the agent's minute
  // history of the interface counters, so the totals are exact.
  const DOWN = "#8f6af2";
  const UP = "#c98500";
  const HOUR = 3_600_000;

  let days = $state<DayTraffic[]>([]);
  let lastHour = $state<TrafficBucket | null>(null);
  let today = $state<TrafficBucket | null>(null);
  let picked = $state<string | null>(null);
  let hours = $state<TrafficBucket[]>([]);
  let error = $state<string | null>(null);

  function midnight(d = new Date()): Date {
    const m = new Date(d);
    m.setHours(0, 0, 0, 0);
    return m;
  }
  function isoDay(d: Date): string {
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  }
  function dayStart(date: string): number {
    const [y, m, d] = date.split("-").map(Number);
    return new Date(y, m - 1, d).getTime();
  }

  // Changes once a minute: reload every minute while visible.
  $effect(() => {
    const load = () => {
      if (document.hidden) return;
      const now = Date.now();
      const start = midnight().getTime();
      Promise.all([
        api.traffic(midnight(new Date(now - 29 * 86_400_000)).getTime()),
        api.trafficBuckets(now - HOUR, now, HOUR),
        api.trafficBuckets(start, now, Math.max(60_000, now - start)),
      ])
        .then(([d, h, td]) => {
          days = d;
          lastHour = h[0] ?? null;
          today = td[0] ?? null;
          error = null;
        })
        .catch((e) => (error = errorText(e)));
    };
    load();
    const id = setInterval(load, 60_000);
    document.addEventListener("visibilitychange", load);
    return () => {
      clearInterval(id);
      document.removeEventListener("visibilitychange", load);
    };
  });

  // Thirty days ending today, with the days nothing was recorded as zero.
  const month = $derived.by(() => {
    const byDate = new Map(days.map((d) => [d.date, d]));
    const base = midnight();
    return Array.from({ length: 30 }, (_, i) => {
      const date = isoDay(new Date(base.getFullYear(), base.getMonth(), base.getDate() - 29 + i));
      return byDate.get(date) ?? { date, rx_bytes: 0, tx_bytes: 0 };
    });
  });
  const selected = $derived(picked ?? isoDay(new Date()));

  $effect(() => {
    const start = dayStart(selected);
    const end = new Date(start);
    end.setDate(end.getDate() + 1);
    api.trafficBuckets(start, end.getTime(), HOUR).then((h) => (hours = h)).catch((e) => (error = errorText(e)));
  });

  const sum = (list: { rx_bytes: number; tx_bytes: number }[]) =>
    list.reduce((a, d) => ({ rx: a.rx + d.rx_bytes, tx: a.tx + d.tx_bytes }), { rx: 0, tx: 0 });
  const tiles = $derived([
    { label: t().ping.usage.hour, ...sum(lastHour ? [lastHour] : []) },
    { label: t().ping.usage.today, ...sum(today ? [today] : []) },
    { label: t().ping.usage.week, ...sum(month.slice(-7)) },
    { label: t().ping.usage.month, ...sum(month) },
  ]);
  const maxDay = $derived(Math.max(1, ...month.map((d) => d.rx_bytes + d.tx_bytes)));
  const maxHour = $derived(Math.max(1, ...hours.map((h) => h.rx_bytes + h.tx_bytes)));
  const dayTotal = $derived(sum(hours));
  const peak = $derived(hours.reduce<TrafficBucket | null>((p, h) => (h.rx_bytes + h.tx_bytes > (p ? p.rx_bytes + p.tx_bytes : 0) ? h : p), null));
  const index = $derived(month.findIndex((d) => d.date === selected));

  function size(b: number): string {
    return b >= 1e9 ? `${num(b / 1e9, 2)} ${t().mon.gb}` : b >= 1e6 ? `${num(b / 1e6, b >= 1e8 ? 0 : 1)} ${t().mon.mb}` : `${num(b / 1e3)} KB`;
  }
  const fmtLocale = () => (locale.current === "fa" ? "fa-IR" : "en-GB");
  function longDay(date: string): string {
    return new Intl.DateTimeFormat(fmtLocale(), { weekday: "long", day: "numeric", month: "long" }).format(new Date(dayStart(date)));
  }
  function shortDay(date: string): string {
    return new Intl.DateTimeFormat(fmtLocale(), { day: "numeric" }).format(new Date(dayStart(date)));
  }
  function hourName(ms: number): string {
    return new Intl.DateTimeFormat(fmtLocale(), { hour: "2-digit", minute: "2-digit", hour12: false }).format(new Date(ms));
  }
</script>

{#if error}<p class="problem">{error}</p>{/if}

<section class="card">
  <h2>{t().ping.usage.title}</h2>
  <div class="tiles">
    {#each tiles as tile}
      <div class="tile">
        <span class="label">{tile.label}</span>
        <b>{size(tile.rx + tile.tx)}</b>
        <span class="sub">↓ <bdi>{size(tile.rx)}</bdi> · ↑ <bdi>{size(tile.tx)}</bdi></span>
      </div>
    {/each}
  </div>
  <p class="small">{t().ping.usage.note}</p>
</section>

<section class="card">
  <div class="row-head">
    <h2>{t().ping.usage.days}</h2>
    <ul class="legend">
      <li><span class="key" style:background={DOWN}></span>{t().mon.down}</li>
      <li><span class="key" style:background={UP}></span>{t().mon.up}</li>
    </ul>
  </div>
  <div class="days" role="listbox" aria-label={t().ping.usage.days}>
    {#each month as d (d.date)}
      <button
        class="day"
        class:on={d.date === selected}
        role="option"
        aria-selected={d.date === selected}
        title={`${longDay(d.date)}: ${size(d.rx_bytes + d.tx_bytes)}`}
        onclick={() => (picked = d.date)}
      >
        <span class="stack">
          <span class="part up" style:--h={d.tx_bytes / maxDay}></span>
          <span class="part down" style:--h={d.rx_bytes / maxDay}></span>
        </span>
        <span class="num">{shortDay(d.date)}</span>
      </button>
    {/each}
  </div>

  <div class="detail">
    <div class="detail-head">
      <button class="nav" disabled={index <= 0} onclick={() => (picked = month[index - 1].date)} aria-label={t().ping.usage.prev}>
        {#if locale.current === "fa"}<ChevronRight size={16} />{:else}<ChevronLeft size={16} />{/if}
      </button>
      <div>
        <b>{longDay(selected)}</b>
        <span class="sub">
          <bdi>{size(dayTotal.rx + dayTotal.tx)}</bdi> · ↓ <bdi>{size(dayTotal.rx)}</bdi> · ↑ <bdi>{size(dayTotal.tx)}</bdi>
          {#if peak && peak.rx_bytes + peak.tx_bytes > 0} · {t().ping.usage.peak} {hourName(peak.start_unix_ms)}{/if}
        </span>
      </div>
      <button class="nav" disabled={index < 0 || index >= month.length - 1} onclick={() => (picked = month[index + 1].date)} aria-label={t().ping.usage.next}>
        {#if locale.current === "fa"}<ChevronLeft size={16} />{:else}<ChevronRight size={16} />{/if}
      </button>
    </div>
    {#if dayTotal.rx + dayTotal.tx === 0}
      <p class="muted">{t().ping.usage.nothing}</p>
    {:else}
      <div class="hours">
        {#each hours as h, i (h.start_unix_ms)}
          <div class="hour" title={`${hourName(h.start_unix_ms)}: ↓ ${size(h.rx_bytes)} · ↑ ${size(h.tx_bytes)}`}>
            <span class="stack">
              <span class="part up" style:--h={h.tx_bytes / maxHour}></span>
              <span class="part down" style:--h={h.rx_bytes / maxHour}></span>
            </span>
            <span class="num">{i % 3 === 0 ? num(new Date(h.start_unix_ms).getHours()) : ""}</span>
          </div>
        {/each}
      </div>
    {/if}
  </div>
</section>

<style>
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; margin-bottom: 16px; min-width: 0; }
  h2 { margin: 0 0 12px; font-size: 0.98rem; font-weight: 700; }
  .problem { margin: 0 0 14px; padding: 12px 16px; border-radius: var(--radius-sm); background: color-mix(in oklab, var(--danger) 12%, var(--card)); color: var(--danger); font-size: 0.9rem; }
  .tiles { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 12px; }
  @media (max-width: 900px) { .tiles { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
  .tile { display: grid; gap: 4px; padding: 12px 14px; border-radius: var(--radius-sm); background: var(--card-2); }
  .tile .label { color: var(--text-2); font-size: 0.82rem; }
  .tile b { font-size: 1.35rem; font-weight: 800; }
  .sub { color: var(--text-2); font-size: 0.8rem; }
  .small { color: var(--text-3); font-size: 0.8rem; margin: 10px 0 0; }
  .muted { color: var(--text-2); margin: 8px 0 0; font-size: 0.9rem; }
  .row-head { display: flex; justify-content: space-between; align-items: center; gap: 12px; }
  .legend { list-style: none; margin: 0 0 12px; padding: 0; display: flex; gap: 14px; font-size: 0.8rem; color: var(--text-2); }
  .legend li { display: flex; align-items: center; gap: 6px; }
  .key { width: 11px; height: 11px; border-radius: 3px; }
  .days { display: flex; gap: 3px; align-items: stretch; height: 120px; }
  .day { all: unset; flex: 1; min-width: 0; display: flex; flex-direction: column; justify-content: flex-end; align-items: center; gap: 4px; padding: 4px 0 2px; border-radius: 6px; cursor: pointer; }
  .day:hover { background: var(--card-2); }
  .day.on { background: color-mix(in oklab, var(--accent) 22%, var(--card)); }
  .day:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  .stack { flex: 1; width: 70%; max-width: 18px; display: flex; flex-direction: column; justify-content: flex-end; gap: 1px; }
  .part { display: block; height: calc(var(--h) * 100%); min-height: 0; }
  .part.up { background: #c98500; border-radius: 3px 3px 0 0; }
  .part.down { background: #8f6af2; }
  .num { font-size: 0.66rem; color: var(--text-3); height: 1em; }
  .detail { margin-top: 16px; padding-top: 14px; border-top: 1px solid var(--line); }
  .detail-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 12px; }
  .detail-head > div { display: grid; gap: 2px; text-align: center; }
  .nav { all: unset; display: inline-flex; padding: 6px; border-radius: 8px; background: var(--card-2); cursor: pointer; }
  .nav:disabled { opacity: 0.4; cursor: default; }
  .nav:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .hours { display: flex; gap: 4px; height: 150px; align-items: stretch; }
  .hour { flex: 1; min-width: 0; display: flex; flex-direction: column; justify-content: flex-end; align-items: center; gap: 4px; }
  .hour .stack { width: 80%; max-width: 22px; }
</style>
