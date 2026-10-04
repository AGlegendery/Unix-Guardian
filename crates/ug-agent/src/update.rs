//! Looks for a new version (see `ug_core::update`): once a day when the
//! Settings page allows it, and on "Check now". A new version is told once
//! with a notification; installing it is always the user's step.
//!
//! What was checked and told is kept in `update.json`, so a login does not
//! check again within the day or repeat the notification.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use ug_core::update::{self, InstallKind};
use ug_proto::UpdateState;

/// A day between automatic checks.
pub const EVERY_MS: u64 = 24 * 3600 * 1000;

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    checked_unix_ms: u64,
    /// The release already told about.
    told: String,
}

pub struct Updater {
    file: PathBuf,
    /// Where downloads go: `~/.cache/unix-guardian/update`.
    cache: PathBuf,
    /// The newer release's tarball and signature URLs.
    assets: Mutex<Option<(String, String, String)>>,
    root: PathBuf,
    etc: PathBuf,
    state: Mutex<UpdateState>,
    saved: Mutex<Saved>,
}

impl Updater {
    pub fn new(ug_config: &Path, root: &Path) -> Updater {
        let file = ug_config.join("update.json");
        let saved: Saved = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let kind = update::install_kind(root);
        let state = UpdateState {
            current: update::VERSION.into(),
            kind: kind.id().into(),
            how: how(&kind),
            checked_unix_ms: saved.checked_unix_ms,
            ..Default::default()
        };
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"))
            .join("unix-guardian/update");
        Updater {
            file,
            cache,
            assets: Mutex::new(None),
            root: root.into(),
            etc: root.join("etc/unix-guardian"),
            state: Mutex::new(state),
            saved: Mutex::new(saved),
        }
    }

    pub fn state(&self) -> UpdateState {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// An automatic check is due.
    pub fn due(&self, now_ms: u64) -> bool {
        let s = self.state();
        !s.checking && now_ms.saturating_sub(s.checked_unix_ms) >= EVERY_MS
    }

    /// Reads the feed (blocking: call it off the async threads). Returns
    /// the state, and the release to tell about when it is new to the user.
    pub fn check(&self, now_ms: u64) -> (UpdateState, Option<String>) {
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if s.checking {
                return (s.clone(), None);
            }
            s.checking = true;
        }
        let kind = update::install_kind(&self.root);
        let result = update::fetch(&update::feed(&self.etc));
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.checking = false;
        s.checked_unix_ms = now_ms;
        s.kind = kind.id().into();
        s.how = how(&kind);
        let mut tell = None;
        match result {
            Err(e) => s.problem = e,
            Ok(releases) => {
                s.problem.clear();
                let newest = update::newest(&releases, update::VERSION);
                s.latest = newest.map(|r| r.tag_name.clone()).unwrap_or_default();
                s.name = newest.and_then(|r| r.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| s.latest.clone());
                s.notes = newest.and_then(|r| r.body.clone()).unwrap_or_default();
                s.url = newest.map(|r| r.html_url.clone()).unwrap_or_default();
                s.published = newest.and_then(|r| r.published_at.clone()).unwrap_or_default();
                let files = newest.and_then(|r| update::tarball_assets(r, std::env::consts::ARCH)).and_then(|(tar, sig, _)| {
                    Some((tar.browser_download_url.clone(), sig?.browser_download_url.clone(), tar.name.clone()))
                });
                s.can_install = kind == InstallKind::Tarball && files.is_some() && update::trusted_key(&self.etc).is_some();
                *self.assets.lock().unwrap_or_else(|e| e.into_inner()) = files;
                let mut saved = self.saved.lock().unwrap_or_else(|e| e.into_inner());
                if !s.latest.is_empty() && saved.told != s.latest {
                    saved.told = s.latest.clone();
                    tell = Some(s.name.clone());
                }
                saved.checked_unix_ms = now_ms;
                if let Ok(t) = serde_json::to_string_pretty(&*saved) {
                    if let Err(e) = ug_core::paths::write_atomic(&self.file, t.as_bytes()) {
                        tracing::warn!(error = %e, "could not save update.json");
                    }
                }
            }
        }
        (s.clone(), tell)
    }
}

impl Updater {
    fn set(&self, f: impl FnOnce(&mut UpdateState)) {
        f(&mut self.state.lock().unwrap_or_else(|e| e.into_inner()));
    }

    pub fn set_previous(&self, v: String) {
        self.set(|s| s.previous = v);
    }

