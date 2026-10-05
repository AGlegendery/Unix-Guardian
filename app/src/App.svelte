<script lang="ts">
  import Problem from "./lib/ui/Problem.svelte";
  import GaugeLoader from "./lib/ui/GaugeLoader.svelte";
  import Mark from "./lib/ui/Mark.svelte";
  import {
    Activity, Bug, ChevronsLeft, ChevronsRight, Clock, Gamepad2, Layers, LayoutDashboard, Power,
    Eye, FlaskConical, RefreshCw, Settings, SquareTerminal, Video, Wrench,
  } from "@lucide/svelte";
  import type { Component } from "svelte";
  import { api, errorText, type Overview } from "./lib/api";
  import { locale, setLocale, t } from "./lib/i18n.svelte";
  import Dashboard from "./pages/Dashboard.svelte";
  import GameMode from "./pages/GameMode.svelte";
  import SystemControl from "./pages/SystemControl.svelte";
  import Monitoring from "./pages/Monitoring.svelte";
  import BugReport from "./pages/BugReport.svelte";
  import TestLab from "./pages/TestLab.svelte";
  import Overlay from "./pages/Overlay.svelte";
  import Playtime from "./pages/Playtime.svelte";
  import SettingsPage from "./pages/Settings.svelte";
  import Drivers from "./pages/Drivers.svelte";
  import Recorder from "./pages/Recorder.svelte";
  import Updates from "./pages/Updates.svelte";

  type Page = keyof ReturnType<typeof t>["nav"];
  // Phoenix Guardian's order, with the Linux-only driver troubleshooter added.
  const pages: [Page, Component][] = [
    ["dashboard", LayoutDashboard],
    ["gamemode", Gamepad2],
    ["system", Power],
    ["monitoring", Activity],
    ["overlay", Layers],
    ["playtime", Clock],
    ["recorder", Video],
    ["drivers", Wrench],
    ["bugreport", Bug],
    ["testlab", FlaskConical],
    ["updates", RefreshCw],
    ["settings", Settings],
  ];
  // Built so far; every other page says plainly that it is not.
  const built = new Set<Page>(["dashboard", "gamemode", "system", "monitoring", "overlay", "playtime", "bugreport", "settings", "drivers", "recorder", "updates", "testlab"]);

  // `#gamemode` in the URL opens that page (handy in the browser preview).
  // Ping stability and internet control live in Monitoring now: old
  // `#ping` links open its tabs.
  if (location.hash.startsWith("#ping")) {
    history.replaceState(null, "", location.hash.includes("/control") ? "#monitoring/traffic" : "#monitoring/ping");
  }
  const fromHash = location.hash.slice(1).split("/")[0] as Page;
  let page = $state<Page>(pages.some(([p]) => p === fromHash) ? fromHash : "dashboard");
  // One scroll area serves every page, so a new page would open at the
  // last page's scroll position, half way down and, in WebKitGTK, stuck
  // there until resized (2026-10-04). Each page opens at its top.
  let content = $state<HTMLElement | null>(null);
  $effect(() => {
    page;
    if (content) content.scrollTop = 0;
  });
  let overview = $state<Overview | null>(null);

  // A narrow sidebar with icons only; remembered on this computer.
  let collapsed = $state(readCollapsed());
  function readCollapsed() {
    try {
      return localStorage.getItem("ug.sidebar") === "collapsed";
    } catch {
      return false;
    }
  }
  function toggleSide() {
    collapsed = !collapsed;
    try {
      localStorage.setItem("ug.sidebar", collapsed ? "collapsed" : "open");
    } catch {}
  }
  // The arrow points the way the sidebar will move, mirrored in Persian.
  const pointRight = $derived(collapsed !== (locale.current === "fa"));
  let loadError = $state<string | null>(null);

  // The intro plays once while the first readings arrive: it stays until
  // the gauge has finished drawing (it says so) and the readings are in,
  // then holds the finished mark for a moment before closing.
  let splash = $state(true);
  let introDone: () => void = () => {};
  const drawn = new Promise<void>((r) => (introDone = r));
  $effect(() => {
    const loaded = api.overview().then((o) => (overview = o)).catch((e) => (loadError = errorText(e)));
    Promise.all([drawn, loaded])
      .then(() => new Promise((r) => setTimeout(r, 200)))
      .then(() => (splash = false));
  });

  // After "Quit tray icon" the window is the place to bring it back. Read
  // when the window opens or comes back into view; no polling.
  let trayHidden = $state(false);
  $effect(() => {
    const check = () => api.trayVisible().then((v) => (trayHidden = v === false)).catch(() => {});
    const onVis = () => { if (!document.hidden) check(); };
    check();
    document.addEventListener("visibilitychange", onVis);
    return () => document.removeEventListener("visibilitychange", onVis);
  });
  function showTray() {
    api.trayShow().then(() => (trayHidden = false)).catch((e) => (loadError = errorText(e)));
  }
