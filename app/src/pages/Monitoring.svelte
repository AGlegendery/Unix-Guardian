<script lang="ts">
  import Problem from "../lib/ui/Problem.svelte";
  import { ArrowDown, ArrowUp } from "@lucide/svelte";
  import { api, errorText, type ExtraSample, type FpsReading, type FpsSession, type HistoryPoint, type Snapshot } from "../lib/api";
  import { num, rate } from "../lib/format";
  import { locale, t } from "../lib/i18n.svelte";
  import LineChart from "../lib/LineChart.svelte";
  import NetRules from "../lib/NetRules.svelte";
  import { NetControl } from "../lib/netcontrol.svelte";
  import PingPanel from "../lib/PingPanel.svelte";
  import TrafficPeriods from "../lib/TrafficPeriods.svelte";
  import TrafficTiles from "../lib/TrafficTiles.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";

  type Tab = "fps" | "thermal" | "traffic" | "ping";
  type Range = "live" | "hour" | "day" | "week";
  const hashTab = location.hash.split("/")[1] as Tab;
  let tab = $state<Tab>(["fps", "thermal", "traffic", "ping"].includes(hashTab) ? hashTab : "thermal");
  // Internet control (apps, rules) for the Traffic and Ping tabs, read
  // every 2 s while one of them is open.
  const ctl = new NetControl();
  $effect(() => {
    if (tab !== "traffic" && tab !== "ping") return;
    return ctl.poll();
  });
  const hashRange = location.hash.split("/")[2] as Range;
  let range = $state<Range>(["live", "hour", "day", "week"].includes(hashRange) ? hashRange : "day");
  let points = $state<HistoryPoint[]>([]);
  let live = $state<Snapshot | null>(null);
  let error = $state<string | null>(null);
  let loaded = $state(false);
  // Live: one snapshot and one extra reading a second, the last two minutes.
  const LIVE_POINTS = 120;
  let snaps = $state.raw<Snapshot[]>([]);
  let extras = $state.raw<ExtraSample[]>([]);
  let fps = $state<FpsReading | null>(null);
  let sessions = $state<FpsSession[]>([]);
  let sessionRange = $state<"week" | "all">("week");
  let fpsBusy = $state(false);

  // Colours checked with the dataviz validator (see LineChart).
  const CPU = "#8f6af2";
  const GPU = "#c98500";

  function since(r: Range): number {
    if (r === "hour") return Date.now() - 3_600_000;
    if (r === "week") return Date.now() - 7 * 86_400_000;
    const d = new Date();
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  }

  // History changes once a minute: reload on range change and every minute
  // while the page is visible.
  $effect(() => {
    const r = range;
    if (r === "live") return;
    const load = () => {
      if (document.hidden) return;
      api.history(since(r), Date.now(), r === "hour" ? 60 : 240)
        .then((p) => { points = p; loaded = true; error = null; })
        .catch((e) => (error = errorText(e)));
    };
    load();
    const id = setInterval(load, 60_000);
    // Opened while hidden (or come back after a while): load when shown.
    document.addEventListener("visibilitychange", load);
    return () => {
      clearInterval(id);
      document.removeEventListener("visibilitychange", load);
    };
  });

  // The traffic tab also shows live speeds: a snapshot every 2 s while open.
  $effect(() => {
    if (tab !== "traffic") return;
    const tick = () => !document.hidden && api.snapshot().then((s) => (live = s)).catch(() => {});
    tick();
    const id = setInterval(tick, 2000);
    return () => clearInterval(id);
  });

  $effect(() => {
    if (tab !== "thermal" || range !== "live") return;
    snaps = [];
    extras = [];
    const tick = () => {
      if (document.hidden) return;
      Promise.all([api.snapshot(), api.extra()])
        .then(([s, e]) => {
          snaps = [...snaps.slice(1 - LIVE_POINTS), s];
          extras = [...extras.slice(1 - LIVE_POINTS), e];
          error = null;
        })
        .catch((e) => (error = errorText(e)));
    };
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  });

  $effect(() => {
    if (tab !== "fps") return;
    const tick = () => !document.hidden && api.fps().then((f) => (fps = f)).catch((e) => (error = errorText(e)));
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  });

  $effect(() => {
    if (tab !== "fps") return;
    const since = sessionRange === "week" ? Date.now() - 7 * 86_400_000 : 0;
    api.fpsSessions(since).then((s) => (sessions = s)).catch((e) => (error = errorText(e)));
  });
  async function forgetSessions() {
    try {
      await api.forgetFpsSessions();
      sessions = [];
    } catch (e) {
      error = errorText(e);
    }
  }
  const maxSessionFps = $derived(Math.max(60, ...sessions.map((s) => s.avg_fps)));
  const when = (ms: number) =>
    new Intl.DateTimeFormat(locale.current === "fa" ? "fa-IR" : "en-GB", { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }).format(new Date(ms));

  async function setUpFps(on: boolean) {
    fpsBusy = true;
    try {
      await api.setUpFps(on);
      fps = await api.fps();
    } catch (e) {
      error = errorText(e);
    } finally {
      fpsBusy = false;
    }
  }

  // The GPU that is doing the work: awake and reporting; a sleeping card is
  // never read (reading it would wake it).
  function workingGpu(s: Snapshot) {
    return s.gpus.find((g) => !g.asleep && g.temp_c != null) ?? null;
  }
  const liveTimes = $derived(snaps.map((s) => s.ts_unix_ms / 1000));
  const liveHasGpu = $derived(snaps.some((s) => workingGpu(s) != null));
  const lastSnap = $derived(snaps.at(-1) ?? null);
  const lastExtra = $derived(extras.at(-1) ?? null);
  function livePair(cpu: (s: Snapshot) => number | null, gpu: (s: Snapshot) => number | null) {
    return [
      { label: t().mon.cpu, color: CPU, values: snaps.map(cpu) },
      ...(liveHasGpu ? [{ label: t().mon.gpu, color: GPU, values: snaps.map(gpu) }] : []),
    ];
  }
  function gpuVal(s: Snapshot, f: (g: NonNullable<ReturnType<typeof workingGpu>>) => number | null): number | null {
    const g = workingGpu(s);
    return g ? f(g) : null;
  }
  const hasBattery = $derived(extras.some((e) => e.battery_w != null));

  const times = $derived(points.map((p) => p.ts_unix_ms / 1000));
  const hasGpu = $derived(points.some((p) => p.gpu_temp_c != null));
  function pair(cpu: (p: HistoryPoint) => number | null, gpu: (p: HistoryPoint) => number | null) {
    return [
      { label: t().mon.cpu, color: CPU, values: points.map(cpu) },
      ...(hasGpu ? [{ label: t().mon.gpu, color: GPU, values: points.map(gpu) }] : []),
    ];
  }
  // Under each history chart: each line's average and peak over the range.
  type Stat = [label: string, get: (p: HistoryPoint) => number | null, unit: string, digits: number];
  function mb(v: number | null): number | null {
    return v == null ? null : v / 1e6;
  }
  function figures(list: Stat[]) {
    return list
      .map(([label, get, unit, digits]) => {
        const vals = points.map(get).filter((v): v is number => v != null && Number.isFinite(v));
        return { label, unit, digits, n: vals.length, avg: vals.reduce((x, y) => x + y, 0) / Math.max(1, vals.length), peak: Math.max(...vals) };
      })
      .filter((r) => r.n > 0);
  }
  const timeStyle = $derived(range === "week" ? "weekday" : "clock");
  function series(a: [string, string, (p: HistoryPoint) => number | null], b?: [string, string, (p: HistoryPoint) => number | null]) {
    const one = (x: [string, string, (p: HistoryPoint) => number | null]) => ({ label: x[0], color: x[1], values: points.map(x[2]) });
    return [one(a), ...(b && points.some((p) => b[2](p) != null) ? [one(b)] : [])];
  }

