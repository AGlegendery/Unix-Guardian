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
use ug_proto::agent::{GamesProxy, MonitorProxy, OverlayProxy, PingProxy, PlaytimeProxy, ProfilesProxy, SettingsProxy, ShortcutsProxy};
use ug_proto::daemon::{CoolingProxy, DaemonProxy, GpuProxy, NetProxy, PowerProxy};
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
    /// Unix Guardian in the terminal: every page, by mouse or keyboard.
    Tui,
    /// Run a program on a chosen GPU, e.g. `ug run --gpu nvidia -- steam`.
    /// No restart or log-out: the program alone renders there (PRIME offload).
    Run {
        /// discrete, integrated, nvidia, amd, intel, or a PCI address.
        /// Defaults to the GPU chosen with `ug gpu default`, else the
        /// discrete one.
        #[arg(long)]
        gpu: Option<String>,
        /// Print the Steam launch options for this GPU instead of running.
        #[arg(long)]
        steam: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
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
    /// Battery protection: stop charging at a limit and run from AC.
    Battery {
        #[command(subcommand)]
        command: Option<BatteryCommand>,
    },
    /// Internet use per app and service; block an app or cap its speed.
    Net {
        #[command(subcommand)]
        command: Option<NetCommand>,
        /// Include apps that have not used the internet.
        #[arg(long)]
        all: bool,
    },
    /// Check the graphics driver setup and explain what is wrong. Reads
    /// files only: works without the service, from a text console.
    Diagnose,
    /// Fix a driver problem `ug diagnose` found (with a backup), list the
    /// changes made, or undo one.
    Driver {
        #[command(subcommand)]
        command: DriverCommand,
    },
    /// Fans: what can be driven, fan curves, and the laptop's fan modes.
    Fans {
        /// Drive a fan by a curve, e.g. --curve amdgpu/pwm1=40:25,70:60,85:100
        #[arg(long, value_name = "ID=T:P,...")]
        curve: Option<String>,
        /// Hand a fan back to automatic control.
        #[arg(long, value_name = "ID")]
        auto: Option<String>,
        /// The laptop firmware's fan mode: quiet, standard or cooling.
        #[arg(long)]
        mode: Option<String>,
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
        #[command(subcommand)]
        command: Option<PlaytimeCommand>,
    },
    /// Temperature, load and power over the past hours (recorded once a minute).
    History {
        #[arg(long, default_value_t = 24)]
        hours: u32,
    },
    /// Internet traffic per day.
    Traffic {
        #[arg(long, default_value_t = 7)]
        days: u32,
    },
    /// Ping to the router, the DNS server and the running game's server.
    Ping {
        /// Seconds to watch before printing (the agent measures once a second).
        #[arg(long, default_value_t = 10)]
        seconds: u32,
    },
    /// Show the tray icon again after "Quit tray icon" (or hide it with
    /// --hide; it then stays hidden across logins).
    Tray {
        #[arg(long)]
        hide: bool,
    },
    /// Close Unix Guardian's windows and terminal UIs and stop the user
    /// agent (game mode, playtime and FPS logging stop until the next login
    /// or until the window is opened). The system service stays, idle.
    Quit,
    /// The in-game overlay (MangoHud): show its settings, set it up, change it.
    Overlay {
        #[command(subcommand)]
        command: Option<OverlayCommand>,
    },
    /// The agent's settings (as on the Settings page); only the ones given
    /// change.
    Settings {
        /// Desktop notifications: on or off.
        #[arg(long)]
        notifications: Option<String>,
        /// Seconds between looks for running games, 1 to 10.
        #[arg(long)]
        scan_interval: Option<u32>,
        /// Close Unix Guardian's window when a game starts: on or off.
        #[arg(long)]
        close_window_on_game: Option<String>,
    },
    /// Record the screen and the game's sound (gpu-screen-recorder). Without
    /// a command shows the recorder.
    Record {
        #[command(subcommand)]
        command: Option<RecordCommand>,
    },
    /// Global shortcuts (the desktop's GlobalShortcuts portal). Without an
    /// argument shows them; `on`/`off` binds or unbinds them (the desktop may
    /// ask to confirm the keys); `open-window`, `toggle-overlay` or
    /// `toggle-gamemode` does what that shortcut does, for binding in a
    /// desktop without the portal; `keys <id> <keys>` picks the keys (e.g.
    /// `ug shortcut keys toggle-gamemode Meta+Alt+G`; no keys: the suggested
    /// ones).
    Shortcut {
        action: Option<String>,
        /// For `keys`: the shortcut id and the keys.
        args: Vec<String>,
    },
    /// Frame statistics of recent game sessions (from MangoHud's log).
    Fps {
        /// Only the last seven days.
        #[arg(long)]
        week: bool,
        /// Delete every recorded session.
        #[arg(long)]
        forget: bool,
    },
    /// Game profiles, and which game uses which.
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
}

