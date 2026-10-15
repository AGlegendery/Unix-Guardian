/*! Animated icons from hugeicons-animated (MIT, (c) 2026 Abdullah Enes Gules)
 * and Hugeicons free icons (MIT, (c) 2025 Hugeicons). Full notice:
 * app/src/lib/icons/LICENSE-hugeicons.md */
// Plays the animated Hugeicons' motion data (hugeicons.json, made by
// scripts/port-hugeicons.cjs from the React + motion originals) with no
// animation library: keyframes, times, per-segment easing, delays, springs
// and repeats, written to the SVG on requestAnimationFrame. It follows
// motion's rules where they show: transform props go to the CSS transform,
// pathLength/pathOffset become a dash pattern on pathLength="1", everything
// else is an SVG attribute.
//
// Every run ends. An "endless" repeat in the original plays twice here:
// WebKitGTK repaints at 60 fps while anything moves (CLAUDE.md), so an icon
// must never keep a frame loop alive on its own.

type Value = number | string;
type Ease = string | number[];
export interface Transition {
  type?: string;
  duration?: number;
  delay?: number;
  ease?: Ease | Ease[];
  times?: number[];
  repeat?: number;
  repeatType?: string;
  repeatDelay?: number;
  bounce?: number;
  stiffness?: number;
  damping?: number;
  mass?: number;
  [prop: string]: unknown;
}
export type Target = { transition?: Transition; transitionEnd?: Record<string, Value> } & Record<string, unknown>;
export interface NodeDef {
  origin?: string;
  box?: string;
  n?: Target | null;
  a?: Target | null;
}

const ENDLESS_RUNS = 2;
const TRANSFORM = ["x", "y", "translateX", "translateY", "scale", "scaleX", "scaleY", "rotate"];
const CAMEL_ATTRS = new Set(["pathLength"]);
const kebab = (k: string) => (CAMEL_ATTRS.has(k) ? k : k.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase()));

// ---- easing ----------------------------------------------------------------
function bezier(x1: number, y1: number, x2: number, y2: number) {
  const a = (a1: number, a2: number) => 1 - 3 * a2 + 3 * a1;
  const b = (a1: number, a2: number) => 3 * a2 - 6 * a1;
  const c = (a1: number) => 3 * a1;
  const at = (t: number, a1: number, a2: number) => ((a(a1, a2) * t + b(a1, a2)) * t + c(a1)) * t;
  const slope = (t: number, a1: number, a2: number) => 3 * a(a1, a2) * t * t + 2 * b(a1, a2) * t + c(a1);
  return (x: number) => {
    if (x <= 0 || x >= 1) return x;
    let t = x;
    for (let i = 0; i < 8; i++) {
      const s = slope(t, x1, x2);
      if (Math.abs(s) < 1e-6) break;
      t -= (at(t, x1, x2) - x) / s;
    }
    if (t < 0 || t > 1) {
      let lo = 0, hi = 1;
      t = x;
      for (let i = 0; i < 30; i++) {
        if (at(t, x1, x2) < x) lo = t; else hi = t;
        t = (lo + hi) / 2;
      }
    }
    return at(t, y1, y2);
  };
}
const NAMED: Record<string, number[]> = {
  easeIn: [0.42, 0, 1, 1],
  easeOut: [0, 0, 0.58, 1],
  easeInOut: [0.42, 0, 0.58, 1],
  circIn: [0.55, 0, 1, 0.45],
  circOut: [0, 0.55, 0.45, 1],
  circInOut: [0.85, 0, 0.15, 1],
  backIn: [0.31, 0.01, 0.66, -0.59],
  backOut: [0.33, 1.53, 0.69, 0.99],
  backInOut: [0.68, -0.6, 0.32, 1.6],
};
function easing(e: Ease | undefined): (t: number) => number {
  if (Array.isArray(e) && e.length === 4) return bezier(e[0], e[1], e[2], e[3]);
  if (typeof e === "string" && NAMED[e]) return bezier(...(NAMED[e] as [number, number, number, number]));
  if (e === "anticipate") {
    const back = bezier(0.33, -0.53, 0.69, 0.99);
    return (t) => ((t *= 2) < 1 ? 0.5 * back(t) : 0.5 * (2 - Math.pow(2, -10 * (t - 1))));
  }
  return (t) => t; // linear
}

