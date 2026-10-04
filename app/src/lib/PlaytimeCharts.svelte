<script lang="ts">
  // Playtime charts from the recorded sessions: hours per day stacked by
  // game (7 or 30 days), a day's sessions on a 24-hour line, the hours of
  // the day you usually play, and the figures behind them. Everything is
  // worked out here from `sessions`; a session over midnight counts on both
  // days, and only the part inside a day or hour is counted there.
  import { api, errorText, type PlaytimeSession } from "./api";
  import { num } from "./format";
  import { locale, t } from "./i18n.svelte";
  import Segmented from "./ui/Segmented.svelte";

  let { focus = null, onfocus }: { focus?: string | null; onfocus?: (key: string | null) => void } = $props();

  const DAY = 86_400_000;
  const HOUR = 3_600_000;
  // Five distinct series colours, then "other games" in a neutral tone.
  const COLORS = ["#8f6af2", "#c98500", "#2a9d8f", "#d1495b", "#4f86c6"];
  const OTHER = "var(--text-3)";

  let span = $state<"7" | "30">("7");
  let all = $state<PlaytimeSession[]>([]);
  let picked = $state<number | null>(null);
  let error = $state<string | null>(null);

  function midnight(ms: number): number {
    const d = new Date(ms);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  }
  // The day `i` days after `start`, at its local midnight (clock changes
  // make some days 23 or 25 hours).
  function dayAt(start: number, i: number): number {
    const d = new Date(start);
    d.setDate(d.getDate() + i);
    return d.getTime();
  }

  $effect(() => {
    const n = Number(span);
    const load = () => {
      if (document.hidden) return;
      api.playtimeSessions(dayAt(midnight(Date.now()), -(n - 1)))
        .then((s) => { all = s; error = null; })
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

  const games = $derived(all.filter((s) => !s.key.startsWith("app:") && (!focus || s.key === focus)));
  const overlap = (s: PlaytimeSession, a: number, b: number) => Math.max(0, Math.min(s.ended_unix_ms, b) - Math.max(s.started_unix_ms, a)) / 1000;

  // The most played games of the span get colours; the rest are "other".
  const ranked = $derived.by(() => {
    const by = new Map<string, { key: string; name: string; secs: number }>();
    for (const s of games) {
      const e = by.get(s.key) ?? { key: s.key, name: s.name, secs: 0 };
      e.secs += (s.ended_unix_ms - s.started_unix_ms) / 1000;
      by.set(s.key, e);
    }
    return [...by.values()].sort((a, b) => b.secs - a.secs);
  });
  const colorOf = $derived(new Map(ranked.slice(0, COLORS.length).map((g, i) => [g.key, COLORS[i]])));
  const legend = $derived([
    ...ranked.slice(0, COLORS.length).map((g) => ({ key: g.key, name: g.name, color: COLORS[ranked.indexOf(g)] })),
    ...(ranked.length > COLORS.length ? [{ key: "", name: t().pc.other, color: OTHER }] : []),
  ]);

  const days = $derived.by(() => {
    const n = Number(span);
    const first = dayAt(midnight(Date.now()), -(n - 1));
    return Array.from({ length: n }, (_, i) => {
      const start = dayAt(first, i);
      const end = dayAt(first, i + 1);
      const parts = new Map<string, number>();
      for (const s of games) {
        const secs = overlap(s, start, end);
        if (secs > 0) {
          const k = colorOf.has(s.key) ? s.key : "";
          parts.set(k, (parts.get(k) ?? 0) + secs);
        }
      }
      const total = [...parts.values()].reduce((a, b) => a + b, 0);
      const segs = [...parts.entries()]
        .map(([key, secs]) => ({ key, secs, color: colorOf.get(key) ?? OTHER }))
        .sort((a, b) => (colorOf.has(a.key) ? COLORS.indexOf(colorOf.get(a.key)!) : 99) - (colorOf.has(b.key) ? COLORS.indexOf(colorOf.get(b.key)!) : 99));
      return { start, end, total, segs };
    });
  });
  // The scale rounds up to whole hours, so the grid lines read as hours.
  const scale = $derived(Math.max(1, Math.ceil(Math.max(...days.map((d) => d.total)) / 3600)) * 3600);
  const ticks = $derived.by(() => {
    const h = scale / 3600;
    const step = h <= 4 ? 1 : h <= 8 ? 2 : Math.ceil(h / 4);
    return Array.from({ length: Math.floor(h / step) + 1 }, (_, i) => i * step * 3600);
  });
  const played = $derived(days.filter((d) => d.total > 0));
  const average = $derived(days.reduce((a, d) => a + d.total, 0) / days.length);

  // Hours of the day: total over the span for each local hour.
  const hours = $derived.by(() => {
    const out = Array.from({ length: 24 }, () => 0);
    for (const s of games) {
      let t0 = s.started_unix_ms;
      while (t0 < s.ended_unix_ms) {
        const d = new Date(t0);
        const next = new Date(d);
        next.setMinutes(0, 0, 0);
        next.setHours(d.getHours() + 1);
        const t1 = Math.min(next.getTime(), s.ended_unix_ms);
        out[d.getHours()] += (t1 - t0) / 1000;
        t0 = t1;
      }
    }
    return out;
  });
  const hourMax = $derived(Math.max(1, ...hours));
  const peakHour = $derived(hours.some((h) => h > 0) ? hours.indexOf(Math.max(...hours)) : null);

  // Figures: sessions inside the span, streak of days played up to today.
  const lengths = $derived(games.map((s) => (s.ended_unix_ms - s.started_unix_ms) / 1000));
  const longest = $derived(games.reduce<PlaytimeSession | null>((a, s) => (!a || s.ended_unix_ms - s.started_unix_ms > a.ended_unix_ms - a.started_unix_ms ? s : a), null));
  const streak = $derived.by(() => {
    let n = 0;
    for (let i = days.length - 1; i >= 0; i--) {
      if (days[i].total >= 60) n++;
      else if (i === days.length - 1) continue; // today may not have started yet
      else break;
    }
    return n;
  });

  const selected = $derived(picked != null ? (days.find((d) => d.start === picked) ?? null) : null);
  const daySessions = $derived(
    selected ? games.filter((s) => overlap(s, selected.start, selected.end) > 0).sort((a, b) => a.started_unix_ms - b.started_unix_ms) : [],
  );

  const fmt = () => (locale.current === "fa" ? "fa-IR" : "en-GB");
  function hm(secs: number): string {
    if (secs < 60) return t().play.lessThanMin;
    return t().play.hm(Math.floor(secs / 3600), Math.floor((secs % 3600) / 60));
  }
  const dayLabel = (ms: number) => new Intl.DateTimeFormat(fmt(), span === "7" ? { weekday: "short" } : { day: "numeric" }).format(ms);
  const longDay = (ms: number) => new Intl.DateTimeFormat(fmt(), { weekday: "long", day: "numeric", month: "long" }).format(ms);
  const clock = (ms: number) => new Intl.DateTimeFormat(fmt(), { hour: "2-digit", minute: "2-digit", hour12: false }).format(ms);
  const nameOf = (key: string) => ranked.find((g) => g.key === key)?.name ?? t().pc.other;
</script>

{#if error}<p class="problem">{error}</p>{/if}

<section class="figures">
  <div><span>{t().pc.dailyAverage}</span><b>{hm(average)}</b></div>
  <div><span>{t().pc.daysPlayed}</span><b>{num(played.length)} / {num(days.length)}</b></div>
  <div><span>{t().pc.avgSession}</span><b>{lengths.length ? hm(lengths.reduce((a, b) => a + b, 0) / lengths.length) : "—"}</b></div>
  <div><span>{t().pc.longest}</span><b>{longest ? hm((longest.ended_unix_ms - longest.started_unix_ms) / 1000) : "—"}</b>{#if longest}<small dir="auto">{longest.name}</small>{/if}</div>
  <div><span>{t().pc.streak}</span><b>{t().pc.days(streak)}</b></div>
</section>

<article class="card">
  <div class="head">
    <h2>{t().pc.perDay}</h2>
    {#if focus}
      <button class="chip on" onclick={() => onfocus?.(null)}>{nameOf(focus)} ✕</button>
    {/if}
    <Segmented label={t().pc.perDay} value={span} options={[{ value: "7", label: t().pc.span7 }, { value: "30", label: t().pc.span30 }]} onchange={(v) => { span = v; picked = null; }} />
  </div>
  {#if legend.length > 1}
    <ul class="legend">
      {#each legend as l (l.key)}
        <li>
          <button disabled={!l.key} onclick={() => onfocus?.(l.key)} title={l.key ? t().pc.only(l.name) : undefined}>
            <span class="key" style:background={l.color}></span><span dir="auto">{l.name}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
  <div class="plot">
    <div class="axis" aria-hidden="true">
      {#each ticks as tk}
        <span style:--y={tk / scale}>{num(tk / 3600)}{t().pc.h}</span>
      {/each}
    </div>
    <div class="area">
      {#each ticks as tk}<span class="grid" style:--y={tk / scale}></span>{/each}
      {#if average > 0}<span class="avg-line" style:--y={average / scale} title={`${t().pc.dailyAverage}: ${hm(average)}`}></span>{/if}
      <div class="bars" class:dense={span === "30"}>
        {#each days as d (d.start)}
          <button
            class="day"
            class:on={picked === d.start}
            title={`${longDay(d.start)}: ${hm(d.total)}`}
            aria-label={`${longDay(d.start)}: ${hm(d.total)}`}
            onclick={() => (picked = picked === d.start ? null : d.start)}
          >
            <span class="stack" style:--h={d.total / scale}>
              {#each d.segs as sg (sg.key)}
                <span class="seg" style:flex-grow={sg.secs} style:background={sg.color} title={`${nameOf(sg.key)}: ${hm(sg.secs)}`}></span>
              {/each}
            </span>
            <span class="lbl">{dayLabel(d.start)}</span>
          </button>
        {/each}
      </div>
    </div>
  </div>
  <p class="small">{t().pc.dayHint}</p>

  {#if selected}
    <div class="dayview">
      <h3>{longDay(selected.start)} · {hm(selected.total)}</h3>
      {#if daySessions.length === 0}
        <p class="muted">{t().pc.noSessions}</p>
      {:else}
        <div class="timeline" dir="ltr" aria-hidden="true">
          {#each [0, 6, 12, 18, 24] as h}<span class="tick" style:--x={h / 24}>{num(h)}</span>{/each}
          {#each daySessions as s, i (s.started_unix_ms + s.key)}
            {@const a = Math.max(s.started_unix_ms, selected.start)}
            {@const b = Math.min(s.ended_unix_ms, selected.end)}
            <span
              class="span"
              style:--x={(a - selected.start) / (selected.end - selected.start)}
              style:--w={(b - a) / (selected.end - selected.start)}
              style:--row={i % 3}
              style:background={colorOf.get(s.key) ?? OTHER}
            ></span>
          {/each}
        </div>
        <ul class="sessions">
          {#each daySessions as s (s.started_unix_ms + s.key)}
            <li>
              <span class="key" style:background={colorOf.get(s.key) ?? OTHER}></span>
              <b dir="auto">{s.name}</b>
              <span class="when" dir="ltr">{clock(s.started_unix_ms)}–{clock(s.ended_unix_ms)}</span>
              <span class="len">{hm((s.ended_unix_ms - s.started_unix_ms) / 1000)}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
</article>

<article class="card">
  <h2>{t().pc.whenTitle}</h2>
  <div class="hours" dir="ltr">
    {#each hours as secs, h}
      <div class="hour" title={`${num(h)}:00–${num(h + 1)}:00 · ${hm(secs)}`}>
        <span class="hbar" class:peak={h === peakHour} style:--h={secs / hourMax}></span>
        <span class="lbl">{h % 3 === 0 ? num(h) : ""}</span>
      </div>
    {/each}
  </div>
  <p class="small">
    {#if peakHour != null}{t().pc.peak(`${num(peakHour)}:00`)}{:else}{t().pc.noneYet}{/if}
  </p>
</article>

<style>
  .problem { margin: 0 0 14px; padding: 12px 16px; border-radius: var(--radius-sm); background: color-mix(in oklab, var(--danger) 12%, var(--card)); color: var(--danger); font-size: 0.9rem; }
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; min-width: 0; margin-bottom: 16px; }
  h2 { margin: 0; font-size: 0.98rem; font-weight: 700; }
  h3 { margin: 0 0 10px; font-size: 0.9rem; font-weight: 700; }
  .head { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; margin-bottom: 10px; }
  .head h2 { margin-inline-end: auto; }
  .figures { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 12px; margin-bottom: 16px; }
  .figures div { display: grid; gap: 3px; padding: 12px 14px; border-radius: var(--radius); background: var(--card); min-width: 0; }
  .figures span { color: var(--text-2); font-size: 0.8rem; }
  .figures b { font-size: 1.15rem; font-weight: 800; }
  .figures small { color: var(--text-3); font-size: 0.75rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .chip { all: unset; padding: 3px 10px; border-radius: 999px; background: var(--card-2); font-size: 0.78rem; cursor: pointer; }
  .chip.on { background: var(--accent-soft, var(--card-2)); color: var(--accent-text); }
  .legend { list-style: none; margin: 0 0 10px; padding: 0; display: flex; flex-wrap: wrap; gap: 4px 12px; }
  .legend button { all: unset; display: inline-flex; align-items: center; gap: 6px; font-size: 0.8rem; color: var(--text-2); cursor: pointer; padding: 2px 4px; border-radius: 6px; }
  .legend button:hover:not(:disabled) { color: var(--text); background: var(--card-2); }
  .legend button:disabled { cursor: default; }
  .key { width: 11px; height: 11px; border-radius: 3px; flex: none; display: inline-block; }
  .plot { display: grid; grid-template-columns: auto 1fr; gap: 8px; height: 200px; }
  .axis { position: relative; width: 2.6em; height: calc(100% - 20px); }
  .axis span { position: absolute; inset-inline-end: 0; bottom: calc(var(--y) * 100%); transform: translateY(50%); font-size: 0.68rem; color: var(--text-3); }
  .area { position: relative; min-width: 0; }
  .grid, .avg-line { position: absolute; inset-inline: 0; bottom: calc(20px + var(--y) * (100% - 20px)); height: 0; border-top: 1px solid var(--line); pointer-events: none; }
  .avg-line { border-top: 1.5px dashed var(--accent-text); opacity: 0.8; z-index: 1; pointer-events: auto; }
  .bars { position: absolute; inset: 0; display: flex; gap: 6px; }
  .bars.dense { gap: 2px; }
  .day { all: unset; flex: 1; min-width: 0; display: flex; flex-direction: column; justify-content: flex-end; align-items: center; cursor: pointer; border-radius: 6px; }
  .day:hover { background: color-mix(in oklab, var(--card-2) 60%, transparent); }
  .day.on { background: color-mix(in oklab, var(--accent) 16%, transparent); }
  .day:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  .stack { width: 70%; max-width: 30px; height: calc(var(--h) * (100% - 20px)); display: flex; flex-direction: column-reverse; border-radius: 5px 5px 0 0; overflow: hidden; }
  .seg { display: block; flex-basis: 0; min-height: 1px; }
  .lbl { height: 20px; line-height: 20px; font-size: 0.68rem; color: var(--text-3); white-space: nowrap; }
  .small { color: var(--text-3); font-size: 0.8rem; margin: 10px 0 0; }
  .muted { color: var(--text-2); font-size: 0.88rem; margin: 0; }
  .dayview { margin-top: 14px; padding-top: 14px; border-top: 1px solid var(--line); }
  .timeline { position: relative; height: 64px; border-radius: 8px; background: var(--card-2); margin-bottom: 10px; }
  .tick { position: absolute; left: calc(var(--x) * 100%); bottom: 2px; transform: translateX(-50%); font-size: 0.64rem; color: var(--text-3); }
  .tick:first-child { transform: none; }
  .tick:nth-child(5) { transform: translateX(-100%); }
  .span { position: absolute; left: calc(var(--x) * 100%); width: max(3px, calc(var(--w) * 100%)); top: calc(6px + var(--row) * 14px); height: 12px; border-radius: 4px; }
  .sessions { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; }
  .sessions li { display: flex; align-items: center; gap: 10px; font-size: 0.86rem; }
  .sessions b { flex: 1; min-width: 0; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .when { color: var(--text-2); font-size: 0.8rem; }
  .len { color: var(--text-3); font-size: 0.8rem; min-width: 6em; text-align: end; }
  .hours { display: flex; gap: 3px; height: 120px; align-items: stretch; margin-top: 12px; }
  .hour { flex: 1; min-width: 0; display: flex; flex-direction: column; justify-content: flex-end; align-items: center; }
  .hbar { width: 80%; height: calc(var(--h) * (100% - 18px)); min-height: 1px; background: color-mix(in oklab, var(--accent) 55%, var(--card-2)); border-radius: 3px 3px 0 0; }
  .hbar.peak { background: var(--accent); }
  .hour .lbl { height: 18px; line-height: 18px; }
</style>