#[derive(Subcommand)]
enum DriverCommand {
    /// Show exactly what a fix would change, without changing anything.
    Plan { fix: String },
    /// Apply a fix: shows the plan, asks, then runs it with a backup.
    Fix {
        fix: String,
        /// Do not ask first.
        #[arg(long)]
        yes: bool,
    },
    /// Driver changes made so far, newest first, with their log.
    History,
    /// Undo a change (the newest one still in place if no id is given).
    Rollback { id: Option<String> },
    /// After the restart a change needed: keep it (the newest waiting one if
    /// no id is given). Undo it instead with `ug driver rollback`.
    Keep { id: Option<String> },
    /// What protects package changes here: the snapshot tool found, or how
    /// to get one. Works without the service.
    Snapshot,
    /// Add or remove a kernel boot parameter, the boot loader's way (GRUB,
    /// systemd-boot, rEFInd, kernelstub), with a backup and undo.
    Param {
        /// add or remove
        action: String,
        /// e.g. mem_sleep_default=deep (remove: the name is enough)
        param: String,
        /// Do not ask first.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum RecordCommand {
    /// Start recording; prints the file.
    Start,
    /// Stop recording (or the replay buffer).
    Stop,
    /// Keep the last seconds in memory, to save with `ug record save`.
    Replay,
    /// Save the replay buffer to a file.
    Save,
    /// Install gpu-screen-recorder for this user (Flatpak from Flathub).
    Setup,
    /// Remove the gpu-screen-recorder Flatpak installed by `setup`.
    Remove,
    /// Change settings; only the ones given change.
    Set {
        /// small, balanced or high
        #[arg(long)]
        quality: Option<String>,
        /// 30, 60 or 120
        #[arg(long)]
        fps: Option<u32>,
        /// portal, screen or a monitor name ("" for the default)
        #[arg(long)]
        capture: Option<String>,
        /// auto, h264, hevc or av1
        #[arg(long)]
        codec: Option<String>,
        /// The game's sound: on or off
        #[arg(long)]
        sound: Option<String>,
        /// The microphone: on or off
        #[arg(long)]
        mic: Option<String>,
        /// Seconds the replay buffer keeps: 15, 30, 60 or 120
        #[arg(long)]
        replay: Option<u32>,
        /// Folder for recordings ("" for ~/Videos/Unix Guardian)
        #[arg(long)]
        folder: Option<String>,
    },
}

#[derive(Subcommand)]
enum OverlayCommand {
    /// Add Unix Guardian's block to MangoHud.conf (your file is backed up).
    Setup,
    /// Remove everything Unix Guardian added to MangoHud's configuration.
    Undo,
    /// Change settings; only the ones given change.
    Set {
        /// gaming, always or off
        #[arg(long)]
        show: Option<String>,
        /// card or compact
        #[arg(long)]
        layout: Option<String>,
        /// top-left, top-right, bottom-left, bottom-right or top-center
        #[arg(long)]
        position: Option<String>,
        /// small, medium or large
        #[arg(long)]
        size: Option<String>,
        /// Background opacity, 0-100.
        #[arg(long)]
        opacity: Option<u8>,
        /// The key that shows and hides it in game, e.g. Alt_L+o.
        #[arg(long)]
        key: Option<String>,
    },
}

#[derive(Subcommand)]
enum NetCommand {
    /// Cut an app off the internet (by the name `ug net` shows).
    Block {
        app: String,
        /// Only while game mode is on.
        #[arg(long)]
        gaming: bool,
    },
    /// Cap an app's upload and/or download speed.
    Limit {
        app: String,
        /// Upload cap, e.g. 500k or 2m (bits per second).
        #[arg(long)]
        up: Option<String>,
        /// Download cap, e.g. 8m.
        #[arg(long)]
        down: Option<String>,
        /// Only while game mode is on.
        #[arg(long)]
        gaming: bool,
    },
    /// Remove an app's block or cap.
    Allow { app: String },
    /// Every rule.
    Rules,
}

/// `500k`, `2m`, `2.5m` or plain kilobits → kbit/s.
fn parse_kbps(s: &str) -> Result<u32> {
    let t = s.trim().to_lowercase();
    let (num, mul) = if let Some(n) = t.strip_suffix('m') {
        (n, 1000.0)
    } else if let Some(n) = t.strip_suffix('k') {
        (n, 1.0)
    } else {
        (t.as_str(), 1.0)
    };
    let v: f64 = num.parse().map_err(|_| anyhow::anyhow!("\"{s}\" is not a speed; try 500k or 2m (bits per second)"))?;
    Ok((v * mul).round() as u32)
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
        /// cpu_priority, memory_tuning, pause_services, network_tuning or power_profile.
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
    /// A Steam game's launch options: which GPU it renders on and the
    /// MangoHud overlay. Shows the change; --apply writes it (Steam closed),
    /// --undo takes Unix Guardian's part back out.
    Launch {
        /// steam:730, 730, or part of the game's name.
        game: String,
        /// default, nvidia, amd, intel, discrete, integrated or a PCI address.
        #[arg(long)]
        gpu: Option<String>,
        /// Load the MangoHud overlay.
        #[arg(long, value_parser = ["on", "off"])]
        overlay: Option<String>,
        /// Write it into Steam's settings (backed up first).
        #[arg(long, conflicts_with = "undo")]
        apply: bool,
        /// Put the game's own launch options back.
        #[arg(long)]
        undo: bool,
    },
}

#[derive(Subcommand)]
enum BatteryCommand {
    /// What this laptop supports and what is set (the default).
    Status,
    /// Stop charging at --limit % (default 80; vendor modes use their own).
    Protect {
        #[arg(long, default_value_t = 80)]
        limit: u8,
    },
    /// Charge to 100 % once (for a trip), then protect again.
    FullOnce {
        #[arg(long, default_value_t = 80)]
        limit: u8,
    },
    /// Charge to 100 % always (the laptop's own behaviour).
    Off,
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
    /// Power draw now and CPU energy per day.
    Usage,
    /// The scheduled and conditional actions, and the wake alarm.
    List,
    /// Remove a scheduled action by its number.
    Cancel { id: u32 },
}

#[derive(Subcommand)]
enum PlaytimeCommand {
    /// Tracked apps, and programs open now that could be tracked.
    Apps,
    /// Count a program's time although it is not a game (never tuned).
    Track {
        /// Executable name, e.g. firefox-esr or "Wondershare Filmora.exe".
        exe: String,
        /// Name to show; defaults to the executable's.
        #[arg(long)]
        name: Option<String>,
    },
    /// Stop counting a program's time (what was counted stays).
    Untrack { exe: String },
    /// A program taken for a game by its behaviour (key exe:…) is not one:
    /// track it as an app instead, with the time counted so far.
    NotAGame { key: String },
    /// Add Steam's own playtime to the all-time totals (only what Unix
    /// Guardian did not count itself; importing again replaces it).
    ImportSteam {
        /// Remove what was imported.
        #[arg(long)]
        undo: bool,
    },
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
    /// Show or choose the GPU the programs Unix Guardian starts use (the
    /// dashboard's card switch): `ug run` without `--gpu`. Never changes
    /// the desktop or programs started elsewhere.
    Default {
        /// integrated, discrete, nvidia, amd, intel, or a PCI address.
        gpu: Option<String>,
    },
    /// Start one application on the discrete GPU from its menu entry: a
    /// per-user copy of its .desktop file with PrefersNonDefaultGPU=true.
    App {
        /// The entry's name, e.g. blender or org.kde.krita (.desktop optional).
        id: String,
        /// Remove Unix Guardian's copy again (back to the system's choice).
        #[arg(long)]
        off: bool,
    },
    /// Show or change which GPU tuning acts on.
    Active {
        /// PCI address to switch to; omit to print the current one.
        pci: Option<String>,
    },
    /// Power limit and clock offsets: show them, or try new values. A new
    /// setting is undone by itself after 30 seconds unless you keep it.
    Tune {
        /// PCI address; defaults to the active GPU.
        #[arg(long)]
        pci: Option<String>,
        /// Power limit in watts (0 = the card's default).
        #[arg(long)]
        power: Option<f64>,
        /// Core clock offset in MHz, e.g. 100 or -50.
        #[arg(long, allow_hyphen_values = true)]
        core: Option<i32>,
        /// Memory clock offset in MHz.
        #[arg(long, allow_hyphen_values = true)]
        mem: Option<i32>,
        /// Run the stability test for this many seconds before asking to
        /// keep it (an unstable setting is undone at once).
        #[arg(long, value_name = "SECONDS")]
        test: Option<u32>,
    },
    /// Find a setting by measuring: `faster` (highest stable core clock)
    /// or `cooler` (lowest power limit at the same clock). Takes minutes.
    Autotune {
        #[arg(long)]
        pci: Option<String>,
        #[arg(long, value_parser = ["faster", "cooler"])]
        goal: String,
    },
    /// Keep the setting being tried.
    Keep {
        #[arg(long)]
        pci: Option<String>,
    },
    /// Back to the card's own defaults, now and from now on.
    Reset {
        #[arg(long)]
        pci: Option<String>,
    },
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e:#}");
            eprintln!("If this looks like a bug in Unix Guardian, please report it to {}.", ug_core::BUG_REPORT_EMAIL);
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    if let Command::Tui = cli.command {
        return tui(cli.session);
    }
    if let Command::Run { gpu, steam, command } = &cli.command {
        let gpu = gpu.clone().or_else(|| ug_core::gpu_default::chosen(&ug_core::paths::user_config()));
        return run_on_gpu(gpu.as_deref().unwrap_or("discrete"), *steam, command);
    }
    if let Command::Gpu { command: GpuCommand::Default { gpu } } = &cli.command {
        return default_gpu(gpu.as_deref(), cli.json);
    }
    if let Command::Gpu { command: GpuCommand::App { id, off } } = &cli.command {
        return gpu_app(id, *off);
    }
    if let Command::Driver { command: DriverCommand::Snapshot } = cli.command {
        return driver_snapshot();
    }
    if let Command::Diagnose = cli.command {
        return diagnose(cli.json, cli.session).await;
    }