// A spring as a 0..1 curve over its own settle time.
function spring(tr: Transition): { ease: (t: number) => number; duration: number } {
  let zeta: number, omega: number, duration: number;
  if (tr.stiffness !== undefined || tr.damping !== undefined || tr.mass !== undefined) {
    const k = tr.stiffness ?? 100, c = tr.damping ?? 10, m = tr.mass ?? 1;
    omega = Math.sqrt(k / m);
    zeta = c / (2 * Math.sqrt(k * m));
    duration = Math.min(2, 6.9 / (Math.max(zeta, 0.05) * omega));
  } else {
    duration = tr.duration ?? 0.8;
    zeta = Math.max(0.05, 1 - (tr.bounce ?? 0.25));
    omega = (zeta >= 1 ? 9.2 : 6.9 / zeta) / duration;
  }
  const ease = (t: number) => {
    const s = t * duration;
    if (zeta >= 1) return 1 - (1 + omega * s) * Math.exp(-omega * s);
    const wd = omega * Math.sqrt(1 - zeta * zeta);
    return 1 - Math.exp(-zeta * omega * s) * (Math.cos(wd * s) + ((zeta * omega) / wd) * Math.sin(wd * s));
  };
  return { ease: (t) => (t >= 1 ? 1 : ease(t)), duration };
}

// ---- values ----------------------------------------------------------------
const NUM = /-?\d*\.?\d+(?:e[-+]?\d+)?/gi;
function mix(prop: string, a: Value, b: Value, p: number): Value {
  if (typeof a === "number" && typeof b === "number") return a + (b - a) * p;
  if (prop === "visibility") {
    // motion: becoming visible happens at once, hiding only at the end.
    if (a === "hidden" || b === "visible") return p <= 0 ? a : b;
    return p >= 1 ? b : a;
  }
  const sa = String(a), sb = String(b);
  const na = sa.match(NUM), nb = sb.match(NUM);
  if (na && nb && na.length === nb.length && sa.replace(NUM, "#") === sb.replace(NUM, "#")) {
    let i = 0;
    return sa.replace(NUM, () => {
      const x = parseFloat(na[i]), y = parseFloat(nb[i]);
      i++;
      return String(+(x + (y - x) * p).toFixed(4));
    });
  }
  return p < 0.5 ? a : b;
}

const DEFAULTS: Record<string, Value> = {
  x: 0, y: 0, translateX: 0, translateY: 0, scale: 1, scaleX: 1, scaleY: 1, rotate: 0,
  opacity: 1, pathLength: 1, pathOffset: 0, visibility: "visible",
};

interface Item {
  el: Element;
  def: NodeDef;
  root: boolean;
  values: Record<string, Value>;
  dirty: boolean;
}
interface Track {
  item: Item;
  prop: string;
  kf: Value[];
  times: number[];
  eases: ((t: number) => number)[];
  duration: number;
  delay: number;
  runs: number;
  mirror: boolean;
  repeatDelay: number;
  end?: Record<string, Value>;
}

export class IconPlayer {
  private items: Item[] = [];
  private tracks: Track[] = [];
  private frame = 0;
  private start = 0;
  private onDone: (() => void) | null = null;

  constructor(svg: SVGSVGElement, root: NodeDef | null, nodes: NodeDef[]) {
    if (root) this.items.push({ el: svg, def: root, root: true, values: {}, dirty: false });
    svg.querySelectorAll("[data-ug-a]").forEach((el) => {
      const def = nodes[Number(el.getAttribute("data-ug-a"))];
      if (def) this.items.push({ el, def, root: false, values: {}, dirty: false });
    });
    for (const it of this.items) {
      const s = (it.el as SVGElement).style;
      if (it.def.origin) s.transformOrigin = it.def.origin;
      if (it.def.box) s.setProperty("transform-box", it.def.box);
    }
    this.set("n");
  }

  get playing() {
    return this.frame !== 0;
  }

  /** Jumps to a variant with no motion. */
  set(which: "n" | "a") {
    this.cancel();
    for (const it of this.items) {
      const t = it.def[which];
      if (!t) continue;
      for (const [k, v] of Object.entries(t)) {
        if (k === "transition" || k === "transitionEnd") continue;
        const val = Array.isArray(v) ? v[v.length - 1] : v;
        if (val !== null && val !== undefined) it.values[k] = val as Value;
      }
      if (t.transitionEnd) Object.assign(it.values, t.transitionEnd);
      it.dirty = true;
    }
    this.render();
  }

  /** Animates to a variant ("a" plays the icon, "n" settles back). */
  to(which: "n" | "a", onDone?: () => void) {
    this.cancel();
    this.onDone = onDone ?? null;
    for (const it of this.items) {
      const t = it.def[which];
      if (!t) continue;
      const base: Transition = t.transition ?? {};
      for (const [prop, v] of Object.entries(t)) {
        if (prop === "transition" || prop === "transitionEnd") continue;
        const own = base[prop];
        const tr: Transition = own && typeof own === "object" ? (own as Transition) : base;
        const current = it.values[prop] ?? DEFAULTS[prop] ?? (Array.isArray(v) ? v[0] : v);
        let kf: Value[] = Array.isArray(v) ? (v as Value[]).slice() : [current, v as Value];
        if (kf.length === 1) kf.unshift(current);
        if (kf[0] === null || kf[0] === undefined) kf[0] = current;
        this.tracks.push(this.track(it, prop, kf, tr, t.transitionEnd));
      }
    }
    if (this.tracks.length === 0) return this.finish();
    this.start = performance.now();
    this.frame = requestAnimationFrame(this.tick);
  }

