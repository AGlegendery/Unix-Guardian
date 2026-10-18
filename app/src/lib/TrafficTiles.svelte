<script lang="ts">
  // The speed right now (two boxes, from the live snapshot the page reads)
  // and how much the internet was used: the last hour, today, 7 and 30
  // days, from the agent's minute history of the interface counters
  // (exact). The reset in the corner starts the counts again from now.
  import HugeIcon from "./icons/HugeIcon.svelte";
  import { api, errorText, type DayTraffic, type Snapshot, type TrafficBucket } from "./api";
  import { num, rate } from "./format";
  import { t } from "./i18n.svelte";

  let { live = null }: { live?: Snapshot | null } = $props();

  const HOUR = 3_600_000;
  let days = $state<DayTraffic[]>([]);
  let lastHour = $state<TrafficBucket | null>(null);
  let today = $state<TrafficBucket | null>(null);
  let error = $state<string | null>(null);

  function midnight(d = new Date()): Date {
    const m = new Date(d);
    m.setHours(0, 0, 0, 0);
    return m;
  }

  // Asks twice: a second press within a few seconds resets.
  let armed = $state(false);
  let said = $state<string | null>(null);
  let armTimer: ReturnType<typeof setTimeout> | undefined;
  let reload = () => {};
  async function reset() {
    if (!armed) {
      armed = true;
      clearTimeout(armTimer);
      armTimer = setTimeout(() => (armed = false), 4000);
      return;
    }
    armed = false;
    try {
      said = (await api.resetTraffic()) ? t().settings.trafficResetDone : t().settings.trafficResetFailed;
      reload();
    } catch (e) {
      error = errorText(e);
    }
  }

  // Changes once a minute: reload every minute while visible.
  $effect(() => {
    const load = () => {
      if (document.hidden) return;
      const now = Date.now();
      const start = midnight().getTime();
      Promise.all([
        api.traffic(midnight(new Date(now - 29 * 86_400_000)).getTime()),
        api.trafficBuckets(now - HOUR, now, HOUR),
        api.trafficBuckets(start, now, Math.max(60_000, now - start)),
      ])
        .then(([d, h, td]) => {
          days = d;
          lastHour = h[0] ?? null;
          today = td[0] ?? null;
          error = null;
        })
        .catch((e) => (error = errorText(e)));
    };
    load();
    reload = load;
    const id = setInterval(load, 60_000);
    document.addEventListener("visibilitychange", load);
    return () => {
      clearInterval(id);
      document.removeEventListener("visibilitychange", load);
    };
  });

  const sum = (list: { rx_bytes: number; tx_bytes: number }[]) =>
    list.reduce((a, d) => ({ rx: a.rx + d.rx_bytes, tx: a.tx + d.tx_bytes }), { rx: 0, tx: 0 });
  const tiles = $derived([
    { label: t().ping.usage.hour, ...sum(lastHour ? [lastHour] : []) },
    { label: t().ping.usage.today, ...sum(today ? [today] : []) },
    { label: t().ping.usage.week, ...sum(days.slice(-7)) },
    { label: t().ping.usage.month, ...sum(days) },
  ]);
  function size(b: number): string {
    return b >= 1e9 ? `${num(b / 1e9, 2)} ${t().mon.gb}` : b >= 1e6 ? `${num(b / 1e6, b >= 1e8 ? 0 : 1)} ${t().mon.mb}` : `${num(b / 1e3)} KB`;
  }
</script>

{#if error}<p class="problem">{error}</p>{/if}
<section class="card" data-cat="net">
  <div class="head">
    <h2>{t().ping.usage.title}</h2>
    <button class="reset" class:armed onclick={reset} title={t().settings.trafficNote}>
      <HugeIcon name="refresh" size={15} /> {armed ? t().settings.trafficResetConfirm : t().settings.trafficReset}
    </button>
  </div>
  {#if said}<p class="said" role="status">{said}</p>{/if}
  <div class="tiles">
    {#each [["down", live?.net.rx_bps], ["up", live?.net.tx_bps]] as [dir, bps]}
      {@const r = bps == null ? null : rate(bps as number)}
      <div class="tile now">
        <span class="label"><HugeIcon name={dir === "down" ? "arrow-down-02" : "arrow-up-02"} size={15} /> {dir === "down" ? t().ping.usage.downNow : t().ping.usage.upNow}</span>
        <b><bdi>{r ? `${r.value} ${r.unit}` : "—"}</bdi></b>
        <span class="sub"><bdi>{live?.net.iface ?? ""}</bdi></span>
      </div>
    {/each}
    {#each tiles as tile}
      <div class="tile">
        <span class="label">{tile.label}</span>
        <b><bdi>{size(tile.rx + tile.tx)}</bdi></b>
        <span class="sub">↓ <bdi>{size(tile.rx)}</bdi> · ↑ <bdi>{size(tile.tx)}</bdi></span>
      </div>
    {/each}
  </div>
  <p class="small">{t().ping.usage.note}</p>
</section>

<style>
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; margin-bottom: 16px; min-width: 0; }
  h2 { margin: 0 0 12px; font-size: 0.98rem; font-weight: 700; }
  .problem { margin: 0 0 14px; padding: 12px 16px; border-radius: var(--radius-sm); background: color-mix(in oklab, var(--danger) 12%, var(--card)); color: var(--danger); font-size: 0.9rem; }
  .tiles { display: grid; grid-template-columns: repeat(6, minmax(0, 1fr)); gap: 12px; }
  @media (max-width: 1250px) { .tiles { grid-template-columns: repeat(3, minmax(0, 1fr)); } }
  @media (max-width: 700px) { .tiles { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
  .tile.now { background: color-mix(in oklab, var(--cat) 12%, var(--card-2)); }
  .tile.now .label { display: inline-flex; align-items: center; gap: 6px; color: var(--cat); }
  .head { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 12px; }
  .head h2 { margin: 0; }
  .reset { all: unset; display: inline-flex; align-items: center; gap: 6px; padding: 5px 12px; border-radius: 9px; background: var(--card-2); color: var(--text-2); font-size: 0.82rem; cursor: pointer; }
  .reset:hover { color: var(--text); }
  .reset.armed { background: color-mix(in oklab, var(--danger) 22%, var(--card-2)); color: var(--danger); font-weight: 700; }
  .reset:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .said { margin: -4px 0 10px; color: var(--ok); font-size: 0.84rem; }
  .tile { display: grid; gap: 4px; padding: 12px 14px; border-radius: var(--radius-sm); background: var(--card-2); }
  .tile .label { color: var(--text-2); font-size: 0.82rem; }
  .tile b { font-size: 1.35rem; font-weight: 800; }
  .sub { color: var(--text-2); font-size: 0.8rem; }
  .small { color: var(--text-3); font-size: 0.8rem; margin: 10px 0 0; }
</style>
