// Two languages, Persian first. A plain typed dictionary rather than Paraglide
// (02-decisions.md planned Paraglide): Paraglide's compiler fetches its
// message-format plugin from a CDN at build time, and distribution builds run
// without network. English is the type every other locale must match, so a
// missing Persian string is a compile error, not a blank label.

import { en } from "./messages/en";
import { fa } from "./messages/fa";

export type Locale = "fa" | "en";
export type Messages = typeof en;

const dictionaries: Record<Locale, Messages> = { fa, en };

function stored(): Locale {
  try {
    const v = localStorage.getItem("ug.locale");
    if (v === "fa" || v === "en") return v;
  } catch {
    /* storage unavailable: fall through to the default */
  }
  return "fa";
}

export const locale = $state({ current: stored() });

export function setLocale(l: Locale) {
  locale.current = l;
  try {
    localStorage.setItem("ug.locale", l);
  } catch {
    /* not persisted; still switches for this session */
  }
  applyDocumentLocale();
}

export function applyDocumentLocale() {
  document.documentElement.lang = locale.current;
  document.documentElement.dir = locale.current === "fa" ? "rtl" : "ltr";
}

/** The current dictionary. Read inside markup or $derived so it re-renders. */
export function t(): Messages {
  return dictionaries[locale.current];
}
