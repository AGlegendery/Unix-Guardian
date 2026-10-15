<script lang="ts">
  // A 24-hour clock picker. WebKitGTK draws <input type="time"> as a bare
  // text box with no picker, which the user read as a fake setting; this one
  // has steppers, the arrow keys and the wheel, and digits in the page's
  // language. Always laid out left to right, as clocks are.
  import HugeIcon from "../icons/HugeIcon.svelte";
  import { locale } from "../i18n.svelte";

  let { value = $bindable("07:00"), label, step = 5 }: { value?: string; label: string; step?: number } = $props();

  const parts = $derived(value.split(":").map(Number));
  const h = $derived(Number.isFinite(parts[0]) ? parts[0] : 0);
  const m = $derived(Number.isFinite(parts[1]) ? parts[1] : 0);
  const fmt = $derived(new Intl.NumberFormat(locale.current === "fa" ? "fa-IR" : "en-GB", { minimumIntegerDigits: 2 }));
  const pad = (n: number) => String(n).padStart(2, "0");

  function set(hh: number, mm: number) {
    value = `${pad(((hh % 24) + 24) % 24)}:${pad(((mm % 60) + 60) % 60)}`;
  }
  // Minutes move by `step` and snap to it; crossing the hour carries.
  function addMinutes(d: number) {
    const total = h * 60 + (d > 0 ? Math.floor(m / step) * step + step : Math.ceil(m / step) * step - step);
    const t = ((total % 1440) + 1440) % 1440;
    set(Math.floor(t / 60), t % 60);
  }
  function key(e: KeyboardEvent, unit: "h" | "m") {
    const d = e.key === "ArrowUp" ? 1 : e.key === "ArrowDown" ? -1 : 0;
    if (!d) return;
    e.preventDefault();
    if (unit === "h") set(h + d, m);
    else addMinutes(d);
  }
  function wheel(e: WheelEvent, unit: "h" | "m") {
    e.preventDefault();
    const d = e.deltaY < 0 ? 1 : -1;
    if (unit === "h") set(h + d, m);
    else addMinutes(d);
  }
</script>

<div class="clock" dir="ltr" role="group" aria-label={label}>
  {#each ["h", "m"] as const as unit, i}
    {#if i === 1}<span class="colon" aria-hidden="true">:</span>{/if}
    <div class="col">
      <button type="button" class="step" tabindex="-1" aria-hidden="true" onclick={() => (unit === "h" ? set(h + 1, m) : addMinutes(1))}>
        <HugeIcon name="chevron-up" size={18} />
      </button>
      <span
        class="digits"
        role="spinbutton"
        tabindex="0"
        aria-label={unit === "h" ? "hour" : "minute"}
        aria-valuenow={unit === "h" ? h : m}
        aria-valuemin={0}
        aria-valuemax={unit === "h" ? 23 : 59}
        onkeydown={(e) => key(e, unit)}
        onwheel={(e) => wheel(e, unit)}
      >{fmt.format(unit === "h" ? h : m)}</span>
      <button type="button" class="step" tabindex="-1" aria-hidden="true" onclick={() => (unit === "h" ? set(h - 1, m) : addMinutes(-1))}>
        <HugeIcon name="chevron-down" size={18} />
      </button>
    </div>
  {/each}
</div>

<style>
  .clock {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border-radius: 16px;
    background: var(--card-2);
    border: 1px solid var(--line);
  }
  .col { display: grid; justify-items: center; }
  .digits {
    font-size: 2.1rem;
    font-weight: 700;
    line-height: 1.1;
    min-width: 2.2ch;
    text-align: center;
    font-variant-numeric: tabular-nums;
    border-radius: 10px;
    padding: 0 4px;
    cursor: ns-resize;
  }
  .digits:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  .colon { font-size: 1.9rem; font-weight: 700; color: var(--text-3); padding-bottom: 4px; }
  .step {
    all: unset;
    display: grid;
    place-items: center;
    width: 34px;
    height: 22px;
    border-radius: 8px;
    color: var(--text-3);
    cursor: pointer;
    transition: color 150ms ease-out, background 150ms ease-out;
  }
  .step:hover { color: var(--text); background: color-mix(in oklab, var(--text) 7%, transparent); }
</style>
