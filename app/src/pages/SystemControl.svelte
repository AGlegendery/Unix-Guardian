<script lang="ts">
  import { AlarmClock, Cpu, Moon, Power, RotateCcw, Snowflake, Trash2 } from "@lucide/svelte";
  import type { Component } from "svelte";
  import { api, errorText, type PowerUsage, type PowerAction, type PowerCapability, type PowerJob, type PowerWatch, type WatchKind } from "../lib/api";
  import { num } from "../lib/format";
  import { locale, t } from "../lib/i18n.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";

  type Tab = "command" | "conditional" | "usage" | "sleep" | "alarm";
  const tabs: Tab[] = ["command", "conditional", "usage", "sleep", "alarm"];
  const hashTab = location.hash.split("/")[1] as Tab;
  let tab = $state<Tab>(tabs.includes(hashTab) ? hashTab : "command");
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
  let alarm = $state(0);
  let alarmSupported = $state(true);
  let alarmClock = $state("07:00");
  let firmwareArmed = $state(false);
  let watches = $state<PowerWatch[]>([]);
  let condAction = $state<PowerAction>("power-off");
  let condKind = $state<WatchKind>("net-idle");
  let condBelow = $state(50);
  let condMinutes = $state(5);
  let condName = $state("");
  let usage = $state<PowerUsage | null>(null);
  let tariff = $state(readTariff());

  function readTariff(): number {
    try {
      return Number(localStorage.getItem("ug.tariff")) || 0;
    } catch {
      return 0;
    }
  }
  $effect(() => {
    try {
      localStorage.setItem("ug.tariff", String(tariff || ""));
    } catch {
      /* not remembered; still used for this session */
    }
  });
  // Live draw while the tab is open, every 5 s (one D-Bus call).
  $effect(() => {
    if (tab !== "usage") return;
    const load = () => !document.hidden && api.powerUsage().then((u) => (usage = u)).catch((e) => (error = errorText(e)));
    load();
    const id = setInterval(load, 5000);
    return () => clearInterval(id);
  });
  const last7 = $derived((usage?.days ?? []).slice(-7));
  const weekWh = $derived(last7.reduce((a, d) => a + d.cpu_wh, 0));
  const todayWh = $derived(last7.at(-1)?.cpu_wh ?? 0);
  const maxWh = $derived(Math.max(1, ...last7.map((d) => d.cpu_wh)));
  function dayName(date: string): string {
    return new Intl.DateTimeFormat(locale.current === "fa" ? "fa-IR" : "en-GB", { weekday: "short" }).format(new Date(date + "T12:00"));
  }

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
    api.powerWatches().then((w) => (watches = w)).catch(() => {});
    api.wakeAlarm().then(([a, ok]) => { alarm = a; alarmSupported = ok; }).catch(() => {});
  };

  function nextClock(hhmm: string): number {
    const [h, m] = hhmm.split(":").map(Number);
    const d = new Date();
    d.setHours(h, m, 0, 0);
    if (d.getTime() <= Date.now()) d.setDate(d.getDate() + 1);
    return d.getTime();
  }

  async function run(f: () => Promise<unknown>, ok?: string) {
    error = null;
    notice = null;
    busy = true;
    try {
      await f();
      if (ok) notice = ok;
      refresh();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  const setAlarm = () => run(() => api.setWakeAlarm(nextClock(alarmClock)), `${t().sys.alarmNow} ${fmt(nextClock(alarmClock))}`);
  const sleepUntil = () => run(async () => {
    await api.setWakeAlarm(nextClock(alarmClock));
    await api.powerNow("suspend");
  });
  // net-idle threshold is entered in kB/s; the daemon takes bytes per second.
  const arm = () =>
    run(() =>
      api.powerWhen(
        condAction,
        condKind,
        condKind === "net-idle" ? condBelow * 1000 : condBelow,
        condMinutes,
        condName.trim(),
      ),
    );
  function describe(w: PowerWatch): string {
    return (t().sys.cond.desc[w.kind] ?? w.kind)
      .replace("{t}", num(w.kind === "net-idle" ? w.threshold / 1000 : w.threshold))
      .replace("{m}", num(w.minutes))
      .replace("{n}", w.name);
  }
  $effect(() => {
    void condKind;
    condBelow = condKind === "net-idle" ? 50 : 10;
  });

  function firmware() {
    if (!firmwareArmed) {
      firmwareArmed = true;
      setTimeout(() => (firmwareArmed = false), 5000);
      return;
    }
    firmwareArmed = false;
    run(() => api.powerNow("reboot-firmware"));
  }
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

  {@const fw = capOf("reboot-firmware")}
  <section class="card row">
    <span class="row-ico"><Cpu size={20} aria-hidden="true" /></span>
    <div class="grow">
      <p class="row-title">{t().sys.firmware}</p>
      <p class="small">{fw && !fw.available ? fw.reason : t().sys.firmwareHint}</p>
    </div>
    <button class="secondary" class:armed={firmwareArmed} disabled={busy || !fw?.available} onclick={firmware}>
      {firmwareArmed ? t().sys.confirm : t().sys.actions.reboot}
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
{:else if tab === "conditional"}
  <section class="card">
    <div class="line">
      <span class="muted">{t().sys.cond.then}</span>
      <Segmented
        label={t().sys.cond.then}
        value={condAction}
        options={(["power-off", "reboot", "suspend"] as const).map((a) => ({ value: a, label: t().sys.actions[a], disabled: !capOf(a)?.available }))}
        onchange={(v) => (condAction = v)}
      />
    </div>
    <p class="muted">{t().sys.cond.when}</p>
    <div class="kinds" role="radiogroup" aria-label={t().sys.cond.when}>
      {#each ["net-idle", "cpu-idle", "process-gone", "game-over"] as const as k}
        <button class="kind" role="radio" aria-checked={condKind === k} onclick={() => (condKind = k)}>{t().sys.cond.kinds[k]}</button>
      {/each}
    </div>
    {#if condKind === "net-idle" || condKind === "cpu-idle"}
      <div class="line">
        <label class="muted" for="below">{t().sys.cond.below}</label>
        <input id="below" class="num" type="number" min="1" bind:value={condBelow} />
        <span class="muted">{condKind === "net-idle" ? "kB/s" : "%"}</span>
        <label class="muted" for="mins">{t().sys.cond.forMin}</label>
        <input id="mins" class="num" type="number" min="1" max="1440" bind:value={condMinutes} />
      </div>
    {:else if condKind === "process-gone"}
      <div class="line">
        <label class="muted" for="prog">{t().sys.cond.program}</label>
        <input id="prog" class="text" dir="ltr" bind:value={condName} placeholder="blender" />
        <span class="small">{t().sys.cond.programHint}</span>
      </div>
    {/if}
    <p class="small">{t().sys.cond.explain}</p>
    <button class="go" disabled={busy || (condKind === "process-gone" && !condName.trim())} onclick={arm}>{t().sys.cond.arm}</button>
  </section>

  <section class="card">
    <h2>{t().sys.cond.armed}</h2>
    {#if watches.length}
      <ul class="jobs">
        {#each watches as w (w.id)}
          <li>
            <span><b>{t().sys.actions[w.action] ?? w.action}</b> {describe(w)}</span>
            <button class="cancel" onclick={() => run(() => api.powerUnwatch(w.id))}><Trash2 size={15} aria-hidden="true" />{t().sys.cond.remove}</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted">{t().sys.cond.none}</p>
    {/if}
  </section>
{:else if tab === "usage"}
  <section class="card">
    <h2>{t().sys.use.now}</h2>
    <div class="meters">
      <div><span class="muted">{t().sys.use.cpu}</span><span class="big">{num(usage?.cpu_w ?? null)} <small>{t().unit.w}</small></span></div>
      <div><span class="muted">{t().sys.use.gpu}</span><span class="big">{num(usage?.gpu_w ?? null)} <small>{t().unit.w}</small></span></div>
      <div>
        <span class="muted">{t().sys.use.battery}</span>
        {#if usage?.battery_w != null}
          <span class="big">{num(usage.battery_w)} <small>{t().unit.w}</small></span>
        {:else}
          <span class="small">{t().sys.use.batteryAc}</span>
        {/if}
      </div>
    </div>
  </section>

  <section class="card">
    <h2>{t().sys.use.week}</h2>
    {#if usage && !usage.cpu_energy_recorded}
      <p class="muted">{t().sys.use.none}</p>
    {:else if last7.length}
      <div class="bars" role="img" aria-label={t().sys.use.week}>
        {#each last7 as d, i (d.date)}
          <div class="bar-col" title={`${d.date}: ${num(d.cpu_wh)} ${t().sys.use.wh}`}>
            {#if i === last7.length - 1 || d.cpu_wh === maxWh}<span class="bar-val">{num(d.cpu_wh)}</span>{/if}
            <span class="bar-fill" style:--h={d.cpu_wh / maxWh}></span>
            <span class="bar-day">{i === last7.length - 1 ? t().sys.use.today : dayName(d.date)}</span>
          </div>
        {/each}
      </div>
      <div class="meters">
        <div><span class="muted">{t().sys.use.today}</span><span class="big">{num(todayWh / 1000, 2)} <small>{t().sys.use.kwh}</small></span></div>
        <div><span class="muted">{t().sys.use.weekTotal}</span><span class="big">{num(weekWh / 1000, 2)} <small>{t().sys.use.kwh}</small></span></div>
        <div>
          <label class="muted" for="tariff">{t().sys.use.tariff} ({t().sys.use.currency})</label>
          <input id="tariff" class="num wide" type="number" min="0" bind:value={tariff} />
        </div>
        {#if tariff > 0}
          <div><span class="muted">{t().sys.use.cost}</span><span class="big">{num((weekWh / 1000) * tariff)} <small>{t().sys.use.currency}</small></span></div>
        {/if}
      </div>
    {/if}
    <p class="small">{t().sys.use.honest}</p>
  </section>
{:else if tab === "sleep" || tab === "alarm"}
  <section class="card">
    <h2><AlarmClock size={18} aria-hidden="true" /> {tab === "sleep" ? t().sys.sleepWake : t().sys.alarmTitle}</h2>
    {#if alarmSupported}
      <div class="line">
        <input type="time" bind:value={alarmClock} aria-label={t().sys.atTime} />
        {#if tab === "sleep"}
          <button class="secondary" disabled={busy || !capOf("suspend")?.available} onclick={sleepUntil}>{t().sys.sleepWakeGo}</button>
        {:else}
          <button class="secondary" disabled={busy} onclick={setAlarm}>{t().sys.alarmSet}</button>
        {/if}
      </div>
      <p class="muted">
        {#if alarm}
          {t().sys.alarmNow} <b>{fmt(alarm)}</b>
          <button class="cancel" onclick={() => run(() => api.clearWakeAlarm())}><Trash2 size={15} aria-hidden="true" />{t().sys.alarmClear}</button>
        {:else}
          {t().sys.alarmNone}
        {/if}
      </p>
      <p class="small">{t().sys.alarmHint}</p>
    {:else}
      <p class="muted">{t().sys.alarmUnsupported}</p>
    {/if}
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
  .meters { display: flex; flex-wrap: wrap; gap: 14px 40px; margin-top: 6px; }
  .meters > div { display: grid; gap: 2px; }
  .big { font-size: 1.6rem; font-weight: 700; }
  .big small { font-size: 0.55em; color: var(--text-2); font-weight: 500; }
  input.num.wide { width: 8em; }
  .bars { display: flex; align-items: flex-end; gap: 14px; height: 170px; margin: 8px 0 18px; }
  .bar-col { flex: 1; max-width: 56px; height: 100%; display: flex; flex-direction: column; justify-content: flex-end; align-items: center; gap: 6px; }
  .bar-fill { width: 24px; height: calc(var(--h) * 120px); min-height: 2px; background: var(--accent); border-radius: 4px 4px 0 0; }
  .bar-val { font-size: 0.75rem; color: var(--text-2); }
  .bar-day { font-size: 0.75rem; color: var(--text-3); }
  .kinds { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 10px; margin: 8px 0 14px; }
  @media (max-width: 900px) { .kinds { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
  .kind {
    all: unset;
    box-sizing: border-box;
    padding: 14px;
    border-radius: 12px;
    background: var(--card-2);
    border: 1.5px solid transparent;
    text-align: center;
    cursor: pointer;
    font-size: 0.9rem;
  }
  .kind[aria-checked="true"] { border-color: var(--accent); background: var(--accent-soft); font-weight: 600; }
  .kind:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  input.num, input.text {
    font: inherit;
    color: var(--text);
    background: var(--card-2);
    border: 1px solid var(--line);
    border-radius: 9px;
    padding: 5px 10px;
  }
  input.num { width: 5.5em; }
  input.text { width: 12em; }
  .row { display: flex; align-items: center; gap: 14px; }
  .row-ico { width: 40px; height: 40px; border-radius: 12px; display: grid; place-items: center; background: var(--card-2); color: var(--text-2); flex: none; }
  .grow { flex: 1; min-width: 0; }
  .row-title { margin: 0; font-weight: 600; }
  .row .small { margin: 2px 0 0; }
  h2 { display: flex; align-items: center; gap: 8px; }
  .secondary {
    all: unset;
    padding: 8px 16px;
    border-radius: 10px;
    background: var(--card-2);
    border: 1px solid var(--line);
    color: var(--text);
    font-size: 0.9rem;
    cursor: pointer;
    white-space: nowrap;
  }
  .secondary:hover:not(:disabled) { border-color: var(--accent); }
  .secondary:disabled { opacity: 0.5; cursor: not-allowed; }
  .secondary.armed { border-color: var(--danger); color: var(--danger); }
  .secondary:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .small { margin: 12px 0 0; color: var(--text-3); font-size: 0.8rem; max-width: 80ch; }
</style>
