<script lang="ts">
  // Power limit and clock offsets for one GPU. Reading them may wake a
  // sleeping discrete GPU, so nothing is read until the user asks.
  import { api, errorText, type GpuTune, type GpuTuneState } from "./api";
  import { num } from "./format";
  import { t } from "./i18n.svelte";
  import Problem from "./ui/Problem.svelte";
  import GaugeLoader from "./ui/GaugeLoader.svelte";

  let { pci, name }: { pci: string; name: string } = $props();

  let st = $state<GpuTuneState | null>(null);
  let draft = $state<GpuTune>({ power_limit_w: 0, core_offset_mhz: 0, mem_offset_mhz: 0 });
  let error = $state<string | null>(null);
  let busy = $state(false);
  // Seconds left to keep a setting on trial; null when nothing is on trial.
  let left = $state<number | null>(null);
  let timer: ReturnType<typeof setInterval> | null = null;
  let testing = $state(false);
  let verdict = $state<{ stable: boolean; text: string } | null>(null);

  let auto = $state<{ running: boolean; step: number; steps: number; message: string; summary: string; ok: boolean } | null>(null);
  async function autoTune(goal: "faster" | "cooler") {
    try {
      await api.gpuAutoTune(pci, goal);
      auto = await api.gpuAutoTuneStatus();
      const id = setInterval(async () => {
        try {
          auto = await api.gpuAutoTuneStatus();
          if (!auto.running) {
            clearInterval(id);
            await load();
            if (auto.ok) {
              left = 120;
              timer = setInterval(() => {
                left = (left ?? 0) - 1;
                if (left <= 0) {
                  stopTimer();
                  load();
                }
              }, 1000);
            }
          }
        } catch (e) {
          clearInterval(id);
          error = errorText(e);
        }
      }, 1000);
    } catch (e) {
      error = errorText(e);
    }
  }

  async function test() {
    testing = true;
    verdict = null;
    stopTimer();
    try {
      const [stable, text] = await api.gpuTestTune(pci, 120);
      verdict = { stable, text };
      if (stable) {
        left = 30;
        timer = setInterval(() => {
          left = (left ?? 0) - 1;
          if (left <= 0) {
            stopTimer();
            load();
          }
        }, 1000);
      } else {
        await load();
      }
    } catch (e) {
      error = errorText(e);
    } finally {
      testing = false;
    }
  }

  async function load() {
    busy = true;
    try {
      st = await api.gpuTuneState(pci);
      const a = st.applied;
      draft = { power_limit_w: a.power_limit_w || st.power_limit_w, core_offset_mhz: a.core_offset_mhz, mem_offset_mhz: a.mem_offset_mhz };
      error = null;
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  function stopTimer() {
    if (timer) clearInterval(timer);
    timer = null;
    left = null;
  }
  $effect(() => () => stopTimer());

  async function tryIt() {
    if (!st) return;
    busy = true;
    try {
      const pl = Math.abs(draft.power_limit_w - st.power_default_w) < 0.5 ? 0 : draft.power_limit_w;
      await api.gpuTryTune(pci, { ...draft, power_limit_w: pl });
      left = 30;
      timer = setInterval(() => {
        left = (left ?? 0) - 1;
        // The service undoes it by itself; this only refreshes what is shown.
        if (left <= 0) {
          stopTimer();
          load();
        }
      }, 1000);
      error = null;
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function act(f: () => Promise<void>) {
    busy = true;
    try {
      await f();
      stopTimer();
      await load();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  // Ready-made settings, all inside the card's own limits.
  function preset(kind: "cooler" | "efficient" | "stronger") {
    if (!st) return;
    const d = st.power_default_w;
    const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
    const pl = { cooler: d * 0.8, efficient: d * 0.7, stronger: st.power_max_w }[kind];
    draft = {
      power_limit_w: Math.round(clamp(pl, st.power_min_w, st.power_max_w)),
      core_offset_mhz: kind === "efficient" && st.core_offset ? clamp(100, st.core_min_mhz, st.core_max_mhz) : 0,
      mem_offset_mhz: 0,
    };
  }
  const changed = $derived(
    !!st &&
      (Math.abs(draft.power_limit_w - (st.applied.power_limit_w || st.power_limit_w)) >= 0.5 ||
        draft.core_offset_mhz !== st.applied.core_offset_mhz ||
        draft.mem_offset_mhz !== st.applied.mem_offset_mhz),
  );
  const sign = (v: number) => (v > 0 ? `+${v}` : `${v}`);
</script>

<div class="tuner">
  <div class="top">
    <p class="row-title">{t().tune.title}: <b>{name}</b></p>
    {#if !st}
      <button class="btn" disabled={busy} onclick={load}>{t().tune.show}</button>
    {/if}
  </div>
  {#if !st}<p class="row-hint">{t().tune.wakeNote}</p>{/if}
  {#if error}<Problem text={error} />{/if}

  {#if st}
    {#if st.crashed.power_limit_w || st.crashed.core_offset_mhz || st.crashed.mem_offset_mhz}
      <p class="warn">
        {t().tune.crashed}
        ({st.crashed.power_limit_w ? `${num(st.crashed.power_limit_w)} W` : t().tune.default}, {t().tune.core} {sign(st.crashed.core_offset_mhz)}, {t().tune.mem} {sign(st.crashed.mem_offset_mhz)} MHz)
      </p>
    {/if}

    <div class="presets">
      {#each ["cooler", "efficient", "stronger"] as const as p}
        <button class="chip-btn" onclick={() => preset(p)} disabled={busy || left !== null}>{t().tune.presets[p]}</button>
      {/each}
    </div>

    {#if st.power_limit}
      <label class="slider">
        <span>{t().tune.power} <b dir="ltr">{num(draft.power_limit_w)} W</b> <small>({t().tune.default} {num(st.power_default_w)} W)</small></span>
        <input type="range" min={st.power_min_w} max={st.power_max_w} step="1" bind:value={draft.power_limit_w} disabled={left !== null} />
      </label>
    {/if}
    {#if st.core_offset}
      <label class="slider">
        <span>{t().tune.core} <b dir="ltr">{sign(draft.core_offset_mhz)} MHz</b></span>
        <input type="range" min={st.core_min_mhz} max={st.core_max_mhz} step="5" bind:value={draft.core_offset_mhz} disabled={left !== null} />
      </label>
    {/if}
    {#if st.mem_offset}
      <label class="slider">
        <span>{t().tune.mem} <b dir="ltr">{sign(draft.mem_offset_mhz)} MHz</b></span>
        <input type="range" min={st.mem_min_mhz} max={st.mem_max_mhz} step="25" bind:value={draft.mem_offset_mhz} disabled={left !== null} />
      </label>
    {/if}

    {#if auto?.running}
      <div class="trial" role="status">
        <GaugeLoader size={20} label={auto.message} />
        <span>{auto.message} <span dir="ltr">({auto.step}/{auto.steps})</span></span>
        <button class="btn" onclick={() => api.gpuStopAutoTune()}>{t().tune.stop}</button>
      </div>
    {:else if testing}
      <div class="trial" role="status"><GaugeLoader size={20} label={t().tune.testing} /><span>{t().tune.testing}</span></div>
    {:else if left !== null}
      <div class="trial" role="status">
        <span>{t().tune.trying.replace("{s}", num(left))}</span>
        <button class="btn primary" disabled={busy} onclick={() => act(() => api.gpuKeepTune(pci))}>{t().tune.keep}</button>
        <button class="btn" disabled={busy} onclick={test}>{t().tune.test}</button>
        <button class="btn" disabled={busy} onclick={() => act(() => api.gpuCancelTune(pci))}>{t().tune.undo}</button>
      </div>
    {:else}
      <div class="actions">
        <button class="btn primary" disabled={busy || !changed} onclick={tryIt}>{t().tune.try}</button>
        <button class="btn" disabled={busy} onclick={() => act(() => api.gpuResetTune(pci))}>{t().tune.reset}</button>
        <button class="btn" disabled={busy} onclick={() => autoTune("faster")}>{t().tune.autoFaster}</button>
        <button class="btn" disabled={busy} onclick={() => autoTune("cooler")}>{t().tune.autoCooler}</button>
      </div>
    {/if}
    {#if auto && !auto.running && auto.summary}<p class="verdict" class:bad={!auto.ok}>{auto.summary}</p>{/if}
    {#if verdict}<p class="verdict" class:bad={!verdict.stable}>{verdict.text}</p>{/if}
    <p class="row-hint">{t().tune.safety}</p>
    {#each st.notes as n}<p class="row-hint">{n}</p>{/each}
  {/if}
</div>

<style>
  .tuner { display: grid; gap: 10px; padding: 12px; border-radius: var(--radius-sm); background: color-mix(in oklab, var(--card-2) 60%, var(--card)); }
  .top { display: flex; justify-content: space-between; align-items: center; gap: 12px; flex-wrap: wrap; }
  .row-title { margin: 0; }
  .row-hint { margin: 0; color: var(--text-3); font-size: 0.8rem; }
  .verdict { margin: 0; font-size: 0.86rem; color: var(--ok, #34d399); }
  .verdict.bad { color: var(--danger); }
  .warn { margin: 0; color: var(--warn, #fbbf24); font-size: 0.85rem; }
  .presets { display: flex; gap: 8px; flex-wrap: wrap; }
  .chip-btn { all: unset; padding: 5px 12px; border-radius: 999px; background: var(--card-2); font-size: 0.82rem; cursor: pointer; }
  .chip-btn:disabled { opacity: 0.5; cursor: default; }
  .slider { display: grid; gap: 6px; font-size: 0.86rem; color: var(--text-2); }
  .slider b { color: var(--text); }
  .slider small { color: var(--text-3); }
  input[type="range"] { width: 100%; accent-color: var(--accent); }
  .actions, .trial { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
  .trial { padding: 10px 12px; border-radius: 10px; background: color-mix(in oklab, var(--accent) 16%, var(--card)); font-size: 0.88rem; }
  .btn { all: unset; padding: 7px 14px; border-radius: 10px; background: var(--card-2); color: var(--text); font-size: 0.86rem; cursor: pointer; }
  .btn.primary { background: var(--accent); color: white; font-weight: 600; }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .btn:focus-visible, .chip-btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
</style>
