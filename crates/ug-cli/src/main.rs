//! `ug` — the command-line client.
//!
//! Every feature the UI offers is reachable here, and that is a project rule
//! rather than a convenience. The driver troubleshooter can leave a machine
//! without working graphics; the person who then has to undo it is sitting at a
//! TTY with no UI to open. A feature that exists only in the GUI is a feature
//! that disappears exactly when it is most needed.
//!
//! Every command takes `--json` so the output can be piped somewhere useful.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use ug_proto::agent::{GamesProxy, PlaytimeProxy, ProfilesProxy};
use ug_proto::daemon::{DaemonProxy, GpuProxy, PowerProxy};
use ug_proto::{GpuCapabilities, Inventory, PowerWatch, Snapshot};

#[derive(Parser)]
#[command(name = "ug", version, about = "Unix Guardian command-line client")]
struct Cli {
    /// Talk to a daemon on the session bus, as started by `ug-daemon --session`.
    #[arg(long, global = true)]
    session: bool,

    /// Emit JSON instead of formatted text.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Machine summary and current readings.
    Status,
    /// Static hardware inventory.
    Inventory,
    /// Live readings, refreshed until interrupted.
    Monitor {
        /// Milliseconds between samples.
        #[arg(long, default_value_t = 1000)]
        interval: u32,
    },
    /// Shut down, restart or sleep, now or on a schedule.
    Power {
        #[command(subcommand)]
        command: PowerCommand,
    },
    /// GPU commands.
    Gpu {
        #[command(subcommand)]
        command: GpuCommand,
    },
    /// Installed and running games.
    Game {
        #[command(subcommand)]
        command: GameCommand,
    },
    /// Time spent in each game.
    Playtime {
        /// Only today (since local midnight).
        #[arg(long, conflicts_with_all = ["week"])]
        today: bool,
        /// Only the last seven days.
        #[arg(long)]
        week: bool,
        /// Delete all recorded sessions of one game, by key (e.g. `steam:730`).
        #[arg(long, value_name = "KEY")]
        forget: Option<String>,
        /// Print every session as CSV (for a spreadsheet) instead of a summary.
        #[arg(long, conflicts_with = "forget")]
        csv: bool,
    },
    /// Game profiles, and which game uses which.
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
}

#[derive(Subcommand)]
enum ProfileCommand {
    /// Every profile, what it changes, and the games assigned to it.
    List,
    /// Run a game with a profile, e.g. `ug profile set steam:730 quiet`.
    /// Game keys are shown by `ug game list` and `ug game running`.
    Set { game: String, profile: String },
    /// Return a game to the profile the game database suggests.
    Reset { game: String },
    /// Turn one setting of a profile on or off, e.g.
    /// `ug profile option quiet cpu_priority off`.
    Option {
        profile: String,
        /// cpu_performance, cpu_no_idle_states, cpu_preferred_cores,
        /// cpu_priority, memory_tuning or pause_services.
        setting: String,
        #[arg(value_parser = ["on", "off"])]
        value: String,
    },
}

#[derive(Subcommand)]
enum GameCommand {
    /// Games installed through Steam, Lutris, Heroic or Bottles.
    List,
    /// Games running now, and the profile each runs with.
    Running,
    /// Re-read every launcher's library.
    Rescan,
    /// Stop treating an executable as a game, e.g. `ug game ignore Render.exe`.
    Ignore { executable: String },
}

