//! Noticing a background task that competes with the game for the network,
//! and offering to cap it while the game runs.

use std::collections::HashSet;
use ug_proto::AppTraffic;

/// Busy enough to hurt ping: 500 kB/s either way.
const BUSY_BPS: u64 = 500_000;
/// Uploading this much makes an unknown program worth a mention too:
/// upload is what fills the modem's queue.
const UPLOAD_BPS: u64 = 125_000;

/// Caps offered from the notification: about 1 MB/s down, 125 kB/s up.
pub const OFFER_DOWN_KBPS: u32 = 8_000;
pub const OFFER_UP_KBPS: u32 = 1_000;

/// The busiest app worth a notification: a known background kind moving
/// 500 kB/s, or anything uploading 125 kB/s; never the game's own group, one
/// that already has a rule, or one already offered this game session.
pub fn pick<'a>(apps: &'a [AppTraffic], ruled: &HashSet<String>, offered: &HashSet<String>) -> Option<&'a AppTraffic> {
    apps.iter()
        // A VPN carries the other apps' traffic: it is never the culprit.
        .filter(|a| !a.has_game && a.state.is_empty() && a.kind != "session" && a.category != "vpn")
        .filter(|a| !ruled.contains(&a.name.to_lowercase()) && !offered.contains(&a.name.to_lowercase()))
        .filter(|a| (!a.category.is_empty() && a.rx_bps + a.tx_bps >= BUSY_BPS) || a.tx_bps >= UPLOAD_BPS)
        .max_by_key(|a| a.rx_bps + a.tx_bps)
}

/// "Steam is downloading 4.2 MB/s while you play."
pub fn sentence(a: &AppTraffic) -> String {
    let mb = |b: u64| b as f64 / 1e6;
    let (verb, rate) = if a.tx_bps > a.rx_bps { ("uploading", a.tx_bps) } else { ("downloading", a.rx_bps) };
    format!("{} is {verb} {:.1} MB/s while you play, which can make your ping jump.", a.name, mb(rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, category: &str, rx: u64, tx: u64) -> AppTraffic {
        AppTraffic {
            id: format!("x/{name}"),
            name: name.into(),
            kind: "app".into(),
            uid: 1000,
            exe: name.into(),
            pids: vec![1],
            rx_bps: rx,
            tx_bps: tx,
            rx_bytes: 0,
            tx_bytes: 0,
            state: String::new(),
            category: category.into(),
            has_game: false,
        }
    }

    #[test]
    fn a_busy_launcher_download_is_offered_once() {
        let apps = vec![app("firefox", "", 3_000_000, 10_000), app("steam", "launcher", 4_200_000, 20_000)];
        let none = HashSet::new();
        let p = pick(&apps, &none, &none).unwrap();
        assert_eq!(p.name, "steam");
        assert_eq!(sentence(p), "steam is downloading 4.2 MB/s while you play, which can make your ping jump.");
        let offered: HashSet<String> = ["steam".to_string()].into();
        assert!(pick(&apps, &none, &offered).is_none(), "a browser download alone is not flagged");
    }

    #[test]
    fn heavy_upload_from_anything_counts_but_not_the_game_itself() {
        let mut up = app("mystery", "", 0, 200_000);
        assert!(pick(std::slice::from_ref(&up), &HashSet::new(), &HashSet::new()).is_some());
        up.has_game = true;
        assert!(pick(&[up], &HashSet::new(), &HashSet::new()).is_none());
    }

    #[test]
    fn the_vpn_carrying_everything_is_not_blamed() {
        let apps = vec![app("v2rayn", "vpn", 6_000_000, 400_000)];
        assert!(pick(&apps, &HashSet::new(), &HashSet::new()).is_none());
    }

    #[test]
    fn an_app_with_a_rule_is_left_alone() {
        let apps = vec![app("qbittorrent", "torrent", 100_000, 900_000)];
        let ruled: HashSet<String> = ["qbittorrent".to_string()].into();
        assert!(pick(&apps, &ruled, &HashSet::new()).is_none());
    }
}
