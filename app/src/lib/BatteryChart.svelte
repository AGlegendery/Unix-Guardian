<script lang="ts">
  // Charge over time with a band underneath saying where the power came
  // from: battery, the charger, or nothing recorded (off or asleep). Plain
  // SVG drawn once per data change; nothing animates while it is shown.
  import type { BatteryReport } from "./api";
  import { num } from "./format";
  import { locale, t } from "./i18n.svelte";

  let { report, since, until }: { report: BatteryReport; since: number; until: number } = $props();

  let width = $state(600);
  const H = 190;
  const PAD = { l: 34, r: 10, t: 10, b: 44 };
  const plotW = $derived(Math.max(100, width - PAD.l - PAD.r));
  const plotH = H - PAD.t - PAD.b;
  const x = (ms: number) => PAD.l + ((ms - since) / Math.max(1, until - since)) * plotW;
  const y = (pct: number) => PAD.t + (1 - pct / 100) * plotH;

  // Breaks the line where readings are more than three minutes apart.
  const segments = $derived.by(() => {
    const out: [number, number][][] = [];
    let cur: [number, number][] = [];
    let last = 0;
    for (const [ts, pct] of report.points) {
      if (pct == null || ts < since) continue;
      if (cur.length && ts - last > 180_000) {
        out.push(cur);
        cur = [];
      }
      cur.push([x(ts), y(pct)]);
      last = ts;
    }
    if (cur.length) out.push(cur);
    return out;
  });
  const line = (seg: [number, number][]) => seg.map(([a, b], i) => `${i ? "L" : "M"}${a.toFixed(1)},${b.toFixed(1)}`).join("");
  const area = (seg: [number, number][]) =>
    `${line(seg)}L${seg[seg.length - 1][0].toFixed(1)},${y(0)}L${seg[0][0].toFixed(1)},${y(0)}Z`;

  const days = $derived(until - since > 30 * 3_600_000);
  const ticks = $derived.by(() => {
    const out: number[] = [];
    const t0 = new Date(since);
    if (days) {
      t0.setHours(24, 0, 0, 0);
      for (let t1 = t0.getTime(); t1 < until; t1 += 86_400_000) out.push(t1);
      return out;
    }
    // Every hour, thinned to every 3 or 6 so labels never crowd.
    const hours = (until - since) / 3_600_000;
    const every = hours / (plotW / 70) > 3 ? 6 : hours / (plotW / 70) > 1 ? 3 : 1;
    t0.setMinutes(60, 0, 0);
    for (let t1 = t0.getTime(); t1 < until; t1 += 3_600_000) {
      if (new Date(t1).getHours() % every === 0) out.push(t1);
    }
    return out;
  });
  const fmtTick = $derived(
    new Intl.DateTimeFormat(locale.current === "fa" ? "fa-IR" : "en-GB", days ? { weekday: "short" } : { hour: "2-digit", minute: "2-digit" }),
  );
  const fmtTip = $derived(
    new Intl.DateTimeFormat(locale.current === "fa" ? "fa-IR" : "en-GB", { weekday: days ? "short" : undefined, hour: "2-digit", minute: "2-digit" }),
  );

  let hover = $state<number | null>(null);
  function move(e: PointerEvent) {
    const rect = (e.currentTarget as SVGElement).getBoundingClientRect();
    const px = e.clientX - rect.left;
    if (px < PAD.l || px > PAD.l + plotW || !report.points.length) {
      hover = null;
      return;
    }
    const ms = since + ((px - PAD.l) / plotW) * (until - since);
    let best = -1;
    let d = Infinity;
    report.points.forEach((p, i) => {
      const dd = Math.abs(p[0] - ms);
      if (dd < d) {
        d = dd;
        best = i;
      }
    });
    hover = d < 10 * 60_000 ? best : null;
  }
  const tip = $derived(hover == null ? null : report.points[hover]);
</script>