#[derive(Subcommand)]
enum PowerCommand {
    /// Which actions this machine supports, and why the others are not.
    Caps,
    /// Do it now: power-off, reboot, suspend, hibernate, hybrid-sleep or
    /// suspend-then-hibernate. Asks for --yes first.
    Now {
        action: String,
        /// Confirm: without it nothing happens.
        #[arg(long)]
        yes: bool,
    },
    /// Schedule it, e.g. `ug power at reboot --in 2h` or
    /// `ug power at power-off --at 01:00 --repeat daily`.
    At {
        action: String,
        /// After this long: 90s, 30m, 2h, 1h30m.
        #[arg(long, conflicts_with = "at", required_unless_present = "at")]
        r#in: Option<String>,
        /// At this local time (HH:MM), today or else tomorrow.
        #[arg(long)]
        at: Option<String>,
        #[arg(long, default_value = "once", value_parser = ["once", "daily", "weekly"])]
        repeat: String,
        #[arg(long, default_value = "")]
        note: String,
    },
    /// Wake the computer at a time (from sleep, or from off where the
    /// firmware allows): `ug power wake --at 07:00`, `--in 8h`, or `--clear`.
    Wake {
        #[arg(long, conflicts_with_all = ["at", "clear"])]
        r#in: Option<String>,
        #[arg(long, conflicts_with = "clear")]
        at: Option<String>,
        #[arg(long)]
        clear: bool,
    },
    /// Do it once a condition holds: `ug power when power-off --download-done`,
    /// `--cpu-idle`, `--process blender`, or `--game-over`.
    When {
        action: String,
        /// Download finished: receive rate stays below --below (default 50k B/s).
        #[arg(long, group = "cond")]
        download_done: bool,
        /// CPU load stays below --below (default 10 %).
        #[arg(long, group = "cond")]
        cpu_idle: bool,
        /// This program (process name) has exited.
        #[arg(long, group = "cond", value_name = "NAME")]
        process: Option<String>,
        /// The game running now has ended.
        #[arg(long, group = "cond")]
        game_over: bool,
        /// Threshold for --download-done (bytes/s, k/M allowed) or --cpu-idle (%).
        #[arg(long)]
        below: Option<String>,
        /// How long it must hold, for --download-done and --cpu-idle.
        #[arg(long, default_value_t = 5)]
        minutes: u32,
    },
    /// Remove a conditional action by its number (shown by `ug power list`).
    Unwatch { id: u32 },
    /// The scheduled and conditional actions, and the wake alarm.
    List,
    /// Remove a scheduled action by its number.
    Cancel { id: u32 },
}

#[derive(Subcommand)]
enum GpuCommand {
    /// List every GPU in the machine.
    List,
    /// Show what a GPU allows, and why anything unavailable is unavailable.
    Caps {
        /// PCI address; defaults to the active GPU.
        pci: Option<String>,
    },
    /// Show or change which GPU tuning acts on.
    Active {
        /// PCI address to switch to; omit to print the current one.
        pci: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // The agent always lives on the session bus, so game and playtime commands
    // work whether or not the daemon is reachable.
    match cli.command {
        Command::Game { command } => return game(command, cli.json).await,
        Command::Playtime { today, week, forget, csv } => return playtime(today, week, forget, csv, cli.json).await,
        Command::Profile { command } => return profile(command, cli.json).await,
        _ => {}
    }

    let conn = if cli.session {
        zbus::Connection::session().await
    } else {
        zbus::Connection::system().await
    }
    .context("connecting to D-Bus")?;

    let daemon = DaemonProxy::new(&conn).await.with_context(|| {
        format!(
            "reaching the daemon on the {} bus. Is unix-guardiand running? \
             For a development daemon started with --session, pass --session here too.",
            if cli.session { "session" } else { "system" }
        )
    })?;

    match cli.command {
        Command::Status => status(&daemon, cli.json).await,
        Command::Inventory => {
            let inv = daemon.inventory().await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&inv)?);
            } else {
                print_inventory(&inv);
            }
            Ok(())
        }
        Command::Monitor { interval } => monitor(&daemon, interval, cli.json).await,
        Command::Gpu { command } => gpu(&conn, &daemon, command, cli.json).await,
        Command::Power { command } => power(&conn, command, cli.json).await,
        Command::Game { .. } | Command::Playtime { .. } | Command::Profile { .. } => unreachable!("handled above"),
    }
}

async fn agent_conn() -> Result<zbus::Connection> {
    zbus::Connection::session().await.context("connecting to the session bus")
}

fn agent_hint(e: zbus::Error) -> anyhow::Error {
    anyhow::anyhow!("{e}\nIs ug-agent running in this session? It starts with your desktop login.")
}

