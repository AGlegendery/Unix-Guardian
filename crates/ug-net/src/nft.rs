//! Unix Guardian's own nftables table, `inet unix_guardian`.
//!
//! Everything we do lives in this one table: other tables (the user's rules,
//! firewalld, ufw, Docker) are never read or changed. Removing the table
//! removes every trace.
//!
//! Counting uses one set per cgroup level with a counter on each element:
//! `socket cgroupv2 level 5 @tx5` bumps the element of the level-5 ancestor
//! of the sending socket's cgroup. The deepest level matches first and
//! `accept`s (which ends only this table's chain, never another table's), so
//! a nested group is not counted twice.
//!
//! Blocking and shaping use two more structures per level, both empty until
//! the user makes a rule: `blockL`, whose members' outgoing packets are
//! dropped, and `markL`, which marks a member's packets (and its connection,
//! so replies can be classified on the way in) for `tc` to cap.
//!
//! Driven through the `nft` program: `nft -f -` applies a script as one
//! atomic transaction, and `nft -j` reports counters as JSON.

use crate::cgroups::{MAX_LEVEL, MIN_LEVEL};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub const TABLE: &str = "unix_guardian";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir {
    Rx,
    Tx,
}

impl Dir {
    fn set(self, level: u8) -> String {
        match self {
            Dir::Rx => format!("rx{level}"),
            Dir::Tx => format!("tx{level}"),
        }
    }
}

/// The whole table, replacing any earlier version in the same transaction.
pub fn table_script() -> String {
    let mut s = String::new();
    // `add` then `delete` succeeds whether or not the table exists.
    s.push_str(&format!(
        "add table inet {TABLE}\ndelete table inet {TABLE}\ntable inet {TABLE} {{\n"
    ));
    for l in MIN_LEVEL..=MAX_LEVEL {
        for d in [Dir::Rx, Dir::Tx] {
            s.push_str(&format!(
                "  set {} {{ type cgroupsv2; flags dynamic; counter; }}\n",
                d.set(l)
            ));
        }
        s.push_str(&format!(
            "  set block{l} {{ type cgroupsv2; flags dynamic; }}\n"
        ));
        s.push_str(&format!(
            "  set vpn{l} {{ type cgroupsv2; flags dynamic; }}\n"
        ));
        s.push_str(&format!(
            "  map mark{l} {{ type cgroupsv2 : mark; flags dynamic; }}\n"
        ));
    }
    for (chain, hook, dir) in [("egress", "output", Dir::Tx), ("ingress", "input", Dir::Rx)] {
        s.push_str(&format!(
            "  chain {chain} {{\n    type filter hook {hook} priority -150; policy accept;\n"
        ));
        if dir == Dir::Tx {
            for l in (MIN_LEVEL..=MAX_LEVEL).rev() {
                s.push_str(&format!("    socket cgroupv2 level {l} @block{l} drop\n"));
            }
            for l in (MIN_LEVEL..=MAX_LEVEL).rev() {
                s.push_str(&format!(
                    "    meta mark 0 meta mark set socket cgroupv2 level {l} map @mark{l}\n"
                ));
            }
            // Only our marks reach the connection: a VPN's own marks are left alone.
            s.push_str("    meta mark & 0xffff0000 == 0x55470000 ct mark set meta mark\n");
        }
        // A VPN or proxy's traffic with the apps it carries (over loopback,
        // in proxy mode) is theirs, already counted: only its own traffic
        // with the outside world counts for it.
        let lo = if dir == Dir::Tx { "oifname" } else { "iifname" };
        for l in (MIN_LEVEL..=MAX_LEVEL).rev() {
            s.push_str(&format!(
                "    {lo} \"lo\" socket cgroupv2 level {l} @vpn{l} accept\n"
            ));
        }
        for l in (MIN_LEVEL..=MAX_LEVEL).rev() {
            s.push_str(&format!(
                "    socket cgroupv2 level {l} @{} accept\n",
                dir.set(l)
            ));
        }
        s.push_str("  }\n");
    }
    s.push_str("}\n");
    s
}

pub fn remove_script() -> String {
    format!("add table inet {TABLE}\ndelete table inet {TABLE}\n")
}

fn quote(path: &str) -> String {
    format!("\"{path}\"")
}

/// Starts counting these cgroups (path, level).
pub fn add_script(groups: &[(String, u8)]) -> String {
    let mut s = String::new();
    for l in MIN_LEVEL..=MAX_LEVEL {
        let paths: Vec<String> = groups
            .iter()
            .filter(|(_, gl)| *gl == l)
            .map(|(p, _)| quote(p))
            .collect();
        if paths.is_empty() {
            continue;
        }
        for d in [Dir::Rx, Dir::Tx] {
            s.push_str(&format!(
                "add element inet {TABLE} {} {{ {} }}\n",
                d.set(l),
                paths.join(", ")
            ));
        }
    }
    s
}

