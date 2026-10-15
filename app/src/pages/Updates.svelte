<script lang="ts">
  import HugeIcon from "../lib/icons/HugeIcon.svelte";
  import { api, errorText, type UpdateState } from "../lib/api";
  import { locale, t } from "../lib/i18n.svelte";
  import GaugeLoader from "../lib/ui/GaugeLoader.svelte";
  import Problem from "../lib/ui/Problem.svelte";

  let st = $state<UpdateState | null>(null);
  let error = $state<string | null>(null);
  let checking = $state(false);
  let busy = $state(false);
  let message = $state<string | null>(null);

  $effect(() => {
    api.updateState().then((s) => (st = s)).catch((e) => (error = errorText(e)));
  });

  // While an install runs, follow its steps once a second; nothing runs
  // otherwise. The install restarts the agent, so a missed answer or two
  // is waited out.
  $effect(() => {
    if (!st?.step) return;
    let misses = 0;
    const id = setInterval(async () => {
      try {
        const s = await api.updateState();
        misses = 0;
        st = s;
        if (!s.step) {
          if (s.problem) error = s.problem;
          else if (s.result) message = s.result;
        }
      } catch (e) {
        if (++misses > 60) {
          error = errorText(e);
          if (st) st.step = "";
        }
      }
    }, 1000);
    return () => clearInterval(id);
  });

  async function check() {
    checking = true;
    error = null;
    message = null;
    try {
      st = await api.updateCheck();
      if (st.problem) error = st.problem;
    } catch (e) {
      error = errorText(e);
    } finally {
      checking = false;
    }
  }

  async function install() {
    error = null;
    message = null;
    try {
      await api.updateInstall();
      st = await api.updateState();
    } catch (e) {
      error = errorText(e);
    }
  }

  async function rollback() {
    busy = true;
    error = null;
    message = null;
    try {
      message = await api.updateRollback();
      st = await api.updateState();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  function when(ms: number): string {
    return new Intl.DateTimeFormat(locale.current, { dateStyle: "medium", timeStyle: "short" }).format(ms);
  }
  function day(iso: string): string {
    const d = Date.parse(iso);
    return Number.isNaN(d) ? "" : new Intl.DateTimeFormat(locale.current, { dateStyle: "medium" }).format(d);
  }

  // The release notes as GitHub writes them: headings and list items are
  // shown as such, everything else as plain text (never as HTML).
  type Line = { kind: "h" | "li" | "p"; text: string };
  const notes = $derived<Line[]>(
    (st?.notes ?? "")
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => l)
      .map((l) =>
        /^#{1,6}\s/.test(l)
          ? { kind: "h" as const, text: l.replace(/^#+\s*/, "") }
          : /^[-*]\s/.test(l)
            ? { kind: "li" as const, text: l.slice(2) }
            : { kind: "p" as const, text: l },
      )
      .map((l) => ({ ...l, text: l.text.replace(/\*\*|`/g, "") })),
  );
</script>

<header class="head">
  <h1>{t().upd.title}</h1>
  {#if api.preview}<span class="badge">{t().preview}</span>{/if}
</header>

{#if error}<Problem text={error} />{/if}

{#if st}
  <section class="card">
    <dl>
      <dt>{t().upd.current}</dt>
      <dd dir="ltr">{st.current}</dd>
      <dt>{t().upd.checked}</dt>
      <dd>{st.checked_unix_ms ? when(st.checked_unix_ms) : t().upd.never}</dd>
      {#if st.previous}
        <dt>{t().upd.previous}</dt>
        <dd dir="ltr">{st.previous}</dd>
      {/if}
    </dl>
    <div class="buttons">
      <button class="btn" onclick={check} disabled={checking || !!st.step}>
        {#if checking}<GaugeLoader size={16} label={t().upd.checking} />{:else}<HugeIcon name="refresh" size={16} />{/if}
        {checking ? t().upd.checking : t().upd.check}
      </button>
      {#if st.previous && !st.step}
        <button class="btn" onclick={rollback} disabled={busy}>
          {#if busy}<GaugeLoader size={16} label={t().upd.rollback} />{:else}<HugeIcon name="undo" size={16} />{/if} {t().upd.rollback}
        </button>
      {/if}
    </div>
    {#if st.previous && !st.step}<p class="small">{t().upd.rollbackNote}</p>{/if}
    {#if message}<p class="ok" role="status" dir="auto">{message}</p>{/if}
    {#if st.checked_unix_ms && !st.latest && !st.problem}
      <p class="ok">{t().upd.upToDate}</p>
    {/if}
  </section>

  {#if st.latest}
    <section class="card">
      <div class="title">
        <h2>{t().upd.newVersion}: <span dir="ltr">{st.name || st.latest}</span></h2>
        {#if st.published}<span class="small">{day(st.published)}</span>{/if}
      </div>

      {#if notes.length}
        <div class="notes" dir="ltr">
          {#each notes as l}
            {#if l.kind === "h"}<h3>{l.text}</h3>{:else if l.kind === "li"}<p class="li">{l.text}</p>{:else}<p>{l.text}</p>{/if}
          {/each}
        </div>
      {/if}

      {#if st.step}
        <p class="step" role="status">
          <GaugeLoader size={28} label={st.step === "downloading" ? t().upd.downloading : t().upd.installing} maxSeconds={120} />
          {st.step === "downloading" ? t().upd.downloading : t().upd.installing}
        </p>
      {:else if st.can_install}
        <div class="buttons">
          <button class="btn primary" onclick={install}><HugeIcon name="cloud-download" size={16} /> {t().upd.install}</button>
        </div>
        <p class="small">{t().upd.installNote}</p>
      {:else if st.kind === "package" || st.kind === "development"}
        <p>{st.kind === "package" ? t().upd.howPackage : t().upd.howDevelopment}</p>
        <code dir="ltr">{st.how}</code>
      {:else}
        <p>{st.kind === "tarball" ? t().upd.howNoTarball : t().upd.howBuild}</p>
      {/if}

      {#if st.url}
        <p class="small"><HugeIcon name="external-link" size={13} /> {t().upd.page}: <span dir="ltr" class="url">{st.url}</span></p>
      {/if}
    </section>
  {/if}

  <p class="small">{t().upd.daily}</p>
{:else if error}
  <p class="small">{t().upd.agentDown}</p>
{/if}

<style>
  .head { display: flex; align-items: center; gap: 14px; margin-bottom: 16px; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 800; }
  h2 { margin: 0; font-size: 0.98rem; font-weight: 700; }
  h3 { margin: 6px 0 0; font-size: 0.9rem; font-weight: 700; }
  .badge { padding: 3px 12px; border-radius: 999px; font-size: 0.8rem; background: var(--card-2); color: var(--text-2); }
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; margin-bottom: 16px; display: grid; gap: 12px; max-width: 760px; }
  .title { display: flex; justify-content: space-between; align-items: baseline; gap: 12px; flex-wrap: wrap; }
  p { margin: 0; line-height: 1.6; font-size: 0.9rem; }
  .small { color: var(--text-3); font-size: 0.8rem; }
  .ok { color: var(--accent-text); font-size: 0.88rem; }
  .step { font-weight: 600; display: flex; align-items: center; gap: 12px; }
  dl { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 8px 18px; margin: 0; font-size: 0.9rem; }
  dt { color: var(--text-2); }
  dd { margin: 0; justify-self: start; }
  .notes { display: grid; gap: 4px; padding: 12px 14px; border-radius: 10px; background: var(--card-2); text-align: left; }
  .li { padding-inline-start: 14px; position: relative; }
  .li::before { content: "•"; position: absolute; inset-inline-start: 2px; color: var(--text-3); }
  .buttons { display: flex; gap: 10px; flex-wrap: wrap; }
  .btn { all: unset; display: inline-flex; align-items: center; gap: 8px; padding: 9px 16px; border-radius: 10px; background: var(--card-2); cursor: pointer; font-size: 0.9rem; }
  .btn.primary { background: var(--accent); color: white; font-weight: 600; }
  .btn:disabled { opacity: 0.55; cursor: default; }
  .btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  code { font-size: 0.82rem; padding: 6px 10px; border-radius: 6px; background: var(--card-2); user-select: all; justify-self: start; overflow-wrap: anywhere; }
  .url { user-select: all; overflow-wrap: anywhere; }
</style>
