<script lang="ts">
  // How much the internet was used: the last hour, today, 7 and 30 days.
  // From the agent's minute history of the interface counters (exact).
  import { api, errorText, type DayTraffic, type TrafficBucket } from "./api";
  import { num } from "./format";
  import { t } from "./i18n.svelte";

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
<section class="card">
  <h2>{t().ping.usage.title}</h2>
  <div class="tiles">
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
  .tiles { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 12px; }
  @media (max-width: 900px) { .tiles { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
  .tile { display: grid; gap: 4px; padding: 12px 14px; border-radius: var(--radius-sm); background: var(--card-2); }
  .tile .label { color: var(--text-2); font-size: 0.82rem; }
  .tile b { font-size: 1.35rem; font-weight: 800; }
  .sub { color: var(--text-2); font-size: 0.8rem; }
  .small { color: var(--text-3); font-size: 0.8rem; margin: 10px 0 0; }
</style>
