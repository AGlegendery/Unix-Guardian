<script lang="ts">
  // Monitoring → Traffic: internet use for a day (hours 1–24), a week
  // (Saturday to Friday) or a month (a calendar month: the Persian calendar
  // in Persian), with a volume scale on the left that grows with the use.
  // A day picked on the week or month chart lists that day's apps; each app
  // can be limited or blocked from the list. "Used so far" is the total
  // since the counts were last reset in Settings.
  import { ChevronLeft, ChevronRight } from "@lucide/svelte";
  import HugeIcon from "./icons/HugeIcon.svelte";
  import { api, errorText, type AppUsage } from "./api";
  import AppRuleActions from "./AppRuleActions.svelte";
  import { num, rate } from "./format";
  import { locale, t } from "./i18n.svelte";
  import type { NetControl } from "./netcontrol.svelte";
  import Segmented from "./ui/Segmented.svelte";

  let { ctl }: { ctl: NetControl } = $props();

  type Span = "day" | "week" | "month";
  type Sort = "total" | "down" | "up" | "name";
  type Bar = { label: string; title: string; rx: number; tx: number; day: Date | null };
  const DOWN = "#8f6af2";
  const UP = "#c98500";
  const HOUR = 3_600_000;

  let span = $state<Span>("day");
  let anchor = $state(midnight(new Date())); // any day inside the shown period
  let picked = $state<Date | null>(null); // a day picked on the week or month chart
  let bars = $state<Bar[]>([]);
  let apps = $state<AppUsage[]>([]);
  let sort = $state<Sort>("total");
  let showAll = $state(false);
  let soFar = $state<{ bytes: number; since: number } | null>(null);
  let error = $state<string | null>(null);

  function midnight(d: Date): Date {
    const m = new Date(d);
    m.setHours(0, 0, 0, 0);
    return m;
  }
  const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);
  const isoDay = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  const sameDay = (a: Date | null, b: Date | null) => !!a && !!b && a.getTime() === b.getTime();
  const fmtTag = () => (locale.current === "fa" ? "fa-IR" : "en-GB");
  const fmt = (o: Intl.DateTimeFormatOptions, d: Date | number) => new Intl.DateTimeFormat(fmtTag(), o).format(d);

  // The day of the month and the month, in the calendar the page shows.
  const calendar = () => (locale.current === "fa" ? "persian" : "gregory");
  function calParts(d: Date): { day: number; month: number } {
    const parts = new Intl.DateTimeFormat(`en-US-u-ca-${calendar()}`, { day: "numeric", month: "numeric" }).formatToParts(d);
    const get = (k: string) => Number(parts.find((p) => p.type === k)?.value ?? 0);
    return { day: get("day"), month: get("month") };
  }

  /** The days of the shown period: first day and how many. */
  const period = $derived.by(() => {
    if (span === "day") return { start: anchor, days: 1 };
    if (span === "week") return { start: addDays(anchor, -((anchor.getDay() + 1) % 7)), days: 7 }; // back to Saturday
    const start = addDays(anchor, 1 - calParts(anchor).day);
    const month = calParts(start).month;
    let days = 1;
    while (days < 31 && calParts(addDays(start, days)).month === month) days++;
    return { start, days };
  });
  const end = $derived(addDays(period.start, period.days));
  const today = $derived(midnight(new Date()));
  const isCurrent = $derived(end.getTime() > today.getTime());

  const title = $derived(
    span === "day"
      ? fmt({ weekday: "long", day: "numeric", month: "long" }, anchor)
      : span === "week"
        ? `${fmt({ day: "numeric", month: "short" }, period.start)} – ${fmt({ day: "numeric", month: "short" }, addDays(end, -1))}`
        : new Intl.DateTimeFormat(`${fmtTag()}-u-ca-${calendar()}`, { month: "long", year: "numeric" }).format(period.start),
  );

  // Earlier / later: a day, a week, or a calendar month.
  function move(dir: -1 | 1) {
    showAll = false;
    picked = null;
    if (span === "day") anchor = addDays(anchor, dir);
    else if (span === "week") anchor = addDays(period.start, dir * 7);
    else anchor = dir < 0 ? addDays(period.start, -1) : end;
  }
  function pick(day: Date | null) {
    if (!day || day.getTime() > today.getTime()) return;
    showAll = false;
    picked = sameDay(picked, day) ? null : day;
  }
  function hourByHour(day: Date) {
    span = "day";
    anchor = day;
    picked = null;
  }

  /** The range the app list covers: the picked day, else the period. */
  const appRange = $derived(picked ? { start: picked, end: addDays(picked, 1) } : { start: period.start, end });

  async function loadBars() {
    if (document.hidden) return;
    const start = period.start.getTime();
    try {
      if (span === "day") {
        const hours = await api.trafficBuckets(start, end.getTime(), HOUR);
        bars = hours.map((h, i) => ({ label: (i + 1) % 3 === 0 ? num(i + 1) : "", title: `${num(i)}–${num(i + 1)}`, rx: h.rx_bytes, tx: h.tx_bytes, day: null }));
      } else {
        const list = await api.traffic(start);
        const byDate = new Map(list.map((d) => [d.date, d]));
        bars = Array.from({ length: period.days }, (_, i) => {
          const d = addDays(period.start, i);
          const v = byDate.get(isoDay(d));
          return {
            label: span === "week" ? fmt({ weekday: "short" }, d) : new Intl.DateTimeFormat(`${fmtTag()}-u-ca-${calendar()}`, { day: "numeric" }).format(d),
            title: fmt({ weekday: "long", day: "numeric", month: "long" }, d),
            rx: v?.rx_bytes ?? 0,
            tx: v?.tx_bytes ?? 0,
            day: d,
          };
        });
      }
      const [all, reset] = await Promise.all([api.traffic(0), api.trafficResetAt().catch(() => 0)]);
      const first = all[0]?.date;
      soFar = { bytes: all.reduce((a, d) => a + d.rx_bytes + d.tx_bytes, 0), since: Math.max(reset, first ? new Date(`${first}T00:00`).getTime() : Date.now()) };
      error = null;
    } catch (e) {
      error = errorText(e);
    }
  }
  async function loadApps() {
    if (document.hidden) return;
    try {
      apps = await api.appTraffic(appRange.start.getTime(), appRange.end.getTime());
    } catch (e) {
      error = errorText(e);
    }
  }

  // Reload when the period changes, and once a minute while it is the
  // current one (the agent records a reading a minute).
  $effect(() => {
    void period.start.getTime();
    void period.days;
    void locale.current;
    loadBars();
    if (!isCurrent) return;
    const id = setInterval(loadBars, 60_000);
    document.addEventListener("visibilitychange", loadBars);
    return () => {
      clearInterval(id);
      document.removeEventListener("visibilitychange", loadBars);
    };
  });
  $effect(() => {
    void appRange.start.getTime();
    void appRange.end.getTime();
    loadApps();
    if (appRange.end.getTime() <= Date.now()) return;
    const id = setInterval(loadApps, 60_000);
    return () => clearInterval(id);
  });

  const total = $derived(bars.reduce((a, b) => ({ rx: a.rx + b.rx, tx: a.tx + b.tx }), { rx: 0, tx: 0 }));

  // The scale on the left: 0 and three or four round steps above the
  // busiest bar (5 MB steps on a quiet day, 300 MB on a busy one).
  const scale = $derived.by(() => {
    const max = Math.max(...bars.map((b) => b.rx + b.tx), 0);
    if (max <= 0) return { top: 1, ticks: [0] };
    const raw = max / 4;
    const p = 10 ** Math.floor(Math.log10(raw));
    const step = [1, 2, 2.5, 3, 5, 10].map((m) => m * p).find((s) => s >= raw) ?? 10 * p;
    const n = Math.max(1, Math.ceil(max / step));
    return { top: n * step, ticks: Array.from({ length: n + 1 }, (_, i) => i * step) };
  });
  function tick(b: number): string {
    if (b === 0) return num(0);
    return b >= 1e9 ? `${num(b / 1e9, b % 1e9 ? 1 : 0)} ${t().mon.gb}` : b >= 1e6 ? `${num(b / 1e6, b % 1e6 ? 1 : 0)} ${t().mon.mb}` : `${num(b / 1e3)} KB`;
  }

  // The apps of the range, with what the service sees now (speed, kind,
  // a game inside) for the ones still running when the range is today.
  const live = $derived.by(() => {
    const m = new Map<string, { rx_bps: number; tx_bps: number; kind: string; has_game: boolean }>();
    for (const a of ctl.apps) {
      const k = a.name.toLowerCase();
      const e = m.get(k) ?? { rx_bps: 0, tx_bps: 0, kind: a.kind, has_game: false };
      e.rx_bps += a.rx_bps;
      e.tx_bps += a.tx_bps;
      e.has_game ||= a.has_game;
      m.set(k, e);
    }
    return m;
  });
  const coversNow = $derived(appRange.end.getTime() > Date.now());
  const sorted = $derived(
    [...apps].sort((a, b) =>
      sort === "name"
        ? a.name.localeCompare(b.name)
        : sort === "down"
          ? b.rx_bytes - a.rx_bytes
          : sort === "up"
            ? b.tx_bytes - a.tx_bytes
            : b.rx_bytes + b.tx_bytes - (a.rx_bytes + a.tx_bytes),
    ),
  );
  const shown = $derived(showAll ? sorted : sorted.slice(0, 8));
  const maxApp = $derived(Math.max(1, ...apps.map((a) => a.rx_bytes + a.tx_bytes)));

  function size(b: number): string {
    return b >= 1e9 ? `${num(b / 1e9, b >= 1e11 ? 0 : b >= 1e10 ? 1 : 2)} ${t().mon.gb}` : b >= 1e6 ? `${num(b / 1e6, b >= 1e8 ? 0 : 1)} ${t().mon.mb}` : `${num(b / 1e3)} KB`;
  }
  const Prev = $derived(locale.current === "fa" ? ChevronRight : ChevronLeft);
  const Next = $derived(locale.current === "fa" ? ChevronLeft : ChevronRight);