    // The agent always lives on the session bus, so game and playtime commands
    // work whether or not the daemon is reachable.
    match cli.command {
        Command::Game { command } => return game(command, cli.json).await,
        Command::Playtime { command: Some(c), .. } => return playtime_command(c, cli.json).await,
        Command::Playtime { today, week, forget, csv, command: None } => return playtime(today, week, forget, csv, cli.json).await,
        Command::Profile { command } => return profile(command, cli.json).await,
        Command::History { hours } => return history(hours, cli.json).await,
        Command::Traffic { days } => return traffic(days, cli.json).await,
        Command::Ping { seconds } => return ping(seconds, cli.json).await,
        Command::Overlay { command } => return overlay(command, cli.json).await,
        Command::Shortcut { action, args } => return shortcut(action, args, cli.json).await,
        Command::Record { command } => return record(command, cli.json).await,
        Command::Settings { notifications, scan_interval, close_window_on_game } => {
            return settings(notifications, scan_interval, close_window_on_game, cli.json).await
        }
        Command::Tray { hide } => return tray(hide).await,
        Command::Quit => {
            let conn = agent_conn().await?;
            ug_proto::agent::TrayProxy::new(&conn).await?.quit().await.map_err(agent_hint)?;
            println!("Unix Guardian is closing. It starts again at your next login, or when you open it.");
            return Ok(());
        }
        Command::Fps { week, forget } => return fps_sessions(week, forget, cli.json).await,
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
        Command::Battery { command } => battery(&conn, command.unwrap_or(BatteryCommand::Status), cli.json).await,
        Command::Net { command, all } => net(&conn, command, all, cli.json).await,
        Command::Fans { curve, auto, mode } => fans(&conn, curve, auto, mode, cli.json).await,
        Command::Driver { command } => driver(&conn, command, cli.json).await,
        Command::Diagnose => unreachable!("handled above"),
        Command::Game { .. } | Command::Playtime { .. } | Command::Profile { .. } | Command::History { .. } | Command::Traffic { .. } | Command::Ping { .. } | Command::Overlay { .. } | Command::Shortcut { .. } | Command::Record { .. } | Command::Settings { .. } | Command::Tray { .. } | Command::Quit | Command::Fps { .. } | Command::Run { .. } | Command::Tui => {
            unreachable!("handled above")
        }
    }
}

async fn agent_conn() -> Result<zbus::Connection> {
    zbus::Connection::session().await.context("connecting to the session bus")
}

fn agent_hint(e: zbus::Error) -> anyhow::Error {
    if let Some(m) = ug_proto::outdated_peer(&e.to_string()) {
        return anyhow::anyhow!("{m}");
    }
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
        GameCommand::Launch { game, gpu, overlay, apply, undo } => {
            return steam_launch(&games, &game, gpu.as_deref(), overlay.as_deref(), apply, undo, json).await;
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
                writeln!(out, "{}", ug_core::playtime::csv_row(s))?;
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
    let (apps, games): (Vec<_>, Vec<_>) = entries.iter().partition(|e| e.key.starts_with("app:"));
    let imported = since == 0 && !pt.imported().await.unwrap_or_default().is_empty();
    if !games.is_empty() {
        let total: u64 = games.iter().map(|e| e.seconds).sum();
        for e in &games {
            let pct = if total > 0 { e.seconds * 100 / total } else { 0 };
            println!("{:>8}  {:>3}%  {}", hms(e.seconds), pct, e.name);
        }
        println!("{:>8}        total, {} games", hms(total), games.len());
        if imported {
            println!("All time includes playtime imported from Steam (ug playtime import-steam --undo removes it).");
        }
    }
    if !apps.is_empty() {
        println!("{}Apps", if games.is_empty() { "" } else { "\n" });
        for e in &apps {
            println!("{:>8}        {}", hms(e.seconds), e.name);
        }
    }
    Ok(())
}

async fn playtime_command(command: PlaytimeCommand, json: bool) -> Result<()> {
    let conn = agent_conn().await?;
    let pt = PlaytimeProxy::new(&conn).await?;
    match command {
        PlaytimeCommand::Apps => {
            let tracked = pt.tracked_apps().await.map_err(agent_hint)?;
            let open = pt.running_programs().await.map_err(agent_hint)?;
            if json {
                println!("{}", serde_json::json!({ "tracked": tracked, "open": open }));
                return Ok(());
            }
            if tracked.is_empty() {
                println!("No apps tracked. Track one with: ug playtime track <executable>");
            } else {
                println!("Tracked");
                for a in &tracked {
                    println!("  {:<28} {}", a.name, a.exe);
                }
            }
            if !open.is_empty() {
                println!("\nOpen now");
                for a in &open {
                    println!("  {:<28} {}", a.name, a.exe);
                }
            }
        }
        PlaytimeCommand::Track { exe, name } => {
            pt.track_app(&exe, name.as_deref().unwrap_or("")).await.map_err(agent_hint)?;
            println!("Counting the time of {exe} from now on (it is never treated as a game).");
        }
        PlaytimeCommand::Untrack { exe } => {
            if pt.untrack_app(&exe).await.map_err(agent_hint)? {
                println!("No longer counting {exe}; the time counted so far stays (ug playtime --forget app:… deletes it).");
            } else {
                println!("{exe} was not tracked.");
            }
        }
        PlaytimeCommand::NotAGame { key } => {
            let n = pt.move_to_apps(&key).await.map_err(agent_hint)?;
            println!("{key} is no longer a game: tracked as an app, with {n} sessions moved.");
        }
        PlaytimeCommand::ImportSteam { undo: true } => {
            let n = pt.forget_imported().await.map_err(agent_hint)?;
            println!("Removed the imported playtime of {n} games.");
        }
        PlaytimeCommand::ImportSteam { undo: false } => {
            let (games, seconds) = pt.import_steam().await.map_err(agent_hint)?;
            if games == 0 {
                println!("Nothing to add: Steam has no playtime that Unix Guardian has not counted already.");
            } else {
                println!("Added {} of Steam playtime across {games} games to the all-time totals.", hms(seconds));
            }
        }
    }
    Ok(())
}

/// Runs the terminal interface; `g` inside it asks for the window instead.
fn tui(session: bool) -> Result<()> {
    // The TUI drives its own small runtime; it cannot run inside this one.
    let exit = std::thread::spawn(move || ug_tui::run(session))
        .join()
        .map_err(|_| anyhow::anyhow!("the terminal interface stopped unexpectedly"))??;
    match exit {
        ug_tui::Exit::Quit => Ok(()),
        ug_tui::Exit::OpenWindow => {
            // The window lives beside ug when installed, or in the dev build.
            let here = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("ug-app")));
            let candidates = [here, Some("ug-app".into()), Some("app/src-tauri/target/release/ug-app".into())];
            for c in candidates.into_iter().flatten() {
                if std::process::Command::new(&c).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().is_ok() {
                    println!("Opened the Unix Guardian window.");
                    return Ok(());
                }
            }
            anyhow::bail!("the Unix Guardian window (ug-app) is not installed")
        }
    }
}

async fn history(hours: u32, json: bool) -> Result<()> {
    let conn = agent_conn().await?;
    let m = MonitorProxy::new(&conn).await?;
    let now = now_ms();
    let pts = m.history(now.saturating_sub(hours as u64 * 3_600_000), now, 24).await.map_err(agent_hint)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&pts)?);
        return Ok(());
    }
    if pts.is_empty() {
        println!("No history yet: the agent records one reading a minute.");
        return Ok(());
    }
    let f = |v: f64, unit: &str| if v.is_nan() { "—".to_string() } else { format!("{v:.0}{unit}") };
    println!("{:<17} {:>8} {:>8} {:>8} {:>8} {:>8}", "time", "CPU °C", "CPU %", "CPU W", "GPU °C", "GPU %");
    for p in pts {
        println!(
            "{:<17} {:>8} {:>8} {:>8} {:>8} {:>8}",
            &local_time(p.ts_unix_ms)[..16],
            f(p.cpu_temp_c, ""),
            f(p.cpu_load_pct, ""),
            f(p.cpu_w, ""),
            f(p.gpu_temp_c, ""),
            f(p.gpu_load_pct, "")
        );
    }
    Ok(())
}

async fn traffic(days: u32, json: bool) -> Result<()> {
    let conn = agent_conn().await?;
    let m = MonitorProxy::new(&conn).await?;
    let t = m.traffic(now_ms().saturating_sub(days as u64 * 86_400_000)).await.map_err(agent_hint)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&t)?);
        return Ok(());
    }
    if t.is_empty() {
        println!("No traffic recorded yet.");
    }
    let mb = |b: u64| b as f64 / 1e6;
    for d in t {
        println!("{}  down {:>9.1} MB  up {:>9.1} MB", d.date, mb(d.rx_bytes), mb(d.tx_bytes));
    }
    Ok(())
}

async fn tray(hide: bool) -> Result<()> {
    let conn = agent_conn().await?;
    let t = ug_proto::agent::TrayProxy::new(&conn).await?;
    if hide {
        t.hide().await.map_err(agent_hint)?;
        println!("Tray icon hidden, also after logging in again. `ug tray` shows it again.");
    } else {
        t.show().await.map_err(agent_hint)?;
        println!("Tray icon shown.");
    }
    Ok(())
}

async fn ping(seconds: u32, json: bool) -> Result<()> {
    let conn = agent_conn().await?;
    let p = PingProxy::new(&conn).await?;
    let start = now_ms();
    // The first call starts the measurement; wait for it to collect samples.
    p.recent(start).await.map_err(agent_hint)?;
    if !json {
        eprintln!("Measuring for {seconds} s…");
    }
    tokio::time::sleep(std::time::Duration::from_secs(seconds.max(1) as u64)).await;
    let r = p.recent(start).await.map_err(agent_hint)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&r)?);
        return Ok(());
    }
    let f = |v: f64| if v.is_nan() { "—".to_string() } else { format!("{v:.1} ms") };
    println!("{:<24} {:<16} {:>10} {:>10} {:>10} {:>6}", "", "address", "median", "jitter", "95%", "loss");
    for t in r.targets {
        let name = if t.id == "game" { format!("Game: {}", t.label) } else { t.label.clone() };
        if !t.problem.is_empty() {
            println!("{name:<24} {}", t.problem);
            continue;
        }
        let s = t.stats;
        println!(
            "{name:<24} {:<16} {:>10} {:>10} {:>10} {:>5.0}%",
            t.addr,
            f(s.median_ms),
            f(s.jitter_ms),
            f(s.p95_ms),
            s.loss_pct
        );
    }
    Ok(())
}

