//! `org.unixguardian.Daemon1.Update`: installs a release the user chose,
//! and takes it back.
//!
//! The agent downloads the release tarball and its signature as the user;
//! the daemon reads each file once, checks the bytes against the trusted
//! minisign key (`ug_core::update::verify`) and only then unpacks those
//! same bytes into its own directory, so nothing the user can change
//! afterwards is run. Only an install made by the release's install.sh is
//! updated this way; a package belongs to its package manager.
//!
//! Before installing, the files of the current version (the list install.sh
//! keeps) are copied aside with the new install.sh, which installs whatever
//! is under `files/`: Rollback runs that copy. install.sh restarts this
//! daemon, so it runs as a transient systemd unit, not as our child.

use crate::polkit::{action, Gate};
use std::path::{Path, PathBuf};

/// Biggest download accepted (the tarball is ~10 MB).
const MAX_BYTES: u64 = 256 << 20;

pub struct UpdateIface {
    /// `/`, or `--sysroot` for a development run.
    pub root: PathBuf,
    /// `--sysroot` given: install.sh runs with `--root`, at once.
    pub scratch: bool,
    /// Development on the real system: refuse.
    pub refused: Option<String>,
    /// `/var/lib/unix-guardian/updates`.
    pub dir: PathBuf,
    pub gate: Gate,
    pub busy: tokio::sync::Mutex<()>,
}

#[zbus::interface(name = "org.unixguardian.Daemon1.Update")]
impl UpdateIface {
    /// The version a rollback returns to; "" when there is none.
    async fn previous(&self) -> String {
        std::fs::read_to_string(self.dir.join("previous/VERSION"))
            .map(|v| v.trim().to_string())
            .unwrap_or_default()
    }

    /// Installs the release tarball at `tarball`, signed by `signature`
    /// (paths the daemon can read). Returns what happens next.
    async fn install(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        tarball: &str,
        signature: &str,
    ) -> zbus::fdo::Result<String> {
        if let Some(r) = &self.refused {
            return Err(zbus::fdo::Error::NotSupported(r.clone()));
        }
        let kind = ug_core::update::install_kind(&self.root);
        if kind != ug_core::update::InstallKind::Tarball {
            return Err(zbus::fdo::Error::NotSupported(format!(
                "This copy was not installed by a release's install.sh ({}), so Unix Guardian does not replace its files.",
                kind.id()
            )));
        }
        let key = ug_core::update::trusted_key(&self.root.join("etc/unix-guardian")).ok_or_else(
            || {
                zbus::fdo::Error::NotSupported(
                    "This build has no update key, so it cannot check a download.".into(),
                )
            },
        )?;
        self.gate.authorize(&hdr, action::UPDATE).await?;
        let _busy = self.busy.try_lock().map_err(|_| {
            zbus::fdo::Error::Failed("An update is being installed already.".into())
        })?;
        let (tarball, signature) = (PathBuf::from(tarball), PathBuf::from(signature));
        let (dir, root, scratch) = (self.dir.clone(), self.root.clone(), self.scratch);
        tokio::task::spawn_blocking(move || -> Result<String, String> {
            let data = read_limited(&tarball)?;
            let sig = std::fs::read_to_string(&signature).map_err(|e| format!("{}: {e}", signature.display()))?;
            ug_core::update::verify(&data, &sig, &key)?;
            let release = unpack(&data, &dir)?;
            keep_previous(&root, &release, &dir.join("previous"))?;
            run_install(&release, &root, scratch)?;
            let name = release.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            tracing::warn!(release = %name, "installing an update");
            Ok(format!("Installing {name}. Unix Guardian restarts by itself in a moment; open the window again afterwards."))
        })
        .await
        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
        .map_err(zbus::fdo::Error::Failed)
    }

    /// Puts the version from before the last update back.
    async fn rollback(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<String> {
        if let Some(r) = &self.refused {
            return Err(zbus::fdo::Error::NotSupported(r.clone()));
        }
        let prev = self.dir.join("previous");
        if !prev.join("install.sh").exists() {
            return Err(zbus::fdo::Error::NotSupported(
                "There is no earlier version to go back to.".into(),
            ));
        }
        self.gate.authorize(&hdr, action::UPDATE).await?;
        let _busy = self.busy.try_lock().map_err(|_| {
            zbus::fdo::Error::Failed("An update is being installed already.".into())
        })?;
        let version = std::fs::read_to_string(prev.join("VERSION")).unwrap_or_default();
        let (root, scratch) = (self.root.clone(), self.scratch);
        tokio::task::spawn_blocking(move || run_install(&prev, &root, scratch))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)?;
        tracing::warn!(version = version.trim(), "rolling back an update");
        Ok(format!(
            "Going back to {}. Unix Guardian restarts by itself in a moment.",
            version.trim()
        ))
    }
}

