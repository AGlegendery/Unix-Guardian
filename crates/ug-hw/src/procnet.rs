//! Sockets, routes and name servers as `/proc` and `/etc` describe them.
//!
//! Used to find what to ping (the router, the DNS server, the server a game
//! is talking to) and which process owns which socket.

use crate::SysRoot;
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proto {
    Tcp,
    Udp,
}

/// One line of `/proc/net/{tcp,tcp6,udp,udp6}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SockEntry {
    pub proto: Proto,
    pub local: SocketAddr,
    pub remote: SocketAddr,
    /// Kernel TCP state (`01` established, `0A` listen); UDP uses `07` for
    /// unconnected and `01` for connected sockets.
    pub state: u8,
    pub uid: u32,
    pub inode: u64,
}

/// The IPv4 default gateway and its interface, lowest metric first.
pub fn gateway4(root: &SysRoot) -> Option<(String, Ipv4Addr)> {
    let text = std::fs::read_to_string(root.join("proc/net/route")).ok()?;
    let mut best: Option<(u32, String, Ipv4Addr)> = None;
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 8 || f[1] != "00000000" || f[7] != "00000000" {
            continue;
        }
        let Ok(gw) = u32::from_str_radix(f[2], 16) else { continue };
        let flags = u16::from_str_radix(f[3], 16).unwrap_or(0);
        // RTF_UP and RTF_GATEWAY; a default route without a gateway (a VPN
        // point-to-point link) has no router to ping.
        if flags & 0x3 != 0x3 || gw == 0 {
            continue;
        }
        let metric: u32 = f[6].parse().unwrap_or(u32::MAX);
        // The file stores the address in host byte order of a little-endian
        // machine: the bytes as they are in memory.
        let addr = Ipv4Addr::from(gw.to_le_bytes());
        if best.as_ref().is_none_or(|(m, _, _)| metric < *m) {
            best = Some((metric, f[0].to_string(), addr));
        }
    }
    best.map(|(_, i, a)| (i, a))
}

/// Name servers, skipping local stub resolvers (systemd-resolved,
/// dnsmasq) in favour of the upstream ones they forward to when known.
pub fn dns_servers(root: &SysRoot) -> Vec<IpAddr> {
    let parse = |rel: &str| -> Vec<IpAddr> {
        std::fs::read_to_string(root.join(rel))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.trim().strip_prefix("nameserver"))
            .filter_map(|a| a.trim().split('%').next()?.parse().ok())
            .collect()
    };
    let main: Vec<IpAddr> = parse("etc/resolv.conf").into_iter().filter(|a| !a.is_loopback()).collect();
    if !main.is_empty() {
        return main;
    }
    parse("run/systemd/resolve/resolv.conf").into_iter().filter(|a| !a.is_loopback()).collect()
}

/// Every socket in one of the `/proc/.../net/*` tables.
pub fn sockets(root: &SysRoot, dir: &str) -> Vec<SockEntry> {
    let mut out = Vec::new();
    for (file, proto) in [("tcp", Proto::Tcp), ("tcp6", Proto::Tcp), ("udp", Proto::Udp), ("udp6", Proto::Udp)] {
        let Ok(text) = std::fs::read_to_string(root.join(format!("{dir}/{file}"))) else { continue };
        out.extend(text.lines().skip(1).filter_map(|l| parse_line(l, proto)));
    }
    out
}

fn parse_line(line: &str, proto: Proto) -> Option<SockEntry> {
    let f: Vec<&str> = line.split_whitespace().collect();
    if f.len() < 10 {
        return None;
    }
    Some(SockEntry {
        proto,
        local: parse_addr(f[1])?,
        remote: parse_addr(f[2])?,
        state: u8::from_str_radix(f[3], 16).ok()?,
        uid: f[7].parse().ok()?,
        inode: f[9].parse().ok()?,
    })
}

/// `0101A8C0:0035` or the 32-hex-digit IPv6 form, each 32-bit word in the
/// machine's byte order.
fn parse_addr(s: &str) -> Option<SocketAddr> {
    let (host, port) = s.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let ip = match host.len() {
        8 => IpAddr::V4(Ipv4Addr::from(u32::from_str_radix(host, 16).ok()?.to_le_bytes())),
        32 => {
            let mut b = [0u8; 16];
            for w in 0..4 {
                let word = u32::from_str_radix(&host[w * 8..w * 8 + 8], 16).ok()?;
                b[w * 4..w * 4 + 4].copy_from_slice(&word.to_le_bytes());
            }
            let v6 = Ipv6Addr::from(b);
            v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6))
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, port))
}

/// Inodes of the sockets a process holds open. Empty when its `fd`
/// directory cannot be read (another user's process, or it exited).
pub fn socket_inodes(root: &SysRoot, pid: i32) -> HashSet<u64> {
    let Ok(dir) = std::fs::read_dir(root.join(format!("proc/{pid}/fd"))) else { return HashSet::new() };
    dir.flatten()
        .filter_map(|e| std::fs::read_link(e.path()).ok())
        .filter_map(|t| t.to_str()?.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok())
        .collect()
}

/// An address on the internet, not on this machine or the local network.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => {
            !(a.is_private()
                || a.is_loopback()
                || a.is_link_local()
                || a.is_unspecified()
                || a.is_multicast()
                || a.is_broadcast()
                || a.octets()[0] == 100 && a.octets()[1] & 0xc0 == 64) // CGNAT
        }
        IpAddr::V6(a) => {
            let s0 = a.segments()[0];
            !(a.is_loopback() || a.is_unspecified() || a.is_multicast() || s0 & 0xfe00 == 0xfc00 || s0 & 0xffc0 == 0xfe80)
        }
    }
}

