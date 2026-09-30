// Numbers in the reader's script: Persian digits in the Persian UI, Latin in
// English. Formatters are built once per locale; the monitor formats a dozen
// values a second and Intl constructors are the expensive part.

import { locale } from "./i18n.svelte";

const cache = new Map<string, Intl.NumberFormat>();

function nf(digits: number): Intl.NumberFormat {
  const tag = locale.current === "fa" ? "fa-IR" : "en-US";
  const key = `${tag}:${digits}`;
  let f = cache.get(key);
  if (!f) {
    f = new Intl.NumberFormat(tag, { maximumFractionDigits: digits, minimumFractionDigits: digits, useGrouping: true });
    cache.set(key, f);
  }
  return f;
}

/** A number, or an em dash when the machine does not report it. */
export function num(v: number | null | undefined, digits = 0): string {
  return v == null || Number.isNaN(v) ? "—" : nf(digits).format(v);
}

/** Bytes per second in the largest unit that keeps the number readable. */
export function rate(bps: number): { value: string; unit: "B/s" | "kB/s" | "MB/s" } {
  if (bps >= 1_000_000) return { value: num(bps / 1_000_000, 1), unit: "MB/s" };
  if (bps >= 1_000) return { value: num(bps / 1_000, 0), unit: "kB/s" };
  return { value: num(bps, 0), unit: "B/s" };
}

/** GiB with one decimal, from KiB. */
export function gib(kb: number): string {
  return num(kb / 1_048_576, 1);
}

/** 3:07 below an hour, 12:05 as hours:minutes above. */
export function duration(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const pad = (n: number) => nf(0).format(n).padStart(2, locale.current === "fa" ? "۰" : "0");
  return h > 0 ? `${nf(0).format(h)}:${pad(m)}` : `${nf(0).format(m)}:${pad(Math.floor(seconds % 60))}`;
}
