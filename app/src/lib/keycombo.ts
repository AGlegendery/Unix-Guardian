// Turns a key press in the window into a shortcut ("Meta+Shift+G") for the
// agent, which reads this spelling and refuses what can't be a shortcut.

const MODIFIER_KEYS = ["Control", "Alt", "Shift", "Meta", "OS", "Super", "Hyper", "AltGraph"];

/** The modifiers held, in KDE's order: "Meta+Ctrl+Alt+Shift". A modifier
 *  key's own press or release counts even where the browser's flags lag. */
export function modifiers(e: KeyboardEvent): string[] {
  const own = (names: string[]) => (names.includes(e.key) ? e.type === "keydown" : null);
  const held = (flag: boolean, names: string[]) => own(names) ?? flag;
  const m: string[] = [];
  if (held(e.metaKey, ["Meta", "OS", "Super"])) m.push("Meta");
  if (held(e.ctrlKey, ["Control"])) m.push("Ctrl");
  if (held(e.altKey, ["Alt"])) m.push("Alt");
  if (held(e.shiftKey, ["Shift"])) m.push("Shift");
  return m;
}

/** The key's name, from its place on the keyboard (so Shift+1 stays "1"
 *  and a Persian layout still gives Latin letters); null for a modifier. */
export function keyName(e: KeyboardEvent): string | null {
  if (MODIFIER_KEYS.includes(e.key)) return null;
  const c = e.code;
  if (/^Key[A-Z]$/.test(c)) return c.slice(3);
  if (/^Digit\d$/.test(c)) return c.slice(5);
  if (/^F\d{1,2}$/.test(c)) return c;
  const named: Record<string, string> = {
    ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right",
    PageUp: "PageUp", PageDown: "PageDown", Enter: "Return", NumpadEnter: "Return",
    Backquote: "Backquote", Quote: "Quote",
  };
  return named[c] ?? c;
}

/** The whole combination, or null while only modifiers are held. */
export function combo(e: KeyboardEvent): string | null {
  const k = keyName(e);
  return k === null ? null : [...modifiers(e), k].join("+");
}
