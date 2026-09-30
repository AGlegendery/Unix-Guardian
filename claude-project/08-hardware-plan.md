# 08 — Hardware plan: what needs which machine

## The user's laptop covers Intel and NVIDIA

The user asked whether their Intel graphics helps: **yes, and it is already in
use.** It is the same laptop's integrated GPU (Intel UHD, Alder Lake-HX):

- **Intel iGPU:** detected and read (current frequency). It has no temperature
  of its own (it is inside the CPU package sensor) and **Linux exposes no tuning
  interface for Intel graphics** — Intel stays monitoring-only by design.
- **Intel hybrid CPU:** gave the "preferred cores" feature (P-cores 0–7),
  verified on this laptop.
- **What this laptop will also cover later:** P3 power control (logind, RTC
  wake), P4 network (Wi-Fi), P6 MangoHud overlay (install MangoHud first), P7
  recording through **VAAPI on the Intel UHD — the same API AMD cards use**, and
  NVENC on the NVIDIA, and **P8 NVIDIA driver troubleshooting plus Optimus
  switching**, for which a hybrid laptop with the proprietary NVIDIA driver is the
  ideal test machine.

No other Intel machine is needed.

## The friend's AMD machine — how much, and when

| When | Phase | What to do there | Time | Consent / risk |
|---|---|---|---|---|
| **Any time now** | P1 | Run `scripts/capture-fixture.sh` (read-only, no root, bash only) and bring back the ~20 KB `.tar.gz`. Optional: run `ug-daemon --session` + `ug --session status` and compare with `radeontop` or the sysfs numbers. | **~5 min** (15 with the optional part) | None — reads only. |
| Now, *only if* the friend's CPU is a multi-CCD Ryzen (7900X/7950X/…X3D) | P2 | The same capture includes CPU topology; the agent's V-Cache pinning can be checked live. | +10 min | None. |
| **P5** (GPU tuning & fan curves; roadmap weeks ~15–21) | P5 | 1) add `amdgpu.ppfeaturemask=0xffffffff` to the boot parameters and reboot, confirm the overdrive files for their generation; 2) stability test + auto-tune (GPU at full load 10–30 min per run); 3) fan-curve calibration (~15 min) and the failsafe test (`kill -9` the daemon → fan returns to automatic within 5 s). | **2–3 visits × 2–3 h** | **Needs the friend's explicit agreement**: a boot-parameter change and sustained GPU stress. Back up / snapshot first. |
| P7 | P7 | AMD encoding through `gpu-screen-recorder` (VAAPI) — mostly covered by the Intel UHD already. | optional 30 min | None. |
| P8 | P8 | AMD diagnosis rules (ppfeaturemask, firmware, Mesa/RADV packages). Diagnosis only — `amdgpu` is in the kernel, nothing to install. | 30–60 min | None. |
| **Total** | | **~5 minutes now; about 6–9 hours later, almost all of it in P5** | | |

Everything else about AMD — the read path, capability detection, the
"ppfeaturemask missing" explanation — is already written and tested against
synthetic RDNA1–RDNA3 layouts (`crates/ug-gpu/src/amd.rs`).

## Running the capture on another machine

Only the script is needed — no Rust, no build:

```bash
bash capture-fixture.sh friend-rx6700xt
```

It writes `fixture-friend-rx6700xt.tar.gz` in the current directory. It copies
hwmon, DRM/`amdgpu` attributes (including `pp_od_clk_voltage`, `gpu_od/`),
cpufreq/topology/caches, RAPL names, zram, `/proc/{stat,meminfo,cpuinfo,version,
cmdline}`, `os-release`, the route table and the default interface's byte
counters, the GPU driver symlinks, Intel hybrid core lists, and a `FIXTURE.txt`
summary (plus `nvidia-smi` output if present). It does **not** collect serial
numbers, MAC addresses, user files or the process list; filesystem UUIDs in the
kernel command line and the gateway address are zeroed; root-only files are
skipped and listed.

To use it: extract into `crates/ug-testfs/fixtures/<name>/`, add it to the table
in `ug_testfs::fixture`'s docs, and write replay tests like the `replay` modules
in `ug-hw`, `ug-gpu` and `ug-core` (they assert on what the machine really is).
The daemon can also replay it: `ug-daemon --session --sysroot <dir>`.

## Running the binaries elsewhere

The release binaries built on the laptop need **glibc ≥ 2.39**: fine on Ubuntu
24.04+, Mint 22+, Fedora 40+, Debian 13, Arch/CachyOS/Manjaro; **not** on Ubuntu
22.04 or Debian 12 (build from source there: `cargo build --release`).
