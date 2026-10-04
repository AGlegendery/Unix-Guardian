//! Updates from GitHub Releases: is there a newer version, what changed,
//! and which file installs it here.
//!
//! Nothing is ever installed without the user asking (roadmap 10.6). The
//! agent checks (a day apart, or on "Check now") and tells; installing is
//! the daemon's job, after the download's signature is checked. A copy
//! installed from a distribution package is updated by that package
//! manager, never by replacing its files.
//!
//! The feed is GitHub's releases API, read with `curl` (it honours the
//! user's proxy variables, and keeps a TLS stack out of the agent). It can
//! be pointed elsewhere with `feed = "…"` in `/etc/unix-guardian/update.toml`
//! or `UG_UPDATE_FEED` (tests use a `file://` feed).

use serde::Deserialize;
use std::cmp::Ordering;
use std::path::Path;

/// This program's version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// GitHub's API for the project's releases.
pub const DEFAULT_FEED: &str = "https://api.github.com/repos/AGlegendery/Unix-Guardian/releases?per_page=20";

/// The feed to read: `UG_UPDATE_FEED`, else `feed` in `<etc>/update.toml`,
/// else GitHub.
pub fn feed(etc: &Path) -> String {
    if let Some(f) = std::env::var("UG_UPDATE_FEED").ok().filter(|f| !f.is_empty()) {
        return f;
    }
    #[derive(Deserialize)]
    struct Conf {
        feed: Option<String>,
    }
    std::fs::read_to_string(etc.join("update.toml"))
        .ok()
        .and_then(|t| toml::from_str::<Conf>(&t).ok())
        .and_then(|c| c.feed)
        .filter(|f| !f.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_FEED.to_string())
}

/// One release as GitHub describes it (only what is used).
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

/// A version as `MAJOR.MINOR.PATCH[-pre]`, with or without a leading `v`;
/// `0.1.0beta1` and `0.1.0~beta1` (the package spellings) are read too.
#[derive(Debug, Clone)]
pub struct Version {
    core: Vec<u64>,
    /// Empty for a release; a pre-release sorts before it.
    pre: Vec<String>,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim().trim_start_matches(['v', 'V']);
        let s = s.split('+').next()?;
        // Where the numbers end, the pre-release starts (`-beta.1`,
        // `~beta1`, `beta1`).
        let end = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
        let (core, pre) = s.split_at(end);
        let core: Vec<u64> = core.trim_end_matches('.').split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
        if core.is_empty() || core.len() > 4 {
            return None;
        }
        let pre = pre.trim_start_matches(['-', '~', '.']);
        let mut parts = Vec::new();
        // `beta1` and `beta.1` are the same: letters and digits apart.
        for chunk in pre.split(['.', '-']).filter(|p| !p.is_empty()) {
            let mut cur = String::new();
            for c in chunk.chars() {
                if !cur.is_empty() && cur.chars().last().unwrap().is_ascii_digit() != c.is_ascii_digit() {
                    parts.push(std::mem::take(&mut cur));
                }
                cur.push(c.to_ascii_lowercase());
            }
            parts.push(cur);
        }
        Some(Version { core, pre: parts })
    }

    pub fn is_pre(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl Ord for Version {
    fn cmp(&self, o: &Self) -> Ordering {
        let n = self.core.len().max(o.core.len());
        for i in 0..n {
            let (a, b) = (self.core.get(i).copied().unwrap_or(0), o.core.get(i).copied().unwrap_or(0));
            if a != b {
                return a.cmp(&b);
            }
        }
        match (self.pre.is_empty(), o.pre.is_empty()) {
            (true, true) => return Ordering::Equal,
            (true, false) => return Ordering::Greater,
            (false, true) => return Ordering::Less,
            _ => {}
        }
        for (a, b) in self.pre.iter().zip(&o.pre) {
            let c = match (a.parse::<u64>(), b.parse::<u64>()) {
                (Ok(x), Ok(y)) => x.cmp(&y),
                (Ok(_), Err(_)) => Ordering::Less,
                (Err(_), Ok(_)) => Ordering::Greater,
                _ => a.cmp(b),
            };
            if c != Ordering::Equal {
                return c;
            }
        }
        self.pre.len().cmp(&o.pre.len())
    }
}

/// `1.2` and `1.2.0` are the same version.
impl PartialEq for Version {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// The newest release worth offering over `current`: never a draft, and a
/// pre-release only to someone already running one (a beta tester).
pub fn newest<'a>(releases: &'a [Release], current: &str) -> Option<&'a Release> {
    let cur = Version::parse(current)?;
    releases
        .iter()
        .filter(|r| !r.draft && (!r.prerelease || cur.is_pre()))
        .filter_map(|r| Some((Version::parse(&r.tag_name)?, r)))
        .filter(|(v, _)| *v > cur)
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, r)| r)
}

