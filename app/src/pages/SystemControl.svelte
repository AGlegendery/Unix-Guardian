<script lang="ts">
  import Problem from "../lib/ui/Problem.svelte";
  import HugeIcon from "../lib/icons/HugeIcon.svelte";
  import BatteryChart from "../lib/BatteryChart.svelte";
  import { api, errorText, type BatteryReport, type ButtonId, type SleepSettings, type BatteryCare, type CoolingState, type PowerUsage, type PowerAction, type PowerCapability, type PowerJob, type PowerWatch, type WatchKind } from "../lib/api";
  import { num } from "../lib/format";
  import { locale, t } from "../lib/i18n.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";
  import Switch from "../lib/ui/Switch.svelte";
  import TimePicker from "../lib/ui/TimePicker.svelte";

  type Tab = "command" | "conditional" | "usage" | "battery" | "cooling" | "sleep";
  const tabs: Tab[] = ["command", "conditional", "usage", "battery", "cooling", "sleep"];
  // The wake alarm had a tab of its own; it is part of Sleep and wake now.
  const hashTab = (location.hash.split("/")[1] === "alarm" ? "sleep" : location.hash.split("/")[1]) as Tab;
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
  let slp = $state<SleepSettings | null>(null);
  $effect(() => {
    if (tab !== "sleep") return;
    api.sleepSettings().then((v) => (slp = v)).catch((e) => (error = errorText(e)));
  });
  const slpRun = (f: () => Promise<void>) => run(async () => { await f(); slp = await api.sleepSettings(); }, t().sys.slp.saved);
  // A clock that ticks only while an alarm is shown, once a minute.
  let nowMs = $state(Date.now());
  $effect(() => {
    if (tab !== "sleep" || !alarm) return;
    const id = setInterval(() => (nowMs = Date.now()), 30_000);
    return () => clearInterval(id);
  });
  function inTime(at: number): string {
    const mins = Math.max(0, Math.round((at - nowMs) / 60_000));
    return t().sys.slp.inTime.replace("{h}", num(Math.floor(mins / 60))).replace("{m}", num(mins % 60));
  }
  function quickAlarm(mins: number) {
    const d = new Date(Date.now() + mins * 60_000);
    alarmClock = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  }
  let bat = $state<BatteryCare | null>(null);
  let batLimit = $state(80);
  // Charge and use: the agent's once-a-minute record, refreshed each minute.
  let histRange = $state<"today" | "day" | "week">("today");
  let rep = $state<BatteryReport | null>(null);
  let span = $state<[number, number]>([0, 0]);
  $effect(() => {
    if (tab !== "battery") return;
    const r = histRange;
    const load = () => {
      if (document.hidden) return;
      const until = Date.now();
      const midnight = new Date();
      midnight.setHours(0, 0, 0, 0);
      const since = r === "today" ? midnight.getTime() : until - (r === "day" ? 1 : 7) * 86_400_000;
      api.batteryReport(since, until).then((v) => { rep = v; span = [since, until]; }).catch((e) => (error = errorText(e)));
    };
    load();
    const id = setInterval(load, 60_000);
    return () => clearInterval(id);
  });
  const two = $derived(new Intl.NumberFormat(locale.current === "fa" ? "fa-IR" : "en-GB", { minimumIntegerDigits: 2 }));
  const hm = (min: number) => `${num(Math.floor(min / 60))}:${two.format(min % 60)}`;
  const fmtClock = $derived(new Intl.DateTimeFormat(locale.current === "fa" ? "fa-IR" : "en-GB", { hour: "2-digit", minute: "2-digit" }));
  const fmtDayClock = $derived(new Intl.DateTimeFormat(locale.current === "fa" ? "fa-IR" : "en-GB", { weekday: "short", hour: "2-digit", minute: "2-digit" }));
  // The headline figure: the gauge when it is sane, else the measurement.
  const headline = $derived(rep?.health.find((h) => h.method === "fuel-gauge" && h.confidence === "high") ?? rep?.health.find((h) => h.method === "measured") ?? rep?.health[0] ?? null);
  function verdict(pct: number): string {
    const v = t().sys.hist.verdict;
    return pct >= 85 ? v.good : pct >= 70 ? v.fair : pct >= 55 ? v.poor : v.bad;
  }
  $effect(() => {
    if (tab !== "battery") return;
    const load = () => !document.hidden && api.battery().then((b) => { bat = b; if (b.mode !== "off") batLimit = b.limit; }).catch((e) => (error = errorText(e)));
    load();
    const id = setInterval(load, 15000);
    return () => clearInterval(id);
  });
  // Cooling: the power profile, the firmware's fan modes and fan curves.
  let cool = $state<CoolingState | null>(null);
  let ppd = $state("");
  $effect(() => {
    if (tab !== "cooling") return;
    const load = () => {
      if (document.hidden) return;
      api.coolingState().then((c) => (cool = c)).catch((e) => (error = errorText(e)));
      api.powerProfile().then((p) => (ppd = p)).catch(() => {});
    };
    load();
    const id = setInterval(load, 3000);
    return () => clearInterval(id);
  });
  const FAN_PRESETS: Record<string, [number, number][]> = {
    quiet: [[40, 20], [60, 35], [75, 60], [85, 100]],
    balanced: [[40, 25], [60, 45], [75, 70], [85, 100]],
    cool: [[40, 35], [55, 55], [70, 85], [80, 100]],
  };
  const coolRun = (f: () => Promise<void>) => run(async () => { await f(); cool = await api.coolingState(); ppd = await api.powerProfile(); });
  const setBat = (mode: string) => run(async () => { await api.setBatteryCare(mode, batLimit); bat = await api.battery(); });
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
  const cards: [PowerAction, string][] = [
    ["suspend", "moon-02"],
    ["hibernate", "snow"],
    ["power-off", "shut-down"],
    ["reboot", "refresh"],
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
    await doNow("suspend", true);
  });

  // Another program keeping the computer awake (a block inhibitor): who,
  // in plain words, and whether to go ahead. `alarmFor`: a wake alarm set
  // just for this sleep, removed again on Cancel.
  type Blocked = { action: PowerAction; list: [string, string][]; alarmFor: boolean; notLocked: boolean };
  let blocked = $state<Blocked | null>(null);
  const isSleep = (a: PowerAction) => !["power-off", "reboot", "reboot-firmware"].includes(a);
  function blockerText([who, why]: [string, string]): string {
    const w = why.toLowerCase();
    if (w.includes("caffeine")) return t().sys.blocked.caffeine(who);
    if (w.includes("lock") && (w.includes("suspend") || w.includes("sleep"))) return t().sys.blocked.lockRoute(who);
    return why ? `${who}: ${why}` : who;
  }
  async function doNow(action: PowerAction, alarmFor = false) {
    const r = await api.powerNow(action);
    if (r === "blocked") {
      blocked = { action, list: await api.powerBlockers(action).catch(() => []), alarmFor, notLocked: false };
      setTimeout(() => document.querySelector(".blocked")?.scrollIntoView({ block: "center" }));
    }
  }
  async function goAnyway(lock: boolean) {
    if (!blocked) return;
    const b = blocked;
    busy = true;
    error = null;
    try {
      const r = await api.powerNowAnyway(b.action, lock);
      blocked = r === "not-locked" ? { ...b, notLocked: true } : null;
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
  async function cancelBlocked() {
    const b = blocked;
    blocked = null;
    if (b?.alarmFor) {
      await api.clearWakeAlarm().catch(() => {});
      notice = t().sys.blocked.cancelled;
      refresh();
    }
  }
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
    run(() => doNow("reboot-firmware"));
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
        await doNow(chosen);
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
  {#each tabs as id}
    <button role="tab" aria-selected={tab === id} class:active={tab === id} onclick={() => (tab = id)}>
      {t().sys.tabs[id]}
    </button>
  {/each}
</div>

{#if error}<Problem text={error} />{/if}
{#if notice}<p class="notice" role="status">{notice}</p>{/if}
{#if blocked}
  <section class="card blocked" data-cat="danger" aria-live="assertive" aria-labelledby="blocked-title">
    <h2 id="blocked-title">{t().sys.blocked.title}</h2>
    <ul>
      {#each blocked.list as b}<li>{blockerText(b)}</li>{/each}
    </ul>
    {#if blocked.notLocked}
      <p class="warn-line">{t().sys.blocked.notLocked}</p>
      <div class="blocked-actions">
        <button class="go danger" disabled={busy} onclick={() => goAnyway(false)}>{t().sys.blocked.noLock}</button>
        <button class="secondary" disabled={busy} onclick={cancelBlocked}>{t().sys.blocked.cancel}</button>
      </div>
    {:else}
      <p class="small">{isSleep(blocked.action) ? t().sys.blocked.sleepNote : t().sys.blocked.shutdownNote}</p>
      <div class="blocked-actions">
        <button class="go" disabled={busy} onclick={() => goAnyway(true)}>{isSleep(blocked.action) ? t().sys.blocked.anyway : t().sys.blocked.anywayShutdown}</button>
        <button class="secondary" disabled={busy} onclick={cancelBlocked}>{t().sys.blocked.cancel}</button>
      </div>
    {/if}
  </section>
{/if}

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
        <span class="ico"><HugeIcon name={Icon} size={26} /></span>
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
        <span class="muted">{t().sys.or}</span>
        {#if clock}
          <TimePicker bind:value={clock} label={t().sys.atTime} />
        {:else}
          <button class="chip-btn" onclick={() => { const d = new Date(Date.now() + 3_600_000); clock = `${String(d.getHours()).padStart(2, "0")}:00`; }}>{t().sys.atTime}…</button>
        {/if}
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
    <span class="row-ico"><HugeIcon name="cpu" size={20} /></span>
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
            <button class="cancel" onclick={() => cancel(j.id)}><HugeIcon name="delete-02" size={15} />{t().sys.cancel}</button>
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
            <button class="cancel" onclick={() => run(() => api.powerUnwatch(w.id))}><HugeIcon name="delete-02" size={15} />{t().sys.cond.remove}</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted">{t().sys.cond.none}</p>
    {/if}
  </section>
{:else if tab === "battery"}
  {#if rep && rep.has_battery}
    <section class="card">
      <div class="card-head">
        <h2><HugeIcon name="history" size={18} /> {t().sys.hist.title}</h2>
        <Segmented
          label={t().sys.hist.title}
          value={histRange}
          options={(["today", "day", "week"] as const).map((r) => ({ value: r, label: t().sys.hist.ranges[r] }))}
          onchange={(r) => (histRange = r)}
        />
      </div>
      {#if rep.points.length < 2}
        <p class="muted">{t().sys.hist.none}</p>
      {:else}
        <BatteryChart report={rep} since={span[0]} until={span[1]} />
        <div class="tiles">
          <div class="tile"><span class="muted">{t().sys.hist.onBattery}</span><span class="big">{hm(rep.on_battery_min)}</span></div>
          <div class="tile"><span class="muted">{t().sys.hist.onAc}</span><span class="big">{hm(rep.on_ac_min)}</span></div>
          <div class="tile"><span class="muted">{t().sys.hist.screen}</span><span class="big">{hm(rep.screen_on_battery_min)}</span></div>
          <div class="tile"><span class="muted">{t().sys.hist.avgW}</span><span class="big">{num(rep.avg_battery_w, 1)} <small>{t().unit.w}</small></span></div>
          <div class="tile"><span class="muted">{t().sys.hist.used}</span><span class="big">{num(rep.used_wh, 1)} <small>Wh · {num(rep.used_pct)}{t().unit.pct}</small></span></div>
          {#if rep.full_runtime_min > 0}
            <div class="tile"><span class="muted">{t().sys.hist.runtime}</span><span class="big">{hm(rep.full_runtime_min)}</span><span class="small tight">{t().sys.hist.runtimeHint}</span></div>
          {/if}
        </div>
        <ul class="periods" aria-label={t().sys.hist.periods}>
          {#each rep.periods.filter((p) => p.end_unix_ms - p.start_unix_ms >= 60_000).slice().reverse() as p}
            <li class={p.kind}>
              <span class="swatch" aria-hidden="true"></span>
              <span class="when" dir="ltr">{(histRange === "week" ? fmtDayClock : fmtClock).format(new Date(p.start_unix_ms))} – {fmtClock.format(new Date(p.end_unix_ms))}</span>
              <b>{t().sys.hist[p.kind === "battery" ? "battery" : p.kind === "ac" ? "ac" : "off"]}</b>
              {#if p.kind !== "off" && p.start_pct != null && p.end_pct != null}
                <span class="muted">{num(p.start_pct)}{t().unit.pct} {locale.current === "fa" ? "←" : "→"} {num(p.end_pct)}{t().unit.pct}</span>
              {/if}
              {#if p.kind === "battery" && p.avg_w != null}<span class="muted">{num(p.avg_w, 1)} {t().unit.w}</span>{/if}
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section class="card">
      <h2>{t().sys.hist.health}</h2>
      {#if headline && headline.pct != null}
        <div class="health-top">
          <div class="ring" style:--p={Math.min(100, headline.pct)}>
            <span>{num(Math.round(headline.pct))}<small>{t().unit.pct}</small></span>
          </div>
          <div>
            <p class="verdict">{verdict(headline.pct)}</p>
            {#if rep.full_wh != null && rep.design_wh != null}
              <p class="muted">{t().sys.hist.capacity}: <b dir="ltr">{num(rep.full_wh, 1)} Wh</b> · {t().sys.hist.design} <span dir="ltr">{num(rep.design_wh, 1)} Wh</span></p>
            {/if}
            {#if rep.cycles > 0}<p class="muted">{t().sys.hist.cycles}: <b>{num(rep.cycles)}</b></p>{/if}
          </div>
        </div>
      {/if}
      <p class="small intro">{t().sys.hist.healthIntro}</p>
      <div class="methods">
        {#each rep.health as h}
          <div class="method">
            <div class="method-top">
              <b>{t().sys.hist.methods[h.method]}</b>
              <span class="chip conf {h.confidence}">{t().sys.hist.conf[h.confidence]}</span>
              <span class="big">{h.pct == null ? "—" : `${num(Math.round(h.pct))}${t().unit.pct}`}</span>
            </div>
            <div class="meter" aria-hidden="true"><span style:--w={Math.min(100, h.pct ?? 0)}></span></div>
            <p class="small tight" dir="auto">{h.basis}</p>
          </div>
        {/each}
        {#if !rep.health.some((h) => h.method === "measured")}
          <div class="method">
            <div class="method-top"><b>{t().sys.hist.methods.measured}</b></div>
            <p class="small tight">{t().sys.hist.noMeasure}</p>
          </div>
        {/if}
      </div>
    </section>
  {/if}
  <section class="card">
    <h2><HugeIcon name="battery-charging-01" size={18} trigger={bat?.mode} /> {t().sys.bat.title}</h2>
    <p class="small intro">{t().sys.bat.intro}</p>
    {#if bat && bat.capacity > 100}
      <p class="muted">{t().sys.bat.noBattery}</p>
    {:else if bat}
      <div class="meters">
        <div><span class="muted">{t().sys.bat.charge}</span><span class="big">{num(bat.capacity)}<small>{t().unit.pct}</small></span><span class="small">{t().sys.bat.status[bat.status] ?? bat.status}</span></div>
      </div>
      {#if !bat.supported}
        <p class="muted">{bat.reason}</p>
      {:else}
        <div class="modes" role="radiogroup" aria-label={t().sys.bat.title}>
          {#each ["off", "protect", "full-once"] as const as m}
            <button class="mode" role="radio" aria-checked={bat.mode === m} disabled={busy} onclick={() => bat?.mode !== m && setBat(m)}>
              <span class="mode-top"><span class="dot" aria-hidden="true"></span><b>{t().sys.bat.modes[m]}</b></span>
              <span class="mode-help">{t().sys.bat.modeHelp[m]}</span>
            </button>
          {/each}
        </div>
        {#if bat.adjustable}
          <div class="line">
            <label class="muted" for="limit">{t().sys.bat.limit}</label>
            <input id="limit" type="range" min="50" max="100" step="5" bind:value={batLimit} onchange={() => bat?.mode !== "off" && setBat(bat!.mode)} />
            <span class="big">{num(batLimit)}<small>{t().unit.pct}</small></span>
          </div>
        {:else}
          <p class="small">{t().sys.bat.vendor}</p>
        {/if}
      {/if}
    {/if}
  </section>
{:else if tab === "cooling"}
  <section class="card">
    <h2>{t().sys.cool.profile}</h2>
    {#if ppd}
      <Segmented
        label={t().sys.cool.profile}
        value={ppd}
        options={(["power-saver", "balanced", "performance"] as const).map((p) => ({ value: p, label: t().sys.cool.profiles[p] }))}
        onchange={(p) => coolRun(() => api.setPowerProfile(p))}
      />
      <p class="small">{t().sys.cool.profileHint}</p>
    {:else}
      <p class="muted">{t().sys.cool.noPpd}</p>
    {/if}
  </section>
  {@const fanModeIgnored = (cool?.notes ?? []).some((n) => n.startsWith("Fan speed is not supported"))}
  {#if fanModeIgnored}
    <section class="card">
      <h2>{t().sys.cool.fanMode} <span class="chip warn">{t().sys.cool.unsupported}</span></h2>
      <p class="muted">{t().sys.cool.fanModeIgnored}</p>
    </section>
  {/if}
  {#if cool && cool.fan_mode >= 0}
    <section class="card">
      <h2>{t().sys.cool.fanMode}</h2>
      <Segmented
        label={t().sys.cool.fanMode}
        value={String(cool.fan_mode)}
        options={cool.fan_modes.map(([v, n]) => ({ value: String(v), label: t().sys.cool.fanModes[n] ?? n }))}
        onchange={(v) => coolRun(() => api.setFanMode(Number(v)))}
      />
      <p class="small">{t().sys.cool.fanModeHint}</p>
    </section>
  {/if}
  <section class="card">
    <h2>{t().sys.cool.fans}</h2>
    {#if cool && !cool.fans.length}
      <p class="muted"><span class="chip warn">{t().sys.cool.unsupported}</span></p>
      <p class="small">{t().sys.cool.noFans}</p>
    {/if}
    {#each cool?.fans ?? [] as f (f.id)}
      <div class="fan">
        <div class="fan-top">
          <b>{f.id.startsWith("amdgpu/") ? t().sys.cool.gpuFan : f.label}</b>
          <span class="muted" dir="ltr">{num(f.rpm)} rpm · {num(f.pct)}% · {num(f.temp_c)}°C</span>
          {#if f.mode === "firmware"}
            <span class="chip warn">{t().sys.cool.unsupported}</span>
          {:else}
            <span class="chip">{f.mode === "curve" ? t().sys.cool.curve : t().sys.cool.auto}</span>
          {/if}
        </div>
        {#if f.mode === "firmware"}
          <p class="small">{t().sys.cool.fanFirmware}</p>
        {:else}
        <div class="line">
          {#each ["quiet", "balanced", "cool"] as const as p}
            <button class="btn" disabled={busy} onclick={() => coolRun(() => api.setFanCurve(f.id, FAN_PRESETS[p]))}>{t().sys.cool.presets[p]}</button>
          {/each}
          <button class="btn" disabled={busy || f.mode === "auto"} onclick={() => coolRun(() => api.setFanAuto(f.id))}>{t().sys.cool.auto}</button>
        </div>
        {#if f.curve.length}
          <p class="small" dir="ltr">{f.curve.map(([c, p]) => `${c}°C → ${p}%`).join("   ")}</p>
        {/if}
        {/if}
      </div>
    {/each}
    {#each (cool?.notes ?? []).filter((n) => cool?.simulated && n.startsWith("Development")) as n}<p class="muted">{n}</p>{/each}
    {#if cool?.fans.some((f) => f.mode !== "firmware")}<p class="small">{t().sys.cool.safety}</p>{/if}
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
{:else if tab === "sleep"}
  <section class="card">
    <h2><HugeIcon name="alarm-clock" size={18} /> {t().sys.slp.alarm}</h2>
    {#if alarmSupported}
      <p class="small intro">{t().sys.slp.alarmIntro}</p>
      <div class="alarm-row">
        <TimePicker bind:value={alarmClock} label={t().sys.atTime} />
        <div class="alarm-side">
          <div class="quick">
            {#each Object.keys(t().sys.slp.quick) as q}
              <button class="chip-btn" onclick={() => quickAlarm(Number(q))}>{t().sys.slp.quick[q]}</button>
            {/each}
          </div>
          <div class="line">
            <button class="primary" disabled={busy} onclick={setAlarm}>{t().sys.alarmSet}</button>
            <button class="secondary" disabled={busy || !capOf("suspend")?.available} onclick={sleepUntil}><HugeIcon name="moon-02" size={15} /> {t().sys.sleepWakeGo}</button>
          </div>
        </div>
      </div>
      <div class="armed-box" class:on={!!alarm}>
        {#if alarm}
          <span class="pulse-dot" aria-hidden="true"></span>
          <span>{t().sys.alarmNow} <b>{fmt(alarm)}</b> <span class="muted">· {inTime(alarm)}</span></span>
          <button class="cancel" onclick={() => run(() => api.clearWakeAlarm())}><HugeIcon name="delete-02" size={15} />{t().sys.alarmClear}</button>
        {:else}
          <span class="muted">{t().sys.alarmNone}</span>
        {/if}
      </div>
      <p class="small">{t().sys.alarmHint}</p>
    {:else}
      <p class="muted"><span class="chip warn">{t().sys.cool.unsupported}</span></p>
      <p class="small">{t().sys.alarmUnsupported}</p>
    {/if}
  </section>

  <section class="card">
    <h2><HugeIcon name="moon-02" size={18} /> {t().sys.slp.modeTitle}</h2>
    {#if slp && slp.sleep_modes.length > 1}
      <div class="modes" role="radiogroup" aria-label={t().sys.slp.modeTitle}>
        {#each slp.sleep_modes as m}
          <button class="mode" role="radio" aria-checked={slp.sleep_mode === m} disabled={busy} onclick={() => slp?.sleep_mode !== m && slpRun(() => api.setSleepMode(m, slp?.sleep_mode_kept ?? false))}>
            <span class="mode-top"><span class="dot" aria-hidden="true"></span><b>{t().sys.slp.modes[m] ?? m}</b></span>
            {#if t().sys.slp.modeHelp[m]}<span class="mode-help">{t().sys.slp.modeHelp[m]}</span>{/if}
          </button>
        {/each}
      </div>
      <div class="line keep">
        <Switch checked={slp.sleep_mode_kept} disabled={busy} label={t().sys.slp.keep} onchange={(v) => slpRun(() => api.setSleepMode(slp!.sleep_mode, v))} />
        <div><p class="row-title">{t().sys.slp.keep}</p><p class="small tight">{t().sys.slp.keepHint}</p></div>
      </div>
    {:else if slp}
      <p class="muted"><span class="chip warn">{t().sys.cool.unsupported}</span></p>
      <p class="small">{t().sys.slp.onlyOne.replace("{m}", slp.sleep_mode || "s2idle")}</p>
    {/if}
  </section>

  <section class="card">
    <h2><HugeIcon name="shut-down" size={18} /> {t().sys.slp.buttons}</h2>
    <p class="small intro">{t().sys.slp.buttonsIntro}</p>
    {#if slp?.buttons_note}
      <p class="muted"><span class="chip warn">{t().sys.cool.unsupported}</span> {slp.buttons_note}</p>
    {:else if slp}
      {#if !slp.has_lid}<p class="small">{t().sys.slp.noLid}</p>{/if}
      {#if slp.has_lid && slp.lid_handled_by.length}
        <p class="held-box">{t().sys.slp.handled.replace("{who}", slp.lid_handled_by.map((h) => h.split(":")[0]).join(", "))}</p>
      {/if}
      {#each (slp.has_lid ? ["lid-battery", "lid-ac", "lid-docked", "power-key"] : ["power-key"]) as ButtonId[] as b}
        {@const cur = { "lid-battery": slp.lid_battery, "lid-ac": slp.lid_ac, "lid-docked": slp.lid_docked, "power-key": slp.power_key }[b]}
        {@const held = b === "power-key" ? slp.power_key_handled_by : slp.lid_handled_by}
        <div class="btn-row">
          <span class="row-title">{t().sys.slp.rows[b]}</span>
          <div><Segmented
            label={t().sys.slp.rows[b]}
            value={cur}
            options={["ignore", "lock", "suspend", "hibernate", "poweroff"].map((a) => ({
              value: a,
              label: t().sys.slp.acts[a],
              disabled: busy || (a === "hibernate" && !capOf("hibernate")?.available),
              why: a === "hibernate" ? t().sys.noHibernate : undefined,
            }))}
            onchange={(a) => slpRun(() => api.setButtonAction(b, a))}
          /></div>
          {#if b === "power-key" && held.length}
            <p class="held-box">{t().sys.slp.handled.replace("{who}", held.map((h) => h.split(":")[0]).join(", "))}</p>
          {/if}
        </div>
      {/each}
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
  .intro { margin: 0 0 14px; max-width: 75ch; }
  input[type="range"] { accent-color: var(--accent); width: 220px; }
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
  .alarm-row { display: flex; align-items: center; gap: 22px; flex-wrap: wrap; margin: 6px 0 14px; }
  .alarm-side { display: grid; gap: 12px; }
  .alarm-side .line { margin: 0; }
  .quick { display: flex; gap: 8px; flex-wrap: wrap; }
  .chip-btn {
    all: unset;
    padding: 5px 12px;
    border-radius: 999px;
    background: var(--card-2);
    color: var(--text-2);
    font-size: 0.82rem;
    cursor: pointer;
    transition: color 150ms ease-out, background 150ms ease-out;
  }
  .chip-btn:hover { color: var(--text); background: color-mix(in oklab, var(--accent) 16%, var(--card-2)); }
  .chip-btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .primary {
    all: unset;
    padding: 8px 18px;
    border-radius: 10px;
    background: var(--accent);
    color: #fff;
    font-weight: 600;
    font-size: 0.9rem;
    cursor: pointer;
    transition: filter 150ms ease-out;
  }
  .primary:hover:not(:disabled) { filter: brightness(1.1); }
  .primary:disabled { opacity: 0.5; cursor: not-allowed; }
  .primary:focus-visible { outline: 2px solid var(--text); outline-offset: 2px; }
  .secondary { display: inline-flex; align-items: center; gap: 6px; }
  .armed-box {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    border-radius: 12px;
    background: var(--card-2);
  }
  .armed-box.on { background: color-mix(in oklab, var(--accent) 12%, var(--card-2)); }
  .armed-box > span:nth-child(2) { flex: 1; }
  .pulse-dot { width: 9px; height: 9px; border-radius: 50%; background: var(--accent-text); flex: none; }
  .keep { margin: 16px 0 0; align-items: flex-start; flex-wrap: nowrap; }
  .tight { margin-top: 2px; }
  .btn-row { display: grid; gap: 8px; padding: 12px 0; border-bottom: 1px solid var(--line); }
  .btn-row:last-child { border-bottom: 0; }
  .held-box {
    margin: 8px 0 4px;
    padding: 10px 14px;
    border-radius: 12px;
    font-size: 0.84rem;
    color: var(--warn);
    background: color-mix(in oklab, var(--warn) 10%, var(--card));
  }
  .card-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; flex-wrap: wrap; margin-bottom: 10px; }
  .card-head h2 { margin: 0; }
  .tiles { display: grid; grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); gap: 10px; margin-top: 14px; }
  .tile { display: grid; gap: 2px; padding: 12px 14px; border-radius: 12px; background: var(--card-2); }
  .tile .big { font-size: 1.35rem; }
  .periods { list-style: none; margin: 14px 0 0; padding: 0; display: grid; gap: 6px; max-height: 260px; overflow: auto; }
  .periods li { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; padding: 8px 12px; border-radius: 10px; background: var(--card-2); font-size: 0.88rem; }
  .periods .when { font-variant-numeric: tabular-nums; color: var(--text-2); min-width: 9em; }
  .swatch { width: 10px; height: 10px; border-radius: 3px; flex: none; }
  .periods .battery .swatch { background: var(--warn); }
  .periods .ac .swatch { background: var(--ok); }
  .periods .off .swatch { background: var(--line); }
  .health-top { display: flex; align-items: center; gap: 22px; margin: 4px 0 8px; flex-wrap: wrap; }
  .ring {
    width: 96px;
    height: 96px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background:
      radial-gradient(closest-side, var(--card) 79%, transparent 80%),
      conic-gradient(var(--accent-text) calc(var(--p) * 1%), var(--card-2) 0);
    flex: none;
  }
  .ring span { font-size: 1.6rem; font-weight: 800; }
  .ring small { font-size: 0.6em; color: var(--text-2); }
  .verdict { margin: 0 0 4px; font-weight: 700; font-size: 1.1rem; }
  .health-top .muted { margin: 2px 0; }
  .methods { display: grid; gap: 10px; margin-top: 12px; }
  .method { padding: 12px 14px; border-radius: 12px; background: var(--card-2); display: grid; gap: 6px; }
  .method-top { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .method-top .big { margin-inline-start: auto; font-size: 1.25rem; }
  .meter { height: 6px; border-radius: 999px; background: var(--card); overflow: hidden; }
  .meter span { display: block; height: 100%; width: calc(var(--w) * 1%); background: var(--accent); border-radius: 999px; }
  .conf.high { color: var(--ok); background: color-mix(in oklab, var(--ok) 14%, var(--card)); }
  .conf.medium { color: var(--accent-text); background: var(--accent-soft); }
  .conf.low { color: var(--text-2); background: var(--card); }
  .chip.warn { background: color-mix(in oklab, var(--warn) 16%, var(--card)); color: var(--warn); font-weight: 600; }
  .modes { display: grid; gap: 10px; margin-top: 16px; }
  .mode {
    all: unset;
    box-sizing: border-box;
    display: grid;
    gap: 4px;
    padding: 14px 16px;
    border-radius: 12px;
    background: var(--card-2);
    border: 1.5px solid transparent;
    cursor: pointer;
    transition: border-color 150ms ease-out, background 150ms ease-out;
  }
  .mode:hover:not(:disabled) { border-color: color-mix(in oklab, var(--accent) 40%, transparent); }
  .mode[aria-checked="true"] { border-color: var(--accent); background: var(--accent-soft); }
  .mode:disabled { cursor: progress; }
  .mode:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .mode-top { display: flex; align-items: center; gap: 10px; }
  .dot { width: 14px; height: 14px; border-radius: 50%; border: 2px solid var(--text-3); flex: none; box-sizing: border-box; }
  .mode[aria-checked="true"] .dot { border-color: var(--accent); background: radial-gradient(var(--accent) 45%, transparent 50%); }
  .mode-help { color: var(--text-2); font-size: 0.84rem; line-height: 1.7; padding-inline-start: 24px; }
  .fan { display: grid; gap: 8px; padding: 12px 0; border-bottom: 1px solid var(--line); }
  .fan:last-of-type { border-bottom: 0; }
  .fan-top { display: flex; gap: 12px; align-items: baseline; flex-wrap: wrap; }
  .blocked { box-shadow: 0 0 0 1px color-mix(in oklab, var(--danger) 55%, transparent); margin-bottom: 16px; }
  .blocked ul { margin: 6px 0 10px; padding-inline-start: 20px; display: grid; gap: 4px; }
  .blocked-actions { display: flex; gap: 10px; flex-wrap: wrap; align-items: center; margin-top: 10px; }
  .blocked-actions .go { width: auto; padding-inline: 22px; }
  .warn-line { color: var(--warn); margin: 8px 0 0; font-size: 0.9rem; }
</style>
