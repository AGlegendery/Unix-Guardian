<script lang="ts">
  // A time series in one unit on one axis (never a second y-scale: charts of
  // different measures are separate charts). Series colours passed the
  // dataviz validator on the card surface (dark): lightness band, CVD
  // separation 31.6, contrast >= 3:1.
  //
  // `live`: the last minutes, time axis unlabelled, hover shows "−Ns".
  // `history`: a longer range, local times on the axis and in the hover.
  import uPlot from "uplot";
  import "uplot/dist/uPlot.min.css";
  import { num } from "./format";
  import { locale } from "./i18n.svelte";

  type Series = { label: string; color: string; values: (number | null)[] };
  let {
    times,
    series,
    waiting,
    unit = "°",
    range = [30, 70],
    mode = "live",
    height = 190,
    timeStyle = "clock",
  }: {
    times: number[];
    series: Series[];
    waiting: string;
    unit?: string;
    /// The y axis always shows at least this span, so a flat line is not blown up.
    range?: [number, number];
    mode?: "live" | "history";
    height?: number;
    /// History axis labels: clock times (a day or less) or weekdays (a week).
    timeStyle?: "clock" | "weekday";
  } = $props();

  const timeFmt = $derived(
    new Intl.DateTimeFormat(
      locale.current === "fa" ? "fa-IR" : "en-GB",
      timeStyle === "weekday" ? { weekday: "short" } : { hour: "2-digit", minute: "2-digit" },
    ),
  );
  const tipFmt = $derived(
    new Intl.DateTimeFormat(locale.current === "fa" ? "fa-IR" : "en-GB", { weekday: "short", hour: "2-digit", minute: "2-digit" }),
  );

  let host: HTMLDivElement;
  let plot: uPlot | null = null;
  let width = $state(0);
  // Hover: the crosshair's sample, shown in a small tooltip beside it.
  let hover = $state<{ idx: number; left: number } | null>(null);

  function css(name: string) {
    return getComputedStyle(host).getPropertyValue(name).trim();
  }

  function build() {
    plot?.destroy();
    const grid = { stroke: css("--line"), width: 1 };
    const axis = { stroke: css("--text-3"), grid, ticks: { show: false }, font: "12px Vazirmatn Variable" };
    plot = new uPlot(
      {
        width: Math.max(200, width),
        height,
        padding: [8, 8, 0, 0],
        legend: { show: false },
        cursor: { points: { size: 8, width: 2, stroke: css("--card") }, y: false },
        hooks: {
          setCursor: [
            (u) => {
              const idx = u.cursor.idx;
              hover = idx == null || u.cursor.left == null || u.cursor.left < 0 ? null : { idx, left: u.cursor.left };
            },
          ],
        },
        scales: {
          x: { time: true },
          y: { range: (_u, lo, hi) => [Math.min(range[0], Math.floor(lo - 3)), Math.max(range[1], Math.ceil(hi + 3))] },
        },
        axes: [
          mode === "live"
            ? { ...axis, values: (_u, ticks) => ticks.map(() => ""), size: 10 }
            : { ...axis, size: 28, space: 70, values: (_u, ticks) => ticks.map((v) => timeFmt.format(new Date(v * 1000))) },
          { ...axis, size: 48, values: (_u, ticks) => ticks.map((v) => `${num(v)}${unit}`) },
        ],
        series: [
          {},
          ...series.map((s) => ({
            label: s.label,
            stroke: s.color,
            width: 2,
            fill: s.color + "1a",
            points: { show: false },
            spanGaps: false,
          })),
        ],
      },
      [times, ...series.map((s) => s.values)] as uPlot.AlignedData,
      host,
    );
  }

  // Rebuild when the set of series or the width changes; otherwise only feed data.
  const key = $derived(series.map((s) => s.label).join("|") + timeStyle + unit);
  $effect(() => {
    void key;
    if (width > 0) build();
    return () => {
      plot?.destroy();
      plot = null;
    };
  });
  $effect(() => {
    plot?.setData([times, ...series.map((s) => s.values)] as uPlot.AlignedData);
  });
</script>

<div class="chart" dir="ltr" bind:clientWidth={width}>
  {#if series.length > 1}
    <ul class="legend" dir="auto">
      {#each series as s}
        <li><span class="key" style:background={s.color}></span>{s.label}</li>
      {/each}
    </ul>
  {/if}
  <div bind:this={host}></div>
  {#if hover && times[hover.idx] != null}
    {@const ago = Math.round(times[times.length - 1] - times[hover.idx])}
    <div class="tip" style:left="{hover.left + 56}px">
      <span class="tip-when">{mode === "live" ? `−${num(ago)}s` : tipFmt.format(new Date(times[hover.idx] * 1000))}</span>
      {#each series as s}
        <span class="tip-row"><span class="key" style:background={s.color}></span>{s.label} {num(s.values[hover.idx])}{unit}</span>
      {/each}
    </div>
  {/if}
  {#if times.length < 2}
    <p class="waiting">{waiting}</p>
  {/if}
</div>

<style>
  .chart { position: relative; }
  .legend { list-style: none; margin: 0 0 6px; padding: 0; display: flex; gap: 16px; font-size: 0.82rem; color: var(--text-2); }
  .legend li { display: flex; align-items: center; gap: 6px; }
  .key { width: 14px; height: 3px; border-radius: 2px; }
  .waiting { position: absolute; inset: 0; display: grid; place-items: center; margin: 0; color: var(--text-3); font-size: 0.85rem; }
  .chart :global(.u-over) { cursor: crosshair; }
  .tip {
    position: absolute;
    top: 20px;
    transform: translateX(-50%);
    background: var(--card-2);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 6px 10px;
    font-size: 0.78rem;
    display: grid;
    gap: 2px;
    pointer-events: none;
    box-shadow: 0 6px 18px -8px rgb(0 0 0 / 0.6);
    white-space: nowrap;
  }
  .tip-when { color: var(--text-3); }
  .tip-row { display: flex; align-items: center; gap: 6px; }
</style>
