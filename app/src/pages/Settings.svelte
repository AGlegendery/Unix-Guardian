<script lang="ts">
  import { api, errorText, type AgentSettings, type Overview, type SettingsState } from "../lib/api";
  import { locale, setLocale, t, type Locale } from "../lib/i18n.svelte";
  import ShortcutsCard from "../lib/ShortcutsCard.svelte";
  import Problem from "../lib/ui/Problem.svelte";
  import Segmented from "../lib/ui/Segmented.svelte";
  import Switch from "../lib/ui/Switch.svelte";

  let { overview }: { overview: Overview | null } = $props();

  let st = $state<SettingsState | null>(null);
  let tray = $state<boolean | null>(null);
  let error = $state<string | null>(null);
  let saved = $state(false);

  $effect(() => {
    api.settingsState().then((s) => (st = s)).catch((e) => (error = errorText(e)));
    api.trayVisible().then((v) => (tray = v)).catch(() => {});
  });

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

<div class="grid">
  <section class="card">
    <h2>{t().settings.language}</h2>
    <Segmented
      label={t().settings.language}
      value={locale.current}
      options={[{ value: "fa" as Locale, label: "فارسی" }, { value: "en" as Locale, label: "English" }]}
      onchange={(v) => setLocale(v)}
    />
    <p class="small">{t().settings.languageNote}</p>
  </section>

  <section class="card">
    <h2>{t().settings.tray}</h2>
    {#if tray != null}
      <div class="row">
        <span>{t().settings.tray}</span>
        <Switch checked={tray} label={t().settings.tray} onchange={setTray} />
      </div>
    {/if}
    <p class="small">{t().settings.trayNote}</p>
  </section>
</div>

<section class="card">
  <h2>{t().settings.general}</h2>
  {#if st}
    <div class="field">
      <div class="row">
        <span>{t().settings.notifications}</span>
        <Switch checked={st.settings.notifications} label={t().settings.notifications} onchange={(v) => change({ notifications: v })} />
      </div>
      <p class="small">{t().settings.notificationsNote}</p>
    </div>
    <div class="field">
      <div class="row">
        <span>{t().settings.closeWindow}</span>
        <Switch checked={st.settings.close_window_on_game} label={t().settings.closeWindow} onchange={(v) => change({ close_window_on_game: v })} />
      </div>
      <p class="small">{t().settings.closeWindowNote}</p>
    </div>
    <div class="field">
      <div class="row">
        <span>{t().settings.checkUpdates}</span>
        <Switch checked={st.settings.check_updates} label={t().settings.checkUpdates} onchange={(v) => change({ check_updates: v })} />
      </div>
      <p class="small">{t().settings.checkUpdatesNote}</p>
    </div>
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
    {#if saved}<p class="saved" role="status">{t().settings.saved}</p>{/if}
  {:else if error}
    <p class="small">{t().settings.agentDown}</p>
  {/if}
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
  @media (max-width: 900px) { .grid { grid-template-columns: 1fr; } }
  .field { display: grid; gap: 8px; font-size: 0.9rem; }
  .row { display: flex; justify-content: space-between; align-items: center; gap: 12px; font-size: 0.9rem; }
  p { margin: 0; }
  .small { color: var(--text-3); font-size: 0.8rem; }
  .saved { color: var(--accent-text); font-size: 0.85rem; }
  dl { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 8px 18px; margin: 0; font-size: 0.88rem; }
  dt { color: var(--text-2); }
  dd { margin: 0; justify-self: start; }
  code { font-size: 0.82rem; padding: 2px 6px; border-radius: 6px; background: var(--card-2); user-select: all; overflow-wrap: anywhere; }
</style>