async fn game(command: GameCommand, json: bool) -> Result<()> {
    let conn = agent_conn().await?;
    let games = GamesProxy::new(&conn).await?;
    match command {
        GameCommand::List => {
            let mut list = games.installed().await.map_err(agent_hint)?;
            list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else if list.is_empty() {
                println!("No installed games found in Steam, Lutris, Heroic or Bottles.");
            } else {
                let width = list.iter().map(|g| g.id.len()).max().unwrap_or(0);
                for g in &list {
                    println!("{:<8} {:<width$}  {}", g.launcher, g.id, g.name);
                }
                println!("\n{} games", list.len());
            }
        }
        GameCommand::Running => {
            let list = games.running().await.map_err(agent_hint)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else if list.is_empty() {
                println!("No game is running.");
            } else {
                let now = now_ms();
                for g in &list {
                    let mins = now.saturating_sub(g.started_unix_ms) / 60_000;
                    println!("{}  [{} profile]  {} min  ({})", g.name, g.profile, mins, g.key);
                    println!("    recognised by: {}   processes: {:?}", g.evidence.replace('_', " "), g.pids);
                    if !g.blocked_note.is_empty() {
                        println!("    note: {}", g.blocked_note);
                    }
                }
            }
        }
        GameCommand::Rescan => {
            let n = games.rescan().await.map_err(agent_hint)?;
            println!("{n} installed games found.");
        }
        GameCommand::Ignore { executable } => {
            games.ignore_executable(&executable).await.map_err(agent_hint)?;
            println!("{executable} will no longer be treated as a game.");
        }
    }
    Ok(())
}

async fn playtime(today: bool, week: bool, forget: Option<String>, csv: bool, json: bool) -> Result<()> {
    let conn = agent_conn().await?;
    let pt = PlaytimeProxy::new(&conn).await?;
    if let Some(key) = forget {
        let n = pt.forget(&key).await.map_err(agent_hint)?;
        println!("Deleted {n} sessions of {key}.");
        return Ok(());
    }
    let since = if today {
        local_midnight_ms()
    } else if week {
        now_ms().saturating_sub(7 * 24 * 3_600_000)
    } else {
        0
    };
    if csv {
        let sessions = pt.sessions(since).await.map_err(agent_hint)?;
        // Written by hand rather than with println!, so `| head` ends the
        // export cleanly instead of panicking on the closed pipe.
        use std::io::Write;
        let mut out = std::io::BufWriter::new(std::io::stdout().lock());
        let written = writeln!(out, "game,key,started,ended,minutes").and_then(|()| {
            for s in &sessions {
                let minutes = s.ended_unix_ms.saturating_sub(s.started_unix_ms) as f64 / 60_000.0;
                writeln!(
                    out,
                    "{},{},{},{},{minutes:.1}",
                    csv_field(&s.name),
                    csv_field(&s.key),
                    local_time(s.started_unix_ms),
                    local_time(s.ended_unix_ms)
                )?;
            }
            out.flush()
        });
        return match written {
            Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => Err(e.into()),
            _ => Ok(()),
        };
    }
    let entries = pt.summary(since).await.map_err(agent_hint)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }
    if entries.is_empty() {
        println!("No playtime recorded{}.", if today { " today" } else if week { " this week" } else { "" });
        return Ok(());
    }
    let total: u64 = entries.iter().map(|e| e.seconds).sum();
    for e in &entries {
        let pct = if total > 0 { e.seconds * 100 / total } else { 0 };
        println!("{:>8}  {:>3}%  {}", hms(e.seconds), pct, e.name);
    }
    println!("{:>8}        total, {} games", hms(total), entries.len());
    Ok(())
}

