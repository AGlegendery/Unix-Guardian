use super::*;
use crate::Launcher;
use ug_testfs::{FakeProc, TestFs};

const UID: u32 = 1000;

fn detector(installed: Vec<InstalledGame>) -> Detector {
    let mut d = Detector::new(GameDb::bundled(), installed, UID);
    d.ticks_per_sec = 100; // fixed, so CPU percentages in tests are exact
    d
}

fn secs(t0: Instant, s: u64) -> Instant {
    t0 + Duration::from_secs(s)
}

fn started(events: &[Event]) -> Vec<&RunningGame> {
    events.iter().filter_map(|e| match e { Event::Started(g) => Some(g), _ => None }).collect()
}

fn stopped(events: &[Event]) -> Vec<(&RunningGame, Duration)> {
    events
        .iter()
        .filter_map(|e| match e { Event::Stopped { game, played } => Some((game, *played)), _ => None })
        .collect()
}

#[test]
fn a_steam_game_starts_groups_its_processes_and_stops_after_the_debounce() {
    let fs = TestFs::new();
    let root = SysRoot::at(fs.root());
    let t0 = Instant::now();
    let mut d = detector(vec![]);

    // Steam's reaper and the Proton game both carry SteamAppId.
    fs.process(&FakeProc { pid: 500, comm: "reaper", cmdline: &["reaper", "SteamLaunch", "AppId=1091500"], environ: &[("SteamAppId", "1091500")], ..Default::default() });
    fs.process(&FakeProc { pid: 510, ppid: 500, comm: "Cyberpunk2077.e", cmdline: &[r"Z:\games\Cyberpunk2077.exe"], environ: &[("SteamAppId", "1091500")], ..Default::default() });

    let ev = d.scan(&root, t0);
    let s = started(&ev);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].key, "steam:1091500");
    assert_eq!(s[0].name, "Cyberpunk 2077", "named from the database when Steam's library is absent");
    assert_eq!(s[0].evidence, Evidence::SteamAppId);
    assert_eq!(s[0].pids, vec![510], "reaper is Steam's wrapper, not part of the game");

    // A child process spawned later joins the same game.
    fs.process(&FakeProc { pid: 520, ppid: 510, comm: "CrashReporter", cmdline: &["CrashReporter"], environ: &[("SteamAppId", "1091500")], ..Default::default() });
    assert!(d.scan(&root, secs(t0, 2)).is_empty());
    assert_eq!(d.running()[0].pids, vec![510, 520]);

    // Game exits. Not stopped until the debounce has passed.
    fs.kill(510).kill(520).kill(500);
    assert!(d.scan(&root, secs(t0, 60)).is_empty(), "debounce: launchers hand off between processes");
    let ev = d.scan(&root, secs(t0, 64));
    let st = stopped(&ev);
    assert_eq!(st.len(), 1);
    assert_eq!(st[0].1, Duration::from_secs(60), "played until the last process vanished, not until the debounce ended");
    assert!(d.running().is_empty());
}

#[test]
fn a_launcher_handing_off_to_the_game_is_one_session() {
    let fs = TestFs::new();
    let root = SysRoot::at(fs.root());
    let t0 = Instant::now();
    let mut d = detector(vec![]);

    fs.process(&FakeProc { pid: 600, comm: "launcher.exe", cmdline: &[r"C:\g\launcher.exe"], environ: &[("SteamAppId", "730")], ..Default::default() });
    assert_eq!(started(&d.scan(&root, t0)).len(), 1);
    fs.kill(600);
    assert!(d.scan(&root, secs(t0, 1)).is_empty());
    fs.process(&FakeProc { pid: 601, comm: "cs2", cmdline: &["/g/cs2"], environ: &[("SteamAppId", "730")], ..Default::default() });
    assert!(d.scan(&root, secs(t0, 2)).is_empty(), "no second Started within the debounce");
    assert_eq!(d.running().len(), 1);
}

#[test]
fn the_installed_library_names_a_wine_game_by_its_exe() {
    let fs = TestFs::new();
    let root = SysRoot::at(fs.root());
    let installed = vec![InstalledGame {
        id: "bottles:RedDead__149:7151074f".into(),
        name: "Fears to Fathom - Ironbark Lookout".into(),
        launcher: Launcher::Bottles,
        steam_appid: None,
        exe_names: vec!["fears to fathom - ironbark lookout".into()],
        install_dir: None,
    }];
    let mut d = detector(installed);
    fs.process(&FakeProc {
        pid: 700,
        comm: "Fears to Fathom",
        cmdline: &[r"C:\Program Files (x86)\Fears to Fathom Ironbark Lookout\Fears to Fathom - Ironbark Lookout.exe"],
        ..Default::default()
    });
    let ev = d.scan(&root, Instant::now());
    let s = started(&ev);
    assert_eq!(s[0].key, "bottles:RedDead__149:7151074f");
    assert_eq!(s[0].evidence, Evidence::Library);
}

