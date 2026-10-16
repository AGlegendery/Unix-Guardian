import { mount } from "svelte";
import HugeIcon, { hugeIconNames } from "../src/lib/icons/HugeIcon.svelte";

const q = new URLSearchParams(location.search);
document.documentElement.dataset.motion = q.get("motion") ?? "classic";
const only = q.get("only")?.split(",");
for (const name of hugeIconNames) {
  if (only && !only.includes(name)) continue;
  const b = document.createElement("button");
  b.className = "gi";
  b.title = name;
  b.style.cssText = "background:none;border:0;color:#ddd;padding:6px;width:44px;height:44px";
  document.getElementById("g")!.appendChild(b);
  mount(HugeIcon, { target: b, props: { name, size: 24 } });
}
