//! Unix Guardian's window.
//!
//! Optional by design: the daemon and agent do all the work, and the user's
//! condition for a web-based UI was that it never costs RAM or frame time while
//! they play. So closing the window ends this process entirely.
//!
//! The roadmap kept a tray icon alive in this process after the window closed.
//! Measured on the development laptop, that "closed" state still held 124 MB
//! (GTK and WebKit stay mapped once initialised, and WebKit's network process
//! stays running), so it is gone: a tray icon belongs in the 5 MB agent, which
//! runs anyway and can start this window on demand.

mod bus;
mod testlab;

/// Sets an environment variable unless the user already chose a value.
fn default_env(key: &str, value: &str) {
    if std::env::var_os(key).is_none() {
        std::env::set_var(key, value);
    }
}

fn main() {
    // Rendering is tuned for memory, the user's condition for a web-based UI.
    // Measured on the development laptop (dashboard at 1 Hz, PSS of all three
    // processes): 307 MB by default, 281 MB with only Mesa's EGL loaded (GLVND
    // otherwise maps NVIDIA's EGL libraries into both processes even when the
    // integrated GPU renders), 246 MB with WebKit's GPU compositing and GTK's
    // GL also off, at the same 3 % of one core — the pages have no animation
    // for a GPU to help with. WebKitGTK cannot run with no EGL at all (the web
    // process aborts), so this is the floor. All of it is set before GTK or
    // WebKit start, while the process is single-threaded.
    if nvidia_drives_display() {
        // The DMA-BUF renderer shows a blank or flickering window when the
        // NVIDIA proprietary driver renders the desktop.
        default_env("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    } else {
        let mesa = "/usr/share/glvnd/egl_vendor.d/50_mesa.json";
        if std::path::Path::new(mesa).exists() {
            default_env("__EGL_VENDOR_LIBRARY_FILENAMES", mesa);
        }
        default_env("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
        default_env("GDK_GL", "disable");
    }
    let daemon_on_session = std::env::var_os("UG_SESSION").is_some_and(|v| v == "1");

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            use tauri::Manager;
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        // `ug-app --open monitoring/thermal/live` opens straight at a page,
        // tab and range (the same path the browser preview takes after `#`).
        .setup(|app| {
            use tauri::Manager;
            let args: Vec<String> = std::env::args().collect();
            let open = args.iter().position(|a| a == "--open").and_then(|i| args.get(i + 1)).cloned();
            // 900 px tall fits the whole dashboard at the default width;
            // on a shorter screen, open no taller than its work area (less
            // a title bar's worth). Wayland may not name the monitor yet;
            // the compositor then keeps the window on screen itself.
            if let Some(w) = app.get_webview_window("main") {
                if let (Ok(Some(m)), Ok(size)) = (w.current_monitor(), w.inner_size()) {
                    let room = m.work_area().size.height.saturating_sub((40.0 * m.scale_factor()) as u32);
                    if size.height > room && room > 0 {
                        let _ = w.set_size(tauri::PhysicalSize::new(size.width, room));
                    }
                }
            }
            testlab::recover_in_background();
            if let (Some(path), Some(w)) = (open, app.get_webview_window("main")) {
                let path: String = path.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '/' || *c == '-').collect();
                let _ = w.eval(&format!("if (location.hash !== '#{path}') {{ location.hash = '{path}'; location.reload(); }}"));
            }
            Ok(())
        })
        .manage(bus::Bus::new(daemon_on_session))
        .invoke_handler(tauri::generate_handler![
            bus::overview,
            bus::snapshot,
            bus::monitor_start,
            bus::monitor_stop,
            bus::gamemode_active,
            bus::games_running,
            bus::games_installed,
            bus::profiles,
            bus::set_game_profile,
            bus::set_profile_option,
            bus::game_mode_hand,
            bus::set_game_mode,
            bus::set_pause_apps,
            bus::create_profile,
            bus::delete_profile,
            bus::playtime,
            bus::playtime_sessions,
            bus::playtime_forget,
            bus::playtime_export,
            bus::playtime_tracked_apps,
            bus::steam_launch_options,
            bus::steam_preview_launch_options,
            bus::steam_set_launch_options,
            bus::steam_undo_launch_options,
            bus::playtime_track_app,
            bus::playtime_untrack_app,
            bus::playtime_running_programs,
            bus::playtime_move_to_apps,
            bus::playtime_import_steam,
            bus::playtime_imported,
            bus::playtime_forget_imported,
            bus::power_caps,
            bus::power_now,
            bus::power_schedule,
            bus::power_jobs,
            bus::power_cancel,
            bus::wake_alarm,
            bus::set_wake_alarm,
            bus::clear_wake_alarm,
            bus::power_when,
            bus::power_watches,
            bus::power_unwatch,
            bus::power_usage,
            bus::battery,
            bus::set_battery_care,
            bus::sleep_settings,
            bus::battery_report,
            bus::set_sleep_mode,
            bus::set_button_action,
            bus::history,
            bus::traffic,
            bus::traffic_buckets,
            bus::app_traffic,
            bus::reset_traffic,
            bus::traffic_reset_at,
            bus::extra,
            bus::fps,
            bus::set_up_fps,
            bus::ping_recent,
            bus::ping_compare,
            bus::net_status,
            bus::net_apps,
            bus::net_rules,
            bus::net_set_rule,
            bus::net_background,
            bus::net_set_background,
            bus::net_remove_rule,
            bus::overlay_state,
            bus::overlay_set,
            bus::overlay_set_up,
            bus::shortcuts_state,
            bus::shortcuts_set_enabled,
            bus::shortcuts_run,
            bus::shortcuts_set_keys,
            bus::shortcuts_desktop_keys,
            bus::shortcuts_set_up_desktop_keys,
            bus::settings_state,
            bus::settings_set,
            bus::update_state,
            bus::update_check,
            bus::update_install,
            bus::update_rollback,
            bus::tray_hide,
            bus::driver_diagnose,
            bus::driver_plan,
            bus::driver_apply,
            bus::driver_ops,
            bus::driver_rollback,
            bus::driver_keep,
            bus::recorder_state,
            bus::recorder_set,
            bus::recorder_do,
            bus::fps_sessions,
            bus::forget_fps_sessions,
            bus::gpu_offload,
            bus::gpu_default,
            bus::tray_visible,
            bus::tray_show,
            bus::set_gpu_default,
            bus::gpu_tune_state,
            bus::gpu_try_tune,
            bus::gpu_keep_tune,
            bus::gpu_test_tune,
            bus::gpu_auto_tune,
            bus::gpu_auto_tune_status,
            bus::gpu_stop_auto_tune,
            bus::gpu_cancel_tune,
            bus::gpu_reset_tune,
            bus::cooling_state,
            bus::set_fan_curve,
            bus::set_fan_auto,
            bus::set_fan_mode,
            bus::power_profile,
            bus::set_power_profile,
            testlab::testlab_run,
            testlab::testlab_recover,
            testlab::testlab_save,
            open_terminal_interface,
            report_bug,
        ])
        .run(tauri::generate_context!())
        .expect("running the Unix Guardian window");
}

/// Opens the user's mail program with a bug report addressed and started:
/// versions filled in, nothing sent until they send it.
#[tauri::command]
fn report_bug(context: String) -> Result<(), String> {
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    let os = std::fs::read_to_string("/etc/os-release")
        .unwrap_or_default()
        .lines()
        .find_map(|l| {
            l.strip_prefix("PRETTY_NAME=")
                .map(|v| v.trim_matches('"').to_string())
        })
        .unwrap_or_default();
    let body = format!(
        "What I did:\n\nWhat happened:\n\nWhat I expected:\n\n---\nUnix Guardian {}\n{}\nLinux {}\n{}",
        env!("CARGO_PKG_VERSION"),
        os,
        kernel.trim(),
        context
    );
    let enc = |s: &str| -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect()
    };
    let url = format!(
        "mailto:{}?subject={}&body={}",
        ug_core::BUG_REPORT_EMAIL,
        enc("Unix Guardian bug report"),
        enc(&body)
    );
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|e| {
            format!(
                "Could not open a mail program ({e}). Write to {} instead.",
                ug_core::BUG_REPORT_EMAIL
            )
        })
}

