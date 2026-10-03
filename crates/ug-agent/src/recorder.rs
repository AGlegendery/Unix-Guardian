//! Recording the screen and the game's sound (roadmap P7.1–7.2), with
//! gpu-screen-recorder: hardware encoding (NVENC, VAAPI), Wayland through
//! the desktop's screen-cast portal, and a replay buffer in memory that
//! saves the last seconds on request.
//!
//! The engine is the program on PATH, or its Flatpak
//! (`com.dec05eba.gpu_screen_recorder`), which "Set up" installs for this
//! user only (no root, undone by removing it). The agent runs one
//! recording at a time and stops it with SIGINT, which makes
//! gpu-screen-recorder finish the file; SIGUSR1 saves the replay buffer, and
//! the program prints the saved file's path.
//!
//! The settings live in `recorder.json`.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use ug_proto::{RecorderSettings, RecorderState};

pub const FLATPAK_ID: &str = "com.dec05eba.gpu_screen_recorder";
pub const QUALITIES: [&str; 3] = ["small", "balanced", "high"];
pub const CODECS: [&str; 4] = ["auto", "h264", "hevc", "av1"];

/// How gpu-screen-recorder is started here.
#[derive(Debug, Clone, PartialEq)]
pub enum Engine {
    Native(PathBuf),
    Flatpak,
}

impl Engine {
    fn argv(&self) -> Vec<String> {
        match self {
            Engine::Native(p) => vec![p.display().to_string()],
            Engine::Flatpak => ["flatpak", "run", "--command=gpu-screen-recorder", FLATPAK_ID].map(String::from).to_vec(),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Engine::Native(_) => "native",
            Engine::Flatpak => "flatpak",
        }
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join(name)).find(|f| f.is_file()))
        .flatten()
}

/// The engine, preferring a program on PATH over the Flatpak.
pub fn find_engine(home: &Path) -> Option<Engine> {
    if let Some(p) = on_path("gpu-screen-recorder") {
        return Some(Engine::Native(p));
    }
    let flatpak = [PathBuf::from("/var/lib/flatpak/app").join(FLATPAK_ID), home.join(".local/share/flatpak/app").join(FLATPAK_ID)];
    (on_path("flatpak").is_some() && flatpak.iter().any(|d| d.exists())).then_some(Engine::Flatpak)
}

/// `--list-capture-options` lines: `portal`, `screen`, `DP-1|2560x1440`;
/// window and region capture need more than a name, so they are left out.
pub fn parse_captures(out: &str) -> Vec<(String, Option<(u32, u32)>)> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !matches!(*l, "window" | "focused" | "region"))
        .map(|l| match l.split_once('|') {
            Some((name, res)) => {
                let wh = res.split_once('x').and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)));
                (name.to_string(), wh)
            }
            None => (l.to_string(), None),
        })
        .collect()
}

/// About how many megabytes a minute takes: gpu-screen-recorder's quality
/// levels land near 6, 12 and 24 Mbit/s for 1080p at 60 fps, and scale
/// with pixels and frame rate.
pub fn mb_per_minute(s: &RecorderSettings, res: Option<(u32, u32)>) -> f64 {
    let mbit = match s.quality.as_str() {
        "small" => 6.0,
        "high" => 24.0,
        _ => 12.0,
    };
    let (w, h) = res.unwrap_or((1920, 1080));
    let scale = (w as f64 * h as f64) / (1920.0 * 1080.0) * (s.fps.max(1) as f64 / 60.0);
    let audio = (s.audio_output as u8 + s.audio_mic as u8) as f64 * 0.128;
    ((mbit * scale + audio) * 60.0 / 8.0 * 10.0).round() / 10.0
}