async fn profile(command: ProfileCommand, json: bool) -> Result<()> {
    let conn = agent_conn().await?;
    let profiles = ProfilesProxy::new(&conn).await?;
    match command {
        ProfileCommand::List => {
            let list = profiles.list().await.map_err(agent_hint)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
                return Ok(());
            }
            for p in &list {
                println!("{}{}", p.id, if p.builtin { "" } else { "  (yours)" });
                let on = |b: bool| if b { "on " } else { "off" };
                println!("    CPU performance mode   {}", on(p.cpu_performance));
                println!("    no CPU idle states     {}", on(p.cpu_no_idle_states));
                println!("    preferred cores        {}", on(p.cpu_preferred_cores));
                println!("    game priority          {}", on(p.cpu_priority));
                println!("    memory tuning          {}", on(p.memory_tuning));
                println!("    pause update services  {}", on(p.pause_services));
                if !p.pause_apps.is_empty() {
                    println!("    pause apps             {}", p.pause_apps.join(", "));
                }
                if !p.games.is_empty() {
                    println!("    chosen for             {}", p.games.join(", "));
                }
            }
            println!("\nChange one with a file in ~/.config/unix-guardian/profiles/<name>.toml");
        }
        ProfileCommand::Set { game, profile } => {
            profiles.set_game_profile(&game, &profile).await.map_err(agent_error)?;
            println!("{game} now runs with the {profile} profile.");
        }
        ProfileCommand::Option { profile, setting, value } => {
            profiles.set_option(&profile, &setting, value == "on").await.map_err(agent_error)?;
            println!("{profile}: {setting} is now {value}.");
        }
        ProfileCommand::Reset { game } => {
            profiles.set_game_profile(&game, "").await.map_err(agent_error)?;
            println!("{game} is back on the profile the game database suggests.");
        }
    }
    Ok(())
}

/// A refusal from the agent is the message itself; anything else may mean the
/// agent is not running.
fn agent_error(e: zbus::Error) -> anyhow::Error {
    match e {
        zbus::Error::MethodError(name, Some(msg), _) if name.as_str() == "org.freedesktop.DBus.Error.InvalidArgs" => {
            anyhow::anyhow!(msg)
        }
        other => agent_hint(other),
    }
}

/// Quotes a CSV field when it needs it (RFC 4180): game names do contain
/// commas ("Warhammer 40,000") and quotes.
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// `2026-09-30 21:04:05` in local time, which spreadsheets read as a date.
fn local_time(unix_ms: u64) -> String {
    // SAFETY: localtime_r writes only into the struct we pass.
    let tm = unsafe {
        let t = (unix_ms / 1000) as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Start of today in local time, as Unix milliseconds.
fn local_midnight_ms() -> u64 {
    // SAFETY: localtime_r writes only into the struct we pass.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm.tm_hour = 0;
        tm.tm_min = 0;
        tm.tm_sec = 0;
        (libc::mktime(&mut tm).max(0) as u64) * 1000
    }
}

/// `3:07` for minutes, `12:05:00` once it reaches hours.
fn hms(s: u64) -> String {
    let (h, m) = (s / 3600, (s % 3600) / 60);
    if h > 0 {
        format!("{h}:{m:02}:{:02}", s % 60)
    } else {
        format!("{m}:{:02}", s % 60)
    }
}

async fn status(daemon: &DaemonProxy<'_>, json: bool) -> Result<()> {
    let inv = daemon.inventory().await?;
    // Two reads so the rate-based figures (load, package power, throughput)
    // have a baseline. One read would always report zero load.
    daemon.snapshot().await?;
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let snap = daemon.snapshot().await?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "inventory": inv,
                "snapshot": snap,
            }))?
        );
        return Ok(());
    }

    print_inventory(&inv);
    println!();
    print_snapshot(&snap, &inv);
    Ok(())
}

