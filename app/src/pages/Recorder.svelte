<script lang="ts">
  // The recorder: gpu-screen-recorder driven by the agent. Record, or keep
  // the last seconds in memory and save them when something happens.
  import { api, errorText, type RecorderAction, type RecorderSettings, type RecorderState } from "../lib/api";
  import { num } from "../lib/format";
  import { t } from "../lib/i18n.svelte";
  import Problem from "../lib/ui/Problem.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";
  import Switch from "../lib/ui/Switch.svelte";

  let st = $state<RecorderState | null>(null);
  let error = $state<string | null>(null);
  let said = $state<string | null>(null);
  let busy = $state(false);
  let now = $state(Date.now());

  const load = () => api.recorderState().then((s) => (st = s)).catch((e) => (error = errorText(e)));
  $effect(() => {
    load();
  });
  // While it records (or installs), the clock and the state once a second;
  // nothing runs while it is idle.
  $effect(() => {
    if (!st || st.mode === "idle") return;
    const id = setInterval(() => {
      now = Date.now();
      if (!document.hidden) load();
    }, 1000);
    return () => clearInterval(id);
  });

  async function act(what: RecorderAction) {
    busy = true;
    try {
      const file = await api.recorderDo(what);
      error = null;
      said = file ? t().recorder.saved + " " + file : null;
      await load();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function change(patch: Partial<RecorderSettings>) {
    if (!st) return;
    const next = { ...st.settings, ...patch };
    try {
      await api.recorderSet(next);
      error = null;
      await load();
    } catch (e) {
      error = errorText(e);
    }
  }

  const elapsed = $derived.by(() => {
    if (!st?.since_unix_ms) return "";
    const s = Math.max(0, Math.floor((now - st.since_unix_ms) / 1000));
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  });
</script>

<header class="head">
  <h1>{t().recorder.title}</h1>
  {#if api.preview}<span class="badge">{t().preview}</span>{/if}
</header>

{#if error}<Problem text={error} />{/if}

{#if st}
  {#if !st.engine}
    <section class="card">
      <h2>{t().recorder.missing}</h2>
      <p class="muted">{t().recorder.missingWhy}</p>
      {#if st.mode === "installing"}
        <p role="status">{t().recorder.installing}</p>
      {:else if st.can_set_up}
        <button class="btn primary" onclick={() => act("setup")} disabled={busy}>{t().recorder.setUp}</button>
        <p class="small">{t().recorder.setUpNote}</p>
      {:else}
        <p class="small">{t().recorder.noFlatpak}</p>
      {/if}
      {#if st.problem}<p class="small" dir="ltr">{st.problem}</p>{/if}
    </section>
  {:else}
    <section class="card now">
      <div class="top">
        {#if st.mode === "recording"}
          <span class="rec" aria-hidden="true"></span><b>{t().recorder.recording}</b> <span dir="ltr">{elapsed}</span>
        {:else if st.mode === "replay"}
          <b>{t().recorder.replayOn.replace("{s}", String(st.settings.replay_seconds))}</b>
        {:else}
          <b>{t().recorder.idle}</b>
        {/if}
      </div>
      <div class="actions">
        {#if st.mode === "idle"}
          <button class="btn primary" onclick={() => act("start")} disabled={busy}>{t().recorder.start}</button>
          <button class="btn" onclick={() => act("replay")} disabled={busy}>{t().recorder.startReplay}</button>
        {:else if st.mode === "recording"}
          <button class="btn primary" onclick={() => act("stop")} disabled={busy}>{t().recorder.stop}</button>
        {:else if st.mode === "replay"}
          <button class="btn primary" onclick={() => act("save")} disabled={busy}>{t().recorder.save}</button>
          <button class="btn" onclick={() => act("stop")} disabled={busy}>{t().recorder.stopReplay}</button>
        {/if}
      </div>
      {#if st.mode === "recording"}<p class="small" dir="ltr">{st.file}</p>{/if}
      {#if said}<p class="saved" role="status" dir="auto">{said}</p>{/if}
      {#if st.problem}<p class="small" dir="ltr">{st.problem}</p>{/if}
      <p class="small">{t().recorder.portalNote}</p>
    </section>

    <section class="card">
      <h2>{t().recorder.quality}</h2>
      <Segmented
        label={t().recorder.quality}
        value={st.settings.quality}
        options={(["small", "balanced", "high"] as const).map((q) => ({ value: q, label: t().recorder.qualities[q] }))}
        onchange={(q) => change({ quality: q })}
      />
      <Segmented
        label={t().recorder.fps}
        value={String(st.settings.fps)}
        options={["30", "60", "120"].map((f) => ({ value: f, label: `${f} fps` }))}
        onchange={(f) => change({ fps: Number(f) })}
      />
      <p class="muted">{t().recorder.size.replace("{mb}", num(st.mb_per_minute, 0))}</p>
      <label class="row">
        <span>{t().recorder.capture}</span>
        <select dir="ltr" value={st.settings.capture} onchange={(e) => change({ capture: (e.target as HTMLSelectElement).value })} disabled={st.mode !== "idle"}>
          <option value="">{t().recorder.captureDefault}</option>
          {#each st.captures as c}<option value={c}>{c === "portal" ? t().recorder.portal : c}</option>{/each}
        </select>
      </label>
      <label class="row">
        <span>{t().recorder.codec}</span>
        <select dir="ltr" value={st.settings.codec} onchange={(e) => change({ codec: (e.target as HTMLSelectElement).value as RecorderSettings["codec"] })} disabled={st.mode !== "idle"}>
          {#each ["auto", "h264", "hevc", "av1"] as c}<option value={c}>{c === "auto" ? t().recorder.auto : c.toUpperCase()}</option>{/each}
        </select>
      </label>
    </section>

    <section class="card">
      <h2>{t().recorder.sound}</h2>
      <div class="row"><span>{t().recorder.gameSound}</span><Switch checked={st.settings.audio_output} label={t().recorder.gameSound} onchange={(v) => change({ audio_output: v })} /></div>
      <div class="row"><span>{t().recorder.mic}</span><Switch checked={st.settings.audio_mic} label={t().recorder.mic} onchange={(v) => change({ audio_mic: v })} /></div>
      <p class="small">{t().recorder.tracks}</p>
    </section>

    <section class="card">
      <h2>{t().recorder.replay}</h2>
      <p class="muted">{t().recorder.replayWhy}</p>
      <Segmented
        label={t().recorder.replay}
        value={String(st.settings.replay_seconds)}
        options={["15", "30", "60", "120"].map((s) => ({ value: s, label: `${s} s` }))}
        onchange={(s) => change({ replay_seconds: Number(s) })}
      />
      <p class="small">{t().recorder.replayMemory.replace("{mb}", num(st.replay_mb, 0))}</p>
    </section>

    <section class="card">
      <h2>{t().recorder.folder}</h2>
      <p dir="ltr"><code>{st.folder}</code></p>
      {#if st.last_saved}<p class="small">{t().recorder.last} <span dir="ltr">{st.last_saved}</span></p>{/if}
      <p class="small">{t().recorder.cli}</p>
    </section>
  {/if}
{/if}

<style>
  .head { display: flex; align-items: center; gap: 14px; margin-bottom: 16px; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 800; }
  h2 { margin: 0; font-size: 1rem; font-weight: 700; }
  .badge { padding: 3px 12px; border-radius: 999px; font-size: 0.8rem; background: var(--card-2); color: var(--text-2); }
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; margin-bottom: 16px; min-width: 0; display: grid; gap: 12px; justify-items: start; }
  p { margin: 0; }
  .muted { color: var(--text-2); font-size: 0.9rem; }
  .small { color: var(--text-3); font-size: 0.8rem; overflow-wrap: anywhere; }
  .saved { color: var(--accent-text); font-size: 0.85rem; overflow-wrap: anywhere; }
  .top { display: flex; align-items: center; gap: 10px; }
  /* A still dot: the UI never animates endlessly (WebKitGTK repaints at 60 fps). */
  .rec { width: 10px; height: 10px; border-radius: 50%; background: #e5484d; }
  .actions { display: flex; gap: 10px; flex-wrap: wrap; }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 12px; width: 100%; font-size: 0.9rem; }
  select { padding: 5px 10px; border-radius: 9px; border: 1px solid var(--card-2); background: var(--card-2); color: var(--text); font: inherit; }
  code { font-size: 0.85rem; }
  .btn { all: unset; padding: 7px 14px; border-radius: 10px; background: var(--card-2); color: var(--text); font-size: 0.88rem; cursor: pointer; }
  .btn.primary { background: var(--accent); color: white; font-weight: 700; }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
</style>
