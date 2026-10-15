<script lang="ts">
  import HugeIcon from "../lib/icons/HugeIcon.svelte";
  import GpuTuner from "../lib/GpuTuner.svelte";
  import LaunchOptions from "../lib/LaunchOptions.svelte";
  import Problem from "../lib/ui/Problem.svelte";
  import { api, errorText, type GpuOffload, type InstalledGame, type Overview, type Profile, type RunningGame } from "../lib/api";
  import { duration } from "../lib/format";
  import { t } from "../lib/i18n.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";
  import ProfilePicker from "../lib/ui/ProfilePicker.svelte";
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
  // Game mode by hand: "on" | "off" | "" and the profile it uses.
  let hand = $state<[string, string]>(["", "balanced"]);
  let busy = $state(false);
  let naming = $state<string | null>(null);
  let newApp = $state("");
  let armedDelete = $state(false);

  const load = () => {
    if (document.hidden) return;
    api.gamesRunning().then((r) => (running = r)).catch((e) => (error = errorText(e)));
    api.profiles().then((p) => (profiles = p)).catch((e) => (error = errorText(e)));
    api.gameModeHand().then((h) => (hand = h)).catch(() => {});
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
  type Opt = (typeof cpuOptions)[number] | "pause_services" | "network_tuning" | "power_profile" | "keep_screen_on" | "do_not_disturb";
  const gmOn = $derived(hand[0] === "on" || (hand[0] !== "off" && (!!game || !!overview?.gamemode_active)));

  /** Runs a change, then reloads the profiles and the by-hand state. */
  async function act(f: () => Promise<unknown>) {
    busy = true;
    try {
      await f();
      [profiles, hand] = await Promise.all([api.profiles(), api.gameModeHand()]);
      savedAt = Date.now();
      error = null;
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
  // The name field takes the keyboard as soon as it opens.
  const focus = (el: HTMLInputElement) => el.focus();
  const turnOn = (id: string) => act(() => api.setGameMode(true, id));
  const turnOff = () => act(() => api.setGameMode(false, ""));
  async function createProfile() {
    const name = naming?.trim() ?? "";
    if (!name) return;
    await act(() => api.createProfile(name, selected));
    if (!error) {
      selected = name;
      naming = null;
    }
  }
  function deleteProfile() {
    if (!profile || profile.builtin) return;
    if (!armedDelete) {
      armedDelete = true;
      setTimeout(() => (armedDelete = false), 4000);
      return;
    }
    armedDelete = false;
    const id = profile.id;
    act(() => api.deleteProfile(id)).then(() => (selected = "balanced"));
  }
  function addApp() {
    if (!profile || !newApp.trim()) return;
    const apps = [...profile.pause_apps, newApp.trim()];
    newApp = "";
    act(() => api.setPauseApps(profile.id, apps));
  }
  const removeApp = (a: string) => profile && act(() => api.setPauseApps(profile.id, profile.pause_apps.filter((x) => x !== a)));

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
  // The Steam game whose launch options are open, by App ID.
  let launchOpen = $state<number | null>(null);
  const steamId = (id: string) => (id.startsWith("steam:") ? Number(id.slice(6)) : null);
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
  <span class="state-pill" class:on={gmOn}>{gmOn ? t().gm.on : t().gm.off}</span>
  <span class="chip">{t().gm.selection}: <b>{hand[0] === "on" ? t().gmp.onByHand(label(hand[1])) : hand[0] === "off" ? t().gm.offByHand : t().gm.automatic}</b></span>
  {#if gmOn}
    <button class="act" disabled={busy} onclick={turnOff}>{t().gm.turnOff}</button>
  {/if}
  {#if savedAt && now - savedAt < 4000}
    <span class="saved" role="status">{t().gmp.saved}</span>
  {/if}
</header>

{#if error}
  <Problem text={error} />
{/if}

<!-- What is happening now. -->
<article class="card now" class:on={!!game}>
  <span class="ico"><HugeIcon name="game-controller-03" size={20} /></span>
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
      <ProfilePicker
        label={t().gmp.profilesTitle}
        value={selected}
        options={profiles.map((p) => ({ value: p.id, label: p.builtin ? label(p.id) : `${p.id} (${t().gmp.yours})`, builtin: p.builtin }))}
        onchange={(v) => (selected = v)}
      />
    </div>
    <p class="hint">{t().gmp.profilesHint}</p>
    {#if profile}
      <div class="profile-actions">
        {#if hand[0] === "on" && hand[1] === profile.id}
          <button class="act" disabled={busy} onclick={turnOff}>{t().gm.turnOff}</button>
        {:else}
          <button class="act primary" disabled={busy} onclick={() => turnOn(profile.id)}>{t().gmp.turnOnWith(label(profile.id))}</button>
        {/if}
        {#if naming === null}
          <button class="act" disabled={busy} onclick={() => (naming = "")}>{t().gmp.newProfile}</button>
        {:else}
          <form class="naming" onsubmit={(e) => { e.preventDefault(); createProfile(); }}>
            <input dir="ltr" bind:value={naming} use:focus placeholder={t().gmp.newName} aria-label={t().gmp.newName} autocomplete="off" spellcheck="false" />
            <button class="act primary" type="submit" disabled={busy || !naming.trim()}>{t().gmp.create}</button>
            <button class="act" type="button" onclick={() => (naming = null)}>{t().gmp.cancel}</button>
            <span class="row-hint">{t().gmp.copyOf(label(profile.id))}</span>
          </form>
        {/if}
        {#if !profile.builtin}
          <button class="act danger" class:armed={armedDelete} disabled={busy} onclick={deleteProfile}>{armedDelete ? t().gmp.deleteConfirm : t().gmp.deleteProfile}</button>
        {/if}
      </div>
    {/if}

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
      {#each [["cpu", "cpu"], ["system", "sliders-horizontal"], ["graphics", "gpu"]] as const as [id, Icon]}
        <button role="tab" aria-selected={tab === id} class:active={tab === id} onclick={() => (tab = id)}>
          <HugeIcon name={Icon} size={16} />{t().gmp.tabs[id]}
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
          {@render row("keep_screen_on", profile.keep_screen_on, null)}
          {@render row("do_not_disturb", profile.do_not_disturb, null)}
          <div class="row static">
            <div class="grow">
              <p class="row-title">{t().gmp.pauseApps}</p>
              {#if profile.pause_apps.length}
                <p class="apps">
                  {#each profile.pause_apps as a (a)}
                    <span class="chip ltr">{a}<button class="x" disabled={busy} aria-label={`${t().ping.remove} ${a}`} onclick={() => removeApp(a)}>✕</button></span>
                  {/each}
                </p>
              {:else}
                <p class="row-hint">{t().gmp.pauseAppsNone}</p>
              {/if}
              <form class="naming" onsubmit={(e) => { e.preventDefault(); addApp(); }}>
                <input dir="ltr" bind:value={newApp} placeholder={t().gmp.pauseAppsAdd} aria-label={t().gmp.pauseAppsAdd} autocomplete="off" spellcheck="false" />
                <button class="act" type="submit" disabled={busy || !newApp.trim()}>{t().gmp.add}</button>
              </form>
              <p class="row-hint">{t().gmp.pauseAppsHint}</p>
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
          {@const appid = steamId(g.id)}
          <li>
            <span class="game-name">{g.name}<small class="ltr">{g.launcher}</small></span>
            {#if appid !== null}
              <button
                class="launch-btn"
                class:open={launchOpen === appid}
                aria-expanded={launchOpen === appid}
                onclick={() => (launchOpen = launchOpen === appid ? null : appid)}
              >{t().gmp.launchButton}</button>
            {/if}
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
          {#if appid !== null && launchOpen === appid}
            <li class="launch-row"><LaunchOptions {appid} gpus={offload} {gpuName} /></li>
          {/if}
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
  @media (max-width: 1050px) { .grid { grid-template-columns: minmax(0, 1fr); } }

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
  .game-name { display: grid; font-size: 0.92rem; min-width: 0; flex: 1; }
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
  .launch-btn {
    all: unset; padding: 4px 10px; border-radius: 8px; font-size: 0.78rem; color: var(--text-2); background: var(--card-2); cursor: pointer;
    transition: background-color 0.15s ease, color 0.15s ease;
  }
  .launch-btn:hover, .launch-btn.open { color: var(--accent-text); background: var(--accent-soft); }
  .launch-btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .game-list li.launch-row { display: block; padding: 0; background: none; }
  .act { all: unset; display: inline-flex; align-items: center; gap: 6px; padding: 7px 13px; border-radius: 10px; background: var(--card-2); color: var(--text); font-size: 0.85rem; cursor: pointer; }
  .act.primary { background: var(--accent); color: #fff; font-weight: 600; }
  .act.danger { color: var(--danger); }
  .act.armed { background: color-mix(in oklab, var(--danger) 16%, transparent); font-weight: 600; }
  .act:disabled { opacity: 0.5; cursor: default; }
  .act:focus-visible, .x:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .profile-actions { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin: 4px 0 12px; }
  .naming { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin-top: 8px; }
  .naming input { min-width: 0; flex: 1 1 12em; padding: 7px 10px; border-radius: 9px; border: 1px solid var(--line); background: var(--card-2); color: var(--text); font: inherit; font-size: 0.85rem; }
  .naming input:focus-visible { outline: 2px solid var(--accent); outline-offset: 0; }
  .grow { flex: 1; min-width: 0; }
  .x { all: unset; margin-inline-start: 6px; cursor: pointer; color: var(--text-3); font-size: 0.75rem; }
  .x:hover { color: var(--danger); }
</style>