/// Empties the counting sets and refills them with live cgroups. `nft`
/// cannot name an element whose cgroup is gone, so dropping those means
/// rebuilding the set; the caller banks the counters read just before.
pub fn refill_script(groups: &[(String, u8)]) -> String {
    let mut s = String::new();
    for l in MIN_LEVEL..=MAX_LEVEL {
        for d in [Dir::Rx, Dir::Tx] {
            s.push_str(&format!("flush set inet {TABLE} {}\n", d.set(l)));
        }
    }
    s + &add_script(groups)
}

/// Replaces the block and mark members: `blocked` (path, level) and
/// `marked` (path, level, mark).
pub fn policy_script(
    blocked: &[(String, u8)],
    marked: &[(String, u8, u32)],
    vpn: &[(String, u8)],
) -> String {
    let mut s = String::new();
    for l in MIN_LEVEL..=MAX_LEVEL {
        s.push_str(&format!("flush set inet {TABLE} block{l}\nflush map inet {TABLE} mark{l}\nflush set inet {TABLE} vpn{l}\n"));
        let v: Vec<String> = vpn
            .iter()
            .filter(|(_, gl)| *gl == l)
            .map(|(p, _)| quote(p))
            .collect();
        if !v.is_empty() {
            s.push_str(&format!(
                "add element inet {TABLE} vpn{l} {{ {} }}\n",
                v.join(", ")
            ));
        }
        let b: Vec<String> = blocked
            .iter()
            .filter(|(_, gl)| *gl == l)
            .map(|(p, _)| quote(p))
            .collect();
        if !b.is_empty() {
            s.push_str(&format!(
                "add element inet {TABLE} block{l} {{ {} }}\n",
                b.join(", ")
            ));
        }
        let m: Vec<String> = marked
            .iter()
            .filter(|(_, gl, _)| *gl == l)
            .map(|(p, _, m)| format!("{} : {m:#x}", quote(p)))
            .collect();
        if !m.is_empty() {
            s.push_str(&format!(
                "add element inet {TABLE} mark{l} {{ {} }}\n",
                m.join(", ")
            ));
        }
    }
    s
}

/// One element's counter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Count {
    pub dir: Dir,
    pub level: u8,
    /// The cgroup path, or a bare number once the cgroup is gone.
    pub path: String,
    pub bytes: u64,
}

impl Count {
    /// `nft` prints the cgroup id instead of a path when the cgroup no
    /// longer exists.
    pub fn is_stale(&self) -> bool {
        self.path.chars().all(|c| c.is_ascii_digit())
    }
}

/// Reads the counters from `nft -j list table inet unix_guardian`.
pub fn parse_counts(json: &str) -> Result<Vec<Count>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("nft printed unreadable JSON: {e}"))?;
    let mut out = Vec::new();
    for item in v["nftables"].as_array().into_iter().flatten() {
        let Some(set) = item.get("set") else { continue };
        let Some(name) = set["name"].as_str() else {
            continue;
        };
        let (dir, level) = match (name.strip_prefix("rx"), name.strip_prefix("tx")) {
            (Some(l), _) => (Dir::Rx, l),
            (_, Some(l)) => (Dir::Tx, l),
            _ => continue,
        };
        let Ok(level) = level.parse::<u8>() else {
            continue;
        };
        for e in set["elem"].as_array().into_iter().flatten() {
            let e = &e["elem"];
            let path = match &e["val"] {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                _ => continue,
            };
            let bytes = e["counter"]["bytes"].as_u64().unwrap_or(0);
            out.push(Count {
                dir,
                level,
                path,
                bytes,
            });
        }
    }
    Ok(out)
}

/// Talks to nftables. A trait so the accounting logic is tested without it.
pub trait Nft: Send {
    fn apply(&mut self, script: &str) -> Result<(), String>;
    fn list(&mut self) -> Result<String, String>;
}

/// The real `nft` program.
pub struct NftCmd {
    bin: PathBuf,
}

impl NftCmd {
    /// `None` when nftables' command-line tool is not installed.
    pub fn find() -> Option<NftCmd> {
        ["/usr/sbin/nft", "/sbin/nft", "/usr/bin/nft", "/bin/nft"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
            .map(|bin| NftCmd { bin })
    }
}

impl Nft for NftCmd {
    fn apply(&mut self, script: &str) -> Result<(), String> {
        let mut child = Command::new(&self.bin)
            .args(["-f", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run nft: {e}"))?;
        child
            .stdin
            .take()
            .expect("piped")
            .write_all(script.as_bytes())
            .map_err(|e| e.to_string())?;
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr)
                .lines()
                .next()
                .unwrap_or("nft failed")
                .to_string())
        }
    }

