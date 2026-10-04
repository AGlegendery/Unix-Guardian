//! The in-game overlay: MangoHud, styled and switched by Unix Guardian.
//!
//! The user's choices live in `overlay.json`; from them the agent writes its
//! marked block in `~/.config/MangoHud/MangoHud.conf`, and MangoHud re-reads
//! that file while a game runs, so changes show without a restart. "Only
//! while gaming" works the same way: when a game starts or stops, the block
//! is rewritten with or without `no_display`.
//!
//! Only what MangoHud can draw is offered: the layout is built from its own
//! options, and the window's preview imitates it rather than promising more.

use crate::fps::Fps;
use std::path::PathBuf;
use std::sync::Mutex;
use ug_proto::{OverlaySettings, OverlayState};

pub const SHOWS: [&str; 3] = ["gaming", "always", "off"];
pub const LAYOUTS: [&str; 2] = ["card", "compact"];
pub const POSITIONS: [&str; 5] = ["top-left", "top-right", "bottom-left", "bottom-right", "top-center"];
pub const SIZES: [&str; 3] = ["small", "medium", "large"];

/// Unix Guardian's purple and the chart colours, as MangoHud's hex.
const ACCENT: &str = "8B5CF6";
const CPU: &str = "8F6AF2";
const GPU: &str = "C98500";

pub struct Overlay {
    pub fps: Fps,
    settings_path: PathBuf,
    settings: Mutex<OverlaySettings>,
    /// The last "game running" this block was written for.
    game_running: Mutex<bool>,
    /// Shown or hidden by hand (the global shortcut) until the next game
    /// starts or stops; `None` follows the settings.
    forced: Mutex<Option<bool>>,
}

/// Checks a settings value against its choices.
fn one_of(what: &str, v: &str, choices: &[&str]) -> Result<(), String> {
    if choices.contains(&v) {
        Ok(())
    } else {
        Err(format!("unknown {what} {v:?}; choose one of {}", choices.join(", ")))
    }
}

pub fn validate(s: &OverlaySettings) -> Result<(), String> {
    one_of("show setting", &s.show, &SHOWS)?;
    one_of("layout", &s.layout, &LAYOUTS)?;
    one_of("position", &s.position, &POSITIONS)?;
    one_of("size", &s.size, &SIZES)?;
    if s.opacity_pct > 100 {
        return Err("opacity is 0 to 100".into());
    }
    // MangoHud key names: letters, digits, `_` and `+`, e.g. Alt_L+o, Shift_R+F12.
    let k = s.toggle_key.trim();
    if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '+') {
        return Err(format!("{k:?} is not a key MangoHud understands (e.g. Alt_L+o or Shift_R+F12)"));
    }
    Ok(())
}

/// MangoHud's configuration lines for these settings. `visible` false adds
/// `no_display` (the overlay starts hidden; the toggle key still shows it).
pub fn block(s: &OverlaySettings, logging: &str, visible: bool) -> String {
    let mut l: Vec<String> = Vec::new();
    let font = match s.size.as_str() {
        "small" => 18,
        "large" => 30,
        _ => 24,
    };
    l.push(format!("position={}", s.position));
    l.push(format!("font_size={font}"));
    l.push(format!("background_alpha={:.2}", s.opacity_pct as f64 / 100.0));
    l.push(format!("round_corners={}", if s.layout == "card" { 10 } else { 6 }));
    l.push(format!("toggle_hud={}", s.toggle_key.trim()));
    l.push("background_color=16141F".into());
    l.push("text_color=ECEAF6".into());
    l.push(format!("engine_color={ACCENT}"));
    l.push(format!("cpu_color={CPU}"));
    l.push(format!("gpu_color={GPU}"));
    l.push(format!("frametime_color={ACCENT}"));
    if s.layout == "compact" {
        // One line: FPS, frame time and both temperatures.
        l.extend(["horizontal", "hud_compact", "fps", "frametime=0", "cpu_temp", "gpu_temp", "cpu_stats=0", "gpu_stats=0"].map(String::from));
    } else {
        l.extend(
            ["table_columns=3", "fps", "frametime", "frame_timing", "cpu_stats", "cpu_temp", "cpu_power", "gpu_stats", "gpu_temp", "gpu_power", "vram", "ram"]
                .map(String::from),
        );
    }
    if !visible {
        l.push("no_display".into());
    }
    let mut out = l.join("\n");
    out.push('\n');
    out.push_str(logging);
    out
}

