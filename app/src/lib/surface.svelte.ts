// The background: the dark default or pure black for OLED screens. A
// choice in Settings → Appearance, remembered on this computer, applied as
// data-surface on <html> so app.css carries it.

export type Surface = "dark" | "oled";

function stored(): Surface {
  try {
    return localStorage.getItem("ug.surface") === "oled" ? "oled" : "dark";
  } catch {
    return "dark";
  }
}

export const surface = $state({ current: stored() });

export function applySurface() {
  document.documentElement.dataset.surface = surface.current;
}

export function setSurface(s: Surface) {
  surface.current = s;
  try {
    localStorage.setItem("ug.surface", s);
  } catch {
    /* not remembered; still applies now */
  }
  applySurface();
}
