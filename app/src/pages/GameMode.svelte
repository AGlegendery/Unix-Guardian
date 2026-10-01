<script lang="ts">
  import GpuTuner from "../lib/GpuTuner.svelte";
  import Problem from "../lib/ui/Problem.svelte";
  import { Cpu, Gamepad2, MonitorSmartphone, Settings2 } from "@lucide/svelte";
  import { api, errorText, type GpuOffload, type InstalledGame, type Overview, type Profile, type RunningGame } from "../lib/api";
  import { duration } from "../lib/format";
  import { t } from "../lib/i18n.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";
  import Switch from "../lib/ui/Switch.svelte";

  let { overview }: { overview: Overview | null } = $props();

  let running = $state<RunningGame[]>([]);
  let profiles = $state<Profile[]>([]);
  let installed = $state<InstalledGame[] | null>(null);
  let selected = $state("competitive");
  let tab = $state<"cpu" | "system" | "graphics">("cpu");
  let error = $state<string | null>(null);
  let savedAt = $state(0);
  let now = $state(Date.now());

  const load = () => {
    if (document.hidden) return;
    api.gamesRunning().then((r) => (running = r)).catch((e) => (error = errorText(e)));
    api.profiles().then((p) => (profiles = p)).catch((e) => (error = errorText(e)));
    now = Date.now();
  };
  $effect(() => {
    load();
    api.gamesInstalled().then((g) => (installed = g.sort((a, b) => a.name.localeCompare(b.name)))).catch(() => (installed = []));
    const id = setInterval(load, 3000);
    return () => clearInterval(id);
  });

  const game = $derived(running[0] ?? null);
  // Follow the running game's profile the first time one shows up.
  let followed = "";
  $effect(() => {
    if (game && game.key !== followed) {
      followed = game.key;
      selected = game.profile;
    }
  });
  const profile = $derived(profiles.find((p) => p.id === selected) ?? null);
  const cpu = $derived(overview?.inventory?.cpu ?? null);
  const hasPreferred = $derived(!!cpu && (cpu.has_vcache || cpu.p_cores.length > 0));

  const cpuOptions = ["cpu_performance", "cpu_no_idle_states", "cpu_preferred_cores", "cpu_priority", "memory_tuning"] as const;
  type Opt = (typeof cpuOptions)[number] | "pause_services" | "network_tuning" | "power_profile";

  async function setOption(opt: Opt, value: boolean) {
    if (!profile) return;
    const before = profile[opt];
    profile[opt] = value; // optimistic; reverted if the agent refuses
    try {
      await api.setProfileOption(profile.id, opt, value);
      savedAt = Date.now();
      error = null;
    } catch (e) {
      profile[opt] = before;
      error = errorText(e);
    }
  }

  function profileOf(key: string): string {
    return profiles.find((p) => p.games.includes(key))?.id ?? "";
  }

  async function setGame(key: string, id: string) {
    try {
      await api.setGameProfile(key, id);
      profiles = await api.profiles();
      savedAt = Date.now();
      error = null;
    } catch (e) {
      error = errorText(e);
    }
  }

  const label = (id: string) => t().gm.profiles[id] ?? id;

  // Which GPU a game runs on: shown for the running game, and how to choose.
  let offload = $state<GpuOffload[]>([]);
  let copied = $state<string | null>(null);
  $effect(() => {
    api.gpuOffload().then((o) => (offload = o)).catch(() => {});
  });
  const gpuName = (pci: string) => overview?.inventory?.gpus.find((g) => g.pci === pci)?.name ?? pci;
  async function copy(text: string, pci: string) {
    try {
      await navigator.clipboard.writeText(text);
      copied = pci;
      setTimeout(() => (copied = null), 2000);
    } catch (e) {
      error = errorText(e);
    }
  }
</script>

