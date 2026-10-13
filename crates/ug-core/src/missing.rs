//! What to tell the user when the service or the agent does not answer:
//! start it when it is installed, install it when it is not (only the
//! window was copied, as on a Fedora report where every check skipped).

use std::path::Path;

const INSTALL: &str = "Install everything from the source folder with: bash ./installer.sh";

fn any(root: &Path, files: &[&str]) -> bool {
    files.iter().any(|f| root.join(f).exists())
}

/// The system service's unit, from installer.sh / dev-install.sh or a package.
pub fn daemon_installed(root: &Path) -> bool {
    any(root, &["etc/systemd/system/unix-guardiand.service", "usr/lib/systemd/system/unix-guardiand.service", "lib/systemd/system/unix-guardiand.service"])
}

/// The user agent's unit or its D-Bus activation file.
pub fn agent_installed(root: &Path) -> bool {
    any(
        root,
        &[
            "etc/systemd/user/ug-agent.service",
            "usr/lib/systemd/user/ug-agent.service",
            "usr/local/share/dbus-1/services/org.unixguardian.Agent1.service",
            "usr/share/dbus-1/services/org.unixguardian.Agent1.service",
        ],
    )
}

/// The sentence for a service that does not answer. `root` is `/` outside tests.
pub fn daemon_down(root: &Path) -> String {
    if daemon_installed(root) {
        "The Unix Guardian service is not running. Start it: sudo systemctl start unix-guardiand".into()
    } else {
        format!("The Unix Guardian service is not installed on this computer, only the app is. {INSTALL}")
    }
}

/// The sentence for an agent that does not answer. `root` is `/` outside tests.
pub fn agent_down(root: &Path) -> String {
    if agent_installed(root) {
        "The Unix Guardian agent is not running in this session. Start it: systemctl --user start ug-agent".into()
    } else {
        format!("The Unix Guardian agent is not installed on this computer, only the app is. {INSTALL}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_window_copied_says_run_the_installer() {
        let d = tempfile::tempdir().unwrap();
        assert!(daemon_down(d.path()).contains("bash ./installer.sh"));
        assert!(agent_down(d.path()).contains("bash ./installer.sh"));
    }

    #[test]
    fn installed_but_stopped_says_start_it() {
        let d = tempfile::tempdir().unwrap();
        for f in ["etc/systemd/system/unix-guardiand.service", "usr/lib/systemd/user/ug-agent.service"] {
            std::fs::create_dir_all(d.path().join(f).parent().unwrap()).unwrap();
            std::fs::write(d.path().join(f), "").unwrap();
        }
        assert!(daemon_down(d.path()).contains("sudo systemctl start unix-guardiand"));
        assert!(agent_down(d.path()).contains("systemctl --user start ug-agent"));
    }
}
