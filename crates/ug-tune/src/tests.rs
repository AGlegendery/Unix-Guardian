use super::*;
use units::FakeUnits;
use ug_testfs::{FakeProc, TestFs};

const UID: u32 = 1000;

/// A laptop-shaped tree: two intel_pstate policies and the three sysctls,
/// holding the user's own non-default values.
fn machine() -> TestFs {
    let fs = TestFs::new();
    for n in 0..2 {
        let d = format!("sys/devices/system/cpu/cpufreq/policy{n}");
        fs.write(format!("{d}/scaling_governor"), "powersave");
        fs.write(format!("{d}/scaling_available_governors"), "performance powersave");
        fs.write(format!("{d}/energy_performance_preference"), "balance_power");
    }
    fs.write("proc/sys/vm/swappiness", "30");
    fs.write("proc/sys/vm/max_map_count", "65530");
    fs.write("proc/sys/vm/compaction_proactiveness", "20");
    fs
}

fn tuner(fs: &TestFs, units: FakeUnits) -> Tuner<FakeUnits> {
    Tuner::new(SysRoot::at(fs.root()), fs.root().join("var/lib/ug/gamemode.json"), units).0
}

fn get(fs: &TestFs, rel: &str) -> String {
    std::fs::read_to_string(fs.root().join(rel)).unwrap().trim().to_string()
}

fn everything() -> Request {
    Request { cpu_performance: true, cpu_no_idle_states: false, cpu_priority: true, memory_tuning: true, pause_services: true, network_tuning: false, pids: vec![] }
}

#[test]
fn enter_applies_and_leave_restores_the_users_own_values() {
    let fs = machine();
    let mut t = tuner(&fs, FakeUnits::default());

    let r = t.enter(&everything(), UID);
    assert_eq!(get(&fs, "sys/devices/system/cpu/cpufreq/policy1/scaling_governor"), "performance");
    assert_eq!(get(&fs, "sys/devices/system/cpu/cpufreq/policy1/energy_performance_preference"), "performance");
    assert_eq!(get(&fs, "proc/sys/vm/swappiness"), "10");
    assert_eq!(get(&fs, "proc/sys/vm/max_map_count"), "2147483642");
    assert!(r.applied.iter().any(|l| l.contains("was powersave/balance_power")), "{r:?}");

    t.leave();
    assert_eq!(get(&fs, "sys/devices/system/cpu/cpufreq/policy1/scaling_governor"), "powersave");
    assert_eq!(get(&fs, "sys/devices/system/cpu/cpufreq/policy1/energy_performance_preference"), "balance_power");
    assert_eq!(get(&fs, "proc/sys/vm/swappiness"), "30", "the user's 30, not the kernel default 60");
    assert_eq!(get(&fs, "proc/sys/vm/compaction_proactiveness"), "20");
    assert!(!t.active());
}

#[test]
fn a_higher_map_count_is_never_lowered() {
    let fs = machine();
    fs.write("proc/sys/vm/max_map_count", "2147483647");
    let mut t = tuner(&fs, FakeUnits::default());
    t.enter(&everything(), UID);
    assert_eq!(get(&fs, "proc/sys/vm/max_map_count"), "2147483647");
}

#[test]
fn only_listed_background_units_that_were_running_are_paused_and_resumed() {
    let fs = machine();
    let units = FakeUnits::with(&["apt-daily.timer", "fstrim.timer", "sshd.service"]);
    let mut t = tuner(&fs, units);

    let r = t.enter(&everything(), UID);
    assert!(r.applied.iter().any(|l| l.contains("apt-daily.timer") && l.contains("fstrim.timer")));
    {
        let active = t.units.active.borrow();
        assert!(!active.contains("apt-daily.timer"));
        assert!(active.contains("sshd.service"), "never touch anything not on the list");
    }
    t.leave();
    let active = t.units.active.borrow();
    assert!(active.contains("apt-daily.timer") && active.contains("fstrim.timer"));
    assert!(!active.contains("man-db.timer"), "a unit that was not running must not be started");
}

#[test]
fn package_manager_services_are_never_on_the_pause_list() {
    for u in BACKGROUND_UNITS {
        assert!(
            !matches!(*u, "apt-daily.service" | "apt-daily-upgrade.service" | "unattended-upgrades.service" | "packagekit.service" | "dnf-automatic.service"),
            "{u}: stopping a package operation mid-run can break the system"
        );
    }
}

