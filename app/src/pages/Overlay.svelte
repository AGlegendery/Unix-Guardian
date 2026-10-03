<script lang="ts">
  import { api, errorText, type OverlaySettings, type OverlayState, type Snapshot } from "../lib/api";
  import { num } from "../lib/format";
  import { t } from "../lib/i18n.svelte";
  import ShortcutsCard from "../lib/ShortcutsCard.svelte";
  import Problem from "../lib/ui/Problem.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";
  import Switch from "../lib/ui/Switch.svelte";

  let ov = $state<OverlayState | null>(null);
  let snap = $state<Snapshot | null>(null);
  let error = $state<string | null>(null);
  let saved = $state(false);
  let capturing = $state(false);

  $effect(() => {
    api.overlayState().then((s) => (ov = s)).catch((e) => (error = errorText(e)));
  });
  // Real readings for the preview, every 2 s while the page is open.
  $effect(() => {
    const tick = () => !document.hidden && api.snapshot().then((s) => (snap = s)).catch(() => {});
    tick();
    const id = setInterval(tick, 2000);
    return () => clearInterval(id);
  });

  async function change(patch: Partial<OverlaySettings>) {
    if (!ov) return;
    const next = { ...ov.settings, ...patch };
    const before = ov.settings;
    ov.settings = next; // optimistic; restored if the agent refuses
    try {
      await api.overlaySet(next);
      ov = await api.overlayState();
      error = null;
      saved = true;
      setTimeout(() => (saved = false), 2500);
    } catch (e) {
      ov.settings = before;
      error = errorText(e);
    }
  }

  async function setUp(on: boolean) {
    try {
      await api.overlaySetUp(on);
      ov = await api.overlayState();
      error = null;
    } catch (e) {
      error = errorText(e);
    }
  }

  // MangoHud's key names: Alt_L, Control_L, Shift_L, Super_L, then the key
  // itself as X names it (a letter in lower case, F1…F12).
  function capture(e: KeyboardEvent) {
    if (!capturing) return;
    e.preventDefault();
    if (e.key === "Escape") {
      capturing = false;
      return;
    }
    if (["Alt", "Control", "Shift", "Meta"].includes(e.key)) return;
    const mods = [e.altKey && "Alt_L", e.ctrlKey && "Control_L", e.shiftKey && "Shift_L", e.metaKey && "Super_L"].filter(Boolean);
    const k = /^F\d{1,2}$/.test(e.key) ? e.key : /^[a-zA-Z0-9]$/.test(e.key) ? e.key.toLowerCase() : null;
    if (!k) return;
    capturing = false;
    change({ toggle_key: [...mods, k].join("+") });
  }

  const s = $derived(ov?.settings ?? null);
  const gpu = $derived(snap?.gpus.find((g) => !g.asleep && g.temp_c != null) ?? null);
  const scale = $derived(s ? { small: 0.8, medium: 1, large: 1.25 }[s.size] : 1);
  // MangoHud draws Latin digits whatever the desktop's language.
  const lat = (v: number | null | undefined, d = 0) => (v == null || Number.isNaN(v) ? "—" : v.toFixed(d));
  const keyLabel = (k: string) => k.replaceAll("_L", "").replaceAll("+", " + ").replace(/\b([a-z])$/, (m) => m.toUpperCase());
</script>

<svelte:window onkeydown={capture} />