/// Opens `ug tui` in the user's terminal and closes this window completely.
/// Tries the desktop's own terminal first, then common ones.
#[tauri::command]
fn open_terminal_interface(app: tauri::AppHandle) -> Result<(), String> {
    let ug = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("ug")))
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "ug".into());
    let mut candidates: Vec<(String, Vec<String>)> = Vec::new();
    if let Ok(t) = std::env::var("TERMINAL") {
        candidates.push((t, vec!["-e".into()]));
    }
    for (t, flag) in [
        ("konsole", "-e"),
        ("gnome-terminal", "--"),
        ("kgx", "--"),
        ("xfce4-terminal", "-x"),
        ("kitty", "--"),
        ("alacritty", "-e"),
        ("wezterm", "start"),
        ("foot", "--"),
        ("x-terminal-emulator", "-e"),
        ("xterm", "-e"),
    ] {
        candidates.push((t.into(), vec![flag.into()]));
    }
    for (term, mut args) in candidates {
        args.extend([ug.clone(), "tui".into()]);
        if std::process::Command::new(&term)
            .args(&args)
            .spawn()
            .is_ok()
        {
            app.exit(0);
            return Ok(());
        }
    }
    Err("No terminal emulator was found. Open a terminal and run: ug tui".into())
}

/// True when the GPU the firmware booted on (`boot_vga`) is an NVIDIA card,
/// i.e. NVIDIA renders the desktop. Reads sysfs only; never touches the GPU.
fn nvidia_drives_display() -> bool {
    let Ok(cards) = std::fs::read_dir("/sys/class/drm") else {
        return false;
    };
    cards.filter_map(|e| e.ok()).any(|e| {
        let dev = e.path().join("device");
        let boot = std::fs::read_to_string(dev.join("boot_vga")).is_ok_and(|s| s.trim() == "1");
        let vendor =
            std::fs::read_to_string(dev.join("vendor")).is_ok_and(|s| s.trim() == "0x10de");
        boot && vendor
    })
}
