<script lang="ts">
  // Monitoring → Alerts: per-alert switch and limit, sensitivity cells,
  // cooldown, test, and the recent alerts. Read every 2 s while open (the
  // agent's own readings; it never wakes a sleeping graphics card).
  import HugeIcon from "./icons/HugeIcon.svelte";
  import Problem from "./ui/Problem.svelte";
  import Segmented from "./ui/Segmented.svelte";
  import Switch from "./ui/Switch.svelte";
  import { api, errorText, type AlertEvent, type AlertKind, type AlertSensor, type AlertSettings, type AlertsState } from "./api";
  import { num } from "./format";
  import { locale, t } from "./i18n.svelte";

  let st = $state<AlertsState | null>(null);
  let error = $state<string | null>(null);
  let tested = $state<string | null>(null);
  let busy = $state(false);
  // Edits show at once; the next poll must not undo one still being saved.
  let saving = 0;

  const load = () => {
    if (document.hidden || saving > 0) return;
    api.alertsState().then((s) => { if (saving === 0) st = s; error = null; }).catch((e) => (error = errorText(e)));
  };
  $effect(() => {
    load();
    const id = setInterval(load, 2000);
    return () => clearInterval(id);
  });

  async function save(next: AlertSettings) {
    if (!st) return;
    st.settings = next;
    saving++;
    try {
      await api.alertsSet(next);
      error = null;
    } catch (e) {
      error = errorText(e);
    } finally {
      saving--;
    }
    load();
  }
  const change = (patch: Partial<AlertSettings>) => st && save({ ...st.settings, ...patch });

  type Rule = { kind: AlertKind; on: keyof AlertSettings; limit: keyof AlertSettings; min: number; max: number; unit: string; icon: string; sub?: boolean };
  const RULES: Rule[] = [
    { kind: "cpu_temp", on: "cpu_temp_on", limit: "cpu_temp_c", min: 50, max: 110, unit: "°C", icon: "cpu" },
    { kind: "gpu_temp", on: "gpu_temp_on", limit: "gpu_temp_c", min: 50, max: 110, unit: "°C", icon: "gpu" },
    { kind: "gpu_hotspot", on: "gpu_hotspot_on", limit: "gpu_hotspot_c", min: 60, max: 120, unit: "°C", icon: "temperature", sub: true },
    { kind: "gpu_vram", on: "gpu_vram_on", limit: "gpu_vram_c", min: 60, max: 120, unit: "°C", icon: "temperature", sub: true },
    { kind: "memory", on: "mem_on", limit: "mem_pct", min: 50, max: 99, unit: "%", icon: "ram-memory" },
    { kind: "disk", on: "disk_on", limit: "disk_pct", min: 50, max: 99, unit: "%", icon: "hard-drive" },
  ];

  const sensorsOf = (kind: AlertKind): AlertSensor[] => st?.sensors.filter((x) => x.kind === kind) ?? [];
  // A kind nothing here can read: shown with the reason, its switch off.
  const unsupported = (kind: AlertKind) => {
    const s = sensorsOf(kind);
    return s.length > 0 && s.every((x) => x.status === "unsupported");
  };
  // Hotspot and memory temperatures only where a card was seen reporting them.
  const offered = (r: Rule) => !r.sub || sensorsOf(r.kind).some((x) => x.status === "ok" || x.status === "asleep") || !!st?.settings[r.on];
  const gpuUnreachable = $derived(sensorsOf("gpu_temp").some((x) => x.status === "unknown" && x.subject === ""));
  const subWaiting = $derived(
    RULES.filter((r) => r.sub).some((r) => !offered(r)) &&
      sensorsOf("gpu_hotspot").some((x) => x.status === "unknown"),
  );

  // As the Dashboard writes them: 90° and 95%, in the page's digits.
  const val = (v: number | null, unit: string) => `${num(v)}${unit === "%" ? t().unit.pct : t().unit.c}`;
  const limitText = (n: number, unit: string) => val(n, unit);
  const fmtTime = (ms: number) =>
    new Intl.DateTimeFormat(locale.current === "fa" ? "fa-IR" : "en-GB", { weekday: "short", hour: "2-digit", minute: "2-digit", hour12: false }).format(ms);
  const eventValue = (e: AlertEvent) => {
    if (e.kind === "test") return "";
    const unit = e.kind === "memory" || e.kind === "disk" ? "%" : "°C";
    return `${val(e.value, unit)} · ${t().mon.alerts.limit} ${limitText(e.threshold, unit)}`;
  };

  async function test() {
    busy = true;
    tested = null;
    try {
      const e = await api.alertsTest();
      tested = t().mon.alerts.tested[e.held] ?? t().mon.alerts.tested[""];
      load();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
  async function clear() {
    try {
      await api.alertsClear();
      load();
    } catch (e) {
      error = errorText(e);
    }
  }
  const toggleMount = (path: string) => {
    if (!st) return;
    const m = st.settings.disk_mounts;
    change({ disk_mounts: m.includes(path) ? m.filter((x) => x !== path) : [...m, path] });
  };
  const extraMounts = $derived(st ? st.settings.disk_mounts.filter((m) => !st!.mounts.some((x) => x.path === m)) : []);
</script>

{#snippet cells(above: number, of: number)}
  <span class="cells" aria-label={t().mon.alerts.aboveFor(above, of)} title={t().mon.alerts.aboveFor(above, of)}>
    {#each Array(of) as _, i}<span class:on={i < above}></span>{/each}
  </span>
{/snippet}

{#if error}<Problem text={error} />{/if}

{#if st}
  {@const s = st.settings}
  <section class="card">
    <p class="muted">{t().mon.alerts.explain}</p>
    {#if st.held}
      <p class="held"><HugeIcon name="notification-off-01" size={18} /> {t().mon.alerts.held}</p>
    {/if}
  </section>

  <section class="card">
    {#each RULES.filter(offered) as r (r.kind)}
      {@const no = unsupported(r.kind)}
      {@const on = !no && (s[r.on] as boolean)}
      {@const limit = s[r.limit] as number}
      <div class="rule" class:sub={r.sub} class:off={!on} data-cat={r.kind === "cpu_temp" ? "cpu" : r.kind.startsWith("gpu") ? "gpu" : r.kind === "memory" ? "mem" : undefined}>
        <div class="rule-head">
          <span class="ico"><HugeIcon name={r.icon} size={20} /></span>
          <div class="names">
            <b>{t().mon.alerts.kinds[r.kind]}</b>
            <span class="hint">{t().mon.alerts.hints[r.kind]}</span>
          </div>
          <Switch checked={on} disabled={no} label={t().mon.alerts.kinds[r.kind]} onchange={(v) => change({ [r.on]: v })} />
        </div>
        {#if no}
          <p class="status"><HugeIcon name="unavailable" size={16} /> {t().mon.alerts.status.unsupported}</p>
        {:else}
          <div class="limit">
            <span class="lbl">{t().mon.alerts.at}</span>
            <button class="step" aria-label={t().mon.alerts.lower} disabled={!on || limit <= r.min} onclick={() => change({ [r.limit]: limit - 1 })}>
              <HugeIcon name="minus-sign" size={16} />
            </button>
            <input
              type="range"
              min={r.min}
              max={r.max}
              step="1"
              value={limit}
              disabled={!on}
              aria-label={t().mon.alerts.at}
              oninput={(e) => st && (st.settings = { ...st.settings, [r.limit]: +e.currentTarget.value })}
              onchange={(e) => change({ [r.limit]: +e.currentTarget.value })}
            />
            <button class="step" aria-label={t().mon.alerts.raise} disabled={!on || limit >= r.max} onclick={() => change({ [r.limit]: limit + 1 })}>
              <HugeIcon name="plus-sign" size={16} />
            </button>
            <b class="value">{limitText(limit, r.unit)}</b>
          </div>
          {#if r.kind === "disk"}
            <div class="mounts" role="group" aria-label={t().mon.alerts.mounts}>
              <span class="lbl">{t().mon.alerts.mounts}</span>
              {#each st.mounts as m (m.path)}
                <button class="chip" class:picked={s.disk_mounts.includes(m.path)} aria-pressed={s.disk_mounts.includes(m.path)} disabled={!on} onclick={() => toggleMount(m.path)}>
                  <bdi>{m.path}</bdi> <small>{m.fs} · {num(m.used_pct)}{t().unit.pct}</small>
                </button>
              {/each}
              {#each extraMounts as m (m)}
                <button class="chip picked" aria-pressed="true" disabled={!on} onclick={() => toggleMount(m)}><bdi>{m}</bdi></button>
              {/each}
            </div>
            {#if on && s.disk_mounts.length === 0}<p class="status">{t().mon.alerts.mountsNone}</p>{/if}
          {/if}
          <ul class="readings">
            {#each sensorsOf(r.kind).filter((x) => x.status !== "unsupported" || r.kind === "gpu_temp") as x (x.subject)}
              <li>
                <bdi class="subject">{x.subject}</bdi>
                {#if x.status === "ok"}
                  <b class:hot={x.value != null && x.value >= limit}>{val(x.value, r.unit)}</b>
                  {#if on}{@render cells(Math.min(x.above_s, s.sustain_s), s.sustain_s)}{/if}
                {:else if x.status === "asleep"}
                  <span class="status"><HugeIcon name="moon-02" size={16} /> {t().mon.alerts.status.asleep}</span>
                {:else if x.status === "unknown" && x.subject === ""}
                  <span class="status">{t().mon.alerts.noService}</span>
                {:else}
                  <span class="status">{t().mon.alerts.status[x.status]}</span>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
        {#if r.kind === "gpu_temp" && subWaiting && !gpuUnreachable}
          <p class="note">{t().mon.alerts.subLater}</p>
        {/if}
      </div>
    {/each}
  </section>

  <section class="card settings">
    <div class="field">
      <h2>{t().mon.alerts.sensitivity}</h2>
      <p class="hint">{t().mon.alerts.sensitivityHint}</p>
      <div class="limit">
        <button class="step" aria-label={t().mon.alerts.lower} disabled={s.sustain_s <= 3} onclick={() => change({ sustain_s: s.sustain_s - 1 })}>
          <HugeIcon name="minus-sign" size={16} />
        </button>
        <input
          type="range"
          min="3"
          max="30"
          step="1"
          value={s.sustain_s}
          aria-label={t().mon.alerts.sensitivity}
          oninput={(e) => st && (st.settings = { ...st.settings, sustain_s: +e.currentTarget.value })}
          onchange={(e) => change({ sustain_s: +e.currentTarget.value })}
        />
        <button class="step" aria-label={t().mon.alerts.raise} disabled={s.sustain_s >= 30} onclick={() => change({ sustain_s: s.sustain_s + 1 })}>
          <HugeIcon name="plus-sign" size={16} />
        </button>
        <b class="value">{t().mon.alerts.seconds(s.sustain_s)}</b>
      </div>
      {@render cells(s.sustain_s, s.sustain_s)}
    </div>
    <div class="field">
      <h2>{t().mon.alerts.cooldown}</h2>
      <p class="hint">{t().mon.alerts.cooldownHint}</p>
      <Segmented
        label={t().mon.alerts.cooldown}
        value={String(s.cooldown_min)}
        options={[5, 15, 30, 60, 120].map((m) => ({ value: String(m), label: t().mon.alerts.minutes(m) }))}
        onchange={(v) => change({ cooldown_min: +v })}
      />
    </div>
    <div class="field">
      <button class="btn primary" disabled={busy} onclick={test}><HugeIcon name="notification-03" size={18} /> {t().mon.alerts.test}</button>
      {#if tested}<p class="hint" role="status">{tested}</p>{/if}
    </div>
  </section>

  <section class="card">
    <div class="recent-head">
      <h2>{t().mon.alerts.recent}</h2>
      {#if st.recent.length}
        <button class="btn" onclick={clear}><HugeIcon name="delete-02" size={16} /> {t().mon.alerts.clear}</button>
      {/if}
    </div>
    {#if st.recent.length === 0}
      <p class="muted ok"><HugeIcon name="circle-check" size={18} /> {t().mon.alerts.empty}</p>
    {:else}
      <ul class="recent">
        {#each st.recent as e (e.ts_unix_ms + e.kind + e.subject)}
          <li>
            <span class="when">{fmtTime(e.ts_unix_ms)}</span>
            <b>{t().mon.alerts.kinds[e.kind]}</b>
            {#if e.subject}<bdi class="subject">{e.subject}</bdi>{/if}
            <span class="what">{eventValue(e)}</span>
            {#if !e.notified}<span class="tag">{t().mon.alerts.heldTag}</span>{/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}

<style>
  .card { background: var(--card); border-radius: var(--radius); padding: 18px 20px; margin-bottom: 16px; min-width: 0; }
  h2 { margin: 0 0 4px; font-size: 0.98rem; font-weight: 700; }
  .muted { color: var(--text-2); margin: 0; font-size: 0.9rem; display: flex; align-items: center; gap: 8px; }
  .muted.ok { color: var(--ok); }
  .held { display: flex; gap: 8px; align-items: center; margin: 12px 0 0; padding: 10px 12px; border-radius: 10px; font-size: 0.88rem; background: color-mix(in oklab, var(--accent) 14%, var(--card)); }
  .rule { padding: 14px 0; border-bottom: 1px solid var(--line); display: grid; gap: 10px; }
  .rule:first-child { padding-top: 0; }
  .rule:last-child { border-bottom: 0; padding-bottom: 0; }
  .rule.sub { padding-inline-start: 34px; }
  .rule-head { display: flex; align-items: center; gap: 12px; }
  .ico { color: var(--cat); background: var(--cat-soft); border-radius: 10px; width: 32px; height: 32px; display: grid; place-items: center; flex: none; }
  .rule.off .ico { color: var(--text-3); }
  .names { display: grid; gap: 2px; flex: 1; min-width: 0; }
  .hint { color: var(--text-3); font-size: 0.8rem; margin: 0; }
  .status { display: inline-flex; align-items: center; gap: 6px; color: var(--text-2); font-size: 0.84rem; margin: 0; }
  .note { color: var(--text-3); font-size: 0.8rem; margin: 0; }
  .limit { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .lbl { color: var(--text-2); font-size: 0.84rem; min-width: 6em; }
  .limit input[type="range"] { flex: 1; min-width: 120px; accent-color: var(--cat); }
  .limit input:disabled { opacity: 0.45; }
  .value { min-width: 4.5em; text-align: end; }
  .step { all: unset; display: inline-flex; padding: 5px; border-radius: 8px; background: var(--card-2); color: var(--text); cursor: pointer; }
  .step:disabled { opacity: 0.4; cursor: default; }
  .step:focus-visible, .chip:focus-visible, .btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .readings { list-style: none; margin: 0; padding: 0; display: grid; gap: 6px; }
  .readings li { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; font-size: 0.88rem; }
  .subject { color: var(--text-2); min-width: 8em; }
  .hot { color: var(--warn, #fbbf24); }
  .cells { display: inline-flex; gap: 2px; }
  .cells span { width: 7px; height: 12px; border-radius: 2px; background: var(--card-2); }
  .cells span.on { background: var(--cat); }
  .mounts { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
  .chip { all: unset; padding: 4px 10px; border-radius: 999px; background: var(--card-2); font-size: 0.84rem; cursor: pointer; border: 1px solid transparent; }
  .chip small { color: var(--text-3); }
  .chip.picked { border-color: var(--accent); color: var(--text); }
  .chip:disabled { opacity: 0.45; cursor: default; }
  .settings { display: grid; gap: 18px; }
  .field { display: grid; gap: 8px; justify-items: start; }
  .field .limit { width: 100%; }
  .btn { all: unset; display: inline-flex; align-items: center; gap: 8px; padding: 8px 16px; border-radius: 10px; background: var(--card-2); color: var(--text); font-size: 0.9rem; cursor: pointer; }
  .btn.primary { background: var(--accent); color: white; font-weight: 600; }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .recent-head { display: flex; justify-content: space-between; align-items: center; gap: 12px; margin-bottom: 10px; }
  .recent { list-style: none; margin: 0; padding: 0; display: grid; gap: 8px; }
  .recent li { display: flex; gap: 12px; align-items: baseline; flex-wrap: wrap; font-size: 0.88rem; padding-bottom: 8px; border-bottom: 1px solid var(--line); }
  .recent li:last-child { border-bottom: 0; padding-bottom: 0; }
  .when { color: var(--text-3); min-width: 8em; }
  .what { color: var(--text-2); }
  .tag { margin-inline-start: auto; padding: 2px 10px; border-radius: 999px; font-size: 0.75rem; background: var(--card-2); color: var(--text-2); }
</style>
