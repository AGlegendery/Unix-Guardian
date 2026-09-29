# Unix Guardian

Free and open-source gaming toolkit for Linux: game mode, GPU tuning, fan curves,
system control, live monitoring, ping stability, overlay, recording, per-app network
control, playtime, and a graphics-driver troubleshooter.

Status: **pre-alpha**, building toward v0.1 (see `docs/ROADMAP.md`).

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

Run the daemon on the session bus for development (no root, no install):

```bash
cargo run -p ug-daemon -- --session
cargo run -p ug-cli -- --session status
```

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