/// A Steam game's launch options, through the agent (which reads and writes
/// Steam's settings): shows the change, writes it with --apply.
async fn steam_launch(
    games: &GamesProxy<'_>,
    game: &str,
    gpu: Option<&str>,
    overlay: Option<&str>,
    apply: bool,
    undo: bool,
    json: bool,
) -> Result<()> {
    let id = game.trim().strip_prefix("steam:").unwrap_or(game.trim());
    let appid = match id.parse::<u32>() {
        Ok(a) => a,
        Err(_) => {
            let q = game.to_lowercase();
            let installed = games.installed().await.map_err(agent_hint)?;
            let hits: Vec<_> = installed.iter().filter(|g| g.launcher == "steam" && g.name.to_lowercase().contains(&q)).collect();
            match hits[..] {
                [g] => g.id.trim_start_matches("steam:").parse().unwrap_or(0),
                [] => anyhow::bail!("no installed Steam game matches {game:?}; give its App ID, e.g. ug game launch 730"),
                _ => anyhow::bail!(
                    "{game:?} matches {}; be more precise or give the App ID",
                    hits.iter().map(|g| format!("{} ({})", g.name, g.id)).collect::<Vec<_>>().join(", ")
                ),
            }
        }
    };
    let info = games.launch_options(appid).await.map_err(agent_hint)?;
    if undo {
        let (options, backup) = games.undo_launch_options(appid).await.map_err(agent_hint)?;
        println!("{}: launch options back to {:?}. Steam's settings were backed up to {backup}", info.name, options);
        return Ok(());
    }
    let mut want_gpu = info.gpu.clone();
    if let Some(g) = gpu {
        want_gpu = match g.trim().to_lowercase().as_str() {
            "default" | "none" | "" => String::new(),
            other => {
                let gpus = ug_hw::gpu_use::gpus(&ug_hw::SysRoot::real());
                ug_hw::gpu_use::pick(&gpus, other).map(|g| g.pci.clone()).ok_or_else(|| anyhow::anyhow!("no GPU matches {other:?}; see ug gpu list"))?
            }
        };
    }
    let want_overlay = overlay.map(|o| o == "on").unwrap_or(info.overlay);
    let after = games.preview_launch_options(appid, &want_gpu, want_overlay).await.map_err(agent_hint)?;
    let shown = |s: &str| if s.is_empty() { "(none)".to_string() } else { s.to_string() };
    if json {
        println!("{}", serde_json::json!({ "info": info, "after": after }));
    } else {
        println!("{} (steam:{appid})", info.name);
        println!("  now:    {}", shown(&info.current));
        println!("  after:  {}", shown(&after));
        if info.changed_outside {
            println!("  Changed in Steam since Unix Guardian set them: building on that.");
        }
        if !info.overlay_problem.is_empty() {
            println!("  Overlay: {}", info.overlay_problem);
        }
    }
    let unchanged = after == info.current && want_gpu == info.gpu && want_overlay == info.overlay;
    if !apply {
        if !unchanged && !json {
            println!(
                "Nothing written. Add --apply to write it{}.",
                if info.steam_running { " once Steam is closed (it rewrites its settings when it exits)" } else { " (Steam's settings are backed up first)" }
            );
        }
        return Ok(());
    }
    if unchanged {
        println!("Already set; nothing to write.");
        return Ok(());
    }
    let (_, backup) = games.set_launch_options(appid, &want_gpu, want_overlay).await.map_err(agent_hint)?;
    println!("Written: Steam uses it the next time the game starts. Steam's settings were backed up to {backup}");
    Ok(())
}

/// Shows or sets the GPU `ug run` starts programs on. Per-user, so no daemon.
fn default_gpu(want: Option<&str>, json: bool) -> Result<()> {
    use ug_core::gpu_default;
    let root = ug_hw::SysRoot::real();
    let gpus = ug_hw::gpu_use::gpus(&root);
    let ug = ug_core::paths::user_config();
    if let Some(want) = want {
        let Some(gpu) = ug_hw::gpu_use::pick(&gpus, want) else {
            anyhow::bail!("no GPU matches {want:?}; see ug gpu list");
        };
        gpu_default::set(&ug, gpu)?;
    }
    let st = gpu_default::state(&ug, &gpus);
    let describe = |pci: &str| {
        gpus.iter()
            .find(|g| g.pci == pci)
            .map(|g| format!("{pci} ({}{})", g.vendor_name(), if g.boot_vga { ", integrated" } else { "" }))
            .unwrap_or_else(|| pci.to_string())
    };
    if json {
        println!("{}", serde_json::json!({ "chosen": st.chosen }));
        return Ok(());
    }
    println!("Programs Unix Guardian starts (`ug run -- <program>`) use {}", describe(&st.chosen));
    println!("The desktop and programs started elsewhere are not changed; for one app's menu entry use `ug gpu app <name>`.");
    if gpus.len() < 2 {
        println!("This machine has one GPU, so there is nothing to switch.");
    }
    Ok(())
}

/// Writes or removes the per-user `.desktop` copy that starts one app on
/// the discrete GPU.
fn gpu_app(id: &str, off: bool) -> Result<()> {
    use ug_core::gpu_default;
    let name = if id.ends_with(".desktop") { id.to_string() } else { format!("{id}.desktop") };
    anyhow::ensure!(!name.contains('/'), "give the entry's name, not a path");
    let user_dir = ug_core::paths::user_data().parent().map(|p| p.join("applications")).unwrap_or_default();
    let target = user_dir.join(&name);
    let existing = std::fs::read_to_string(&target).ok();
    if off {
        match existing {
            Some(t) if gpu_default::is_our_override(&t) => {
                std::fs::remove_file(&target)?;
                println!("Removed {}; the app starts on the system's choice again.", target.display());
            }
            Some(_) => anyhow::bail!("{} was not written by Unix Guardian; leaving it alone", target.display()),
            None => println!("Unix Guardian has no entry for {name}; nothing to remove."),
        }
        return Ok(());
    }
    // A user's own copy is edited in place only when it is ours.
    let source = match &existing {
        Some(t) if !gpu_default::is_our_override(t) => {
            anyhow::bail!("{} is your own copy; add PrefersNonDefaultGPU=true to it by hand", target.display())
        }
        _ => {
            let dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
            let flatpak = [
                ug_core::paths::user_data().parent().map(|p| p.join("flatpak/exports/share")).unwrap_or_default(),
                "/var/lib/flatpak/exports/share".into(),
            ];
            let found = dirs
                .split(':')
                .map(std::path::PathBuf::from)
                .chain(flatpak)
                .map(|d| d.join("applications").join(&name))
                .find(|p| p.is_file());
            match (found, existing) {
                (Some(p), _) => std::fs::read_to_string(p)?,
                (None, Some(t)) => t,
                (None, None) => anyhow::bail!("no application entry {name}; see the names in /usr/share/applications"),
            }
        }
    };
    let Some(text) = gpu_default::desktop_override(&source) else {
        anyhow::bail!("{name} has no [Desktop Entry] group");
    };
    ug_core::paths::write_atomic(&target, text.as_bytes())?;
    println!("Wrote {}: the app starts on the discrete GPU from its menu entry.", target.display());
    println!("Undo with `ug gpu app {id} --off`.");
    Ok(())
}

/// Execs `command` with the environment that puts it on `want`.
fn run_on_gpu(want: &str, steam: bool, command: &[String]) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let root = ug_hw::SysRoot::real();
    let gpus = ug_hw::gpu_use::gpus(&root);
    let Some(gpu) = ug_hw::gpu_use::pick(&gpus, want) else {
        let have: Vec<String> = gpus.iter().map(|g| format!("{} ({}{})", g.pci, g.vendor_name(), if g.boot_vga { ", integrated" } else { "" })).collect();
        anyhow::bail!("no GPU matches {want:?}; this machine has: {}", have.join(", "));
    };
    if steam {
        println!("{}", ug_hw::gpu_use::launch_options(&root, gpu));
        return Ok(());
    }
    let Some((prog, args)) = command.split_first() else {
        anyhow::bail!("give a program to run, e.g. ug run --gpu {want} -- vkcube");
    };
    let err = std::process::Command::new(prog).args(args).envs(ug_hw::gpu_use::offload_env(&root, gpu)).exec();
    anyhow::bail!("could not run {prog}: {err}")
}

