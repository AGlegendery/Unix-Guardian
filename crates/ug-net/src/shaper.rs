//! Caps on an app's upload and download, with `tc`.
//!
//! Upload: an HTB tree on the interface that carries the default route. Its
//! default class is uncapped; each limited app gets a class whose rate and
//! ceiling are the cap, chosen by the packet mark nftables set (`fw` filter).
//!
//! Download: packets arriving on the interface are redirected to our IFB
//! device `ug-ifb0` after `connmark` copied the connection's mark back onto
//! them (the mark was set when the app sent on that connection), and an
//! identical HTB tree there caps them.
//!
//! Nothing is installed while no limit applies. Another tool's shaping on the
//! interface (a VPN's, wondershaper's) is never replaced: the cap is refused
//! with a sentence saying why.

use crate::rules::{mark, Limit};

pub const IFB: &str = "ug-ifb0";
const HANDLE: &str = "5547";

/// Root qdiscs the kernel or distribution puts there by default; replacing
/// one takes nothing away from anybody.
const DEFAULT_QDISCS: &[&str] = &[
    "mq",
    "fq_codel",
    "pfifo_fast",
    "noqueue",
    "fq",
    "pfifo",
    "sfq",
];

/// Runs `tc` or `ip`. A trait so the plan is tested without a kernel.
pub trait Cmd: Send {
    fn run(&mut self, prog: &str, args: &[String]) -> Result<String, String>;
}

pub struct RealCmd;

impl Cmd for RealCmd {
    fn run(&mut self, prog: &str, args: &[String]) -> Result<String, String> {
        let bin = ["/usr/sbin", "/sbin", "/usr/bin", "/bin"]
            .iter()
            .map(|d| std::path::Path::new(d).join(prog))
            .find(|p| p.exists())
            .ok_or_else(|| format!("{prog} is not installed (iproute2)"))?;
        let out = std::process::Command::new(bin)
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&out.stderr)
                .lines()
                .next()
                .unwrap_or("failed")
                .trim()
                .to_string())
        }
    }
}

fn a(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// What is installed now, so it can be undone exactly.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Installed {
    pub iface: String,
    pub up: bool,
    pub down: bool,
    pub limits: Vec<Limit>,
}

pub struct Shaper<C: Cmd> {
    cmd: C,
    pub now: Installed,
}

/// The HTB tree on `dev`: an uncapped default class and one class per cap.
fn tree(dev: &str, caps: &[(u16, u32)]) -> Vec<Vec<String>> {
    let mut out = vec![
        a(&[
            "qdisc",
            "replace",
            "dev",
            dev,
            "root",
            "handle",
            &format!("{HANDLE}:"),
            "htb",
            "default",
            "1",
        ]),
        a(&[
            "class",
            "add",
            "dev",
            dev,
            "parent",
            &format!("{HANDLE}:"),
            "classid",
            &format!("{HANDLE}:1"),
            "htb",
            "rate",
            "10gbit",
        ]),
        a(&[
            "qdisc",
            "add",
            "dev",
            dev,
            "parent",
            &format!("{HANDLE}:1"),
            "fq_codel",
        ]),
    ];
    for &(class, kbps) in caps {
        let id = format!("{HANDLE}:{:x}", 0x10 + class);
        let rate = format!("{kbps}kbit");
        out.push(a(&[
            "class",
            "add",
            "dev",
            dev,
            "parent",
            &format!("{HANDLE}:"),
            "classid",
            &id,
            "htb",
            "rate",
            &rate,
            "ceil",
            &rate,
        ]));
        out.push(a(&["qdisc", "add", "dev", dev, "parent", &id, "fq_codel"]));
        out.push(a(&[
            "filter",
            "add",
            "dev",
            dev,
            "parent",
            &format!("{HANDLE}:"),
            "protocol",
            "all",
            "prio",
            "1",
            "handle",
            &format!("{:#x}/0xffffffff", mark(class)),
            "fw",
            "flowid",
            &id,
        ]));
    }
    out
}

impl<C: Cmd> Shaper<C> {
    pub fn new(cmd: C) -> Self {
        Shaper {
            cmd,
            now: Installed::default(),
        }
    }

    fn tc(&mut self, args: Vec<String>) -> Result<String, String> {
        self.cmd.run("tc", &args)
    }

    /// The root qdisc kind and handle on `dev`, e.g. ("fq_codel", "0:").
    fn root(&mut self, dev: &str) -> Option<(String, String)> {
        let out = self.tc(a(&["qdisc", "show", "dev", dev, "root"])).ok()?;
        let f: Vec<&str> = out.split_whitespace().collect();
        Some((f.get(1)?.to_string(), f.get(2)?.to_string()))
    }

    fn has_ingress(&mut self, dev: &str) -> bool {
        self.tc(a(&["qdisc", "show", "dev", dev, "ingress"]))
            .is_ok_and(|o| o.contains("ingress"))
    }

