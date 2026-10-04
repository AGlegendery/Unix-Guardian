<script lang="ts">
  // The test lab: why it exists, one button, a confirmation of what turns
  // on for a moment, the G gauge and a bar while it runs, then the report
  // to copy or save and email. The engine is ug-testlab (also `ug testlab`
  // and the terminal UI); this page only shows it.
  import { Copy, FlaskConical, Mail, RotateCcw, Save, TriangleAlert } from "@lucide/svelte";
  import { api, errorText, type TestlabProgress, type TestlabReport } from "../lib/api";
  import { t } from "../lib/i18n.svelte";
  import GaugeLoader from "../lib/ui/GaugeLoader.svelte";
  import Problem from "../lib/ui/Problem.svelte";

  type Phase = "idle" | "confirm" | "running" | "done";
  let phase = $state<Phase>("idle");
  let progress = $state<TestlabProgress | null>(null);
  let report = $state<TestlabReport | null>(null);
  let error = $state<string | null>(null);
  let recovered = $state<string[]>([]);
  let copied = $state(false);
  let saved = $state<string | null>(null);

  const pct = $derived(progress ? Math.round((progress.done * 100) / Math.max(1, progress.total)) : 0);
  const stepText = $derived(progress ? (t().testlab.steps[progress.id] ?? progress.label) : "");

  // A test this window was running when it closed is put back at start
  // (main.rs); this catches one left by the terminal UI or `ug testlab`.
  $effect(() => {
    api.testlabRecover().then((r) => (recovered = r)).catch(() => {});
  });

  async function start() {
    phase = "running";
    error = null;
    report = null;
    saved = null;
    progress = null;
    const off = await api.onTestlabProgress((p) => (progress = p));
    try {
      report = await api.testlabRun();
      phase = "done";
    } catch (e) {
      error = errorText(e);
      phase = "idle";
    } finally {
      off();
    }
  }

  async function copy() {
    if (!report) return;
    try {
      await navigator.clipboard.writeText(report.text);
      copied = true;
      setTimeout(() => (copied = false), 2000);
    } catch (e) {
      error = errorText(e);
    }
  }

  async function save() {
    if (!report) return;
    try {
      saved = await api.testlabSave(report.text);
    } catch (e) {
      error = errorText(e);
    }
  }

  const fill = (s: string, v: Record<string, string | number>) => s.replace(/\{(\w)\}/g, (_, k) => String(v[k] ?? ""));
</script>

<header class="head"><h1>{t().testlab.title}</h1></header>

