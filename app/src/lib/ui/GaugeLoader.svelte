<script lang="ts">
  // The G gauge in motion. "busy" loops while something is being waited for;
  // "intro" draws the mark once at startup and then stands still.
  //
  // WebKitGTK repaints the whole window for any running animation, so the
  // loop stops by itself after `maxSeconds` (a wait that long is not going
  // to end soon, and an endless loop costs a CPU core), and the intro has no
  // idle motion afterwards.
  let {
    variant = "busy",
    size = 48,
    label = "Loading",
    maxSeconds = 20,
  }: { variant?: "busy" | "intro"; size?: number; label?: string; maxSeconds?: number } = $props();

  let resting = $state(false);
  $effect(() => {
    if (variant !== "busy") return;
    resting = false;
    const id = setTimeout(() => (resting = true), maxSeconds * 1000);
    return () => clearTimeout(id);
  });
</script>

<svg class="ga {variant}" class:resting width={size} height={size} viewBox="16 18 68 68" role="img" aria-label={label}>
  <circle class="glow" cx="50" cy="52" r="10" />
  <path class="arc" pathLength="100" d="M69.8 32.2 A28 28 0 1 0 78 52" />
  <path class="arc2" pathLength="100" d="M69.8 32.2 A28 28 0 1 0 78 52" />
  <g class="needle"><path d="M53 52 H78" /></g>
  <circle class="hub" cx="50" cy="52" r="6" />
</svg>

<style>
  .ga { --fg: var(--ug-fg, #f3eeff); --ac: var(--ug-accent, #b794ff); overflow: visible; flex: none; }
  .arc, .arc2 { fill: none; stroke: var(--fg); stroke-width: 8; stroke-linecap: round; stroke-dasharray: 100; stroke-dashoffset: 0; }
  .arc2 { stroke: var(--ac); stroke-dasharray: 14 86; opacity: 0; }
  .needle, .hub, .glow { transform-box: view-box; transform-origin: 50px 52px; }
  .needle path { fill: none; stroke: var(--ac); stroke-width: 8; stroke-linecap: round; }
  .hub { fill: var(--ac); }
  .glow { fill: var(--ac); opacity: 0; }
  .busy .arc { opacity: 0.35; }
  .busy .arc2 { opacity: 1; animation: run 1.6s linear infinite; }
  .busy .needle { animation: swing 1.6s ease-in-out infinite alternate; }
  .busy.resting .arc2, .busy.resting .needle { animation: none; }
  .intro .arc { animation: draw 1.2s cubic-bezier(0.6, 0, 0.3, 1) 0.15s both; }
  .intro .needle { animation: sweep 1.65s cubic-bezier(0.6, 0, 0.3, 1) 0.15s both; }
  .intro .hub { animation: pop 0.5s cubic-bezier(0.3, 1.6, 0.5, 1) 1.2s both; }
  .intro .glow { animation: glow 1.1s ease-out 1.3s both; }
  @keyframes run { from { stroke-dashoffset: 100; } to { stroke-dashoffset: 0; } }
  @keyframes swing { from { transform: rotate(-315deg); } to { transform: rotate(-20deg); } }
  @keyframes draw { from { stroke-dashoffset: 100; } to { stroke-dashoffset: 0; } }
  @keyframes sweep {
    0% { transform: rotate(-45deg); }
    73% { transform: rotate(-374deg); }
    87% { transform: rotate(-356deg); }
    100% { transform: rotate(-360deg); }
  }
  @keyframes pop { from { transform: scale(0); } to { transform: scale(1); } }
  @keyframes glow { 0% { opacity: 0.5; transform: scale(0.4); } 100% { opacity: 0; transform: scale(2.2); } }
  @media (prefers-reduced-motion: reduce) {
    .ga * { animation: none !important; }
    .busy .arc { opacity: 1; }
    .busy .arc2 { opacity: 0; }
  }
</style>