    /// Makes the kernel match `limits` on `iface`. Idempotent: an unchanged
    /// plan does nothing. Returns why a direction could not be capped.
    pub fn apply(&mut self, iface: &str, limits: &[Limit]) -> Vec<String> {
        let want = Installed {
            iface: iface.to_string(),
            up: limits.iter().any(|l| l.up_kbps > 0) && !iface.is_empty(),
            down: limits.iter().any(|l| l.down_kbps > 0) && !iface.is_empty(),
            limits: limits.to_vec(),
        };
        if want == self.now {
            return Vec::new();
        }
        self.clear();
        let mut problems = Vec::new();
        if want.up {
            match self.root(iface) {
                Some((kind, handle))
                    if !DEFAULT_QDISCS.contains(&kind.as_str())
                        && handle != format!("{HANDLE}:") =>
                {
                    problems.push(format!(
                        "Upload caps are off: {iface} is already shaped by another tool ({kind}), which Unix Guardian leaves alone."
                    ));
                }
                _ => {
                    let caps: Vec<(u16, u32)> = limits
                        .iter()
                        .filter(|l| l.up_kbps > 0)
                        .map(|l| (l.class, l.up_kbps))
                        .collect();
                    match self.batch(tree(iface, &caps)) {
                        Ok(()) => self.now.up = true,
                        Err(e) => {
                            problems.push(format!("Upload caps could not be set on {iface}: {e}"));
                            let _ = self.tc(a(&["qdisc", "del", "dev", iface, "root"]));
                        }
                    }
                }
            }
        }
        if want.down {
            if self.has_ingress(iface) {
                problems.push(format!(
                    "Download caps are off: incoming traffic on {iface} is already handled by another tool, which Unix Guardian leaves alone."
                ));
            } else {
                let caps: Vec<(u16, u32)> = limits
                    .iter()
                    .filter(|l| l.down_kbps > 0)
                    .map(|l| (l.class, l.down_kbps))
                    .collect();
                let mut steps = vec![
                    a(&["qdisc", "add", "dev", iface, "handle", "ffff:", "ingress"]),
                    a(&[
                        "filter", "add", "dev", iface, "parent", "ffff:", "protocol", "all",
                        "prio", "1", "matchall", "action", "connmark", "action", "mirred",
                        "egress", "redirect", "dev", IFB,
                    ]),
                ];
                steps.splice(0..0, tree(IFB, &caps));
                let made = self
                    .cmd
                    .run("ip", &a(&["link", "add", IFB, "type", "ifb"]))
                    .or_else(|e| {
                        if e.contains("exists") {
                            Ok(String::new())
                        } else {
                            Err(e)
                        }
                    })
                    .and_then(|_| self.cmd.run("ip", &a(&["link", "set", IFB, "up"])));
                match made.and_then(|_| self.batch(steps)) {
                    Ok(()) => self.now.down = true,
                    Err(e) => {
                        problems.push(format!("Download caps could not be set on {iface}: {e}"));
                        // Remove the parts that were made; upload caps stay.
                        self.clear_down(iface);
                    }
                }
            }
        }
        self.now.iface = iface.to_string();
        self.now.limits = limits.to_vec();
        problems
    }

    fn batch(&mut self, steps: Vec<Vec<String>>) -> Result<(), String> {
        for s in steps {
            self.tc(s)?;
        }
        Ok(())
    }

    /// Removes everything we installed, and only that.
    pub fn clear(&mut self) {
        let iface = self.now.iface.clone();
        if self.now.up
            && !iface.is_empty()
            && self
                .root(&iface)
                .is_some_and(|(_, h)| h == format!("{HANDLE}:"))
        {
            let _ = self.tc(a(&["qdisc", "del", "dev", &iface, "root"]));
        }
        if self.now.down {
            self.clear_down(&iface);
        }
        self.now = Installed::default();
    }

    fn clear_down(&mut self, iface: &str) {
        if !iface.is_empty() {
            let ours = self
                .tc(a(&["filter", "show", "dev", iface, "parent", "ffff:"]))
                .is_ok_and(|o| o.contains(IFB));
            if ours {
                let _ = self.tc(a(&[
                    "qdisc", "del", "dev", iface, "handle", "ffff:", "ingress",
                ]));
            }
        }
        let _ = self.cmd.run("ip", &a(&["link", "del", IFB]));
    }

