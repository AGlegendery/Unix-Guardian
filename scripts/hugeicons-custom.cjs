// Animated icons the hugeicons-animated set does not have, drawn for Unix
// Guardian. The shapes are Hugeicons' own free icons (@hugeicons/core-free-icons,
// MIT), the same family the set is built from, so stroke, corners and grid
// match; the motion follows the set's timing and easing. Read by
// port-hugeicons.cjs: `shape` names the free icon, `nodes` animates its
// elements by index, `root` the whole SVG. Same data format as the port.

"use strict";
const OUT = [0.23, 1, 0.32, 1]; // the set's settle curve
const IN_OUT = [0.77, 0, 0.175, 1]; // the set's wind-up curve
const nudge = (prop, by, delay = 0, duration = 0.5) => ({
  origin: "50% 50%",
  box: "fill-box",
  n: { [prop]: 0 },
  a: { [prop]: [0, by, -by * 0.2, 0], transition: { duration, delay, times: [0, 0.45, 0.75, 1], ease: "easeInOut" } },
});
const pop = (to, delay = 0, duration = 0.45, origin = "50% 50%") => ({
  origin,
  box: "fill-box",
  n: { scale: 1 },
  a: { scale: [1, to, 1], transition: { duration, delay, ease: "easeInOut" } },
});
const draw = (delay = 0, duration = 0.45) => ({
  origin: "50% 50%",
  box: "fill-box",
  n: { pathLength: 1 },
  a: { pathLength: [0, 1], transition: { duration, delay, ease: OUT } },
});

