<script lang="ts">
  import HugeIcon from "../lib/icons/HugeIcon.svelte";
  import { api, type AlertEvent, type GpuDefault, type GpuInfo, type Overview, type PlaytimeEntry, type Profile, type RunningGame, type Snapshot } from "../lib/api";
  import { duration, gib, num, rate } from "../lib/format";
  import { locale, t } from "../lib/i18n.svelte";
  import LineChart from "../lib/LineChart.svelte";
  import ProfilePicker from "../lib/ui/ProfilePicker.svelte";

  let { overview }: { overview: Overview | null } = $props();

  const HISTORY = 120; // two minutes at one sample a second
  let snap = $state<Snapshot | null>(null);
  let times = $state<number[]>([]);
  let cpuTemps = $state<(number | null)[]>([]);
  let gpuTemps = $state<(number | null)[]>([]);
  let running = $state<RunningGame[]>([]);
  let profiles = $state<Profile[]>([]);
  let week = $state<PlaytimeEntry[] | null>(null);
  let gpuDef = $state<GpuDefault | null>(null);
  let gpuBusy = $state(false);
  let gpuErr = $state("");
  let gpuMenu = $state(false);
  let gpuPicker = $state<HTMLElement | null>(null);
  // Game mode by hand: "on", "off" or "" (follows the games), and its profile.
  let hand = $state<[string, string]>(["", "balanced"]);
  let gmBusy = $state(false);
  let gmErr = $state("");

  // The picker closes on a click elsewhere or Escape.
  $effect(() => {
    if (!gpuMenu) return;
    const away = (e: PointerEvent) => {
      if (gpuPicker && !gpuPicker.contains(e.target as Node)) gpuMenu = false;
    };
    const esc = (e: KeyboardEvent) => {
      if (e.key === "Escape") gpuMenu = false;
    };
    document.addEventListener("pointerdown", away);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("pointerdown", away);
      document.removeEventListener("keydown", esc);
    };
  });

  const inv = $derived(overview?.inventory ?? null);
  const dGpu = $derived(inv?.gpus.find((g) => g.pci === overview?.active_gpu) ?? inv?.gpus.find((g) => !g.boot_vga) ?? null);
  const iGpu = $derived(inv?.gpus.find((g) => g !== dGpu) ?? null);

  function take(s: Snapshot) {
    snap = s;
    const g = dGpu ? s.gpus.find((x) => x.pci === dGpu.pci) : null;
    times = [...times, s.ts_unix_ms / 1000].slice(-HISTORY);
    cpuTemps = [...cpuTemps, s.cpu.temp_c].slice(-HISTORY);
    gpuTemps = [...gpuTemps, g && !g.asleep ? g.temp_c : null].slice(-HISTORY);
  }

  // Samples flow only while this page is on screen: hidden or closed, the
  // subscription ends and the daemon parks its loop.
  $effect(() => {
    let unlisten: (() => void) | null = null;
    let stopped = false;
    // Leaving the page while these calls are in flight must not leave a
    // listener (samples counted twice on return) or a subscription (the
    // service sampling for nobody) behind.
    const start = async () => {
      if (!unlisten) {
        const off = await api.onSample(take);
        if (stopped || unlisten) off();
        else unlisten = off;
      }
      if (stopped) return;
      await api.monitorStart(1000).catch(() => {});
      if (stopped || document.hidden) stop();
    };
    const stop = () => api.monitorStop().catch(() => {});
    const onVis = () => (document.hidden ? stop() : start());
    api.snapshot().then((s) => { if (!snap) take(s); }).catch(() => {});
    start();
    document.addEventListener("visibilitychange", onVis);
    return () => {
      stopped = true;
      document.removeEventListener("visibilitychange", onVis);
      unlisten?.();
      stop();
    };
  });

  // Games change rarely: a slow poll while visible, one small D-Bus call.
  $effect(() => {
    const load = () => {
      if (document.hidden) return;
      api.gamesRunning().then((r) => (running = r)).catch(() => {});
      api.profiles().then((p) => (profiles = p)).catch(() => {});
      api.gpuDefault().then((d) => (gpuDef = d)).catch(() => {});
      api.gameModeHand().then((h) => (hand = h)).catch(() => {});
    };
    load();
    // Once even when opened hidden, so the card switch is usable at once.
    if (document.hidden) api.gpuDefault().then((d) => (gpuDef = d)).catch(() => {});
    api.playtime(Date.now() - 7 * 24 * 3600 * 1000).then((w) => (week = w)).catch(() => (week = []));
    const id = setInterval(load, 3000);
    return () => clearInterval(id);
  });

  // The last alert, for its card: a slow poll while visible.
  let lastAlert = $state<AlertEvent | null | undefined>(undefined);
  $effect(() => {
    const load = () => {
      if (document.hidden) return;
      api.alertsState().then((a) => (lastAlert = a.recent.find((e) => e.kind !== "test") ?? null)).catch(() => {});
    };
    load();
    const id = setInterval(load, 10000);
    return () => clearInterval(id);
  });
  const alertTime = (ms: number) =>
    new Intl.DateTimeFormat(locale.current === "fa" ? "fa-IR" : "en-GB", { weekday: "short", hour: "2-digit", minute: "2-digit", hour12: false }).format(ms);
  const alertValue = (e: AlertEvent) => {
    const pct = e.kind === "memory" || e.kind === "disk";
    const u = pct ? t().unit.pct : t().unit.c;
    return `${num(e.value)}${u} · ${t().mon.alerts.limit} ${num(e.threshold)}${u}`;
  };

  const game = $derived(running[0] ?? null);
  // By hand wins; otherwise it follows the games.
  const gmOn = $derived(hand[0] === "on" || (hand[0] !== "off" && !!game));
  const profileId = $derived(game && hand[0] !== "on" ? game.profile : hand[1]);
  async function setGameMode(on: boolean, profile = "") {
    gmBusy = true;
    gmErr = "";
    try {
      await api.setGameMode(on, profile);
      hand = await api.gameModeHand();
    } catch (e) {
      gmErr = String(e);
    } finally {
      gmBusy = false;
    }
  }
  const profile = $derived(profiles.find((p) => p.id === profileId) ?? null);
  const switches = [
    "cpu_performance", "cpu_no_idle_states", "cpu_preferred_cores",
    "cpu_priority", "memory_tuning", "pause_services", "network_tuning", "power_profile",
  ] as const;

  const memUsed = $derived(snap ? snap.mem.total_kb - snap.mem.available_kb : 0);
  const memPct = $derived(snap && snap.mem.total_kb ? (memUsed * 100) / snap.mem.total_kb : null);
  // Which GPU is working now: the discrete one only while it is awake.
  const sampleFor = (pci: string) => snap?.gpus.find((g) => g.pci === pci) ?? null;
  const dAwake = $derived(!!dGpu && !!snap && !(sampleFor(dGpu.pci)?.asleep ?? true));
  const workingGpu = $derived(dAwake || !iGpu ? dGpu : iGpu);
  // With two cards the tile shows the one the switch has chosen; without
  // the switch's state yet, the one doing the work now.
  const chosenGpu = $derived(gpuDef ? (inv?.gpus.find((g) => g.pci === gpuDef?.chosen) ?? null) : null);
  const activeGpu = $derived(iGpu && dGpu && chosenGpu ? chosenGpu : workingGpu);
  const otherGpu = $derived(activeGpu === dGpu ? iGpu : dGpu);
  const sameVendor = $derived(!!iGpu && !!dGpu && iGpu.vendor === dGpu.vendor);
  function shortName(g: GpuInfo): string {
    if (sameVendor || g.vendor === "unknown") return g.boot_vga ? t().gpu.integrated : t().gpu.discrete;
    return { nvidia: "NVIDIA", amd: "AMD", intel: "Intel" }[g.vendor];
  }
  async function chooseGpu(pci: string) {
    if (gpuBusy || gpuDef?.chosen === pci) return;
    gpuMenu = false;
    gpuBusy = true;
    gpuErr = "";
    try {
      gpuDef = await api.setGpuDefault(pci);
    } catch (e) {
      gpuErr = String(e);
    } finally {
      gpuBusy = false;
    }
  }
  const dSample = $derived(dGpu && snap ? snap.gpus.find((g) => g.pci === dGpu.pci) ?? null : null);
  const iSample = $derived(iGpu && snap ? snap.gpus.find((g) => g.pci === iGpu.pci) ?? null : null);
  const pSet = $derived(new Set(inv?.cpu.p_cores ?? []));
  const cores = $derived((snap?.cpu.per_core_load ?? []).map((l, i) => ({ i, load: l ?? 0, p: pSet.size === 0 || pSet.has(i) })));
  const weekTotal = $derived((week ?? []).reduce((a, e) => a + e.seconds, 0));
  const coreGroups = $derived([
    { label: t().cpu.pcores, list: cores.filter((c) => c.p) },
    { label: t().cpu.ecores, list: cores.filter((c) => !c.p) },
  ].filter((g) => g.list.length));

  const chartSeries = $derived([
    { label: t().chart.cpu, color: "#8f6af2", values: cpuTemps },
    ...(gpuTemps.some((v) => v != null) ? [{ label: t().chart.gpu, color: "#c98500", values: gpuTemps }] : []),
  ]);

  function heat(c: number | null | undefined): "ok" | "warm" | "hot" | "none" {
    if (c == null) return "none";
    return c >= 85 ? "hot" : c >= 70 ? "warm" : "ok";
  }
