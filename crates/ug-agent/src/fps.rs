//! FPS of the running game, read from MangoHud's CSV log.
//!
//! Linux has no system-wide frame counter: frames are counted inside the game
//! by a Vulkan/OpenGL layer. MangoHud is that layer on most distributions,
//! and it appends one CSV row per `log_interval` while it logs, flushing each
//! row. Setting up adds a marked block to the user's MangoHud.conf that
//! starts logging into our folder whenever a game runs with MangoHud; undo
//! removes exactly that block.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use ug_proto::FpsReading;

const BEGIN: &str = "# >>> Unix Guardian: FPS reading (remove with Unix Guardian's Undo)";
const END: &str = "# <<< Unix Guardian";
/// A log not written for this long belongs to a game that has ended.
const FRESH: Duration = Duration::from_secs(3);

pub struct Fps {
    /// Where MangoHud writes for us.
    pub folder: PathBuf,
    /// `~/.config/MangoHud/MangoHud.conf`.
    pub config: PathBuf,
    /// Where to look for the MangoHud program and Vulkan layer.
    pub root: PathBuf,
}

impl Fps {
    pub fn for_user(config_home: &Path, data: &Path) -> Fps {
        Fps { folder: data.join("fps"), config: config_home.join("MangoHud/MangoHud.conf"), root: PathBuf::from("/") }
    }

    fn installed(&self) -> bool {
        let layer_dirs = ["usr/share/vulkan/implicit_layer.d", "etc/vulkan/implicit_layer.d"];
        let has_layer = layer_dirs.iter().any(|d| {
            std::fs::read_dir(self.root.join(d))
                .into_iter()
                .flatten()
                .flatten()
                .any(|e| e.file_name().to_string_lossy().to_lowercase().contains("mangohud"))
        });
        has_layer || ["usr/bin/mangohud", "usr/local/bin/mangohud"].iter().any(|p| self.root.join(p).exists())
    }

    pub fn set_up(&self) -> bool {
        std::fs::read_to_string(&self.config).is_ok_and(|c| c.contains(BEGIN))
    }

    fn install_hint(&self) -> String {
        let has = |p: &str| self.root.join(p).exists();
        if has("usr/bin/apt") {
            "sudo apt install mangohud"
        } else if has("usr/bin/dnf") {
            "sudo dnf install mangohud"
        } else if has("usr/bin/pacman") {
            "sudo pacman -S mangohud"
        } else if has("usr/bin/zypper") {
            "sudo zypper install mangohud"
        } else {
            "flatpak install flathub org.freedesktop.Platform.VulkanLayer.MangoHud"
        }
        .into()
    }