</script>

{#if error}<p class="problem">{error}</p>{/if}

<section class="card">
  <div class="top">
    <h2>{t().mon.tu.title}</h2>
    <Segmented
      label={t().mon.tu.title}
      value={span}
      options={(["day", "week", "month"] as const).map((v) => ({ value: v, label: t().mon.tu.spans[v] }))}
      onchange={(v) => { span = v; anchor = today; picked = null; showAll = false; }}
    />
    {#if soFar}
      <div class="so-far" title={t().mon.tu.soFarHint}>
        <span>{t().mon.tu.soFar}</span>
        <b><bdi>{size(soFar.bytes)}</bdi></b>
        <small>{t().mon.tu.since(fmt({ day: "numeric", month: "long" }, soFar.since))}</small>
      </div>
    {/if}
  </div>

  <div class="nav-row">
    <button class="nav" onclick={() => move(-1)} aria-label={t().mon.tu.prev[span]} title={t().mon.tu.prev[span]}><Prev size={16} /></button>
    <div class="what">
      <b>{title}</b>
      <span class="sub"><bdi>{size(total.rx + total.tx)}</bdi> · ↓ <bdi>{size(total.rx)}</bdi> · ↑ <bdi>{size(total.tx)}</bdi></span>
    </div>
    <button class="nav" disabled={isCurrent} onclick={() => move(1)} aria-label={t().mon.tu.next[span]} title={t().mon.tu.next[span]}><Next size={16} /></button>
  </div>

  <ul class="legend">
    <li><span class="key" style:background={DOWN}></span>{t().mon.down}</li>
    <li><span class="key" style:background={UP}></span>{t().mon.up}</li>
    {#if span !== "day"}<li class="hint">{t().mon.tu.pickHint}</li>{/if}
  </ul>
  <!-- The scale stays on the left in both languages; the days run the
       page's way (right to left in Persian). -->
  <div class="chart">
    <div class="axis" aria-hidden="true">
      {#each scale.ticks as v (v)}
        <span dir="auto" style:bottom={`calc(${(v / scale.top) * 100}% - 0.5em)`}>{tick(v)}</span>
      {/each}
    </div>
    <div class="plot">
      {#each scale.ticks as v (v)}
        <span class="grid" style:bottom={`${(v / scale.top) * 100}%`}></span>
      {/each}
      <div class="bars" class:many={bars.length > 20} dir={locale.current === "fa" ? "rtl" : "ltr"}>
        {#each bars as b, i (i)}
          {@const future = b.day ? b.day.getTime() > today.getTime() : false}
          <button
            class="bar"
            class:on={sameDay(picked, b.day)}
            class:static={!b.day}
            disabled={!b.day || future}
            aria-pressed={b.day ? sameDay(picked, b.day) : undefined}
            title={`${b.title}: ${size(b.rx + b.tx)} (↓ ${size(b.rx)} · ↑ ${size(b.tx)})`}
            onclick={() => pick(b.day)}
          >
            <span class="stack">
              <span class="part up" style:--h={b.tx / scale.top}></span>
              <span class="part down" style:--h={b.rx / scale.top}></span>
            </span>
          </button>
        {/each}
      </div>
    </div>
    <span></span>
    <div class="labels" class:many={bars.length > 20} dir={locale.current === "fa" ? "rtl" : "ltr"}>
      {#each bars as b, i (i)}<span>{b.label}</span>{/each}
    </div>
  </div>
</section>

<section class="card">
  <div class="top">
    <h2>
      {#if picked}
        {t().mon.tu.appsOn(fmt({ weekday: "long", day: "numeric", month: "long" }, picked))}
      {:else}
        {t().mon.tu.apps}
      {/if}
    </h2>
    {#if picked}
      <button class="chip-btn" onclick={() => picked && hourByHour(picked)}>{t().mon.tu.hourByHour}</button>
      <button class="chip-btn" onclick={() => (picked = null)}><HugeIcon name="cancel-01" size={13} /> {t().mon.tu.wholePeriod}</button>
    {/if}
    <Segmented
      label={t().mon.tu.sortBy}
      value={sort}
      options={(["total", "down", "up", "name"] as const).map((v) => ({ value: v, label: t().mon.tu.sorts[v] }))}
      onchange={(v) => (sort = v)}
    />
  </div>
  {#if ctl.status?.simulated}<p class="note">{t().ping.simulated}</p>{/if}
  {#if ctl.status && !ctl.status.available}<p class="note">{t().ping.unavailable} {ctl.status.problem}</p>{/if}
  {#if apps.length}
    <ol class="apps">
      {#each shown as a (a.name)}
        {@const now = coversNow ? live.get(a.name.toLowerCase()) : undefined}
        <li>
          <span class="name">
            <bdi>{a.name}</bdi>
            <small>
              {#if now}{t().ping.kinds[now.kind] ?? now.kind}{/if}
              {#if a.category}<span class="chip">{t().ping.categories[a.category] ?? a.category}</span>{/if}
              {#if now?.has_game}<span class="warn" title={t().ping.hasGame}>⚠</span>{/if}
            </small>
          </span>
          <span class="meter" aria-hidden="true">
            <span class="down" style:--w={a.rx_bytes / maxApp}></span>
            <span class="up" style:--w={a.tx_bytes / maxApp}></span>
          </span>
          <span class="nums">
            <b><bdi>{size(a.rx_bytes + a.tx_bytes)}</bdi></b>
            <small>↓ <bdi>{size(a.rx_bytes)}</bdi> · ↑ <bdi>{size(a.tx_bytes)}</bdi></small>
            {#if now && now.rx_bps + now.tx_bps > 0}
              {@const d = rate(now.rx_bps)}
              {@const u = rate(now.tx_bps)}
              <small class="speed">{t().mon.tu.nowSpeed} <bdi>↓ {d.value} {d.unit} · ↑ {u.value} {u.unit}</bdi></small>
            {/if}
          </span>
          <AppRuleActions {ctl} name={a.name} />
        </li>
        {#if ctl.editing?.app.toLowerCase() === a.name.toLowerCase()}<li class="editing"><AppRuleActions {ctl} name={a.name} part="form" /></li>{/if}
      {/each}
    </ol>
    {#if sorted.length > 8}
      <button class="more" onclick={() => (showAll = !showAll)}>{showAll ? t().mon.tu.fewer : t().mon.tu.all(sorted.length)}</button>
    {/if}
  {:else}
    <p class="muted">{t().mon.tu.noApps}</p>
  {/if}
  <p class="small">{t().mon.tu.appsNote}</p>
</section>

<style>
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; margin-bottom: 16px; min-width: 0; }
  h2 { margin: 0; font-size: 0.98rem; font-weight: 700; }
  .problem { margin: 0 0 14px; padding: 12px 16px; border-radius: var(--radius-sm); background: color-mix(in oklab, var(--danger) 12%, var(--card)); color: var(--danger); font-size: 0.9rem; }
  .note { margin: 0 0 12px; padding: 10px 14px; border-radius: var(--radius-sm); background: var(--card-2); color: var(--text-2); font-size: 0.85rem; }
  .top { display: flex; align-items: center; flex-wrap: wrap; gap: 12px; margin-bottom: 14px; }
  .so-far { margin-inline-start: auto; display: grid; justify-items: end; gap: 1px; padding: 8px 14px; border-radius: var(--radius-sm); background: var(--card-2); }
  .so-far span { color: var(--text-2); font-size: 0.78rem; }
  .so-far b { font-size: 1.25rem; font-weight: 800; }
  .so-far small { color: var(--text-3); font-size: 0.72rem; }
  .nav-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 10px; }
  .what { display: grid; gap: 2px; text-align: center; }
  .sub { color: var(--text-2); font-size: 0.8rem; }
  .nav { all: unset; display: inline-flex; padding: 6px; border-radius: 8px; background: var(--card-2); cursor: pointer; }
  .nav:disabled { opacity: 0.4; cursor: default; }
  .nav:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .legend { list-style: none; margin: 0 0 10px; padding: 0; display: flex; flex-wrap: wrap; gap: 14px; font-size: 0.8rem; color: var(--text-2); }
  .legend li { display: flex; align-items: center; gap: 6px; }
  .legend .hint { color: var(--text-3); margin-inline-start: auto; }
  .key { width: 11px; height: 11px; border-radius: 3px; }
  .chart { display: grid; grid-template-columns: auto 1fr; grid-template-rows: 180px auto; column-gap: 8px; row-gap: 4px; direction: ltr; }
  .axis { position: relative; min-width: 3.4em; }
  .axis span { position: absolute; right: 0; font-size: 0.66rem; line-height: 1; color: var(--text-3); white-space: nowrap; }
  .plot { position: relative; min-width: 0; }
  .grid { position: absolute; left: 0; right: 0; height: 0; border-top: 1px dashed color-mix(in oklab, var(--text-3) 30%, transparent); }
  .bars { position: absolute; inset: 0; display: flex; gap: 4px; align-items: stretch; }
  .bars.many, .labels.many { gap: 2px; }
  .bar { all: unset; flex: 1; min-width: 0; display: flex; justify-content: center; border-radius: 5px; cursor: pointer; }
  .bar.static { cursor: default; }
  .bar:not(.static):not(:disabled):hover { background: color-mix(in oklab, var(--card-2) 70%, transparent); }
  .bar.on { background: color-mix(in oklab, var(--accent) 22%, transparent); }
  .bar:disabled:not(.static) { opacity: 0.35; cursor: default; }
  .bar:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  .stack { width: 78%; max-width: 26px; display: flex; flex-direction: column; justify-content: flex-end; gap: 1px; }
  .part { display: block; height: calc(var(--h) * 100%); min-height: 0; }
  .part.up { background: #c98500; border-radius: 3px 3px 0 0; }
  .part.down { background: #8f6af2; }
  .labels { display: flex; gap: 4px; }
  .labels span { flex: 1; min-width: 0; text-align: center; font-size: 0.66rem; color: var(--text-3); white-space: nowrap; overflow: hidden; }
  .chip-btn { all: unset; display: inline-flex; align-items: center; gap: 4px; padding: 5px 11px; border-radius: 999px; background: var(--card-2); font-size: 0.8rem; color: var(--text-2); cursor: pointer; }
  .chip-btn:hover { color: var(--text); }
  .chip-btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .apps { list-style: none; margin: 0; padding: 0; display: grid; }
  .apps li { display: grid; grid-template-columns: minmax(130px, 1.1fr) 1.4fr auto auto; align-items: center; gap: 14px; padding: 10px 0; border-bottom: 1px solid var(--line); }
  .apps li:last-child { border-bottom: 0; }
  .apps li.editing { display: block; padding-top: 0; }
  .name { display: grid; gap: 2px; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .name small { color: var(--text-3); font-size: 0.74rem; display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
  .chip { padding: 1px 8px; border-radius: 999px; background: var(--card-2); color: var(--text-2); font-size: 0.72rem; }
  .warn { color: var(--warn, #fbbf24); cursor: help; }
  .meter { display: flex; height: 8px; border-radius: 4px; background: var(--card-2); overflow: hidden; }
  .meter span { display: block; width: calc(var(--w) * 100%); }
  .meter .down { background: #8f6af2; }
  .meter .up { background: #c98500; }
  .nums { display: grid; justify-items: end; gap: 1px; white-space: nowrap; }
  .nums small { color: var(--text-2); font-size: 0.76rem; }
  .nums .speed { color: var(--accent-text); }
  .more { all: unset; margin-top: 12px; padding: 6px 12px; border-radius: 8px; background: var(--card-2); font-size: 0.84rem; cursor: pointer; }
  .more:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .muted { color: var(--text-2); margin: 0; font-size: 0.9rem; }
  .small { color: var(--text-3); font-size: 0.8rem; margin: 12px 0 0; }
  @media (max-width: 900px) { .apps li { grid-template-columns: 1fr auto; } .meter { display: none; } }
</style>
