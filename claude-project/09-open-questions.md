# 09 — Open questions for the user

None of these blocks current work; each has an assumption already in place.
Ask when the relevant phase comes up (or earlier if convenient).

| # | Needed by | Question | Current assumption |
|---|---|---|---|
| 1 | now | **Resource budget:** accept *daemon ≤ 8 MB idle, ≤ 25 MB while reading an NVIDIA GPU* instead of the roadmap's flat 15 MB? (NVML alone is ~15 MB; it is released when idle.) | Yes, pending OK. |
| 2 | now | **Merge `p2-game-mode` into `master`?** | Not merged until the user reviews. |
| 3 | now | **First real install** with `sudo ./scripts/dev-install.sh` (really changes governor/sysctls/timers during a game, reverted after). When? | Waiting for the user. |
| 4 | P2 UI | **Preferred-core pinning default.** Off in all built-in profiles. It usually helps on AMD X3D; on Intel hybrid it can hurt heavily-threaded games (fewer threads). Turn it on for "competitive" on X3D only? | Off everywhere; user can enable per profile. |
| 5 | P2 UI | **Apps to pause by default** while gaming (Discord, browsers, …)? | None by default; the user picks. Pausing (SIGSTOP) rather than closing is the default behaviour, as the roadmap assumed. |
| 6 | P2 | **Should persistent ("always") optimisations exist at all?** The roadmap had them; they are not implemented, because a persistent change that is never reverted is the riskiest thing this app could do. | Temporary only. |
| 7 | P2 | How to present **games whose anti-cheat blocks Linux** (League of Legends and VALORANT — both in the Windows screenshots): hide, show with a notice, or warn at detection? | The detector attaches `blocked_note`; the agent's notification shows it. UI undecided. |
| 8 | P2 | **More games in `games.json`** (roadmap target 80; now 64 verified). Which titles matter most to the user's audience? | Grow by verified PRs. |
| 9 | P3+ | **GitHub organisation / repository name** and when to go public. `Cargo.toml` has a placeholder `github.com/unix-guardian/unix-guardian`. | Private, local only. |
| 10 | P7 | **Telegram bot per user** (each user creates their own with @BotFather — no server, no cost) or one central bot (needs a server and privacy responsibility)? | Per-user bot; SOCKS5/HTTP proxy support from day one. |
| 11 | P8 | A **machine that may be broken** for testing the driver installer (VM with GPU passthrough, or a spare disk/partition)? Testing installs on the main laptop is risky even with snapshots. | Logic tested in VMs; real installs only with snapshots on the user's machine. |
| 12 | P10 | **Visual identity**: logo, colours, domain. The user said the Linux UI need not copy the Windows screenshots; does the Windows app have a brand to continue, or is Unix Guardian independent? | Independent identity. |
| 13 | P10 | **Anonymous usage statistics**? | No telemetry; voluntary hardware reports via issues only. |
