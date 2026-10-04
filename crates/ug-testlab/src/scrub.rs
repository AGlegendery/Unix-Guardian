//! Takes personal identifiers out of the report before anyone sees it.
//!
//! The report is meant to be pasted into an email to strangers, so it is
//! scrubbed as a whole, after it is written: a check added later cannot leak
//! a disk UUID by forgetting to call a helper. What goes: the home folder
//! (replaced by `~`), the host name and user name as whole words, UUIDs, MAC
//! addresses, IPv4 and IPv6 addresses, and long hex strings (machine-id,
//! serial numbers). What stays: PCI addresses, versions, dates and times —
//! the things a bug report needs and that identify a model, not a person.

/// What to remove, read from this session.
#[derive(Debug, Clone, Default)]
pub struct Scrubber {
    home: String,
    /// (word, replacement), longest first so `alice-pc` goes before `alice`.
    words: Vec<(String, &'static str)>,
}

impl Scrubber {
    /// The real session's home folder, user name and host name. Words in
    /// `keep` are never treated as names: a host is often called after its
    /// distribution (`debian`, `parrot`), and the report must still say
    /// which distribution it ran on.
    pub fn from_session(keep: &[&str]) -> Scrubber {
        let host = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
        let user = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_default();
        let home = std::env::var("HOME").unwrap_or_default();
        Scrubber::new(&home, &user, host.trim()).keeping(keep)
    }

    /// Never treats these words as names.
    pub fn keeping(mut self, keep: &[&str]) -> Scrubber {
        self.words.retain(|w| !keep.iter().any(|k| k.eq_ignore_ascii_case(&w.0)));
        self
    }

    pub fn new(home: &str, user: &str, host: &str) -> Scrubber {
        let mut words = Vec::new();
        // Very short names would eat ordinary words ("a", "pc"); a user called
        // that is not identified by them either.
        if host.len() >= 3 && host != "localhost" {
            words.push((host.to_string(), "<host>"));
        }
        if user.len() >= 3 && user != "root" {
            words.push((user.to_string(), "<user>"));
        }
        words.sort_by_key(|w| std::cmp::Reverse(w.0.len()));
        let home = home.trim_end_matches('/');
        Scrubber { home: if home.len() > 1 { home.to_string() } else { String::new() }, words }
    }

    pub fn scrub(&self, text: &str) -> String {
        let mut s = if self.home.is_empty() { text.to_string() } else { text.replace(&self.home, "~") };
        for (w, with) in &self.words {
            s = replace_word(&s, w, with);
        }
        scrub_patterns(&s)
    }
}

/// Replaces `word` where it is not part of a longer name.
fn replace_word(s: &str, word: &str, with: &str) -> String {
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(word) {
        let before = rest[..i].chars().next_back();
        let after = rest[i + word.len()..].chars().next();
        out.push_str(&rest[..i]);
        if before.is_some_and(ident) || after.is_some_and(ident) {
            out.push_str(word);
        } else {
            out.push_str(with);
        }
        rest = &rest[i + word.len()..];
    }
    out.push_str(rest);
    out
}

fn is_hex(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// What a run of hex digits, `:`, `.` and `-` is, if it identifies someone.
fn classify(run: &str) -> Option<&'static str> {
    let dash: Vec<&str> = run.split('-').collect();
    if dash.len() == 5 && [8, 4, 4, 4, 12].iter().zip(&dash).all(|(n, p)| p.len() == *n && is_hex(p)) {
        return Some("<uuid>");
    }
    let colon: Vec<&str> = run.split(':').collect();
    if colon.len() == 6 && colon.iter().all(|p| p.len() == 2 && is_hex(p)) {
        return Some("<mac>");
    }
    let dot: Vec<&str> = run.split('.').collect();
    if dot.len() == 4 && dot.iter().all(|p| !p.is_empty() && p.len() <= 3 && p.parse::<u8>().is_ok()) {
        return Some("<ip>");
    }
    // An IPv6 address has a `::` or at least four colons; times (15:40:22)
    // and PCI addresses (0000:01:00.0) have fewer.
    let colons = run.matches(':').count();
    if (run.contains("::") || colons >= 4)
        && run.chars().all(|c| c.is_ascii_hexdigit() || c == ':')
        && run.chars().any(|c| c.is_ascii_hexdigit())
    {
        return Some("<ipv6>");
    }
    if run.len() >= 32 && is_hex(run) {
        return Some("<id>");
    }
    None
}

fn scrub_patterns(s: &str) -> String {
    let in_run = |c: char| c.is_ascii_hexdigit() || matches!(c, ':' | '.' | '-');
    let mut out = String::with_capacity(s.len());
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];
        // A run must start at a word edge, so `abc` inside `cabcd`-like words
        // and `x` of `0x10de` are not cut in half.
        let edge = i == 0 || !chars[i - 1].1.is_alphanumeric();
        if !in_run(c) || !edge {
            out.push(c);
            i += 1;
            continue;
        }
        let mut j = i;
        while j < chars.len() && in_run(chars[j].1) {
            j += 1;
        }
        let end = chars.get(j).map(|c| c.0).unwrap_or(s.len());
        let whole = &s[start..end];
        // The run must also end at a word edge (`deadbeef…zz` is a word).
        let after_ok = chars.get(j).is_none_or(|c| !c.1.is_alphanumeric());
        // Sentence punctuation after an address is not part of it.
        let trimmed = whole.trim_end_matches(['.', ':', '-']);
        match classify(trimmed).filter(|_| after_ok) {
            Some(tag) => {
                out.push_str(tag);
                out.push_str(&whole[trimmed.len()..]);
            }
            None => out.push_str(whole),
        }
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Scrubber {
        Scrubber::new("/home/alice/", "alice", "alice-laptop")
    }

    #[test]
    fn the_home_folder_user_and_host_names_are_removed() {
        let out = s().scrub("config in /home/alice/.config, user alice on alice-laptop; malice stays");
        assert_eq!(out, "config in ~/.config, user <user> on <host>; malice stays");
    }

    #[test]
    fn disk_ids_and_network_addresses_are_removed() {
        let out = s().scrub(
            "root UUID=ba83246d-1c2f-4e5a-9b7c-0123456789ab, mac b8:1e:a4:12:34:56, \
             gw 192.168.1.1. dns fe80::1c2b:3dff:fe4e:5f60 and 2001:db8:0:0:0:0:0:1 \
             machine 0123456789abcdef0123456789abcdef.",
        );
        assert_eq!(out, "root UUID=<uuid>, mac <mac>, gw <ip>. dns <ipv6> and <ipv6> machine <id>.");
    }

    #[test]
    fn what_a_bug_report_needs_is_kept() {
        let text = "GPU 0000:01:00.0 (0x10de) driver 550.144.03, kernel 6.12.33+deb13, \
                    2026-10-04 15:40:22, 12.5 ms, id cafe";
        assert_eq!(s().scrub(text), text);
    }

    #[test]
    fn a_host_named_after_the_distribution_does_not_hide_it() {
        let sc = Scrubber::new("/home/bob", "bob", "debian").keeping(&["debian"]);
        assert_eq!(sc.scrub("Debian 13 (id debian) for bob"), "Debian 13 (id debian) for <user>");
    }

    #[test]
    fn short_or_generic_names_are_not_used_as_words() {
        let sc = Scrubber::new("/root", "root", "localhost");
        assert_eq!(sc.scrub("root on localhost"), "root on localhost");
    }
}