fn read_limited(p: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let f = std::fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let mut data = Vec::new();
    f.take(MAX_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_BYTES {
        return Err("The download is far bigger than a release; not installed.".into());
    }
    Ok(data)
}

/// Unpacks the checked bytes into `<dir>/<release folder>`; the folder must
/// hold install.sh and files/.
fn unpack(data: &[u8], dir: &Path) -> Result<PathBuf, String> {
    use std::io::Write;
    let work = dir.join("incoming");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let mut child = std::process::Command::new("tar")
        .args([
            "-xzf",
            "-",
            "--no-same-owner",
            "--no-same-permissions",
            "-C",
        ])
        .arg(&work)
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run tar: {e}"))?;
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(data)
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "The release could not be unpacked: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let tops: Vec<PathBuf> = std::fs::read_dir(&work)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .collect();
    let [top] = tops.as_slice() else {
        return Err("The release is not one folder.".into());
    };
    if !top.join("install.sh").is_file() || !top.join("files").is_dir() {
        return Err("The release has no install.sh and files/; not installed.".into());
    }
    let name = top
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    if !name.starts_with("unix-guardian-") {
        return Err(format!("Unexpected release folder {name:?}."));
    }
    let dest = dir.join(&name);
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(top, &dest).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&work);
    Ok(dest)
}

/// Copies the installed files (install.sh's list) and the new install.sh
/// to `prev`, so running `prev/install.sh` brings this version back.
fn keep_previous(root: &Path, release: &Path, prev: &Path) -> Result<(), String> {
    let list = std::fs::read_to_string(root.join("usr/local/share/unix-guardian/installed-files"))
        .map_err(|e| format!("the list of installed files could not be read: {e}"))?;
    let _ = std::fs::remove_dir_all(prev);
    for f in list.lines().map(str::trim).filter(|f| f.starts_with('/')) {
        // The list itself is written by install.sh.
        if f.ends_with("/installed-files") {
            continue;
        }
        let src = root.join(f.trim_start_matches('/'));
        let dst = prev.join("files").join(f.trim_start_matches('/'));
        if let Some(d) = dst.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        // A file missing now is simply not brought back.
        if let Err(e) = std::fs::copy(&src, &dst) {
            tracing::warn!(file = f, error = %e, "not kept for the rollback");
        }
    }
    std::fs::copy(release.join("install.sh"), prev.join("install.sh"))
        .map_err(|e| e.to_string())?;
    std::fs::write(
        prev.join("VERSION"),
        format!("{}\n", ug_core::update::VERSION),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Runs a release folder's install.sh: into the scratch root at once, or
/// on the real system as its own systemd unit (it restarts this daemon).
fn run_install(release: &Path, root: &Path, scratch: bool) -> Result<(), String> {
    let script = release.join("install.sh");
    let out = if scratch {
        std::process::Command::new("bash")
            .arg(&script)
            .arg("--root")
            .arg(root)
            .stdin(std::process::Stdio::null())
            .output()
    } else {
        let unit = format!("unix-guardian-update-{}", std::process::id());
        std::process::Command::new("systemd-run")
            .args([
                "--no-block",
                "--collect",
                "--quiet",
                "--unit",
                &unit,
                "--",
                "bash",
            ])
            .arg(&script)
            .stdin(std::process::Stdio::null())
            .output()
    }
    .map_err(|e| format!("could not start the installer: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "The installer failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A release tarball as build-packages.sh makes it.
    fn release(work: &Path, name: &str, version_text: &str) -> Vec<u8> {
        let top = work.join(name);
        std::fs::create_dir_all(top.join("files/usr/local/bin")).unwrap();
        std::fs::write(top.join("files/usr/local/bin/ug"), version_text).unwrap();
        std::fs::copy(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../packaging/install.sh"),
            top.join("install.sh"),
        )
        .unwrap();
        let out = std::process::Command::new("tar")
            .arg("-czf")
            .arg("-")
            .arg("-C")
            .arg(work)
            .arg(name)
            .output()
            .unwrap();
        assert!(out.status.success());
        out.stdout
    }

    #[test]
    fn a_release_is_unpacked_installed_and_rolled_back() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("root");
        // Version 1, installed by its install.sh.
        let v1 = release(&d.path().join("w1"), "unix-guardian-1-x86_64", "one");
        let r1 = unpack(&v1, &d.path().join("u1")).unwrap();
        run_install(&r1, &root, true).unwrap();
        assert_eq!(
            ug_core::update::install_kind(&root),
            ug_core::update::InstallKind::Tarball
        );
        assert_eq!(
            std::fs::read_to_string(root.join("usr/local/bin/ug")).unwrap(),
            "one"
        );
        // Version 2 over it, keeping version 1 aside.
        let dir = d.path().join("updates");
        let v2 = release(&d.path().join("w2"), "unix-guardian-2-x86_64", "two");
        let r2 = unpack(&v2, &dir).unwrap();
        keep_previous(&root, &r2, &dir.join("previous")).unwrap();
        run_install(&r2, &root, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("usr/local/bin/ug")).unwrap(),
            "two"
        );
        // And back.
        run_install(&dir.join("previous"), &root, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("usr/local/bin/ug")).unwrap(),
            "one"
        );
    }

    #[test]
    fn something_that_is_not_a_release_is_refused() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("w/other")).unwrap();
        std::fs::write(d.path().join("w/other/x"), "x").unwrap();
        let out = std::process::Command::new("tar")
            .args(["-czf", "-", "-C"])
            .arg(d.path().join("w"))
            .arg("other")
            .output()
            .unwrap();
        assert!(unpack(&out.stdout, &d.path().join("u"))
            .unwrap_err()
            .contains("no install.sh"));
        assert!(unpack(b"not a tarball", &d.path().join("u"))
            .unwrap_err()
            .contains("could not be unpacked"));
    }
}
