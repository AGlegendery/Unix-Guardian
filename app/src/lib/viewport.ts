// How the window's scroll areas and page scale behave, the same on every page.

/** How long a scrollbar stays visible after the last scroll. */
const SCROLLBAR_LINGER_MS = 900;

/**
 * Scrollbars fade out when idle (the user, 2026-10-17): a box that scrolls
 * carries `data-scrolling` for a moment after its last scroll, and app.css
 * shows its thumb only then (or while the pointer is on the thumb). One
 * listener for the whole document; scroll events do not bubble, so it
 * listens in the capture phase.
 */
export function watchScrolling() {
  const timers = new WeakMap<Element, number>();
  document.addEventListener(
    "scroll",
    (e) => {
      const el = e.target === document ? document.scrollingElement : (e.target as Element);
      if (!(el instanceof Element)) return;
      el.setAttribute("data-scrolling", "");
      clearTimeout(timers.get(el));
      timers.set(el, window.setTimeout(() => el.removeAttribute("data-scrolling"), SCROLLBAR_LINGER_MS));
    },
    { capture: true, passive: true },
  );
}

/**
 * The page never zooms: Ctrl + wheel, Ctrl + plus/minus/zero and WebKit's
 * gesture events are swallowed. The window also drops touchpad pinches
 * before WebKitGTK sees them (src-tauri main.rs).
 */
export function blockZoom() {
  window.addEventListener(
    "wheel",
    (e) => {
      if (e.ctrlKey) e.preventDefault();
    },
    { passive: false, capture: true },
  );
  window.addEventListener(
    "keydown",
    (e) => {
      if ((e.ctrlKey || e.metaKey) && ["+", "=", "-", "_", "0"].includes(e.key)) e.preventDefault();
    },
    { capture: true },
  );
  for (const type of ["gesturestart", "gesturechange", "gestureend"]) {
    window.addEventListener(type, (e) => e.preventDefault(), { passive: false });
  }
}

/**
 * Svelte action: makes a scroll area redraw at its real offset when it
 * changes size. When a box grows past its content, the browser lowers its
 * offset itself without a scroll event, and the real window (WebKitGTK)
 * went on drawing the old offset: a sidebar scrolled to the bottom in a
 * short window stayed scrolled once the window grew, its first items
 * hidden above the top with nothing left to scroll (2026-10-17). Layout
 * reads were right all along, so this rebuilds the box's scrolling layer
 * whenever it changes size.
 */
export function clampScroll(el: HTMLElement) {
  // Runs only when the box changes size (a window resize), so the cost of
  // the rebuild does not matter; a scroll event of its own cannot be relied
  // on to say the offset moved.
  const fit = () => {
    const max = Math.max(0, el.scrollHeight - el.clientHeight);
    const want = Math.min(el.scrollTop, max);
    const keep = el.style.overflow;
    el.style.overflow = "hidden";
    void el.offsetHeight;
    el.style.overflow = keep;
    el.scrollTop = want;
  };
  const ro = new ResizeObserver(fit);
  ro.observe(el);
  return { destroy: () => ro.disconnect() };
}
