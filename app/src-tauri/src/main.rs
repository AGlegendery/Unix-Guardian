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

fn main() {
    // WebKitGTK's DMA-BUF renderer shows a blank or flickering window with the
    // NVIDIA proprietary driver when NVIDIA renders the desktop. On hybrid
    // laptops the integrated GPU renders it and the renderer works (and costs
    // less CPU: measured 2 % vs 5 % of a core at one update a second), so it is
    // only disabled when an NVIDIA GPU drives the display.
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() && nvidia_drives_display() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
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
