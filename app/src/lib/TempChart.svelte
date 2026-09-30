<script lang="ts">
  // Temperature over the last two minutes. One unit, one axis: load is a
  // different measure and lives in the tiles, never on a second y-scale.
  // Series colours passed the dataviz validator on the card surface (dark):
  // lightness band, CVD separation 31.6, contrast >= 3:1.
  import uPlot from "uplot";
  import "uplot/dist/uPlot.min.css";
  import { num } from "./format";

  type Series = { label: string; color: string; values: (number | null)[] };
  let { times, series, waiting }: { times: number[]; series: Series[]; waiting: string } = $props();

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
        height: 190,
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
        scales: { x: { time: true }, y: { range: (_u, lo, hi) => [Math.min(30, Math.floor(lo - 3)), Math.max(70, Math.ceil(hi + 3))] } },
        axes: [
          { ...axis, values: (_u, ticks) => ticks.map(() => "") , size: 10 },
          { ...axis, size: 42, values: (_u, ticks) => ticks.map((v) => `${num(v)}°`) },
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
  const key = $derived(series.map((s) => s.label).join("|"));
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
    <div class="tip" style:left="{hover.left + 50}px">
      <span class="tip-when">−{num(ago)}s</span>
      {#each series as s}
        <span class="tip-row"><span class="key" style:background={s.color}></span>{s.label} {num(s.values[hover.idx])}°</span>
      {/each}
    </div>
  {/if}
  {#if times.length < 2}
    <p class="waiting">{waiting}</p>
  {/if}
</div>

<style>
  .chart { position: relative; min-height: 190px; }
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
