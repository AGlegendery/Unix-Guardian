<script lang="ts">
  import {
    Activity, Bug, Clock, Gamepad2, Layers, LayoutDashboard, Power,
    RefreshCw, Settings, Video, Wifi, Wrench,
  } from "@lucide/svelte";
  import type { Component } from "svelte";
  import { api, errorText, type Overview } from "./lib/api";
  import { locale, setLocale, t } from "./lib/i18n.svelte";
  import Dashboard from "./pages/Dashboard.svelte";
  import GameMode from "./pages/GameMode.svelte";
  import SystemControl from "./pages/SystemControl.svelte";
  import Monitoring from "./pages/Monitoring.svelte";

  type Page = keyof ReturnType<typeof t>["nav"];
  // Phoenix Guardian's order, with the Linux-only driver troubleshooter added.
  const pages: [Page, Component][] = [
    ["dashboard", LayoutDashboard],
    ["gamemode", Gamepad2],
    ["system", Power],
    ["monitoring", Activity],
    ["ping", Wifi],
    ["overlay", Layers],
    ["playtime", Clock],
    ["recorder", Video],
    ["drivers", Wrench],
    ["bugreport", Bug],
    ["updates", RefreshCw],
    ["settings", Settings],
  ];
  // Built so far; every other page says plainly that it is not.
  const built = new Set<Page>(["dashboard", "gamemode", "system", "monitoring"]);

  // `#gamemode` in the URL opens that page (handy in the browser preview).
  const fromHash = location.hash.slice(1).split("/")[0] as Page;
  let page = $state<Page>(pages.some(([p]) => p === fromHash) ? fromHash : "dashboard");
  let overview = $state<Overview | null>(null);
  let loadError = $state<string | null>(null);

  $effect(() => {
    api.overview().then((o) => (overview = o)).catch((e) => (loadError = errorText(e)));
  });
</script>

<div class="shell">
  <nav class="side" aria-label={t().app}>
    <div class="brand">
      <span class="mark" aria-hidden="true"></span>
      <span>{t().app}</span>
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
          >
            <Icon size={18} strokeWidth={1.8} aria-hidden="true" />
            <span>{t().nav[p]}</span>
          </button>
        </li>
      {/each}
    </ul>
    <div class="foot">
      {@render dot(t().status.service, overview ? !overview.daemon_error : null)}
      {@render dot(t().status.agent, overview ? !overview.agent_error : null)}
      <button class="lang" onclick={() => setLocale(locale.current === "fa" ? "en" : "fa")}>
        {locale.current === "fa" ? "English" : "فارسی"}
      </button>
    </div>
  </nav>

  <main class="content">
    {#if loadError}
      <p class="problem" role="alert">{loadError}</p>
    {/if}
    {#if page === "dashboard"}
      <Dashboard {overview} />
    {:else if page === "gamemode"}
      <GameMode {overview} />
    {:else if page === "system"}
      <SystemControl />
    {:else if page === "monitoring"}
      <Monitoring />
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
  <div class="svc">
    <span class="lamp" class:ok={on === true} class:bad={on === false} aria-hidden="true"></span>
    <span>{label}</span>
    <span class="svc-state">{on === null ? "…" : on ? t().status.running : t().status.stopped}</span>
  </div>
{/snippet}

<style>
  .shell {
    display: grid;
    grid-template-columns: 232px 1fr;
    height: 100%;
  }

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
  /* Placeholder mark until the project has a logo (09-open-questions #12). */
  .mark {
    width: 26px;
    height: 26px;
    border-radius: 8px;
    background: var(--accent);
    box-shadow: 0 4px 14px -4px var(--accent);
    position: relative;
  }
  .mark::after {
    content: "";
    position: absolute;
    inset: 8px;
    border-radius: 3px;
    background: var(--side);
  }
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
  .problem {
    margin: 0 0 18px;
    padding: 12px 16px;
    border-radius: var(--radius-sm);
    background: color-mix(in oklab, var(--warn) 12%, var(--card));
    color: var(--warn);
    font-size: 0.9rem;
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
