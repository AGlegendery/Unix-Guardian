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

pub const FLATPAK_ID: &str = ug_core::capture::FLATPAK_ID;
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
            // --die-with-parent: the sandbox goes when the agent does.
            Engine::Flatpak => [
                "flatpak",
                "run",
                "--die-with-parent",
                "--command=gpu-screen-recorder",
                FLATPAK_ID,
            ]
            .map(String::from)
            .to_vec(),
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
        .map(|p| {
            std::env::split_paths(&p)
                .map(|d| d.join(name))
                .find(|f| f.is_file())
        })
        .flatten()
}

/// The engine, preferring a program on PATH over the Flatpak.
pub fn find_engine(home: &Path) -> Option<Engine> {
    if let Some(p) = on_path("gpu-screen-recorder") {
        return Some(Engine::Native(p));
    }
    let flatpak = [
        PathBuf::from("/var/lib/flatpak/app").join(FLATPAK_ID),
        home.join(".local/share/flatpak/app").join(FLATPAK_ID),
    ];
    (on_path("flatpak").is_some() && flatpak.iter().any(|d| d.exists())).then_some(Engine::Flatpak)
}

/// `--list-capture-options` lines: `portal`, `focused` (the active window,
/// X11 only), `DP-1|2560x1440`; picking a window or a region needs more
/// than a name, so those are left out.
/// Cameras (`/dev/video0|1280x720@30hz|mjpeg`, one line per mode) are
/// left out: this records the screen. Each name is offered once.
pub fn parse_captures(out: &str) -> Vec<(String, Option<(u32, u32)>)> {
    let mut v: Vec<(String, Option<(u32, u32)>)> = Vec::new();
    for l in out.lines().map(str::trim) {
        if l.is_empty() || l.starts_with("/dev/") || matches!(l, "window" | "region") {
            continue;
        }
        let c = match l.split_once('|') {
            Some((name, res)) => {
                let res = res.split('|').next().unwrap_or(res);
                let wh = res
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)));
                (name.to_string(), wh)
            }
            None => (l.to_string(), None),
        };
        if !v.iter().any(|x| x.0 == c.0) {
            v.push(c);
        }
    }
    v
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
    if !s.capture.is_empty()
        && !s
            .capture
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    {
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
    let mut a: Vec<String> = vec![
        "-w".into(),
        capture.into(),
        "-f".into(),
        s.fps.to_string(),
        "-q".into(),
        q.into(),
    ];
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
        Target::Replay(dir) => a.extend([
            "-r".into(),
            s.replay_seconds.to_string(),
            "-o".into(),
            dir.display().to_string(),
        ]),
    }
    a
}

const MONITOR_UNAVAILABLE: &str = "Recording a whole screen on X11 with Intel or AMD graphics needs gpu-screen-recorder installed for all users \
     (flatpak install --system flathub com.dec05eba.gpu_screen_recorder) or from your distribution. \
     Choose \"focused\" (the active window) or Automatic to record without it.";

fn capture_env() -> ug_core::capture::CaptureEnv {
    ug_core::capture::CaptureEnv::detect(Path::new("/"))
}

/// The capture choices to show: monitors are left out where they cannot be
/// recorded, and there is always at least the automatic choice.
pub fn offered(mut v: Vec<String>, env: &ug_core::capture::CaptureEnv) -> Vec<String> {
    if env.monitor_unavailable() {
        v.retain(|c| !ug_core::capture::is_monitor(c));
    }
    if env.wayland {
        v.retain(|c| c != "focused");
    }
    let auto = env.auto_capture().to_string();
    if !v.contains(&auto) {
        v.insert(0, auto);
    }
    v
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
        .map(|v| {
            v.trim_matches('"')
                .replace("$HOME", &home.display().to_string())
        })
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("Videos"))
}

// The Flatpak's process tree. `flatpak run` becomes `bwrap`, which starts
// an inner `bwrap` (the sandbox's first process) and that starts the
// recorder. A signal to our child reaches only the outer bwrap: it dies, the
// inner one and the recorder are handed to the session's init and keep
// recording, unseen (found on the user's machine after the software test,
// 2026-10-05). So signals go to the recorder process itself, found in /proc.