module.exports = [
  {
    // Monitoring: the trace runs out to the right and is drawn again.
    name: "pulse-rectangle-01",
    shape: "PulseRectangle01Icon",
    nodes: {
      1: {
        origin: "50% 50%",
        box: "fill-box",
        n: { pathLength: 1, pathOffset: 0, visibility: "visible" },
        a: {
          pathLength: [1, 0, 0, 0, 1],
          pathOffset: [0, 1, 0, 0, 0],
          visibility: ["visible", "visible", "hidden", "hidden", "visible"],
          transition: { duration: 1.1, times: [0, 0.42, 0.43, 0.5, 1], ease: ["easeIn", "linear", "linear", OUT] },
        },
      },
    },
  },
  {
    // Game mode: the pad rocks in the hands, the d-pad turns, buttons press.
    name: "game-controller-03",
    shape: "GameController03Icon",
    root: {
      origin: "50% 55%",
      n: { rotate: 0 },
      a: { rotate: [0, -7, 5, -3, 1.5, 0], transition: { duration: 0.75, ease: "easeInOut" } },
    },
    nodes: {
      1: nudge("translateY", 0.9, 0, 0.45),
      2: {
        origin: "7.5px 11.5px",
        box: "view-box",
        n: { rotate: 0 },
        a: { rotate: [0, 90], transition: { duration: 0.6, ease: OUT } },
      },
      3: pop(1.9, 0.1),
      4: pop(1.9, 0.22),
    },
  },
  {
    // System control: the ring closes to the bottom and opens again while
    // the line is pressed in.
    name: "shut-down",
    shape: "ShutDownIcon",
    nodes: {
      0: {
        origin: "50% 50%",
        box: "fill-box",
        n: { pathLength: 1, pathOffset: 0 },
        a: { pathLength: [1, 0.2, 1], pathOffset: [0, 0.4, 0], transition: { duration: 0.8, ease: IN_OUT } },
      },
      1: {
        origin: "50% 50%",
        box: "fill-box",
        n: { translateY: 0 },
        a: { translateY: [0, 2.5, 0], transition: { duration: 0.8, times: [0, 0.45, 1], ease: IN_OUT } },
      },
    },
  },
  {
    // Overlay: the sheets lift off each other and settle, top first.
    name: "layers-01",
    shape: "Layers01Icon",
    nodes: {
      0: lift(-4, 0),
      1: lift(-2.5, 0.06),
      2: lift(-1.2, 0.12),
    },
  },
  {
    // Recorder: the lens blinks like a shutter, the side flap breathes.
    name: "video-01",
    shape: "Video01Icon",
    nodes: {
      1: {
        origin: "17px 12px",
        box: "view-box",
        n: { translateX: 0, scaleY: 1 },
        a: { translateX: [0, 1.2, 0], scaleY: [1, 0.8, 1], transition: { duration: 0.6, delay: 0.1, ease: "easeInOut" } },
      },
      2: {
        origin: "50% 50%",
        box: "fill-box",
        n: { scale: 1 },
        a: { scale: [1, 0.4, 1.25, 1], transition: { duration: 0.7, times: [0, 0.35, 0.7, 1], ease: "easeInOut" } },
      },
    },
  },
  {
    // Driver fix: a turn of the wrench around its head, and back.
    name: "wrench-01",
    shape: "Wrench01Icon",
    root: {
      origin: "64.583% 35.417%",
      n: { rotate: 0 },
      a: {
        rotate: [0, -28, 10, -4, 0],
        transition: { duration: 0.9, times: [0, 0.35, 0.65, 0.85, 1], ease: "easeInOut" },
      },
    },
  },
  {
    // Test lab: the flask tips and the liquid sloshes the other way.
    name: "flask-conical",
    shape: "FlaskConicalIcon",
    root: {
      origin: "50% 90%",
      n: { rotate: 0 },
      a: { rotate: [0, -6, 5, -2, 0], transition: { duration: 0.9, ease: "easeInOut" } },
    },
    nodes: {
      2: {
        origin: "50% 50%",
        box: "fill-box",
        n: {
          d: "M5.49707 14.0183C5.74095 14.0063 5.99072 14 6.24707 14C9.99707 14 13.7471 17 17.4971 17C18.2729 17 18.9332 16.9384 19.4971 16.8334",
        },
        a: {
          d: [
            "M5.49707 14.0183C5.74095 14.0063 5.99072 14 6.24707 14C9.99707 14 13.7471 17 17.4971 17C18.2729 17 18.9332 16.9384 19.4971 16.8334",
            "M5.49707 16.9817C5.74095 16.9937 5.99072 17 6.24707 17C9.99707 17 13.7471 14 17.4971 14C18.2729 14 18.9332 14.0616 19.4971 14.1666",
            "M5.49707 14.0183C5.74095 14.0063 5.99072 14 6.24707 14C9.99707 14 13.7471 17 17.4971 17C18.2729 17 18.9332 16.9384 19.4971 16.8334",
          ],
          transition: { duration: 0.9, ease: "easeInOut" },
        },
      },
    },
  },
  {
    // Updates: the arrows go round once, the gear turns a tooth.
    name: "system-update-01",
    shape: "SystemUpdate01Icon",
    nodes: {
      0: {
        origin: "12px 12px",
        box: "view-box",
        n: { rotate: 0 },
        a: { rotate: [0, -60], transition: { duration: 0.9, ease: IN_OUT } },
      },
      1: {
        origin: "12px 12px",
        box: "view-box",
        n: { rotate: 0 },
        a: { rotate: [0, -25, 360], transition: { duration: 0.9, times: [0, 0.2, 1], ease: ["easeIn", "easeOut"] } },
      },
    },
  },
  {
    // Sidebar fold: both chevrons step the way the sidebar will go.
    name: "arrow-left-double",
    shape: "ArrowLeftDoubleIcon",
    nodes: { 0: nudge("translateX", -3.5, 0, 0.55), 1: nudge("translateX", -3.5, 0.08, 0.55) },
  },
  {
    name: "arrow-right-double",
    shape: "ArrowRightDoubleIcon",
    nodes: { 0: nudge("translateX", 3.5, 0.08, 0.55), 1: nudge("translateX", 3.5, 0, 0.55) },
  },
  {
    // Terminal: the prompt steps forward, the cursor blinks twice.
    name: "command-line",
    shape: "CommandLineIcon",
    nodes: {
      1: nudge("translateX", 2.5, 0, 0.5),
      2: {
        origin: "50% 50%",
        box: "fill-box",
        n: { opacity: 1 },
        a: { opacity: [1, 0, 1, 0, 1], transition: { duration: 0.9, ease: "linear" } },
      },
    },
  },
  {
    // Rules: the check is drawn into the shield.
    name: "security-check",
    shape: "SecurityCheckIcon",
    root: { origin: "50% 50%", n: { scale: 1 }, a: { scale: [1, 1.06, 1], transition: { duration: 0.5, ease: "easeInOut" } } },
    nodes: { 1: draw(0.1, 0.5) },
  },
  {
    // Block: the bar is struck through again and the sign shakes.
    name: "unavailable",
    shape: "UnavailableIcon",
    root: { origin: "50% 50%", n: { rotate: 0 }, a: { rotate: [0, -14, 10, -5, 0], transition: { duration: 0.6, ease: "easeInOut" } } },
    nodes: { 0: draw(0, 0.5) },
  },
  {
    // Limit: the needle swings back, overshoots and settles.
    name: "dashboard-speed-01",
    shape: "DashboardSpeed01Icon",
    nodes: {
      0: {
        origin: "12px 15px",
        box: "view-box",
        n: { rotate: 0 },
        a: { rotate: [0, -95, 30, 0], transition: { duration: 0.9, times: [0, 0.35, 0.7, 1], ease: [IN_OUT, OUT, "easeOut"] } },
      },
    },
  },
  {
    // Swap: the two arrows pass each other.
    name: "arrow-data-transfer-horizontal",
    shape: "ArrowDataTransferHorizontalIcon",
    nodes: { 0: nudge("translateX", -3.5, 0, 0.6), 1: nudge("translateX", 3.5, 0.08, 0.6) },
  },
  {
    // CPU: the chip is pressed into its socket, the pins spring out in
    // turn and the die marks are drawn again.
    name: "cpu",
    shape: "CpuIcon",
    root: { origin: "50% 50%", n: { scale: 1 }, a: { scale: [1, 0.88, 1.04, 1], transition: { duration: 0.6, times: [0, 0.3, 0.7, 1], ease: "easeInOut" } } },
    nodes: {
      1: nudge("translateY", -1.8, 0.1, 0.45),
      2: nudge("translateY", -1.8, 0.15, 0.45),
      3: nudge("translateY", 1.8, 0.2, 0.45),
      4: nudge("translateY", 1.8, 0.25, 0.45),
      5: draw(0.05, 0.45),
      6: draw(0.25, 0.4),
      7: nudge("translateX", 1.8, 0.3, 0.45),
      8: nudge("translateX", -1.8, 0.35, 0.45),
      9: nudge("translateX", -1.8, 0.4, 0.45),
      10: nudge("translateX", 1.8, 0.45, 0.45),
    },
  },
  {
    // Memory: the stick lifts and its cells flicker down and up.
    name: "ram-memory",
    shape: "RamMemoryIcon",
    root: { origin: "50% 50%", n: { translateY: 0 }, a: { translateY: [0, -2, 0.4, 0], transition: { duration: 0.6, times: [0, 0.4, 0.75, 1], ease: "easeInOut" } } },
    nodes: {
      1: nudge("translateY", 1.2, 0.05, 0.5),
      2: nudge("translateY", 1.2, 0.1, 0.5),
      3: {
        origin: "50% 50%",
        box: "fill-box",
        n: { pathLength: 1, scaleY: 1 },
        a: { pathLength: [0, 1], scaleY: [0.2, 1.15, 1], transition: { duration: 0.6, delay: 0.1, ease: OUT } },
      },
    },
  },
  {
    // Graphics: the card is pushed into its slot, the fan winds up and
    // the vents are drawn one after another.
    name: "gpu",
    shape: "GpuIcon",
    root: { origin: "15% 50%", n: { translateX: 0 }, a: { translateX: [0, 1.5, -0.3, 0], transition: { duration: 0.6, times: [0, 0.35, 0.7, 1], ease: "easeInOut" } } },
    nodes: {
      2: {
        origin: "50% 50%",
        box: "fill-box",
        n: { scale: 1 },
        a: { scale: [1, 0.35, 1.2, 1], transition: { duration: 0.75, times: [0, 0.35, 0.7, 1], ease: "easeInOut" } },
      },
      3: draw(0.15, 0.5),
    },
  },
  {
    // Network: the links are drawn from the top box down to the others.
    name: "network",
    shape: "NetworkIcon",
    nodes: {
      0: pop(1.12, 0, 0.4),
      1: pop(1.12, 0.45, 0.35),
      2: pop(1.12, 0.5, 0.35),
      3: draw(0, 0.25),
      4: draw(0.2, 0.35),
    },
  },
  {
    // Sleep: the moon rocks like a cradle.
    name: "moon-02",
    shape: "Moon02Icon",
    root: {
      origin: "50% 50%",
      n: { rotate: 0, scale: 1 },
      a: { rotate: [0, -28, 14, -5, 0], scale: [1, 0.9, 1.05, 1, 1], transition: { duration: 0.9, times: [0, 0.35, 0.65, 0.85, 1], ease: "easeInOut" } },
    },
  },
  {
    // Hibernate: the flake turns one arm and pulses.
    name: "snow",
    shape: "SnowIcon",
    root: {
      origin: "50% 50%",
      n: { rotate: 0, scale: 1 },
      a: { rotate: [0, 100, 90], scale: [1, 0.8, 1], transition: { duration: 0.8, ease: OUT } },
    },
  },
  {
    // Save: the disk is pressed, its shutter slides, the label is written.
    name: "floppy-disk",
    shape: "FloppyDiskIcon",
    root: { origin: "50% 50%", n: { scale: 1 }, a: { scale: [1, 0.92, 1.03, 1], transition: { duration: 0.5, ease: "easeInOut" } } },
    nodes: { 0: nudge("translateY", 1.5, 0, 0.45), 1: draw(0.15, 0.35) },
  },
  {
    // Export: the arrow drops into the tray, which gives a little.
    name: "download-04",
    shape: "Download04Icon",
    nodes: {
      0: {
        origin: "50% 50%",
        box: "fill-box",
        n: { translateY: 0 },
        a: { translateY: [0, -2.5, 2, 0], transition: { duration: 0.75, times: [0, 0.3, 0.7, 1], ease: "easeInOut" } },
      },
      1: {
        origin: "50% 50%",
        box: "fill-box",
        n: { scaleX: 1 },
        a: { scaleX: [1, 1, 1.15, 1], transition: { duration: 0.75, times: [0, 0.3, 0.7, 1], ease: "easeInOut" } },
      },
    },
  },
  {
    // Tuning: the three knobs slide along their tracks one after another
    // and come back.
    name: "sliders-horizontal",
    shape: "SlidersHorizontalIcon",
    nodes: {
      3: slide(5, 0),
      2: slide(-6, 0.08),
      4: slide(4, 0.16),
    },
  },
];

function lift(by, delay) {
  return {
    origin: "50% 50%",
    box: "fill-box",
    n: { translateY: 0 },
    a: {
      translateY: [0, by, -by * 0.2, 0],
      transition: { duration: 0.75, delay, times: [0, 0.4, 0.75, 1], ease: [OUT, "easeInOut", "easeOut"] },
    },
  };
}

function slide(by, delay) {
  return {
    origin: "50% 50%",
    box: "fill-box",
    n: { translateX: 0 },
    a: {
      translateX: [0, by, -by * 0.12, 0],
      transition: { duration: 0.8, delay, times: [0, 0.45, 0.8, 1], ease: [IN_OUT, "easeInOut", "easeOut"] },
    },
  };
}