async fn monitor(daemon: &DaemonProxy<'_>, interval: u32, json: bool) -> Result<()> {
    use futures_lite::StreamExt;
    let inv = daemon.inventory().await?;

    // Listen before subscribing. The daemon emits its first sample the moment
    // the subscription lands, and a match rule added afterwards would miss it.
    let mut samples = daemon.receive_sample().await?;
    daemon.subscribe(interval).await?;

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let result: Result<()> = async {
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => return Ok(()),
                _ = term.recv() => return Ok(()),
                next = samples.next() => {
                    let Some(signal) = next else { return Ok(()) };
                    let snap: Snapshot = signal.args()?.snapshot;
                    if json {
                        // `ug monitor --json | head` closes the pipe on purpose.
                        // That ends the stream cleanly — and still reaches the
                        // unsubscribe below — rather than panicking in println!.
                        use std::io::Write;
                        let line = serde_json::to_string(&snap)?;
                        let mut out = std::io::stdout().lock();
                        match writeln!(out, "{line}").and_then(|_| out.flush()) {
                            Ok(()) => {}
                            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return Ok(()),
                            Err(e) => return Err(e.into()),
                        }
                    } else {
                        print_snapshot(&snap, &inv);
                        println!();
                    }
                }
            }
        }
    }
    .await;

    // Best effort: the daemon also releases us when we leave the bus, so a
    // SIGKILL that skips this still cannot leave the sampling loop pinned on.
    let _ = daemon.unsubscribe().await;
    result
}

async fn power(conn: &zbus::Connection, command: PowerCommand, json: bool) -> Result<()> {
    let p = PowerProxy::new(conn).await?;
    match command {
        PowerCommand::Caps => {
            let caps = p.capabilities().await.map_err(daemon_error)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&caps)?);
            } else {
                for c in caps {
                    if c.available {
                        println!("{:<24} available", c.action);
                    } else {
                        println!("{:<24} not available: {}", c.action, c.reason);
                    }
                }
            }
        }
        PowerCommand::Now { action, yes } => {
            if !yes {
                anyhow::bail!("this will {action} the computer now; run again with --yes to confirm");
            }
            p.now(&action).await.map_err(daemon_error)?;
            println!("{action}: requested.");
        }
        PowerCommand::At { action, r#in, at, repeat, note } => {
            let when = match (r#in, at) {
                (Some(d), _) => now_ms() + parse_duration(&d)?,
                (None, Some(t)) => next_local_time_ms(&t)?,
                (None, None) => unreachable!("clap requires one"),
            };
            let job = p.schedule(&action, when, &repeat, &note).await.map_err(daemon_error)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&job)?);
            } else {
                println!("Scheduled #{}: {} at {} ({}).", job.id, job.action, local_time(job.at_unix_ms), job.repeat);
            }
        }
        PowerCommand::Wake { r#in, at, clear } => {
            if clear {
                p.clear_wake_alarm().await.map_err(daemon_error)?;
                println!("Wake alarm cleared.");
            } else {
                let when = match (r#in, at) {
                    (Some(d), _) => now_ms() + parse_duration(&d)?,
                    (None, Some(t)) => next_local_time_ms(&t)?,
                    (None, None) => anyhow::bail!("say when: --at HH:MM or --in 8h (or --clear)"),
                };
                p.set_wake_alarm(when).await.map_err(daemon_error)?;
                println!("The computer will wake at {}.", local_time(when));
            }
        }
        PowerCommand::When { action, download_done, cpu_idle, process, game_over, below, minutes } => {
            let (kind, threshold, name) = if download_done {
                ("net-idle", parse_rate(below.as_deref().unwrap_or("50k"))?, String::new())
            } else if cpu_idle {
                let pct: f64 = below.as_deref().unwrap_or("10").trim_end_matches('%').parse()
                    .map_err(|_| anyhow::anyhow!("--below for --cpu-idle is a percentage, e.g. 10"))?;
                ("cpu-idle", pct, String::new())
            } else if let Some(p) = process {
                ("process-gone", 0.0, p)
            } else if game_over {
                ("game-over", 0.0, String::new())
            } else {
                anyhow::bail!("say when: --download-done, --cpu-idle, --process NAME or --game-over");
            };
            let w = p.when(&action, kind, threshold, minutes, &name, "").await.map_err(daemon_error)?;
            println!("Armed #w{}: {} {}.", w.id, w.action, describe_watch(&w));
        }
        PowerCommand::Unwatch { id } => {
            if !p.cancel_watch(id).await.map_err(daemon_error)? {
                anyhow::bail!("no conditional action with number {id} (see `ug power list`)");
            }
            println!("Removed #w{id}.");
        }
        PowerCommand::List => {
            if !json {
                for w in p.watches().await.map_err(daemon_error)? {
                    println!("#w{:<2} {:<24} {}", w.id, w.action, describe_watch(&w));
                }
            }
            let alarm = p.wake_alarm().await.unwrap_or(0);
            if !json && alarm > 0 {
                println!("Wake alarm: {}", local_time(alarm));
            }
            let jobs = p.jobs().await.map_err(daemon_error)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&jobs)?);
            } else if jobs.is_empty() {
                println!("Nothing scheduled.");
            } else {
                for j in jobs {
                    let note = if j.note.is_empty() { String::new() } else { format!("  {}", j.note) };
                    println!("#{:<3} {:<24} {}  {}{note}", j.id, j.action, local_time(j.at_unix_ms), j.repeat);
                }
            }
        }
        PowerCommand::Cancel { id } => {
            if p.cancel(id).await.map_err(daemon_error)? {
                println!("Cancelled #{id}.");
            } else {
                anyhow::bail!("nothing scheduled with number {id} (see `ug power list`)");
            }
        }
    }
    Ok(())
}