/// How this copy was installed, which decides how it is updated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallKind {
    /// From a distribution package (`/usr/bin`): its package manager updates it.
    Package(String),
    /// The release tarball's install.sh (`/usr/local`, with its file list):
    /// Unix Guardian can update it.
    Tarball,
    /// `scripts/dev-install.sh` from a source tree: `git pull` and run it again.
    Development,
    /// Not installed (running from a build folder).
    None,
}

impl InstallKind {
    pub fn id(&self) -> &'static str {
        match self {
            InstallKind::Package(_) => "package",
            InstallKind::Tarball => "tarball",
            InstallKind::Development => "development",
            InstallKind::None => "none",
        }
    }
}

/// Finds the install under `root` (`/` normally).
pub fn install_kind(root: &Path) -> InstallKind {
    if root.join("usr/bin/ug-daemon").exists() {
        let how = if root.join("var/lib/dpkg/info/unix-guardian.list").exists() {
            "sudo apt update && sudo apt install --only-upgrade unix-guardian"
        } else if root.join("var/lib/pacman/local").read_dir().into_iter().flatten().flatten().any(|e| {
            e.file_name().to_string_lossy().starts_with("unix-guardian-")
        }) {
            "yay -Syu unix-guardian   (or your AUR helper; makepkg -si for a local PKGBUILD)"
        } else {
            "update the unix-guardian package with your package manager"
        };
        return InstallKind::Package(how.to_string());
    }
    if root.join("usr/local/share/unix-guardian/installed-files").exists() {
        return InstallKind::Tarball;
    }
    if root.join("etc/systemd/system/unix-guardiand.service").exists() && root.join("usr/local/bin/ug-daemon").exists() {
        return InstallKind::Development;
    }
    InstallKind::None
}

/// The files of a release that update a tarball install: the tarball, its
/// minisign signature and the checksum list, if the release has them.
pub fn tarball_assets<'a>(r: &'a Release, arch: &str) -> Option<(&'a Asset, Option<&'a Asset>, Option<&'a Asset>)> {
    let tar = r.assets.iter().find(|a| a.name.starts_with("unix-guardian-") && a.name.ends_with(&format!("-{arch}.tar.gz")))?;
    let sig = r.assets.iter().find(|a| a.name == format!("{}.minisig", tar.name));
    let sums = r.assets.iter().find(|a| a.name == "SHA256SUMS");
    Some((tar, sig, sums))
}

/// Reads the feed: GitHub's JSON list of releases. Errors are sentences
/// for the user (offline, a private or missing repository, rate limits).
pub fn fetch(feed: &str) -> Result<Vec<Release>, String> {
    let out = std::process::Command::new("curl")
        .args(["--silent", "--show-error", "--location", "--max-time", "20", "--write-out", "\n%{http_code}"])
        .args(["--header", "Accept: application/vnd.github+json", "--user-agent"])
        .arg(format!("unix-guardian/{VERSION}"))
        .arg(feed)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("curl is needed to check for updates ({e})"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or((&text, ""));
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(format!("Could not reach the update server ({err}). Are you online, or behind a proxy?"));
    }
    match code.trim() {
        "200" | "000" | "" => {}
        "404" => return Err("No releases found: the repository is private or has none yet.".into()),
        "403" | "429" => return Err("GitHub is limiting requests; try again in an hour.".into()),
        c => return Err(format!("The update server answered {c}.")),
    }
    parse_feed(body)
}