/// The internet host a set of processes talks to most directly: a connected
/// UDP socket first (game traffic), then an established TCP connection.
pub fn game_peer(root: &SysRoot, pids: &[i32]) -> Option<IpAddr> {
    let mut inodes = HashSet::new();
    let mut table = Vec::new();
    for &pid in pids {
        let own = socket_inodes(root, pid);
        if own.is_empty() {
            continue;
        }
        if table.is_empty() {
            // Every process in one network namespace sees the same tables.
            table = sockets(root, &format!("proc/{pid}/net"));
        }
        inodes.extend(own);
    }
    let mine = || table.iter().filter(|s| inodes.contains(&s.inode) && is_public(s.remote.ip()));
    mine()
        .find(|s| s.proto == Proto::Udp && s.remote.port() != 0)
        .or_else(|| mine().find(|s| s.proto == Proto::Tcp && s.state == 1))
        .map(|s| s.remote.ip())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    const ROUTE: &str = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
        wlp8s0\t00000000\t0101A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
        enp7s0\t00000000\t0100A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
        lxcbr0\t0003000A\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0";

    #[test]
    fn gateway_takes_the_lowest_metric_default_route() {
        let fs = TestFs::new();
        fs.write("proc/net/route", ROUTE);
        let (iface, gw) = gateway4(&SysRoot::at(fs.root())).unwrap();
        assert_eq!(iface, "enp7s0");
        assert_eq!(gw, Ipv4Addr::new(192, 168, 0, 1));
    }

    #[test]
    fn no_gateway_without_a_default_route() {
        let fs = TestFs::new();
        fs.write("proc/net/route", "Iface\tDestination\tGateway\n lo\t0000007F\t00000000\t0001\t0\t0\t0\t000000FF");
        assert_eq!(gateway4(&SysRoot::at(fs.root())), None);
    }

    #[test]
    fn dns_skips_the_local_stub_for_the_upstream_servers() {
        let fs = TestFs::new();
        fs.write("etc/resolv.conf", "# stub\nnameserver 127.0.0.53\noptions edns0");
        fs.write("run/systemd/resolve/resolv.conf", "nameserver 192.168.1.1\nnameserver fe80::1%wlp8s0");
        let got = dns_servers(&SysRoot::at(fs.root()));
        assert_eq!(got, vec!["192.168.1.1".parse::<IpAddr>().unwrap(), "fe80::1".parse().unwrap()]);
    }

    #[test]
    fn parses_v4_and_v6_socket_lines() {
        let v4 = "4262: 6401A8C0:0044 0101A8C0:0043 01 00000000:00000000 00:00000000 00000000     0        0 12381997 2 0 0";
        let s = parse_line(v4, Proto::Udp).unwrap();
        assert_eq!(s.local, "192.168.1.100:68".parse().unwrap());
        assert_eq!(s.remote, "192.168.1.1:67".parse().unwrap());
        assert_eq!((s.state, s.uid, s.inode), (1, 0, 12381997));

        // ::1 port 631, and a v4-mapped peer that must read as plain IPv4.
        let v6 = "0: 00000000000000000000000001000000:0277 0000000000000000FFFF00000101A8C0:0050 0A 0:0 00:0 0 1000 0 3480939 1";
        let s = parse_line(v6, Proto::Tcp).unwrap();
        assert_eq!(s.local, "[::1]:631".parse().unwrap());
        assert_eq!(s.remote, "192.168.1.1:80".parse().unwrap());
        assert_eq!(s.uid, 1000);
    }

    #[test]
    fn public_excludes_local_networks() {
        for a in ["192.168.1.1", "10.0.0.1", "127.0.0.1", "100.64.0.1", "fe80::1", "fd00::1", "0.0.0.0"] {
            assert!(!is_public(a.parse().unwrap()), "{a}");
        }
        for a in ["1.1.1.1", "155.133.248.34", "2606:4700::1111"] {
            assert!(is_public(a.parse().unwrap()), "{a}");
        }
    }

    #[test]
    fn game_peer_prefers_the_games_connected_udp_socket() {
        let fs = TestFs::new();
        let root = SysRoot::at(fs.root());
        fs.dir("proc/42/fd");
        std::os::unix::fs::symlink("socket:[111]", fs.root().join("proc/42/fd/5")).unwrap();
        std::os::unix::fs::symlink("socket:[222]", fs.root().join("proc/42/fd/6")).unwrap();
        std::os::unix::fs::symlink("/dev/null", fs.root().join("proc/42/fd/0")).unwrap();
        // 111: TCP to a web API; 222: UDP to the match server; 333: someone else's.
        fs.write(
            "proc/42/net/tcp",
            "sl local rem st\n0: 6401A8C0:C000 01010101:01BB 01 0:0 0:0 0 1000 0 111 1",
        );
        fs.write(
            "proc/42/net/udp",
            "sl local rem st\n0: 6401A8C0:C001 22F8859B:6987 01 0:0 0:0 0 1000 0 222 2\n\
             1: 6401A8C0:C002 08080808:0035 01 0:0 0:0 0 1000 0 333 2",
        );
        assert_eq!(game_peer(&root, &[42, 43]), Some("155.133.248.34".parse().unwrap()));
        assert_eq!(game_peer(&root, &[43]), None);
    }
}
