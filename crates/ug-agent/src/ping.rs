//! Ping to the router, the DNS server and the game's server.
//!
//! Uses unprivileged ICMP sockets (`SOCK_DGRAM` + `IPPROTO_ICMP`, allowed by
//! `net.ipv4.ping_group_range`), so the agent needs no capabilities. One echo
//! per target per second, on a thread that runs only while someone is looking
//! (a window, the TUI or `ug ping` asked within the last minute) or a game is
//! running. Nothing is stored on disk: the last 15 minutes live in memory.

use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ug_hw::{procnet, SysRoot};
use ug_proto::{PingComparison, PingMark, PingReport, PingSample, PingStats, PingTarget};

const KEEP_MS: u64 = 15 * 60_000;
const INTERVAL: Duration = Duration::from_secs(1);
const TIMEOUT: Duration = Duration::from_millis(900);
const LINGER: Duration = Duration::from_secs(60);

/// Median, jitter, 95th percentile and loss of a run of samples.
pub fn stats(samples: &[PingSample]) -> PingStats {
    let sent = samples.len() as u32;
    let rtts: Vec<f64> = samples
        .iter()
        .map(|s| s.rtt_ms)
        .filter(|r| !r.is_nan())
        .collect();
    let received = rtts.len() as u32;
    if rtts.is_empty() {
        let loss_pct = if sent == 0 { 0.0 } else { 100.0 };
        return PingStats {
            sent,
            received,
            median_ms: f64::NAN,
            jitter_ms: f64::NAN,
            p95_ms: f64::NAN,
            loss_pct,
        };
    }
    let mut sorted = rtts.clone();
    sorted.sort_by(f64::total_cmp);
    let pick = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    let jitter_ms = if rtts.len() < 2 {
        0.0
    } else {
        rtts.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>() / (rtts.len() - 1) as f64
    };
    PingStats {
        sent,
        received,
        median_ms: pick(0.5),
        jitter_ms,
        p95_ms: pick(0.95),
        loss_pct: 100.0 * (sent - received) as f64 / sent as f64,
    }
}

/// Before and after `mark`, judged on jitter, the 95th percentile and loss,
/// so a label never claims an improvement the numbers do not show.
pub fn compare(samples: &[PingSample], mark: u64, window_ms: u64) -> PingComparison {
    let before: Vec<PingSample> = samples
        .iter()
        .filter(|s| s.ts_unix_ms < mark && s.ts_unix_ms + window_ms >= mark)
        .copied()
        .collect();
    let after: Vec<PingSample> = samples
        .iter()
        .filter(|s| s.ts_unix_ms >= mark && s.ts_unix_ms < mark + window_ms)
        .copied()
        .collect();
    let (b, a) = (stats(&before), stats(&after));
    let verdict = if b.received < 10 || a.received < 10 {
        "not-enough-data"
    } else {
        // A few milliseconds of difference is noise on any real link.
        let better = |x: f64, y: f64| y < x * 0.7 && x - y > 2.0;
        let worse = |x: f64, y: f64| y > x * 1.3 && y - x > 2.0;
        let improved = better(b.jitter_ms, a.jitter_ms) || better(b.p95_ms, a.p95_ms);
        let degraded = worse(b.jitter_ms, a.jitter_ms)
            || worse(b.p95_ms, a.p95_ms)
            || a.loss_pct > b.loss_pct + 5.0;
        match (improved, degraded) {
            (true, false) => "flatter",
            (false, true) => "worse",
            _ => "same",
        }
    };
    PingComparison {
        before: b,
        after: a,
        verdict: verdict.into(),
    }
}

struct Target {
    id: &'static str,
    label: String,
    addr: Option<IpAddr>,
    samples: VecDeque<PingSample>,
    problem: String,
    via: String,
    through_vpn: bool,
}

/// Below this, an internet host "answering" through a TUN device is the VPN
/// program replying for it: no real path to a server is that short.
const FAKE_MS: f64 = 2.0;

/// The verdict on a target's numbers, given how its echoes travel.
fn judge(id: &str, through_vpn: bool, s: &PingStats) -> Option<String> {
    (id != "gateway" && through_vpn && s.received >= 5 && s.median_ms < FAKE_MS).then(|| {
        "Answered by the VPN program itself (TUN mode), not by the real server: this number is not a real ping.".into()
    })
}

