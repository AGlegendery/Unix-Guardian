# 06 — Development environment

## The user's machine (the only test hardware so far)

| | |
|---|---|
| Machine | Laptop (battery, USB-C PD), hybrid graphics (Optimus) |
| OS | **Parrot Security 7.3 "echo"** (Debian 13 trixie base), kernel `7.0.13+parrot7-amd64`, KDE Plasma (Wayland), shell **zsh** |
| CPU | Intel Core **i5-12450HX** (Alder Lake-HX): 4 P-cores with HT = CPUs **0–7**, 4 E-cores = CPUs **8–11**; `intel_pstate`, governor `powersave`, EPP `balance_power`, 12 cpufreq policies, one 12 MB L3 |
| iGPU | Intel UHD (Alder Lake-HX), `8086:468b`, `i915`, `card0`, PCI `0000:00:02.0`, boot VGA |
| dGPU | **NVIDIA GeForce RTX 3050 6GB Laptop GPU** (GA107), `10de:25ec`, `card1`, PCI `0000:01:00.0`, proprietary driver **550.163.01**, `nvidia-drm.modeset=1`; runtime PM `auto`, usually **suspended** |
| RAM | 27 GB usable |
| Sensors (hwmon) | `acpitz`, `BAT1`, `nvme`, `ucsi_source_psy_USBC000:001`, `ACAD`, `spd5118`, `coretemp` (package + per-core) |
| Network | Wi-Fi `wlp8s0` carries the default route |
| Kernel settings | cgroup v2; autogroup **on**; THP `always`; RAPL readable by root only |
| Games installed | **Bottles (Flatpak)** with one game ("Fears to Fathom – Ironbark Lookout" in bottle dir `RedDead__149`). **No** Steam, Lutris, Heroic, GameMode, MangoHud or gamescope |
| User groups | `sudo`, `docker`, `podman`, `video`, `input`, … |
| No AMD hardware | A friend has an AMD machine, available later (see 08-hardware-plan.md) |

## Toolchain on that machine

- `rustc`/`cargo` **1.94.1**, Debian source build, **no rustup** →
  **`rustfmt` and `clippy` are not installed**. apt offers 1.85 versions, which
  do not match the 1.94 compiler (clippy would not work). CI runs them instead.
- `node` 26.8.1, `npm` 11.19 works, **`pnpm` is broken** (ESM error).
- **Missing for the UI:** `libwebkit2gtk-4.1-dev`, `libsoup-3.0-dev` (Tauri).
- Available: `git`, `jq`, `python3` (pip blocked by PEP 668 — use a venv),
  `xmllint`, `busctl`, `dbus-run-session`, `systemd-analyze`, `journalctl`.
  `sysctl` is not on the user's PATH (`/usr/sbin`).
- Network access works (crates.io, Steam store API).

## Repository

`/home/aglegend/SSDBkup/IMPORTANT/ProgramingTable/unix-guardian` — git, branches
`master` and `p2-game-mode`, no remote. A git identity is set for this
repository only (`git config user.name` / `user.email`).

```text
unix-guardian/
├─ Cargo.toml  Cargo.lock  rust-toolchain.toml  rustfmt.toml  clippy.toml  deny.toml
├─ crates/     ug-proto ug-testfs ug-hw ug-gpu ug-core ug-games ug-tune ug-daemon ug-agent ug-cli
├─ data/games.json
├─ packaging/  systemd/ dbus/ polkit/
├─ scripts/    capture-fixture.sh dev-install.sh smoke.sh verify-games-db.py
│              agent-e2e.sh gamemode-e2e.sh make-knowledge-pack.sh
├─ docs/ROADMAP.md
├─ claude-project/   ← this knowledge pack
├─ CLAUDE.md         ← short brief for Claude Code sessions
└─ .github/workflows/ci.yml
```

## Everyday commands

```bash
cd /home/aglegend/SSDBkup/IMPORTANT/ProgramingTable/unix-guardian
cargo build --workspace
cargo test --workspace
```

Development daemon — no root, **never touches real services**, polkit skipped:

```bash
cargo run -p ug-daemon -- --session
cargo run -p ug-cli -- --session status
cargo run -p ug-agent -- --daemon-session
```

Isolated checks on a private D-Bus bus (nothing on the desktop session is used):

```bash
dbus-run-session -- ./scripts/smoke.sh
dbus-run-session -- ./scripts/agent-e2e.sh
dbus-run-session -- ./scripts/gamemode-e2e.sh
```

Tools:

```bash
cargo run --release -p ug-games --example scan
cargo run --release -p ug-games --example detect 12
cargo run --release -p ug-gpu --example nvml_lifecycle
./scripts/capture-fixture.sh my-machine
./scripts/verify-games-db.py
cargo run -p ug-daemon -- --session --sysroot crates/ug-testfs/fixtures/dev-laptop-rtx3050
```

Real install as a system service (**untested; needs sudo; changes the real system
while a game runs**):

```bash
cargo build --release && sudo ./scripts/dev-install.sh
```

Fake a game without installing one — Steam's variable is all detection needs:

```bash
SteamAppId=730 bash -c 'exec -a cs2 sleep 120'
```

Register a process the way Lutris/Heroic do:

```bash
busctl --user call com.feralinteractive.GameMode /com/feralinteractive/GameMode com.feralinteractive.GameMode RegisterGame i 12345
```

## Pitfalls met on this machine

- **zsh** is the shell: an unquoted `$var` does *not* word-split (use arrays or
  `${=var}`), and pipeline statuses are `$pipestatus`, not `PIPESTATUS`.
- `pkill -f pattern` also kills the shell whose command line contains the
  pattern. Kill by PID, or `pgrep -x ug-daemon`.
- A process started with `&` inside a one-off shell may lose its D-Bus
  connection when that shell ends. Run multi-process tests inside one
  `dbus-run-session -- script.sh`.
- Poll with a deadline and `sleep 0.2`; `read -t 0.2 < /dev/zero` returns
  instantly and does not wait.
- Tests that run the agent must set `XDG_DATA_HOME`/`XDG_CONFIG_HOME` to a
  scratch directory, or they write into the user's real playtime database.
- After anything that could touch privileged paths, check the journal:
  `journalctl --since -10min | grep -E 'polkit|Stopped|Started'`.
- `set -e` scripts: a function whose last command is `cond && cmd` returns
  failure when `cond` is false and kills the script.
- When Claude Code was used: its scratch directory under `/tmp` is wiped
  between days; anything worth keeping belongs in the repo.