</script>

<header class="head">
  <h1>{t().nav.dashboard}</h1>
  {#if api.preview}
    <span class="badge">{t().preview}</span>
  {:else}
    <span class="badge live"><span class="pulse" aria-hidden="true"></span>{t().live}</span>
  {/if}
</header>

<!-- The four live tiles, as on Phoenix Guardian's monitoring page. -->
<section class="tiles">
  <!-- The GPU doing the work now: on a hybrid laptop the integrated one
       unless the discrete one is awake. The other sits underneath with its
       state; a sleeping GPU is never woken to fill this in. -->
  <article class="tile">
    <div class="tile-head">
      <span class="ico"><HugeIcon name="gpu" size={18} /></span>
      <h2>{t().parts.gpu}</h2>
      <!-- The card switch: which GPU the programs Unix Guardian starts use.
           Only with two cards; never session-wide, so the desktop stays on
           the card the panel is wired to. -->
      {#if iGpu && dGpu}
        <div class="gpu-picker" bind:this={gpuPicker}>
          <button
            type="button"
            class="gpu-toggle"
            class:open={gpuMenu}
            aria-haspopup="true"
            aria-expanded={gpuMenu}
            aria-label={t().gpu.switchLabel}
            title={t().gpu.switchLabel}
            disabled={gpuBusy || !gpuDef}
            onclick={() => (gpuMenu = !gpuMenu)}
          ><HugeIcon name="arrow-data-transfer-horizontal" size={14} /></button>
          {#if gpuMenu}
            <div class="gpu-menu" role="radiogroup" aria-label={t().gpu.switchLabel}>
              <p class="gpu-menu-title">{t().gpu.switchLabel}</p>
              {#each [iGpu, dGpu] as g (g.pci)}
                <button
                  type="button"
                  role="radio"
                  aria-checked={activeGpu === g}
                  class:on={activeGpu === g}
                  onclick={() => chooseGpu(g.pci)}
                >
                  <span class="tick" aria-hidden="true">{#if activeGpu === g}<HugeIcon name="tick-02" size={14} />{/if}</span>
                  <span class="gpu-name">
                    <b>{shortName(g)}</b>
                    <small class="ltr-inline">{g.name}</small>
                  </span>
                </button>
              {/each}
              <p class="gpu-menu-why">{t().gpu.switchWhy}</p>
            </div>
          {/if}
        </div>
      {/if}
    </div>
    {#if activeGpu}
      {@const s = sampleFor(activeGpu.pci)}
      {#if s && s.temp_c != null}
        <p class="value">
          <span class="temp {heat(s.temp_c)}">{num(s.temp_c)}{t().unit.c}</span>
          <span class="pct">{s.load_pct != null ? `${num(s.load_pct)}${t().unit.pct}` : `${num(s.core_clock_mhz)} ${t().unit.mhz}`}</span>
        </p>
      {:else if s?.asleep}
        <!-- The switch can show a sleeping card; it is never woken to fill this in. -->
        <p class="value"><span>{t().gpu.asleep}</span></p>
      {:else}
        <p class="value">
          <span>{num(s?.core_clock_mhz ?? null)}<small> {t().unit.mhz}</small></span>
        </p>
      {/if}
      <p class="hw ltr">{activeGpu.name}</p>
      {#if s?.asleep}<p class="hint">{t().gpu.asleepWhy}</p>
      {:else if s && s.temp_c == null}<p class="hint">{t().gpu.sharedTemp}</p>{/if}
      {#if s?.load_pct != null}{@render bar(s.load_pct)}{/if}
    {/if}
    {#if gpuErr}
      <p class="hint err">{gpuErr}</p>
    {/if}
    {#if otherGpu}
      {@const o = sampleFor(otherGpu.pci)}
      <p class="other">
        <span class="lamp-s" class:on={!!o && !o.asleep} aria-hidden="true"></span>
        <span class="ltr-inline">{otherGpu.name}</span>
        <span>{o?.asleep ? t().gpu.asleep : o?.temp_c != null ? `${num(o.temp_c)}${t().unit.c}` : t().gpu.idle}</span>
      </p>
    {/if}
  </article>

  <article class="tile">
    <div class="tile-head">
      <span class="ico"><HugeIcon name="cpu" size={18} /></span>
      <h2>{t().parts.cpu}</h2>
    </div>
    <p class="value">
      <span class="temp {heat(snap?.cpu.temp_c)}">{num(snap?.cpu.temp_c)}{t().unit.c}</span>
      <span class="pct">{num(snap?.cpu.load_pct)}{t().unit.pct}</span>
    </p>
    <p class="hw ltr">{inv?.cpu.model ?? "—"}</p>
    {@render bar(snap?.cpu.load_pct ?? 0)}
  </article>

  <article class="tile">
    <div class="tile-head">
      <span class="ico"><HugeIcon name="ram-memory" size={18} /></span>
      <h2>{t().parts.mem}</h2>
    </div>
    <p class="value">
      <span>{snap ? gib(memUsed) : "—"}<small> / {snap ? gib(snap.mem.total_kb) : "—"} {t().unit.gb}</small></span>
      <span class="pct">{num(memPct)}{t().unit.pct}</span>
    </p>
    <p class="hw">{t().mem.swap} {snap ? gib(snap.mem.swap_used_kb) : "—"} {t().mem.of} {snap ? gib(snap.mem.swap_total_kb) : "—"} {t().unit.gb}</p>
    {@render bar(memPct ?? 0)}
  </article>

  <article class="tile">
    <div class="tile-head">
      <span class="ico"><HugeIcon name="network" size={18} /></span>
      <h2>{t().parts.net}</h2>
    </div>
    {#if snap}
      {@const d = rate(snap.net.rx_bps)}
      {@const u = rate(snap.net.tx_bps)}
      <p class="value net">
        <span><HugeIcon name="arrow-down-02" size={16} label={t().net.down} /><bdi>{d.value} {d.unit}</bdi></span>
        <span class="pct"><HugeIcon name="arrow-up-02" size={16} label={t().net.up} /><bdi>{u.value} {u.unit}</bdi></span>
      </p>
      <p class="hw ltr">{snap.net.iface}</p>
    {:else}
      <p class="value">—</p>
    {/if}
  </article>
</section>

<section class="row">
  <!-- Game mode at a glance: state, the profile, and what it will change. -->
  <article class="card gm" class:on={gmOn}>
    <div class="card-head">
      <span class="ico"><HugeIcon name="game-controller-03" size={18} /></span>
      <h2>{t().parts.gamemode}</h2>
      <span class="state-pill" class:on={gmOn}>{gmOn ? t().gm.on : t().gm.off}</span>
    </div>
    <p class="gm-line">
      {#if game}
        <strong>{game.name}</strong> {t().gm.running}
      {:else}
        {t().gm.waiting}
      {/if}
    </p>
    <div class="gm-actions">
      {#if gmOn}
        <button class="gm-btn" disabled={gmBusy} onclick={() => setGameMode(false)}><HugeIcon name="shut-down" size={15} /> {t().gm.turnOff}</button>
      {:else}
        <button class="gm-btn primary" disabled={gmBusy} onclick={() => setGameMode(true, hand[1])}><HugeIcon name="shut-down" size={15} /> {t().gm.turnOn}</button>
      {/if}
      <ProfilePicker
        quiet
        label={t().gm.activeProfile}
        value={profileId}
        disabled={gmBusy}
        options={profiles.map((p) => ({ value: p.id, label: t().gm.profiles[p.id] ?? p.id, builtin: p.builtin }))}
        onchange={(id) => (gmOn ? setGameMode(true, id) : (hand = [hand[0], id]))}
      />
    </div>
    {#if gmErr}<p class="gm-err">{gmErr}</p>{/if}
    <div class="chips">
      <span class="chip">{t().gm.selection}: <b>{hand[0] === "on" ? t().gm.byHand : hand[0] === "off" ? t().gm.offByHand : t().gm.automatic}</b></span>
      <span class="chip">{gmOn ? t().gm.activeProfile : t().gm.defaultProfile}: <b>{t().gm.profiles[profileId] ?? profileId}</b></span>
    </div>
    <p class="sub-title">{t().gm.changes}</p>
    <ul class="changes">
      {#each switches as k}
        {@const set = profile?.[k] ?? false}
        <li class:set>
          <span class="tick" aria-hidden="true"></span>
          {t().gm.settings[k]}
        </li>
      {/each}
    </ul>
  </article>

  <article class="card">
    <div class="card-head">
      <h2>{t().chart.title}</h2>
    </div>
    <LineChart {times} series={chartSeries} waiting={t().chart.waiting} />
  </article>
</section>

<section class="row">
  <article class="card">
    <div class="card-head">
      <h2>{t().cpu.cores}</h2>
      <span class="muted">{t().cpu.coresHint}</span>
    </div>
    {#each coreGroups as g}
      <div class="core-group">
        <span class="muted">{g.label}</span>
        <div class="cores">
          {#each g.list as c (c.i)}
            <span class="core" title={`${c.i}: ${Math.round(c.load)}%`}>
              <span class="core-fill" style:--v={Math.max(3, c.load) / 100}></span>
            </span>
          {/each}
        </div>
      </div>
    {/each}
    {#if iGpu}
      <p class="muted small">{iGpu.name}: {num(iSample?.core_clock_mhz ?? null)} {t().unit.mhz}</p>
    {/if}
  </article>

  <article class="card">
    <div class="card-head">
      <h2>{t().pt.title}</h2>
      {#if weekTotal > 0}<span class="muted">{duration(weekTotal)}</span>{/if}
    </div>
    {#if week && week.length}
      <ol class="pt">
        {#each week.slice(0, 5) as e (e.key)}
          <li>
            <span class="pt-name">{e.name}</span>
            <span class="pt-time">{duration(e.seconds)}</span>
            {@render bar(weekTotal ? (e.seconds * 100) / weekTotal : 0)}
          </li>
        {/each}
      </ol>
    {:else if week}
      <p class="empty">{t().pt.empty}</p>
    {/if}
  </article>
</section>

{#if lastAlert !== undefined}
  <section class="row one">
    <article class="card">
      <div class="card-head">
        <HugeIcon name={lastAlert ? "alert-circle" : "circle-check"} size={20} />
        <h2>{t().mon.alerts.last}</h2>
        <a class="muted link" href="#monitoring/alerts" onclick={() => history.replaceState(null, "", "#dashboard")}>{t().mon.alerts.all}</a>
      </div>
      {#if lastAlert}
        <p class="alert-line">
          <b>{t().mon.alerts.kinds[lastAlert.kind]}</b>
          {#if lastAlert.subject}<bdi>{lastAlert.subject}</bdi>{/if}
          <span>{alertValue(lastAlert)}</span>
          <span class="muted">{alertTime(lastAlert.ts_unix_ms)}</span>
        </p>
      {:else}
        <p class="empty calm">{t().mon.alerts.empty}</p>
      {/if}
    </article>
  </section>
{/if}

{#snippet bar(pct: number)}
  <span class="bar" aria-hidden="true"><span style:--v={Math.min(100, Math.max(0, pct)) / 100}></span></span>
{/snippet}

<style>
  .head { display: flex; align-items: center; gap: 14px; margin-bottom: 20px; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 800; }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 3px 12px;
    border-radius: 999px;
    font-size: 0.8rem;
    background: var(--card-2);
    color: var(--text-2);
  }
  .badge.live { background: color-mix(in oklab, var(--ok) 14%, var(--card)); color: var(--ok); }
  /* Static on purpose: an endless animation keeps WebKit repainting at 60 fps,
     which measured as a full CPU core with the window open. */
  .pulse { width: 7px; height: 7px; border-radius: 50%; background: var(--ok); box-shadow: 0 0 0 3px color-mix(in oklab, var(--ok) 25%, transparent); }

  .tiles { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 16px; }
  .row { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 16px; margin-top: 16px; }
  @media (max-width: 1100px) { .tiles { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
  @media (max-width: 860px) { .row { grid-template-columns: minmax(0, 1fr); } }

  .tile, .card {
    background: var(--card);
    border-radius: var(--radius);
    padding: 18px 20px 20px;
    min-width: 0;
  }
  .tile { display: flex; flex-direction: column; }
  .tile-head, .card-head { display: flex; align-items: center; gap: 10px; }
  .tile-head h2 { white-space: nowrap; }
  .card-head { margin-bottom: 12px; }
  h2 { margin: 0; font-size: 0.98rem; font-weight: 700; }
  .ico {
    width: 32px;
    height: 32px;
    border-radius: 10px;
    display: grid;
    place-items: center;
    background: var(--accent-soft);
    color: var(--accent-text);
    flex: none;
  }
  .value {
    margin: 14px 0 2px;
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 10px;
    font-size: 1.9rem;
    font-weight: 700;
    line-height: 1.2;
  }
  .value small { font-size: 0.5em; color: var(--text-2); font-weight: 500; }
  .value.quiet { color: var(--text-2); font-weight: 600; }
  .value.net { font-size: 1.15rem; }
  .value.net span { display: inline-flex; align-items: center; gap: 4px; }
  .pct { font-size: 0.62em; color: var(--text-2); font-weight: 600; }
  .temp.warm { color: var(--warn); }
  .temp.hot { color: var(--danger); }
  .hw { margin: 0; color: var(--text-3); font-size: 0.82rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tile:has(.gpu-picker) { position: relative; }
  .gpu-picker { margin-inline-start: auto; flex: none; position: relative; }
  .gpu-toggle {
    all: unset; box-sizing: border-box; display: grid; place-items: center;
    width: 26px; height: 26px; border-radius: 8px; cursor: pointer;
    color: var(--text-2); background: var(--card-2);
    transition: background-color 0.15s ease, color 0.15s ease;
  }
  .gpu-toggle:hover:not(:disabled), .gpu-toggle.open { color: var(--accent-text); background: var(--accent-soft); }
  .gpu-toggle:disabled { cursor: progress; opacity: 0.6; }
  .gpu-toggle:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  /* Opens toward the neighbouring tile, away from the window edge. */
  .gpu-menu {
    position: absolute; top: calc(100% + 6px); inset-inline-start: 0; z-index: 20;
    width: 250px; padding: 6px; border-radius: 12px;
    background: var(--card); border: 1px solid var(--line);
    box-shadow: 0 12px 32px rgb(0 0 0 / 0.35);
    transform-origin: top left;
    animation: menu-in 0.14s ease-out;
  }
  :global([dir="rtl"]) .gpu-menu { transform-origin: top right; }
  @keyframes menu-in { from { opacity: 0; transform: scale(0.96) translateY(-4px); } }
  @media (prefers-reduced-motion: reduce) { .gpu-menu { animation: none; } }
  .gpu-menu-title { margin: 4px 8px 6px; font-size: 0.72rem; color: var(--text-3); }
  .gpu-menu button {
    all: unset; box-sizing: border-box; width: 100%; display: flex; align-items: center; gap: 8px;
    padding: 7px 8px; border-radius: 8px; cursor: pointer; color: var(--text-2);
  }
  .gpu-menu button:hover { background: var(--card-2); color: var(--text); }
  .gpu-menu button.on { color: var(--accent-text); }
  .gpu-menu button:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
  .gpu-menu .tick { width: 14px; flex: none; display: grid; place-items: center; }
  .gpu-name { display: flex; flex-direction: column; min-width: 0; }
  .gpu-name b { font-weight: 600; font-size: 0.82rem; }
  .gpu-name small { direction: ltr; unicode-bidi: isolate; text-align: start; font-size: 0.72rem; color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .gpu-menu-why { margin: 6px 8px 4px; padding-top: 8px; border-top: 1px solid var(--line); font-size: 0.72rem; line-height: 1.5; color: var(--text-3); }
  .hint.err { color: var(--danger); }
  .other { margin: 10px 0 0; padding-top: 10px; border-top: 1px solid var(--line); display: flex; align-items: center; gap: 8px; font-size: 0.78rem; color: var(--text-3); }
  .other .ltr-inline { direction: ltr; unicode-bidi: isolate; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .lamp-s { width: 7px; height: 7px; border-radius: 50%; background: var(--text-3); flex: none; }
  .lamp-s.on { background: var(--ok); }
  .hint { margin: 8px 0 0; color: var(--text-2); font-size: 0.8rem; line-height: 1.55; }
  .ltr { direction: ltr; text-align: end; }

  .bar { display: block; position: relative; height: 6px; margin-top: auto; padding-top: 12px; box-sizing: content-box; }
  .bar::before { content: ""; position: absolute; inset: 12px 0 0; border-radius: 999px; background: var(--card-2); }
  .bar > span {
    position: absolute;
    top: 12px;
    bottom: 0;
    inset-inline: 0;
    border-radius: 999px;
    background: var(--accent);
    transform-origin: right;
    transform: scaleX(var(--v));
  }
  :global([dir="ltr"]) .bar > span { transform-origin: left; }

  .state-pill {
    margin-inline-start: auto;
    padding: 2px 12px;
    border-radius: 999px;
    font-size: 0.8rem;
    background: var(--card-2);
    color: var(--text-2);
  }
  .state-pill.on { background: var(--accent); color: #fff; }
  .gm-actions { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; margin: 4px 0 10px; }
  .gm-btn { all: unset; display: inline-flex; align-items: center; gap: 7px; padding: 8px 14px; border-radius: 10px; background: var(--card-2); font-size: 0.88rem; font-weight: 600; cursor: pointer; }
  .gm-btn.primary { background: var(--accent); color: white; }
  .gm-btn:disabled { opacity: 0.55; cursor: default; }
  .gm-btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .gm-err { margin: 0 0 8px; color: var(--danger); font-size: 0.82rem; }
  .gm.on { box-shadow: inset 0 0 0 1px color-mix(in oklab, var(--accent) 55%, transparent); }
  .gm-line { margin: 0 0 12px; color: var(--text-2); font-size: 0.9rem; }
  .gm-line strong { color: var(--text); }
  .chips { display: flex; flex-wrap: wrap; gap: 8px; }
  .chip { padding: 4px 12px; border-radius: 999px; background: var(--card-2); color: var(--text-2); font-size: 0.82rem; }
  .chip b { color: var(--text); font-weight: 600; }
  .sub-title { margin: 16px 0 8px; font-size: 0.82rem; color: var(--text-3); }
  .changes {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px 16px;
    font-size: 0.86rem;
    color: var(--text-3);
  }
  .changes li { display: flex; align-items: center; gap: 8px; }
  .changes li.set { color: var(--text); }
  .tick { width: 16px; height: 16px; border-radius: 5px; border: 1.5px solid var(--line); flex: none; box-sizing: border-box; }
  .set .tick { background: var(--accent); border-color: var(--accent); box-shadow: inset 0 0 0 3px var(--card); }

  .muted { color: var(--text-3); font-size: 0.82rem; }
  .card-head .muted { margin-inline-start: auto; }
  .row.one { grid-template-columns: minmax(0, 1fr); }
  .link { text-decoration: none; }
  .link:hover { color: var(--text); }
  .alert-line { display: flex; flex-wrap: wrap; gap: 6px 14px; align-items: baseline; margin: 0; font-size: 0.92rem; }
  .alert-line .muted { margin-inline-start: auto; }
  .calm { color: var(--ok); }
  .small { margin: 14px 0 0; }
  .core-group { display: grid; grid-template-columns: 8.5rem 1fr; align-items: center; gap: 12px; margin-top: 10px; }
  .cores { display: flex; gap: 6px; height: 54px; }
  .core { flex: 1; max-width: 26px; border-radius: 6px; background: var(--card-2); position: relative; overflow: hidden; }
  .core-fill {
    position: absolute;
    inset: 0;
    background: var(--accent);
    transform-origin: bottom;
    transform: scaleY(var(--v));
  }

  .pt { list-style: none; margin: 0; padding: 0; display: grid; gap: 12px; }
  .pt li { display: grid; grid-template-columns: 1fr auto; gap: 0 12px; font-size: 0.9rem; }
  .pt-time { color: var(--text-2); }
  .pt li .bar { grid-column: 1 / -1; padding-top: 6px; }
  .pt li .bar::before { top: 6px; }
  .pt li .bar > span { top: 6px; }
  .empty { margin: 0; color: var(--text-2); font-size: 0.88rem; max-width: 46ch; }
</style>