async fn overlay(command: Option<OverlayCommand>, json: bool) -> Result<()> {
    let conn = agent_conn().await?;
    let o = OverlayProxy::new(&conn).await?;
    match command {
        None => {}
        Some(OverlayCommand::Setup) => o.set_up(true).await.map_err(agent_hint)?,
        Some(OverlayCommand::Undo) => {
            o.set_up(false).await.map_err(agent_hint)?;
            println!("MangoHud's configuration is back as it was.");
            return Ok(());
        }
        Some(OverlayCommand::Set { show, layout, position, size, opacity, key }) => {
            let mut s = o.state().await.map_err(agent_hint)?.settings;
            if let Some(v) = show { s.show = v; }
            if let Some(v) = layout { s.layout = v; }
            if let Some(v) = position { s.position = v; }
            if let Some(v) = size { s.size = v; }
            if let Some(v) = opacity { s.opacity_pct = v; }
            if let Some(v) = key { s.toggle_key = v; }
            o.set(&s).await.map_err(agent_hint)?;
        }
    }
    let st = o.state().await.map_err(agent_hint)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&st)?);
        return Ok(());
    }
    if !st.installed {
        println!("MangoHud is not installed. Install it with: {}", st.install_hint);
        return Ok(());
    }
    let s = &st.settings;
    println!("Overlay     {}", if st.set_up { "set up" } else { "not set up (ug overlay setup)" });
    println!("Shows       {}", match s.show.as_str() { "gaming" => "only while a game runs", "always" => "always", _ => "never (FPS is still recorded)" });
    println!("Layout      {}, {}, {} size, background {} %", s.layout, s.position, s.size, s.opacity_pct);
    println!("Key         {} shows and hides it in game", s.toggle_key);
    println!("Games       per game: `mangohud %command%` in a Steam game's launch options, or `mangohud <game>`");
    Ok(())
}

fn on_off(v: &str) -> Result<bool> {
    match v {
        "on" => Ok(true),
        "off" => Ok(false),
        other => anyhow::bail!("give on or off, not {other}"),
    }
}

async fn settings(notifications: Option<String>, scan: Option<u32>, close: Option<String>, json: bool) -> Result<()> {
    let conn = agent_conn().await?;
    let p = SettingsProxy::new(&conn).await?;
    if notifications.is_some() || scan.is_some() || close.is_some() {
        let mut s = p.state().await.map_err(agent_hint)?.settings;
        if let Some(v) = notifications {
            s.notifications = on_off(&v)?;
        }
        if let Some(v) = scan {
            s.scan_interval_s = v;
        }
        if let Some(v) = close {
            s.close_window_on_game = on_off(&v)?;
        }
        p.set(&s).await.map_err(agent_hint)?;
    }
    let st = p.state().await.map_err(agent_hint)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&st)?);
        return Ok(());
    }
    let s = &st.settings;
    let yes = |b: bool| if b { "on" } else { "off" };
    println!("Notifications          {}", yes(s.notifications));
    println!(
        "Look for games every   {} s{}",
        s.scan_interval_s,
        if st.scan_interval_fixed { " (set by ug-agent --scan-interval)" } else { "" }
    );
    println!("Close window in game   {}", yes(s.close_window_on_game));
    println!("Agent                  {}", st.agent_version);
    println!("Settings and data      {}  {}", st.config_dir, st.data_dir);
    Ok(())
}

async fn diagnose(json: bool, session: bool) -> Result<()> {
    let mut env = ug_driver::Env::from_process();
    if env.kernel_log.is_empty() {
        // Most users may not read the kernel log; the service may.
        let conn = if session { zbus::Connection::session().await } else { zbus::Connection::system().await };
        if let Ok(conn) = conn {
            if let Ok(d) = ug_proto::daemon::DriverProxy::new(&conn).await {
                env.kernel_log = d.firmware_log().await.unwrap_or_default();
            }
        }
    }
    let r = ug_driver::diagnose(&ug_hw::SysRoot::real(), &env);
    let text = if json { serde_json::to_string_pretty(&r)? + "\n" } else { diagnosis_text(&r) };
    // `ug diagnose | head` closes the pipe early: a clean end, not an error.
    use std::io::Write;
    match std::io::stdout().write_all(text.as_bytes()) {
        Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => Err(e.into()),
        _ => Ok(()),
    }
}

/// The report as text, for a terminal.
fn diagnosis_text(r: &ug_proto::DriverReport) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    macro_rules! println {
        () => { out.push('\n') };
        ($($a:tt)*) => { let _ = writeln!(out, $($a)*); };
    }
    println!("Hardware and driver check — {}", r.distro);
    for f in &r.facts {
        if !f.value.is_empty() && f.key != "kernel.cmdline" {
            println!("  {:<30} {}", f.key, f.value);
        }
    }
    println!();
    if r.findings.is_empty() {
        println!("No problems found.");
        return out;
    }
    for f in &r.findings {
        let mark = match f.severity.as_str() {
            "critical" => "CRITICAL ",
            "important" => "IMPORTANT",
            _ => "ADVICE   ",
        };
        println!("{mark} {} ({})", f.title_en, f.category);
        println!("          {}", f.what_en);
        println!("          Why: {}", f.why_en);
        println!("          Fix: {}", f.how_en);
        if f.fix_auto {
            println!("          Unix Guardian can do it, with a backup: ug driver fix {}", f.fix_id);
        }
        if !f.commands.is_empty() {
            println!("          {}:", if f.fix_auto { "Or by hand" } else { "Commands" });
            for c in &f.commands {
                println!("            {c}");
            }
        }
        if !f.doc_url.is_empty() {
            println!("          More: {}", f.doc_url);
        }
        println!();
    }
    out
}

fn print_plan(p: &ug_proto::DriverPlan) {
    println!("{} — {}", p.title, p.fix_id);
    if !p.refused.is_empty() {
        println!("  Not available here: {}", p.refused);
        return;
    }
    for (i, s) in p.steps.iter().enumerate() {
        println!("  {}. {} {}", i + 1, s.kind, s.target);
        if s.kind == "write-file" || s.kind == "swap" {
            for l in s.detail.lines() {
                println!("       | {l}");
            }
        } else {
            println!("       {}", s.detail);
        }
        println!("       undo: {}", s.undo);
    }
    if !p.snapshot.is_empty() {
        println!("  Safety net: {}", p.snapshot);
    }
    if p.needs_reboot {
        println!("  A restart is needed afterwards.");
    }
}

/// A text polkit agent for this command, used only when the desktop has
/// none (a text console after the graphics broke), so the password can be
/// typed here. Stopped when dropped.
struct TtyAgent(Option<std::process::Child>);

