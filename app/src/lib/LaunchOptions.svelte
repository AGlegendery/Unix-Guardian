<script lang="ts">
  // One Steam game's launch options: the card it renders on and the
  // MangoHud overlay. The change is shown before anything is written; the
  // agent refuses to write while Steam runs (it would undo it on exit),
  // backs Steam's file up, and can take Unix Guardian's part back out.
  import { api, errorText, type GpuOffload, type LaunchInfo } from "./api";
  import { t } from "./i18n.svelte";
  import Switch from "./ui/Switch.svelte";

  let { appid, gpus, gpuName }: { appid: number; gpus: GpuOffload[]; gpuName: (pci: string) => string } = $props();

  let info = $state<LaunchInfo | null>(null);
  let gpu = $state("");
  let overlay = $state(false);
  let after = $state<string | null>(null);
  let error = $state<string | null>(null);
  let done = $state<{ text: string; path?: string } | null>(null);
  let busy = $state(false);

  async function load(keepChoice: boolean) {
    try {
      const i = await api.launchOptions(appid);
      info = i;
      if (!keepChoice) {
        gpu = i.gpu;
        overlay = i.overlay && !i.overlay_problem;
      }
      error = null;
    } catch (e) {
      error = errorText(e);
    }
  }

  // Whether Steam is running changes while this is open: a slow re-read.
  $effect(() => {
    load(false);
    const id = setInterval(() => { if (!document.hidden) load(true); }, 5000);
    return () => clearInterval(id);
  });

  // The options this choice gives, recomputed by the agent as it changes.
  $effect(() => {
    const [g, o] = [gpu, overlay];
    if (!info) return;
    api.previewLaunchOptions(appid, g, o).then((a) => (after = a)).catch((e) => { after = null; error = errorText(e); });
  });

  const unchanged = $derived(!!info && gpu === info.gpu && overlay === info.overlay);

  async function act(f: () => Promise<{ text: string; path?: string }>) {
    busy = true;
    done = null;
    try {
      done = await f();
      await load(false);
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
  const apply = () => act(async () => ({ text: t().gmp.launchSaved, path: (await api.setLaunchOptions(appid, gpu, overlay))[1] }));
  const undo = () => act(async () => (await api.undoLaunchOptions(appid), { text: t().gmp.launchUndone }));
</script>

<div class="launch-editor">
  {#if info}
    <div class="choices">
      <label class="field">
        <span>{t().gmp.launchGpu}</span>
        <select bind:value={gpu} disabled={busy}>
          <option value="">{t().gmp.launchGpuDefault}</option>
          {#each gpus as g (g.pci)}
            <option value={g.pci}>{gpuName(g.pci)}</option>
          {/each}
        </select>
      </label>
      <div class="field switch">
        <span>{t().gmp.launchOverlay}</span>
        <Switch checked={overlay} disabled={busy || !!info.overlay_problem} label={t().gmp.launchOverlay} onchange={(v) => (overlay = v)} />
      </div>
    </div>
    {#if info.overlay_problem}<p class="note">{info.overlay_problem}</p>{/if}

    <dl class="diff">
      <dt>{t().gmp.launchNow}</dt>
      <dd dir="ltr">{info.current || t().gmp.launchNone}</dd>
      <dt>{t().gmp.launchAfter}</dt>
      <dd dir="ltr" class:changed={after !== null && after !== info.current}>{after === null ? "…" : after || t().gmp.launchNone}</dd>
    </dl>

    {#if info.steam_running}<p class="note warn">{t().gmp.launchSteamRunning}</p>{/if}
    {#if info.changed_outside}<p class="note">{t().gmp.launchChanged}</p>{/if}
    {#if info.flatpak}<p class="note">{t().gmp.launchFlatpak}</p>{/if}
    {#if error}<p class="note err">{error}</p>{/if}
    {#if done}<p class="note ok" role="status">{done.text}{#if done.path}{" "}<bdi dir="ltr">{done.path}</bdi>{/if}</p>{/if}

    <div class="actions">
      <button class="btn primary" onclick={apply} disabled={busy || info.steam_running || unchanged || after === null}>{t().gmp.launchApply}</button>
      {#if (info.gpu || info.overlay) && !info.changed_outside}
        <button class="btn" onclick={undo} disabled={busy || info.steam_running}>{t().gmp.launchUndo}</button>
      {/if}
    </div>
    <p class="how">{t().gmp.launchHow} <code dir="ltr">ug game launch {appid}</code></p>
  {:else if error}
    <p class="note err">{error}</p>
  {:else}
    <p class="note">…</p>
  {/if}
</div>

<style>
  .launch-editor { margin: 4px 0 6px; padding: 14px; border-radius: var(--radius-sm); background: color-mix(in oklab, var(--card-2) 55%, var(--card)); display: grid; gap: 10px; }
  .choices { display: flex; flex-wrap: wrap; gap: 12px 24px; align-items: center; }
  .field { display: flex; align-items: center; gap: 10px; font-size: 0.85rem; color: var(--text-2); }
  .field > span { white-space: nowrap; }
  select { min-width: 0; max-width: 100%; }
  .how code { font-size: 0.75rem; color: var(--text-2); }
  select { padding: 6px 10px; border-radius: 9px; border: 1px solid var(--line); background: var(--card-2); color: var(--text); font: inherit; font-size: 0.85rem; }
  .diff { display: grid; grid-template-columns: auto 1fr; gap: 6px 12px; margin: 0; font-size: 0.82rem; }
  .diff dt { color: var(--text-3); }
  .diff dd { margin: 0; font-family: ui-monospace, monospace; overflow-wrap: anywhere; color: var(--text-2); text-align: start; }
  .diff dd.changed { color: var(--accent-text); }
  .note { margin: 0; font-size: 0.8rem; line-height: 1.55; color: var(--text-2); overflow-wrap: anywhere; }
  .note.warn { color: var(--warn); }
  .note.err { color: var(--danger); }
  .note.ok { color: var(--ok); }
  .actions { display: flex; gap: 8px; flex-wrap: wrap; }
  .how { margin: 0; font-size: 0.75rem; color: var(--text-3); line-height: 1.5; }
  .btn { all: unset; display: inline-flex; align-items: center; gap: 6px; padding: 7px 12px; border-radius: 10px; background: var(--card-2); color: var(--text); font-size: 0.85rem; cursor: pointer; }
  .btn:hover:not(:disabled) { background: color-mix(in oklab, var(--card-2) 80%, var(--text) 8%); }
  .btn.primary { background: var(--accent); color: white; font-weight: 600; }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
</style>
