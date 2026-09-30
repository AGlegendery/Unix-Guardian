# Unix Guardian

Free and open-source gaming toolkit for Linux: game mode, GPU tuning, fan curves,
system control, live monitoring, ping stability, overlay, recording, per-app network
control, playtime, and a graphics-driver troubleshooter.

Status: **pre-alpha.** Monitoring, game detection, playtime and reversible game mode work;
the UI is not started. Plan, status and project knowledge: [`claude-project/`](claude-project/00-README.md).

## Architecture

Three processes, one protocol:

| Process      | Runs as                  | Job                                                        |
|--------------|--------------------------|------------------------------------------------------------|
| `ug-daemon`  | root, systemd system     | sysfs/NVML/nftables/tc/logind — everything needing privilege |
| `ug-agent`   | user, systemd --user     | game detection, playtime, MangoHud, recorder, Telegram       |
| `ug-app`     | user, optional           | Tauri UI — closed during games, zero cost when not open      |

The UI never touches sysfs. Everything goes through D-Bus so the CLI, scripts and the
Telegram bot share one API.

## Building

```bash
cargo build --workspace
cargo test --workspace
```

Run the daemon on the session bus for development (no root, no install, never
touches the real system's services), and the agent against it:

```bash
cargo run -p ug-daemon -- --session
cargo run -p ug-cli -- --session status
cargo run -p ug-agent -- --daemon-session
```

End-to-end checks on a private bus:

```bash
dbus-run-session -- ./scripts/smoke.sh
dbus-run-session -- ./scripts/agent-e2e.sh
dbus-run-session -- ./scripts/gamemode-e2e.sh
```

## Reporting bugs

Email **unixguardian@atomicmail.io** with what you did, what happened, and the
output of `ug status` and `journalctl -u unix-guardiand -b`. The app's Bug
report page, the TUI's help (`?`) and every CLI error show this address too.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