    fn list(&mut self) -> Result<String, String> {
        let out = Command::new(&self.bin)
            .args(["-j", "list", "table", "inet", TABLE])
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("cannot run nft: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&out.stderr)
                .lines()
                .next()
                .unwrap_or("nft failed")
                .to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_counts_deepest_level_first_in_both_directions() {
        let s = table_script();
        assert!(s.starts_with("add table inet unix_guardian\ndelete table inet unix_guardian\n"));
        let egress = &s[s.find("chain egress").unwrap()..s.find("chain ingress").unwrap()];
        assert!(egress.contains("hook output"));
        let l8 = egress.find("level 8 @tx8").unwrap();
        let l2 = egress.find("level 2 @tx2").unwrap();
        assert!(l8 < l2);
        assert!(s[s.find("chain ingress").unwrap()..].contains("level 5 @rx5 accept"));
        // The only drop is for members of the (empty) block sets, before
        // marking and counting; nothing touches another table.
        assert_eq!(
            s.matches(" drop").count(),
            (MAX_LEVEL - MIN_LEVEL + 1) as usize
        );
        assert!(egress.find("@block8 drop").unwrap() < egress.find("map @mark8").unwrap());
        assert!(egress.find("ct mark set").unwrap() < l8);
        assert!(!s[s.find("chain ingress").unwrap()..].contains("drop"));
        // A VPN's loopback traffic is skipped before counting, both ways.
        assert!(
            egress
                .find("oifname \"lo\" socket cgroupv2 level 8 @vpn8 accept")
                .unwrap()
                < l8
        );
        assert!(s[s.find("chain ingress").unwrap()..]
            .contains("iifname \"lo\" socket cgroupv2 level 5 @vpn5 accept"));
        assert_eq!(s.matches("table inet").count(), 3);
    }

    #[test]
    fn add_groups_elements_by_level() {
        let s = add_script(&[
            ("a/b".into(), 2),
            ("x/y/z/w/app.scope".into(), 5),
            ("c/d".into(), 2),
        ]);
        assert!(s.contains("add element inet unix_guardian rx2 { \"a/b\", \"c/d\" }\n"));
        assert!(s.contains("add element inet unix_guardian tx5 { \"x/y/z/w/app.scope\" }\n"));
        assert_eq!(s.lines().count(), 4);
        assert_eq!(add_script(&[]), "");
    }

    #[test]
    fn refill_flushes_every_set_first() {
        let s = refill_script(&[("a/b".into(), 2)]);
        assert_eq!(
            s.matches("flush set").count(),
            2 * (MAX_LEVEL - MIN_LEVEL + 1) as usize
        );
        assert!(s.rfind("flush").unwrap() < s.find("add element").unwrap());
    }

    #[test]
    fn policy_replaces_members_by_level() {
        let s = policy_script(
            &[("system.slice/x.service".into(), 2)],
            &[("a/b/c/d/steam.scope".into(), 5, 0x5547_0001)],
            &[("a/b/c/d/v2rayn.service".into(), 5)],
        );
        assert!(s.contains("add element inet unix_guardian vpn5 { \"a/b/c/d/v2rayn.service\" }"));
        assert!(
            s.contains("flush set inet unix_guardian block5\nflush map inet unix_guardian mark5\n")
        );
        assert!(s.contains("add element inet unix_guardian block2 { \"system.slice/x.service\" }"));
        assert!(s.contains(
            "add element inet unix_guardian mark5 { \"a/b/c/d/steam.scope\" : 0x55470001 }"
        ));
        assert!(!policy_script(&[], &[], &[]).contains("add element"));
    }

    #[test]
    fn parses_counters_and_stale_ids() {
        // As nft 1.1.3 prints it; the second element's cgroup was removed.
        let json = r#"{"nftables": [{"metainfo": {"version": "1.1.3", "json_schema_version": 1}},
            {"table": {"family": "inet", "name": "unix_guardian", "handle": 1}},
            {"set": {"family": "inet", "name": "tx5", "table": "unix_guardian", "type": "cgroupsv2", "handle": 2,
              "flags": ["dynamic"], "elem": [
                {"elem": {"val": "user.slice/user-1000.slice/user@1000.service/app.slice/app-firefox\\x2desr@7f.service",
                          "counter": {"packets": 11, "bytes": 100580}}},
                {"elem": {"val": "40568", "counter": {"packets": 0, "bytes": 7}}}]}},
            {"set": {"family": "inet", "name": "rx2", "table": "unix_guardian", "type": "cgroupsv2", "handle": 3}},
            {"chain": {"family": "inet", "table": "unix_guardian", "name": "egress", "handle": 4}}]}"#;
        let c = parse_counts(json).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].dir, Dir::Tx);
        assert_eq!(c[0].level, 5);
        assert!(c[0].path.ends_with("app-firefox\\x2desr@7f.service"));
        assert_eq!(c[0].bytes, 100580);
        assert!(!c[0].is_stale());
        assert!(c[1].is_stale());
        assert!(parse_counts("not json").is_err());
    }
}
