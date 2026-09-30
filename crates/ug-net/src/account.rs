//! Per-app byte counts and rates, kept in step with the cgroups that exist.

use crate::cgroups::{self, Label};
use crate::nft::{self, Dir, Nft};
use crate::rules::{mark, Policy};
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use ug_hw::SysRoot;
use ug_proto::AppTraffic;

struct Entry {
    level: u8,
    pids: Vec<i32>,
    label: Label,
    exe: String,
    /// Bytes counted before the sets were last refilled.
    bank: [u64; 2],
    /// The set element's counter now.
    cur: [u64; 2],
    last: Option<([u64; 2], Instant)>,
    bps: [u64; 2],
}

impl Entry {
    fn total(&self, i: usize) -> u64 {
        self.bank[i] + self.cur[i]
    }
}

fn idx(d: Dir) -> usize {
    match d {
        Dir::Rx => 0,
        Dir::Tx => 1,
    }
}

pub struct Accountant<N: Nft> {
    nft: N,
    root: SysRoot,
    installed: bool,
    /// cgroup paths nft refused (too long, odd characters); not retried.
    rejected: HashSet<String>,
    apps: HashMap<String, Entry>,
    /// A counted group vanished: its element is stale until a refill.
    gone: bool,
    /// Paths seen by the last scan.
    live: HashSet<String>,
    policy: Policy,
    /// The block and mark members need rewriting.
    policy_dirty: bool,
}

impl<N: Nft> Accountant<N> {
    pub fn new(nft: N, root: SysRoot) -> Self {
        Accountant { nft, root, installed: false, rejected: HashSet::new(), apps: HashMap::new(), gone: false, live: HashSet::new(), policy: Policy::default(), policy_dirty: false }
    }

    /// Creates (or recreates) the table. Counting starts from zero.
    pub fn install(&mut self) -> Result<(), String> {
        self.nft.apply(&nft::table_script())?;
        self.installed = true;
        for e in self.apps.values_mut() {
            for i in 0..2 {
                e.bank[i] += e.cur[i];
                e.cur[i] = 0;
            }
        }
        // A recreated table is empty: count the known groups again.
        let known: Vec<(String, u8)> = self.apps.iter().map(|(p, e)| (p.clone(), e.level)).collect();
        let ok = self.add(&known);
        self.apps.retain(|p, _| ok.contains(p));
        self.policy_dirty = true;
        Ok(())
    }

    pub fn uninstall(&mut self) -> Result<(), String> {
        self.installed = false;
        self.nft.apply(&nft::remove_script())
    }

    pub fn installed(&self) -> bool {
        self.installed
    }

    /// Starts counting cgroups that appeared since the last call. Cheap: it
    /// runs `nft` only when there is something new. Returns whether any
    /// counted group has gone.
    pub fn sync_groups(&mut self) -> Result<bool, String> {
        if !self.installed {
            self.install()?;
        }
        let groups = cgroups::scan(&self.root);
        let live: HashSet<&str> = groups.iter().map(|g| g.path.as_str()).collect();
        let vanished = self.apps.keys().any(|p| !live.contains(p.as_str()));
        if vanished {
            self.gone = true;
        }
        let new: Vec<(String, u8)> = groups
            .iter()
            .filter(|g| !self.apps.contains_key(&g.path) && !self.rejected.contains(&g.path))
            .map(|g| (g.path.clone(), g.level))
            .collect();
        let added = self.add(&new);
        for g in &groups {
            if let Some(e) = self.apps.get_mut(&g.path) {
                if e.pids != g.pids {
                    e.pids = g.pids.clone();
                }
            } else if added.contains(&g.path) {
                let exe = g.pids.first().map(|&p| cgroups::comm(&self.root, p)).unwrap_or_default();
                let label = cgroups::label(&g.path, &exe);
                if self.policy.blocks(&label.name) || self.policy.class_of(&label.name).is_some() {
                    self.policy_dirty = true;
                }
                self.apps.insert(
                    g.path.clone(),
                    Entry { level: g.level, pids: g.pids.clone(), label, exe, bank: [0; 2], cur: [0; 2], last: None, bps: [0; 2] },
                );
            }
        }
        self.live = groups.into_iter().map(|g| g.path).collect();
        if self.policy_dirty {
            self.apply_policy()?;
        }
        Ok(vanished)
    }