    /// Adds (or with `on == false` removes) our block. The first change keeps
    /// a copy of the user's file as `MangoHud.conf.ug-backup`.
    pub fn configure(&self, on: bool) -> Result<(), String> {
        let old = std::fs::read_to_string(&self.config).unwrap_or_default();
        let kept = strip_block(&old);
        let new = if on {
            std::fs::create_dir_all(&self.folder).map_err(|e| e.to_string())?;
            let sep = if kept.is_empty() || kept.ends_with('\n') { "" } else { "\n" };
            format!(
                "{kept}{sep}{BEGIN}\noutput_folder={}\nautostart_log=1\nlog_interval=200\n{END}\n",
                self.folder.display()
            )
        } else {
            kept
        };
        if new == old {
            return Ok(());
        }
        if let Some(dir) = self.config.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let backup = self.config.with_extension("conf.ug-backup");
        if on && !old.trim().is_empty() && !old.contains(BEGIN) && !backup.exists() {
            std::fs::write(&backup, &old).map_err(|e| e.to_string())?;
        }
        if new.trim().is_empty() && !backup.exists() {
            // We created the file; leave no empty file behind.
            return match std::fs::remove_file(&self.config) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        std::fs::write(&self.config, new).map_err(|e| e.to_string())
    }

    pub fn read(&self, now: SystemTime) -> FpsReading {
        let mut r = FpsReading { fps: f64::NAN, frametime_ms: f64::NAN, low_1pct_fps: f64::NAN, ..Default::default() };
        if !self.installed() {
            r.state = "not-installed".into();
            r.install_hint = self.install_hint();
            return r;
        }
        if !self.set_up() {
            r.state = "not-set-up".into();
            return r;
        }
        r.state = "idle".into();
        let Some((path, modified)) = newest_log(&self.folder) else { return r };
        if now.duration_since(modified).unwrap_or_default() > FRESH {
            return r;
        }
        let Some(rows) = tail_rows(&path) else { return r };
        if rows.is_empty() {
            return r;
        }
        let end_ms = modified.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        fill(&mut r, &rows, end_ms);
        r.game = game_name(&path);
        r.state = "live".into();
        r
    }

    /// Deletes our logs older than a day; MangoHud never does.
    pub fn prune(&self, now: SystemTime) {
        for e in std::fs::read_dir(&self.folder).into_iter().flatten().flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|m| now.duration_since(m).unwrap_or_default() > Duration::from_secs(86_400));
            if old && e.path().extension().is_some_and(|x| x == "csv") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

fn strip_block(s: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in s.lines() {
        if line == BEGIN {
            inside = true;
        } else if inside && line == END {
            inside = false;
        } else if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn newest_log(dir: &Path) -> Option<(PathBuf, SystemTime)> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.ends_with(".csv") && !n.ends_with("_summary.csv")
        })
        .filter_map(|e| Some((e.path(), e.metadata().ok()?.modified().ok()?)))
        .max_by_key(|(_, m)| *m)
}

/// (fps, frametime ms, elapsed ns) of the rows in the last part of the log.
fn tail_rows(path: &Path) -> Option<Vec<(f64, f64, u64)>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    // Ten seconds at 5 rows a second is ~50 rows of ~100 bytes.
    let start = len.saturating_sub(16 * 1024);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = String::new();
    f.read_to_string(&mut buf).ok()?;
    let mut lines: Vec<&str> = buf.lines().collect();
    if start > 0 {
        lines.remove(0); // probably cut in half
    }
    Some(
        lines
            .iter()
            .filter_map(|l| {
                let f: Vec<&str> = l.split(',').collect();
                let fps: f64 = f.first()?.trim().parse().ok()?;
                let ft: f64 = f.get(1)?.trim().parse().ok()?;
                let elapsed: u64 = f.last()?.trim().parse().ok()?;
                Some((fps, ft, elapsed))
            })
            .collect(),
    )
}

fn fill(r: &mut FpsReading, rows: &[(f64, f64, u64)], end_ms: u64) {
    let last_elapsed = rows.last().map(|x| x.2).unwrap_or(0);
    // MangoHud's elapsed column is nanoseconds since logging started.
    let window: Vec<&(f64, f64, u64)> =
        rows.iter().filter(|x| last_elapsed.saturating_sub(x.2) <= 10_000_000_000).collect();
    r.recent = window
        .iter()
        .map(|x| (end_ms.saturating_sub(last_elapsed.saturating_sub(x.2) / 1_000_000), x.0))
        .collect();
    let last_second: Vec<&&(f64, f64, u64)> =
        window.iter().filter(|x| last_elapsed.saturating_sub(x.2) <= 1_000_000_000).collect();
    let n = last_second.len().max(1) as f64;
    r.fps = last_second.iter().map(|x| x.0).sum::<f64>() / n;
    r.frametime_ms = last_second.iter().map(|x| x.1).sum::<f64>() / n;
    let mut ft: Vec<f64> = window.iter().map(|x| x.1).filter(|v| *v > 0.0).collect();
    ft.sort_by(|a, b| b.total_cmp(a));
    if !ft.is_empty() {
        let idx = ((ft.len() as f64) * 0.01).floor() as usize;
        r.low_1pct_fps = 1000.0 / ft[idx.min(ft.len() - 1)];
    }
}

