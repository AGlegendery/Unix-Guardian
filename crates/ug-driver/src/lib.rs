//! The driver troubleshooter (roadmap P8): graphics first, and every other
//! device a Linux user fights with (`hardware`).
//!
//! - [`inventory`] reads the machine into facts, from files only.
//! - [`rules`] turns facts into findings; the problems are data in
//!   `data/driver-rules/*.toml`.
//! - [`fixes`] says what each fix does: automatic ones (our own files in
//!   `modprobe.d`, units, an initramfs rebuild) or commands to copy.
//! - [`apply`] runs automatic fixes with a backup and an undo that works
//!   from a text console. The daemon owns it; diagnosis runs anywhere.

pub mod apply;
pub mod fixes;
pub mod hardware;
pub mod inventory;
pub mod rules;

pub use inventory::{Env, Facts, Value};
use ug_hw::SysRoot;
use ug_proto::{DriverFact, DriverFinding, DriverReport};

/// The facts shown under "what was found", in this order.
const SHOWN: &[&str] = &[
    "distro.name",
    "kernel.release",
    "session.type",
    "gpu.names",
    "nvidia.kernel_version",
    "nvidia.lib_version",
    "dkms.nvidia_versions",
    "kernel.headers",
    "module.nvidia",
    "module.nouveau",
    "nouveau.blacklisted",
    "nvidia.drm_modeset",
    "nvidia.drm_fbdev",
    "nvidia.preserve_vram",
    "nvidia.suspend_units_enabled",
    "boot.secure_boot",
    "kernel.lockdown",
    "boot.initramfs_tool",
    "vulkan.icds",
    "flatpak.gl_nvidia",
    "wifi.drivers",
    "bt.adapters",
    "audio.cards",
    "audio.server",
    "camera.video_devices",
    "sleep.mem_sleep",
    "firmware.missing",
    "kernel.cmdline",
];

/// Reads the machine and checks every rule.
pub fn diagnose(root: &SysRoot, env: &Env) -> DriverReport {
    report(&inventory::facts(root, env))
}

/// Findings for already-read facts, most severe first.
pub fn report(facts: &Facts) -> DriverReport {
    let mut findings = Vec::new();
    match rules::shipped() {
        Ok(rules) => {
            for r in rules.iter().filter(|r| rules::matches(r, facts)) {
                findings.push(DriverFinding {
                    id: r.id.clone(),
                    severity: r.severity.clone(),
                    category: r.category.clone(),
                    title_en: r.title_en.clone(),
                    title_fa: r.title_fa.clone(),
                    what_en: r.what_en.clone(),
                    what_fa: r.what_fa.clone(),
                    why_en: r.why_en.clone(),
                    why_fa: r.why_fa.clone(),
                    how_en: r.how_en.clone(),
                    how_fa: r.how_fa.clone(),
                    fix_id: r.fix.clone(),
                    fix_auto: fixes::AUTO.contains(&r.fix.as_str()) && facts.get("distro.can_change").is_some_and(Value::truthy),
                    commands: fixes::commands(&r.fix, facts),
                    doc_url: r.doc_url.clone(),
                });
            }
        }
        Err(e) => findings.push(DriverFinding {
            id: "rules-unreadable".into(),
            severity: "advice".into(),
            title_en: "The troubleshooter's rules could not be read".into(),
            title_fa: "قوانین عیب‌یاب خوانده نشد".into(),
            what_en: e.clone(),
            what_fa: e,
            ..Default::default()
        }),
    }
    let rank = |s: &str| match s {
        "critical" => 0,
        "important" => 1,
        _ => 2,
    };
    findings.sort_by_key(|f| rank(&f.severity));
    // One fix offered once: the first (most severe) finding keeps it.
    let mut seen = std::collections::HashSet::new();
    for f in &mut findings {
        if !f.fix_id.is_empty() && !seen.insert(f.fix_id.clone()) {
            f.fix_auto = false;
        }
    }
    let shown = SHOWN
        .iter()
        .filter_map(|k| facts.get(*k).map(|v| DriverFact { key: k.to_string(), value: v.text() }))
        .collect();
    DriverReport {
        facts: shown,
        findings,
        distro: facts.get("distro.name").map(Value::text).unwrap_or_default(),
        can_change: facts.get("distro.can_change").is_some_and(Value::truthy),
    }
}

#[cfg(test)]
mod tests;