pub fn parse_feed(body: &str) -> Result<Vec<Release>, String> {
    serde_json::from_str::<Vec<Release>>(body).map_err(|e| format!("The release list could not be read ({e})."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap_or_else(|| panic!("{s}"))
    }

    #[test]
    fn versions_sort_like_semver_and_read_the_package_spellings() {
        assert!(v("0.1.0-beta.1") < v("0.1.0"));
        assert!(v("0.1.0-beta.2") > v("0.1.0-beta.1"));
        assert!(v("0.1.0-beta.10") > v("0.1.0-beta.9"), "numbers, not text");
        assert!(v("0.1.0-rc.1") > v("0.1.0-beta.3"));
        assert!(v("v0.2.0") > v("0.1.9"));
        assert_eq!(v("0.1.0beta1"), v("0.1.0-beta.1"));
        assert_eq!(v("0.1.0~beta1"), v("v0.1.0-beta1"));
        assert_eq!(v("1.2"), v("1.2.0"));
        assert!(Version::parse("nightly").is_none());
    }

    const FEED: &str = r#"[
      {"tag_name":"v0.3.0","name":"Draft","draft":true,"prerelease":false,"html_url":"u","assets":[]},
      {"tag_name":"v0.2.0-beta.1","name":"Beta","draft":false,"prerelease":true,"html_url":"u","assets":[]},
      {"tag_name":"v0.1.1","name":"0.1.1","body":"Fixes","draft":false,"prerelease":false,"html_url":"https://github.com/x/releases/v0.1.1",
       "assets":[{"name":"unix-guardian-0.1.1-x86_64.tar.gz","browser_download_url":"https://x/t.tgz","size":9},
                 {"name":"unix-guardian-0.1.1-x86_64.tar.gz.minisig","browser_download_url":"https://x/t.sig","size":1},
                 {"name":"SHA256SUMS","browser_download_url":"https://x/s","size":1}]},
      {"tag_name":"v0.1.0","draft":false,"prerelease":false,"html_url":"u","assets":[]}
    ]"#;

    #[test]
    fn the_newest_release_skips_drafts_and_offers_betas_only_to_beta_users() {
        let r = parse_feed(FEED).unwrap();
        assert_eq!(newest(&r, "0.1.0").map(|r| r.tag_name.as_str()), Some("v0.1.1"));
        assert_eq!(newest(&r, "0.1.0-beta.1").map(|r| r.tag_name.as_str()), Some("v0.2.0-beta.1"));
        assert_eq!(newest(&r, "0.1.1"), None);
        assert_eq!(newest(&r, "0.3.0"), None);
        let (tar, sig, sums) = tarball_assets(newest(&r, "0.1.0").unwrap(), "x86_64").unwrap();
        assert_eq!(tar.browser_download_url, "https://x/t.tgz");
        assert!(sig.is_some() && sums.is_some());
        assert!(tarball_assets(&r[1], "x86_64").is_none());
    }

    #[test]
    fn the_install_kind_decides_how_to_update() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(install_kind(d.path()), InstallKind::None);
        std::fs::create_dir_all(d.path().join("usr/local/share/unix-guardian")).unwrap();
        std::fs::write(d.path().join("usr/local/share/unix-guardian/installed-files"), "").unwrap();
        assert_eq!(install_kind(d.path()), InstallKind::Tarball);
        std::fs::create_dir_all(d.path().join("usr/bin")).unwrap();
        std::fs::write(d.path().join("usr/bin/ug-daemon"), "").unwrap();
        std::fs::create_dir_all(d.path().join("var/lib/dpkg/info")).unwrap();
        std::fs::write(d.path().join("var/lib/dpkg/info/unix-guardian.list"), "").unwrap();
        assert!(matches!(install_kind(d.path()), InstallKind::Package(c) if c.contains("apt")));
    }

    #[test]
    fn a_file_feed_is_read_and_a_missing_one_is_explained() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("releases.json");
        std::fs::write(&f, FEED).unwrap();
        assert_eq!(fetch(&format!("file://{}", f.display())).unwrap().len(), 4);
        let e = fetch(&format!("file://{}/nope.json", d.path().display())).unwrap_err();
        assert!(e.contains("Could not reach"), "{e}");
    }
}
