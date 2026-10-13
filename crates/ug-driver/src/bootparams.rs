//! Kernel boot parameters, changed the boot loader's own way (roadmap 8.7).
//!
//! GRUB: `GRUB_CMDLINE_LINUX_DEFAULT` in `/etc/default/grub`, then the
//! distribution's config generator. systemd-boot: the `options` line of each
//! entry in `/boot/loader/entries` (and `/etc/kernel/cmdline`, which
//! `kernel-install` reads for new kernels). rEFInd: the options in
//! `/boot/refind_linux.conf`. Pop!_OS: `kernelstub`.
//!
//! Edits are idempotent and exact: a parameter is added at the end, or
//! replaces the one with the same name; removing it again gives back the
//! file byte for byte. Files are written through the executor, which backs
//! them up first.

use ug_hw::SysRoot;

/// How this machine's boot loader takes kernel parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum Loader {
    /// The generator to run after editing `/etc/default/grub`.
    Grub {
        regen: Vec<String>,
    },
    /// Entry files (absolute paths), and whether `/etc/kernel/cmdline` is used.
    SystemdBoot {
        entries: Vec<String>,
        kernel_cmdline: bool,
    },
    Refind,
    Kernelstub,
}

fn has(root: &SysRoot, p: &str) -> bool {
    ["usr/bin", "usr/sbin", "bin", "sbin"]
        .iter()
        .any(|d| root.join(d).join(p).exists())
}

pub fn detect(root: &SysRoot) -> Option<Loader> {
    if has(root, "kernelstub") {
        return Some(Loader::Kernelstub);
    }
    if root.join("etc/default/grub").exists() {
        let regen: Vec<&str> = if has(root, "update-grub") {
            vec!["update-grub"]
        } else if has(root, "grub2-mkconfig") {
            vec!["grub2-mkconfig", "-o", "/boot/grub2/grub.cfg"]
        } else if has(root, "grub-mkconfig") {
            vec!["grub-mkconfig", "-o", "/boot/grub/grub.cfg"]
        } else {
            return None;
        };
        return Some(Loader::Grub {
            regen: regen.into_iter().map(String::from).collect(),
        });
    }
    let mut entries: Vec<String> = [
        "boot/loader/entries",
        "efi/loader/entries",
        "boot/efi/loader/entries",
    ]
    .iter()
    .flat_map(|d| {
        std::fs::read_dir(root.join(d))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "conf"))
            .map(move |e| format!("/{d}/{}", e.file_name().to_string_lossy()))
    })
    .collect();
    entries.sort();
    if !entries.is_empty() {
        return Some(Loader::SystemdBoot {
            entries,
            kernel_cmdline: root.join("etc/kernel/cmdline").exists(),
        });
    }
    if root.join("boot/refind_linux.conf").exists() {
        return Some(Loader::Refind);
    }
    None
}

fn key(p: &str) -> &str {
    p.split('=').next().unwrap_or(p)
}

/// A command line with `add` added (replacing a parameter of the same name)
/// and every parameter named in `remove` taken out. Unchanged text comes
/// back unchanged.
pub fn edit_line(line: &str, add: &[&str], remove: &[&str]) -> String {
    let mut out = line.to_string();
    for r in remove {
        // Words are single-space separated; dropping one keeps the rest as is.
        let words: Vec<&str> = out.split(' ').collect();
        if !words.iter().any(|w| key(w) == key(r)) {
            continue;
        }
        out = words
            .into_iter()
            .filter(|w| key(w) != key(r))
            .collect::<Vec<_>>()
            .join(" ");
    }
    for a in add {
        if out.split(' ').any(|w| w == *a) {
            continue;
        }
        if out.split(' ').any(|w| key(w) == key(a)) {
            out = out
                .split(' ')
                .map(|w| if key(w) == key(a) { *a } else { w })
                .collect::<Vec<_>>()
                .join(" ");
        } else if out.is_empty() {
            out = a.to_string();
        } else {
            out = format!("{out} {a}");
        }
    }
    out
}

/// `/etc/default/grub` with `GRUB_CMDLINE_LINUX_DEFAULT` edited (added if
/// the file has none).
pub fn edit_grub(text: &str, add: &[&str], remove: &[&str]) -> String {
    let var = "GRUB_CMDLINE_LINUX_DEFAULT=";
    let mut found = false;
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|l| {
            let Some(v) = l.strip_prefix(var) else {
                return l.to_string();
            };
            found = true;
            let (q, inner) = match v.chars().next() {
                Some(c @ ('"' | '\'')) => {
                    (c.to_string(), v.trim_start_matches(c).trim_end_matches(c))
                }
                _ => (String::new(), v),
            };
            format!("{var}{q}{}{q}", edit_line(inner, add, remove))
        })
        .collect();
    if !found && !add.is_empty() {
        let at = if text.ends_with('\n') {
            lines.len() - 1
        } else {
            lines.len()
        };
        lines.insert(at, format!("{var}\"{}\"", edit_line("", add, &[])));
    }
    lines.join("\n")
}