    /// Picks up new and vanished cgroups and reads the counters.
    pub fn refresh(&mut self, now: Instant) -> Result<(), String> {
        self.sync_groups()?;
        // Their last bytes were read on the previous refresh.
        let live = std::mem::take(&mut self.live);
        self.apps.retain(|p, _| live.contains(p));

        let json = match self.nft.list() {
            Ok(j) => j,
            Err(e) => {
                // Someone removed our table (`nft flush ruleset`); put it back.
                tracing::info!(error = %e, "the nftables table is gone; recreating it");
                self.installed = false;
                return Err(e);
            }
        };
        let counts = nft::parse_counts(&json)?;
        let mut stale = std::mem::take(&mut self.gone);
        for c in &counts {
            if c.is_stale() {
                stale = true;
                continue;
            }
            if let Some(e) = self.apps.get_mut(&c.path) {
                e.cur[idx(c.dir)] = c.bytes;
            }
        }
        for e in self.apps.values_mut() {
            let total = [e.total(0), e.total(1)];
            if let Some((prev, at)) = e.last {
                let secs = now.duration_since(at).as_secs_f64();
                if secs > 0.2 {
                    for i in 0..2 {
                        e.bps[i] = (total[i].saturating_sub(prev[i]) as f64 / secs) as u64;
                    }
                    e.last = Some((total, now));
                }
            } else {
                e.last = Some((total, now));
            }
        }
        if stale {
            self.refill();
        }
        Ok(())
    }

    /// What to block and cap; applied on the next sync.
    pub fn set_policy(&mut self, p: Policy) {
        if p != self.policy {
            self.policy = p;
            self.policy_dirty = true;
        }
    }

    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    fn apply_policy(&mut self) -> Result<(), String> {
        let mut blocked = Vec::new();
        let mut marked = Vec::new();
        for (p, e) in &self.apps {
            if self.policy.blocks(&e.label.name) {
                blocked.push((p.clone(), e.level));
            } else if let Some(c) = self.policy.class_of(&e.label.name) {
                marked.push((p.clone(), e.level, mark(c)));
            }
        }
        self.nft.apply(&nft::policy_script(&blocked, &marked))?;
        self.policy_dirty = false;
        Ok(())
    }

    /// Adds groups to the sets; returns the ones that made it.
    fn add(&mut self, new: &[(String, u8)]) -> HashSet<String> {
        if new.is_empty() {
            return HashSet::new();
        }
        if self.nft.apply(&nft::add_script(new)).is_ok() {
            return new.iter().map(|(p, _)| p.clone()).collect();
        }
        // One bad path fails the whole transaction; find it. A group that
        // exited since the scan fails too, and is simply not counted.
        let mut ok = HashSet::new();
        for g in new {
            match self.nft.apply(&nft::add_script(std::slice::from_ref(g))) {
                Ok(()) => {
                    ok.insert(g.0.clone());
                }
                Err(e) => {
                    if self.root.join("sys/fs/cgroup").join(&g.0).exists() {
                        tracing::debug!(path = %g.0, error = %e, "cgroup not counted");
                        self.rejected.insert(g.0.clone());
                    }
                }
            }
        }
        ok
    }

