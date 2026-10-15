<script lang="ts">
  // The game-profile picker (a radio group). It never grows past the width the
  // shipped profiles take: with more profiles, or long names, the items scroll
  // sideways inside it, a fade and an arrow button sit on each side that has
  // hidden items (the arrow brings the next one into view), and a thin
  // scrollbar shows under it. Works the same in left-to-right and Persian.
  import HugeIcon from "../icons/HugeIcon.svelte";
  import { t } from "../i18n.svelte";

  type Option = { value: string; label: string; builtin?: boolean };
  let {
    options,
    value,
    label,
    onchange,
    disabled = false,
    quiet = false,
  }: { options: Option[]; value: string; label: string; onchange: (v: string) => void; disabled?: boolean; quiet?: boolean } = $props();

  let strip = $state<HTMLDivElement | null>(null);
  let cap = $state(0); // px; 0 = no cap beyond the space the parent gives
  let hiddenLeft = $state(false);
  let hiddenRight = $state(false);
  let thumb = $state<[number, number]>([0, 100]); // left and width, % of the track

  const items = () => (strip ? (Array.from(strip.children) as HTMLElement[]) : []);

  /** The width of the shipped profiles alone: the picker's size before any of the user's own. */
  function measure() {
    if (!strip) return;
    const css = getComputedStyle(strip);
    const gap = parseFloat(css.columnGap) || 0;
    const pad = (parseFloat(css.paddingLeft) || 0) + (parseFloat(css.paddingRight) || 0);
    const shipped = items().filter((_, i) => options[i]?.builtin);
    cap = shipped.length && shipped.length < options.length ? shipped.reduce((w, el) => w + el.offsetWidth, 0) + gap * (shipped.length - 1) + pad : 0;
  }

  /** Which sides have items out of view (from the items' boxes, so the
   *  sign of scrollLeft in right-to-left pages does not matter). */
  function edges() {
    if (!strip) return;
    const box = strip.getBoundingClientRect();
    const all = items();
    hiddenLeft = all.some((el) => el.getBoundingClientRect().left < box.left - 1);
    hiddenRight = all.some((el) => el.getBoundingClientRect().right > box.right + 1);
    // Where the view is, from the physical left (scrollLeft runs negative
    // in right-to-left pages).
    const full = strip.scrollWidth;
    const span = full - strip.clientWidth;
    const fromLeft = getComputedStyle(strip).direction === "rtl" ? span + strip.scrollLeft : strip.scrollLeft;
    const w = full > 0 ? (strip.clientWidth / full) * 100 : 100;
    thumb = [Math.min(100 - w, Math.max(0, (fromLeft / full) * 100)), w];
  }

  // Dragging the thin bar scrolls by the same share of the whole list.
  let drag: number | null = null;
  function grab(e: PointerEvent) {
    drag = e.clientX;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function pull(e: PointerEvent) {
    if (drag === null || !strip) return;
    strip.scrollBy({ left: ((e.clientX - drag) * strip.scrollWidth) / strip.clientWidth });
    drag = e.clientX;
  }

  /** Scrolls just enough to show `el` whole, clear of the arrow on that side. */
  function reveal(el: HTMLElement) {
    if (!strip) return;
    const box = strip.getBoundingClientRect();
    const r = el.getBoundingClientRect();
    const lefts = items().map((x) => x.getBoundingClientRect().left);
    const room = 30; // the arrow button and its fade, when there is more past it
    const more = (side: "l" | "r") => lefts.some((l) => (side === "l" ? l < r.left - 1 : l > r.left + 1));
    let dx = 0;
    if (r.left < box.left + room && more("l")) dx = r.left - box.left - room;
    else if (r.left < box.left) dx = r.left - box.left;
    else if (r.right > box.right - room && more("r")) dx = r.right - box.right + room;
    else if (r.right > box.right) dx = r.right - box.right;
    if (Math.abs(dx) < 1) return;
    strip.scrollBy({ left: dx }); // a jump: smooth scrolling stalls when the window is not drawing
    edges();
  }

  /** The arrow on one side: the nearest item still hidden (even in part) on
   *  that side, by position on screen so it works in both directions. */
  function step(dir: "left" | "right") {
    if (!strip) return;
    const box = strip.getBoundingClientRect();
    const edge = 29; // just inside the arrow's room, so a revealed item counts as shown
    const boxes = items().map((el) => ({ el, r: el.getBoundingClientRect() }));
    const next = dir === "right"
      ? boxes.filter((b) => b.r.right > box.right - edge).sort((a, b) => a.r.left - b.r.left)[0]
      : boxes.filter((b) => b.r.left < box.left + edge).sort((a, b) => b.r.right - a.r.right)[0];
    if (next) reveal(next.el);
  }

  // Vertical wheel scrolls sideways while there is something hidden.
  function wheel(e: WheelEvent) {
    if (!strip || (!hiddenLeft && !hiddenRight) || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
    e.preventDefault();
    strip.scrollBy({ left: e.deltaY });
  }

  // Not passive, so the page itself does not scroll at the same time.
  $effect(() => {
    const el = strip;
    if (!el) return;
    el.addEventListener("wheel", wheel, { passive: false });
    return () => el.removeEventListener("wheel", wheel);
  });

  $effect(() => {
    if (!strip) return;
    void options.map((o) => o.label + (o.builtin ? "*" : "")).join("|");
    measure();
    const ro = new ResizeObserver(() => { measure(); edges(); });
    ro.observe(strip);
    edges();
    return () => ro.disconnect();
  });

  // Show the chosen profile when it changes (and once at the start), but not
  // on every reload of the list: the pages poll it, and that must not pull
  // the strip back while someone is looking through it.
  let shown = "";
  $effect(() => {
    const v = value;
    const el = items()[options.findIndex((o) => o.value === v)];
    if (!el || v === shown) return;
    shown = v;
    queueMicrotask(() => { reveal(el); edges(); });
  });
</script>

<div class="picker" class:quiet style:max-width={cap ? `min(100%, ${cap}px)` : undefined}>
  <div
    class="strip"
    class:fade-l={hiddenLeft}
    class:fade-r={hiddenRight}
    role="radiogroup"
    aria-label={label}
    bind:this={strip}
    onscroll={edges}
    onfocusin={(e) => e.target instanceof HTMLElement && e.target.parentElement === strip && reveal(e.target)}
  >
    {#each options as o (o.value)}
      <button
        type="button"
        role="radio"
        aria-checked={o.value === value}
        title={o.label}
        dir="auto"
        {disabled}
        onclick={() => onchange(o.value)}
      >{o.label}</button>
    {/each}
  </div>
  {#if hiddenLeft}
    <button type="button" class="arrow l" tabindex="-1" aria-label={t().gm.moreProfiles} onclick={() => step("left")}><HugeIcon name="chevron-left" size={15} /></button>
  {/if}
  {#if hiddenRight}
    <button type="button" class="arrow r" tabindex="-1" aria-label={t().gm.moreProfiles} onclick={() => step("right")}><HugeIcon name="chevron-right" size={15} /></button>
  {/if}
  {#if hiddenLeft || hiddenRight}
    <div class="bar" aria-hidden="true">
      <span
        class="thumb"
        role="presentation"
        style:left={`${thumb[0]}%`}
        style:width={`${thumb[1]}%`}
        onpointerdown={grab}
        onpointermove={pull}
        onpointerup={() => (drag = null)}
        onpointercancel={() => (drag = null)}
      ></span>
    </div>
  {/if}
</div>

<style>
  .picker {
    position: relative;
    display: inline-block;
    min-width: 0;
    max-width: 100%;
    vertical-align: middle;
    border-radius: 12px;
    background: var(--card-2);
  }
  .strip {
    display: flex;
    padding: 3px;
    gap: 2px;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
    overscroll-behavior-x: contain;
  }
  /* The native scrollbar differs between engines and adds height; the thin
     bar below is drawn in the strip's bottom padding instead. */
  .strip::-webkit-scrollbar { display: none; }
  .bar { position: absolute; left: 12px; right: 12px; bottom: 0.5px; height: 2px; }
  .thumb {
    position: absolute;
    top: 0;
    bottom: 0;
    min-width: 16px;
    border-radius: 3px;
    background: color-mix(in oklab, var(--text-3) 70%, transparent);
    cursor: grab;
    touch-action: none;
  }
  .thumb::before { content: ""; position: absolute; inset: -5px 0; } /* easier to grab */
  .thumb:hover { background: var(--text-2); }
  .strip.fade-l { -webkit-mask-image: linear-gradient(to right, transparent 0, #000 40px); mask-image: linear-gradient(to right, transparent 0, #000 40px); }
  .strip.fade-r { -webkit-mask-image: linear-gradient(to left, transparent 0, #000 40px); mask-image: linear-gradient(to left, transparent 0, #000 40px); }
  .strip.fade-l.fade-r {
    -webkit-mask-image: linear-gradient(to right, transparent 0, #000 40px, #000 calc(100% - 40px), transparent 100%);
    mask-image: linear-gradient(to right, transparent 0, #000 40px, #000 calc(100% - 40px), transparent 100%);
  }
  .strip button {
    all: unset;
    flex: none;
    max-width: min(16em, calc(100% - 64px)); /* a long name is cut with … and still leaves room for the arrows */
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 6px 14px;
    border-radius: 9px;
    font-size: 0.86rem;
    color: var(--text-2);
    cursor: pointer;
    transition: background 150ms ease-out, color 150ms ease-out;
  }
  .strip button:hover:not(:disabled) { color: var(--text); }
  .strip button[aria-checked="true"] { background: var(--accent); color: #fff; font-weight: 600; }
  .strip button:disabled { cursor: default; }
  .strip button:focus-visible { outline: 2px solid var(--accent-text); outline-offset: -2px; }
  .quiet { border-radius: 10px; }
  .quiet .strip button { padding: 5px 11px; border-radius: 8px; font-size: 0.8rem; }
  .quiet .strip button[aria-checked="true"] { background: var(--card); color: var(--text); }
  .arrow {
    all: unset;
    position: absolute;
    top: 3px;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    margin-top: 2px;
    border-radius: 999px;
    background: var(--card);
    color: var(--text);
    box-shadow: 0 1px 4px rgb(0 0 0 / 0.35);
    cursor: pointer;
  }
  .arrow:hover { background: var(--accent); color: #fff; }
  .arrow.l { left: 3px; }
  .arrow.r { right: 3px; }
</style>