    /// After a crash: our IFB device and our root handle on `iface` are
    /// ours to remove, whatever the daemon remembers.
    pub fn clean_leftovers(&mut self, iface: &str) {
        self.now = Installed {
            iface: iface.to_string(),
            up: true,
            down: true,
            limits: Vec::new(),
        };
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Answers `qdisc show` from a fake state and records every command.
    #[derive(Clone, Default)]
    struct Fake {
        log: Arc<Mutex<Vec<String>>>,
        root: Arc<Mutex<String>>,
        ingress: Arc<Mutex<bool>>,
    }

    impl Cmd for Fake {
        fn run(&mut self, prog: &str, args: &[String]) -> Result<String, String> {
            let line = format!("{prog} {}", args.join(" "));
            self.log.lock().unwrap().push(line.clone());
            let mut root = self.root.lock().unwrap();
            if line.contains("qdisc show") && line.ends_with("root") {
                return Ok(root.clone());
            }
            if line.contains("qdisc show") && line.ends_with("ingress") {
                return Ok(if *self.ingress.lock().unwrap() {
                    "qdisc ingress ffff: parent ffff:fff1".into()
                } else {
                    String::new()
                });
            }
            if line.contains("filter show") {
                return Ok(format!(
                    "filter protocol all pref 1 matchall action mirred egress redirect dev {IFB}"
                ));
            }
            if line.starts_with("tc qdisc replace dev wlan0 root handle 5547:") {
                *root = "qdisc htb 5547: root refcnt 2".into();
            }
            if line == "tc qdisc del dev wlan0 root" {
                *root = "qdisc fq_codel 0: root refcnt 2".into();
            }
            if line.contains("handle ffff: ingress") && line.contains(" add ") {
                *self.ingress.lock().unwrap() = true;
            }
            Ok(String::new())
        }
    }

    fn limit(class: u16, up: u32, down: u32) -> Limit {
        Limit {
            app: format!("app{class}"),
            class,
            up_kbps: up,
            down_kbps: down,
        }
    }

    fn fake() -> Fake {
        let f = Fake::default();
        *f.root.lock().unwrap() = "qdisc fq_codel 0: root refcnt 2 limit 10240p".into();
        f
    }

    #[test]
    fn an_upload_cap_builds_one_class_with_its_mark() {
        let f = fake();
        let mut s = Shaper::new(f.clone());
        assert!(s.apply("wlan0", &[limit(1, 4000, 0)]).is_empty());
        let log = f.log.lock().unwrap().join("\n");
        assert!(log.contains(
            "tc class add dev wlan0 parent 5547: classid 5547:11 htb rate 4000kbit ceil 4000kbit"
        ));
        assert!(log.contains("handle 0x55470001/0xffffffff fw flowid 5547:11"));
        assert!(!log.contains("ifb"), "no download cap, no IFB device");
        assert!(s.now.up && !s.now.down);
    }

    #[test]
    fn the_same_plan_twice_changes_nothing() {
        let f = fake();
        let mut s = Shaper::new(f.clone());
        s.apply("wlan0", &[limit(1, 4000, 0)]);
        let n = f.log.lock().unwrap().len();
        s.apply("wlan0", &[limit(1, 4000, 0)]);
        assert_eq!(f.log.lock().unwrap().len(), n);
    }

    #[test]
    fn a_download_cap_redirects_through_the_ifb_with_connmark() {
        let f = fake();
        let mut s = Shaper::new(f.clone());
        assert!(s.apply("wlan0", &[limit(2, 0, 8000)]).is_empty());
        let log = f.log.lock().unwrap().join("\n");
        assert!(log.contains("ip link add ug-ifb0 type ifb"));
        assert!(
            log.contains("tc class add dev ug-ifb0 parent 5547: classid 5547:12 htb rate 8000kbit")
        );
        assert!(log.contains("matchall action connmark action mirred egress redirect dev ug-ifb0"));
        assert!(
            !log.contains("dev wlan0 root"),
            "no upload cap, the root qdisc is untouched"
        );
    }

    #[test]
    fn removing_every_limit_restores_the_interface() {
        let f = fake();
        let mut s = Shaper::new(f.clone());
        s.apply("wlan0", &[limit(1, 4000, 8000)]);
        s.apply("wlan0", &[]);
        let log = f.log.lock().unwrap().join("\n");
        assert!(log.contains("tc qdisc del dev wlan0 root"));
        assert!(log.contains("tc qdisc del dev wlan0 handle ffff: ingress"));
        assert!(log.contains("ip link del ug-ifb0"));
        assert!(f.root.lock().unwrap().contains("fq_codel"));
        assert!(!s.now.up && !s.now.down);
    }

    #[test]
    fn another_tools_shaping_is_left_alone_and_said() {
        let f = fake();
        *f.root.lock().unwrap() = "qdisc cake 8001: root refcnt 2 bandwidth 20Mbit".into();
        *f.ingress.lock().unwrap() = true;
        let mut s = Shaper::new(f.clone());
        let problems = s.apply("wlan0", &[limit(1, 4000, 8000)]);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].contains("cake"));
        let log = f.log.lock().unwrap().join("\n");
        assert!(!log.contains("replace"), "{log}");
        assert!(!log.contains(" del "), "{log}");
    }
}
