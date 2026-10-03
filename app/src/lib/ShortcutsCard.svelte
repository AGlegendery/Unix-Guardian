<script lang="ts">
  // Global shortcuts through the desktop's GlobalShortcuts portal. Off until
  // the user turns them on, because binding makes the desktop show a dialog.
  import { api, errorText, type ShortcutsState } from "./api";
  import { t } from "./i18n.svelte";
  import Problem from "./ui/Problem.svelte";
  import Switch from "./ui/Switch.svelte";

  let st = $state<ShortcutsState | null>(null);
  let error = $state<string | null>(null);
  let said = $state<string | null>(null);

  const load = () => api.shortcutsState().then((s) => (st = s)).catch((e) => (error = errorText(e)));
  $effect(() => {
    load();
  });

  // While the desktop is connecting or waiting for the user's confirmation,
  // look again once a second; stop when it settles, or after two minutes.
  $effect(() => {
    if (st?.status !== "connecting" && st?.status !== "waiting") return;
    const until = Date.now() + 120_000;
    const id = setInterval(() => (Date.now() > until ? clearInterval(id) : load()), 1000);
    return () => clearInterval(id);
  });

  async function enable(on: boolean) {
    try {
      await api.shortcutsSetEnabled(on);
      error = null;
      await load();
    } catch (e) {
      error = errorText(e);
    }
  }

  async function run(id: string) {
    try {
      said = await api.shortcutsRun(id);
      error = null;
      setTimeout(() => (said = null), 2500);
      await load();
    } catch (e) {
      error = errorText(e);
    }
  }
</script>

<section class="card">
  <div class="top">
    <h2>{t().shortcuts.title}</h2>
    {#if st}<Switch checked={st.enabled} label={t().shortcuts.enable} onchange={enable} />{/if}
  </div>
  <p class="muted">{t().shortcuts.explain}</p>
  {#if error}<Problem text={error} />{/if}
  {#if st}
    <p class="status {st.status}" role="status">{t().shortcuts.status[st.status]}</p>
    {#if st.status === "failed" && st.problem}<p class="small">{st.problem}</p>{/if}
    <ul>
      {#each st.shortcuts as k}
        <li>
          <span>{t().shortcuts.actions[k.id] ?? k.description}</span>
          <kbd dir="ltr" class:none={!k.trigger}>{k.trigger || t().shortcuts.notBound}</kbd>
          <button class="btn" onclick={() => run(k.id)}>{t().shortcuts.run}</button>
          {#if st.status === "unavailable" || st.status === "failed"}
            <code dir="ltr">ug shortcut {k.id}</code>
          {/if}
        </li>
      {/each}
    </ul>
    {#if st.status === "unavailable" || st.status === "failed"}<p class="small">{t().shortcuts.command}</p>{/if}
    {#if st.gamemode_by_hand === "on"}<p class="small">{t().shortcuts.byHandOn}</p>{/if}
    {#if st.gamemode_by_hand === "off"}<p class="small">{t().shortcuts.byHandOff}</p>{/if}
    {#if said}<p class="saved" role="status">{said}</p>{/if}
  {/if}
</section>

<style>
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; margin-bottom: 16px; min-width: 0; display: grid; gap: 10px; }
  .top { display: flex; justify-content: space-between; align-items: center; gap: 12px; }
  h2 { margin: 0; font-size: 0.98rem; font-weight: 700; }
  p { margin: 0; }
  .muted { color: var(--text-2); font-size: 0.9rem; }
  .small { color: var(--text-3); font-size: 0.8rem; }
  .status { font-size: 0.88rem; font-weight: 600; }
  .status.active { color: var(--accent-text); }
  .status.failed, .status.unavailable { color: var(--text-2); }
  ul { list-style: none; margin: 0; padding: 0; display: grid; gap: 8px; }
  li { display: grid; grid-template-columns: minmax(0, 1fr) auto auto; align-items: center; gap: 10px; font-size: 0.9rem; }
  li code { grid-column: 1 / -1; font-size: 0.82rem; padding: 4px 8px; border-radius: 6px; background: var(--card-2); justify-self: start; user-select: all; }
  kbd { padding: 4px 10px; border-radius: 8px; background: var(--card-2); font-family: inherit; font-weight: 600; font-size: 0.85rem; }
  kbd.none { color: var(--text-3); font-weight: 400; }
  .btn { all: unset; padding: 5px 12px; border-radius: 9px; background: var(--card-2); color: var(--text); font-size: 0.82rem; cursor: pointer; }
  .btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .saved { color: var(--accent-text); font-size: 0.85rem; }
</style>
