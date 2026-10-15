#!/usr/bin/env node
// Ports animated Hugeicons (hugeicons-animated.com, MIT) from their React +
// motion source into plain data for app/src/lib/icons/HugeIcon.svelte.
//
//   node scripts/port-hugeicons.cjs <dir with the registry .json files>
//
// Each registry item (https://hugeicons-animated.com/r/<name>.json) is a
// React component. Rather than parse it, this runs it: the TSX is compiled
// with the app's TypeScript and evaluated against small stand-ins for
// react, motion/react and the registry's helpers, so the result is exactly
// the SVG the component renders plus, for every motion.* element, its
// resolved "normal" and "animate" variants. No React reaches the app.
//
// Output: app/src/lib/icons/hugeicons.json ({ notice, icons }, sorted by name).

"use strict";
const fs = require("fs");
const path = require("path");
const ts = require(path.join(__dirname, "../app/node_modules/typescript"));

const src = process.argv[2];
if (!src) {
  console.error("usage: port-hugeicons.cjs <dir with registry json files>");
  process.exit(2);
}
const out = path.join(__dirname, "../app/src/lib/icons/hugeicons.json");

// SVG attributes that keep their camelCase spelling.
const CAMEL_ATTRS = new Set([
  "viewBox", "maskUnits", "maskContentUnits", "clipPathUnits", "gradientUnits",
  "gradientTransform", "patternUnits", "patternTransform", "preserveAspectRatio",
  "pathLength", "stdDeviation", "markerWidth", "markerHeight", "refX", "refY",
]);
const UNITLESS = new Set(["opacity", "flex", "zIndex", "fillOpacity", "strokeOpacity", "strokeWidth", "lineHeight"]);
const kebab = (k) => k.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase());
const esc = (v) => String(v).replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;");

// ---- stand-ins -----------------------------------------------------------
const MOTION = Symbol("motion");
let idCounter = 0;
const react = {
  forwardRef: (fn) => (props) => fn(props, null),
  useId: () => `:ug${idCounter++}:`,
  useRef: (v) => ({ current: v }),
  useCallback: (f) => f,
  useMemo: (f) => f(),
  useImperativeHandle: () => {},
  useEffect: () => {},
  useLayoutEffect: () => {},
  useState: (v) => [typeof v === "function" ? v() : v, () => {}],
};
let imperative = null; // search-01 animates through useAnimate
const motionReact = {
  motion: new Proxy({}, { get: (_, tag) => ({ [MOTION]: tag }) }),
  useAnimation: () => ({ start: async () => {}, set: () => {} }),
  useReducedMotion: () => false,
  useAnimate: () => [{ current: null }, (target, keyframes, options) => {
    imperative = { keyframes, options };
    return Promise.resolve();
  }],
};
const jsxRuntime = {
  Fragment: Symbol("fragment"),
  jsx: (type, props) => h(type, props),
  jsxs: (type, props) => h(type, props),
};
function h(type, props) {
  if (typeof type === "function") return type(props);
  return { type, props: props || {} };
}
const stubs = {
  react,
  "react/jsx-runtime": jsxRuntime,
  "motion/react": motionReact,
  "@/lib/utils": { cn: (...c) => c.filter(Boolean).join(" ") },
  "@/lib/use-icon-animation": {
    useIconAnimation: () => ({ handleMouseEnter() {}, handleMouseLeave() {} }),
  },
};

function load(code, file) {
  const js = ts.transpileModule(code, {
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2022,
      jsx: ts.JsxEmit.ReactJSX,
    },
    fileName: file,
  }).outputText;
  const module = { exports: {} };
  const req = (m) => {
    if (!(m in stubs)) throw new Error(`${file}: unexpected import ${m}`);
    return stubs[m];
  };
  new Function("require", "module", "exports", js)(req, module, module.exports);
  return module.exports;
}

// ---- tree walk -------------------------------------------------------------
function resolveVariant(v, custom) {
  if (v === undefined) return undefined;
  return typeof v === "function" ? v(custom) : v;
}

