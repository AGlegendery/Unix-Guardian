<script lang="ts">
  // Playtime: what the agent recorded, by period. The agent counts every
  // detected game by itself; this page only reads, deletes and exports.
  import { Download, Trash2 } from "@lucide/svelte";
  import Problem from "../lib/ui/Problem.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";
  import { api, errorText, type PlaytimeEntry, type PlaytimeSession } from "../lib/api";
  import { num } from "../lib/format";
  import { locale, t } from "../lib/i18n.svelte";

  type Range = "today" | "week" | "all";
  const DAY = 86_400_000;
  let range = $state<Range>("week");
  let entries = $state<PlaytimeEntry[] | null>(null);
  let sessions = $state<PlaytimeSession[]>([]);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let savedTo = $state<string | null>(null);
  let armed = $state<string | null>(null);
  let busy = $state(false);

  function midnight(ms: number): number {
    const d = new Date(ms);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  }
  // The same periods as `ug playtime --today/--week`.
  function since(r: Range): number {
    return r === "today" ? midnight(Date.now()) : r === "week" ? Date.now() - 7 * DAY : 0;
  }

  async function load() {
    try {
      const [e, s] = await Promise.all([api.playtime(since(range)), api.playtimeSessions(midnight(Date.now()) - 6 * DAY)]);
      entries = e;
      sessions = s;
      error = null;
    } catch (e) {
      error = errorText(e);
    }
  }

  // A running game's time grows by the minute (the agent's heartbeat), so
  // a slow refresh while the page is on screen is enough.
  $effect(() => {
    void range;
    load();
    const id = setInterval(() => { if (!document.hidden) load(); }, 60_000);
    return () => clearInterval(id);
  });

  const total = $derived((entries ?? []).reduce((a, e) => a + e.seconds, 0));
  const top = $derived(entries?.[0] ?? null);

  // Seconds played on each of the last seven local days; a session over
  // midnight counts on both days.
  const days = $derived.by(() => {
    const start = midnight(Date.now()) - 6 * DAY;
    const out = Array.from({ length: 7 }, (_, i) => ({ start: midnight(start + i * DAY + DAY / 2), secs: 0 }));
    for (const s of sessions) {
      for (const d of out) {
        const end = d.start + DAY;
        const overlap = Math.min(s.ended_unix_ms, end) - Math.max(s.started_unix_ms, d.start);
        if (overlap > 0) d.secs += overlap / 1000;
      }
    }
    return out;
  });
  const dayMax = $derived(Math.max(3600, ...days.map((d) => d.secs)));

  function hm(secs: number): string {
    if (secs < 60) return t().play.lessThanMin;
    const h = Math.floor(secs / 3600);
    return t().play.hm(h, Math.floor((secs % 3600) / 60));
  }
  function weekday(ms: number): string {
    return new Intl.DateTimeFormat(locale.current === "fa" ? "fa-IR" : "en-US", { weekday: "short" }).format(ms);
  }
  function lastPlayed(ms: number): string {
    return new Intl.DateTimeFormat(locale.current === "fa" ? "fa-IR" : "en-US", { dateStyle: "medium" }).format(ms);
  }

  // Delete asks twice: the first click turns the button into "Delete all of
  // it?", which goes back by itself after a few seconds.
  let disarm: ReturnType<typeof setTimeout> | undefined;
  async function remove(e: PlaytimeEntry) {
    if (armed !== e.key) {
      armed = e.key;
      clearTimeout(disarm);
      disarm = setTimeout(() => (armed = null), 4000);
      return;
    }
    armed = null;
    busy = true;
    try {
      const n = await api.playtimeForget(e.key);
      notice = t().play.removed(e.name, n);
      savedTo = null;
      await load();
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }

  async function exportCsv() {
    busy = true;
    try {
      savedTo = await api.playtimeExport(since(range));
      notice = null;
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

<header class="head">
  <h1>{t().play.title}</h1>
  {#if api.preview}<span class="badge">{t().preview}</span>{/if}
  <div class="tools">
    <Segmented
      label={t().play.rangeLabel}
      value={range}
      options={(["today", "week", "all"] as const).map((r) => ({ value: r, label: t().play.ranges[r] }))}
      onchange={(r) => { range = r; notice = null; savedTo = null; }}
    />
    <button class="btn" onclick={exportCsv} disabled={busy || !entries?.length}>
      <Download size={15} aria-hidden="true" />{t().play.export}
    </button>
  </div>
</header>

{#if error}<Problem text={error} />{/if}
{#if notice}<p class="note" role="status">{notice}</p>{/if}
{#if savedTo}<p class="note" role="status">{t().play.exported} <bdi dir="ltr">{savedTo}</bdi></p>{/if}

<section class="stats">
  <article class="card stat">
    <span class="label">{t().play.total[range]}</span>
    <b>{entries ? hm(total) : "—"}</b>
  </article>
  <article class="card stat">
    <span class="label">{t().play.games}</span>
    <b>{entries ? num(entries.length) : "—"}</b>
  </article>
  <article class="card stat">
    <span class="label">{t().play.top}</span>
    <b class="name" dir="auto">{top?.name ?? "—"}</b>
    {#if top}<small>{hm(top.seconds)}</small>{/if}
  </article>
</section>

<div class="grid">
  <article class="card">
    <h2>{t().play.list}</h2>
    {#if entries && entries.length}
      <ol class="list">
        {#each entries as e (e.key)}
          {@const pct = total ? (e.seconds * 100) / total : 0}
          <li>
            <div class="row1">
              <span class="game" dir="auto">{e.name}</span>
              <span class="time">{hm(e.seconds)}</span>
              <span class="pct">{num(pct)}{t().unit.pct}</span>
            </div>
            <span class="bar" aria-hidden="true"><span style:--w={pct / 100}></span></span>
            <div class="row2">
              <span class="meta">{t().play.sessions(e.sessions)} · {t().play.last} {lastPlayed(e.last_played_unix_ms)}</span>
              <button
                class="del"
                class:armed={armed === e.key}
                onclick={() => remove(e)}
                disabled={busy}
                title={t().play.removeWhy}
              >
                <Trash2 size={13} aria-hidden="true" />{armed === e.key ? t().play.confirm : t().play.remove}
              </button>
            </div>
          </li>
        {/each}
      </ol>
    {:else if entries}
      <p class="muted">{t().play.empty[range]}</p>
    {/if}
    <p class="small">{t().play.how}</p>
  </article>

  <article class="card">
    <h2>{t().play.days}</h2>
    <div class="days" role="img" aria-label={t().play.daysHint}>
      {#each days as d (d.start)}
        <div class="day" title={`${weekday(d.start)}: ${hm(d.secs)}`}>
          <span class="col"><span style:--h={d.secs / dayMax}></span></span>
          <span class="dlabel">{weekday(d.start)}</span>
        </div>
      {/each}
    </div>
    <p class="small">{t().play.daysHint}</p>
  </article>
</div>

<style>
  .head { display: flex; align-items: center; gap: 14px; margin-bottom: 16px; flex-wrap: wrap; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 800; }
  h2 { margin: 0 0 12px; font-size: 0.98rem; font-weight: 700; }
  .badge { padding: 3px 12px; border-radius: 999px; font-size: 0.8rem; background: var(--card-2); color: var(--text-2); }
  .tools { margin-inline-start: auto; display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
  .note { margin: 0 0 14px; padding: 10px 16px; border-radius: var(--radius-sm); background: var(--card-2); color: var(--text-2); font-size: 0.88rem; overflow-wrap: anywhere; }
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; min-width: 0; }
  .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 16px; margin-bottom: 16px; }
  .stat { display: grid; gap: 6px; align-content: start; }
  .stat .label { color: var(--text-2); font-size: 0.85rem; }
  .stat b { font-size: 1.55rem; font-weight: 800; }
  .stat b.name { font-size: 1.2rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .stat small { color: var(--text-2); font-size: 0.82rem; }
  .grid { display: grid; grid-template-columns: minmax(0, 1.6fr) minmax(0, 1fr); gap: 16px; align-items: start; }
  @media (max-width: 980px) { .grid { grid-template-columns: 1fr; } }

  .list { list-style: none; margin: 0; padding: 0; }
  .list li { padding: 12px 0; border-bottom: 1px solid var(--line); display: grid; gap: 7px; }
  .list li:first-child { padding-top: 2px; }
  .list li:last-child { border-bottom: 0; }
  .row1, .row2 { display: flex; align-items: baseline; gap: 12px; min-width: 0; }
  .game { flex: 1; min-width: 0; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .time { color: var(--text); font-size: 0.9rem; }
  .pct { color: var(--text-3); font-size: 0.82rem; min-width: 3.2em; text-align: end; }
  .bar { display: block; height: 6px; border-radius: 999px; background: var(--card-2); overflow: hidden; }
  .bar span { display: block; height: 100%; width: calc(var(--w) * 100%); background: var(--accent); border-radius: inherit; }
  .meta { flex: 1; color: var(--text-3); font-size: 0.78rem; }
  .del {
    all: unset; display: inline-flex; align-items: center; gap: 5px; padding: 3px 9px; border-radius: 8px;
    font-size: 0.78rem; color: var(--text-3); cursor: pointer; transition: background-color 0.15s ease, color 0.15s ease;
  }
  .del:hover:not(:disabled) { color: var(--danger); background: color-mix(in oklab, var(--danger) 10%, transparent); }
  .del.armed { color: var(--danger); background: color-mix(in oklab, var(--danger) 14%, transparent); font-weight: 600; }
  .del:disabled { opacity: 0.5; cursor: default; }
  .del:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }

  .days { display: grid; grid-template-columns: repeat(7, 1fr); gap: 8px; height: 170px; align-items: end; }
  .day { display: grid; grid-template-rows: 1fr auto; height: 100%; gap: 6px; justify-items: center; }
  .col { width: 100%; max-width: 30px; height: 100%; border-radius: 7px; background: var(--card-2); display: flex; align-items: flex-end; overflow: hidden; }
  .col span { display: block; width: 100%; height: calc(var(--h) * 100%); background: var(--accent); border-radius: 7px; }
  .dlabel { font-size: 0.72rem; color: var(--text-3); }

  .muted { color: var(--text-2); margin: 0; font-size: 0.9rem; }
  .small { color: var(--text-3); font-size: 0.8rem; margin: 14px 0 0; line-height: 1.55; }
  .btn { all: unset; display: inline-flex; align-items: center; gap: 6px; padding: 7px 12px; border-radius: 10px; background: var(--card-2); color: var(--text); font-size: 0.85rem; cursor: pointer; }
  .btn:hover:not(:disabled) { background: color-mix(in oklab, var(--card-2) 80%, var(--text) 8%); }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
</style>