<header class="head">
  <h1>{t().overlay.title}</h1>
  {#if api.preview}<span class="badge">{t().preview}</span>{/if}
</header>

{#if error}<Problem text={error} />{/if}

<section class="card intro">
  <p class="muted">{t().overlay.explain}</p>
  {#if ov && !ov.installed}
    <p>{t().overlay.notInstalled}</p>
    <code class="cmd" dir="ltr">{ov.install_hint}</code>
  {:else if ov && !ov.set_up}
    <p>{t().overlay.notSetUp}</p>
    <button class="btn primary" onclick={() => setUp(true)}>{t().overlay.setUp}</button>
  {/if}
</section>

{#if s && ov?.set_up}
  <div class="grid">
    <section class="card">
      <h2>{t().overlay.preview}</h2>
      <div class="scene" aria-label={t().overlay.previewNote}>
        {#if s.show !== "off"}
          <div class="hud {s.position} {s.layout}" style:--a={s.opacity_pct / 100} style:--k={scale}>
            {#if s.layout === "compact"}
              <span><b class="accent">{lat(141)}</b> FPS</span>
              <span>7.1 ms</span>
              <span class="cpu">CPU {lat(snap?.cpu.temp_c)}°</span>
              <span class="gpu">GPU {lat(gpu?.temp_c)}°</span>
            {:else}
              <div class="row"><span class="cpu">CPU</span><span>{lat(snap?.cpu.load_pct)}%</span><span>{lat(snap?.cpu.temp_c)}°C</span><span>{lat(snap?.cpu.power_w)} W</span></div>
              <div class="row"><span class="gpu">GPU</span><span>{lat(gpu?.load_pct)}%</span><span>{lat(gpu?.temp_c)}°C</span><span>{lat(gpu?.power_w)} W</span></div>
              <div class="row"><span class="gpu">VRAM</span><span>{gpu ? lat(gpu.vram_used_mb / 1024, 1) : "—"} GB</span><span></span><span></span></div>
              <div class="row"><span class="cpu">RAM</span><span>{snap ? lat((snap.mem.total_kb - snap.mem.available_kb) / 1048576, 1) : "—"} GB</span><span></span><span></span></div>
              <div class="row"><span class="accent">FPS</span><b>141</b><span>7.1 ms</span><span></span></div>
              <svg class="ft" viewBox="0 0 120 20" aria-hidden="true"><polyline points="0,12 10,11 20,12 30,10 40,12 50,4 60,12 70,11 80,12 90,11 100,12 110,11 120,12" /></svg>
            {/if}
          </div>
        {/if}
      </div>
      <p class="small">{t().overlay.previewNote}</p>
    </section>

    <section class="card controls">
      <div class="field">
        <span>{t().overlay.show}</span>
        <Segmented label={t().overlay.show} value={s.show} options={(["gaming", "always", "off"] as const).map((v) => ({ value: v, label: t().overlay.shows[v] }))} onchange={(v) => change({ show: v })} />
        {#if s.show === "off"}<p class="small">{t().overlay.offNote}</p>{/if}
      </div>
      <div class="field">
        <span>{t().overlay.layout}</span>
        <Segmented label={t().overlay.layout} value={s.layout} options={(["card", "compact"] as const).map((v) => ({ value: v, label: t().overlay.layouts[v] }))} onchange={(v) => change({ layout: v })} />
      </div>
      <div class="field">
        <span>{t().overlay.position}</span>
        <div class="corners" role="radiogroup" aria-label={t().overlay.position} dir="ltr">
          {#each ["top-left", "top-center", "top-right", "bottom-left", "bottom-right"] as const as p}
            <button role="radio" aria-checked={s.position === p} class="corner {p}" title={t().overlay.positions[p]} aria-label={t().overlay.positions[p]} onclick={() => change({ position: p })}></button>
          {/each}
        </div>
      </div>
      <div class="field">
        <span>{t().overlay.size}</span>
        <Segmented label={t().overlay.size} value={s.size} options={(["small", "medium", "large"] as const).map((v) => ({ value: v, label: t().overlay.sizes[v] }))} onchange={(v) => change({ size: v })} />
      </div>
      <label class="field">
        <span>{t().overlay.opacity} <b dir="ltr">{s.opacity_pct}%</b></span>
        <input type="range" min="0" max="100" step="5" value={s.opacity_pct} onchange={(e) => change({ opacity_pct: Number((e.target as HTMLInputElement).value) })} />
      </label>
      <div class="field">
        <span>{t().overlay.key}</span>
        <button class="keybox" class:capturing onclick={() => (capturing = true)} dir="ltr">{capturing ? t().overlay.keyPress : keyLabel(s.toggle_key)}</button>
        <p class="small">{t().overlay.keyNote}</p>
      </div>
      <div class="field">
        <div class="switchrow">
          <span>{t().overlay.everyGame}</span>
          <Switch checked={s.every_game} label={t().overlay.everyGame} onchange={(v) => change({ every_game: v })} />
        </div>
        {#if s.every_game && !ov.every_game_active}
          <p class="small">{t().overlay.everyGamePending}</p>
        {:else if !s.every_game}
          <p class="small">{t().overlay.everyGameNote}</p>
          <code class="cmd" dir="ltr">mangohud %command%</code>
        {/if}
      </div>
      {#if saved}<p class="saved" role="status">{t().overlay.saved}</p>{/if}
      <button class="btn" onclick={() => setUp(false)}>{t().overlay.undo}</button>
    </section>
  </div>
{/if}

<ShortcutsCard />

<style>
  .head { display: flex; align-items: center; gap: 14px; margin-bottom: 16px; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 800; }
  h2 { margin: 0 0 12px; font-size: 0.98rem; font-weight: 700; }
  .badge { padding: 3px 12px; border-radius: 999px; font-size: 0.8rem; background: var(--card-2); color: var(--text-2); }
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; margin-bottom: 16px; min-width: 0; }
  .intro { display: grid; gap: 10px; }
  .intro p { margin: 0; }
  .muted { color: var(--text-2); font-size: 0.9rem; }
  .small { color: var(--text-3); font-size: 0.8rem; margin: 6px 0 0; }
  .cmd { display: inline-block; padding: 8px 12px; border-radius: 8px; background: var(--card-2); font-size: 0.9rem; user-select: all; justify-self: start; }
  .btn { all: unset; justify-self: start; padding: 8px 16px; border-radius: 10px; background: var(--card-2); color: var(--text); font-size: 0.9rem; cursor: pointer; }
  .btn.primary { background: var(--accent); color: white; font-weight: 600; }
  .btn:focus-visible, .keybox:focus-visible, .corner:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .grid { display: grid; grid-template-columns: minmax(0, 1.3fr) minmax(0, 1fr); gap: 16px; }
  .grid .card { margin: 0; }
  @media (max-width: 1050px) { .grid { grid-template-columns: 1fr; } }

  /* A dim "game" behind the overlay, so contrast reads as it will in game. */
  .scene {
    position: relative;
    aspect-ratio: 16 / 9;
    border-radius: 12px;
    overflow: hidden;
    background:
      radial-gradient(120% 90% at 70% 110%, #3a5a2c 0%, transparent 55%),
      radial-gradient(90% 70% at 20% 0%, #6a7fa8 0%, transparent 60%),
      linear-gradient(180deg, #2b3550 0%, #4d4a3d 70%, #2d3a22 100%);
  }
  .hud {
    position: absolute;
    font-family: ui-monospace, "JetBrains Mono", monospace;
    font-size: calc(11px * var(--k));
    color: #eceaf6;
    background: rgb(22 20 31 / var(--a));
    border-radius: 8px;
    padding: calc(6px * var(--k)) calc(9px * var(--k));
    line-height: 1.45;
  }
  .hud.top-left { top: 10px; left: 10px; }
  .hud.top-right { top: 10px; right: 10px; }
  .hud.top-center { top: 10px; left: 50%; transform: translateX(-50%); }
  .hud.bottom-left { bottom: 10px; left: 10px; }
  .hud.bottom-right { bottom: 10px; right: 10px; }
  .hud.compact { display: flex; gap: calc(10px * var(--k)); white-space: nowrap; }
  .hud .row { display: grid; grid-template-columns: 4.5em 3.2em 3.6em 3em; }
  .hud .cpu { color: #8f6af2; font-weight: 700; }
  .hud .gpu { color: #c98500; font-weight: 700; }
  .hud .accent { color: #a78bfa; font-weight: 700; }
  .ft { width: 100%; height: calc(18px * var(--k)); display: block; margin-top: 3px; }
  .ft polyline { fill: none; stroke: #8b5cf6; stroke-width: 1.4; }

  .controls { display: grid; gap: 16px; align-content: start; }
  .field { display: grid; gap: 8px; font-size: 0.88rem; color: var(--text-2); }
  .field b { color: var(--text); }
  .field p { margin: 0; }
  input[type="range"] { width: 100%; accent-color: var(--accent); }
  .corners { display: grid; grid-template-columns: repeat(3, 34px); grid-template-rows: repeat(2, 24px); gap: 6px; padding: 8px; border-radius: 10px; background: var(--card-2); justify-self: start; }
  .corner { all: unset; border-radius: 5px; background: color-mix(in oklab, var(--card) 70%, transparent); cursor: pointer; }
  .corner.bottom-left { grid-column: 1; grid-row: 2; }
  .corner.bottom-right { grid-column: 3; grid-row: 2; }
  .corner[aria-checked="true"] { background: var(--accent); }
  .keybox { all: unset; justify-self: start; min-width: 140px; padding: 8px 14px; border-radius: 10px; background: var(--card-2); color: var(--text); font-weight: 600; cursor: pointer; text-align: center; }
  .keybox.capturing { outline: 2px solid var(--accent); }
  .switchrow { display: flex; justify-content: space-between; align-items: center; gap: 12px; color: var(--text); }
  .saved { color: var(--accent-text); font-size: 0.85rem; margin: 0; }
</style>