#[test]
fn the_database_recognises_games_outside_steam() {
    let fs = TestFs::new();
    let root = SysRoot::at(fs.root());
    let mut d = detector(vec![]);
    fs.process(&FakeProc { pid: 10, comm: "eldenring.exe", cmdline: &[r"C:\ER\Game\eldenring.exe"], ..Default::default() });
    fs.process(&FakeProc { pid: 11, comm: "java", cmdline: &["/usr/bin/java", "-Xmx4G", "net.minecraft.client.main.Main", "--username", "x"], ..Default::default() });
    let ev = d.scan(&root, Instant::now());
    let mut keys: Vec<&str> = started(&ev).iter().map(|g| g.key.as_str()).collect();
    keys.sort();
    assert_eq!(keys, vec!["db:eldenring", "db:minecraft-java"]);
}

#[test]
fn a_blocked_game_carries_the_reason_to_show_the_user() {
    let fs = TestFs::new();
    let mut d = detector(vec![]);
    fs.process(&FakeProc { pid: 12, comm: "VALORANT-Win64-", cmdline: &[r"C:\Riot\VALORANT-Win64-Shipping.exe"], ..Default::default() });
    let ev = d.scan(&SysRoot::at(fs.root()), Instant::now());
    assert!(started(&ev)[0].blocked_note.as_deref().unwrap().contains("Vanguard"));
}

#[test]
fn an_unknown_wine_game_is_promoted_only_after_sustained_cpu() {
    let fs = TestFs::new();
    let root = SysRoot::at(fs.root());
    let t0 = Instant::now();
    let mut d = detector(vec![]);
    let game = |cpu: u64| FakeProc {
        pid: 800,
        comm: "IndieGame.exe",
        cmdline: &[r"C:\Indie\IndieGame.exe"],
        maps: &["/pfx/drive_c/windows/system32/d3d11.dll"],
        cpu_ticks: cpu,
        ..Default::default()
    };

    fs.process(&game(0));
    assert!(d.scan(&root, t0).is_empty(), "a candidate, not yet a game");
    fs.process(&game(100));
    assert!(d.scan(&root, secs(t0, 3)).is_empty(), "window not elapsed");
    // 5 s at 100 ticks/s: 400 ticks = 80% of a core.
    fs.process(&game(400));
    let ev = d.scan(&root, secs(t0, 5));
    let s = started(&ev);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].key, "exe:indiegame");
    assert_eq!(s[0].name, "IndieGame");
    assert_eq!(s[0].evidence, Evidence::Heuristic);
}

#[test]
fn an_idle_graphical_program_never_becomes_a_game() {
    let fs = TestFs::new();
    let root = SysRoot::at(fs.root());
    let t0 = Instant::now();
    let mut d = detector(vec![]);
    let tool = |cpu: u64| FakeProc { pid: 801, comm: "Viewer.exe", cmdline: &[r"C:\V\Viewer.exe"], maps: &["/x/d3d9.dll"], cpu_ticks: cpu, ..Default::default() };
    fs.process(&tool(0));
    d.scan(&root, t0);
    for (s, cpu) in [(5, 20), (10, 40), (15, 60)] {
        fs.process(&tool(cpu)); // ~4% of a core
        assert!(d.scan(&root, secs(t0, s)).is_empty());
    }
}

#[test]
fn browsers_and_tools_are_never_games_however_hard_they_work() {
    let fs = TestFs::new();
    let root = SysRoot::at(fs.root());
    let t0 = Instant::now();
    let mut d = detector(vec![]);
    let ff = |cpu| FakeProc { pid: 900, comm: "firefox", cmdline: &["/usr/lib/firefox/firefox"], maps: &["/usr/lib/libvulkan.so.1"], cpu_ticks: cpu, ..Default::default() };
    let blender = |cpu| FakeProc { pid: 901, comm: "blender", cmdline: &["blender"], maps: &["/usr/lib/libvulkan.so.1"], cpu_ticks: cpu, ..Default::default() };
    fs.process(&ff(0)).process(&blender(0));
    d.scan(&root, t0);
    fs.process(&ff(1000)).process(&blender(1000));
    assert!(d.scan(&root, secs(t0, 6)).is_empty());
}