pub fn validate(s: &RecorderSettings) -> Result<(), String> {
    if !QUALITIES.contains(&s.quality.as_str()) {
        return Err(format!("quality is one of {}", QUALITIES.join(", ")));
    }
    if ![30, 60, 120].contains(&s.fps) {
        return Err("frame rate is 30, 60 or 120".into());
    }
    if !CODECS.contains(&s.codec.as_str()) {
        return Err(format!("codec is one of {}", CODECS.join(", ")));
    }
    if ![15, 30, 60, 120].contains(&s.replay_seconds) {
        return Err("the replay buffer is 15, 30, 60 or 120 seconds".into());
    }
    if !s.capture.is_empty() && !s.capture.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) {
        return Err(format!("{:?} is not a capture choice", s.capture));
    }
    Ok(())
}

/// The arguments for one run: a recording into `file`, or the replay buffer
/// writing into `folder`.
pub fn record_args(s: &RecorderSettings, capture: &str, target: Target<'_>) -> Vec<String> {
    let q = match s.quality.as_str() {
        "small" => "medium",
        "high" => "very_high",
        _ => "high",
    };
    let mut a: Vec<String> = vec!["-w".into(), capture.into(), "-f".into(), s.fps.to_string(), "-q".into(), q.into()];
    if capture == "portal" {
        // Remember the screen chosen in the desktop's dialog.
        a.extend(["-restore-portal-session".into(), "yes".into()]);
    }
    if s.codec != "auto" {
        a.extend(["-k".into(), s.codec.clone()]);
    }
    if s.audio_output {
        a.extend(["-a".into(), "default_output".into()]);
    }
    if s.audio_mic {
        a.extend(["-a".into(), "default_input".into()]);
    }
    a.extend(["-c".into(), "mp4".into()]);
    match target {
        Target::File(f) => a.extend(["-o".into(), f.display().to_string()]),
        Target::Replay(dir) => a.extend(["-r".into(), s.replay_seconds.to_string(), "-o".into(), dir.display().to_string()]),
    }
    a
}

pub enum Target<'a> {
    File(&'a Path),
    Replay(&'a Path),
}

/// `XDG_VIDEOS_DIR` from `user-dirs.dirs`, else ~/Videos.
pub fn videos_dir(home: &Path) -> PathBuf {
    let conf = std::fs::read_to_string(home.join(".config/user-dirs.dirs")).unwrap_or_default();
    conf.lines()
        .find_map(|l| l.strip_prefix("XDG_VIDEOS_DIR="))
        .map(|v| v.trim_matches('"').replace("$HOME", &home.display().to_string()))
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("Videos"))
}

struct Running {
    child: std::process::Child,
    replay: bool,
    file: String,
    since: u64,
}

#[derive(Default)]
struct Inner {
    running: Option<Running>,
    last_saved: String,
    problem: String,
    installing: bool,
    captures: Option<Vec<(String, Option<(u32, u32)>)>>,
}

