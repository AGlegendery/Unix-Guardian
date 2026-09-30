<script lang="ts">
  import { Moon, Power, RotateCcw, Snowflake, Trash2 } from "@lucide/svelte";
  import type { Component } from "svelte";
  import { api, errorText, type PowerAction, type PowerCapability, type PowerJob } from "../lib/api";
  import { locale, t } from "../lib/i18n.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";

  type Tab = "command" | "conditional" | "usage" | "sleep" | "alarm";
  let tab = $state<Tab>("command");
  let caps = $state<PowerCapability[]>([]);
  let jobs = $state<PowerJob[]>([]);
  let chosen = $state<PowerAction>("power-off");
  let when = $state<"now" | "later">("now");
  let preset = $state<string>("1h");
  let clock = $state("");
  let repeat = $state<"once" | "daily" | "weekly">("once");
  let armed = $state(false); // first press of "now" arms, the second acts
  let busy = $state(false);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);

  // The four cards of the Windows app; the rarer logind actions stay in the CLI.
  const cards: [PowerAction, Component][] = [
    ["suspend", Moon],
    ["hibernate", Snowflake],
    ["power-off", Power],
    ["reboot", RotateCcw],
  ];
  const presets: Record<string, number> = { "15m": 15, "30m": 30, "1h": 60, "2h": 120 };

  const refresh = () => {
    api.powerJobs().then((j) => (jobs = j)).catch((e) => (error = errorText(e)));
  };
  $effect(() => {
    api.powerCaps().then((c) => (caps = c)).catch((e) => (error = errorText(e)));
    refresh();
    // The queue changes only through this page or the CLI; a slow refresh
    // while visible picks up the CLI and jobs that just ran.
    const id = setInterval(() => !document.hidden && refresh(), 10_000);
    return () => clearInterval(id);
  });

  const capOf = (a: PowerAction) => caps.find((c) => c.action === a);
  const destructive = $derived(chosen === "power-off" || chosen === "reboot");

  // Disarm when anything about the request changes, or after a few seconds.
  $effect(() => {
    void chosen;
    void when;
    armed = false;
  });
  $effect(() => {
    if (!armed) return;
    const id = setTimeout(() => (armed = false), 5000);
    return () => clearTimeout(id);
  });

  function targetMs(): number | null {
    if (clock) {
      const [h, m] = clock.split(":").map(Number);
      const d = new Date();
      d.setHours(h, m, 0, 0);
      if (d.getTime() <= Date.now()) d.setDate(d.getDate() + 1);
      return d.getTime();
    }
    return Date.now() + presets[preset] * 60_000;
  }

  async function act() {
    error = null;
    notice = null;
    if (when === "now") {
      if (!armed) {
        armed = true;
        return;
      }
      armed = false;
      busy = true;
      try {
        await api.powerNow(chosen);
      } catch (e) {
        error = errorText(e);
      } finally {
        busy = false;
      }
      return;
    }
    const at = targetMs();
    if (!at) return;
    busy = true;
    try {
      const job = await api.powerSchedule(chosen, at, repeat);
      notice = `${t().sys.scheduled}: ${t().sys.actions[job.action]}, ${fmt(job.at_unix_ms)}`;
      refresh();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function cancel(id: number) {
    try {
      await api.powerCancel(id);
      refresh();
    } catch (e) {
      error = errorText(e);
    }
  }

  function fmt(ms: number): string {
    return new Intl.DateTimeFormat(locale.current === "fa" ? "fa-IR" : "en-GB", {
      weekday: "short",
      hour: "2-digit",
      minute: "2-digit",
    }).format(new Date(ms));
  }
</script>

<header class="head">
  <h1>{t().nav.system}</h1>
</header>

<div class="tabs" role="tablist">
  {#each ["command", "conditional", "usage", "sleep", "alarm"] as const as id}
    <button role="tab" aria-selected={tab === id} class:active={tab === id} onclick={() => (tab = id)}>
      {t().sys.tabs[id]}
    </button>
  {/each}
</div>

{#if error}<p class="problem" role="alert">{error}</p>{/if}
{#if notice}<p class="notice" role="status">{notice}</p>{/if}

{#if tab === "command"}
  <div class="cards" role="radiogroup" aria-label={t().sys.tabs.command}>
    {#each cards as [a, Icon]}
      {@const c = capOf(a)}
      <button
        class="action"
        class:danger={a === "power-off"}
        role="radio"
        aria-checked={chosen === a}
        disabled={c ? !c.available : false}
        onclick={() => (chosen = a)}
      >
        <span class="ico"><Icon size={26} aria-hidden="true" /></span>
        <span class="name">{t().sys.actions[a]}</span>
        {#if c && !c.available}<span class="why">{a === "hibernate" ? t().sys.noHibernate : c.reason}</span>{/if}
      </button>
    {/each}
  </div>

  <section class="card">
    <div class="line">
      <span class="muted">{t().sys.when}</span>
      <Segmented
        label={t().sys.when}
        value={when}
        options={[{ value: "now", label: t().sys.now }, { value: "later", label: t().sys.later }]}
        onchange={(v) => (when = v)}
      />
    </div>

    {#if when === "later"}
      <div class="line">
        <span class="muted">{t().sys.after}</span>
        <Segmented
          label={t().sys.after}
          value={clock ? "" : preset}
          options={Object.keys(presets).map((k) => ({ value: k, label: t().sys.presets[k] }))}
          onchange={(v) => { preset = v; clock = ""; }}
        />
        <label class="muted" for="clock">{t().sys.or}</label>
        <input id="clock" type="time" bind:value={clock} />
      </div>
      <div class="line">
        <span class="muted">{t().sys.repeatLabel}</span>
        <Segmented
          label={t().sys.repeatLabel}
          value={repeat}
          options={(["once", "daily", "weekly"] as const).map((r) => ({ value: r, label: t().sys.repeat[r] }))}
          onchange={(v) => (repeat = v)}
        />
      </div>
    {/if}

    <button class="go" class:danger={destructive} class:armed disabled={busy || !capOf(chosen)?.available} onclick={act}>
      {#if when === "now"}
        {armed ? t().sys.confirm : (t().sys.verbs[chosen] ?? t().sys.actions[chosen])}
      {:else}
        {t().sys.schedule}
      {/if}
    </button>
  </section>

  <section class="card">
    <h2>{t().sys.queued}</h2>
    {#if jobs.length}
      <ul class="jobs">
        {#each jobs as j (j.id)}
          <li>
            <span><b>{t().sys.actions[j.action] ?? j.action}</b> {fmt(j.at_unix_ms)}</span>
            <span class="chip">{t().sys.repeat[j.repeat]}</span>
            <button class="cancel" onclick={() => cancel(j.id)}><Trash2 size={15} aria-hidden="true" />{t().sys.cancel}</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted">{t().sys.none}</p>
    {/if}
    <p class="small">{t().sys.missed}</p>
    <p class="small ltr-mix">{t().sys.cli}</p>
  </section>
{:else}
  <section class="card">
    <span class="chip">{t().unbuilt.title}</span>
    <p class="muted">{t().unbuilt.body}</p>
  </section>
{/if}

<style>
  .head { margin-bottom: 16px; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 800; }
  h2 { margin: 0 0 12px; font-size: 1rem; font-weight: 700; }
  .tabs { display: flex; gap: 4px; border-bottom: 1px solid var(--line); margin-bottom: 16px; flex-wrap: wrap; }
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

  .problem, .notice { margin: 0 0 14px; padding: 12px 16px; border-radius: var(--radius-sm); font-size: 0.9rem; }
  .problem { background: color-mix(in oklab, var(--danger) 12%, var(--card)); color: var(--danger); }
  .notice { background: color-mix(in oklab, var(--ok) 12%, var(--card)); color: var(--ok); }

  .cards { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 14px; }
  @media (max-width: 900px) { .cards { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
  .action {
    all: unset;
    box-sizing: border-box;
    background: var(--card);
    border-radius: var(--radius);
    padding: 20px 16px;
    display: grid;
    justify-items: center;
    gap: 10px;
    cursor: pointer;
    border: 1.5px solid transparent;
    text-align: center;
    transition: border-color 150ms ease-out, background 150ms ease-out;
  }
  .action:hover:not(:disabled) { background: color-mix(in oklab, var(--text) 4%, var(--card)); }
  .action[aria-checked="true"] { border-color: var(--accent); background: var(--accent-soft); }
  .action.danger[aria-checked="true"] { border-color: var(--danger); background: color-mix(in oklab, var(--danger) 14%, var(--card)); }
  .action:disabled { cursor: not-allowed; opacity: 0.55; }
  .action:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .action .ico {
    width: 52px;
    height: 52px;
    border-radius: 14px;
    display: grid;
    place-items: center;
    background: var(--card-2);
    color: var(--text-2);
  }
  .action[aria-checked="true"] .ico { color: var(--accent-text); }
  .action.danger[aria-checked="true"] .ico { color: var(--danger); }
  .name { font-weight: 700; }
  .why { font-size: 0.76rem; color: var(--text-3); line-height: 1.5; }

  .card { background: var(--card); border-radius: var(--radius); padding: 20px 22px; margin-top: 16px; }
  .line { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; margin-bottom: 14px; }
  .muted { color: var(--text-3); font-size: 0.86rem; margin: 0; }
  input[type="time"] {
    font: inherit;
    color: var(--text);
    background: var(--card-2);
    border: 1px solid var(--line);
    border-radius: 9px;
    padding: 5px 10px;
    color-scheme: dark;
  }
  .go {
    all: unset;
    box-sizing: border-box;
    width: 100%;
    text-align: center;
    padding: 14px;
    border-radius: 12px;
    font-weight: 700;
    font-size: 1rem;
    background: var(--accent);
    color: #fff;
    cursor: pointer;
    transition: filter 150ms ease-out;
  }
  .go.danger { background: #dc2626; }
  .go.armed { box-shadow: 0 0 0 3px color-mix(in oklab, var(--danger) 45%, transparent); }
  .go:hover:not(:disabled) { filter: brightness(1.08); }
  .go:disabled { opacity: 0.5; cursor: not-allowed; }
  .go:focus-visible { outline: 2px solid var(--text); outline-offset: 2px; }

  .jobs { list-style: none; margin: 0; padding: 0; display: grid; gap: 8px; }
  .jobs li {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    border-radius: 12px;
    background: var(--card-2);
  }
  .jobs li > span:first-child { flex: 1; }
  .chip { padding: 3px 12px; border-radius: 999px; background: var(--card-2); color: var(--text-2); font-size: 0.8rem; }
  .jobs .chip { background: var(--card); }
  .cancel {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--danger);
    font-size: 0.86rem;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: 8px;
  }
  .cancel:hover { background: color-mix(in oklab, var(--danger) 12%, transparent); }
  .cancel:focus-visible { outline: 2px solid var(--danger); }
  .small { margin: 12px 0 0; color: var(--text-3); font-size: 0.8rem; max-width: 80ch; }
</style>
