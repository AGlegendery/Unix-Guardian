<script lang="ts">
  // Global shortcuts through the desktop's GlobalShortcuts portal. Off until
  // the user turns them on, because binding makes the desktop show a dialog.
  import { api, errorText, type ShortcutsState } from "./api";
  import { t } from "./i18n.svelte";
  import { combo, modifiers } from "./keycombo";
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

  // The recorder: "Change" listens for the next key press with its
  // modifiers; Esc on its own cancels.
  let recording = $state<string | null>(null);
  let held = $state("");
  // Keys the agent refused (taken, or not a shortcut): a choice to make
  // again, not a fault to report.
  let refused = $state<string | null>(null);

  function record(e: KeyboardEvent) {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape" && modifiers(e).length === 0) {
      recording = null;
      return;
    }
    const keys = combo(e);
    if (keys === null) {
      held = modifiers(e).join("+");
      return;
    }
    const id = recording;
    recording = null;
    setKeys(id, keys);
  }

  function release(e: KeyboardEvent) {
    if (recording) held = modifiers(e).join("+");
  }

  function start(id: string) {
    recording = id;
    held = "";
    refused = null;
  }

  async function setKeys(id: string, keys: string) {
    try {
      said = await api.shortcutsSetKeys(id, keys);
      refused = null;
      setTimeout(() => (said = null), 5000);
    } catch (e) {
      refused = errorText(e);
    }
    await load();
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

<svelte:window onkeydowncapture={record} onkeyupcapture={release} />

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
          {#if recording === k.id}
            <kbd dir="ltr" class="rec" role="status">{held ? held + "+…" : t().shortcuts.pressKeys}</kbd>
            <button class="btn" onclick={() => (recording = null)}>{t().shortcuts.cancel}</button>
          {:else}
            <!-- Bound: the desktop's keys; not yet: the keys that will be asked for. -->
            <kbd dir="ltr" class:none={!k.trigger}
              title={k.trigger ? "" : k.chosen ? t().shortcuts.yours : t().shortcuts.suggested}>{k.trigger || k.keys}</kbd>
            <button class="btn" onclick={() => start(k.id)}>{t().shortcuts.change}</button>
          {/if}
          <button class="btn" onclick={() => run(k.id)}>{t().shortcuts.run}</button>
          {#if k.chosen && recording !== k.id}
            <button class="link" onclick={() => setKeys(k.id, "")}>{t().shortcuts.reset}</button>
          {/if}
          {#if st.status === "unavailable" || st.status === "failed"}
            <code dir="ltr">ug shortcut {k.id}</code>
          {/if}
        </li>
      {/each}
    </ul>
    {#if st.status === "unavailable" || st.status === "failed"}<p class="small">{t().shortcuts.command}</p>{/if}
    {#if st.gamemode_by_hand === "on"}<p class="small">{t().shortcuts.byHandOn}</p>{/if}
    {#if st.gamemode_by_hand === "off"}<p class="small">{t().shortcuts.byHandOff}</p>{/if}
    {#if recording}<p class="small">{t().shortcuts.recordNote}</p>{/if}
    {#if refused}<p class="refused" role="alert">{refused}</p>{/if}
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
  li { display: grid; grid-template-columns: minmax(0, 1fr) auto auto auto; align-items: center; gap: 10px; font-size: 0.9rem; }
  li code { grid-column: 1 / -1; font-size: 0.82rem; padding: 4px 8px; border-radius: 6px; background: var(--card-2); justify-self: start; user-select: all; }
  kbd { padding: 4px 10px; border-radius: 8px; background: var(--card-2); font-family: inherit; font-weight: 600; font-size: 0.85rem; }
  kbd.none { color: var(--text-3); font-weight: 400; }
  kbd.rec { outline: 2px solid var(--accent-text); color: var(--accent-text); font-weight: 400; }
  .link { all: unset; grid-column: 2 / -1; justify-self: start; color: var(--text-3); font-size: 0.8rem; cursor: pointer; text-decoration: underline; }
  .link:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .btn { all: unset; padding: 5px 12px; border-radius: 9px; background: var(--card-2); color: var(--text); font-size: 0.82rem; cursor: pointer; }
  .btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .saved { color: var(--accent-text); font-size: 0.85rem; }
  .refused { color: var(--warn); font-size: 0.85rem; }
</style>
