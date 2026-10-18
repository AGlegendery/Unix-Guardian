<script lang="ts">
  import { api, errorText, type AgentSettings, type Overview, type SettingsState } from "../lib/api";
  import { locale, setLocale, t, type Locale } from "../lib/i18n.svelte";
  import ShortcutsCard from "../lib/ShortcutsCard.svelte";
  import Problem from "../lib/ui/Problem.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";
  import Switch from "../lib/ui/Switch.svelte";
  import { motion, setMotion, type Motion } from "../lib/motion.svelte";
  import { setSurface, surface, type Surface } from "../lib/surface.svelte";
  import HugeIcon from "../lib/icons/HugeIcon.svelte";
  import { setSidebarCollapsed, sidebar } from "../lib/sidebar.svelte";

  let { overview }: { overview: Overview | null } = $props();

  let st = $state<SettingsState | null>(null);
  let tray = $state<boolean | null>(null);
  let error = $state<string | null>(null);
  let saved = $state(false);

  $effect(() => {
    api.settingsState().then((s) => (st = s)).catch((e) => (error = errorText(e)));
    api.trayVisible().then((v) => (tray = v)).catch(() => {});
  });

  // Resetting the internet-use counts asks twice (the second press within
  // a few seconds does it).
  let resetArmed = $state(false);
  let resetDone = $state(false);
  let resetTimer: ReturnType<typeof setTimeout> | undefined;
  async function resetTraffic() {
    if (!resetArmed) {
      resetArmed = true;
      clearTimeout(resetTimer);
      resetTimer = setTimeout(() => (resetArmed = false), 4000);
      return;
    }
    resetArmed = false;
    try {
      resetDone = await api.resetTraffic();
      if (!resetDone) error = t().settings.trafficResetFailed;
    } catch (e) {
      error = errorText(e);
    }
  }

  function done() {
    error = null;
    saved = true;
    setTimeout(() => (saved = false), 2500);
  }

  async function change(patch: Partial<AgentSettings>) {
    if (!st) return;
    const before = st.settings;
    st.settings = { ...before, ...patch }; // optimistic; restored if refused
    try {
      await api.settingsSet(st.settings);
      st = await api.settingsState();
      done();
    } catch (e) {
      st.settings = before;
      error = errorText(e);
    }
  }

  async function setTray(on: boolean) {
    try {
      await (on ? api.trayShow() : api.trayHide());
      tray = on;
      done();
    } catch (e) {
      error = errorText(e);
    }
  }

  const intervals = [1, 2, 5, 10];
</script>