fn describe_watch(w: &PowerWatch) -> String {
    match w.kind.as_str() {
        "net-idle" => format!("when downloads stay below {} B/s for {} min", w.threshold as u64, w.minutes),
        "cpu-idle" => format!("when CPU load stays below {} % for {} min", w.threshold, w.minutes),
        "process-gone" => format!("when {} exits", w.name),
        "game-over" => "when the running game ends".into(),
        other => other.into(),
    }
}

/// `50k`, `2M`, `1000` → bytes per second.
fn parse_rate(s: &str) -> Result<f64> {
    let s = s.trim();
    let (num, mult) = match s.chars().last() {
        Some('k' | 'K') => (&s[..s.len() - 1], 1e3),
        Some('m' | 'M') => (&s[..s.len() - 1], 1e6),
        _ => (s, 1.0),
    };
    num.parse::<f64>().map(|n| n * mult).map_err(|_| anyhow::anyhow!("cannot read {s:?} as a rate; e.g. 50k or 2M"))
}

/// A refusal from the daemon is already a sentence for people.
fn daemon_error(e: zbus::Error) -> anyhow::Error {
    match e {
        zbus::Error::MethodError(_, Some(msg), _) => anyhow::anyhow!(msg),
        other => anyhow::anyhow!(other),
    }
}

/// `90s`, `30m`, `2h`, `1h30m`, `1d` → milliseconds.
fn parse_duration(s: &str) -> Result<u64> {
    let mut total = 0u64;
    let mut num = String::new();
    for c in s.trim().chars() {
        if c.is_ascii_digit() {
            num.push(c);
            continue;
        }
        let n: u64 = num.parse().map_err(|_| anyhow::anyhow!("cannot read {s:?} as a duration; try 90s, 30m, 2h or 1h30m"))?;
        num.clear();
        total += n * match c {
            's' => 1_000,
            'm' => 60_000,
            'h' => 3_600_000,
            'd' => 86_400_000,
            _ => anyhow::bail!("cannot read {s:?} as a duration; try 90s, 30m, 2h or 1h30m"),
        };
    }
    if !num.is_empty() || total == 0 {
        anyhow::bail!("cannot read {s:?} as a duration; try 90s, 30m, 2h or 1h30m");
    }
    Ok(total)
}

/// The next local wall-clock `HH:MM`: today if still ahead, else tomorrow.
fn next_local_time_ms(hhmm: &str) -> Result<u64> {
    let bad = || anyhow::anyhow!("cannot read {hhmm:?} as a time; use HH:MM, e.g. 01:30");
    let (h, m) = hhmm.split_once(':').ok_or_else(bad)?;
    let (h, m): (i32, i32) = (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return Err(bad());
    }
    // SAFETY: localtime_r/mktime only touch the struct we pass.
    let at = unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm.tm_hour = h;
        tm.tm_min = m;
        tm.tm_sec = 0;
        let mut t = libc::mktime(&mut tm);
        if t <= now {
            tm.tm_mday += 1;
            t = libc::mktime(&mut tm);
        }
        t.max(0) as u64
    };
    Ok(at * 1000)
}

