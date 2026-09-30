# 03 — Architecture (as built)

This describes the code that exists at commit `2fbfa46` on branch
`p2-game-mode`. For the full intended architecture see the roadmap
(10-roadmap.fa.md, section «معماری»).

## Processes

```text
 user session (session bus)                         system (system bus, root)
┌──────────────────────────────────────┐          ┌─────────────────────────────────┐
│ ug-agent                             │  Enter/  │ ug-daemon                       │
│  · scans /proc every 2 s → games     │  Leave   │  · sensors, GPU (NVML/sysfs)    │
│  · Steam/Lutris/Heroic/Bottles libs  │ ───────▶ │  · game mode tuner (ug-tune)    │
│  · playtime (SQLite)                 │  polkit  │  · polkit gate, undo state file │
│  · pins game to preferred cores      │          │  · sampling loop (only if       │
│  · pauses apps (SIGSTOP/SIGCONT)     │          │    someone subscribed)          │
│  · notifications                     │          └───────────────▲─────────────────┘
│  · answers com.feralinteractive.     │                          │ Snapshot/Inventory/Gpu
│    GameMode (Steam/Lutris/Heroic)    │                          │
└───────────────▲──────────────────────┘                ┌─────────┴──────────┐
                │ Games / Playtime                      │ ug  (CLI)          │
                └───────────────────────────────────────│ ug-app (Tauri, TBD)│
                                                        └────────────────────┘
```

Rules that hold everywhere:
- The UI and CLI never touch sysfs/procfs directly for anything privileged —
  everything privileged goes through the daemon's D-Bus API.
- **Every feature has a CLI equivalent** (a user whose graphics driver is broken
  has only a TTY — this matters most in P8).