/// A systemd-boot entry with its `options` line edited.
pub fn edit_entry(text: &str, add: &[&str], remove: &[&str]) -> String {
    text.split('\n')
        .map(|l| match l.strip_prefix("options") {
            Some(rest) if rest.starts_with([' ', '\t']) => {
                let pad: String = rest.chars().take_while(|c| c.is_whitespace()).collect();
                format!("options{pad}{}", edit_line(rest.trim_start(), add, remove))
            }
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// rEFInd's `"title"  "options"` lines with the options edited.
pub fn edit_refind(text: &str, add: &[&str], remove: &[&str]) -> String {
    text.split('\n')
        .map(|l| {
            let parts: Vec<&str> = l.splitn(4, '"').collect();
            // ["", title, spaces, rest-with-options"]
            if parts.len() == 4 && parts[0].trim().is_empty() {
                if let Some(opts) = parts[3].strip_suffix('"') {
                    return format!(
                        "\"{}\"{}\"{}\"",
                        parts[1],
                        parts[2],
                        edit_line(opts, add, remove)
                    );
                }
            }
            l.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `/etc/kernel/cmdline`: one line of parameters.
pub fn edit_kernel_cmdline(text: &str, add: &[&str], remove: &[&str]) -> String {
    let nl = if text.ends_with('\n') { "\n" } else { "" };
    format!(
        "{}{nl}",
        edit_line(text.trim_end_matches('\n'), add, remove)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ug_testfs::TestFs;

    const GRUB: &str = "# If you change this file, run 'update-grub'\nGRUB_DEFAULT=0\nGRUB_CMDLINE_LINUX_DEFAULT=\"quiet splash nomodeset\"\nGRUB_CMDLINE_LINUX=\"\"\n";

    #[test]
    fn a_parameter_added_and_removed_gives_the_file_back_byte_for_byte() {
        let added = edit_grub(GRUB, &["mem_sleep_default=deep"], &[]);
        assert!(added.contains(
            "GRUB_CMDLINE_LINUX_DEFAULT=\"quiet splash nomodeset mem_sleep_default=deep\""
        ));
        assert_eq!(
            edit_grub(&added, &["mem_sleep_default=deep"], &[]),
            added,
            "idempotent"
        );
        assert_eq!(
            edit_grub(&added, &[], &["mem_sleep_default"]),
            GRUB,
            "diff-empty round trip"
        );
        let no = edit_grub(GRUB, &[], &["nomodeset"]);
        assert!(no.contains("=\"quiet splash\""));
        assert_eq!(edit_grub(GRUB, &[], &["absent"]), GRUB);
        // The same name with a new value replaces it in place.
        let v = edit_grub(&added, &["mem_sleep_default=s2idle"], &[]);
        assert!(v.contains("nomodeset mem_sleep_default=s2idle\""));
        let none = "GRUB_DEFAULT=0\n";
        assert_eq!(
            edit_grub(none, &["a=1"], &[]),
            "GRUB_DEFAULT=0\nGRUB_CMDLINE_LINUX_DEFAULT=\"a=1\"\n"
        );
    }

    #[test]
    fn systemd_boot_refind_and_kernel_cmdline() {
        let entry = "title   Arch Linux\nlinux   /vmlinuz-linux\noptions root=UUID=x rw quiet\n";
        let e = edit_entry(entry, &["psmouse.synaptics_intertouch=1"], &[]);
        assert!(e.contains("options root=UUID=x rw quiet psmouse.synaptics_intertouch=1\n"));
        assert_eq!(
            edit_entry(&e, &[], &["psmouse.synaptics_intertouch"]),
            entry
        );
        let refind = "\"Boot with standard options\"  \"ro root=UUID=x quiet\"\n\"Boot to single-user mode\"    \"ro root=UUID=x single\"\n";
        let r = edit_refind(refind, &["mem_sleep_default=deep"], &[]);
        assert!(r.contains(
            "\"Boot with standard options\"  \"ro root=UUID=x quiet mem_sleep_default=deep\""
        ));
        assert_eq!(edit_refind(&r, &[], &["mem_sleep_default"]), refind);
        assert_eq!(
            edit_kernel_cmdline("root=x quiet\n", &["a=1"], &[]),
            "root=x quiet a=1\n"
        );
    }

    #[test]
    fn the_boot_loader_is_found_from_its_files() {
        let fs = TestFs::new();
        let root = SysRoot::at(fs.root());
        assert_eq!(detect(&root), None);
        fs.write("boot/loader/entries/arch.conf", "options rw\n")
            .write("etc/kernel/cmdline", "rw\n");
        assert_eq!(
            detect(&root),
            Some(Loader::SystemdBoot {
                entries: vec!["/boot/loader/entries/arch.conf".into()],
                kernel_cmdline: true
            })
        );
        fs.write("etc/default/grub", GRUB)
            .write("usr/sbin/update-grub", "");
        assert_eq!(
            detect(&root),
            Some(Loader::Grub {
                regen: vec!["update-grub".into()]
            })
        );
        fs.write("usr/bin/kernelstub", "");
        assert_eq!(detect(&root), Some(Loader::Kernelstub));
    }
}