async fn gpu(
    conn: &zbus::Connection,
    daemon: &DaemonProxy<'_>,
    command: GpuCommand,
    json: bool,
) -> Result<()> {
    let gpu = GpuProxy::new(conn).await?;
    match command {
        GpuCommand::List => {
            let gpus = daemon.inventory().await?.gpus;
            if json {
                println!("{}", serde_json::to_string_pretty(&gpus)?);
                return Ok(());
            }
            let active = gpu.active_gpu().await.unwrap_or_default();
            for g in &gpus {
                let marker = if g.pci == active { "*" } else { " " };
                let role = if g.boot_vga { "integrated" } else { "discrete" };
                println!("{marker} {}  {}", g.pci, g.name);
                print!("    {} · {role}", g.vendor);
                if !g.driver.is_empty() {
                    print!(" · driver {}", g.driver);
                }
                if !g.driver_version.is_empty() {
                    print!(" {}", g.driver_version);
                }
                if g.vram_total_mb > 0 {
                    print!(" · {} MB VRAM", g.vram_total_mb);
                }
                println!();
            }
            if !gpus.is_empty() {
                println!("\n* is the GPU tuning acts on. Change it with `ug gpu active <pci>`.");
            }
            Ok(())
        }
        GpuCommand::Caps { pci } => {
            let pci = match pci {
                Some(p) => p,
                None => gpu.active_gpu().await?,
            };
            let caps = gpu.capabilities(&pci).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&caps)?);
            } else {
                print_caps(&pci, &caps);
            }
            Ok(())
        }
        GpuCommand::Active { pci } => {
            match pci {
                Some(p) => {
                    gpu.set_active_gpu(&p).await?;
                    println!("Tuning now acts on {p}.");
                }
                None => println!("{}", gpu.active_gpu().await?),
            }
            Ok(())
        }
    }
}

fn print_inventory(inv: &Inventory) {
    println!("{}", inv.distro_name);
    println!("  kernel    {}", inv.kernel);
    println!(
        "  cpu       {} ({} cores / {} threads{})",
        inv.cpu.model,
        inv.cpu.physical,
        inv.cpu.logical,
        if inv.cpu.smt { ", SMT on" } else { "" }
    );
    if inv.cpu.l3_domains.len() > 1 {
        println!(
            "            {} L3 domains{}",
            inv.cpu.l3_domains.len(),
            if inv.cpu.has_vcache {
                " (3D V-Cache present)"
            } else {
                ""
            }
        );
    }
    println!("  memory    {} GB", inv.mem_total_kb / 1024 / 1024);
    for g in &inv.gpus {
        println!(
            "  gpu       {} [{}]{}",
            g.name,
            g.pci,
            if g.boot_vga { " (integrated)" } else { "" }
        );
    }
    if !inv.privileged {
        println!("\n  read-only: the daemon is not running privileged, so tuning is unavailable.");
    }
}

