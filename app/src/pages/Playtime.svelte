<script lang="ts">
  // Playtime: what the agent recorded, by period, for games and for the apps
  // the user asked to have timed. The agent does the counting; this page
  // reads, corrects ("not a game"), imports Steam's own count, deletes and
  // exports.
  import { Download } from "@lucide/svelte";
  import HugeIcon from "../lib/icons/HugeIcon.svelte";
  import Problem from "../lib/ui/Problem.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";
  import { api, errorText, type PlaytimeEntry, type PlaytimeSession, type TrackedApp } from "../lib/api";
  import { num } from "../lib/format";
  import { locale, t } from "../lib/i18n.svelte";
  import PlaytimeCharts from "../lib/PlaytimeCharts.svelte";

  type Range = "today" | "week" | "all";
  type Tab = "games" | "apps";
  const DAY = 86_400_000;
  const isApp = (key: string) => key.startsWith("app:");
  let tab = $state<Tab>(location.hash.split("/")[1] === "apps" ? "apps" : "games");
  let range = $state<Range>("week");
  let entries = $state<PlaytimeEntry[] | null>(null);
  let sessions = $state<PlaytimeSession[]>([]);
  let imported = $state<PlaytimeEntry[]>([]);
  let tracked = $state<TrackedApp[]>([]);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let savedTo = $state<string | null>(null);
  let armed = $state<string | null>(null);
  let busy = $state(false);
  // A game picked in the list or the legend: the charts show only it.
  let focus = $state<string | null>(null);

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
      const [e, s, i, a] = await Promise.all([
        api.playtime(since(range)),
        api.playtimeSessions(midnight(Date.now()) - 6 * DAY),
        api.playtimeImported(),
        api.trackedApps(),
      ]);
      entries = e;
      sessions = s;
      imported = i;
      tracked = a;
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

  const games = $derived((entries ?? []).filter((e) => !isApp(e.key)));
  // Every tracked app, with or without time in this period, and apps no
  // longer tracked that still have time.
  const apps = $derived.by(() => {
    const timed = (entries ?? []).filter((e) => isApp(e.key));
    const rows = timed.map((e) => ({ ...e, exe: e.key.slice(4), counting: tracked.some((a) => `app:${a.exe}` === e.key) }));
    for (const a of tracked) {
      if (!rows.some((r) => r.exe === a.exe)) {
        rows.push({ key: `app:${a.exe}`, exe: a.exe, name: a.name, seconds: 0, sessions: 0, last_played_unix_ms: 0, counting: true });
      }
    }
    return rows;
  });
  const gamesTotal = $derived(games.reduce((a, e) => a + e.seconds, 0));
  const appsTotal = $derived(apps.reduce((a, e) => a + e.seconds, 0));
  const top = $derived(games[0] ?? null);
  const importedBy = $derived(new Map(imported.map((i) => [i.key, i.seconds])));

  function hm(secs: number): string {
    if (secs < 60) return t().play.lessThanMin;
    const h = Math.floor(secs / 3600);
    return t().play.hm(h, Math.floor((secs % 3600) / 60));
  }
  function lastPlayed(ms: number): string {
    return new Intl.DateTimeFormat(locale.current === "fa" ? "fa-IR" : "en-US", { dateStyle: "medium" }).format(ms);
  }

  /** Runs an action that changes the history, then reloads. */
  async function act(f: () => Promise<string | null>) {
    busy = true;
    savedTo = null;
    try {
      notice = await f();
      error = null;
      await load();
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }

  // Delete asks twice: the first click turns the button into "Delete all of
  // it?", which goes back by itself after a few seconds.
  let disarm: ReturnType<typeof setTimeout> | undefined;
  function remove(e: PlaytimeEntry) {
    if (armed !== e.key) {
      armed = e.key;
      clearTimeout(disarm);
      disarm = setTimeout(() => (armed = null), 4000);
      return;
    }
    armed = null;
    act(async () => t().play.removed(e.name, await api.playtimeForget(e.key)));
  }
  const notAGame = (e: PlaytimeEntry) => act(async () => t().play.moved(e.name, await api.moveToApps(e.key)));
  const importSteam = () =>
    act(async () => {
      const [n, secs] = await api.importSteam();
      return n ? t().play.imported(n, hm(secs)) : t().play.importedNone;
    });
  const forgetImport = () => act(async () => t().play.importRemoved(await api.forgetImported()));
  const untrack = (a: { exe: string; name: string }) => act(async () => (await api.untrackApp(a.exe), t().play.untracked(a.name)));
  const track = (a: { exe: string; name: string }) => act(async () => (await api.trackApp(a.exe, a.name), t().play.addedApp(a.name)));

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

  // "Add an app": a small picker of programs open now, plus a name field.
  let picker = $state(false);
  let pickerEl = $state<HTMLElement | null>(null);
  let open = $state<TrackedApp[] | null>(null);
  let typed = $state("");
  $effect(() => {
    if (!picker) return;
    open = null;
    api.runningPrograms().then((p) => (open = p)).catch((e) => { open = []; error = errorText(e); });
    const away = (e: PointerEvent) => { if (pickerEl && !pickerEl.contains(e.target as Node)) picker = false; };
    const esc = (e: KeyboardEvent) => { if (e.key === "Escape") picker = false; };
    document.addEventListener("pointerdown", away);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("pointerdown", away);
      document.removeEventListener("keydown", esc);
    };
  });
  function pick(a: TrackedApp) {
    picker = false;
    typed = "";
    track(a);
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

<div class="tabs" role="tablist">
  {#each ["games", "apps"] as const as id}
    <button role="tab" aria-selected={tab === id} class:active={tab === id} onclick={() => (tab = id)}>{t().play.tabs[id]}</button>
  {/each}
</div>

{#if error}<Problem text={error} />{/if}
{#if notice}<p class="note" role="status">{notice}</p>{/if}
{#if savedTo}<p class="note" role="status">{t().play.exported} <bdi dir="ltr">{savedTo}</bdi></p>{/if}

{#if tab === "games"}
  <section class="stats">
    <article class="card stat">
      <span class="label">{t().play.total[range]}</span>
      <b>{entries ? hm(gamesTotal) : "—"}</b>
    </article>
    <article class="card stat">
      <span class="label">{t().play.games}</span>
      <b>{entries ? num(games.length) : "—"}</b>
    </article>
    <article class="card stat">
      <span class="label">{t().play.top}</span>
      <b class="name" dir="auto">{top?.name ?? "—"}</b>
      {#if top}<small>{hm(top.seconds)}</small>{/if}
    </article>
  </section>

  <PlaytimeCharts {focus} onfocus={(k) => (focus = k)} />

  <div class="grid">
    <article class="card">
      <h2>{t().play.list}</h2>
      {#if games.length}
        <ol class="list">
          {#each games as e (e.key)}
            {@const pct = gamesTotal ? (e.seconds * 100) / gamesTotal : 0}
            {@const fromSteam = range === "all" ? (importedBy.get(e.key) ?? 0) : 0}
            <li>
              <div class="row1">
                <button class="game pick" class:on={focus === e.key} dir="auto" title={t().pc.only(e.name)} onclick={() => (focus = focus === e.key ? null : e.key)}>{e.name}</button>
                {#if e.key.startsWith("exe:")}<span class="chip" title={t().play.guessedWhy}>{t().play.guessed}</span>{/if}
                <span class="time">{hm(e.seconds)}</span>
                <span class="pct">{num(pct)}{t().unit.pct}</span>
              </div>
              <span class="bar" aria-hidden="true"><span style:--w={pct / 100}></span></span>
              <div class="row2">
                <span class="meta">
                  {#if e.sessions === 0}{t().play.fromSteam}{:else}{t().play.sessions(e.sessions)}{#if fromSteam}{" · "}{t().play.inclSteam(hm(fromSteam))}{/if}{/if}
                  {#if e.last_played_unix_ms}{" · "}{t().play.last} {lastPlayed(e.last_played_unix_ms)}{/if}
                </span>
                {#if e.key.startsWith("exe:")}
                  <button class="del move" onclick={() => notAGame(e)} disabled={busy} title={t().play.notGameWhy}>{t().play.notGame}</button>
                {/if}
                <button class="del" class:armed={armed === e.key} onclick={() => remove(e)} disabled={busy} title={t().play.removeWhy}>
                  <HugeIcon name="delete-02" size={13} />{armed === e.key ? t().play.confirm : t().play.remove}
                </button>
              </div>
            </li>
          {/each}
        </ol>
      {:else if entries}
        <p class="muted">{t().play.empty[range]}</p>
      {/if}
      <div class="steam">
        <button class="btn" onclick={importSteam} disabled={busy}>{t().play.importSteam}</button>
        {#if imported.length}<button class="btn" onclick={forgetImport} disabled={busy}>{t().play.forgetImport}</button>{/if}
      </div>
      <p class="small">{t().play.importWhy}</p>
      <p class="small">{t().play.how}</p>
    </article>

  </div>
{:else}
  <section class="stats">
    <article class="card stat">
      <span class="label">{t().play.appsTotal[range]}</span>
      <b>{entries ? hm(appsTotal) : "—"}</b>
    </article>
    <article class="card stat">
      <span class="label">{t().play.appsCount}</span>
      <b>{num(tracked.length)}</b>
    </article>
  </section>

  <article class="card">
    <div class="card-head">
      <h2>{t().play.appsList}</h2>
      <div class="picker" bind:this={pickerEl}>
        <button class="btn primary" onclick={() => (picker = !picker)} aria-haspopup="true" aria-expanded={picker} disabled={busy}>
          <HugeIcon name="plus-sign" size={15} />{t().play.addApp}
        </button>
        {#if picker}
          <div class="menu" role="dialog" aria-label={t().play.addApp}>
            <p class="menu-title">{t().play.openNow}</p>
            {#if open === null}
              <p class="menu-note">…</p>
            {:else if open.length}
              <ul>
                {#each open as a (a.exe)}
                  <li><button onclick={() => pick(a)}><b dir="auto">{a.name}</b><small dir="ltr">{a.exe}</small></button></li>
                {/each}
              </ul>
            {:else}
              <p class="menu-note">{t().play.noneOpen}</p>
            {/if}
            <form class="typed" onsubmit={(e) => { e.preventDefault(); if (typed.trim()) pick({ exe: typed.trim(), name: "" }); }}>
              <label for="pt-typed">{t().play.typeName}</label>
              <div>
                <input id="pt-typed" dir="ltr" bind:value={typed} placeholder={t().play.typePlaceholder} autocomplete="off" spellcheck="false" />
                <button class="btn" type="submit" disabled={!typed.trim()}>{t().play.add}</button>
              </div>
            </form>
          </div>
        {/if}
      </div>
    </div>
    {#if apps.length}
      <ol class="list">
        {#each apps as a (a.key)}
          {@const pct = appsTotal ? (a.seconds * 100) / appsTotal : 0}
          <li>
            <div class="row1">
              <span class="game" dir="auto">{a.name}</span>
              <span class="time">{hm(a.seconds)}</span>
              <span class="pct">{num(pct)}{t().unit.pct}</span>
            </div>
            <span class="bar" aria-hidden="true"><span style:--w={pct / 100}></span></span>
            <div class="row2">
              <span class="meta">
                <span class="state" class:on={a.counting}>{a.counting ? t().play.counting : t().play.notCounting}</span>
                {#if a.sessions}{" · "}{t().play.sessions(a.sessions)}{/if}
                {#if a.last_played_unix_ms}{" · "}{t().play.last} {lastPlayed(a.last_played_unix_ms)}{/if}
              </span>
              {#if a.counting}
                <button class="del" onclick={() => untrack(a)} disabled={busy}>{t().play.untrack}</button>
              {:else}
                <button class="del move" onclick={() => track(a)} disabled={busy}>{t().play.retrack}</button>
              {/if}
              {#if a.sessions}
                <button class="del" class:armed={armed === a.key} onclick={() => remove(a)} disabled={busy} title={t().play.removeWhy}>
                  <HugeIcon name="delete-02" size={13} />{armed === a.key ? t().play.confirm : t().play.remove}
                </button>
              {/if}
            </div>
          </li>
        {/each}
      </ol>
    {:else if entries}
      <p class="muted">{t().play.appsEmpty}</p>
    {/if}
    <p class="small">{t().play.appsHow}</p>
  </article>
{/if}

<style>
  .head { display: flex; align-items: center; gap: 14px; margin-bottom: 12px; flex-wrap: wrap; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 800; }
  h2 { margin: 0 0 12px; font-size: 0.98rem; font-weight: 700; }
  .badge { padding: 3px 12px; border-radius: 999px; font-size: 0.8rem; background: var(--card-2); color: var(--text-2); }
  .tools { margin-inline-start: auto; display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
  .tabs { display: flex; gap: 4px; border-bottom: 1px solid var(--line); margin-bottom: 16px; }
  .tabs button { all: unset; padding: 9px 14px; font-size: 0.9rem; color: var(--text-2); cursor: pointer; border-bottom: 2px solid transparent; margin-bottom: -1px; }
  .tabs button:hover { color: var(--text); }
  .tabs button.active { color: var(--text); border-bottom-color: var(--accent); font-weight: 600; }
  .tabs button:focus-visible { outline: 2px solid var(--accent-text); outline-offset: -2px; border-radius: 6px; }
  .note { margin: 0 0 14px; padding: 10px 16px; border-radius: var(--radius-sm); background: var(--card-2); color: var(--text-2); font-size: 0.88rem; overflow-wrap: anywhere; }
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; min-width: 0; }
  .card-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 4px; }
  .card-head h2 { margin: 0; }
  .stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 16px; margin-bottom: 16px; }
  .stat { display: grid; gap: 6px; align-content: start; }
  .stat .label { color: var(--text-2); font-size: 0.85rem; }
  .stat b { font-size: 1.55rem; font-weight: 800; }
  .stat b.name { font-size: 1.2rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .stat small { color: var(--text-2); font-size: 0.82rem; }
  .grid { display: grid; grid-template-columns: minmax(0, 1fr); gap: 16px; align-items: start; }
  .pick { all: unset; cursor: pointer; }
  .pick:hover, .pick.on { color: var(--accent-text); }
  .pick:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; border-radius: 4px; }

  .list { list-style: none; margin: 0; padding: 0; }
  .list li { padding: 12px 0; border-bottom: 1px solid var(--line); display: grid; gap: 7px; }
  .list li:first-child { padding-top: 2px; }
  .list li:last-child { border-bottom: 0; }
  .row1, .row2 { display: flex; align-items: baseline; gap: 12px; min-width: 0; }
  .row2 { flex-wrap: wrap; row-gap: 4px; }
  .game { flex: 1; min-width: 0; font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .chip { padding: 1px 8px; border-radius: 999px; background: var(--card-2); color: var(--text-3); font-size: 0.72rem; cursor: help; }
  .time { color: var(--text); font-size: 0.9rem; }
  .pct { color: var(--text-3); font-size: 0.82rem; min-width: 3.2em; text-align: end; }
  .bar { display: block; height: 6px; border-radius: 999px; background: var(--card-2); overflow: hidden; }
  .bar span { display: block; height: 100%; width: calc(var(--w) * 100%); background: var(--accent); border-radius: inherit; }
  .meta { flex: 1; color: var(--text-3); font-size: 0.78rem; min-width: 12em; }
  .state.on { color: var(--accent-text); }
  .del {
    all: unset; display: inline-flex; align-items: center; gap: 5px; padding: 3px 9px; border-radius: 8px;
    font-size: 0.78rem; color: var(--text-3); cursor: pointer; transition: background-color 0.15s ease, color 0.15s ease;
  }
  .del:hover:not(:disabled) { color: var(--danger); background: color-mix(in oklab, var(--danger) 10%, transparent); }
  .del.move:hover:not(:disabled) { color: var(--accent-text); background: var(--accent-soft); }
  .del.armed { color: var(--danger); background: color-mix(in oklab, var(--danger) 14%, transparent); font-weight: 600; }
  .del:disabled { opacity: 0.5; cursor: default; }
  .del:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  .steam { display: flex; gap: 8px; flex-wrap: wrap; margin-top: 14px; }


  .picker { position: relative; }
  .menu {
    position: absolute; top: calc(100% + 6px); inset-inline-end: 0; z-index: 20;
    width: 300px; padding: 8px; border-radius: 12px;
    background: var(--card); border: 1px solid var(--line); box-shadow: 0 12px 32px rgb(0 0 0 / 0.35);
    animation: menu-in 0.14s ease-out;
  }
  @keyframes menu-in { from { opacity: 0; transform: translateY(-4px); } }
  @media (prefers-reduced-motion: reduce) { .menu { animation: none; } }
  .menu-title { margin: 2px 6px 6px; font-size: 0.72rem; color: var(--text-3); }
  .menu-note { margin: 4px 6px 8px; font-size: 0.82rem; color: var(--text-2); }
  .menu ul { list-style: none; margin: 0; padding: 0; max-height: 260px; overflow: auto; }
  .menu li button {
    all: unset; box-sizing: border-box; width: 100%; display: flex; align-items: baseline; justify-content: space-between; gap: 10px;
    padding: 7px 8px; border-radius: 8px; cursor: pointer; color: var(--text);
  }
  .menu li button:hover { background: var(--card-2); }
  .menu li button:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .menu li b { font-weight: 600; font-size: 0.86rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .menu li small { color: var(--text-3); font-size: 0.72rem; white-space: nowrap; }
  .typed { margin-top: 8px; padding-top: 8px; border-top: 1px solid var(--line); display: grid; gap: 6px; }
  .typed label { font-size: 0.72rem; color: var(--text-3); margin: 0 6px; }
  .typed div { display: flex; gap: 6px; }
  .typed input {
    flex: 1; min-width: 0; padding: 7px 10px; border-radius: 9px; border: 1px solid var(--line);
    background: var(--card-2); color: var(--text); font: inherit; font-size: 0.85rem;
  }
  .typed input:focus-visible { outline: 2px solid var(--accent); outline-offset: 0; }

  .muted { color: var(--text-2); margin: 0; font-size: 0.9rem; }
  .small { color: var(--text-3); font-size: 0.8rem; margin: 14px 0 0; line-height: 1.55; }
  .small + .small { margin-top: 6px; }
  .btn { all: unset; display: inline-flex; align-items: center; gap: 6px; padding: 7px 12px; border-radius: 10px; background: var(--card-2); color: var(--text); font-size: 0.85rem; cursor: pointer; }
  .btn:hover:not(:disabled) { background: color-mix(in oklab, var(--card-2) 80%, var(--text) 8%); }
  .btn.primary { background: var(--accent); color: white; font-weight: 600; }
  .btn.primary:hover:not(:disabled) { background: color-mix(in oklab, var(--accent) 88%, white 12%); }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
</style>