#[test]
fn wine_system_processes_and_installers_are_not_games() {
    let fs = TestFs::new();
    let root = SysRoot::at(fs.root());
    let t0 = Instant::now();
    let mut d = detector(vec![]);
    for (pid, exe) in [(20, r"C:\windows\explorer.exe"), (21, r"C:\windows\system32\services.exe"), (22, r"C:\dl\GameSetup.exe"), (23, r"C:\Epic\EpicGamesLauncher.exe")] {
        fs.process(&FakeProc { pid, comm: "x", cmdline: &[exe], maps: &["/x/d3d11.dll"], cpu_ticks: 0, ..Default::default() });
    }
    d.scan(&root, t0);
    for pid in 20..24 {
        fs.process(&FakeProc { pid, comm: "x", cmdline: &[match pid { 20 => r"C:\windows\explorer.exe", 21 => r"C:\windows\system32\services.exe", 22 => r"C:\dl\GameSetup.exe", _ => r"C:\Epic\EpicGamesLauncher.exe" }], maps: &["/x/d3d11.dll"], cpu_ticks: 1000, ..Default::default() });
    }
    assert!(d.scan(&root, secs(t0, 6)).is_empty());
}

#[test]
fn another_users_games_are_not_ours() {
    let fs = TestFs::new();
    let mut d = detector(vec![]);
    fs.process(&FakeProc { pid: 30, uid: 1001, comm: "cs2", cmdline: &["cs2"], environ: &[("SteamAppId", "730")], ..Default::default() });
    assert!(d.scan(&SysRoot::at(fs.root()), Instant::now()).is_empty());
}

#[test]
fn steam_shader_precompilation_is_not_a_game() {
    let fs = TestFs::new();
    let mut d = detector(vec![]);
    fs.process(&FakeProc { pid: 31, comm: "fossilize_repla", cmdline: &["fossilize_replay"], environ: &[("SteamAppId", "730")], ..Default::default() });
    fs.process(&FakeProc { pid: 32, comm: "steamwebhelper", cmdline: &["steamwebhelper"], environ: &[("SteamAppId", "730")], ..Default::default() });
    fs.process(&FakeProc { pid: 33, comm: "x", cmdline: &["x"], environ: &[("SteamAppId", "1628350")], ..Default::default() });
    assert!(d.scan(&SysRoot::at(fs.root()), Instant::now()).is_empty(), "shader cache, Steam UI and runtime tools");
}

#[test]
fn a_reused_pid_is_a_new_process() {
    let fs = TestFs::new();
    let root = SysRoot::at(fs.root());
    let t0 = Instant::now();
    let mut d = detector(vec![]);
    fs.process(&FakeProc { pid: 40, comm: "cs2", cmdline: &["cs2"], environ: &[("SteamAppId", "730")], start_ticks: 100, ..Default::default() });
    d.scan(&root, t0);
    // Between scans the game exited and an unrelated process got PID 40.
    fs.process(&FakeProc { pid: 40, comm: "bash", cmdline: &["bash"], start_ticks: 999, ..Default::default() });
    d.scan(&root, secs(t0, 1));
    let ev = d.scan(&root, secs(t0, 5));
    assert_eq!(stopped(&ev).len(), 1, "bash on the old PID must not keep CS2 'running'");
}

#[test]
fn gamemode_registration_makes_an_unknown_process_a_game() {
    let fs = TestFs::new();
    let root = SysRoot::at(fs.root());
    let mut d = detector(vec![]);
    fs.process(&FakeProc { pid: 50, comm: "mygame", cmdline: &["/opt/mygame/mygame"], ..Default::default() });
    assert!(d.scan(&root, Instant::now()).is_empty());
    d.register(50);
    let ev = d.scan(&root, Instant::now());
    let s = started(&ev);
    assert_eq!(s[0].key, "exe:mygame");
    assert_eq!(s[0].evidence, Evidence::GameMode);
}

#[test]
fn gamemode_registration_of_a_steam_game_keeps_the_steam_identity() {
    let fs = TestFs::new();
    let mut d = detector(vec![]);
    fs.process(&FakeProc { pid: 51, comm: "cs2", cmdline: &["cs2"], environ: &[("SteamAppId", "730")], ..Default::default() });
    d.register(51);
    let ev = d.scan(&SysRoot::at(fs.root()), Instant::now());
    assert_eq!(started(&ev)[0].key, "steam:730");
}