impl TtyAgent {
    fn start() -> TtyAgent {
        use std::io::IsTerminal;
        // SAFETY: getuid cannot fail and has no preconditions.
        if unsafe { libc::getuid() } == 0 || !std::io::stdin().is_terminal() {
            return TtyAgent(None);
        }
        let child = std::process::Command::new("pkttyagent")
            .args(["--process", &std::process::id().to_string(), "--fallback"])
            .spawn()
            .ok();
        if child.is_some() {
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        TtyAgent(child)
    }
}

impl Drop for TtyAgent {
    fn drop(&mut self) {
        if let Some(c) = &mut self.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Reads files only, so it works with the service down.
fn driver_snapshot() -> Result<()> {
    let root = ug_hw::SysRoot::real();
    let family = format!("{:?}", ug_core::distro::Distro::detect(&root).family).to_lowercase();
    let st = ug_driver::snapshot::detect(&root, &family);
    println!("{}", st.summary);
    if st.tool.is_none() {
        println!("Without one, Unix Guardian shows package commands instead of running them.");
        for c in &st.get_one {
            println!("  {c}");
        }
    }
    Ok(())
}

async fn driver(conn: &zbus::Connection, command: DriverCommand, json: bool) -> Result<()> {
    let command = match command {
        DriverCommand::Param { action, param, yes } => match action.as_str() {
            "add" | "remove" => DriverCommand::Fix { fix: format!("param-{action}:{param}"), yes },
            other => anyhow::bail!("`{other}`: say add or remove, e.g. ug driver param add mem_sleep_default=deep"),
        },
        c => c,
    };
    let d = ug_proto::daemon::DriverProxy::new(conn).await?;
    let _agent = matches!(command, DriverCommand::Fix { .. } | DriverCommand::Rollback { .. }).then(TtyAgent::start);
    let follow = |id: String| {
        let d = d.clone();
        async move {
        let mut shown = 0;
        loop {
            let ops = d.ops().await?;
            let Some(op) = ops.iter().find(|o| o.id == id) else { anyhow::bail!("the change {id} disappeared") };
            for l in &op.log[shown..] {
                println!("  {l}");
            }
            shown = op.log.len();
            if op.state != "running" && op.state != "undoing" {
                println!("{}: {}", op.id, op.state);
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        }
    };
    match command {
        DriverCommand::Snapshot | DriverCommand::Param { .. } => unreachable!("handled before"),
        DriverCommand::Plan { fix } => {
            let p = d.plan(&fix).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&p)?);
            } else {
                print_plan(&p);
            }
        }
        DriverCommand::Fix { fix, yes } => {
            let p = d.plan(&fix).await?;
            print_plan(&p);
            if !p.refused.is_empty() {
                anyhow::bail!("not applied");
            }
            if !yes {
                use std::io::Write;
                print!("Type yes to make these changes (a backup is kept; undo with `ug driver rollback`): ");
                std::io::stdout().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if answer.trim() != "yes" {
                    println!("Nothing was changed.");
                    return Ok(());
                }
            }
            let id = d.apply(&fix).await?;
            follow(id).await?;
        }
        DriverCommand::History => {
            let ops = d.ops().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&ops)?);
            } else if ops.is_empty() {
                println!("No driver changes have been made.");
            } else {
                for o in ops {
                    let check = match o.verify.as_str() {
                        "restart" => "  (restart to finish)",
                        "check" => "  (restarted: does it work? ug driver keep, or ug driver rollback)",
                        _ => "",
                    };
                    println!("{}  {}  {}{check}", o.id, o.state, o.title);
                    for l in &o.log {
                        println!("    {l}");
                    }
                }
            }
        }
        DriverCommand::Keep { id } => {
            let ops = d.ops().await?;
            let id = match id {
                Some(i) => i,
                None => ops.iter().find(|o| o.verify == "check").map(|o| o.id.clone()).ok_or_else(|| anyhow::anyhow!("No change is waiting to be checked."))?,
            };
            d.keep(&id).await?;
            println!("Kept {id}.");
        }
        DriverCommand::Rollback { id } => {
            d.rollback(id.as_deref().unwrap_or("")).await?;
            let ops = d.ops().await?;
            let op = match &id {
                Some(i) => ops.iter().find(|o| &o.id == i),
                None => ops.iter().find(|o| o.state == "undone" || o.state == "interrupted"),
            };
            if let Some(o) = op {
                println!("{}: {} ({})", o.id, o.state, o.title);
                if o.needs_reboot {
                    println!("Restart for it to take effect.");
                }
            }
        }
    }
    Ok(())
}

async fn record(command: Option<RecordCommand>, json: bool) -> Result<()> {
    let conn = agent_conn().await?;
    let r = ug_proto::agent::RecorderProxy::new(&conn).await?;
    let onoff = |v: &str| match v {
        "on" | "yes" | "true" => Ok(true),
        "off" | "no" | "false" => Ok(false),
        other => Err(anyhow::anyhow!("`{other}`: say on or off")),
    };
    match command {
        None => {}
        Some(RecordCommand::Start) => {
            println!("Recording to {}", r.start().await.map_err(agent_hint)?);
            println!("Stop with: ug record stop");
            return Ok(());
        }
        Some(RecordCommand::Stop) => {
            let f = r.stop().await.map_err(agent_hint)?;
            println!("{}", if f.is_empty() { "Replay buffer stopped.".to_string() } else { format!("Saved {f}") });
            return Ok(());
        }
        Some(RecordCommand::Replay) => {
            r.start_replay().await.map_err(agent_hint)?;
        }
        Some(RecordCommand::Save) => {
            println!("Saved {}", r.save_replay().await.map_err(agent_hint)?);
            return Ok(());
        }
        Some(RecordCommand::Setup) | Some(RecordCommand::Remove) => {
            let on = matches!(command, Some(RecordCommand::Setup));
            r.set_up(on).await.map_err(agent_hint)?;
            println!("{} gpu-screen-recorder (Flatpak, for you only)…", if on { "Installing" } else { "Removing" });
            use std::io::{IsTerminal, Write};
            let tty = std::io::stdout().is_terminal();
            let mut shown = String::new();
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let st = r.state().await.map_err(agent_hint)?;
                if st.mode != "installing" {
                    if tty && !shown.is_empty() {
                        println!();
                    }
                    if !st.problem.is_empty() {
                        anyhow::bail!("{}", st.problem);
                    }
                    break;
                }
                if st.setup_step.is_empty() {
                    continue;
                }
                let filled = st.setup_pct as usize / 5;
                let line = format!("[{}{}] {:>3} %  step {}", "#".repeat(filled), "-".repeat(20 - filled), st.setup_pct, st.setup_step);
                if line != shown {
                    // On a terminal one line redrawn; into a pipe one line each.
                    if tty { print!("\r{line}"); } else { println!("{line}"); }
                    std::io::stdout().flush().ok();
                    shown = line;
                }
            }
        }
        Some(RecordCommand::Set { quality, fps, capture, codec, sound, mic, replay, folder }) => {
            let mut s = r.state().await.map_err(agent_hint)?.settings;
            if let Some(v) = quality { s.quality = v; }
            if let Some(v) = fps { s.fps = v; }
            if let Some(v) = capture { s.capture = v; }
            if let Some(v) = codec { s.codec = v; }
            if let Some(v) = sound { s.audio_output = onoff(&v)?; }
            if let Some(v) = mic { s.audio_mic = onoff(&v)?; }
            if let Some(v) = replay { s.replay_seconds = v; }
            if let Some(v) = folder { s.folder = v; }
            r.set(&s).await.map_err(agent_hint)?;
        }
    }
    let st = r.state().await.map_err(agent_hint)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&st)?);
        return Ok(());
    }
    let engine = match st.engine.as_str() {
        "" if st.can_set_up => "not installed (ug record setup installs it for you)",
        "" => "not installed (install gpu-screen-recorder with your package manager)",
        "flatpak" => "gpu-screen-recorder (Flatpak)",
        _ => "gpu-screen-recorder",
    };
    let s = &st.settings;
    println!("Recorder      {engine}");
    println!("Now           {}", match st.mode.as_str() { "recording" => format!("recording to {}", st.file), "replay" => format!("keeping the last {} s (ug record save)", s.replay_seconds), "installing" => "installing…".into(), _ => "idle".into() });
    let capture = if s.capture.is_empty() { "default".to_string() } else { s.capture.clone() };
    println!("Capture       {capture}   (offered: {})", st.captures.join(", "));
    println!("Quality       {}, {} fps, codec {}  ≈ {} MB a minute", s.quality, s.fps, s.codec, st.mb_per_minute);
    println!("Sound         game {}, microphone {}", if s.audio_output { "on" } else { "off" }, if s.audio_mic { "on" } else { "off" });
    println!("Replay        last {} s ≈ {} MB in memory", s.replay_seconds, st.replay_mb);
    println!("Folder        {}", st.folder);
    if !st.last_saved.is_empty() {
        println!("Last saved    {}", st.last_saved);
    }
    if !st.problem.is_empty() {
        println!("Problem       {}", st.problem);
    }
    Ok(())
}

async fn shortcut(action: Option<String>, args: Vec<String>, json: bool) -> Result<()> {
    let conn = agent_conn().await?;
    let s = ShortcutsProxy::new(&conn).await?;
    if action.as_deref() != Some("keys") && !args.is_empty() {
        anyhow::bail!("Unexpected `{}`. To pick keys: ug shortcut keys <id> <keys>", args.join(" "));
    }
    match action.as_deref() {
        None => {}
        Some("keys") => {
            let Some((id, keys)) = args.split_first() else {
                anyhow::bail!("Which shortcut? ug shortcut keys <id> <keys>, e.g. ug shortcut keys toggle-gamemode Meta+Alt+G");
            };
            println!("{}", s.set_keys(id, &keys.join("+")).await.map_err(agent_hint)?);
            return Ok(());
        }
        Some("on") => s.set_enabled(true).await.map_err(agent_hint)?,
        Some("off") => s.set_enabled(false).await.map_err(agent_hint)?,
        Some(id) => {
            println!("{}", s.run(id).await.map_err(agent_hint)?);
            return Ok(());
        }
    }
    if action.as_deref() == Some("on") {
        // Binding waits for the desktop (and maybe the user's confirmation).
        for _ in 0..20 {
            let st = s.state().await.map_err(agent_hint)?;
            if st.status != "connecting" && st.status != "waiting" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }
    let st = s.state().await.map_err(agent_hint)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&st)?);
        return Ok(());
    }
    let status = match st.status.as_str() {
        "off" => "off (ug shortcut on)",
        "connecting" => "connecting to the desktop",
        "waiting" => "waiting for you to confirm the keys in the desktop's dialog",
        "active" => "on",
        "unavailable" => "not offered by this desktop",
        _ => "failed",
    };
    println!("Shortcuts   {status}");
    if !st.problem.is_empty() {
        println!("            {}", st.problem);
    }
    for k in &st.shortcuts {
        // What the desktop gave; before binding, what will be asked for.
        let keys = if !k.trigger.is_empty() {
            k.trigger.clone()
        } else if k.chosen {
            format!("({}, yours)", k.keys)
        } else {
            format!("({})", k.keys)
        };
        println!("  {:<16} {:<20} {}", k.id, keys, k.description);
    }
    if !st.gamemode_by_hand.is_empty() {
        println!("Game mode   turned {} by hand", st.gamemode_by_hand);
    }
    Ok(())
}