    /// Starts the install: download the tarball and its signature, then
    /// the service checks and installs them (asking for the password).
    pub fn begin_install(self: &std::sync::Arc<Self>, daemon_session: bool) -> Result<(), String> {
        let s = self.state();
        if !s.can_install {
            return Err(if s.latest.is_empty() { "There is nothing newer to install.".into() } else { format!("Update this copy with: {}", s.how) });
        }
        if !s.step.is_empty() {
            return Err("It is being installed already.".into());
        }
        let Some((tar, sig, name)) = self.assets.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
            return Err("Check for updates first.".into());
        };
        self.set(|s| {
            s.step = "downloading".into();
            s.result.clear();
            s.problem.clear();
        });
        let me = std::sync::Arc::clone(self);
        tokio::spawn(async move {
            let r = me.install(daemon_session, &tar, &sig, &name).await;
            me.set(|s| {
                s.step.clear();
                match r {
                    Ok(m) => s.result = m,
                    Err(e) => s.problem = e,
                }
            });
        });
        Ok(())
    }

    async fn install(&self, daemon_session: bool, tar: &str, sig: &str, name: &str) -> Result<String, String> {
        std::fs::create_dir_all(&self.cache).map_err(|e| format!("{}: {e}", self.cache.display()))?;
        let (tar_path, sig_path) = (self.cache.join(name), self.cache.join(format!("{name}.minisig")));
        for (url, path) in [(tar, &tar_path), (sig, &sig_path)] {
            let out = tokio::process::Command::new("curl")
                .args(["--fail", "--silent", "--show-error", "--location", "--max-time", "1800", "--output"])
                .arg(path)
                .arg(url)
                .stdin(std::process::Stdio::null())
                .output()
                .await
                .map_err(|e| format!("curl is needed to download the update ({e})"))?;
            if !out.status.success() {
                return Err(format!("The download failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
            }
        }
        self.set(|s| s.step = "installing".into());
        let conn = if daemon_session { zbus::Connection::session().await } else { zbus::Connection::system().await }
            .map_err(|e| format!("The Unix Guardian service is not reachable ({e})."))?;
        let d = ug_proto::daemon::UpdateProxy::new(&conn).await.map_err(|e| e.to_string())?;
        let msg = d
            .install(&tar_path.to_string_lossy(), &sig_path.to_string_lossy())
            .await
            .map_err(|e| match e {
                zbus::Error::MethodError(_, Some(m), _) => m,
                e => e.to_string(),
            })?;
        let _ = std::fs::remove_file(&tar_path);
        let _ = std::fs::remove_file(&sig_path);
        Ok(msg)
    }
}

/// The command that updates a copy Unix Guardian does not update itself.
fn how(kind: &InstallKind) -> String {
    match kind {
        InstallKind::Package(cmd) => cmd.clone(),
        InstallKind::Development => "git pull, then: sudo ./scripts/dev-install.sh".into(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_release_is_found_and_told_once() {
        let d = tempfile::tempdir().unwrap();
        let feed = d.path().join("releases.json");
        std::fs::write(
            &feed,
            r#"[{"tag_name":"v99.0.0","name":"Big one","body":"- New things","html_url":"https://x/r","draft":false,"prerelease":false,
                "assets":[{"name":"unix-guardian-99.0.0-x86_64.tar.gz","browser_download_url":"https://x/t"}]}]"#,
        )
        .unwrap();
        let etc = d.path().join("root/etc/unix-guardian");
        std::fs::create_dir_all(&etc).unwrap();
        std::fs::write(etc.join("update.toml"), format!("feed = \"file://{}\"\n", feed.display())).unwrap();
        let u = Updater::new(&d.path().join("cfg"), &d.path().join("root"));
        assert!(u.due(EVERY_MS));
        let (s, tell) = u.check(EVERY_MS);
        assert_eq!((s.latest.as_str(), s.name.as_str(), s.notes.as_str(), s.kind.as_str()), ("v99.0.0", "Big one", "- New things", "none"));
        assert_eq!(tell.as_deref(), Some("Big one"));
        assert!(!s.can_install, "not a tarball install, and no signature");
        assert!(!u.due(EVERY_MS + 1000), "a day apart");
        // A new agent (the next login) remembers both.
        let again = Updater::new(&d.path().join("cfg"), &d.path().join("root"));
        assert!(!again.due(EVERY_MS + 1000));
        assert_eq!(again.check(2 * EVERY_MS).1, None, "told once");
    }

    #[test]
    fn a_failed_check_says_why() {
        let d = tempfile::tempdir().unwrap();
        let etc = d.path().join("etc/unix-guardian");
        std::fs::create_dir_all(&etc).unwrap();
        std::fs::write(etc.join("update.toml"), format!("feed = \"file://{}/none.json\"\n", d.path().display())).unwrap();
        let (s, tell) = Updater::new(&d.path().join("cfg"), d.path()).check(5);
        assert!(s.problem.contains("Could not reach") && s.latest.is_empty() && tell.is_none(), "{s:?}");
    }
}