struct State {
    targets: Vec<Target>,
    marks: Vec<PingMark>,
    wanted_until: Option<Instant>,
    running: bool,
}

/// The shared measuring state; cheap to clone.
#[derive(Clone)]
pub struct Pinger {
    state: Arc<Mutex<State>>,
    root: SysRoot,
}

impl Pinger {
    pub fn new(root: SysRoot) -> Pinger {
        let targets = [
            ("gateway", "Router"),
            ("internet", "DNS server"),
            ("game", ""),
        ]
        .into_iter()
        .map(|(id, label)| Target {
            id,
            label: label.into(),
            addr: None,
            samples: VecDeque::new(),
            problem: String::new(),
            via: String::new(),
            through_vpn: false,
        })
        .collect();
        Pinger {
            state: Arc::new(Mutex::new(State {
                targets,
                marks: Vec::new(),
                wanted_until: None,
                running: false,
            })),
            root,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Keeps measuring for another minute, starting the thread if needed.
    pub fn touch(&self) {
        let mut s = self.lock();
        s.wanted_until = Some(Instant::now() + LINGER);
        if !s.running {
            s.running = true;
            let me = self.clone();
            let spawned = std::thread::Builder::new()
                .name("ping".into())
                .stack_size(128 * 1024)
                .spawn(move || me.run());
            if let Err(e) = spawned {
                tracing::warn!(error = %e, "could not start measuring ping");
                s.running = false;
            }
        }
    }

    /// The game's server, or `None` when no game is running.
    pub fn set_game(&self, game: Option<(String, Option<IpAddr>)>) {
        let mut s = self.lock();
        let t = s
            .targets
            .iter_mut()
            .find(|t| t.id == "game")
            .expect("game target");
        match game {
            Some((name, addr)) => {
                if t.addr != addr && addr.is_some() {
                    t.samples.clear();
                }
                t.label = name;
                t.addr = addr.or(t.addr);
                t.problem = if t.addr.is_none() {
                    "No server connection seen yet.".into()
                } else {
                    String::new()
                };
            }
            None => {
                t.label.clear();
                t.addr = None;
                t.problem.clear();
            }
        }
    }

    pub fn mark(&self, kind: &str, ts: u64) {
        let mut s = self.lock();
        s.marks.push(PingMark {
            ts_unix_ms: ts,
            kind: kind.into(),
        });
        s.marks.retain(|m| m.ts_unix_ms + KEEP_MS > ts);
    }

    pub fn report(&self, since: u64) -> PingReport {
        let s = self.lock();
        let now = crate::iface::now_ms();
        PingReport {
            targets: s
                .targets
                .iter()
                .filter(|t| t.addr.is_some() || !t.label.is_empty() || !t.problem.is_empty())
                .map(|t| {
                    let recent: Vec<PingSample> = t
                        .samples
                        .iter()
                        .filter(|p| p.ts_unix_ms + 60_000 >= now)
                        .copied()
                        .collect();
                    let stats = stats(&recent);
                    let problem = if t.problem.is_empty() {
                        judge(t.id, t.through_vpn, &stats).unwrap_or_default()
                    } else {
                        t.problem.clone()
                    };
                    PingTarget {
                        id: t.id.into(),
                        label: t.label.clone(),
                        addr: t.addr.map(|a| a.to_string()).unwrap_or_default(),
                        samples: t
                            .samples
                            .iter()
                            .filter(|p| p.ts_unix_ms >= since)
                            .copied()
                            .collect(),
                        stats,
                        problem,
                        via: t.via.clone(),
                        through_vpn: t.through_vpn,
                    }
                })
                .collect(),
            marks: s
                .marks
                .iter()
                .filter(|m| m.ts_unix_ms >= since)
                .cloned()
                .collect(),
        }
    }

    pub fn compare(&self, target: &str, mark: u64, window_s: u32) -> PingComparison {
        let s = self.lock();
        let samples: Vec<PingSample> = s
            .targets
            .iter()
            .find(|t| t.id == target)
            .map(|t| t.samples.iter().copied().collect())
            .unwrap_or_default();
        compare(&samples, mark, window_s.clamp(10, 600) as u64 * 1000)
    }

    fn run(self) {
        tracing::debug!("measuring ping");
        let mut probes: Vec<(&'static str, IpAddr, Probe)> = Vec::new();
        let mut resolved = Instant::now() - Duration::from_secs(3600);
        loop {
            let started = Instant::now();
            {
                let mut s = self.lock();
                if s.wanted_until.is_none_or(|t| t < started) {
                    s.running = false;
                    break;
                }
                // Routes and resolvers change when the network does.
                if resolved.elapsed() > Duration::from_secs(30) {
                    resolved = Instant::now();
                    let gw = procnet::gateway4(&self.root).map(|(_, a)| IpAddr::V4(a));
                    let dns = procnet::dns_servers(&self.root)
                        .into_iter()
                        .find(|a| a.is_ipv4())
                        .or_else(|| procnet::dns_servers(&self.root).into_iter().next());
                    for (id, addr, none) in [
                        ("gateway", gw, "No router: not connected."),
                        ("internet", dns, "No DNS server configured."),
                    ] {
                        let t = s.targets.iter_mut().find(|t| t.id == id).expect("target");
                        if t.addr != addr {
                            t.samples.clear();
                        }
                        t.addr = addr;
                        t.problem = if addr.is_none() {
                            none.into()
                        } else {
                            String::new()
                        };
                    }
                    // Which way each host is reached: directly, or through a
                    // VPN's TUN device (which may answer pings itself).
                    for t in s.targets.iter_mut() {
                        match t.addr {
                            Some(a) => {
                                t.via = procnet::route_dev(a).unwrap_or_default();
                                t.through_vpn =
                                    !t.via.is_empty() && procnet::is_tun(&self.root, &t.via);
                            }
                            None => {
                                t.via.clear();
                                t.through_vpn = false;
                            }
                        }
                    }
                }
                let want: Vec<(&'static str, IpAddr)> = s
                    .targets
                    .iter()
                    .filter_map(|t| Some((t.id, t.addr?)))
                    .collect();
                probes.retain(|(id, a, _)| want.contains(&(*id, *a)));
                for (id, addr) in want {
                    if probes.iter().any(|(i, _, _)| *i == id) {
                        continue;
                    }
                    match Probe::open(addr) {
                        Ok(p) => probes.push((id, addr, p)),
                        Err(e) => {
                            if let Some(t) = s.targets.iter_mut().find(|t| t.id == id) {
                                t.problem = format!("Cannot send ping: {e}");
                            }
                        }
                    }
                }
            }

            let ts = crate::iface::now_ms();
            let rtts = Probe::round(
                &mut probes.iter_mut().map(|(_, _, p)| p).collect::<Vec<_>>(),
                TIMEOUT,
            );
            {
                let mut s = self.lock();
                for ((id, _, _), rtt) in probes.iter().zip(rtts) {
                    if let Some(t) = s.targets.iter_mut().find(|t| t.id == *id) {
                        t.samples.push_back(PingSample {
                            ts_unix_ms: ts,
                            rtt_ms: rtt.unwrap_or(f64::NAN),
                        });
                        while t
                            .samples
                            .front()
                            .is_some_and(|p| p.ts_unix_ms + KEEP_MS < ts)
                        {
                            t.samples.pop_front();
                        }
                    }
                }
            }
            std::thread::sleep(INTERVAL.saturating_sub(started.elapsed()));
        }
        tracing::debug!("stopped measuring ping");
    }
}

/// One unprivileged ICMP echo socket for one host.
struct Probe {
    fd: OwnedFd,
    v6: bool,
    seq: u16,
    sent_at: Option<Instant>,
}

impl Probe {
    fn open(addr: IpAddr) -> std::io::Result<Probe> {
        let v6 = addr.is_ipv6();
        let (domain, proto) = if v6 {
            (libc::AF_INET6, libc::IPPROTO_ICMPV6)
        } else {
            (libc::AF_INET, libc::IPPROTO_ICMP)
        };
        // SAFETY: plain socket(2); the result is checked before use.
        let fd = unsafe {
            libc::socket(
                domain,
                libc::SOCK_DGRAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                proto,
            )
        };
        if fd < 0 {
            let e = std::io::Error::last_os_error();
            return Err(if e.raw_os_error() == Some(libc::EACCES) {
                std::io::Error::other("this user is outside net.ipv4.ping_group_range")
            } else {
                e
            });
        }
        // SAFETY: fd is a fresh descriptor we own.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let sa = socket2_addr(SocketAddr::new(addr, 0));
        // SAFETY: sa is a valid sockaddr of the stated length.
        if unsafe { libc::connect(fd.as_raw_fd(), sa.0.as_ptr() as *const libc::sockaddr, sa.1) }
            < 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Probe {
            fd,
            v6,
            seq: 0,
            sent_at: None,
        })
    }

    fn send(&mut self) {
        self.seq = self.seq.wrapping_add(1);
        // Type, code, checksum (the kernel fills it in for ping sockets), id
        // (the kernel sets it), sequence, then a little payload.
        let mut pkt = [0u8; 16];
        pkt[0] = if self.v6 { 128 } else { 8 };
        pkt[6..8].copy_from_slice(&self.seq.to_be_bytes());
        pkt[8..].copy_from_slice(b"ug-ping!");
        // SAFETY: the buffer is valid for its length; the socket is connected.
        let n = unsafe { libc::send(self.fd.as_raw_fd(), pkt.as_ptr() as *const _, pkt.len(), 0) };
        self.sent_at = (n == pkt.len() as isize).then(Instant::now);
    }

    /// Reads whatever arrived; `Some(rtt)` once the current echo is answered.
    fn receive(&mut self) -> Option<f64> {
        let mut buf = [0u8; 128];
        loop {
            // SAFETY: the buffer is valid for its length.
            let n = unsafe {
                libc::recv(
                    self.fd.as_raw_fd(),
                    buf.as_mut_ptr() as *mut _,
                    buf.len(),
                    0,
                )
            };
            if n < 8 {
                return None;
            }
            let reply = if self.v6 { 129 } else { 0 };
            if buf[0] == reply && u16::from_be_bytes([buf[6], buf[7]]) == self.seq {
                return self
                    .sent_at
                    .take()
                    .map(|t| t.elapsed().as_secs_f64() * 1000.0);
            }
            // A late reply to an earlier echo: skip it.
        }
    }

    /// Sends one echo on every probe and waits for replies until `timeout`.
    fn round(probes: &mut [&mut Probe], timeout: Duration) -> Vec<Option<f64>> {
        let mut out = vec![None; probes.len()];
        for p in probes.iter_mut() {
            p.send();
        }
        let deadline = Instant::now() + timeout;
        while out.iter().any(|r| r.is_none()) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let mut fds: Vec<libc::pollfd> = probes
                .iter()
                .map(|p| libc::pollfd {
                    fd: p.fd.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                })
                .collect();
            // SAFETY: fds is a valid array of pollfd for its length.
            let n = unsafe {
                libc::poll(
                    fds.as_mut_ptr(),
                    fds.len() as libc::nfds_t,
                    left.as_millis().max(1) as i32,
                )
            };
            if n <= 0 {
                if n < 0
                    && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                {
                    continue;
                }
                break;
            }
            for (i, f) in fds.iter().enumerate() {
                if f.revents & libc::POLLIN != 0 && out[i].is_none() {
                    out[i] = probes[i].receive();
                }
            }
        }
        out
    }
}

/// A `sockaddr_in`/`sockaddr_in6` as bytes plus its length.
fn socket2_addr(a: SocketAddr) -> (Vec<u8>, libc::socklen_t) {
    match a {
        SocketAddr::V4(v4) => {
            // SAFETY: sockaddr_in is plain old data; zeroed is a valid value.
            let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
            sa.sin_family = libc::AF_INET as libc::sa_family_t;
            sa.sin_addr.s_addr = u32::from_ne_bytes(v4.ip().octets());
            let len = std::mem::size_of::<libc::sockaddr_in>();
            // SAFETY: reading the struct's bytes.
            let bytes =
                unsafe { std::slice::from_raw_parts(&sa as *const _ as *const u8, len) }.to_vec();
            (bytes, len as libc::socklen_t)
        }
        SocketAddr::V6(v6) => {
            // SAFETY: as above.
            let mut sa: libc::sockaddr_in6 = unsafe { std::mem::zeroed() };
            sa.sin6_family = libc::AF_INET6 as libc::sa_family_t;
            sa.sin6_addr.s6_addr = v6.ip().octets();
            sa.sin6_scope_id = v6.scope_id();
            let len = std::mem::size_of::<libc::sockaddr_in6>();
            // SAFETY: reading the struct's bytes.
            let bytes =
                unsafe { std::slice::from_raw_parts(&sa as *const _ as *const u8, len) }.to_vec();
            (bytes, len as libc::socklen_t)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(rtts: &[f64], start: u64) -> Vec<PingSample> {
        rtts.iter()
            .enumerate()
            .map(|(i, &r)| PingSample {
                ts_unix_ms: start + i as u64 * 1000,
                rtt_ms: r,
            })
            .collect()
    }

    #[test]
    fn stats_of_a_steady_link() {
        let s = stats(&run(&[20.0, 22.0, 20.0, 22.0, f64::NAN], 0));
        assert_eq!((s.sent, s.received), (5, 4));
        assert_eq!(s.loss_pct, 20.0);
        assert_eq!(s.median_ms, 22.0); // index round(1.5) = 2 of [20,20,22,22]
        assert_eq!(s.jitter_ms, 2.0);
        assert_eq!(s.p95_ms, 22.0);
    }

    #[test]
    fn stats_with_nothing_back_are_all_loss() {
        let s = stats(&run(&[f64::NAN, f64::NAN], 0));
        assert_eq!(s.loss_pct, 100.0);
        assert!(s.median_ms.is_nan());
        assert_eq!(stats(&[]).loss_pct, 0.0);
    }

    #[test]
    fn compare_says_flatter_only_when_the_numbers_do() {
        let spiky: Vec<f64> = (0..30)
            .map(|i| if i % 2 == 0 { 20.0 } else { 90.0 })
            .collect();
        let flat: Vec<f64> = (0..30).map(|i| 24.0 + (i % 3) as f64).collect();
        let mut s = run(&spiky, 0);
        s.extend(run(&flat, 30_000));
        let c = compare(&s, 30_000, 30_000);
        assert_eq!(c.verdict, "flatter");
        assert!(c.after.jitter_ms < c.before.jitter_ms);

        // The same link before and after: no claim of improvement.
        let mut s = run(&flat, 0);
        s.extend(run(&flat, 30_000));
        assert_eq!(compare(&s, 30_000, 30_000).verdict, "same");

        let mut s = run(&flat, 0);
        s.extend(run(&spiky, 30_000));
        assert_eq!(compare(&s, 30_000, 30_000).verdict, "worse");

        assert_eq!(
            compare(&run(&flat[..5], 25_000), 30_000, 30_000).verdict,
            "not-enough-data"
        );
    }

    #[test]
    fn compare_ignores_samples_outside_the_window() {
        let spiky: Vec<f64> = (0..60)
            .map(|i| if i % 2 == 0 { 20.0 } else { 90.0 })
            .collect();
        let flat: Vec<f64> = (0..30).map(|_| 25.0).collect();
        let mut s = run(&spiky, 0);
        s.extend(run(&flat, 60_000));
        let c = compare(&s, 60_000, 20_000);
        assert_eq!(c.before.sent, 20);
        assert_eq!(c.after.sent, 20);
    }

    #[test]
    fn a_vpn_answering_pings_itself_is_called_out() {
        let fast = stats(&run(&[0.3, 0.2, 0.4, 0.3, 0.3, 0.2], 0));
        assert!(judge("internet", true, &fast).is_some());
        assert!(
            judge("internet", false, &fast).is_none(),
            "a direct fast host is fine (a LAN server)"
        );
        assert!(
            judge("gateway", true, &fast).is_none(),
            "the router is near"
        );
        let real = stats(&run(&[80.0, 82.0, 79.0, 81.0, 80.0], 0));
        assert!(
            judge("internet", true, &real).is_none(),
            "a real ping through the VPN is kept"
        );
    }

    #[test]
    fn a_game_without_a_server_says_so() {
        let p = Pinger::new(SysRoot::at("/nonexistent"));
        p.set_game(Some(("Counter-Strike 2".into(), None)));
        let r = p.report(0);
        let g = r.targets.iter().find(|t| t.id == "game").unwrap();
        assert_eq!(g.label, "Counter-Strike 2");
        assert!(!g.problem.is_empty());
        p.set_game(None);
        assert!(p.report(0).targets.iter().all(|t| t.id != "game"));
    }

    /// Real ICMP to this machine; skipped where ping sockets are not allowed.
    #[test]
    fn probe_measures_loopback() {
        let Ok(mut p) = Probe::open("127.0.0.1".parse().unwrap()) else {
            return;
        };
        let r = Probe::round(&mut [&mut p], Duration::from_millis(500));
        assert!(r[0].is_some_and(|ms| ms >= 0.0 && ms < 100.0), "{r:?}");
    }
}