async fn fps_sessions(week: bool, forget: bool, json: bool) -> Result<()> {
    let conn = agent_conn().await?;
    let m = MonitorProxy::new(&conn).await?;
    if forget {
        let n = m.forget_fps_sessions().await.map_err(agent_hint)?;
        println!("Deleted {n} sessions.");
        return Ok(());
    }
    let since = if week { now_ms().saturating_sub(7 * 86_400_000) } else { 0 };
    let list = m.fps_sessions(since).await.map_err(agent_hint)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&list)?);
        return Ok(());
    }
    if list.is_empty() {
        println!("No sessions yet. They are recorded when a game runs with MangoHud and Unix Guardian's overlay set up.");
        return Ok(());
    }
    println!("{:<17} {:<22} {:>7} {:>6} {:>7} {:>8} {:>9}", "started", "game", "minutes", "avg", "1% low", "0.1% low", "stutters");
    for s in list {
        println!(
            "{:<17} {:<22} {:>7} {:>6.0} {:>7.0} {:>8.0} {:>9}",
            &local_time(s.started_unix_ms)[..16],
            s.game.chars().take(22).collect::<String>(),
            s.duration_s / 60,
            s.avg_fps,
            s.low_1pct_fps,
            s.low_01pct_fps,
            s.stutters
        );
    }
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
                println!("    network tuning         {}", on(p.network_tuning));
                println!("    performance profile    {}", on(p.power_profile));
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
        PowerCommand::Usage => {
            let u = p.usage().await.map_err(daemon_error)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&u)?);
                return Ok(());
            }
            let w = |v: f64| if v.is_nan() { "—".to_string() } else { format!("{v:.1} W") };
            println!("CPU package   {}", w(u.cpu_w));
            println!("GPU           {}", w(u.gpu_w));
            println!("On battery    {}{}", w(u.battery_w), if u.battery_w.is_nan() { "  (only measurable while on battery)" } else { "  (whole machine)" });
            if u.cpu_energy_recorded {
                println!("\nCPU energy per day (the rest of the machine has no sensor):");
                for d in u.days.iter().rev().take(7) {
                    println!("  {}  {:.0} Wh", d.date, d.cpu_wh);
                }
            } else {
                println!("\nDaily energy is not recorded: this machine has no readable RAPL counter.");
            }
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

async fn fans(conn: &zbus::Connection, curve: Option<String>, auto: Option<String>, mode: Option<String>, json: bool) -> Result<()> {
    let c = CoolingProxy::new(conn).await?;
    if let Some(spec) = curve {
        let (id, pts) = spec.split_once('=').ok_or_else(|| anyhow::anyhow!("write it as ID=T:P,T:P (e.g. amdgpu/pwm1=40:25,85:100)"))?;
        let points: Vec<(f64, f64)> = pts
            .split(',')
            .map(|p| {
                let (t, d) = p.split_once(':').ok_or_else(|| anyhow::anyhow!("{p:?} is not T:P"))?;
                Ok((t.trim().parse()?, d.trim().parse()?))
            })
            .collect::<Result<_>>()?;
        c.set_curve(id, &points).await.map_err(daemon_error)?;
    }
    if let Some(id) = auto {
        c.set_auto(&id).await.map_err(daemon_error)?;
    }
    let st = c.state().await.map_err(daemon_error)?;
    if let Some(m) = mode {
        let v = st.fan_modes.iter().find(|(_, n)| n == &m).map(|(v, _)| *v).ok_or_else(|| {
            let names: Vec<&str> = st.fan_modes.iter().map(|(_, n)| n.as_str()).collect();
            anyhow::anyhow!("unknown fan mode {m:?}; this machine has: {}", if names.is_empty() { "none".into() } else { names.join(", ") })
        })?;
        c.set_fan_mode(v).await.map_err(daemon_error)?;
    }
    let st = c.state().await.map_err(daemon_error)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&st)?);
        return Ok(());
    }
    for f in &st.fans {
        let curve: Vec<String> = f.curve.iter().map(|(t, p)| format!("{t:.0}°C→{p:.0}%")).collect();
        println!(
            "{:<18} {:<24} {:>5} rpm {:>4.0} %  {:>4.0} °C  {}",
            f.id,
            f.label,
            f.rpm,
            f.pct,
            f.temp_c,
            if f.mode == "curve" { curve.join(" ") } else { "automatic".into() }
        );
    }
    if st.fan_mode >= 0 {
        let now = st.fan_modes.iter().find(|(v, _)| *v == st.fan_mode).map(|(_, n)| n.as_str()).unwrap_or("other");
        let all: Vec<&str> = st.fan_modes.iter().map(|(_, n)| n.as_str()).collect();
        println!("Firmware fan mode: {now} (choices: {})", all.join(", "));
    }
    for n in &st.notes {
        println!("{n}");
    }
    Ok(())
}

async fn net(conn: &zbus::Connection, command: Option<NetCommand>, all: bool, json: bool) -> Result<()> {
    let p = NetProxy::new(conn).await?;
    let when = |g: bool| if g { "gaming" } else { "always" };
    match command {
        None => {}
        Some(NetCommand::Block { app, gaming }) => {
            p.set_rule(&app, true, 0, 0, when(gaming)).await.map_err(daemon_error)?;
            println!("{app} is blocked{}.", if gaming { " while game mode is on" } else { "" });
            return Ok(());
        }
        Some(NetCommand::Limit { app, up, down, gaming }) => {
            let up = up.as_deref().map(parse_kbps).transpose()?.unwrap_or(0);
            let down = down.as_deref().map(parse_kbps).transpose()?.unwrap_or(0);
            if up == 0 && down == 0 {
                anyhow::bail!("give --up and/or --down, e.g. --up 500k");
            }
            p.set_rule(&app, false, up, down, when(gaming)).await.map_err(daemon_error)?;
            println!("{app} is capped{}.", if gaming { " while game mode is on" } else { "" });
            let st = p.status().await.map_err(daemon_error)?;
            for problem in st.shaping_problems {
                println!("Note: {problem}");
            }
            return Ok(());
        }
        Some(NetCommand::Allow { app }) => {
            if p.remove_rule(&app).await.map_err(daemon_error)? {
                println!("{app} has no limits any more.");
            } else {
                println!("{app} had no rule.");
            }
            return Ok(());
        }
        Some(NetCommand::Rules) => {
            let rules = p.rules().await.map_err(daemon_error)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rules)?);
            } else if rules.is_empty() {
                println!("No rules. `ug net block APP` or `ug net limit APP --up 500k` makes one.");
            } else {
                for r in rules {
                    let what = if r.block {
                        "blocked".to_string()
                    } else {
                        let f = |k: u32| if k == 0 { "—".to_string() } else if k >= 1000 { format!("{:.1} Mbit/s", k as f64 / 1000.0) } else { format!("{k} kbit/s") };
                        format!("up {}  down {}", f(r.up_kbps), f(r.down_kbps))
                    };
                    let when = if r.when == "gaming" { "while gaming" } else { "always" };
                    println!("{:<24} {:<32} {:<13} {}", r.app, what, when, if r.active { "in effect" } else { "waiting" });
                }
            }
            return Ok(());
        }
    }
    let status = p.status().await.map_err(daemon_error)?;
    if !status.available {
        anyhow::bail!("{}", status.problem);
    }
    // The first call starts two-second updates; the second has real rates.
    p.apps().await.map_err(daemon_error)?;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let apps = p.apps().await.map_err(daemon_error)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&apps)?);
        return Ok(());
    }
    if status.simulated {
        println!("(development service: invented numbers)");
    }
    let mb = |b: u64| format!("{:.1} MB", b as f64 / 1e6);
    println!("{:<28} {:<8} {:>12} {:>12} {:>11} {:>11}", "app", "kind", "down", "up", "down total", "up total");
    let mut hidden = 0;
    let (vpn, apps): (Vec<_>, Vec<_>) = apps.into_iter().partition(|a| a.category == "vpn");
    for a in apps {
        if !all && a.rx_bytes + a.tx_bytes == 0 {
            hidden += 1;
            continue;
        }
        let name: String = a.name.chars().take(28).collect();
        let name = match a.state.as_str() {
            "blocked" => format!("{} ⛔", name.chars().take(26).collect::<String>()),
            "limited" => format!("{} ⏬", name.chars().take(26).collect::<String>()),
            _ => name,
        };
        println!(
            "{name:<28} {:<8} {:>12} {:>12} {:>11} {:>11}",
            a.kind,
            rate(a.rx_bps),
            rate(a.tx_bps),
            mb(a.rx_bytes),
            mb(a.tx_bytes)
        );
    }
    if hidden > 0 {
        println!("({hidden} more without traffic; --all shows them)");
    }
    for v in vpn {
        println!(
            "\nVPN {} ({}): {} down, {} up of its own. It carries the apps above in {} mode; their traffic is not counted twice.",
            v.name,
            if status.vpn_tun { status.iface.as_str() } else { "proxy" },
            rate(v.rx_bps),
            rate(v.tx_bps),
            if status.vpn_tun { "TUN" } else { "proxy" }
        );
    }
    for p in &status.shaping_problems {
        println!("Note: {p}");
    }
    Ok(())
}