impl Overlay {
    pub fn new(fps: Fps, ug_config: &std::path::Path) -> Overlay {
        let settings_path = ug_config.join("overlay.json");
        let settings = std::fs::read_to_string(&settings_path)
            .ok()
            .and_then(|s| serde_json::from_str::<OverlaySettings>(&s).ok())
            .filter(|s| validate(s).is_ok())
            .unwrap_or_default();
        Overlay {
            fps,
            settings_path,
            settings: Mutex::new(settings),
            game_running: Mutex::new(false),
            forced: Mutex::new(None),
        }
    }

    pub fn settings(&self) -> OverlaySettings {
        self.settings.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn visible(s: &OverlaySettings, gaming: bool) -> bool {
        s.show == "always" || (s.show == "gaming" && gaming)
    }

    pub fn state(&self) -> OverlayState {
        let gaming = *self.game_running.lock().unwrap_or_else(|e| e.into_inner());
        OverlayState {
            settings: self.settings(),
            installed: self.fps.installed(),
            set_up: self.fps.set_up(),
            install_hint: self.fps.install_hint(),
            game_running: gaming,
        }
    }

    /// Whether the overlay shows now: by hand, else by the settings.
    fn visible_now(&self) -> bool {
        let forced = *self.forced.lock().unwrap_or_else(|e| e.into_inner());
        let gaming = *self.game_running.lock().unwrap_or_else(|e| e.into_inner());
        forced.unwrap_or_else(|| Self::visible(&self.settings(), gaming))
    }

    /// Writes the block for the current settings (only if set up).
    fn write(&self) -> Result<(), String> {
        if !self.fps.set_up() {
            return Ok(());
        }
        let s = self.settings();
        self.fps.write_block(Some(&block(&s, &self.fps.logging_lines(), self.visible_now())))
    }

    /// Shows the overlay if hidden, hides it if shown, until the next game
    /// starts or stops (the global shortcut). MangoHud re-reads its file, so
    /// a running game follows. Returns whether it shows now.
    pub fn toggle(&self) -> Result<bool, String> {
        if !self.fps.set_up() {
            return Err("The overlay is not set up yet: set it up on the Overlay page or with `ug overlay setup`.".into());
        }
        let show = !self.visible_now();
        *self.forced.lock().unwrap_or_else(|e| e.into_inner()) = Some(show);
        self.write()?;
        Ok(show)
    }

    pub fn set(&self, s: OverlaySettings) -> Result<(), String> {
        validate(&s)?;
        if let Some(d) = self.settings_path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(&s).expect("settings serialize");
        std::fs::write(&self.settings_path, json).map_err(|e| format!("{}: {e}", self.settings_path.display()))?;
        *self.settings.lock().unwrap_or_else(|e| e.into_inner()) = s;
        self.write()
    }

    /// Adds (or removes) our MangoHud block. MangoHud is loaded per game
    /// (Steam launch options, `mangohud <game>`), never with `MANGOHUD=1`
    /// for the whole session: that reaches every Vulkan program, GTK 4
    /// apps included (see `ug_core::session_env`).
    pub fn set_up(&self, on: bool) -> Result<(), String> {
        if on {
            let s = self.settings();
            self.fps.write_block(Some(&block(&s, &self.fps.logging_lines(), self.visible_now())))
        } else {
            self.fps.write_block(None)
        }
    }

    /// Called every tick: when a game starts or stops, "only while gaming"
    /// shows or hides the overlay. Writes only on a change.
    pub fn follow(&self, gaming: bool) {
        {
            let mut g = self.game_running.lock().unwrap_or_else(|e| e.into_inner());
            if *g == gaming {
                return;
            }
            *g = gaming;
        }
        // A game starting or stopping ends a choice made by hand.
        let was_forced = self.forced.lock().unwrap_or_else(|e| e.into_inner()).take().is_some();
        if self.settings().show == "gaming" || was_forced {
            if let Err(e) = self.write() {
                tracing::warn!(error = %e, "could not update the overlay");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Overlay) {
        let d = tempfile::tempdir().unwrap();
        let fps = Fps {
            folder: d.path().join("data/fps"),
            config: d.path().join("config/MangoHud/MangoHud.conf"),
            root: d.path().join("root"),
        };
        std::fs::create_dir_all(d.path().join("root/usr/share/vulkan/implicit_layer.d")).unwrap();
        std::fs::write(d.path().join("root/usr/share/vulkan/implicit_layer.d/MangoHud.json"), "{}").unwrap();
        let o = Overlay::new(fps, &d.path().join("config/unix-guardian"));
        (d, o)
    }

    fn conf(o: &Overlay) -> String {
        std::fs::read_to_string(&o.fps.config).unwrap_or_default()
    }

    #[test]
    fn the_card_layout_logs_every_frame_and_uses_our_colours() {
        let b = block(&OverlaySettings::default(), "output_folder=/x\nautostart_log=1\nlog_interval=0\n", true);
        for want in ["position=top-left", "font_size=24", "background_alpha=0.50", "toggle_hud=Alt_L+o", "frame_timing", "gpu_temp", "engine_color=8B5CF6", "log_interval=0"] {
            assert!(b.lines().any(|l| l == want), "missing {want}");
        }
        assert!(!b.contains("no_display"));
        let c = block(&OverlaySettings { layout: "compact".into(), size: "large".into(), ..Default::default() }, "", false);
        assert!(c.contains("horizontal") && c.contains("font_size=30") && c.contains("no_display"));
        assert!(!c.contains("frame_timing"));
    }

    #[test]
    fn only_while_gaming_shows_the_overlay_when_a_game_starts_and_hides_it_after() {
        let (_d, o) = fixture();
        o.set_up(true).unwrap();
        assert!(conf(&o).contains("no_display"), "no game: hidden");
        o.follow(true);
        assert!(!conf(&o).contains("no_display"), "game running: shown");
        o.follow(false);
        assert!(conf(&o).contains("no_display"));
    }

    #[test]
    fn the_shortcut_shows_or_hides_it_until_the_next_game_starts_or_stops() {
        let (_d, o) = fixture();
        assert!(o.toggle().unwrap_err().contains("not set up"));
        o.set_up(true).unwrap();
        o.follow(true);
        assert!(!conf(&o).contains("no_display"));
        assert!(!o.toggle().unwrap(), "hidden mid-game");
        assert!(conf(&o).contains("no_display"));
        assert!(o.toggle().unwrap(), "and shown again");
        assert!(!conf(&o).contains("no_display"));
        o.toggle().unwrap();
        o.follow(false);
        o.follow(true);
        assert!(!conf(&o).contains("no_display"), "the next game shows it as the settings say");
    }

    #[test]
    fn settings_are_saved_validated_and_applied() {
        let (d, o) = fixture();
        o.set_up(true).unwrap();
        let s = OverlaySettings { position: "bottom-right".into(), opacity_pct: 80, show: "always".into(), ..Default::default() };
        o.set(s.clone()).unwrap();
        assert!(conf(&o).contains("position=bottom-right") && conf(&o).contains("background_alpha=0.80"));
        assert!(!conf(&o).contains("no_display"), "always: shown even without a game");
        let again = Overlay::new(
            Fps { folder: o.fps.folder.clone(), config: o.fps.config.clone(), root: o.fps.root.clone() },
            &d.path().join("config/unix-guardian"),
        );
        assert_eq!(again.settings(), s);
        assert!(o.set(OverlaySettings { position: "middle".into(), ..Default::default() }).is_err());
        assert!(o.set(OverlaySettings { toggle_key: "Alt; rm -rf".into(), ..Default::default() }).is_err());
        assert_eq!(o.settings(), s, "a refused change changes nothing");
    }

    /// Nothing the overlay writes may land where the whole session reads
    /// it: `MANGOHUD=1` there loads MangoHud into every Vulkan program.
    #[test]
    fn setting_up_and_changing_the_overlay_writes_nothing_session_wide() {
        let (d, o) = fixture();
        std::fs::create_dir_all(o.fps.config.parent().unwrap()).unwrap();
        std::fs::write(&o.fps.config, "fps_limit=144\n").unwrap();
        o.set_up(true).unwrap();
        // An overlay.json from the old version still says every_game.
        o.set(OverlaySettings { show: "always".into(), ..Default::default() }).unwrap();
        let old = r#"{"show":"gaming","layout":"card","position":"top-left","size":"medium","opacity_pct":50,"toggle_key":"Alt_L+o","every_game":true}"#;
        assert!(serde_json::from_str::<OverlaySettings>(old).is_ok(), "an old overlay.json still loads");
        // The fixture's `config` is `~/.config`, its root the home.
        for s in ug_core::session_env::SESSION_WIDE {
            let p = d.path().join(s.replacen(".config", "config", 1));
            assert!(!p.exists(), "{s} was written");
        }
        o.set_up(false).unwrap();
        assert_eq!(conf(&o), "fps_limit=144\n", "the user's own line is all that is left");
    }

    #[test]
    fn nothing_is_written_before_set_up() {
        let (_d, o) = fixture();
        o.set(OverlaySettings::default()).unwrap();
        o.follow(true);
        assert!(!o.fps.config.exists());
    }
}
