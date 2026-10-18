<script lang="ts">
  // The driver troubleshooter: check, explain, and fix with a review first.
  // The check runs in this process (files only); fixes go to the service,
  // which asks for the password once (kept a few minutes) and keeps an undo.
  import { api, errorText, type DriverFinding, type DriverOp, type DriverPlan, type DriverReport } from "../lib/api";
  import { locale, t } from "../lib/i18n.svelte";
  import GaugeLoader from "../lib/ui/GaugeLoader.svelte";
  import Problem from "../lib/ui/Problem.svelte";

  let report = $state<DriverReport | null>(null);
  let checking = $state(false);
  let error = $state<string | null>(null);
  let ops = $state<DriverOp[]>([]);
  let serviceDown = $state(false);
  let copied = $state<string | null>(null);
  let showFacts = $state(false);

  // Why a fix the service will not run cannot be done here, from the
  // service itself (the same check "Do it for me" would hit), so the
  // button is shown disabled with its reason instead of left out.
  let notHere = $state<Record<string, string>>({});
  $effect(() => {
    const r = report;
    if (!r || serviceDown) return;
    for (const f of r.findings) {
      if (f.fix_auto || !f.fix_id || f.fix_id === "reboot" || notHere[f.id]) continue;
      api.driverPlan(f.fix_id).then(
        () => {},
        (e) => (notHere = { ...notHere, [f.id]: errorText(e) }),
      );
    }
  });
  // A restart is its own button: two presses, then the system's restart.
  let restartArmed = $state(false);
  async function restart() {
    if (!restartArmed) {
      restartArmed = true;
      setTimeout(() => (restartArmed = false), 5000);
      return;
    }
    restartArmed = false;
    try {
      if ((await api.powerNow("reboot")) === "blocked") throw t().sys.blocked.title;
    } catch (e) {
      error = errorText(e);
    }
  }

  // The review panel for one fix.
  let review = $state<{ finding: DriverFinding; plan: DriverPlan | null; agreed: boolean; busy: boolean } | null>(null);

  const fa = $derived(locale.current === "fa");
  const tx = (f: DriverFinding, k: "title" | "what" | "why" | "how") => (fa ? f[`${k}_fa`] : f[`${k}_en`]);

  async function check() {
    checking = true;
    try {
      report = await api.driverDiagnose();
      error = null;
    } catch (e) {
      error = errorText(e);
    } finally {
      checking = false;
    }
  }

  async function loadOps() {
    try {
      ops = await api.driverOps();
      serviceDown = false;
    } catch {
      serviceDown = true;
    }
  }

  $effect(() => {
    loadOps();
  });

  // While a change runs, follow its log; stop as soon as it settles.
  $effect(() => {
    if (!ops.some((o) => o.state === "running" || o.state === "undoing")) return;
    const id = setInterval(async () => {
      await loadOps();
      if (!ops.some((o) => o.state === "running" || o.state === "undoing")) check();
    }, 800);
    return () => clearInterval(id);
  });

  async function copy(f: DriverFinding) {
    const text = f.commands.join("\n");
    try {
      await navigator.clipboard.writeText(text);
      copied = f.id;
      setTimeout(() => (copied = null), 2000);
    } catch {
      // No clipboard (an old WebKit): the commands are on screen to select.
    }
  }

  async function openReview(f: DriverFinding) {
    review = { finding: f, plan: null, agreed: false, busy: false };
    try {
      review.plan = await api.driverPlan(f.fix_id);
    } catch (e) {
      error = errorText(e);
      review = null;
    }
  }

  // The boot-parameter editor reviews like a fix.
  let param = $state("");
  function openParam(action: "add" | "remove") {
    const p = param.trim();
    const blank = { title_en: "", title_fa: "", what_en: "", what_fa: "", why_en: "", why_fa: "", how_en: "", how_fa: "" };
    openReview({ ...blank, id: "param", severity: "advice", category: "graphics", fix_id: `param-${action}:${p}`, fix_auto: true, commands: [], doc_url: "" });
  }

  async function apply() {
    if (!review?.plan) return;
    review.busy = true;
    try {
      await api.driverApply(review.plan.fix_id);
      review = null;
      error = null;
      await loadOps();
    } catch (e) {
      error = errorText(e);
      if (review) review.busy = false;
    }
  }

  async function keep(op: DriverOp) {
    try {
      await api.driverKeep(op.id);
      error = null;
      await loadOps();
    } catch (e) {
      error = errorText(e);
    }
  }

  async function rollback(op: DriverOp) {
    try {
      await api.driverRollback(op.id);
      error = null;
      await loadOps();
      check();
    } catch (e) {
      error = errorText(e);
    }
  }

  const when = (ms: number) => new Date(ms).toLocaleString(locale.current === "fa" ? "fa-IR" : "en-GB", { dateStyle: "medium", timeStyle: "short" });