- Only the daemon runs as root. The agent does everything it can unprivileged
  (affinity, pausing its own user's processes, reading launcher files).

## Crates (workspace members)

| Crate | Kind | Lines | What it does |
|---|---|---|---|
| `ug-proto` | lib | ~600 | The D-Bus contract: wire types (`types.rs`), client proxies (`daemon::*`, `agent::*`), bus names, `PROTOCOL_VERSION = 1`, `UgError`. |
| `ug-testfs` | lib | ~300 | Builds fake `/sys` + `/proc` trees for tests (`TestFs`, `FakeProc`, `hwmon()`, `cpu()`, `drm_card()`…), and holds **captured fixtures** of real machines (`fixture("dev-laptop-rtx3050")`). |
| `ug-hw` | lib | ~1600 | Read-only hardware: `hwmon` (chips/sensors, CPU package temp), `cpu` (load from /proc/stat, cpufreq state), `topology` (sockets/cores/SMT, L3 domains, V-Cache, Intel P/E cores, `preferred_cpus`), `rapl` (package watts), `mem` (meminfo, zram), `net` (default route iface + rates). Everything takes a `SysRoot`. |
| `ug-gpu` | lib | ~1600 | GPU discovery from DRM/PCI sysfs; `GpuBackend` trait with `nvidia` (NVML, lazily loaded `NvmlSlot`), `amd` (amdgpu sysfs — **not yet run on real AMD hardware**), `intel` (read-only); names from `pci.ids`; runtime-PM check so a sleeping GPU is not woken. |
| `ug-core` | lib | ~660 | `distro` (os-release → family + support tier + immutable/systemd flags), `paths` (XDG + system dirs, `write_atomic`), `logging` (reloadable filter), `profile` (competitive/balanced/quiet + user TOML overrides + per-game choice). |
| `ug-games` | lib | ~2150 | `db` (bundled `data/games.json`), launcher scanners (`steam` + `vdf` parser, `lutris`, `heroic`, `bottles`), `proc` (process reading), `detect` (the detector state machine). |
| `ug-tune` | lib | ~700 | Reversible game-mode tuning with a write-ahead undo list on disk (`Tuner`), `units` (service manager trait: `NoUnits`, `FakeUnits`). |
| `ug-daemon` | bin | ~1160 | `main` (args, bus setup, polkit gate choice, shutdown revert), `sampler` (one Snapshot from all readers), `iface` (System + Gpu interfaces, subscriber tracking), `gamemode` (GameMode interface, `ServiceManager`), `polkit` (CheckAuthorization, action constants). |
| `ug-agent` | bin | ~1300 | `main` (loop, GameMode compat claim, notifications, daemon reconciliation `DaemonLink`), `core` (`AgentCore`: detection → profiles → playtime/pinning/pausing), `iface` (Games + Playtime), `gamemode` (Feral-compatible interface), `playtime` (SQLite), `affinity`, `apps` (pause/resume), `notify`. |
| `ug-cli` | bin `ug` | ~550 | `status`, `inventory`, `monitor`, `gpu list|caps|active`, `game list|running|rescan|ignore`, `playtime [--today|--week|--forget KEY]`, all with `--json`; `--session` targets a development daemon. |

Planned crates not created yet: `ug-net`, `ug-power`, `ug-overlay`,
`ug-record`, `ug-telegram`, `ug-driver`, `ug-stress`, and the `app/` (Tauri).

## D-Bus API

Bus names and paths: daemon `org.unixguardian.Daemon1` at
`/org/unixguardian/Daemon1` (system bus); agent `org.unixguardian.Agent1` at
`/org/unixguardian/Agent1` (session bus); GameMode compat
`com.feralinteractive.GameMode` at `/com/feralinteractive/GameMode` (session).

Versioning rule: interfaces only gain methods; a breaking change means a new
interface name (`Daemon2`). Clients read `ProtocolVersion`.

**Missing readings travel as sentinels** (D-Bus has no option type): `NaN` for
floats, `0` for counts, `""` for strings. Use the `Option`-returning accessors
in `ug-proto` (`CpuSample::temp()`, `GpuSample::load()`…), never raw fields.

### `org.unixguardian.Daemon1.System`
| Member | Signature | Notes |
|---|---|---|
| `Snapshot()` | → `Snapshot` | One read of everything; ~9 ms. Never fails on missing hardware. |
| `Inventory()` | → `Inventory` | Static description; GPU entries re-read so NVML-learned details appear. |
| `SetLogFilter(filter)` | `s` → | polkit `org.unixguardian.daemon.debug`. e.g. `"ug_gpu=trace,info"`. |
| `Subscribe(interval_ms)` | `u` → | Keyed by caller's unique name; interval clamped 250 ms–1 h; fastest wins. |
| `Unsubscribe()` | → | Also released automatically when the caller leaves the bus. |
| `Sample` (signal) | `Snapshot` | Emitted only while ≥ 1 subscriber; otherwise the loop parks (0 CPU). |
| `ProtocolVersion`, `Version` (props) | `u`, `s` | |

### `org.unixguardian.Daemon1.Gpu`
| Member | Signature | Notes |
|---|---|---|
| `List()` | → `a(GpuInfo)` | All GPUs in PCI order, including ones without a backend. |
| `Capabilities(pci)` | `s` → `GpuCapabilities` | Only call that may wake a sleeping GPU (user-initiated). `notes` explains every `false`. |
| `ActiveGpu` (prop, rw) | `s` | PCI address tuning acts on. Defaults to first discrete GPU (not the boot/integrated one). |

### `org.unixguardian.Daemon1.GameMode`
| Member | Signature | Notes |
|---|---|---|
| `Enter(request)` | `GameModeRequest` → `GameModeReport` | polkit `org.unixguardian.gamemode`. First call applies; later calls add PIDs (priority). Owner = caller's user; another user is refused. |
| `Leave()` | → `GameModeReport` | Restores everything. Same user only. |
| `Active` (prop) | `b` | Read from an atomic, never blocks. |
Also: if the owning connection leaves the bus, the daemon reverts by itself; on
SIGTERM it reverts; on start it reverts anything a crashed run left.

### `org.unixguardian.Agent1.Games` / `.Playtime` / `.Profiles`
| Member | Signature | Notes |
|---|---|---|
| `Installed()` | → `a(InstalledGameInfo)` | From all launchers, last scan (every 10 min or `Rescan`). |
| `Running()` | → `a(RunningGameInfo)` | |
| `Rescan()` | → `u` | Also reloads profiles. |
| `IgnoreExecutable(name)` | `s` → | "Not a game": ends a heuristic detection now and forever. |
| `GameStarted` / `GameStopped` (signals) | `RunningGameInfo` / `(RunningGameInfo, played_s u64)` | |
| `Playtime.Summary(since_unix_ms)` | `t` → `a(PlaytimeEntry)` | 0 = all time; sessions straddling the start count only their later part. |
| `Playtime.Forget(key)` | `s` → `u` | Deletes a game's sessions. |
| `Playtime.Sessions(since_unix_ms)` | `t` → `a(PlaytimeSession)` | Every session ending after the bound, oldest first (for `ug playtime --csv`). |
| `Profiles.List()` | → `a(ProfileInfo)` | Built-ins + user files, each with the game keys the user assigned to it. |
| `Profiles.SetGameProfile(key, profile)` | `ss` → | Writes `games.toml` atomically; `""` clears the choice. Unknown profile → `InvalidArgs` listing the available ones. A running game switches on the next tick. |

### `com.feralinteractive.GameMode` (drop-in)
`RegisterGame(i)→i`, `UnregisterGame(i)→i`, `QueryStatus(i)→i`,
`RegisterGameByPID(i,i)→i`, `UnregisterGameByPID(i,i)→i`,
`QueryStatusByPID(i,i)→i`, `RefreshConfig()→i`, prop `ClientCount i`.
Returns 0 = ok, −1 = error; status 0 inactive / 1 active / 2 active and this
PID registered. If Feral's `gamemoded` already owns the name, the agent steps
aside and logs it.

### Wire types (`ug-proto/src/types.rs`)
- `Snapshot { ts_unix_ms, cpu: CpuSample, gpus: [GpuSample], mem: MemSample, net: NetSample }`
- `CpuSample { load_pct, temp_c, power_w, freq_avg_mhz, freq_max_mhz, per_core_load[], governor, epp, driver }`
- `GpuSample { pci, temp_c, temp_hotspot_c, temp_mem_c, load_pct, power_w, power_limit_w, core_clock_mhz, mem_clock_mhz, vram_used_mb, fan_pct, fan_rpm, asleep }`
- `MemSample { total_kb, available_kb, swap_total_kb, swap_used_kb, zram_used_kb }` (+ `used_kb()`, `used_pct()`)
- `NetSample { iface, rx_bps, tx_bps, rx_total_bytes, tx_total_bytes }`
- `Inventory { cpu: CpuTopology, gpus: [GpuInfo], distro_id, distro_name, kernel, mem_total_kb, privileged }`
- `CpuTopology { model, logical, physical, sockets, smt, l3_domains: [L3Domain{id,size_kb,cpus}], has_vcache, p_cores[], e_cores[] }`
- `GpuInfo { pci, name, vendor: Vendor(nvidia|amd|intel|unknown), device_id, driver, driver_version, vram_total_mb, boot_vga, card_index }`
- `GpuCapabilities { power_limit, clock_offset, voltage, fan_control, fan_curve, power_limit_min_w, power_limit_max_w, core_offset_min_mhz, core_offset_max_mhz, notes[] }`
- `RunningGameInfo { key, name, evidence, profile, pids[], blocked_note, started_unix_ms }`
- `InstalledGameInfo { id, name, launcher }`, `PlaytimeEntry { key, name, seconds, sessions, last_played_unix_ms }`
- `GameModeRequest { cpu_performance, cpu_no_idle_states, cpu_priority, memory_tuning, pause_services, pids[] }`, `GameModeReport { applied[], skipped[] }`

## polkit actions (`packaging/polkit/org.unixguardian.policy`)

| Action | Active session | Other | Used by |
|---|---|---|---|
| `org.unixguardian.daemon.debug` | `auth_admin_keep` | `auth_admin` | `SetLogFilter` |
| `org.unixguardian.gamemode` | **`yes`** | `auth_admin` | `GameMode.Enter` |

Rule (enforced by a test in `ug-daemon/src/polkit.rs`): the policy file defines
exactly the actions the code checks — an action is added in the same change as
the first method that enforces it. On the session bus (development) polkit is
skipped.

## Files on disk

| Path | Owner | Contents |
|---|---|---|
| `/var/lib/unix-guardian/gamemode.json` | daemon | Write-ahead undo list while game mode is on (empty otherwise). |
| `~/.local/share/unix-guardian/agent.db` | agent | Playtime (`sessions(id, key, name, started_ms, ended_ms)`), WAL mode. |
| `~/.config/unix-guardian/profiles/<id>.toml` | user | Overrides/new profiles, any subset of fields; unknown keys rejected. |
| `~/.config/unix-guardian/games.toml` | user | `[games] "steam:730" = "quiet"` — profile per game key. |
| `packaging/systemd/unix-guardiand.service` | package | `Type=dbus`, hardened, `Delegate=yes`, capability bounding set. |
| `packaging/dbus/org.unixguardian.Daemon1.{conf,service}` | package | System-bus policy (root owns, all may call) + activation. |

| `packaging/systemd/ug-agent.service` | package | User unit, `Type=exec`, `PartOf=graphical-session.target`, enabled with `systemctl --global enable`. |
| `packaging/dbus/org.unixguardian.Agent1.service` | package | Session-bus activation through the user unit. |
| `packaging/autostart/ug-agent.desktop` | package | XDG autostart fallback, `X-systemd-skip=true`. |

Only one daemon and one agent ever serve: both claim their bus name with
`DoNotQueue` and no replacement (zbus's defaults would allow a takeover), the
daemon before crash recovery. A second daemon exits with an error; a second
agent exits 0.

## How the key pieces work

**Sampling (daemon).** `Sampler` owns the rate readers (CPU load, RAPL, network)
behind one lock so concurrent callers share deltas. The sampling loop parks on a
`Notify` with zero subscribers. GPU telemetry skips any GPU whose
`/sys/bus/pci/devices/<pci>/power/runtime_status` is `suspended`/`suspending`
and reports `asleep = true` instead. NVML (`NvmlSlot`) loads on first use and a
release task drops it 30 s after the last use (event-driven, no idle polling).

**Detection (agent, `ug-games::detect`).** Every 2 s: list `/proc`; classify
only new `(pid, starttime)`; evidence order: `SteamAppId` env (non-tool app ID)
→ GameMode registration → installed library exe match → games.json exe/cmdline
→ heuristic (Windows exe with D3D/Vulkan/GL mapped, or native with Vulkan, *and*
≥ 25% of a core for 5 s). Excluded: other users, kernel threads, a name list
(browsers, editors, terminals, compositors, creative tools, launcher helpers),
Wine system exes, installers, and **any Chromium/Electron/CEF helper
(`--type=` anywhere in the args, including a rewritten process title)**.
Stop = no processes for 3 s (debounce); played time ends at the last process.
Native programs are named by `/proc/<pid>/exe`; Windows programs by `argv[0]`.

**Game mode (daemon `ug-tune`).** `enter`: governor+EPP per cpufreq policy
(restored governor-first), hold `/dev/cpu_dma_latency` at 0 (released by the
kernel when the fd closes), sysctls `vm.swappiness=10`,
`vm.max_map_count=2147483642` (only raised), `vm.compaction_proactiveness=0`,
stop listed background **timers** (`BACKGROUND_UNITS`), nice −10 on every thread
of the caller's own game PIDs plus autogroup nice −10. Every change: record undo
→ persist atomically → apply; a failed apply withdraws its undo entry and is
reported as skipped. `leave`/crash-recovery pops the list in reverse; entries
that fail to restore are kept for the next attempt.

**Agent ↔ daemon.** `AgentCore::gamemode_desired()` merges the profiles of all
running games (a flag is on if any game wants it; PIDs unioned). `DaemonLink`
compares it with what the daemon last confirmed each tick and calls
`Enter`/`Leave` only on a difference; failures back off 30 s; every ~60 s while
active it re-reads `Active` in case the daemon restarted.