fn print_snapshot(s: &Snapshot, inv: &Inventory) {
    print!("cpu   {:>5.1}%", s.cpu.load_pct);
    if let Some(t) = s.cpu.temp() {
        print!("  {t:>5.1}°C");
    }
    if let Some(w) = s.cpu.power() {
        print!("  {w:>5.1} W");
    }
    if s.cpu.freq_avg_mhz > 0 {
        print!("  {} MHz avg / {} max", s.cpu.freq_avg_mhz, s.cpu.freq_max_mhz);
    }
    if !s.cpu.governor.is_empty() {
        print!("  [{}]", s.cpu.governor);
    }
    println!();

    for g in &s.gpus {
        let name = inv
            .gpus
            .iter()
            .find(|i| i.pci == g.pci)
            .map(|i| i.name.as_str())
            .unwrap_or(g.pci.as_str());
        print!("gpu   ");
        if g.asleep {
            println!(" sleep  (powered down to save energy)  {name}");
            continue;
        }
        match g.load() {
            Some(l) => print!("{l:>5.1}%"),
            None => print!("    — "),
        }
        if let Some(t) = g.temp() {
            print!("  {t:>5.1}°C");
        }
        if let Some(h) = g.hotspot() {
            print!("  hotspot {h:.0}°C");
        }
        if let Some(w) = g.power() {
            print!("  {w:>5.1} W");
        }
        if g.core_clock_mhz > 0 {
            print!("  {} MHz", g.core_clock_mhz);
        }
        if let Some(f) = g.fan() {
            print!("  fan {f:.0}%");
        }
        println!("  {name}");
    }

    println!(
        "mem   {:>5.1}%  {:.1} / {:.1} GB",
        s.mem.used_pct(),
        s.mem.used_kb() as f64 / 1024.0 / 1024.0,
        s.mem.total_kb as f64 / 1024.0 / 1024.0
    );

    if s.net.iface.is_empty() {
        println!("net   offline");
    } else {
        println!(
            "net   {}  down {}  up {}",
            s.net.iface,
            rate(s.net.rx_bps),
            rate(s.net.tx_bps)
        );
    }
}

fn print_caps(pci: &str, c: &GpuCapabilities) {
    println!("{pci}");
    let yes_no = |b: bool| if b { "yes" } else { "no " };
    println!("  power limit    {}", yes_no(c.power_limit));
    if c.power_limit {
        println!(
            "                 {:.0} – {:.0} W",
            c.power_limit_min_w, c.power_limit_max_w
        );
    }
    println!("  clock offset   {}", yes_no(c.clock_offset));
    println!("  voltage        {}", yes_no(c.voltage));
    println!("  fan control    {}", yes_no(c.fan_control));
    println!("  fan curve      {}", yes_no(c.fan_curve));
    if !c.notes.is_empty() {
        println!();
        for n in &c.notes {
            println!("  · {n}");
        }
    }
}

/// Formats a byte rate at a sensible scale.
fn rate(bps: u64) -> String {
    const UNITS: [(&str, f64); 3] = [("MB/s", 1_048_576.0), ("kB/s", 1024.0), ("B/s", 1.0)];
    for (unit, scale) in UNITS {
        if bps as f64 >= scale {
            return format!("{:.1} {unit}", bps as f64 / scale);
        }
    }
    "0 B/s".to_string()
}

#[cfg(test)]
mod tests {
    use super::rate;

    #[test]
    fn durations_read_naturally() {
        assert_eq!(super::hms(65), "1:05");
        assert_eq!(super::hms(3600 * 12 + 300), "12:05:00");
    }

    #[test]
    fn durations_read_the_way_people_write_them() {
        assert_eq!(super::parse_duration("90s").unwrap(), 90_000);
        assert_eq!(super::parse_duration("2h").unwrap(), 7_200_000);
        assert_eq!(super::parse_duration("1h30m").unwrap(), 5_400_000);
        for bad in ["", "2", "h", "2x", "0m"] {
            assert!(super::parse_duration(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_clock_time_is_always_in_the_future_and_bad_ones_are_refused() {
        let now = super::now_ms();
        let t = super::next_local_time_ms("00:00").unwrap();
        assert!(t > now && t - now <= 24 * 3_600_000);
        assert!(super::next_local_time_ms("25:00").is_err());
        assert!(super::next_local_time_ms("noon").is_err());
    }

    #[test]
    fn csv_fields_are_quoted_only_when_needed() {
        assert_eq!(super::csv_field("Counter-Strike 2"), "Counter-Strike 2");
        assert_eq!(super::csv_field("Warhammer 40,000"), "\"Warhammer 40,000\"");
        assert_eq!(super::csv_field("The \"Game\""), "\"The \"\"Game\"\"\"");
    }

    #[test]
    fn rates_pick_a_readable_scale() {
        assert_eq!(rate(0), "0 B/s");
        assert_eq!(rate(512), "512.0 B/s");
        assert_eq!(rate(2048), "2.0 kB/s");
        assert_eq!(rate(5_242_880), "5.0 MB/s");
    }
}