// Motion's transform origin for an SVG child: px values are user units
// (view-box), fractions and percentages are of the element's own box.
function originOf(style, isRoot) {
  const s = style || {};
  const o = {};
  if (isRoot) {
    if (s.transformOrigin) {
      o.origin = String(s.transformOrigin).replace(/(-?[\d.]+)px/g, (_, n) => `${+(n / 24 * 100).toFixed(3)}%`);
    } else if (s.originX !== undefined || s.originY !== undefined) {
      o.origin = `${+((s.originX ?? 0.5) * 100).toFixed(3)}% ${+((s.originY ?? 0.5) * 100).toFixed(3)}%`;
    }
    return o;
  }
  let box = s.transformBox;
  if (s.transformOrigin) {
    o.origin = String(s.transformOrigin);
    if (!box) box = /px/.test(o.origin) ? "view-box" : "fill-box";
  } else {
    o.origin = `${+((s.originX ?? 0.5) * 100).toFixed(3)}% ${+((s.originY ?? 0.5) * 100).toFixed(3)}%`;
  }
  o.box = box || "fill-box";
  return o;
}

const STYLE_SKIP = new Set(["transformOrigin", "transformBox", "originX", "originY"]);
function cssOf(style, skipMotion) {
  if (!style) return "";
  return Object.entries(style)
    .filter(([k]) => !(skipMotion && STYLE_SKIP.has(k)))
    .map(([k, v]) => `${kebab(k)}:${typeof v === "number" && !UNITLESS.has(k) ? v + "px" : v}`)
    .join(";");
}

function convert(name, mod) {
  const Comp = Object.values(mod).find((v) => typeof v === "function");
  imperative = null;
  idCounter = 0;
  const tree = Comp({ size: 24 });
  const icon = { name, markup: "", root: null, nodes: [] };

  // The component's outer <div> (and search-01's <span>) wrap the <svg>.
  let node = tree;
  let wrapperStyle = null;
  while (node && node.type !== "svg" && !(node.type && node.type[MOTION] === "svg")) {
    if (node.type === "span") wrapperStyle = node.props.style;
    node = node.props.children;
  }
  if (!node) throw new Error(`${name}: no <svg>`);
  const rootIsMotion = node.type && node.type[MOTION];
  if (rootIsMotion) {
    icon.root = {
      ...originOf(node.props.style, true),
      n: resolveVariant(node.props.variants?.normal, node.props.custom),
      a: resolveVariant(node.props.variants?.animate, node.props.custom),
    };
  }
  for (const k of Object.keys(node.props)) {
    if (!["xmlns", "width", "height", "viewBox", "fill", "overflow", "children", "variants", "animate", "initial", "style", "custom"].includes(k)) {
      throw new Error(`${name}: unexpected svg prop ${k}`);
    }
  }
  // Most draw outside the 24×24 box while they move; a few clip on purpose
  // (the take-off plane flies out of frame).
  if (node.props.overflow && node.props.overflow !== "visible") icon.overflow = node.props.overflow;
  icon.markup = children(node.props.children, icon);
  icon.markup = icon.markup.replace(/:ug(\d+):/g, "__UID__$1");
  return { icon, wrapperStyle };
}

function children(c, icon) {
  if (c === undefined || c === null || c === false || c === true) return "";
  if (Array.isArray(c)) return c.map((x) => children(x, icon)).join("");
  if (typeof c === "string" || typeof c === "number") return esc(c);
  return element(c, icon);
}