#[test]
fn a_crash_mid_game_is_undone_on_the_next_start() {
    let fs = machine();
    let state = fs.root().join("var/lib/ug/gamemode.json");
    {
        let mut t = tuner(&fs, FakeUnits::with(&["apt-daily.timer"]));
        t.enter(&everything(), UID);
        assert!(state.exists(), "undo list is on disk while game mode is on");
        // Dropped without leave(): the daemon was killed.
    }
    assert_eq!(get(&fs, "proc/sys/vm/swappiness"), "10");

    let units = FakeUnits::default();
    let (t, report) = Tuner::new(SysRoot::at(fs.root()), state.clone(), units);
    assert_eq!(get(&fs, "proc/sys/vm/swappiness"), "30");
    assert_eq!(get(&fs, "sys/devices/system/cpu/cpufreq/policy0/scaling_governor"), "powersave");
    assert!(t.units.active.borrow().contains("apt-daily.timer"), "the paused timer is running again");
    assert!(report.applied.iter().any(|l| l.contains("restored")));
    let left: Persisted = serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
    assert!(left.undo.is_empty());
}

#[test]
fn a_second_game_does_not_stack_changes() {
    let fs = machine();
    let mut t = tuner(&fs, FakeUnits::default());
    t.enter(&everything(), UID);
    let n = t.undo.len();
    t.enter(&everything(), UID);
    assert_eq!(t.undo.len(), n);
    t.leave();
    assert_eq!(get(&fs, "proc/sys/vm/swappiness"), "30");
}

#[test]
fn someone_elses_process_is_refused() {
    let fs = machine();
    fs.process(&FakeProc { pid: 77, uid: 1001, comm: "game", ..Default::default() });
    let mut t = tuner(&fs, FakeUnits::default());
    let r = t.enter(&Request { cpu_priority: true, pids: vec![77], ..Default::default() }, UID);
    assert!(r.skipped.iter().any(|l| l.contains("does not belong to you")));
}

#[test]
fn the_games_autogroup_is_boosted_and_restored_through_a_surviving_member() {
    let fs = machine();
    fs.process(&FakeProc { pid: 80, comm: "game", ..Default::default() });
    fs.process(&FakeProc { pid: 81, comm: "launcher", ..Default::default() });
    fs.write("proc/80/autogroup", "/autogroup-42 nice 0");
    fs.write("proc/81/autogroup", "/autogroup-42 nice 0");
    let mut t = tuner(&fs, FakeUnits::default());
    t.enter(&Request { cpu_priority: true, pids: vec![80], ..Default::default() }, UID);
    assert_eq!(get(&fs, "proc/80/autogroup"), "-10");

    fs.kill(80); // the game exits; its launcher lives on in the same group
    t.leave();
    assert_eq!(get(&fs, "proc/81/autogroup"), "0");
}

#[test]
fn a_restore_that_fails_is_kept_for_the_next_attempt() {
    let fs = machine();
    let mut t = tuner(&fs, FakeUnits::default());
    t.enter(&Request { memory_tuning: true, ..Default::default() }, UID);
    // Make the restore impossible: the file has become a directory.
    let p = fs.root().join("proc/sys/vm/swappiness");
    std::fs::remove_file(&p).unwrap();
    std::fs::create_dir(&p).unwrap();

    let r = t.leave();
    assert!(!r.skipped.is_empty());
    assert_eq!(t.undo.len(), 1, "forgetting it would leave the change in place forever");
}

#[test]
fn a_machine_without_cpufreq_says_so() {
    let fs = TestFs::new();
    let mut t = tuner(&fs, FakeUnits::default());
    let r = t.enter(&Request { cpu_performance: true, ..Default::default() }, UID);
    assert!(r.skipped.iter().any(|l| l.contains("no cpufreq")));
}

#[test]
fn leaving_when_nothing_is_on_does_nothing() {
    let fs = machine();
    let mut t = tuner(&fs, FakeUnits::default());
    assert_eq!(t.leave(), Report::default());
}