    /// Drops elements of cgroups that are gone, banking what live ones had.
    fn refill(&mut self) {
        let live: Vec<(String, u8)> = self.apps.iter().map(|(p, e)| (p.clone(), e.level)).collect();
        match self.nft.apply(&nft::refill_script(&live)) {
            Ok(()) => {
                for e in self.apps.values_mut() {
                    for i in 0..2 {
                        e.bank[i] += e.cur[i];
                        e.cur[i] = 0;
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "could not tidy the counting sets"),
        }
    }

    /// Every counted app, busiest first.
    pub fn apps(&self) -> Vec<AppTraffic> {
        let mut out: Vec<AppTraffic> = self
            .apps
            .iter()
            .map(|(p, e)| AppTraffic {
                id: p.clone(),
                name: e.label.name.clone(),
                kind: e.label.kind.into(),
                uid: e.label.uid,
                exe: e.exe.clone(),
                pids: e.pids.clone(),
                rx_bps: e.bps[0],
                tx_bps: e.bps[1],
                rx_bytes: e.total(0),
                tx_bytes: e.total(1),
                state: if self.policy.blocks(&e.label.name) {
                    "blocked".into()
                } else if self.policy.class_of(&e.label.name).is_some() {
                    "limited".into()
                } else {
                    String::new()
                },
            })
            .collect();
        out.sort_by(|a, b| {
            (b.rx_bps + b.tx_bps)
                .cmp(&(a.rx_bps + a.tx_bps))
                .then((b.rx_bytes + b.tx_bytes).cmp(&(a.rx_bytes + a.tx_bytes)))
                .then(a.name.cmp(&b.name))
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use ug_testfs::TestFs;

    /// Keeps a fake kernel set: path → [rx, tx] per level, and records scripts.
    #[derive(Clone, Default)]
    struct FakeNft {
        elems: Arc<Mutex<HashMap<(String, u8), [u64; 2]>>>,
        /// Counters of elements whose cgroup is gone (printed as ids).
        stale: Arc<Mutex<Vec<(u8, u64)>>>,
        scripts: Arc<Mutex<Vec<String>>>,
        refuse: Arc<Mutex<HashSet<String>>>,
    }

    impl FakeNft {
        fn traffic(&self, path: &str, level: u8, rx: u64, tx: u64) {
            let mut e = self.elems.lock().unwrap();
            let c = e.get_mut(&(path.to_string(), level)).expect("counted");
            c[0] += rx;
            c[1] += tx;
        }
        fn vanish(&self, path: &str, level: u8) {
            let c = self.elems.lock().unwrap().remove(&(path.to_string(), level)).unwrap();
            self.stale.lock().unwrap().push((level, c[1]));
        }
    }

    impl Nft for FakeNft {
        fn apply(&mut self, script: &str) -> Result<(), String> {
            self.scripts.lock().unwrap().push(script.to_string());
            for bad in self.refuse.lock().unwrap().iter() {
                if script.contains(bad.as_str()) {
                    return Err("Error: cgroupv2 path fails".into());
                }
            }
            let mut e = self.elems.lock().unwrap();
            if script.contains("flush set inet unix_guardian rx") {
                e.values_mut().for_each(|c| *c = [0; 2]);
                self.stale.lock().unwrap().clear();
                e.clear();
            }
            for line in script.lines().filter(|l| l.starts_with("add element")) {
                let set = line.split_whitespace().nth(4).unwrap();
                if !(set.starts_with("rx") || set.starts_with("tx")) {
                    continue; // block and mark members are checked from the script text
                }
                let level: u8 = set[2..].parse().unwrap();
                let body = &line[line.find('{').unwrap() + 1..line.rfind('}').unwrap()];
                for p in body.split(", ") {
                    e.entry((p.trim().trim_matches('"').to_string(), level)).or_insert([0; 2]);
                }
            }
            Ok(())
        }

        fn list(&mut self) -> Result<String, String> {
            let mut sets: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
            for ((p, l), c) in self.elems.lock().unwrap().iter() {
                for (i, d) in ["rx", "tx"].iter().enumerate() {
                    sets.entry(format!("{d}{l}")).or_default().push(
                        serde_json::json!({"elem": {"val": p, "counter": {"packets": 1, "bytes": c[i]}}}),
                    );
                }
            }
            for (i, (l, b)) in self.stale.lock().unwrap().iter().enumerate() {
                sets.entry(format!("tx{l}")).or_default().push(
                    serde_json::json!({"elem": {"val": (9000 + i).to_string(), "counter": {"packets": 1, "bytes": b}}}),
                );
            }
            let items: Vec<serde_json::Value> =
                sets.into_iter().map(|(n, e)| serde_json::json!({"set": {"name": n, "elem": e}})).collect();
            Ok(serde_json::json!({ "nftables": items }).to_string())
        }
    }

    const FF: &str = "user.slice/user-1000.slice/user@1000.service/app.slice/app-firefox-10.scope";
    const PK: &str = "system.slice/packagekit.service";

    fn machine() -> TestFs {
        let fs = TestFs::new();
        fs.write(format!("sys/fs/cgroup/{FF}/cgroup.procs"), "10");
        fs.write(format!("sys/fs/cgroup/{PK}/cgroup.procs"), "20");
        fs.write("proc/10/comm", "firefox");
        fs.write("proc/20/comm", "packagekitd");
        fs
    }

    #[test]
    fn counts_bytes_and_rates_per_app() {
        let fs = machine();
        let nft = FakeNft::default();
        let mut a = Accountant::new(nft.clone(), SysRoot::at(fs.root()));
        let t0 = Instant::now();
        a.refresh(t0).unwrap();
        assert!(nft.scripts.lock().unwrap()[0].contains("table inet unix_guardian {"));
        assert_eq!(a.apps().len(), 2);

        nft.traffic(FF, 5, 4_000_000, 200_000);
        nft.traffic(PK, 2, 10_000, 1_000);
        a.refresh(t0 + Duration::from_secs(2)).unwrap();
        let apps = a.apps();
        assert_eq!(apps[0].name, "firefox");
        assert_eq!(apps[0].rx_bytes, 4_000_000);
        assert_eq!(apps[0].rx_bps, 2_000_000);
        assert_eq!(apps[0].tx_bps, 100_000);
        assert_eq!((apps[1].name.as_str(), apps[1].kind.as_str()), ("packagekit", "service"));
        assert_eq!(apps[0].exe, "firefox");
    }

    #[test]
    fn a_new_app_is_added_and_a_gone_one_dropped_keeping_totals() {
        let fs = machine();
        let nft = FakeNft::default();
        let mut a = Accountant::new(nft.clone(), SysRoot::at(fs.root()));
        let t0 = Instant::now();
        a.refresh(t0).unwrap();
        nft.traffic(FF, 5, 1000, 100);
        nft.traffic(PK, 2, 50, 5);
        a.refresh(t0 + Duration::from_secs(2)).unwrap();

        // packagekit exits; its element turns into a bare id in nft.
        std::fs::remove_dir_all(fs.root().join(format!("sys/fs/cgroup/{PK}"))).unwrap();
        nft.vanish(PK, 2);
        let steam = "user.slice/user-1000.slice/user@1000.service/app.slice/app-steam@x.service";
        fs.write(format!("sys/fs/cgroup/{steam}/cgroup.procs"), "30");
        a.refresh(t0 + Duration::from_secs(4)).unwrap();

        let apps = a.apps();
        assert_eq!(apps.len(), 2);
        assert!(apps.iter().any(|x| x.name == "steam"));
        // The stale element forced a refill; Firefox's bytes were banked.
        assert!(nft.scripts.lock().unwrap().last().unwrap().contains("flush set inet unix_guardian rx"));
        nft.traffic(FF, 5, 10, 1);
        a.refresh(t0 + Duration::from_secs(6)).unwrap();
        let ff = a.apps().into_iter().find(|x| x.name == "firefox").unwrap();
        assert_eq!((ff.rx_bytes, ff.tx_bytes), (1010, 101));
    }

    #[test]
    fn a_removed_table_comes_back_with_totals_kept() {
        let fs = machine();
        let nft = FakeNft::default();
        let mut a = Accountant::new(nft.clone(), SysRoot::at(fs.root()));
        let t0 = Instant::now();
        a.refresh(t0).unwrap();
        nft.traffic(FF, 5, 500, 50);
        a.refresh(t0 + Duration::from_secs(2)).unwrap();
        // `nft flush ruleset` elsewhere: the next list fails, the one after
        // recreates the table and re-adds Firefox.
        nft.elems.lock().unwrap().clear();
        a.installed = false;
        a.refresh(t0 + Duration::from_secs(4)).unwrap();
        nft.traffic(FF, 5, 5, 0);
        a.refresh(t0 + Duration::from_secs(6)).unwrap();
        let ff = a.apps().into_iter().find(|x| x.name == "firefox").unwrap();
        assert_eq!(ff.rx_bytes, 505);
    }

    #[test]
    fn a_policy_blocks_and_marks_by_app_name_including_apps_started_later() {
        use crate::rules::{Rule, When};
        let fs = machine();
        let nft = FakeNft::default();
        let mut a = Accountant::new(nft.clone(), SysRoot::at(fs.root()));
        a.refresh(Instant::now()).unwrap();
        let rules = vec![
            Rule { app: "packagekit".into(), block: true, up_kbps: 0, down_kbps: 0, when: When::Always },
            Rule { app: "steam".into(), block: false, up_kbps: 500, down_kbps: 0, when: When::Always },
        ];
        a.set_policy(Policy::from_rules(&rules, false));
        a.sync_groups().unwrap();
        let last = nft.scripts.lock().unwrap().last().unwrap().clone();
        assert!(last.contains(&format!("add element inet unix_guardian block2 {{ \"{PK}\" }}")), "{last}");
        assert!(!last.contains("mark5 {"), "steam is not running yet");
        let pk = a.apps().into_iter().find(|x| x.name == "packagekit").unwrap();
        assert_eq!(pk.state, "blocked");

        // Steam starts: its new group is marked without another rule change.
        let steam = "user.slice/user-1000.slice/user@1000.service/app.slice/app-steam@x.service";
        fs.write(format!("sys/fs/cgroup/{steam}/cgroup.procs"), "30");
        a.sync_groups().unwrap();
        let last = nft.scripts.lock().unwrap().last().unwrap().clone();
        assert!(last.contains(&format!("add element inet unix_guardian mark5 {{ \"{steam}\" : 0x55470001 }}")), "{last}");
        assert_eq!(a.apps().into_iter().find(|x| x.name == "steam").unwrap().state, "limited");

        // Nothing changes: no new nft call.
        let n = nft.scripts.lock().unwrap().len();
        a.sync_groups().unwrap();
        assert_eq!(nft.scripts.lock().unwrap().len(), n);
    }

    #[test]
    fn a_path_nft_refuses_does_not_stop_the_others() {
        let fs = machine();
        let nft = FakeNft::default();
        nft.refuse.lock().unwrap().insert(PK.into());
        let mut a = Accountant::new(nft.clone(), SysRoot::at(fs.root()));
        a.refresh(Instant::now()).unwrap();
        let apps = a.apps();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "firefox");
        let n = nft.scripts.lock().unwrap().len();
        a.refresh(Instant::now()).unwrap();
        // Not retried on every refresh.
        assert_eq!(nft.scripts.lock().unwrap().len(), n);
    }
}