<div class="wrap" dir="ltr" bind:clientWidth={width}>
  <svg
    width={width}
    height={H}
    role="img"
    aria-label={t().sys.hist.chart}
    onpointermove={move}
    onpointerleave={() => (hover = null)}
  >
    <defs>
      <linearGradient id="bat-fill" x1="0" x2="0" y1="0" y2="1">
        <stop offset="0" stop-color="var(--accent)" stop-opacity="0.35" />
        <stop offset="1" stop-color="var(--accent)" stop-opacity="0.02" />
      </linearGradient>
      <pattern id="bat-off" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
        <rect width="6" height="6" fill="var(--card-2)" />
        <line x1="0" y1="0" x2="0" y2="6" stroke="var(--line)" stroke-width="2" />
      </pattern>
    </defs>
    {#each [0, 25, 50, 75, 100] as g}
      <line class="grid" x1={PAD.l} x2={PAD.l + plotW} y1={y(g)} y2={y(g)} />
      <text class="ylab" x={PAD.l - 6} y={y(g) + 4} text-anchor="end">{num(g)}</text>
    {/each}
    {#each segments as seg}
      <path d={area(seg)} fill="url(#bat-fill)" />
      <path d={line(seg)} class="charge" />
    {/each}
    {#each report.periods as p}
      {@const a = Math.max(x(p.start_unix_ms), PAD.l)}
      {@const b = Math.min(x(p.end_unix_ms), PAD.l + plotW)}
      {#if b > a}
        <rect class="band {p.kind}" x={a} y={H - PAD.b + 8} width={Math.max(1.5, b - a)} height="10" rx="2" fill={p.kind === "off" ? "url(#bat-off)" : undefined} />
      {/if}
    {/each}
    {#each ticks as tk}
      <text class="xlab" x={x(tk)} y={H - 6} text-anchor="middle">{fmtTick.format(new Date(tk))}</text>
    {/each}
    {#if tip}
      <line class="cursor" x1={x(tip[0])} x2={x(tip[0])} y1={PAD.t} y2={H - PAD.b + 18} />
      {#if tip[1] != null}<circle cx={x(tip[0])} cy={y(tip[1])} r="4" class="dot" />{/if}
    {/if}
  </svg>
  {#if tip}
    <div class="tip" style:left="{Math.min(Math.max(x(tip[0]) - 70, 0), width - 150)}px">
      <b>{fmtTip.format(new Date(tip[0]))}</b>
      <span>{tip[1] == null ? "—" : `${num(tip[1])}${t().unit.pct}`}</span>
      <span>{tip[3] ? t().sys.hist.ac : t().sys.hist.battery}{tip[2] != null && Math.abs(tip[2]) > 0.2 ? ` · ${num(Math.abs(tip[2]), 1)} ${t().unit.w}` : ""}</span>
    </div>
  {/if}
  <div class="legend">
    <span><i class="sw battery"></i>{t().sys.hist.battery}</span>
    <span><i class="sw ac"></i>{t().sys.hist.ac}</span>
    <span><i class="sw off"></i>{t().sys.hist.off}</span>
  </div>
</div>

<style>
  .wrap { position: relative; min-width: 0; overflow: hidden; }
  svg { display: block; touch-action: none; }
  .grid { stroke: var(--line); stroke-width: 1; stroke-dasharray: 2 4; }
  .ylab, .xlab { fill: var(--text-3); font-size: 11px; font-family: inherit; }
  .charge { fill: none; stroke: var(--accent-text); stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; }
  .band.battery { fill: var(--warn); }
  .band.ac { fill: var(--ok); opacity: 0.85; }
  .cursor { stroke: var(--text-3); stroke-width: 1; }
  .dot { fill: var(--accent-text); stroke: var(--card); stroke-width: 2; }
  .tip {
    position: absolute;
    top: 0;
    width: 150px;
    display: grid;
    gap: 2px;
    padding: 8px 10px;
    border-radius: 10px;
    background: var(--card-2);
    border: 1px solid var(--line);
    font-size: 0.78rem;
    pointer-events: none;
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.35);
  }
  .legend { display: flex; gap: 18px; flex-wrap: wrap; font-size: 0.78rem; color: var(--text-2); margin-top: 4px; }
  .legend span { display: inline-flex; align-items: center; gap: 6px; }
  .sw { width: 14px; height: 8px; border-radius: 2px; display: inline-block; }
  .sw.battery { background: var(--warn); }
  .sw.ac { background: var(--ok); }
  .sw.off { background: repeating-linear-gradient(45deg, var(--card-2) 0 3px, var(--line) 3px 5px); }
</style>