/// The recorder's name in /proc (`comm` is cut to 15 characters).
const ENGINE_COMM: &str = "gpu-screen-reco";

fn ppid(proc_root: &Path, pid: i32) -> Option<i32> {
    let stat = std::fs::read_to_string(proc_root.join(pid.to_string()).join("stat")).ok()?;
    // After the last ')' (the name may hold spaces): state, then the parent.
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

fn comm(proc_root: &Path, pid: i32) -> Option<String> {
    std::fs::read_to_string(proc_root.join(pid.to_string()).join("comm"))
        .ok()
        .map(|c| c.trim_end().to_string())
}

fn pids(proc_root: &Path) -> Vec<i32> {
    std::fs::read_dir(proc_root)
        .map(|d| {
            d.flatten()
                .filter_map(|e| e.file_name().to_str()?.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Every process below `root`.
fn descendants(proc_root: &Path, root: i32) -> Vec<i32> {
    let all: Vec<(i32, i32)> = pids(proc_root)
        .into_iter()
        .filter_map(|p| Some((p, ppid(proc_root, p)?)))
        .collect();
    let mut out = Vec::new();
    let mut todo = vec![root];
    while let Some(p) = todo.pop() {
        for &(c, _) in all.iter().filter(|(_, pp)| *pp == p) {
            out.push(c);
            todo.push(c);
        }
    }
    out
}

/// The recorder processes to signal for our child `root`: the recorder
/// inside the Flatpak, or the child itself when it is the recorder.
fn engine_pids(proc_root: &Path, root: i32) -> Vec<i32> {
    let inside: Vec<i32> = descendants(proc_root, root)
        .into_iter()
        .filter(|p| comm(proc_root, *p).as_deref() == Some(ENGINE_COMM))
        .collect();
    if inside.is_empty() {
        vec![root]
    } else {
        inside
    }
}

/// Recorders of this user writing into `folder` that are not `ours`: left
/// running by an older agent or one that died.
fn leftover_engines(proc_root: &Path, folder: &Path, ours: &[i32]) -> Vec<i32> {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid cannot fail and has no preconditions.
    let me = unsafe { libc::geteuid() };
    let folder = folder.to_string_lossy().trim_end_matches('/').to_string();
    pids(proc_root)
        .into_iter()
        .filter(|p| !ours.contains(p) && comm(proc_root, *p).as_deref() == Some(ENGINE_COMM))
        .filter(|p| std::fs::metadata(proc_root.join(p.to_string())).is_ok_and(|m| m.uid() == me))
        .filter(|p| {
            let cmd =
                std::fs::read(proc_root.join(p.to_string()).join("cmdline")).unwrap_or_default();
            cmd.split(|b| *b == 0).any(|a| {
                let a = String::from_utf8_lossy(a);
                a == folder || a.starts_with(&format!("{folder}/"))
            })
        })
        .collect()
}

fn alive(proc_root: &Path, pid: i32) -> bool {
    match std::fs::read_to_string(proc_root.join(pid.to_string()).join("stat")) {
        Ok(stat) => {
            stat.rsplit_once(')')
                .and_then(|(_, rest)| rest.split_whitespace().next())
                != Some("Z")
        }
        Err(_) => false,
    }
}

/// SIGINT (the recorder then finishes its file), then waits for them all to
/// be gone; SIGKILL to what is left after `wait`. True when all ended by
/// themselves.
fn interrupt_and_wait(proc_root: &Path, list: &[i32], wait: std::time::Duration) -> bool {
    for &p in list {
        // SAFETY: plain kill(2); a pid that is gone gives ESRCH, nothing else.
        unsafe { libc::kill(p, libc::SIGINT) };
    }
    let until = std::time::Instant::now() + wait;
    while std::time::Instant::now() < until {
        if !list.iter().any(|p| alive(proc_root, *p)) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    for &p in list.iter().filter(|p| alive(proc_root, **p)) {
        // SAFETY: as above.
        unsafe { libc::kill(p, libc::SIGKILL) };
    }
    false
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
    /// Flatpak's progress while installing: overall percent and "n/m".
    setup: Option<(u8, String)>,
    captures: Option<Vec<(String, Option<(u32, u32)>)>>,
}

pub struct Recorder {
    file: PathBuf,
    home: PathBuf,
    settings: Mutex<RecorderSettings>,
    inner: Arc<Mutex<Inner>>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
            settings: Mutex::new(settings),
            inner: Arc::new(Mutex::new(Inner::default())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn settings(&self) -> RecorderSettings {
        self.settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
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
        let out = std::process::Command::new(&argv[0])
            .args(&argv[1..])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
        let c = out
            .map(|o| parse_captures(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default();
        self.lock().captures = Some(c.clone());
        c
    }

    /// What to capture: the setting, else the portal on Wayland and on X11
    /// the whole screen, or the active window where a monitor cannot be
    /// recorded (see `ug_core::capture`).
    fn capture(&self, s: &RecorderSettings) -> String {
        capture_env().effective(&s.capture).to_string()
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
                i.problem = format!("The recorder stopped by itself ({status}). {}", i.problem)
                    .trim()
                    .to_string();
            } else if !r.replay {
                i.last_saved = r.file;
            }
        }
    }

    pub fn state(&self) -> RecorderState {
        self.reap();
        let s = self.settings();
        let engine = find_engine(&self.home);
        let captures = engine
            .as_ref()
            .map(|e| self.capture_list(e))
            .unwrap_or_default();
        let res = engine.as_ref().and_then(|e| {
            self.captures(e)
                .into_iter()
                .find(|c| c.0 == self.capture(&s))
                .and_then(|c| c.1)
        });
        let mb = mb_per_minute(&s, res);
        let i = self.lock();
        RecorderState {
            engine: engine
                .as_ref()
                .map(|e| e.name().to_string())
                .unwrap_or_default(),
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
            file: i
                .running
                .as_ref()
                .filter(|r| !r.replay)
                .map(|r| r.file.clone())
                .unwrap_or_default(),
            last_saved: i.last_saved.clone(),
            problem: i.problem.clone(),
            setup_pct: i.setup.as_ref().map(|s| s.0).unwrap_or(0),
            setup_step: i.setup.as_ref().map(|s| s.1.clone()).unwrap_or_default(),
            captures,
            mb_per_minute: mb,
            replay_mb: (mb * s.replay_seconds as f64 / 60.0 * 10.0).round() / 10.0,
            folder: self.folder(&s).display().to_string(),
            settings: s,
        }
    }

    fn capture_list(&self, e: &Engine) -> Vec<String> {
        offered(
            self.captures(e).into_iter().map(|c| c.0).collect(),
            &capture_env(),
        )
    }

    pub fn set(&self, s: RecorderSettings) -> Result<(), String> {
        validate(&s)?;
        let text = serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?;
        ug_core::paths::write_atomic(&self.file, text.as_bytes())
            .map_err(|e| format!("{}: {e}", self.file.display()))?;
        *self.settings.lock().unwrap_or_else(|e| e.into_inner()) = s;
        Ok(())
    }

    fn spawn(&self, replay: bool) -> Result<String, String> {
        self.reap();
        if self.lock().running.is_some() {
            return Err("The recorder is already running; stop it first.".into());
        }
        let engine = find_engine(&self.home)
            .ok_or("gpu-screen-recorder is not installed; set it up first (ug record setup).")?;
        self.stop_leftovers(&[]);
        let s = self.settings();
        let env = capture_env();
        if ug_core::capture::is_monitor(&self.capture(&s)) && env.monitor_unavailable() {
            return Err(MONITOR_UNAVAILABLE.into());
        }
        let folder = self.folder(&s);
        std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        let stamp = chrono_like(now_ms());
        let file = folder.join(format!("Recording {stamp}.mp4"));
        let target = if replay {
            Target::Replay(&folder)
        } else {
            Target::File(&file)
        };
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
                        inner.lock().unwrap_or_else(|e| e.into_inner()).problem =
                            l.trim().to_string();
                    }
                }
            });
        }
        let mut i = self.lock();
        i.problem.clear();
        let shown = if replay {
            String::new()
        } else {
            file.display().to_string()
        };
        i.running = Some(Running {
            child,
            replay,
            file: shown.clone(),
            since: now_ms(),
        });
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

    /// Stops the run: SIGINT to the recorder itself, then up to ten seconds
    /// for the file to be finished. With nothing running it stops recorders
    /// left behind in our folder (by an older agent or a crash), if any.
    pub fn stop(&self) -> Result<String, String> {
        self.reap();
        let proc_root = Path::new("/proc");
        let Some(mut r) = self.lock().running.take() else {
            let n = self.stop_leftovers(&[]);
            if n > 0 {
                return Ok(String::new());
            }
            // It may have ended by itself: say why rather than "not running".
            let problem = self.lock().problem.clone();
            return Err(if problem.is_empty() {
                "The recorder is not running.".into()
            } else {
                problem
            });
        };
        let engines = engine_pids(proc_root, r.child.id() as i32);
        let finished = interrupt_and_wait(proc_root, &engines, std::time::Duration::from_secs(10));
        // The Flatpak's bwrap ends when the recorder does; make sure, then reap.
        let rest = descendants(proc_root, r.child.id() as i32);
        if r.child.try_wait().ok().flatten().is_none() {
            interrupt_and_wait(proc_root, &rest, std::time::Duration::from_secs(2));
            let _ = r.child.kill();
        }
        let _ = r.child.wait();
        if !finished {
            return Err(
                "The recorder did not finish in time and was stopped; the file may be incomplete."
                    .into(),
            );
        }
        if r.replay {
            return Ok(String::new());
        }
        self.lock().last_saved = r.file.clone();
        Ok(r.file)
    }

    /// Stops recorders writing into our folder that this agent did not
    /// start (or no longer knows about). Returns how many there were.
    pub fn stop_leftovers(&self, ours: &[i32]) -> usize {
        let proc_root = Path::new("/proc");
        let folder = self.folder(&self.settings());
        let left = leftover_engines(proc_root, &folder, ours);
        if !left.is_empty() {
            tracing::warn!(pids = ?left, "stopping recorders left running in the recordings folder");
            interrupt_and_wait(proc_root, &left, std::time::Duration::from_secs(10));
        }
        left.len()
    }

    /// Saves the replay buffer: SIGUSR1, then the path the engine prints.
    pub fn save_replay(&self) -> Result<String, String> {
        self.reap();
        let pid = match &self.lock().running {
            Some(r) if r.replay => r.child.id(),
            _ => return Err("The replay buffer is not running.".into()),
        };
        let before = self.lock().last_saved.clone();
        for p in engine_pids(Path::new("/proc"), pid as i32) {
            // SAFETY: plain kill(2) to the recorder (see `engine_pids`).
            unsafe {
                libc::kill(p, libc::SIGUSR1);
            }
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
            i.setup = None;
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
        let r = if on {
            run(&[
                "flatpak",
                "remote-add",
                "--user",
                "--if-not-exists",
                "flathub",
                "https://dl.flathub.org/repo/flathub.flatpakrepo",
            ])
            .and_then(|_| self.install())
        } else {
            run(&[
                "flatpak",
                "uninstall",
                "--user",
                "--noninteractive",
                "-y",
                FLATPAK_ID,
            ])
        };
        let mut i = self.lock();
        i.installing = false;
        i.setup = None;
        i.captures = None;
        r
    }

    /// `flatpak install` on a terminal of its own: only there does Flatpak
    /// print its progress line (with `--noninteractive`, or into a pipe, it
    /// says nothing for minutes while it downloads a ~300 MB runtime, and on
    /// 2026-10-03 the user took that for a hang and shut down mid-download).
    fn install(&self) -> Result<(), String> {
        use std::io::Read;
        let argv = ["flatpak", "install", "--user", "-y", "flathub", FLATPAK_ID];
        let (mut child, mut master) =
            spawn_on_pty(&argv).map_err(|e| format!("could not start flatpak: {e}"))?;
        let mut progress = InstallProgress::default();
        let mut buf = [0u8; 4096];
        loop {
            match master.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if progress.feed(&buf[..n]) {
                        self.lock().setup = progress.overall();
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                // EIO: every slave end is closed, flatpak has exited.
                Err(_) => break,
            }
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        progress.finish();
        if status.success() {
            return Ok(());
        }
        Err(install_failure(&progress.messages))
    }

    /// Stops whatever runs (agent shutdown), finishing the file.
    pub fn shutdown(&self) {
        if self.lock().running.is_some() {
            let _ = self.stop();
        }
    }
}

/// Runs one command, its error output becoming the error.
fn run(argv: &[&str]) -> Result<(), String> {
    let out = std::process::Command::new(argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "`{}` failed: {}",
            argv.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Starts `argv` with a new pseudo-terminal as its stdout and stderr, and
/// returns the terminal's reading end.
fn spawn_on_pty(argv: &[&str]) -> std::io::Result<(std::process::Child, std::fs::File)> {
    use std::os::fd::{FromRawFd, OwnedFd};
    let (mut m, mut s) = (0, 0);
    let ws = libc::winsize {
        ws_row: 30,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: openpty writes two descriptors we then own; the other pointers
    // may be null.
    if unsafe { libc::openpty(&mut m, &mut s, std::ptr::null_mut(), std::ptr::null(), &ws) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: both were just opened and are owned by nobody else.
    let (master, slave) = unsafe { (std::fs::File::from_raw_fd(m), OwnedFd::from_raw_fd(s)) };
    // SAFETY: plain fcntl on a descriptor we own; the child must not keep
    // the reading end open.
    unsafe { libc::fcntl(m, libc::F_SETFD, libc::FD_CLOEXEC) };
    let child = std::process::Command::new(argv[0])
        .args(&argv[1..])
        .env("TERM", "xterm")
        .env("LC_ALL", "C.UTF-8")
        .stdin(std::process::Stdio::null())
        .stdout(slave.try_clone()?)
        .stderr(slave)
        .spawn()?;
    Ok((child, master))
}

/// Reads Flatpak's terminal output: its redrawn progress line
/// ("Installing 2/7… ████▌   42%  1.2 MB/s  00:30") and the messages
/// worth keeping (warnings, errors).
#[derive(Default)]
struct InstallProgress {
    pending: Vec<u8>,
    /// Step n of m, and that step's percent.
    step: Option<(u32, u32, u8)>,
    messages: Vec<String>,
}

impl InstallProgress {
    /// Takes more output; true when the progress moved.
    fn feed(&mut self, bytes: &[u8]) -> bool {
        self.pending.extend_from_slice(bytes);
        let text = String::from_utf8_lossy(&self.pending).to_string();
        // Keep a possibly cut-off last piece (or a split UTF-8 character).
        let cut = text.rfind(['\r', '\n']).map(|i| i + 1).unwrap_or(0);
        let (done, rest) = text.split_at(cut);
        self.pending = rest.as_bytes().to_vec();
        let before = self.step;
        for seg in done.split(['\r', '\n']) {
            self.segment(seg);
        }
        self.step != before
    }

    fn finish(&mut self) {
        let rest = String::from_utf8_lossy(&std::mem::take(&mut self.pending)).to_string();
        self.segment(&rest);
    }

    fn segment(&mut self, raw: &str) {
        let line = strip_ansi(raw);
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        if let Some(p) = parse_progress(line) {
            self.step = Some(p);
        } else if ["Warning:", "error:", "Error:"]
            .iter()
            .any(|w| line.starts_with(w))
        {
            self.messages.push(line.to_string());
        }
    }

    /// The whole install's percent (each step an equal share) and "n/m".
    fn overall(&self) -> Option<(u8, String)> {
        let (n, m, pct) = self.step?;
        let total = ((n.saturating_sub(1) * 100 + pct as u32) / m.max(1)).min(100);
        Some((total as u8, format!("{n}/{m}")))
    }
}

/// Removes terminal escape sequences (`ESC [ … letter`, `ESC ] … BEL`).
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('[') => {
                for c in it.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                for c in it.by_ref() {
                    if c == '\x07' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// `Installing 2/7… ███   42%  …` → (2, 7, 42).
fn parse_progress(line: &str) -> Option<(u32, u32, u8)> {
    let rest = ["Installing ", "Updating ", "Uninstalling "]
        .iter()
        .find_map(|w| line.strip_prefix(w))?;
    let (nm, rest) = rest.split_once('…').or_else(|| rest.split_once(' '))?;
    let (n, m) = nm.trim_end_matches("...").split_once('/')?;
    let pct = rest
        .split_whitespace()
        .find_map(|w| w.strip_suffix('%')?.parse::<u8>().ok())?;
    Some((n.parse().ok()?, m.parse().ok()?, pct.min(100)))
}

/// What to tell the user when `flatpak install` failed, from its own
/// messages: a broken download is said plainly, since Flatpak keeps what
/// it already fetched and a second try goes on from there.
fn install_failure(messages: &[String]) -> String {
    let errors: Vec<&String> = messages
        .iter()
        .filter(|m| !m.starts_with("Warning:"))
        .collect();
    let all = messages.join(" ");
    let network = [
        "Failure when receiving data",
        "Failed sending data",
        "Could not resolve host",
        "Timeout was reached",
        "Connection reset",
        "HTTP error",
    ]
    .iter()
    .any(|w| all.contains(w));
    let detail = errors
        .last()
        .map(|e| e.as_str())
        .or(messages.last().map(|e| e.as_str()))
        .unwrap_or("no message");
    if network {
        format!("The download from Flathub broke off ({detail}). Set up again: Flatpak keeps what it already downloaded.")
    } else {
        format!("flatpak install failed: {detail}")
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

    /// Flatpak's real output on a terminal (2026-10-04, two steps).
    #[test]
    fn flatpak_progress_is_read_from_its_terminal_output() {
        let raw = include_bytes!("../testdata/flatpak-install-tty.txt");
        let mut p = InstallProgress::default();
        let mut seen = Vec::new();
        // In small pieces, as a terminal delivers it.
        for chunk in raw.chunks(37) {
            if p.feed(chunk) {
                seen.push(p.overall().unwrap());
            }
        }
        p.finish();
        let pcts: Vec<u8> = seen.iter().map(|s| s.0).collect();
        assert_eq!(seen.first(), Some(&(0, "1/2".to_string())));
        assert!(pcts.contains(&4), "step 1 at 8% is 4% overall: {pcts:?}");
        assert!(
            pcts.windows(2).all(|w| w[0] <= w[1]),
            "never goes back: {pcts:?}"
        );
        assert_eq!(p.overall(), Some((100, "2/2".to_string())));
        assert!(p.messages.is_empty(), "{:?}", p.messages);
    }

    #[test]
    fn a_broken_download_is_said_plainly_and_a_warning_is_not_an_error() {
        let mut p = InstallProgress::default();
        p.feed(b"Installing 1/3\xe2\x80\xa6 \x1b[2m\xe2\x96\x88\x1b[22m  5%  1 MB/s\r");
        p.feed(b"Warning: Failed to install org.freedesktop.Platform.openh264: While downloading http://ciscobinary.openh264.org/x: Server returned status 403\r\n");
        p.feed(b"error: While pulling runtime/org.freedesktop.Platform/x86_64/24.08 from remote flathub: Failure when receiving data from the peer\r\n");
        p.finish();
        assert_eq!(p.overall(), Some((1, "1/3".to_string())));
        let m = install_failure(&p.messages);
        assert!(
            m.starts_with("The download from Flathub broke off (error: While pulling"),
            "{m}"
        );
        assert!(
            install_failure(&["error: No remote refs found for 'x'".into()])
                .starts_with("flatpak install failed: error: No remote")
        );
        assert_eq!(
            parse_progress("Installing 3/7… ██▍   42%  1.2 MB/s  00:30"),
            Some((3, 7, 42))
        );
        assert_eq!(parse_progress("Looking for matches…"), None);
    }

    /// XFCE on X11, Intel desktop, the Flatpak installed per user
    /// (2026-10-10): only `eDP-1` was offered, and recording it failed
    /// because the KMS helper is unreachable. The window is offered instead.
    #[test]
    fn x11_without_the_kms_helper_offers_the_window_not_the_monitor() {
        use ug_core::capture::CaptureEnv;
        let list = || vec!["focused".to_string(), "eDP-1".into()];
        let user_flatpak = CaptureEnv {
            wayland: false,
            nvidia_desktop: false,
            kms_reachable: false,
        };
        assert_eq!(offered(list(), &user_flatpak), ["focused"]);
        assert_eq!(
            offered(vec!["eDP-1".into()], &user_flatpak),
            ["focused"],
            "older engines do not list it"
        );
        let system = CaptureEnv {
            kms_reachable: true,
            ..user_flatpak
        };
        assert_eq!(offered(list(), &system), ["screen", "focused", "eDP-1"]);
        let wayland = CaptureEnv {
            wayland: true,
            nvidia_desktop: false,
            kms_reachable: false,
        };
        assert_eq!(
            offered(vec!["eDP-1".into(), "portal".into()], &wayland),
            ["portal"]
        );
    }

    /// The real gpu-screen-recorder 5 (Flatpak) on the laptop, 2026-10-04:
    /// the webcam came back as nine `/dev/video0` choices.
    #[test]
    fn cameras_are_not_offered_and_nothing_twice() {
        let real = "eDP-1|1920x1080\nregion\n/dev/video0|1280x720@30hz|mjpeg\n/dev/video0|1280x720@10hz|yuyv\n\
                    /dev/video0|640x480@30hz|mjpeg\nportal\neDP-1|1920x1080\n";
        assert_eq!(
            parse_captures(real),
            [
                ("eDP-1".to_string(), Some((1920, 1080))),
                ("portal".into(), None)
            ]
        );
    }

    #[test]
    fn capture_choices_and_their_sizes_are_read_from_the_engine() {
        let c =
            parse_captures("window\nfocused\nregion\nportal\nDP-1|2560x1440\neDP-1|1920x1080\n");
        assert_eq!(
            c,
            [
                ("focused".to_string(), None),
                ("portal".into(), None),
                ("DP-1".into(), Some((2560, 1440))),
                ("eDP-1".into(), Some((1920, 1080)))
            ]
        );
        let mut s = RecorderSettings::default();
        assert_eq!(
            mb_per_minute(&s, None),
            91.0,
            "12 Mbit/s and one audio track ≈ 91 MB a minute"
        );
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
        assert_eq!(
            a.join(" "),
            "-w DP-1 -f 60 -q high -k hevc -c mp4 -r 60 -o /v"
        );
        assert!(validate(&RecorderSettings {
            fps: 50,
            ..Default::default()
        })
        .is_err());
        assert!(validate(&RecorderSettings {
            capture: "DP-1; rm".into(),
            ..Default::default()
        })
        .is_err());
        assert!(validate(&RecorderSettings::default()).is_ok());
    }

    #[test]
    fn the_videos_folder_comes_from_user_dirs() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(videos_dir(d.path()), d.path().join("Videos"));
        std::fs::create_dir_all(d.path().join(".config")).unwrap();
        std::fs::write(
            d.path().join(".config/user-dirs.dirs"),
            "XDG_VIDEOS_DIR=\"$HOME/ویدیو\"\n",
        )
        .unwrap();
        assert_eq!(videos_dir(d.path()), d.path().join("ویدیو"));
    }

    /// A fake /proc entry: `stat` with the parent, `comm`, NUL-separated
    /// `cmdline`.
    fn proc_entry(fs: &ug_testfs::TestFs, pid: i32, ppid: i32, comm: &str, args: &[&str]) {
        fs.write(
            format!("{pid}/stat"),
            &format!("{pid} ({comm}) S {ppid} 1 1 0 -1"),
        );
        fs.write(format!("{pid}/comm"), &format!("{comm}\n"));
        fs.write(format!("{pid}/cmdline"), &(args.join("\0") + "\0"));
    }

    /// The Flatpak runs as `bwrap` (flatpak run becomes it) → an inner
    /// `bwrap` → gpu-screen-recorder. A signal to our child reaches only the
    /// outer bwrap, which dies and leaves the recorder running (seen on the
    /// user's machine after the software test, 2026-10-05). The signal must
    /// go to the recorder itself.
    #[test]
    fn the_signal_goes_to_the_recorder_inside_the_flatpak_not_to_bwrap() {
        let fs = ug_testfs::TestFs::new();
        proc_entry(
            &fs,
            100,
            1,
            "bwrap",
            &["bwrap", "--args", "74", "--", "gpu-screen-recorder"],
        );
        proc_entry(
            &fs,
            101,
            100,
            "bwrap",
            &["bwrap", "--args", "74", "--", "gpu-screen-recorder"],
        );
        proc_entry(
            &fs,
            102,
            101,
            "gpu-screen-reco",
            &[
                "gpu-screen-recorder",
                "-w",
                "portal",
                "-o",
                "/v/Unix Guardian/a.mp4",
            ],
        );
        proc_entry(
            &fs,
            200,
            1,
            "gpu-screen-reco",
            &["gpu-screen-recorder", "-o", "/elsewhere/b.mp4"],
        );
        assert_eq!(engine_pids(fs.root(), 100), [102]);
        // A recorder on PATH is our child itself.
        proc_entry(
            &fs,
            300,
            1,
            "gpu-screen-reco",
            &["gpu-screen-recorder", "-o", "/v/c.mp4"],
        );
        assert_eq!(engine_pids(fs.root(), 300), [300]);
    }

    /// A recorder left running (by an older agent, or one that died) and
    /// writing into our folder is found, so it can be stopped.
    #[test]
    fn a_leftover_recorder_writing_into_our_folder_is_found() {
        let fs = ug_testfs::TestFs::new();
        proc_entry(
            &fs,
            101,
            1480,
            "bwrap",
            &["bwrap", "--", "gpu-screen-recorder"],
        );
        proc_entry(
            &fs,
            102,
            101,
            "gpu-screen-reco",
            &[
                "gpu-screen-recorder",
                "-w",
                "portal",
                "-o",
                "/v/Unix Guardian/Recording 1.mp4",
            ],
        );
        proc_entry(
            &fs,
            103,
            1480,
            "gpu-screen-reco",
            &["gpu-screen-recorder", "-r", "30", "-o", "/v/Unix Guardian"],
        );
        proc_entry(
            &fs,
            200,
            1,
            "gpu-screen-reco",
            &["gpu-screen-recorder", "-o", "/home/u/mine.mp4"],
        );
        proc_entry(
            &fs,
            300,
            1,
            "firefox",
            &["firefox", "-o", "/v/Unix Guardian/x"],
        );
        let mut found = leftover_engines(fs.root(), Path::new("/v/Unix Guardian"), &[]);
        found.sort();
        assert_eq!(found, [102, 103]);
        assert_eq!(
            leftover_engines(fs.root(), Path::new("/v/Unix Guardian"), &[102]),
            [103],
            "ours is not a leftover"
        );
    }

    /// With real processes: the signal reaches a grandchild, and stopping
    /// waits for it to be gone.
    #[test]
    fn interrupting_reaches_a_real_grandchild() {
        let mut child = std::process::Command::new("sh")
            .args(["-c", "sleep 300; true"])
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let proc_root = Path::new("/proc");
        let kids = descendants(proc_root, child.id() as i32);
        let sleeper = *kids
            .iter()
            .find(|p| comm(proc_root, **p).as_deref() == Some("sleep"))
            .expect("sleep is a child of sh");
        assert!(interrupt_and_wait(
            proc_root,
            &[sleeper],
            std::time::Duration::from_secs(5)
        ));
        assert!(!alive(proc_root, sleeper));
        let _ = child.wait();
    }
}