#[test]
fn a_change_that_cannot_be_made_is_reported_not_claimed() {
    use std::os::unix::fs::PermissionsExt;
    let fs = machine();
    for rel in ["proc/sys/vm/swappiness", "proc/sys/vm/max_map_count", "proc/sys/vm/compaction_proactiveness",
                "sys/devices/system/cpu/cpufreq/policy0/scaling_governor", "sys/devices/system/cpu/cpufreq/policy1/scaling_governor"] {
        std::fs::set_permissions(fs.root().join(rel), std::fs::Permissions::from_mode(0o444)).unwrap();
    }
    // Root (CI containers) writes read-only files anyway: nothing to test.
    if std::fs::OpenOptions::new().write(true).open(fs.root().join("proc/sys/vm/swappiness")).is_ok() {
        return;
    }
    let mut t = tuner(&fs, FakeUnits::default());
    let r = t.enter(&Request { cpu_performance: true, memory_tuning: true, ..Default::default() }, UID);
    assert!(r.applied.is_empty(), "nothing was changed, so nothing may be reported as done: {r:?}");
    assert!(r.skipped.iter().all(|l| l.contains("must run as root")), "{r:?}");
    assert!(t.undo.is_empty(), "no phantom undo entries left to 'restore' later");
}

#[test]
fn network_tuning_is_applied_and_every_value_comes_back() {
    let fs = machine();
    fs.write("proc/sys/net/ipv4/tcp_fastopen", "1");
    fs.write("proc/sys/net/ipv4/tcp_notsent_lowat", "4294967295");
    fs.write("proc/sys/net/ipv4/tcp_congestion_control", "cubic");
    fs.dir("sys/class/net/wlan0/wireless").dir("sys/class/net/eth0");
    let units = FakeUnits::default();
    units.wifi_saving.borrow_mut().insert("wlan0".into());
    let mut t = tuner(&fs, units);
    let req = Request { network_tuning: true, ..Default::default() };
    let r = t.enter(&req, UID);
    assert_eq!(get(&fs, "proc/sys/net/ipv4/tcp_fastopen"), "3");
    assert_eq!(get(&fs, "proc/sys/net/ipv4/tcp_notsent_lowat"), "16384");
    assert_eq!(get(&fs, "proc/sys/net/ipv4/tcp_congestion_control"), "bbr");
    assert!(t.units.wifi_saving.borrow().is_empty(), "power saving off on wlan0");
    assert!(r.applied.iter().any(|l| l.contains("Wi-Fi power saving off on wlan0")), "{r:?}");

    t.leave();
    assert_eq!(get(&fs, "proc/sys/net/ipv4/tcp_fastopen"), "1");
    assert_eq!(get(&fs, "proc/sys/net/ipv4/tcp_notsent_lowat"), "4294967295");
    assert_eq!(get(&fs, "proc/sys/net/ipv4/tcp_congestion_control"), "cubic");
    assert!(t.units.wifi_saving.borrow().contains("wlan0"), "power saving back on");
}

#[test]
fn wifi_power_saving_already_off_is_left_as_it_is() {
    let fs = machine();
    fs.dir("sys/class/net/wlan0/wireless");
    let mut t = tuner(&fs, FakeUnits::default());
    t.enter(&Request { network_tuning: true, ..Default::default() }, UID);
    t.leave();
    assert!(t.units.wifi_saving.borrow().is_empty(), "not turned on by the restore");
}

#[test]
fn a_crash_during_the_game_restores_network_settings_on_the_next_start() {
    let fs = machine();
    fs.write("proc/sys/net/ipv4/tcp_fastopen", "1");
    fs.dir("sys/class/net/wlan0/wireless");
    let units = FakeUnits::default();
    units.wifi_saving.borrow_mut().insert("wlan0".into());
    let mut t = tuner(&fs, units);
    t.enter(&Request { network_tuning: true, ..Default::default() }, UID);
    std::mem::forget(t); // the daemon dies without leaving
    let units = FakeUnits::default();
    let (t2, _) = Tuner::new(SysRoot::at(fs.root()), fs.root().join("var/lib/ug/gamemode.json"), units);
    assert_eq!(get(&fs, "proc/sys/net/ipv4/tcp_fastopen"), "1");
    assert!(t2.units.wifi_saving.borrow().contains("wlan0"));
}