pub struct Recorder {
    file: PathBuf,
    home: PathBuf,
    wayland: bool,
    settings: Mutex<RecorderSettings>,
    inner: Arc<Mutex<Inner>>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl Recorder {
    pub fn new(config_dir: &Path, home: &Path) -> Recorder {
        let file = config_dir.join("recorder.json");
        let settings = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str::<RecorderSettings>(&t).ok())
            .filter(|s| validate(s).is_ok())
            .unwrap_or_default();
        Recorder {
            file,
            home: home.to_path_buf(),
            wayland: std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "wayland") || std::env::var_os("WAYLAND_DISPLAY").is_some(),
            settings: Mutex::new(settings),
            inner: Arc::new(Mutex::new(Inner::default())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn settings(&self) -> RecorderSettings {
        self.settings.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn folder(&self, s: &RecorderSettings) -> PathBuf {
        if s.folder.is_empty() {
            videos_dir(&self.home).join("Unix Guardian")
        } else {
            PathBuf::from(&s.folder)
        }
    }

    /// The capture choices, asked of the engine once.
    fn captures(&self, engine: &Engine) -> Vec<(String, Option<(u32, u32)>)> {
        if let Some(c) = self.lock().captures.clone() {
            return c;
        }
        let mut argv = engine.argv();
        argv.push("--list-capture-options".into());
        let out = std::process::Command::new(&argv[0]).args(&argv[1..]).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output();
        let c = out.map(|o| parse_captures(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default();
        self.lock().captures = Some(c.clone());
        c
    }

    /// What to capture: the setting, else the portal on Wayland and the whole
    /// screen on X11.
    fn capture(&self, s: &RecorderSettings) -> String {
        if !s.capture.is_empty() {
            s.capture.clone()
        } else if self.wayland {
            "portal".into()
        } else {
            "screen".into()
        }
    }

    /// Notices a run that ended by itself (the portal dialog cancelled, an
    /// encoder error) and keeps its last words.
    fn reap(&self) {
        let mut i = self.lock();
        let ended = match i.running.as_mut() {
            Some(r) => r.child.try_wait().ok().flatten(),
            None => None,
        };
        if let Some(status) = ended {
            let r = i.running.take().expect("checked above");
            if !status.success() {
                i.problem = format!("The recorder stopped by itself ({status}). {}", i.problem).trim().to_string();
            } else if !r.replay {
                i.last_saved = r.file;
            }
        }
    }

    pub fn state(&self) -> RecorderState {
        self.reap();
        let s = self.settings();
        let engine = find_engine(&self.home);
        let captures = engine.as_ref().map(|e| self.capture_list(e)).unwrap_or_default();
        let res = engine.as_ref().and_then(|e| self.captures(e).into_iter().find(|c| c.0 == self.capture(&s)).and_then(|c| c.1));
        let mb = mb_per_minute(&s, res);
        let i = self.lock();
        RecorderState {
            engine: engine.as_ref().map(|e| e.name().to_string()).unwrap_or_default(),
            can_set_up: engine.is_none() && on_path("flatpak").is_some(),
            mode: if i.installing {
                "installing".into()
            } else {
                match &i.running {
                    Some(r) if r.replay => "replay".into(),
                    Some(_) => "recording".into(),
                    None => "idle".into(),
                }
            },
            since_unix_ms: i.running.as_ref().map(|r| r.since).unwrap_or(0),
            file: i.running.as_ref().filter(|r| !r.replay).map(|r| r.file.clone()).unwrap_or_default(),
            last_saved: i.last_saved.clone(),
            problem: i.problem.clone(),
            captures,
            mb_per_minute: mb,
            replay_mb: (mb * s.replay_seconds as f64 / 60.0 * 10.0).round() / 10.0,
            folder: self.folder(&s).display().to_string(),
            settings: s,
        }
    }

    fn capture_list(&self, e: &Engine) -> Vec<String> {
        let mut v: Vec<String> = self.captures(e).into_iter().map(|c| c.0).collect();
        if v.is_empty() {
            v.push(if self.wayland { "portal".into() } else { "screen".into() });
        }
        v
    }

    pub fn set(&self, s: RecorderSettings) -> Result<(), String> {
        validate(&s)?;
        let text = serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?;
        ug_core::paths::write_atomic(&self.file, text.as_bytes()).map_err(|e| format!("{}: {e}", self.file.display()))?;
        *self.settings.lock().unwrap_or_else(|e| e.into_inner()) = s;
        Ok(())
    }

    fn spawn(&self, replay: bool) -> Result<String, String> {
        self.reap();
        if self.lock().running.is_some() {
            return Err("The recorder is already running; stop it first.".into());
        }
        let engine = find_engine(&self.home).ok_or("gpu-screen-recorder is not installed; set it up first (ug record setup).")?;
        let s = self.settings();
        let folder = self.folder(&s);
        std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        let stamp = chrono_like(now_ms());
        let file = folder.join(format!("Recording {stamp}.mp4"));
        let target = if replay { Target::Replay(&folder) } else { Target::File(&file) };
        let mut argv = engine.argv();
        argv.extend(record_args(&s, &self.capture(&s), target));
        tracing::info!(argv = ?argv, "recorder starting");
        let mut child = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start {}: {e}", argv[0]))?;
        // Saved files on stdout; the last error line on stderr.
        let inner = Arc::clone(&self.inner);
        if let Some(out) = child.stdout.take() {
            std::thread::spawn(move || {
                for l in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                    let l = l.trim().to_string();
                    if l.ends_with(".mp4") || l.ends_with(".mkv") {
                        inner.lock().unwrap_or_else(|e| e.into_inner()).last_saved = l;
                    }
                }
            });
        }
        let inner = Arc::clone(&self.inner);
        if let Some(err) = child.stderr.take() {
            std::thread::spawn(move || {
                for l in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                    if l.to_lowercase().contains("error") {
                        inner.lock().unwrap_or_else(|e| e.into_inner()).problem = l.trim().to_string();
                    }
                }
            });
        }
        let mut i = self.lock();
        i.problem.clear();
        let shown = if replay { String::new() } else { file.display().to_string() };
        i.running = Some(Running { child, replay, file: shown.clone(), since: now_ms() });
        Ok(shown)
    }

    pub fn is_recording(&self) -> bool {
        self.reap();
        self.lock().running.as_ref().is_some_and(|r| !r.replay)
    }

    pub fn is_replaying(&self) -> bool {
        self.reap();
        self.lock().running.as_ref().is_some_and(|r| r.replay)
    }

    pub fn start(&self) -> Result<String, String> {
        self.spawn(false)
    }

    pub fn start_replay(&self) -> Result<(), String> {
        self.spawn(true).map(|_| ())
    }

    /// Stops the run: SIGINT, then up to ten seconds for the file to be
    /// finished.
    pub fn stop(&self) -> Result<String, String> {
        self.reap();
        let Some(mut r) = self.lock().running.take() else {
            return Err("The recorder is not running.".into());
        };
        // SAFETY: a signal to our own child, whose pid is still ours (it has
        // not been waited for).
        unsafe {
            libc::kill(r.child.id() as i32, libc::SIGINT);
        }
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Ok(Some(_)) = r.child.try_wait() {
                break;
            }
            if std::time::Instant::now() > until {
                let _ = r.child.kill();
                let _ = r.child.wait();
                return Err("The recorder did not finish in time and was stopped; the file may be incomplete.".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if r.replay {
            return Ok(String::new());
        }
        self.lock().last_saved = r.file.clone();
        Ok(r.file)
    }

    /// Saves the replay buffer: SIGUSR1, then the path the engine prints.
    pub fn save_replay(&self) -> Result<String, String> {
        self.reap();
        let pid = match &self.lock().running {
            Some(r) if r.replay => r.child.id(),
            _ => return Err("The replay buffer is not running.".into()),
        };
        let before = self.lock().last_saved.clone();
        // SAFETY: as in `stop`.
        unsafe {
            libc::kill(pid as i32, libc::SIGUSR1);
        }
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < until {
            let now = self.lock().last_saved.clone();
            if now != before && !now.is_empty() {
                return Ok(now);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err("The replay was asked to save, but no file was reported.".into())
    }

    /// Installs (or removes) the Flatpak for this user, in the background
    /// (a download of a few minutes); `state` says `installing` meanwhile and
    /// `problem` if it failed.
    pub fn begin_set_up(self: &Arc<Self>, on: bool) -> Result<(), String> {
        if on_path("flatpak").is_none() {
            return Err("Flatpak is not installed; install gpu-screen-recorder with your package manager instead.".into());
        }
        {
            let mut i = self.lock();
            if i.installing {
                return Err("It is being installed already.".into());
            }
            i.installing = true;
            i.problem.clear();
        }
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            if let Err(e) = me.set_up(on) {
                me.lock().problem = e;
            }
        });
        Ok(())
    }

    fn set_up(&self, on: bool) -> Result<(), String> {
        let steps: Vec<Vec<&str>> = if on {
            vec![
                vec!["flatpak", "remote-add", "--user", "--if-not-exists", "flathub", "https://dl.flathub.org/repo/flathub.flatpakrepo"],
                vec!["flatpak", "install", "--user", "--noninteractive", "-y", "flathub", FLATPAK_ID],
            ]
        } else {
            vec![vec!["flatpak", "uninstall", "--user", "--noninteractive", "-y", FLATPAK_ID]]
        };
        let r = (|| {
            for s in steps {
                let out = std::process::Command::new(s[0]).args(&s[1..]).stdin(std::process::Stdio::null()).output().map_err(|e| e.to_string())?;
                if !out.status.success() {
                    return Err(format!("`{}` failed: {}", s.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
                }
            }
            Ok(())
        })();
        let mut i = self.lock();
        i.installing = false;
        i.captures = None;
        r
    }

    /// Stops whatever runs (agent shutdown), finishing the file.
    pub fn shutdown(&self) {
        if self.lock().running.is_some() {
            let _ = self.stop();
        }
    }
}

/// `2026-10-03 19-40-12` in local time, without a date library.
fn chrono_like(ms: u64) -> String {
    let secs = (ms / 1000) as libc::time_t;
    // SAFETY: localtime_r fills the struct we own.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&secs, &mut tm);
    }
    format!(
        "{:04}-{:02}-{:02} {:02}-{:02}-{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_choices_and_their_sizes_are_read_from_the_engine() {
        let c = parse_captures("window\nfocused\nregion\nportal\nDP-1|2560x1440\neDP-1|1920x1080\n");
        assert_eq!(c, [("portal".to_string(), None), ("DP-1".into(), Some((2560, 1440))), ("eDP-1".into(), Some((1920, 1080)))]);
        let mut s = RecorderSettings::default();
        assert_eq!(mb_per_minute(&s, None), 91.0, "12 Mbit/s and one audio track ≈ 91 MB a minute");
        s.quality = "small".into();
        s.fps = 30;
        assert_eq!(mb_per_minute(&s, None), 23.5);
        s.quality = "high".into();
        s.fps = 60;
        assert!(mb_per_minute(&s, Some((2560, 1440))) > 300.0);
    }

    #[test]
    fn arguments_follow_the_settings() {
        let mut s = RecorderSettings::default();
        s.audio_mic = true;
        let a = record_args(&s, "portal", Target::File(Path::new("/v/a.mp4")));
        assert_eq!(a.join(" "), "-w portal -f 60 -q high -restore-portal-session yes -a default_output -a default_input -c mp4 -o /v/a.mp4");
        s.codec = "hevc".into();
        s.audio_output = false;
        s.audio_mic = false;
        let a = record_args(&s, "DP-1", Target::Replay(Path::new("/v")));
        assert_eq!(a.join(" "), "-w DP-1 -f 60 -q high -k hevc -c mp4 -r 60 -o /v");
        assert!(validate(&RecorderSettings { fps: 50, ..Default::default() }).is_err());
        assert!(validate(&RecorderSettings { capture: "DP-1; rm".into(), ..Default::default() }).is_err());
        assert!(validate(&RecorderSettings::default()).is_ok());
    }

    #[test]
    fn the_videos_folder_comes_from_user_dirs() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(videos_dir(d.path()), d.path().join("Videos"));
        std::fs::create_dir_all(d.path().join(".config")).unwrap();
        std::fs::write(d.path().join(".config/user-dirs.dirs"), "XDG_VIDEOS_DIR=\"$HOME/ویدیو\"\n").unwrap();
        assert_eq!(videos_dir(d.path()), d.path().join("ویدیو"));
    }
}