</script>

{#if splash}
  <div class="splash" aria-live="polite">
    <GaugeLoader variant="intro" size={112} label={t().app} ondone={() => introDone()} />
    <span>{t().app}</span>
  </div>
{/if}

<div class="shell" class:collapsed>
  <nav class="side" aria-label={t().app}>
    <div class="brand">
      <span class="mark"><Mark size={collapsed ? 30 : 28} /></span>
      {#if !collapsed}<span>{t().app}</span>{/if}
      <button
        class="fold"
        onclick={toggleSide}
        aria-expanded={!collapsed}
        aria-label={collapsed ? t().side.expand : t().side.collapse}
        title={collapsed ? t().side.expand : t().side.collapse}
      >
        {#if pointRight}<ChevronsRight size={16} aria-hidden="true" />{:else}<ChevronsLeft size={16} aria-hidden="true" />{/if}
      </button>
    </div>
    <ul>
      {#each pages as [p, Icon]}
        <li>
          <button
            class="item"
            class:active={page === p}
            class:unbuilt={!built.has(p)}
            aria-current={page === p ? "page" : undefined}
            onclick={() => (page = p)}
            title={collapsed ? t().nav[p] : undefined}
            aria-label={collapsed ? t().nav[p] : undefined}
          >
            <Icon size={18} strokeWidth={1.8} aria-hidden="true" />
            {#if !collapsed}<span>{t().nav[p]}</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
    <div class="foot">
      {@render dot(t().status.service, overview ? !overview.daemon_error : null)}
      {@render dot(t().status.agent, overview ? !overview.agent_error : null)}
      <button
        class="lang"
        onclick={() => api.openTerminal().catch((e) => (loadError = errorText(e)))}
        title={collapsed ? t().terminal : undefined}
        aria-label={collapsed ? t().terminal : undefined}
      >
        <SquareTerminal size={14} aria-hidden="true" />{#if !collapsed}{" "}{t().terminal}{/if}
      </button>
      {#if trayHidden}
        <button
          class="lang"
          onclick={showTray}
          title={collapsed ? t().showTray : undefined}
          aria-label={collapsed ? t().showTray : undefined}
        >
          <Eye size={14} aria-hidden="true" />{#if !collapsed}{" "}{t().showTray}{/if}
        </button>
      {/if}
      <button class="lang" onclick={() => setLocale(locale.current === "fa" ? "en" : "fa")}>
        {#if collapsed}{locale.current === "fa" ? "EN" : "فا"}{:else}{locale.current === "fa" ? "English" : "فارسی"}{/if}
      </button>
    </div>
  </nav>

  <main class="content" bind:this={content}>
    {#if loadError}
      <Problem text={loadError} />
    {/if}
    {#if page === "dashboard"}
      <Dashboard {overview} />
    {:else if page === "gamemode"}
      <GameMode {overview} />
    {:else if page === "system"}
      <SystemControl />
    {:else if page === "monitoring"}
      <Monitoring />
    {:else if page === "overlay"}
      <Overlay />
    {:else if page === "playtime"}
      <Playtime />
    {:else if page === "drivers"}
      <Drivers />
    {:else if page === "recorder"}
      <Recorder />
    {:else if page === "updates"}
      <Updates />
    {:else if page === "settings"}
      <SettingsPage {overview} />
    {:else if page === "bugreport"}
      <BugReport />
    {:else if page === "testlab"}
      <TestLab />
    {:else}
      <section class="unbuilt-page">
        <h1>{t().nav[page]}</h1>
        <div class="unbuilt-card">
          <span class="pill">{t().unbuilt.title}</span>
          <p>{t().unbuilt.body}</p>
        </div>
      </section>
    {/if}
  </main>
</div>

{#snippet dot(label: string, on: boolean | null)}
  {@const state = on === null ? "…" : on ? t().status.running : t().status.stopped}
  <div class="svc" title={collapsed ? `${label}: ${state}` : undefined}>
    <span class="lamp" class:ok={on === true} class:bad={on === false} aria-hidden="true"></span>
    {#if collapsed}
      <span class="sr">{label}: {state}</span>
    {:else}
      <span>{label}</span>
      <span class="svc-state">{state}</span>
    {/if}
  </div>
{/snippet}

<style>
  .shell {
    display: grid;
    grid-template-columns: 232px 1fr;
    height: 100%;
  }
  .shell.collapsed { grid-template-columns: 64px 1fr; }
  .collapsed .side { padding-inline: 10px; }
  .collapsed .brand { flex-direction: column; padding-inline: 0; gap: 12px; }
  .collapsed .item { justify-content: center; padding-inline: 0; }
  .collapsed .foot { padding-inline: 0; justify-items: center; }
  .collapsed .svc { justify-content: center; }
  .fold {
    all: unset;
    display: inline-grid;
    place-items: center;
    width: 26px;
    height: 26px;
    margin-inline-start: auto;
    border-radius: 8px;
    color: var(--text-3);
    cursor: pointer;
  }
  .collapsed .fold { margin-inline-start: 0; }
  .fold:hover { color: var(--text); background: color-mix(in oklab, var(--text) 6%, transparent); }
  .fold:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }

  .side {
    background: var(--side);
    border-inline-end: 1px solid var(--line);
    display: flex;
    flex-direction: column;
    padding: 22px 14px 16px;
    min-height: 0;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    padding-inline: 10px;
    margin-bottom: 22px;
    font-weight: 800;
    font-size: 1.08rem;
  }
  .mark { display: inline-grid; place-items: center; flex: none; }
  .splash {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 18px;
    background: var(--bg);
    font-weight: 800;
    font-size: 1.15rem;
    letter-spacing: 0.02em;
    animation: splash-text 0.6s ease-out 0.9s both;
  }
  @keyframes splash-text { from { color: transparent; } to { color: var(--text); } }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    align-content: start;
    gap: 2px;
    overflow: auto;
    flex: 1;
    min-height: 0;
  }
  .item {
    all: unset;
    box-sizing: border-box;
    width: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 9px 12px;
    border-radius: var(--radius-sm);
    color: var(--text-2);
    cursor: pointer;
    font-size: 0.94rem;
    transition: background 150ms ease-out, color 150ms ease-out;
  }
  .item:hover { background: color-mix(in oklab, var(--text) 5%, transparent); color: var(--text); }
  .item:focus-visible { outline-offset: -2px; }
  .item.active { background: var(--accent-soft); color: var(--text); font-weight: 600; }
  .item.active :global(svg) { color: var(--accent-text); }
  .item.unbuilt:not(.active) { color: var(--text-3); }

  .foot {
    border-top: 1px solid var(--line);
    padding: 14px 10px 0;
    display: grid;
    gap: 6px;
  }
  .svc { display: flex; align-items: center; gap: 8px; font-size: 0.82rem; }
  .svc-state { color: var(--text-3); margin-inline-start: auto; font-size: 0.78rem; }
  .lamp { width: 8px; height: 8px; border-radius: 50%; background: var(--text-3); flex: none; }
  .lamp.ok { background: var(--ok); }
  .lamp.bad { background: var(--danger); }
  .lang {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
    margin-top: 8px;
    color: var(--text-2);
    font-size: 0.82rem;
    width: fit-content;
  }
  .lang:hover { color: var(--text); text-decoration: underline; text-underline-offset: 4px; }

  .content {
    overflow: auto;
    padding: 26px 30px 32px;
    min-width: 0;
  }

  .unbuilt-page h1 { font-size: 1.5rem; font-weight: 800; margin: 0 0 18px; }
  .unbuilt-card {
    background: var(--card);
    border-radius: var(--radius);
    padding: 22px 24px;
    max-width: 60ch;
  }
  .unbuilt-card p { margin: 12px 0 0; color: var(--text-2); }
  .pill {
    display: inline-block;
    padding: 3px 12px;
    border-radius: 999px;
    background: var(--card-2);
    color: var(--text-2);
    font-size: 0.82rem;
  }
</style>
