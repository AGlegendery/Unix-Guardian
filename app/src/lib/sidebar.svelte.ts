// Whether the sidebar shows icons only: the arrow in the sidebar and
// Settings → Window and tray change it, remembered on this computer. Narrow
// windows fold it by themselves without changing this (App.svelte).

function stored(): boolean {
  try {
    return localStorage.getItem("ug.sidebar") === "collapsed";
  } catch {
    return false;
  }
}

export const sidebar = $state({ collapsed: stored() });

export function setSidebarCollapsed(on: boolean) {
  sidebar.collapsed = on;
  try {
    localStorage.setItem("ug.sidebar", on ? "collapsed" : "open");
  } catch {
    /* not remembered; still applies now */
  }
}
