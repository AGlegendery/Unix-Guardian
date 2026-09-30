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
            bus::playtime,
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
        ])
        .run(tauri::generate_context!())
        .expect("running the Unix Guardian window");
}

/// True when the GPU the firmware booted on (`boot_vga`) is an NVIDIA card,
/// i.e. NVIDIA renders the desktop. Reads sysfs only; never touches the GPU.
fn nvidia_drives_display() -> bool {
    let Ok(cards) = std::fs::read_dir("/sys/class/drm") else { return false };
    cards.filter_map(|e| e.ok()).any(|e| {
        let dev = e.path().join("device");
        let boot = std::fs::read_to_string(dev.join("boot_vga")).is_ok_and(|s| s.trim() == "1");
        let vendor = std::fs::read_to_string(dev.join("vendor")).is_ok_and(|s| s.trim() == "0x10de");
        boot && vendor
    })
}