</script>

{#snippet stats(list: Stat[])}
  <p class="figs">
    {#each figures(list) as f (f.label)}
      <span><b>{f.label}</b> {t().mon.avg} <bdi>{num(f.avg, f.digits)}{f.unit}</bdi> · {t().mon.peak} <bdi>{num(f.peak, f.digits)}{f.unit}</bdi></span>
    {/each}
  </p>
{/snippet}

<header class="head">
  <h1>{t().mon.title}</h1>
  {#if api.preview}<span class="badge">{t().preview}</span>{/if}
</header>

<div class="tabs" role="tablist">
  {#each ["fps", "thermal", "traffic", "ping"] as const as id}
    <button role="tab" aria-selected={tab === id} class:active={tab === id} onclick={() => (tab = id)}>{t().mon.tabs[id]}</button>
  {/each}
</div>

{#if error}<Problem text={error} />{/if}
{#if (tab === "traffic" || tab === "ping") && ctl.error}<Problem text={ctl.error} />{/if}

{#if tab === "fps"}
  <section class="card">
    <p class="muted">{t().mon.fpsTab.explain}</p>
    {#if fps?.state === "not-installed"}
      <p class="step">{t().mon.fpsTab.notInstalled}</p>
      <code class="cmd" dir="ltr">{fps.install_hint}</code>
    {:else if fps?.state === "not-set-up"}
      <p class="step">{t().mon.fpsTab.notSetUp}</p>
      <button class="btn primary" disabled={fpsBusy} onclick={() => setUpFps(true)}>{t().mon.fpsTab.setUp}</button>
    {:else if fps?.state === "idle"}
      <p class="step">{t().mon.fpsTab.idle}</p>
      <code class="cmd" dir="ltr">mangohud %command%</code>
      <p class="small">{t().mon.fpsTab.other}</p>
      {#if fps.live_unsupported}<p class="small">{t().mon.fpsTab.oldMangohud.replace("{v}", fps.mangohud_version)}</p>{/if}
      <button class="btn" disabled={fpsBusy} onclick={() => setUpFps(false)}>{t().mon.fpsTab.undo}</button>
    {/if}
  </section>
  {#if fps?.state === "live"}
    <div class="stats">
      <section class="card stat"><span class="label">{t().mon.fpsTab.live}</span><b dir="ltr">{fps.game}</b></section>
      <section class="card stat"><span class="label">{t().mon.fpsTab.fps}</span><b>{num(fps.fps)}</b></section>
      <section class="card stat"><span class="label">{t().mon.fpsTab.low}</span><b>{num(fps.low_1pct_fps)}</b></section>
      <section class="card stat"><span class="label">{t().mon.fpsTab.frametime}</span><b>{num(fps.frametime_ms, 1)} <small>{t().mon.fpsTab.ms}</small></b></section>
    </div>
    <section class="card">
      <h2>{t().mon.fpsTab.chart}</h2>
      <LineChart times={fps.recent.map((r) => r[0] / 1000)} series={[{ label: t().mon.fpsTab.fps, color: CPU, values: fps.recent.map((r) => r[1]) }]} waiting="" unit="" range={[0, 60]} height={200} />
    </section>
    <button class="btn" disabled={fpsBusy} onclick={() => setUpFps(false)}>{t().mon.fpsTab.undo}</button>
  {/if}
  <section class="card">
    <div class="sess-head">
      <h2>{t().mon.fpsTab.sessions}</h2>
      <Segmented label={t().mon.fpsTab.sessions} value={sessionRange} options={(["week", "all"] as const).map((v) => ({ value: v, label: t().mon.fpsTab.ranges[v] }))} onchange={(v) => (sessionRange = v)} />
    </div>
    {#if sessions.length === 0}
      <p class="muted">{t().mon.fpsTab.sessionsEmpty}</p>
    {:else}
      <ul class="sessions">
        {#each sessions as s (s.started_unix_ms + s.game)}
          {@const smooth = s.stutters <= Math.max(1, s.duration_s / 600)}
          <li>
            <div class="sess-top">
              <b dir="ltr">{s.game}</b>
              <span class="muted">{when(s.started_unix_ms)} · {num(s.duration_s / 60)} {t().mon.fpsTab.minutes}</span>
              <span class="tag" class:ok={smooth}>{smooth ? t().mon.fpsTab.smooth : `${num(s.stutters)} ${t().mon.fpsTab.stutters}`}</span>
            </div>
            {#each [[t().mon.fpsTab.avg, s.avg_fps], [t().mon.fpsTab.low, s.low_1pct_fps], [t().mon.fpsTab.low01, s.low_01pct_fps]] as const as [label, v]}
              <div class="meter">
                <span>{label}</span>
                <span class="track"><span style:--w={v / maxSessionFps}></span></span>
                <b dir="ltr">{num(v)}</b>
              </div>
            {/each}
          </li>
        {/each}
      </ul>
      <p class="small">{t().mon.fpsTab.howSmooth}</p>
      <button class="btn" onclick={forgetSessions}>{t().mon.fpsTab.forget}</button>
    {/if}
  </section>
{:else if tab === "thermal"}
  <div class="filters">
    <Segmented
      label={t().mon.tabs.thermal}
      value={range}
      options={(["live", "hour", "day", "week"] as const).map((r) => ({ value: r, label: t().mon.ranges[r] }))}
      onchange={(v) => (range = v)}
    />
  </div>
  {#if range === "live"}
    {@const g = lastSnap ? workingGpu(lastSnap) : null}
    <div class="stats">
      <section class="card stat"><span class="label">{t().mon.live.load1}</span><b>{num(lastExtra?.load_avg_1, 2)}</b></section>
      <section class="card stat"><span class="label">{t().mon.live.procs}</span><b>{num(lastExtra?.processes)}</b></section>
      <section class="card stat"><span class="label">{t().mon.live.busy}</span><b>{num(lastExtra?.disk_busy_pct)}٪</b></section>
      <section class="card stat">
        <span class="label">{t().mon.live.battery}</span>
        {#if lastExtra?.battery_pct != null}
          <b>{num(lastExtra.battery_pct)}٪ <small>{lastExtra.battery_w != null ? `${num(Math.abs(lastExtra.battery_w), 1)} ${t().unit.w}` : ""}{lastExtra.battery_status === "Charging" ? ` · ${t().mon.live.charging}` : ""}</small></b>
        {:else}
          <b class="muted">{t().mon.live.batteryNone}</b>
        {/if}
      </section>
    </div>
    <div class="charts">
      <section class="card">
        <h2>{t().mon.temp}</h2>
        <LineChart times={liveTimes} series={livePair((s) => s.cpu.temp_c, (s) => gpuVal(s, (g) => g.temp_c))} waiting={t().mon.live.waiting} height={150} />
      </section>
      <section class="card">
        <h2>{t().mon.load}</h2>
        <LineChart times={liveTimes} series={livePair((s) => s.cpu.load_pct, (s) => gpuVal(s, (g) => g.load_pct))} waiting={t().mon.live.waiting} unit="٪" range={[0, 100]} height={150} />
      </section>
      <section class="card">
        <h2>{t().mon.power}</h2>
        <LineChart times={liveTimes} series={livePair((s) => s.cpu.power_w, (s) => gpuVal(s, (g) => g.power_w))} waiting={t().mon.live.waiting} unit={` ${t().unit.w}`} range={[0, 30]} height={150} />
      </section>
      <section class="card">
        <h2>{t().mon.live.clocks} <small class="muted">{t().unit.mhz}</small></h2>
        <LineChart times={liveTimes} series={[
          { label: t().mon.live.cpuMax, color: CPU, values: snaps.map((s) => s.cpu.freq_max_mhz || null) },
          ...(liveHasGpu ? [{ label: t().mon.live.gpuClock, color: GPU, values: snaps.map((s) => gpuVal(s, (g) => g.core_clock_mhz || null)) }] : []),
        ]} waiting={t().mon.live.waiting} unit="" range={[0, 3000]} height={150} />
      </section>
      <section class="card">
        <h2>{t().mon.live.memory} <small class="muted">{t().unit.gb}</small></h2>
        <LineChart times={liveTimes} series={[
          { label: t().mon.live.ram, color: CPU, values: snaps.map((s) => (s.mem.total_kb - s.mem.available_kb) / 1_048_576) },
          { label: t().mon.live.swap, color: GPU, values: snaps.map((s) => s.mem.swap_used_kb / 1_048_576) },
        ]} waiting={t().mon.live.waiting} unit="" digits={1} range={[0, Math.ceil((lastSnap?.mem.total_kb ?? 0) / 1_048_576)]} height={150} />
      </section>
      <section class="card">
        <h2>{t().mon.live.disk} <small class="muted">MB/s</small></h2>
        <LineChart times={liveTimes} series={[
          { label: t().mon.live.read, color: CPU, values: extras.map((e) => e.disk_read_bps / 1e6) },
          { label: t().mon.live.write, color: GPU, values: extras.map((e) => e.disk_write_bps / 1e6) },
        ]} waiting={t().mon.live.waiting} unit="" digits={1} range={[0, 5]} height={150} />
      </section>
      <section class="card">
        <h2>{t().mon.live.network} <small class="muted">MB/s</small></h2>
        <LineChart times={liveTimes} series={[
          { label: t().mon.down, color: CPU, values: snaps.map((s) => s.net.rx_bps / 1e6) },
          { label: t().mon.up, color: GPU, values: snaps.map((s) => s.net.tx_bps / 1e6) },
        ]} waiting={t().mon.live.waiting} unit="" digits={2} range={[0, 1]} height={150} />
      </section>
      {#if hasBattery}
        <section class="card">
          <h2>{t().mon.live.battery}</h2>
          <LineChart times={liveTimes} series={[{ label: t().mon.live.battery, color: CPU, values: extras.map((e) => e.battery_w) }]} waiting={t().mon.live.waiting} unit={` ${t().unit.w}`} digits={1} range={[0, 20]} height={150} />
        </section>
      {/if}
      <section class="card">
        <h2>{t().mon.live.cores}</h2>
        {#if lastSnap}
          <div class="cores" dir="ltr">
            {#each lastSnap.cpu.per_core_load as l, i}
              <div class="core" title={`CPU ${i}: ${num(l)}٪ · ${num(lastExtra?.per_core_mhz[i])} ${t().unit.mhz}`}>
                <span class="core-bar"><span style:--h={(l ?? 0) / 100}></span></span>
                <span class="core-mhz">{lastExtra?.per_core_mhz[i] ? num(lastExtra.per_core_mhz[i] / 1000, 1) : "—"}</span>
              </div>
            {/each}
          </div>
          <p class="small">{t().unit.ghz}</p>
        {/if}
      </section>
    </div>
    {#if lastSnap && !g && lastSnap.gpus.some((x) => x.asleep)}<p class="small">{t().mon.live.gpuAsleep}</p>{/if}
    <p class="small">{t().mon.live.note}</p>
  {:else if loaded && points.length < 2}
    <section class="card"><p class="muted">{t().mon.empty}</p></section>
  {:else}
    <div class="charts">
      <section class="card">
        <h2>{t().mon.temp}</h2>
        <LineChart {times} series={pair((p) => p.cpu_temp_c, (p) => p.gpu_temp_c)} waiting="" mode="history" height={160} {timeStyle} />
        {@render stats([[t().mon.cpu, (p) => p.cpu_temp_c, "°", 0], [t().mon.gpu, (p) => p.gpu_temp_c, "°", 0]])}
      </section>
      <section class="card">
        <h2>{t().mon.load}</h2>
        <LineChart {times} series={pair((p) => p.cpu_load_pct, (p) => p.gpu_load_pct)} waiting="" unit="٪" range={[0, 100]} mode="history" height={160} {timeStyle} />
        {@render stats([[t().mon.cpu, (p) => p.cpu_load_pct, "٪", 0], [t().mon.gpu, (p) => p.gpu_load_pct, "٪", 0]])}
      </section>
      <section class="card">
        <h2>{t().mon.power}</h2>
        <LineChart {times} series={pair((p) => p.cpu_w, (p) => p.gpu_w)} waiting="" unit={` ${t().unit.w}`} range={[0, 30]} mode="history" height={160} {timeStyle} />
        {@render stats([[t().mon.cpu, (p) => p.cpu_w, ` ${t().unit.w}`, 1], [t().mon.gpu, (p) => p.gpu_w, ` ${t().unit.w}`, 1]])}
      </section>
      <section class="card">
        <h2>{t().mon.live.clocks} <small class="muted">{t().unit.mhz}</small></h2>
        <LineChart {times} series={series([t().mon.live.cpuMax, CPU, (p) => p.cpu_mhz], [t().mon.live.gpuClock, GPU, (p) => p.gpu_mhz])} waiting="" unit="" range={[0, 3000]} mode="history" height={160} {timeStyle} />
        {@render stats([[t().mon.live.cpuMax, (p) => p.cpu_mhz, ` ${t().unit.mhz}`, 0], [t().mon.live.gpuClock, (p) => p.gpu_mhz, ` ${t().unit.mhz}`, 0]])}
      </section>
      <section class="card">
        <h2>{t().mon.live.memory} <small class="muted">{t().unit.gb}</small></h2>
        <LineChart {times} series={series([t().mon.live.ram, CPU, (p) => p.mem_used_kb / 1_048_576], [t().mon.live.swap, GPU, (p) => (p.swap_used_kb == null ? null : p.swap_used_kb / 1_048_576)])} waiting="" unit="" digits={1} mode="history" height={160} {timeStyle} />
        {@render stats([[t().mon.live.ram, (p) => p.mem_used_kb / 1_048_576, ` ${t().unit.gb}`, 1], [t().mon.live.swap, (p) => (p.swap_used_kb == null ? null : p.swap_used_kb / 1_048_576), ` ${t().unit.gb}`, 2]])}
      </section>
      <section class="card">
        <h2>{t().mon.live.network} <small class="muted">MB/s</small></h2>
        <LineChart {times} series={series([t().mon.down, CPU, (p) => mb(p.net_rx_bps)], [t().mon.up, GPU, (p) => mb(p.net_tx_bps)])} waiting="" unit="" digits={2} range={[0, 1]} mode="history" height={160} {timeStyle} />
        {@render stats([[t().mon.down, (p) => mb(p.net_rx_bps), " MB/s", 2], [t().mon.up, (p) => mb(p.net_tx_bps), " MB/s", 2]])}
      </section>
      <section class="card">
        <h2>{t().mon.live.disk} <small class="muted">MB/s</small></h2>
        <LineChart {times} series={series([t().mon.live.read, CPU, (p) => mb(p.disk_read_bps)], [t().mon.live.write, GPU, (p) => mb(p.disk_write_bps)])} waiting="" unit="" digits={1} range={[0, 5]} mode="history" height={160} {timeStyle} />
        {@render stats([[t().mon.live.read, (p) => mb(p.disk_read_bps), " MB/s", 1], [t().mon.live.write, (p) => mb(p.disk_write_bps), " MB/s", 1], [t().mon.live.busy, (p) => p.disk_busy_pct, "٪", 0]])}
      </section>
      <section class="card">
        <h2>{t().mon.live.load1}</h2>
        <LineChart {times} series={series([t().mon.live.load1, CPU, (p) => p.load_avg_1])} waiting="" unit="" digits={2} range={[0, 4]} mode="history" height={160} {timeStyle} />
        {@render stats([[t().mon.live.load1, (p) => p.load_avg_1, "", 2], [t().mon.live.procs, (p) => p.processes, "", 0]])}
      </section>
      {#if points.some((p) => p.battery_w != null)}
        <section class="card">
          <h2>{t().mon.live.battery}</h2>
          <LineChart {times} series={series([t().mon.live.battery, CPU, (p) => p.battery_w])} waiting="" unit={` ${t().unit.w}`} digits={1} range={[0, 20]} mode="history" height={160} {timeStyle} />
        {@render stats([[t().mon.live.battery, (p) => p.battery_w, ` ${t().unit.w}`, 1]])}
        </section>
      {/if}
    </div>
    <p class="small">{t().mon.note}</p>
  {/if}
{:else if tab === "ping"}
  <PingPanel {ctl} />
{:else}
  <section class="card">
    <h2>{t().mon.now}</h2>
    {#if live}
      {@const d = rate(live.net.rx_bps)}
      {@const u = rate(live.net.tx_bps)}
      <div class="speeds">
        <span><ArrowDown size={18} aria-hidden="true" /> {t().mon.down} <b dir="ltr">{d.value} {d.unit}</b></span>
        <span><ArrowUp size={18} aria-hidden="true" /> {t().mon.up} <b dir="ltr">{u.value} {u.unit}</b></span>
        <span class="muted" dir="ltr">{live.net.iface}</span>
      </div>
    {/if}
  </section>
  <TrafficTiles />
  <TrafficPeriods {ctl} />
  <NetRules {ctl} />
{/if}

<style>
  .head { display: flex; align-items: center; gap: 14px; margin-bottom: 16px; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 800; }
  h2 { margin: 0 0 10px; font-size: 0.98rem; font-weight: 700; }
  .badge { padding: 3px 12px; border-radius: 999px; font-size: 0.8rem; background: var(--card-2); color: var(--text-2); }
  .tabs { display: flex; gap: 4px; border-bottom: 1px solid var(--line); margin-bottom: 16px; }
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
  .filters { margin-bottom: 14px; }
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; margin-bottom: 16px; min-width: 0; }
  .charts { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 16px; }
  .charts .card { margin: 0; }
  @media (max-width: 1150px) { .charts { grid-template-columns: minmax(0, 1fr); } }
  .muted { color: var(--text-2); margin: 0; font-size: 0.9rem; }
  .small { color: var(--text-3); font-size: 0.8rem; margin: 10px 0 0; }
  .sess-head { display: flex; justify-content: space-between; align-items: center; gap: 12px; margin-bottom: 10px; }
  .sess-head h2 { margin: 0; }
  .sessions { list-style: none; margin: 0; padding: 0; display: grid; gap: 14px; }
  .sessions li { padding-bottom: 14px; border-bottom: 1px solid var(--line); display: grid; gap: 6px; }
  .sessions li:last-child { border-bottom: 0; padding-bottom: 0; }
  .sess-top { display: flex; gap: 12px; align-items: baseline; flex-wrap: wrap; }
  .tag { margin-inline-start: auto; padding: 2px 10px; border-radius: 999px; font-size: 0.75rem; background: var(--card-2); color: var(--warn, #fbbf24); }
  .tag.ok { color: var(--ok, #34d399); }
  .meter { display: grid; grid-template-columns: 7em 1fr 3em; gap: 10px; align-items: center; font-size: 0.82rem; color: var(--text-2); }
  .meter b { color: var(--text); text-align: end; }
  .track { height: 8px; border-radius: 4px; background: var(--card-2); overflow: hidden; }
  .track span { display: block; height: 100%; width: calc(var(--w) * 100%); background: #8f6af2; }
  .stats { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 16px; margin-bottom: 16px; }
  .stats .card { margin: 0; }
  @media (max-width: 900px) { .stats { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
  .stat { display: grid; gap: 4px; }
  .stat .label { color: var(--text-2); font-size: 0.85rem; }
  .stat b { font-size: 1.45rem; font-weight: 800; }
  .stat small { font-size: 0.8rem; font-weight: 500; color: var(--text-2); }
  h2 small { font-weight: 500; font-size: 0.8rem; }
  .step { margin: 12px 0 10px; font-size: 0.92rem; }
  .cmd { display: inline-block; padding: 8px 12px; border-radius: 8px; background: var(--card-2); font-size: 0.9rem; user-select: all; margin-bottom: 8px; }
  .btn { all: unset; padding: 8px 16px; border-radius: 10px; background: var(--card-2); color: var(--text); font-size: 0.9rem; cursor: pointer; margin-top: 10px; }
  .btn.primary { background: var(--accent); color: white; font-weight: 600; }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .cores { display: flex; gap: 6px; align-items: flex-end; height: 110px; }
  .core { flex: 1; display: flex; flex-direction: column; align-items: center; gap: 4px; height: 100%; min-width: 0; }
  .core-bar { flex: 1; width: 100%; max-width: 22px; background: var(--card-2); border-radius: 4px; display: flex; align-items: flex-end; overflow: hidden; }
  .core-bar span { display: block; width: 100%; height: calc(var(--h) * 100%); background: #8f6af2; }
  .core-mhz { font-size: 0.68rem; color: var(--text-3); }
  .speeds { display: flex; flex-wrap: wrap; gap: 12px 36px; align-items: center; font-size: 0.95rem; }
  .speeds span { display: inline-flex; align-items: center; gap: 8px; }
  .speeds b { font-size: 1.3rem; }
  .figs { display: flex; flex-wrap: wrap; gap: 4px 14px; margin: 8px 0 0; font-size: 0.78rem; color: var(--text-3); }
  .figs b { color: var(--text-2); font-weight: 600; }
  .figs bdi { color: var(--text); }
</style>
