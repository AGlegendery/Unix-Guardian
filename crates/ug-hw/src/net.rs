//! Default-route interface and its throughput.
//!
//! Which interface matters is a routing question, not a naming one: a machine
//! can have `eth0`, `wlan0`, a VPN `tun0` and several bridges up at once. The
//! default route is read from `/proc/net/route`, so the answer follows the
//! kernel rather than a guess about naming conventions, and moving between
//! Wi-Fi and Ethernet is picked up without a restart.

use crate::{read_trimmed, SysRoot};
use std::time::Instant;
use ug_proto::NetSample;

/// Turns successive interface byte counters into rates.
///
/// Holds the previous counters, so it must live across the sampling loop.
#[derive(Debug, Default)]
pub struct NetReader {
    prev: Option<(String, u64, u64, Instant)>,
}

impl NetReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Samples the current default-route interface.
    ///
    /// Rates are zero on the first read, and again whenever the interface
    /// changes: the counters belong to a different device, so a delta across
    /// the change would be meaningless rather than merely imprecise.
    pub fn read(&mut self, root: &SysRoot) -> NetSample {
        let Some(iface) = default_iface(root) else {
            self.prev = None;
            return NetSample {
                iface: String::new(),
                rx_bps: 0,
                tx_bps: 0,
                rx_total_bytes: 0,
                tx_total_bytes: 0,
            };
        };

        let rx = counter(root, &iface, "rx_bytes").unwrap_or(0);
        let tx = counter(root, &iface, "tx_bytes").unwrap_or(0);
        let now = Instant::now();

        let (rx_bps, tx_bps) = match &self.prev {
            Some((prev_iface, prev_rx, prev_tx, at)) if *prev_iface == iface => {
                let secs = now.duration_since(*at).as_secs_f64();
                if secs > 0.0 {
                    (
                        ((rx.saturating_sub(*prev_rx)) as f64 / secs) as u64,
                        ((tx.saturating_sub(*prev_tx)) as f64 / secs) as u64,
                    )
                } else {
                    (0, 0)
                }
            }
            _ => (0, 0),
        };

        self.prev = Some((iface.clone(), rx, tx, now));
        NetSample {
            iface,
            rx_bps,
            tx_bps,
            rx_total_bytes: rx,
            tx_total_bytes: tx,
        }
    }
}

/// Name of the interface carrying the IPv4 default route.
///
/// `/proc/net/route` is tab-separated with a header; a default route has
/// destination `00000000`. Where several exist, the lowest metric wins, which
/// is the same rule the kernel applies.
pub fn default_iface(root: &SysRoot) -> Option<String> {
    let content = read_trimmed(root.join("proc/net/route"))?;
    content
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 7 || f[1] != "00000000" {
                return None;
            }
            let metric: u32 = f[6].parse().unwrap_or(u32::MAX);
            Some((metric, f[0].to_string()))
        })
        .min_by_key(|(metric, _)| *metric)
        .map(|(_, iface)| iface)
}

/// Reads one interface statistics counter.
fn counter(root: &SysRoot, iface: &str, name: &str) -> Option<u64> {
    crate::read_u64(root.join(format!("sys/class/net/{iface}/statistics/{name}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    fn route_fs() -> TestFs {
        let fs = TestFs::new();
        fs.write(
            "proc/net/route",
            "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
             wlan0\t00000000\t0100A8C0\t0003\t0\t0\t600\t00000000\n\
             eth0\t00000000\t0100A8C0\t0003\t0\t0\t100\t00000000\n\
             eth0\t0000A8C0\t00000000\t0001\t0\t0\t100\t00FFFFFF",
        );
        fs
    }

    #[test]
    fn lowest_metric_default_route_wins() {
        let fs = route_fs();
        assert_eq!(
            default_iface(&SysRoot::at(fs.root())).as_deref(),
            Some("eth0")
        );
    }

    #[test]
    fn offline_machine_reports_an_empty_iface() {
        let fs = TestFs::new();
        fs.write(
            "proc/net/route",
            "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask",
        );
        let mut r = NetReader::new();
        let s = r.read(&SysRoot::at(fs.root()));
        assert_eq!(s.iface, "");
        assert_eq!(s.rx_bps, 0);
    }

    #[test]
    fn rates_come_from_the_delta_between_reads() {
        let fs = route_fs();
        let root = SysRoot::at(fs.root());
        fs.write("sys/class/net/eth0/statistics/rx_bytes", "1000");
        fs.write("sys/class/net/eth0/statistics/tx_bytes", "500");

        let mut r = NetReader::new();
        let first = r.read(&root);
        assert_eq!(first.rx_bps, 0, "no baseline yet");
        assert_eq!(first.rx_total_bytes, 1000);

        std::thread::sleep(std::time::Duration::from_millis(100));
        fs.write("sys/class/net/eth0/statistics/rx_bytes", "11000");
        let second = r.read(&root);
        assert!(
            second.rx_bps > 50_000,
            "10 kB in ~0.1 s, got {}",
            second.rx_bps
        );
    }

    #[test]
    fn switching_interface_does_not_produce_a_bogus_rate() {
        let fs = route_fs();
        let root = SysRoot::at(fs.root());
        fs.write("sys/class/net/eth0/statistics/rx_bytes", "9000000");
        fs.write("sys/class/net/eth0/statistics/tx_bytes", "0");
        fs.write("sys/class/net/wlan0/statistics/rx_bytes", "10");
        fs.write("sys/class/net/wlan0/statistics/tx_bytes", "0");

        let mut r = NetReader::new();
        r.read(&root);

        // Cable unplugged: wlan0 is now the only default route.
        fs.write(
            "proc/net/route",
            "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
             wlan0\t00000000\t0100A8C0\t0003\t0\t0\t600\t00000000",
        );
        let s = r.read(&root);
        assert_eq!(s.iface, "wlan0");
        assert_eq!(s.rx_bps, 0, "counters belong to a different device");
    }
}