</script>

{#snippet reviewPanel()}
{#if review}
        <div class="review" role="region" aria-label={t().drivers.review}>
          <h3>{t().drivers.review}</h3>
          {#if !review.plan}
            <p class="small">…</p>
          {:else if review.plan.refused}
            <p><b>{t().drivers.refused}:</b> {review.plan.refused}</p>
            <button class="btn" onclick={() => (review = null)}>{t().drivers.cancel}</button>
          {:else}
            <p class="small">{t().drivers.reviewIntro}</p>
            <ol>
              {#each review.plan.steps as s}
                <li>
                  <div><b>{t().drivers.kinds[s.kind] ?? s.kind}</b> <code dir="ltr">{s.target}</code></div>
                  {#if s.kind === "write-file" || s.kind === "swap"}
                    <pre dir="ltr">{s.detail}</pre>
                  {:else}
                    <p class="small" dir="ltr">{s.detail}</p>
                  {/if}
                  <p class="small" dir="ltr">{t().drivers.undo}: {s.undo}</p>
                </li>
              {/each}
            </ol>
            {#if review.plan.needs_reboot}
              <label class="agree">
                <input type="checkbox" bind:checked={review.agreed} />
                {t().drivers.reboot}
              </label>
            {/if}
            {#if review.plan.snapshot}<p><b>{t().drivers.safety}:</b> <span dir="ltr">{review.plan.snapshot}</span></p>{/if}
            <p class="small">{t().drivers.password}</p>
            <div class="actions">
              <button class="btn primary" onclick={apply} disabled={review.busy || (review.plan.needs_reboot && !review.agreed)}>{t().drivers.apply}</button>
              <button class="btn" onclick={() => (review = null)}>{t().drivers.cancel}</button>
            </div>
          {/if}
        </div>
{/if}
{/snippet}

<header class="head">
  <h1>{t().drivers.title}</h1>
  {#if api.preview}<span class="badge">{t().preview}</span>{/if}
</header>

{#if error}<Problem text={error} />{/if}

{#each ops.filter((o) => o.verify === "check") as op (op.id)}
  <section class="card checkit" role="alert">
    <h2>{t().drivers.checkTitle}</h2>
    <p><b dir="ltr">{op.title}</b></p>
    <p class="muted">{t().drivers.checkText}</p>
    <div class="actions">
      <button class="btn primary" onclick={() => keep(op)}>{t().drivers.keep}</button>
      <button class="btn" onclick={() => rollback(op)}>{t().drivers.rollback}</button>
    </div>
  </section>
{/each}

<section class="card intro">
  <p class="muted">{t().drivers.intro}</p>
  <button class="btn primary big" onclick={check} disabled={checking}>
    {#if checking}<span class="on-accent"><GaugeLoader size={18} label={t().drivers.checking} /></span>{/if}
    {checking ? t().drivers.checking : report ? t().drivers.again : t().drivers.check}
  </button>
  {#if report}
    <p class="small" dir="auto">{report.distro}</p>
  {/if}
</section>

{#if report}
  {#if report.findings.length === 0}
    <section class="card ok"><p>{t().drivers.healthy}</p></section>
  {/if}

  {#each report.findings as f (f.id)}
    <section class="card finding {f.severity}">
      <div class="top">
        <span class="sev {f.severity}">{t().drivers.severity[f.severity]}</span>
        <span class="cat">{t().drivers.categories[f.category] ?? f.category}</span>
        <h2>{tx(f, "title")}</h2>
      </div>
      <dl>
        <dt>{t().drivers.what}</dt>
        <dd>{tx(f, "what")}</dd>
        <dt>{t().drivers.why}</dt>
        <dd>{tx(f, "why")}</dd>
        <dt>{t().drivers.how}</dt>
        <dd>{tx(f, "how")}</dd>
      </dl>
      {#if !f.fix_auto && f.commands.length}
        <p class="small">{t().drivers.commandsOnly}</p>
      {/if}
      {#if f.commands.length}
        <pre dir="ltr">{f.commands.join("\n")}</pre>
      {/if}
      <div class="actions">
        {#if f.fix_id === "reboot"}
          <button class="btn primary" class:armed={restartArmed} onclick={restart} disabled={serviceDown}>{restartArmed ? t().drivers.restartConfirm : t().drivers.restartNow}</button>
        {:else if f.fix_auto}
          <button class="btn primary" onclick={() => openReview(f)} disabled={serviceDown} title={serviceDown ? t().drivers.serviceDown : undefined}>{t().drivers.doIt}</button>
        {:else if f.fix_id && notHere[f.id]}
          <button class="btn primary" disabled title={notHere[f.id]}>{t().drivers.doIt}</button>
        {/if}
        {#if f.commands.length}
          <button class="btn" onclick={() => copy(f)}>{copied === f.id ? t().drivers.copied : t().drivers.copy}</button>
        {/if}
        {#if f.doc_url}
          <a class="link" href={f.doc_url} target="_blank" rel="noreferrer">{t().drivers.more}</a>
        {/if}
      </div>
      {#if f.fix_auto && serviceDown}<p class="small">{t().drivers.serviceDown}</p>{/if}
      {#if !f.fix_auto && f.fix_id !== "reboot" && notHere[f.id]}
        <p class="why-not"><span class="chip-warn">{t().drivers.notHere}</span> {notHere[f.id]}</p>
      {/if}

      {#if review?.finding.id === f.id}{@render reviewPanel()}{/if}
    </section>
  {/each}

  <section class="card">
    <h2>{t().drivers.params}</h2>
    <p class="small">{t().drivers.paramsNote}</p>
    <pre dir="ltr">{report.facts.find((x) => x.key === "kernel.cmdline")?.value ?? ""}</pre>
    <div class="actions">
      <input class="param" dir="ltr" placeholder="mem_sleep_default=deep" bind:value={param} />
      <button class="btn" disabled={!param.trim() || serviceDown} onclick={() => openParam("add")}>{t().drivers.paramAdd}</button>
      <button class="btn" disabled={!param.trim() || serviceDown} onclick={() => openParam("remove")}>{t().drivers.paramRemove}</button>
    </div>
    {#if review?.finding.id === "param"}{@render reviewPanel()}{/if}
  </section>

  <section class="card">
    <button class="fold" onclick={() => (showFacts = !showFacts)} aria-expanded={showFacts}>{t().drivers.found} {showFacts ? "▾" : "▸"}</button>
    {#if showFacts}
      <dl class="facts" dir="ltr">
        {#each report.facts.filter((x) => x.value !== "") as x}
          <dt>{x.key}</dt>
          <dd>{x.value}</dd>
        {/each}
      </dl>
    {/if}
  </section>
{/if}

<section class="card">
  <h2>{t().drivers.history}</h2>
  {#if serviceDown}
    <p class="small">{t().drivers.serviceDown}</p>
  {:else if ops.length === 0}
    <p class="small">{t().drivers.noHistory}</p>
  {/if}
  {#each ops as op (op.id)}
    <div class="op">
      <div class="top">
        <span class="state {op.state}">
          {#if op.state === "running" || op.state === "undoing"}<GaugeLoader size={14} label={t().drivers.states[op.state] ?? op.state} maxSeconds={120} />{/if}
          {t().drivers.states[op.state] ?? op.state}
        </span>
        <b dir="ltr">{op.title}</b>
        <span class="small">{when(op.started_unix_ms)}</span>
      </div>
      <pre dir="ltr">{op.log.join("\n")}</pre>
      {#if op.verify === "restart"}<p class="small">{t().drivers.rebootNow}</p>{/if}
      {#if op.state === "done" || op.state === "interrupted"}
        <button class="btn" onclick={() => rollback(op)}>{t().drivers.rollback}</button>
      {/if}
    </div>
  {/each}
</section>

<style>
  .why-not { margin: 10px 0 0; font-size: 0.82rem; color: var(--text-2); line-height: 1.7; }
  .chip-warn { display: inline-block; padding: 1px 10px; border-radius: 999px; font-weight: 600; color: var(--warn); background: color-mix(in oklab, var(--warn) 14%, var(--card)); margin-inline-end: 6px; }
  .btn.armed { box-shadow: 0 0 0 3px color-mix(in oklab, var(--danger) 45%, transparent); }
  .on-accent { display: inline-flex; --ug-accent: #fff; --ug-fg: rgb(255 255 255 / 0.55); }
  .head { display: flex; align-items: center; gap: 14px; margin-bottom: 16px; }
  h1 { margin: 0; font-size: 1.55rem; font-weight: 800; }
  h2 { margin: 0; font-size: 1rem; font-weight: 700; }
  h3 { margin: 0; font-size: 0.95rem; }
  .badge { padding: 3px 12px; border-radius: 999px; font-size: 0.8rem; background: var(--card-2); color: var(--text-2); }
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; margin-bottom: 16px; min-width: 0; display: grid; gap: 12px; }
  .intro { justify-items: start; }
  p { margin: 0; }
  .muted { color: var(--text-2); font-size: 0.9rem; }
  .small { color: var(--text-3); font-size: 0.8rem; }
  .btn { all: unset; padding: 8px 16px; border-radius: 10px; background: var(--card-2); color: var(--text); font-size: 0.9rem; cursor: pointer; }
  .btn.primary { background: var(--accent); color: white; font-weight: 600; }
  .btn.big { padding: 12px 22px; font-size: 1rem; display: inline-flex; align-items: center; gap: 10px; }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .btn:focus-visible, .fold:focus-visible, .link:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .link { color: var(--accent-text); font-size: 0.88rem; align-self: center; }
  .top { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .sev, .state { padding: 2px 10px; border-radius: 999px; font-size: 0.78rem; font-weight: 700; display: inline-flex; align-items: center; gap: 6px; background: var(--card-2); color: var(--text-2); }
  .param { flex: 1; min-width: 12rem; padding: 6px 10px; border-radius: 9px; border: 1px solid var(--card-2); background: var(--bg, var(--card-2)); color: var(--text); font: inherit; }
  .checkit { border: 1px solid #f5a524; }
  .cat { padding: 2px 10px; border-radius: 999px; font-size: 0.78rem; color: var(--text-2); border: 1px solid var(--card-2); }
  .sev.critical, .state.failed, .state.interrupted { background: color-mix(in oklab, #e5484d 22%, transparent); color: #ff8f93; }
  .sev.important, .state.running, .state.undoing { background: color-mix(in oklab, #f5a524 22%, transparent); color: #ffc46b; }
  .sev.advice { background: color-mix(in oklab, var(--accent) 22%, transparent); color: var(--accent-text); }
  .state.done, .state.undone { background: color-mix(in oklab, #30a46c 22%, transparent); color: #6fdba0; }
  .finding.critical { box-shadow: inset 3px 0 0 #e5484d; }
  :global([dir="rtl"]) .finding.critical { box-shadow: inset -3px 0 0 #e5484d; }
  dl { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 6px 16px; margin: 0; font-size: 0.9rem; }
  dt { color: var(--text-2); }
  dd { margin: 0; line-height: 1.6; }
  .facts { font-size: 0.8rem; }
  .facts dd { overflow-wrap: anywhere; }
  pre { margin: 0; padding: 10px 12px; border-radius: 8px; background: var(--card-2); font-size: 0.8rem; white-space: pre-wrap; overflow-wrap: anywhere; user-select: text; }
  code { font-size: 0.82rem; overflow-wrap: anywhere; }
  .actions { display: flex; gap: 10px; flex-wrap: wrap; }
  .review { display: grid; gap: 10px; padding: 14px; border-radius: 12px; border: 1px solid color-mix(in oklab, var(--accent) 45%, transparent); }
  ol { margin: 0; padding-inline-start: 20px; display: grid; gap: 10px; }
  .agree { display: flex; gap: 8px; align-items: center; font-size: 0.9rem; }
  .agree input { accent-color: var(--accent); }
  .fold { all: unset; cursor: pointer; font-weight: 700; font-size: 0.95rem; }
  .op { display: grid; gap: 8px; padding-top: 10px; border-top: 1px solid var(--card-2); }
  .ok p { color: #6fdba0; }
</style>