{#if error}<Problem text={error} />{/if}
{#if recovered.length}
  <p class="notice" role="status">{fill(t().testlab.recovered, { l: recovered.join("; ") })}</p>
{/if}

<section class="card intro">
  <FlaskConical size={28} aria-hidden="true" />
  <p>{t().testlab.why}</p>
  {#if phase === "idle" || phase === "done"}
    <div class="buttons">
      <button class="btn primary" onclick={() => (phase = "confirm")}>
        {#if phase === "done"}<RotateCcw size={16} aria-hidden="true" /> {t().testlab.again}{:else}{t().testlab.run}{/if}
      </button>
    </div>
  {/if}
</section>

{#if phase === "confirm"}
  <section class="card" aria-labelledby="lab-confirm">
    <h2 id="lab-confirm">{t().testlab.confirmTitle}</h2>
    <p>{t().testlab.confirmBody}</p>
    <ul>
      {#each t().testlab.temporary as item}<li>{item}</li>{/each}
    </ul>
    <p class="muted">{t().testlab.never}</p>
    <div class="buttons">
      <button class="btn primary" onclick={start}>{t().testlab.start}</button>
      <button class="btn" onclick={() => (phase = report ? "done" : "idle")}>{t().testlab.cancel}</button>
    </div>
  </section>
{/if}

{#if phase === "running"}
  <section class="card run" aria-live="polite">
    <!-- The needle follows the real progress; nothing loops (GaugeLoader). -->
    <GaugeLoader variant="progress" value={pct} size={64} label={t().testlab.running} />
    <div class="run-text">
      <p role="status"><b>{t().testlab.running}</b> <bdi dir="ltr">{pct}%</bdi></p>
      <div class="bar" aria-hidden="true"><span style="width: {pct}%"></span></div>
      <p class="muted">
        {#if progress}{fill(t().testlab.stepOf, { n: Math.min(progress.done + 1, progress.total), t: progress.total })} · {stepText}{/if}
      </p>
    </div>
  </section>
{/if}

{#if phase === "done" && report}
  <section class="card">
    <h2>{t().testlab.doneTitle}</h2>
    <p class="summary">
      <span class="chip ok">{report.passed}</span>
      <span class="chip warn">{report.warned}</span>
      <span class="chip bad">{report.failed}</span>
      <span class="chip">{report.skipped}</span>
      <span>{fill(t().testlab.summary, { p: report.passed, w: report.warned, f: report.failed, s: report.skipped })}</span>
    </p>
    {#if report.leftovers.length}
      <div class="left">
        <TriangleAlert size={18} aria-hidden="true" />
        <div>
          <p>{t().testlab.leftovers}</p>
          <ul dir="ltr">{#each report.leftovers as l}<li>{l}</li>{/each}</ul>
        </div>
      </div>
    {/if}
    <textarea class="out" dir="ltr" readonly spellcheck="false" rows="18" aria-label={t().testlab.title}>{report.text}</textarea>
    <p class="note"><Mail size={16} aria-hidden="true" /> {t().testlab.copyNote}</p>
    <p class="email" dir="ltr">{t().bug.email}</p>
    <div class="buttons">
      <button class="btn primary" onclick={copy}><Copy size={16} aria-hidden="true" /> {copied ? t().testlab.copied : t().testlab.copy}</button>
      <button class="btn" onclick={save}><Save size={16} aria-hidden="true" /> {t().testlab.save}</button>
    </div>
    {#if saved}<p class="muted" dir="auto">{fill(t().testlab.saved, { p: saved })}</p>{/if}
  </section>
{/if}

<style>
  .head { margin-bottom: 16px; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 800; }
  h2 { margin: 0; font-size: 1.1rem; font-weight: 700; }
  .card { background: var(--card); border-radius: var(--radius); padding: 22px 24px; display: grid; gap: 12px; max-width: 760px; color: var(--text); margin-bottom: 16px; }
  .card p { margin: 0; line-height: 1.7; }
  .card ul { margin: 0; padding-inline-start: 22px; line-height: 1.8; }
  .buttons { display: flex; gap: 10px; flex-wrap: wrap; }
  .btn { all: unset; display: inline-flex; align-items: center; gap: 8px; padding: 9px 16px; border-radius: 10px; background: var(--card-2); cursor: pointer; font-size: 0.9rem; }
  .btn.primary { background: var(--accent); color: white; font-weight: 600; }
  .btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .muted { color: var(--text-2); font-size: 0.88rem; }
  .notice { margin: 0 0 16px; max-width: 760px; padding: 10px 14px; border-radius: var(--radius-sm); background: var(--accent-soft); font-size: 0.9rem; }
  .run { grid-template-columns: auto 1fr; align-items: center; gap: 18px; }
  .run-text { display: grid; gap: 8px; min-width: 0; }
  .bar { height: 8px; border-radius: 999px; background: var(--card-2); overflow: hidden; }
  .bar span { display: block; height: 100%; background: var(--accent); border-radius: inherit; transition: width 0.4s ease-out; }
  .summary { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .chip { min-width: 28px; text-align: center; padding: 1px 8px; border-radius: 999px; background: var(--card-2); font-weight: 700; font-size: 0.85rem; }
  .chip.ok { color: var(--ok); }
  .chip.warn { color: var(--warn); }
  .chip.bad { color: var(--danger); }
  .left { display: flex; gap: 10px; color: var(--warn); }
  .left p, .left ul { color: var(--text); }
  .out {
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: var(--radius-sm);
    padding: 12px 14px;
    font-family: ui-monospace, "JetBrains Mono", "DejaVu Sans Mono", monospace;
    font-size: 0.8rem;
    line-height: 1.55;
    white-space: pre;
    overflow: auto;
  }
  .note { display: flex; gap: 8px; align-items: flex-start; }
  .note :global(svg) { flex: none; margin-top: 5px; color: var(--accent-text); }
  .email { font-size: 1.1rem; font-weight: 700; color: var(--accent-text); user-select: all; }
</style>