function element(el, icon) {
  if (el.type === jsxRuntime.Fragment) return children(el.props.children, icon);
  const motionTag = el.type && el.type[MOTION];
  const tag = motionTag || el.type;
  if (typeof tag !== "string") throw new Error(`${icon.name}: odd element ${String(tag)}`);
  const p = el.props;
  let attrs = "";
  if (motionTag && p.variants) {
    const n = resolveVariant(p.variants.normal, p.custom);
    const a = resolveVariant(p.variants.animate, p.custom);
    const def = { ...originOf(p.style, false), n, a };
    attrs += ` data-ug-a="${icon.nodes.length}"`;
    icon.nodes.push(def);
  } else if (motionTag && (p.animate || p.initial)) {
    throw new Error(`${icon.name}: motion.${tag} without variants`);
  }
  for (const [k, v] of Object.entries(p)) {
    if (["children", "variants", "custom", "initial", "animate", "transition", "key"].includes(k)) continue;
    if (v === undefined || v === null || v === false) continue;
    if (k === "style") {
      const css = cssOf(v, Boolean(motionTag));
      if (css) attrs += ` style="${esc(css)}"`;
      continue;
    }
    if (typeof v === "function" || typeof v === "object") throw new Error(`${icon.name}: prop ${k} is ${typeof v}`);
    const name = k === "className" ? "class" : CAMEL_ATTRS.has(k) ? k : k === "xlinkHref" ? "href" : kebab(k);
    attrs += ` ${name}="${esc(v === true ? "" : v)}"`;
  }
  const inner = children(p.children, icon);
  return `<${tag}${attrs}>${inner}</${tag}>`;
}

// ---- main ------------------------------------------------------------------
const icons = [];
for (const f of fs.readdirSync(src).filter((f) => f.endsWith(".json")).sort()) {
  const item = JSON.parse(fs.readFileSync(path.join(src, f), "utf8"));
  if (!item.files || item.type !== "registry:ui") continue;
  const file = item.files.find((x) => x.path.startsWith("icons/"));
  const name = item.name;
  const mod = load(file.content, file.path);
  let { icon, wrapperStyle } = convert(name, mod);
  if (/useAnimate\b/.test(file.content)) {
    // search-01: the handle's startAnimation animates the wrapper span.
    const handle = {};
    const Comp = Object.values(mod).find((v) => typeof v === "function");
    react.useImperativeHandle = (_ref, make) => Object.assign(handle, make());
    const origQuery = { current: { querySelector: () => ({}) } };
    motionReact.useAnimate = () => [origQuery, (t, keyframes, options) => {
      imperative = { keyframes, options };
      return Promise.resolve();
    }];
    Comp({ size: 24 });
    handle.startAnimation();
    react.useImperativeHandle = () => {};
    if (!imperative) throw new Error(`${name}: no imperative animation found`);
    const { transition: _t, ...kf } = imperative.keyframes;
    const first = Object.fromEntries(Object.entries(kf).map(([k, v]) => [k, Array.isArray(v) ? v[0] : v]));
    icon.root = { ...originOf(wrapperStyle, true), n: first, a: { ...kf, transition: imperative.options } };
  }
  icon.loops = /loops:\s*true/.test(file.content);
  icons.push(icon);
}
fs.mkdirSync(path.dirname(out), { recursive: true });
// JSON has no Infinity: an endless repeat is written as -1.
// The MIT notice travels inside the data, so it is in every built copy.
const notice =
  "Animated icons: hugeicons-animated, MIT License, Copyright (c) 2026 Abdullah Enes Gules. " +
  "Icon shapes: Hugeicons free icons, MIT License, Copyright (c) Hugeicons. " +
  "Permission is hereby granted, free of charge, to any person obtaining a copy of this software " +
  "and associated documentation files (the \"Software\"), to deal in the Software without restriction, " +
  "including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, " +
  "and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, " +
  "subject to the following conditions: The above copyright notice and this permission notice shall be " +
  "included in all copies or substantial portions of the Software. THE SOFTWARE IS PROVIDED \"AS IS\", " +
  "WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF " +
  "MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR " +
  "COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, " +
  "TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS " +
  "IN THE SOFTWARE.";
const json = JSON.stringify({ notice, icons }, (_, v) => (v === Infinity ? -1 : v));
fs.writeFileSync(out, json + "\n");
console.log(`${icons.length} icons -> ${path.relative(process.cwd(), out)} (${fs.statSync(out).size} bytes)`);