async fn battery(conn: &zbus::Connection, command: BatteryCommand, json: bool) -> Result<()> {
    let p = PowerProxy::new(conn).await?;
    match command {
        BatteryCommand::Status => {}
        BatteryCommand::Protect { limit } => p.set_battery_care("protect", limit).await.map_err(daemon_error)?,
        BatteryCommand::FullOnce { limit } => p.set_battery_care("full-once", limit).await.map_err(daemon_error)?,
        BatteryCommand::Off => p.set_battery_care("off", 100).await.map_err(daemon_error)?,
    }
    let b = p.battery().await.map_err(daemon_error)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&b)?);
        return Ok(());
    }
    if b.capacity <= 100 {
        println!("Battery       {} % ({})", b.capacity, b.status);
    }
    if !b.supported {
        println!("Protection    not available: {}", b.reason);
        return Ok(());
    }
    let how = if b.adjustable { format!("stops at {} %", b.limit) } else { "the vendor's conservation mode (limit set by the firmware)".into() };
    match b.mode.as_str() {
        "protect" => println!("Protection    on: {how}"),
        "full-once" => println!("Protection    paused for one full charge, then back on ({how})"),
        _ => println!("Protection    off: charges to 100 %"),
    }
    Ok(())
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
        GpuCommand::Default { .. } | GpuCommand::App { .. } => unreachable!("handled before connecting to the daemon"),
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
        GpuCommand::Autotune { pci, goal } => {
            let pci = match pci { Some(p) => p, None => gpu.active_gpu().await? };
            gpu.auto_tune(&pci, &goal).await.map_err(daemon_error)?;
            let mut last = String::new();
            let st = loop {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let s = gpu.auto_tune_status().await.map_err(daemon_error)?;
                if s.message != last {
                    eprintln!("[{}/{}] {}", s.step, s.steps, s.message);
                    last = s.message.clone();
                }
                if !s.running {
                    break s;
                }
            };
            println!("{}", st.summary);
            if !st.ok {
                return Ok(());
            }
            eprint!("Keep it? Type y and Enter within two minutes: ");
            let answer = tokio::time::timeout(std::time::Duration::from_secs(115), tokio::task::spawn_blocking(|| {
                let mut s = String::new();
                let _ = std::io::stdin().read_line(&mut s);
                s
            }))
            .await;
            if matches!(answer, Ok(Ok(ref s)) if s.trim().eq_ignore_ascii_case("y")) {
                gpu.keep_tune(&pci).await.map_err(daemon_error)?;
                println!("Kept.");
            } else {
                gpu.cancel_tune(&pci).await.ok();
                println!("\nUndone.");
            }
            Ok(())
        }
        GpuCommand::Keep { pci } => {
            let pci = match pci { Some(p) => p, None => gpu.active_gpu().await? };
            gpu.keep_tune(&pci).await.map_err(daemon_error)?;
            println!("Kept. It is applied again whenever the card wakes; `ug gpu reset` undoes it.");
            Ok(())
        }
        GpuCommand::Reset { pci } => {
            let pci = match pci { Some(p) => p, None => gpu.active_gpu().await? };
            gpu.reset_tune(&pci).await.map_err(daemon_error)?;
            println!("Back to the card's own defaults.");
            Ok(())
        }
        GpuCommand::Tune { pci, power, core, mem, test } => {
            let pci = match pci { Some(p) => p, None => gpu.active_gpu().await? };
            let st = gpu.tune_state(&pci).await.map_err(daemon_error)?;
            if power.is_none() && core.is_none() && mem.is_none() {
                if json {
                    println!("{}", serde_json::to_string_pretty(&st)?);
                    return Ok(());
                }
                print_tune(&st);
                return Ok(());
            }
            let mut t = st.applied;
            if let Some(p) = power { t.power_limit_w = p; }
            if let Some(c) = core { t.core_offset_mhz = c; }
            if let Some(m) = mem { t.mem_offset_mhz = m; }
            gpu.try_tune(&pci, &t).await.map_err(daemon_error)?;
            println!(
                "Trying: power {}, core {:+} MHz, memory {:+} MHz.",
                if t.power_limit_w == 0.0 { "default".to_string() } else { format!("{:.0} W", t.power_limit_w) },
                t.core_offset_mhz,
                t.mem_offset_mhz
            );
            if let Some(secs) = test {
                eprintln!("Testing stability for {secs} s (the setting stays on trial meanwhile)…");
                let (stable, what) = gpu.test_tune(&pci, secs).await.map_err(daemon_error)?;
                println!("{what}");
                if !stable {
                    return Ok(());
                }
            }
            eprint!("Keep it? Type y and Enter within 30 seconds (anything else, or nothing, undoes it): ");
            let answer = tokio::time::timeout(std::time::Duration::from_secs(28), tokio::task::spawn_blocking(|| {
                let mut s = String::new();
                let _ = std::io::stdin().read_line(&mut s);
                s
            }))
            .await;
            match answer {
                Ok(Ok(s)) if s.trim().eq_ignore_ascii_case("y") => {
                    gpu.keep_tune(&pci).await.map_err(daemon_error)?;
                    println!("Kept.");
                }
                _ => {
                    gpu.cancel_tune(&pci).await.ok();
                    println!("\nUndone.");
                }
            }
            Ok(())
        }
    }
}

fn print_tune(st: &ug_proto::GpuTuneState) {
    if st.power_limit {
        println!("power limit   {:.0} W (default {:.0}, allowed {:.0}–{:.0} W)", st.power_limit_w, st.power_default_w, st.power_min_w, st.power_max_w);
    }
    if st.core_offset {
        println!("core offset   {:+} MHz (allowed {:+}…{:+})", st.core_offset_mhz, st.core_min_mhz, st.core_max_mhz);
    }
    if st.mem_offset {
        println!("memory offset {:+} MHz (allowed {:+}…{:+})", st.mem_offset_mhz, st.mem_min_mhz, st.mem_max_mhz);
    }
    if !st.applied.is_default() {
        println!("kept by Unix Guardian: {:.0} W, core {:+}, memory {:+}", st.applied.power_limit_w, st.applied.core_offset_mhz, st.applied.mem_offset_mhz);
    }
    if !st.crashed.is_default() {
        println!("NOT applied again: the setting being tried when the machine stopped ({:.0} W, core {:+}, memory {:+})", st.crashed.power_limit_w, st.crashed.core_offset_mhz, st.crashed.mem_offset_mhz);
    }
    for n in &st.notes {
        println!("note: {n}");
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
    use super::{parse_kbps, rate};

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
    fn speeds_read_as_kilobits() {
        assert_eq!(parse_kbps("500k").unwrap(), 500);
        assert_eq!(parse_kbps("2.5M").unwrap(), 2500);
        assert_eq!(parse_kbps("800").unwrap(), 800);
        assert!(parse_kbps("fast").is_err());
    }

    #[test]
    fn rates_pick_a_readable_scale() {
        assert_eq!(rate(0), "0 B/s");
        assert_eq!(rate(512), "512.0 B/s");
        assert_eq!(rate(2048), "2.0 kB/s");
        assert_eq!(rate(5_242_880), "5.0 MB/s");
    }
}