/// MangoHud names logs `<program>_<YYYY-MM-DD_HH-MM-SS>.csv`.
fn game_name(path: &Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let parts: Vec<&str> = stem.rsplitn(3, '_').collect();
    if parts.len() == 3 && parts[0].contains('-') && parts[1].contains('-') {
        parts[2].to_string()
    } else {
        stem
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Fps) {
        let d = tempfile::tempdir().unwrap();
        let fps = Fps {
            folder: d.path().join("data/fps"),
            config: d.path().join("config/MangoHud/MangoHud.conf"),
            root: d.path().join("root"),
        };
        std::fs::create_dir_all(d.path().join("root/usr/share/vulkan/implicit_layer.d")).unwrap();
        (d, fps)
    }

    const HEAD: &str = "os,cpu,gpu,ram,kernel,driver,cpuscheduler\nLinux,i5,UHD,16GB,7.0,mesa,\n\
        fps,frametime,cpu_load,cpu_power,gpu_load,cpu_temp,gpu_temp,gpu_core_clock,gpu_mem_clock,gpu_vram_used,gpu_power,ram_used,swap_used,process_rss,cpu_mhz,elapsed\n";

    #[test]
    fn says_how_to_install_when_mangohud_is_missing() {
        let (d, fps) = fixture();
        std::fs::create_dir_all(d.path().join("root/usr/bin")).unwrap();
        std::fs::write(d.path().join("root/usr/bin/apt"), "").unwrap();
        let r = fps.read(SystemTime::now());
        assert_eq!(r.state, "not-installed");
        assert_eq!(r.install_hint, "sudo apt install mangohud");
    }

    #[test]
    fn set_up_adds_one_block_and_undo_restores_the_file_exactly() {
        let (d, fps) = fixture();
        std::fs::write(d.path().join("root/usr/share/vulkan/implicit_layer.d/MangoHud.x86_64.json"), "{}").unwrap();
        std::fs::create_dir_all(fps.config.parent().unwrap()).unwrap();
        let mine = "fps_limit=144\nposition=top-right";
        std::fs::write(&fps.config, mine).unwrap();
        assert_eq!(fps.read(SystemTime::now()).state, "not-set-up");

        fps.configure(true).unwrap();
        fps.configure(true).unwrap();
        let c = std::fs::read_to_string(&fps.config).unwrap();
        assert_eq!(c.matches(BEGIN).count(), 1);
        assert!(c.starts_with("fps_limit=144\nposition=top-right\n"));
        assert!(c.contains(&format!("output_folder={}", fps.folder.display())));
        assert_eq!(fps.read(SystemTime::now()).state, "idle");
        assert_eq!(std::fs::read_to_string(fps.config.with_extension("conf.ug-backup")).unwrap(), mine);

        fps.configure(false).unwrap();
        assert_eq!(std::fs::read_to_string(&fps.config).unwrap(), format!("{mine}\n"));
    }

    #[test]
    fn undo_removes_a_file_we_created() {
        let (d, fps) = fixture();
        std::fs::write(d.path().join("root/usr/share/vulkan/implicit_layer.d/MangoHud.json"), "{}").unwrap();
        fps.configure(true).unwrap();
        assert!(fps.config.exists());
        fps.configure(false).unwrap();
        assert!(!fps.config.exists());
    }

    #[test]
    fn reads_fps_and_1pct_low_from_a_live_log() {
        let (d, fps) = fixture();
        std::fs::write(d.path().join("root/usr/share/vulkan/implicit_layer.d/MangoHud.json"), "{}").unwrap();
        fps.configure(true).unwrap();
        let mut log = HEAD.to_string();
        // 50 rows, 200 ms apart: 60 FPS with one 50 ms stutter.
        for i in 0..50u64 {
            let (f, ft) = if i == 30 { (20.0, 50.0) } else { (60.0, 16.7) };
            log += &format!("{f},{ft},10,5,30,50,45,900,1200,800,10,4.1,0,500,2400,{}\n", i * 200_000_000);
        }
        std::fs::write(fps.folder.join("cs2_2026-09-30_20-11-05.csv"), &log).unwrap();
        std::fs::write(fps.folder.join("cs2_2026-09-30_20-11-05_summary.csv"), "x").unwrap();
        let r = fps.read(SystemTime::now());
        assert_eq!(r.state, "live");
        assert_eq!(r.game, "cs2");
        assert!((r.fps - 60.0).abs() < 0.01, "{}", r.fps);
        assert!((r.low_1pct_fps - 20.0).abs() < 0.01, "{}", r.low_1pct_fps);
        assert_eq!(r.recent.len(), 50);
        assert!(r.recent.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn a_log_that_stopped_growing_is_idle() {
        let (d, fps) = fixture();
        std::fs::write(d.path().join("root/usr/share/vulkan/implicit_layer.d/MangoHud.json"), "{}").unwrap();
        fps.configure(true).unwrap();
        std::fs::write(fps.folder.join("game_2026-09-30_20-11-05.csv"), format!("{HEAD}60,16.6,0,0,0,0,0,0,0,0,0,0,0,0,0,0\n")).unwrap();
        let r = fps.read(SystemTime::now() + Duration::from_secs(10));
        assert_eq!(r.state, "idle");
        assert!(r.fps.is_nan());
    }
}