<header class="head">
  <h1>{t().gmp.title}</h1>
  <span class="state-pill" class:on={!!game || overview?.gamemode_active}>{game || overview?.gamemode_active ? t().gm.on : t().gm.off}</span>
  <span class="chip">{t().gm.selection}: <b>{t().gm.automatic}</b></span>
  {#if savedAt && now - savedAt < 4000}
    <span class="saved" role="status">{t().gmp.saved}</span>
  {/if}
</header>

{#if error}
  <Problem text={error} />
{/if}

<!-- What is happening now. -->
<article class="card now" class:on={!!game}>
  <span class="ico"><Gamepad2 size={20} aria-hidden="true" /></span>
  {#if game}
    <div>
      <p class="eyebrowless">{t().gmp.nowPlaying}</p>
      <h2>{game.name}</h2>
      <p class="meta">
        {t().gm.activeProfile}: <b>{label(game.profile)}</b>
        <span class="sep" aria-hidden="true"></span>
        {t().gmp.since} {duration(Math.max(0, (now - game.started_unix_ms) / 1000))}
        <span class="sep" aria-hidden="true"></span>
        {t().gmp.detectedBy} {t().gmp.evidence[game.evidence] ?? game.evidence}
        {#if game.gpu}
          <span class="sep" aria-hidden="true"></span>
          {t().gmp.rendersOn} <b>{gpuName(game.gpu)}</b>
        {/if}
      </p>
      {#if game.blocked_note}<p class="warn">{game.blocked_note}</p>{/if}
    </div>
  {:else}
    <p class="idle">{t().gmp.noGame}</p>
  {/if}
</article>

<div class="grid">
  <!-- The selected profile's settings, in the Windows app's tabbed card. -->
  <article class="card settings">
    <div class="card-head">
      <h2>{t().gmp.profilesTitle}</h2>
      <Segmented
        label={t().gmp.profilesTitle}
        value={selected}
        options={profiles.map((p) => ({ value: p.id, label: p.builtin ? label(p.id) : `${p.id} (${t().gmp.yours})` }))}
        onchange={(v) => (selected = v)}
      />
    </div>
    <p class="hint">{t().gmp.profilesHint}</p>

    <div class="when">
      <span class="muted">{t().gmp.when}</span>
      <Segmented
        label={t().gmp.when}
        value="game"
        options={[
          { value: "game", label: t().gmp.whileGaming },
          { value: "always", label: t().gmp.always, disabled: true, why: t().gmp.alwaysWhy },
        ]}
        onchange={() => {}}
      />
      <p class="row-hint">{t().gmp.alwaysWhy}</p>
    </div>

    <div class="tabs" role="tablist">
      {#each [["cpu", Cpu], ["system", Settings2], ["graphics", MonitorSmartphone]] as const as [id, Icon]}
        <button role="tab" aria-selected={tab === id} class:active={tab === id} onclick={() => (tab = id)}>
          <Icon size={16} aria-hidden="true" />{t().gmp.tabs[id]}
        </button>
      {/each}
    </div>

    {#if profile}
      <div class="rows" role="tabpanel">
        {#if tab === "cpu"}
          {#each cpuOptions as opt}
            {@const blocked = opt === "cpu_preferred_cores" && !hasPreferred}
            {@render row(opt, profile[opt], blocked ? t().gmp.noPreferredCores : null)}
          {/each}
        {:else if tab === "system"}
          {@render row("pause_services", profile.pause_services, null)}
          {@render row("network_tuning", profile.network_tuning, null)}
          {@render row("power_profile", profile.power_profile, null)}
          <div class="row static">
            <div>
              <p class="row-title">{t().gmp.pauseApps}</p>
              {#if profile.pause_apps.length}
                <p class="apps">{#each profile.pause_apps as a}<span class="chip ltr">{a}</span>{/each}</p>
              {:else}
                <p class="row-hint">{t().gmp.pauseAppsNone}</p>
              {/if}
            </div>
          </div>
        {:else}
          <div class="gpu-choice">
            <p class="row-title">{t().gmp.gpuTitle}</p>
            <p class="row-hint">{t().gmp.gpuExplain}</p>
            {#each offload as g (g.pci)}
              <div class="gpu-row">
                <div class="gpu-name">
                  <b>{gpuName(g.pci)}</b>
                  <span class="chip">{g.boot_vga ? t().gmp.gpuDefault : t().gmp.gpuDiscrete}</span>
                </div>
                <code class="launch" dir="ltr">{g.launch_options}</code>
                <button class="copy" onclick={() => copy(g.launch_options, g.pci)}>{copied === g.pci ? t().gmp.copied : t().gmp.copyLaunch}</button>
              </div>
            {/each}
            <p class="row-hint">{t().gmp.gpuCli} <code dir="ltr">ug run --gpu nvidia -- program</code></p>
            {#each offload.filter((g) => !g.boot_vga) as g (g.pci)}
              <GpuTuner pci={g.pci} name={gpuName(g.pci)} />
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </article>

  <!-- Per-game choice. -->
  <article class="card games">
    <div class="card-head"><h2>{t().gmp.gamesTitle}</h2></div>
    <p class="hint">{t().gmp.gamesHint}</p>
    {#if installed && installed.length}
      <ul class="game-list">
        {#each installed as g (g.id)}
          <li>
            <span class="game-name">{g.name}<small class="ltr">{g.launcher}</small></span>
            <select
              aria-label={`${g.name}: ${t().gmp.profilesTitle}`}
              value={profileOf(g.id)}
              onchange={(e) => setGame(g.id, e.currentTarget.value)}
            >
              <option value="">{t().gmp.suggested}</option>
              {#each profiles as p (p.id)}
                <option value={p.id}>{p.builtin ? label(p.id) : p.id}</option>
              {/each}
            </select>
          </li>
        {/each}
      </ul>
    {:else if installed}
      <p class="idle">{t().gmp.noGames}</p>
    {/if}
  </article>
</div>

{#snippet row(opt: Opt, on: boolean, blockedWhy: string | null)}
  <div class="row">
    <div>
      <p class="row-title">{t().gm.settings[opt]}</p>
      <p class="row-hint">{blockedWhy ?? t().gmp.explain[opt]}</p>
    </div>
    <Switch checked={on && !blockedWhy} disabled={!!blockedWhy} label={t().gm.settings[opt]} onchange={(v) => setOption(opt, v)} />
  </div>
{/snippet}

<style>
  .head { display: flex; align-items: center; flex-wrap: wrap; gap: 12px; margin-bottom: 18px; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 800; }
  h2 { margin: 0; font-size: 1rem; font-weight: 700; }
  .state-pill { padding: 3px 14px; border-radius: 999px; font-size: 0.82rem; background: var(--card-2); color: var(--text-2); }
  .state-pill.on { background: var(--accent); color: #fff; }
  .chip { padding: 4px 12px; border-radius: 999px; background: var(--card-2); color: var(--text-2); font-size: 0.82rem; }
  .chip b { color: var(--text); font-weight: 600; }
  .saved { margin-inline-start: auto; color: var(--ok); font-size: 0.85rem; }

  .card { background: var(--card); border-radius: var(--radius); padding: 20px 22px; min-width: 0; }
  .card-head { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 12px; }
  .hint { margin: 10px 0 0; color: var(--text-2); font-size: 0.86rem; max-width: 70ch; }
  .muted { color: var(--text-3); font-size: 0.85rem; }

  .now { display: flex; gap: 16px; align-items: flex-start; }
  .now.on { box-shadow: inset 0 0 0 1px color-mix(in oklab, var(--accent) 55%, transparent); }
  .ico {
    width: 42px;
    height: 42px;
    border-radius: 12px;
    display: grid;
    place-items: center;
    background: var(--accent-soft);
    color: var(--accent-text);
    flex: none;
  }
  .eyebrowless { margin: 0; color: var(--text-3); font-size: 0.82rem; }
  .now h2 { font-size: 1.25rem; margin: 2px 0 4px; }
  .meta { margin: 0; color: var(--text-2); font-size: 0.88rem; display: flex; flex-wrap: wrap; align-items: center; gap: 6px 10px; }
  .meta b { color: var(--text); }
  .sep { width: 4px; height: 4px; border-radius: 50%; background: var(--text-3); }
  .warn { margin: 8px 0 0; color: var(--warn); font-size: 0.86rem; }
  .idle { margin: 0; color: var(--text-2); font-size: 0.9rem; max-width: 70ch; align-self: center; }

  .grid { display: grid; grid-template-columns: minmax(0, 3fr) minmax(0, 2fr); gap: 16px; margin-top: 16px; }
  @media (max-width: 1050px) { .grid { grid-template-columns: 1fr; } }

  .when { display: flex; align-items: center; gap: 12px; margin-top: 16px; flex-wrap: wrap; }
  .tabs { display: flex; gap: 4px; margin-top: 18px; border-bottom: 1px solid var(--line); }
  .tabs button {
    all: unset;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 9px 14px;
    font-size: 0.88rem;
    color: var(--text-2);
    cursor: pointer;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }
  .tabs button:hover { color: var(--text); }
  .tabs button.active { color: var(--text); border-bottom-color: var(--accent); font-weight: 600; }
  .tabs button:focus-visible { outline: 2px solid var(--accent-text); outline-offset: -2px; border-radius: 6px; }

  .rows { display: grid; }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 18px;
    padding: 14px 0;
    border-bottom: 1px solid var(--line);
  }
  .row:last-child { border-bottom: 0; }
  .row-title { margin: 0; font-weight: 600; font-size: 0.93rem; }
  .row-hint { margin: 3px 0 0; color: var(--text-2); font-size: 0.83rem; max-width: 62ch; }
  .apps { display: flex; flex-wrap: wrap; gap: 6px; margin: 8px 0 0; }
  .ltr { direction: ltr; }

  .game-list { list-style: none; margin: 14px 0 0; padding: 0; display: grid; gap: 8px; }
  .game-list li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 12px;
    border-radius: 12px;
    background: var(--card-2);
  }
  .game-name { display: grid; font-size: 0.92rem; min-width: 0; }
  .game-name small { color: var(--text-3); font-size: 0.76rem; text-align: start; }
  select {
    font: inherit;
    font-size: 0.86rem;
    color: var(--text);
    background: var(--card);
    border: 1px solid var(--line);
    border-radius: 9px;
    padding: 5px 10px;
  }
  select:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  .gpu-choice { display: grid; gap: 10px; }
  .gpu-row { display: grid; gap: 6px; padding: 12px; border-radius: var(--radius-sm); background: color-mix(in oklab, var(--card-2) 60%, var(--card)); }
  .gpu-name { display: flex; gap: 10px; align-items: center; flex-wrap: wrap; }
  .launch { font-size: 0.78rem; padding: 6px 8px; border-radius: 6px; background: var(--card-2); overflow-wrap: anywhere; user-select: all; }
  .copy { all: unset; justify-self: start; font-size: 0.82rem; color: var(--accent-text); cursor: pointer; }
  .copy:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; border-radius: 4px; }
</style>
