//! What the user asked for: block an app, or cap its upload or download.
//!
//! Rules name the app as the interfaces show it (`firefox-esr`, `steam`),
//! not a cgroup: a restarted app gets a new cgroup (`app-firefox-5678.scope`)
//! and must be caught again. Saved as JSON in the daemon's state directory.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum When {
    Always,
    /// Only while game mode is on.
    Gaming,
}

impl When {
    pub fn parse(s: &str) -> Result<When, String> {
        match s {
            "always" => Ok(When::Always),
            "gaming" => Ok(When::Gaming),
            _ => Err(format!("unknown time \"{s}\"; use always or gaming")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            When::Always => "always",
            When::Gaming => "gaming",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub app: String,
    pub block: bool,
    /// Kilobits per second; 0 means no cap.
    pub up_kbps: u32,
    pub down_kbps: u32,
    pub when: When,
}

impl Rule {
    pub fn is_limit(&self) -> bool {
        !self.block && (self.up_kbps > 0 || self.down_kbps > 0)
    }

    fn applies(&self, gaming: bool) -> bool {
        self.when == When::Always || gaming
    }
}

/// Lowest and highest caps accepted, kbit/s: below 64 nothing loads, above
/// 10 Gbit/s is no cap at all.
pub const MIN_KBPS: u32 = 64;
pub const MAX_KBPS: u32 = 10_000_000;

pub struct Rules {
    path: PathBuf,
    pub list: Vec<Rule>,
}

impl Rules {
    /// A missing or unreadable file is an empty list (and says why).
    pub fn load(path: &Path) -> (Rules, Option<String>) {
        let (list, problem) = match std::fs::read_to_string(path) {
            Ok(s) => match serde_json::from_str(&s) {
                Ok(l) => (l, None),
                Err(e) => (Vec::new(), Some(format!("{} is unreadable ({e}); starting with no rules", path.display()))),
            },
            Err(_) => (Vec::new(), None),
        };
        (Rules { path: path.to_path_buf(), list }, problem)
    }

    fn save(&self) -> Result<(), String> {
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&self.list).expect("rules serialize"))
            .and_then(|_| std::fs::rename(&tmp, &self.path))
            .map_err(|e| format!("saving {}: {e}", self.path.display()))
    }

    /// Adds or replaces the rule for `rule.app`.
    pub fn set(&mut self, rule: Rule) -> Result<(), String> {
        let app = rule.app.trim();
        if app.is_empty() {
            return Err("an app name is required".into());
        }
        if !rule.block && rule.up_kbps == 0 && rule.down_kbps == 0 {
            return Err("a rule blocks, or caps upload or download".into());
        }
        for k in [rule.up_kbps, rule.down_kbps] {
            if k != 0 && !(MIN_KBPS..=MAX_KBPS).contains(&k) {
                return Err(format!("a cap must be between {MIN_KBPS} kbit/s and {} Gbit/s", MAX_KBPS / 1_000_000));
            }
        }
        let rule = Rule { app: app.to_string(), ..rule };
        self.list.retain(|r| !r.app.eq_ignore_ascii_case(&rule.app));
        self.list.push(rule);
        self.save()
    }

    pub fn remove(&mut self, app: &str) -> Result<bool, String> {
        let before = self.list.len();
        self.list.retain(|r| !r.app.eq_ignore_ascii_case(app.trim()));
        if self.list.len() == before {
            return Ok(false);
        }
        self.save().map(|_| true)
    }
}

/// What the kernel should do right now, derived from the rules and whether a
/// game is running.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    /// Lower-case app names whose traffic is dropped.
    pub blocked: Vec<String>,
    /// Lower-case app name → shaping class (1-based), with its caps.
    pub limits: Vec<Limit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limit {
    pub app: String,
    pub class: u16,
    pub up_kbps: u32,
    pub down_kbps: u32,
}

/// The packet mark of a shaping class: "UG" in the top half.
pub fn mark(class: u16) -> u32 {
    0x5547_0000 | class as u32
}

impl Policy {
    pub fn from_rules(rules: &[Rule], gaming: bool) -> Policy {
        let mut p = Policy::default();
        for r in rules.iter().filter(|r| r.applies(gaming)) {
            let app = r.app.to_lowercase();
            if r.block {
                p.blocked.push(app);
            } else if r.is_limit() {
                let class = p.limits.len() as u16 + 1;
                p.limits.push(Limit { app, class, up_kbps: r.up_kbps, down_kbps: r.down_kbps });
            }
        }
        p
    }

    pub fn blocks(&self, name: &str) -> bool {
        self.blocked.iter().any(|b| b.eq_ignore_ascii_case(name))
    }

    pub fn class_of(&self, name: &str) -> Option<u16> {
        self.limits.iter().find(|l| l.app.eq_ignore_ascii_case(name)).map(|l| l.class)
    }

    pub fn is_empty(&self) -> bool {
        self.blocked.is_empty() && self.limits.is_empty()
    }

    /// Per app name, what the interfaces report as its current state.
    pub fn states(&self) -> HashMap<String, &'static str> {
        let mut m = HashMap::new();
        for b in &self.blocked {
            m.insert(b.clone(), "blocked");
        }
        for l in &self.limits {
            m.insert(l.app.clone(), "limited");
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(app: &str, block: bool, up: u32, when: When) -> Rule {
        Rule { app: app.into(), block, up_kbps: up, down_kbps: 0, when }
    }

    #[test]
    fn rules_are_saved_replaced_by_name_and_removed() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("net.json");
        let (mut r, problem) = Rules::load(&path);
        assert!(problem.is_none() && r.list.is_empty());
        r.set(rule("steam", false, 1000, When::Gaming)).unwrap();
        r.set(rule(" Steam ", true, 0, When::Always)).unwrap();
        let (again, _) = Rules::load(&path);
        assert_eq!(again.list, vec![rule("Steam", true, 0, When::Always)]);
        assert!(r.remove("STEAM").unwrap());
        assert!(!r.remove("steam").unwrap());
        assert!(Rules::load(&path).0.list.is_empty());
    }

    #[test]
    fn nonsense_rules_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let (mut r, _) = Rules::load(&d.path().join("net.json"));
        assert!(r.set(rule("", true, 0, When::Always)).is_err());
        assert!(r.set(rule("x", false, 0, When::Always)).is_err(), "neither block nor cap");
        assert!(r.set(rule("x", false, 10, When::Always)).is_err(), "too low to load anything");
        assert!(r.list.is_empty());
    }

    #[test]
    fn a_corrupt_file_is_reported_not_fatal() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("net.json");
        std::fs::write(&path, "{nope").unwrap();
        let (r, problem) = Rules::load(&path);
        assert!(r.list.is_empty());
        assert!(problem.unwrap().contains("unreadable"));
    }

    #[test]
    fn gaming_rules_apply_only_while_gaming() {
        let rules = vec![rule("qbittorrent", false, 500, When::Gaming), rule("Dropbox", true, 0, When::Always)];
        let idle = Policy::from_rules(&rules, false);
        assert_eq!(idle.blocked, vec!["dropbox"]);
        assert!(idle.limits.is_empty());
        let game = Policy::from_rules(&rules, true);
        assert_eq!(game.class_of("qBittorrent"), Some(1));
        assert!(game.blocks("DROPBOX"));
        assert_eq!(mark(1), 0x5547_0001);
    }
}
