// The motion style: Classic (the calm original) or Lively (pages and cards
// slide in, hover lifts, presses give). A choice in Settings, remembered on
// this computer, applied as data-motion on <html> so app.css carries it.
// Every Lively animation runs once and ends: WebKitGTK repaints at 60 fps
// while anything moves, and one endless pulse once cost a whole core. The
// system's reduced-motion setting turns all of it off (app.css).

export type Motion = "classic" | "lively";

function stored(): Motion {
  try {
    return localStorage.getItem("ug.motion") === "lively" ? "lively" : "classic";
  } catch {
    return "classic";
  }
}

export const motion = $state({ current: stored() });

export function applyMotion() {
  document.documentElement.dataset.motion = motion.current;
}

export function setMotion(m: Motion) {
  motion.current = m;
  try {
    localStorage.setItem("ug.motion", m);
  } catch {
    /* not remembered; still applies now */
  }
  applyMotion();
}
