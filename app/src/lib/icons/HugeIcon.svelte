<!--
  An animated Hugeicon (hugeicons-animated.com, MIT; see LICENSE-hugeicons.md
  next to this file). It plays once when the pointer enters the button, link
  or nav item around it (or the icon itself), when that is clicked, and
  whenever `trigger` changes, e.g. after a copy. It moves in both motion
  styles: Classic only keeps pages and cards calm. The system's
  reduced-motion setting keeps it still.
-->
<script lang="ts" module>
  import data from "./hugeicons.json";
  import type { NodeDef } from "./player";

  export interface IconData {
    name: string;
    markup: string;
    root: NodeDef | null;
    nodes: NodeDef[];
    loops: boolean;
    overflow?: string;
  }
  // data.notice carries the icons' MIT licence into the built app.
  export const hugeIconsNotice: string = data.notice;
  const byName = new Map((data.icons as IconData[]).map((i) => [i.name, i]));
  export const hugeIconNames = [...byName.keys()];
  let uid = 0;

  function allowed() {
    return !window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  }
</script>

<script lang="ts">
  import { IconPlayer } from "./player";

  let {
    name,
    size = 18,
    class: cls = "",
    trigger = undefined,
    hover = true,
    label = undefined,
  }: {
    name: string;
    size?: number;
    class?: string;
    /** Plays the icon when this changes to a truthy value (not on the first render). */
    trigger?: unknown;
    /** Plays on pointer enter and click of the surrounding control. */
    hover?: boolean;
    /** Names the icon for screen readers when no text says what it is. */
    label?: string;
  } = $props();

  const icon = $derived(byName.get(name));
  const id = `ugi${uid++}-`;
  const markup = $derived(icon ? icon.markup.replaceAll("__UID__", id) : "");

  let svg: SVGSVGElement | undefined = $state();
  let player: IconPlayer | null = null;

  export function play() {
    if (!player || !allowed() || player.playing) return;
    player.set("n");
    player.to("a");
  }

  $effect(() => {
    if (!svg || !icon) return;
    void markup;
    const p = new IconPlayer(svg, icon.root, icon.nodes);
    player = p;
    if (!hover) return () => p.cancel();
    const host =
      svg.parentElement?.closest<HTMLElement>("button, a, [role='button'], [role='tab'], [role='radio'], label, .item") ??
      svg.parentElement!;
    const enter = () => play();
    // Icons that loop while hovered settle back when the pointer leaves;
    // the rest finish their single run, as in the original.
    const leave = () => {
      if (icon.loops && p.playing) p.to("n");
    };
    host.addEventListener("pointerenter", enter);
    host.addEventListener("pointerleave", leave);
    // A click plays it too (touch has no hover; a run in progress goes on).
    host.addEventListener("click", enter);
    return () => {
      host.removeEventListener("click", enter);
      host.removeEventListener("pointerenter", enter);
      host.removeEventListener("pointerleave", leave);
      p.cancel();
      if (player === p) player = null;
    };
  });

  // A change of `trigger` to a truthy value plays the icon, so a flag
  // like `copied` plays it once and not again when it resets.
  let seen = false;
  $effect(() => {
    const now = trigger;
    if (!seen) {
      seen = true;
      return;
    }
    if (now) play();
  });
</script>

<span
  class="hugeicon {cls}"
  style:width="{size}px"
  style:height="{size}px"
  role={label ? "img" : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : "true"}
>
  {#if icon}
    <svg
      bind:this={svg}
      xmlns="http://www.w3.org/2000/svg"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      style:overflow={icon.overflow ?? "visible"}
    >
      {@html markup}
    </svg>
  {/if}
</span>

<style>
  .hugeicon {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    line-height: 0;
  }
  .hugeicon svg {
    display: block;
  }
</style>