  cancel() {
    if (this.frame) cancelAnimationFrame(this.frame);
    this.frame = 0;
    this.tracks = [];
    this.onDone = null;
  }

  private track(item: Item, prop: string, kf: Value[], tr: Transition, end?: Record<string, Value>): Track {
    const n = kf.length;
    let duration = tr.duration ?? (n > 2 ? 0.8 : 0.3);
    let eases: ((t: number) => number)[];
    if (tr.type === "spring" && n === 2) {
      const s = spring(tr);
      duration = s.duration;
      eases = [s.ease];
    } else {
      const e = tr.ease ?? (tr.type === "spring" ? "easeOut" : "easeInOut");
      const list = Array.isArray(e) && e.length > 0 && typeof e[0] !== "number" ? (e as Ease[]) : [e as Ease];
      eases = Array.from({ length: n - 1 }, (_, i) => easing(list[Math.min(i, list.length - 1)]));
    }
    const times = tr.times && tr.times.length === n ? tr.times : kf.map((_, i) => (n === 1 ? 1 : i / (n - 1)));
    const repeat = tr.repeat ?? 0;
    const runs = repeat < 0 ? ENDLESS_RUNS : Math.min(repeat + 1, ENDLESS_RUNS + 1);
    return {
      item, prop, kf, times, eases, duration: Math.max(duration, 0), delay: tr.delay ?? 0, runs,
      mirror: tr.repeatType === "reverse" || tr.repeatType === "mirror",
      repeatDelay: tr.repeatDelay ?? 0, end,
    };
  }

  private sample(t: Track, elapsed: number): { v: Value; done: boolean } {
    const local = elapsed - t.delay;
    if (local <= 0) return { v: t.kf[0], done: false };
    if (t.duration === 0) return { v: t.kf[t.kf.length - 1], done: true };
    const cycle = t.duration + t.repeatDelay;
    const total = cycle * t.runs - t.repeatDelay;
    let run = Math.floor(local / cycle);
    let p = (local - run * cycle) / t.duration;
    const done = local >= total;
    if (done) {
      run = t.runs - 1;
      p = 1;
    }
    p = Math.min(p, 1);
    if (t.mirror && run % 2 === 1) p = 1 - p;
    const { kf, times } = t;
    let i = 0;
    while (i < kf.length - 2 && p > times[i + 1]) i++;
    const span = times[i + 1] - times[i];
    const lp = span > 0 ? Math.min(1, Math.max(0, (p - times[i]) / span)) : 1;
    return { v: mix(t.prop, kf[i], kf[i + 1], t.eases[Math.min(i, t.eases.length - 1)](lp)), done };
  }

  private tick = (now: number) => {
    const elapsed = (now - this.start) / 1000;
    let live = false;
    for (const t of this.tracks) {
      const { v, done } = this.sample(t, elapsed);
      t.item.values[t.prop] = v;
      t.item.dirty = true;
      if (!done) live = true;
      else if (t.end) Object.assign(t.item.values, t.end);
    }
    this.render();
    if (live) this.frame = requestAnimationFrame(this.tick);
    else this.finish();
  };

  private finish() {
    this.frame = 0;
    this.tracks = [];
    const cb = this.onDone;
    this.onDone = null;
    cb?.();
  }

  private render() {
    for (const it of this.items) {
      if (!it.dirty) continue;
      it.dirty = false;
      const v = it.values;
      const el = it.el as SVGElement;
      let transform = typeof v.transform === "string" ? v.transform : "";
      const parts: string[] = [];
      const tx = Number(v.x ?? 0) + Number(v.translateX ?? 0);
      const ty = Number(v.y ?? 0) + Number(v.translateY ?? 0);
      if (tx) parts.push(`translateX(${tx}px)`);
      if (ty) parts.push(`translateY(${ty}px)`);
      for (const s of ["scale", "scaleX", "scaleY"]) if (v[s] !== undefined && v[s] !== 1) parts.push(`${s}(${v[s]})`);
      if (v.rotate) parts.push(`rotate(${v.rotate}deg)`);
      if (parts.length) transform = (transform + " " + parts.join(" ")).trim();
      el.style.transform = transform;
      if (v.pathLength !== undefined || v.pathOffset !== undefined) {
        el.setAttribute("pathLength", "1");
        el.setAttribute("stroke-dasharray", `${v.pathLength ?? 1} 1`);
        el.setAttribute("stroke-dashoffset", String(-Number(v.pathOffset ?? 0)));
      }
      for (const [k, val] of Object.entries(v)) {
        if (k === "transform" || k === "pathLength" || k === "pathOffset" || TRANSFORM.includes(k)) continue;
        el.setAttribute(kebab(k), String(val));
      }
    }
  }
}