#[test]
fn dismissing_a_wrong_guess_stops_it_now_and_forever() {
    let fs = TestFs::new();
    let root = SysRoot::at(fs.root());
    let t0 = Instant::now();
    let mut d = detector(vec![]);
    let p = |pid, cpu| FakeProc { pid, comm: "Render.exe", cmdline: &[r"C:\r\Render.exe"], maps: &["/x/d3d12.dll"], cpu_ticks: cpu, ..Default::default() };
    fs.process(&p(60, 0));
    d.scan(&root, t0);
    fs.process(&p(60, 500));
    assert_eq!(started(&d.scan(&root, secs(t0, 5))).len(), 1);

    let ev = d.ignore_exe("Render.exe", secs(t0, 7));
    assert_eq!(stopped(&ev).len(), 1, "tuning must be reverted immediately, not when it exits");
    assert!(d.running().is_empty());

    fs.kill(60).process(&p(61, 0));
    d.scan(&root, secs(t0, 8));
    fs.process(&p(61, 900));
    assert!(d.scan(&root, secs(t0, 14)).is_empty(), "stays dismissed for new instances");
}

#[test]
fn electron_and_cef_helpers_are_never_games_whatever_they_are_called() {
    // Names taken from a real desktop: none of them is in any ignore list.
    const HELPERS: [(i32, &str, &[&str]); 4] = [
        (70, "gapgpt", &["/opt/GapGPT/gapgpt", "--type=gpu-process", "--ozone-platform=wayland"]),
        (71, "desktop", &["/tmp/.mount_desktoCOFAbJ/desktop", "--type=gpu-process"]),
        (72, "claude-desktop", &["/usr/lib/claude-desktop/claude-desktop", "--type=gpu-process"]),
        (73, "EpicWebHelper.", &[r"C:\Epic\EpicWebHelper.exe", "--type=renderer"]),
    ];
    let fs = TestFs::new();
    let root = SysRoot::at(fs.root());
    let t0 = Instant::now();
    let mut d = detector(vec![]);
    for cpu in [0, 2000] {
        for (pid, comm, cmdline) in HELPERS {
            fs.process(&FakeProc {
                pid,
                comm,
                cmdline,
                maps: &["/usr/lib/libvulkan.so.1", "/x/d3d11.dll"],
                cpu_ticks: cpu,
                ..Default::default()
            });
        }
        let at = if cpu == 0 { t0 } else { secs(t0, 6) };
        assert!(d.scan(&root, at).is_empty());
    }
}

#[test]
fn chromium_helpers_with_a_rewritten_process_title_are_not_games() {
    // Chromium overwrites its argv, so /proc/<pid>/cmdline is ONE string with
    // spaces rather than NUL-separated arguments. Shape copied from the Claude
    // desktop app's GPU process on the dev machine, which the heuristic took
    // for a game while it was busy rendering.
    const TITLE: &str = "/usr/lib/claude-desktop/claude-desktop --type=gpu-process --ozone-platform=wayland \
        --user-data-dir=/home/u/.config/Claude --gpu-preferences=YAAAAAAAAAAgAQAEAAAAAAAAAAAAAGAA";
    let fs = TestFs::new();
    let root = SysRoot::at(fs.root());
    let t0 = Instant::now();
    let mut d = detector(vec![]);
    for (cpu, at) in [(0, t0), (5000, secs(t0, 6))] {
        fs.process(&FakeProc {
            pid: 90,
            comm: "claude-desktop",
            cmdline: &[TITLE],
            exe: Some("/usr/lib/claude-desktop/claude-desktop"),
            maps: &["/usr/lib/libvulkan.so.1"],
            cpu_ticks: cpu,
            ..Default::default()
        });
        assert!(d.scan(&root, at).is_empty());
    }
}

#[test]
fn a_native_program_is_named_after_its_executable_not_its_title() {
    let fs = TestFs::new();
    fs.process(&FakeProc {
        pid: 91,
        comm: "MainThread",
        cmdline: &["some rewritten title --with=/odd/paths inside"],
        exe: Some("/opt/game/bin/realgame"),
        ..Default::default()
    });
    let p = crate::proc::read(&SysRoot::at(fs.root()), 91).unwrap();
    assert_eq!(p.exe_basename(), "realgame");
}
