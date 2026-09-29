# Roadmap

The full roadmap — 11 phases, 84 tasks with acceptance criteria — is published at
<https://claude.ai/artifact/KNhtWo36zHutPKv7UKuMw6>. This file tracks where the
code actually is against it.

## Status

| Phase | Scope | State |
|-------|-------|-------|
| P0 | Repo, CI, D-Bus contract, fixtures, logging | **Done** except the Tauri shell (0.6), which needs `libwebkit2gtk-4.1-dev` |
| P1 | Daemon, sensors, GPU read path, CLI | **Done** except the agent (1.2), the UI (1.7) and the perf-budget gate (1.9) |
| P2 | Game mode, detection, profiles, playtime | Next |
| P3–P10 | See roadmap | Not started |

## AMD verification

The AMD read path (`crates/ug-gpu/src/amd.rs`) is written and tested against
synthetic RDNA1–RDNA3 layouts, but has not yet run on a physical AMD card. To
close that gap, on a machine with an RX 5000-series or newer card:

```bash
./scripts/capture-fixture.sh rx6700xt     # read-only, no root needed
```

and add the resulting fixture under `crates/ug-testfs/fixtures/`. Then run the
daemon there with `ug-daemon --session` and compare `ug status` against
`radeontop` or the numbers in `/sys/class/drm/card*/device`.

## Findings from running on real hardware

- **Polling wakes a sleeping laptop dGPU.** Any NVML call (and any `amdgpu`
  sysfs read) brings a runtime-suspended GPU back up — 1.5 s the first time, and
  it then stays powered while polling continues. Telemetry now checks
  `power/runtime_status` first and reports the GPU as asleep instead. Verified:
  5 s of 1 Hz monitoring added 0 ms of dGPU active time.
- **NVML costs ~15 MB while loaded and wakes a sleeping GPU to load.** It is now
  loaded only for a GPU that is already awake, and released 30 s after last use.
  Measured on the release build: daemon starts in 59 ms without waking the dGPU,
  idles at 6.2 MB, stays at 6.3 MB while monitoring a sleeping dGPU, rises to
  21.7 MB while NVML is needed and returns to 6.4 MB ~31 s after. CPU: 0.000 %
  idle, 0.2 % sampling at 1 Hz. (Roadmap budget 15 MB is met whenever NVML is not
  in use; while reading an NVIDIA GPU the floor is NVIDIA's library.)
- **GPU names come from `pci.ids`, not a hand table.** The one hand-written entry
  checked against real hardware was wrong; NVIDIA's own `/proc` info file wakes
  the GPU, so it is not usable either.
- **RAPL is root-only on current kernels** (`energy_uj` is mode 0400 since the
  PLATYPUS mitigation). CPU package power appears only under the system service.
- **NVIDIA laptop fans belong to the EC**, not the driver; `fan_speed` errors and
  the capability report says so.