<header class="head">
  <h1>{t().settings.title}</h1>
  {#if api.preview}<span class="badge">{t().preview}</span>{/if}
</header>

{#if error}<Problem text={error} />{/if}

<!-- Grouped by what each setting is about (2026-10-17): how the window
     looks, the window and tray, what Unix Guardian tells you, how it finds
     games, then shortcuts, internet-use data and what this installation is. -->
<div class="grid">
  <section class="card">
    <h2>{t().settings.appearance}</h2>
    <div class="field">
      <span>{t().settings.language}</span>
      <Segmented
        label={t().settings.language}
        value={locale.current}
        options={[{ value: "fa" as Locale, label: "فارسی" }, { value: "en" as Locale, label: "English" }]}
        onchange={(v) => setLocale(v)}
      />
      <p class="small">{t().settings.languageNote}</p>
    </div>
    <div class="field">
      <span>{t().settings.motion}</span>
      <Segmented
        label={t().settings.motion}
        value={motion.current}
        options={[{ value: "classic" as Motion, label: t().settings.motions.classic }, { value: "lively" as Motion, label: t().settings.motions.lively }]}
        onchange={(v) => setMotion(v)}
      />
      <p class="small">{t().settings.motionNote}</p>
    </div>
    <div class="field">
      <span>{t().settings.surface}</span>
      <Segmented
        label={t().settings.surface}
        value={surface.current}
        options={[{ value: "dark" as Surface, label: t().settings.surfaces.dark }, { value: "oled" as Surface, label: t().settings.surfaces.oled }]}
        onchange={(v) => setSurface(v)}
      />
      <p class="small">{t().settings.surfaceNote}</p>
    </div>
  </section>

  <section class="card">
    <h2>{t().settings.windowTray}</h2>
    <div class="field">
      <div class="row">
        <span>{t().settings.tray}</span>
        {#if tray != null}<Switch checked={tray} label={t().settings.tray} onchange={setTray} />{/if}
      </div>
      <p class="small">{t().settings.trayNote}</p>
    </div>
    {#if st}
      <div class="field">
        <div class="row">
          <span>{t().settings.closeWindow}</span>
          <Switch checked={st.settings.close_window_on_game} label={t().settings.closeWindow} onchange={(v) => change({ close_window_on_game: v })} />
        </div>
        <p class="small">{t().settings.closeWindowNote}</p>
      </div>
    {/if}
    <div class="field">
      <div class="row">
        <span>{t().settings.compactSidebar}</span>
        <Switch checked={sidebar.collapsed} label={t().settings.compactSidebar} onchange={setSidebarCollapsed} />
      </div>
      <p class="small">{t().settings.compactSidebarNote}</p>
    </div>
  </section>
</div>

{#if st}
  <div class="grid">
    <section class="card">
      <h2>{t().settings.notifyUpdates}</h2>
      <div class="field">
        <div class="row">
          <span class="with-icon">
            <HugeIcon name={st.settings.notifications ? "notification-03" : "notification-off-01"} size={17} hover={false} trigger={st.settings.notifications ? "on" : "off"} />
            {t().settings.notifications}
          </span>
          <Switch checked={st.settings.notifications} label={t().settings.notifications} onchange={(v) => change({ notifications: v })} />
        </div>
        <p class="small">{t().settings.notificationsNote}</p>
      </div>
      <div class="field">
        <div class="row">
          <span>{t().settings.checkUpdates}</span>
          <Switch checked={st.settings.check_updates} label={t().settings.checkUpdates} onchange={(v) => change({ check_updates: v })} />
        </div>
        <p class="small">{t().settings.checkUpdatesNote}</p>
      </div>
      <div class="field">
        <div class="row">
          <span class="with-icon"><HugeIcon name="alert-circle" size={17} /> {t().settings.alertsLink}</span>
          <a class="open-link" href="#monitoring/alerts" onclick={() => history.replaceState(null, "", "#settings")}>{t().settings.alertsOpen}</a>
        </div>
        <p class="small">{t().settings.alertsLinkNote}</p>
      </div>
    </section>

    <section class="card">
      <h2>{t().settings.games}</h2>
      <div class="field">
        <span>{t().settings.scan}</span>
        {#if st.scan_interval_fixed}
          <p><b>{st.settings.scan_interval_s} {t().settings.seconds}</b></p>
          <p class="small">{t().settings.scanFixed}</p>
        {:else}
          <Segmented
            label={t().settings.scan}
            value={String(st.settings.scan_interval_s)}
            options={intervals.map((s) => ({ value: String(s), label: `${s.toLocaleString(locale.current)} ${t().settings.seconds}` }))}
            onchange={(v) => change({ scan_interval_s: Number(v) })}
          />
          <p class="small">{t().settings.scanNote}</p>
        {/if}
      </div>
    </section>
  </div>
{:else if error}
  <section class="card"><p class="small">{t().settings.agentDown}</p></section>
{/if}
{#if saved}<p class="saved" role="status">{t().settings.saved}</p>{/if}

<section class="card">
  <h2>{t().settings.traffic}</h2>
  <div class="row">
    <p class="small">{t().settings.trafficNote}</p>
    <button class="reset" class:armed={resetArmed} onclick={resetTraffic}>{resetArmed ? t().settings.trafficResetConfirm : t().settings.trafficReset}</button>
  </div>
  {#if resetDone}<p class="saved" role="status">{t().settings.trafficResetDone}</p>{/if}
</section>

<ShortcutsCard />

<section class="card about">
  <h2>{t().settings.about}</h2>
  <dl>
    <dt>{t().settings.version}</dt>
    <dd dir="ltr">{st?.agent_version ?? "—"}</dd>
    <dt>{t().settings.service}</dt>
    <dd dir="ltr">{overview?.daemon_version ?? "—"}</dd>
    {#if st}
      <dt>{t().settings.files}</dt>
      <dd dir="ltr"><code>{st.config_dir}</code> <code>{st.data_dir}</code></dd>
    {/if}
    <dt>{t().settings.icons}</dt>
    <dd>{t().settings.iconsCredit}</dd>
  </dl>
  <p class="small">{t().settings.licence} <span dir="ltr">{t().bug.email}</span></p>
</section>

<style>
  .head { display: flex; align-items: center; gap: 14px; margin-bottom: 16px; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 800; }
  h2 { margin: 0 0 12px; font-size: 0.98rem; font-weight: 700; }
  .badge { padding: 3px 12px; border-radius: 999px; font-size: 0.8rem; background: var(--card-2); color: var(--text-2); }
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; margin-bottom: 16px; min-width: 0; display: grid; gap: 14px; align-content: start; }
  .card h2 { margin: 0; }
  .grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 16px; }
  @media (max-width: 900px) { .grid { grid-template-columns: minmax(0, 1fr); } }
  .field { display: grid; gap: 8px; font-size: 0.9rem; }
  .row { display: flex; justify-content: space-between; align-items: center; gap: 12px; font-size: 0.9rem; }
  .with-icon { display: inline-flex; align-items: center; gap: 8px; }
  p { margin: 0; }
  .small { color: var(--text-3); font-size: 0.8rem; }
  .reset { all: unset; flex: none; padding: 7px 14px; border-radius: 9px; background: var(--card-2); font-size: 0.86rem; cursor: pointer; }
  .reset:hover { color: var(--text); }
  .reset.armed { background: var(--danger); color: #fff; font-weight: 600; }
  .reset:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .saved { color: var(--accent-text); font-size: 0.85rem; margin: -6px 0 12px; }
  .card .saved { margin: 0; }
  dl { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 8px 18px; margin: 0; font-size: 0.88rem; }
  dt { color: var(--text-2); }
  dd { margin: 0; justify-self: start; }
  code { font-size: 0.82rem; padding: 2px 6px; border-radius: 6px; background: var(--card-2); user-select: all; overflow-wrap: anywhere; }
  .open-link { padding: 6px 14px; border-radius: 10px; background: var(--accent); color: #fff; font-size: 0.86rem; font-weight: 600; text-decoration: none; flex: none; }
  .open-link:hover { filter: brightness(1.12); }
</style>
