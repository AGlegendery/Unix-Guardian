//! Guard for the 2026-10-04 incident: the card switch wrote NVIDIA's
//! offload variables into `~/.config/environment.d` and KDE's compositor
//! rendered on the wrong card. Unix Guardian sets GPU and MangoHud
//! variables per program only.
//!
//! This reads every source file in the workspace and fails when a place
//! the whole session reads shows up anywhere except the module that lists
//! them and moves old files away (`session_env.rs`), the old file's name
//! (`gpu_default.rs`) and the agent's call to that move (`main.rs`).

use std::path::{Path, PathBuf};

const PLACES: [&str; 7] = [
    "environment.d",
    "/etc/environment",
    "plasma-workspace/env",
    ".pam_environment",
    ".xprofile",
    ".xsessionrc",
    "set-environment",
];

const ALLOWED: [&str; 4] = [
    "crates/ug-core/src/session_env.rs",
    "crates/ug-core/src/gpu_default.rs",
    "crates/ug-core/tests/no_session_wide_writes.rs",
    "crates/ug-agent/src/main.rs",
];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if !["target", "node_modules", ".git", "dist", "build", ".svelte-kit"].contains(&name.as_str()) {
                sources(&p, out);
            }
        } else if [".rs", ".ts", ".svelte", ".sh", ".toml", ".service", ".desktop"].iter().any(|x| name.ends_with(x)) {
            out.push(p);
        }
    }
}

#[test]
fn nothing_writes_where_the_whole_session_reads() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for d in ["crates", "app/src", "app/src-tauri/src", "data", "packaging", "scripts"] {
        sources(&root.join(d), &mut files);
    }
    assert!(files.len() > 50, "found only {} source files under {}", files.len(), root.display());
    let mut found = Vec::new();
    for f in files {
        let rel = f.strip_prefix(&root).unwrap().display().to_string();
        if ALLOWED.contains(&rel.as_str()) || rel.ends_with("-e2e.sh") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        for (n, line) in text.lines().enumerate() {
            for p in PLACES {
                if line.contains(p) {
                    found.push(format!("{rel}:{}: {}", n + 1, line.trim()));
                }
            }
        }
    }
    assert!(found.is_empty(), "session-wide places in the code (GPU and MangoHud variables go per program):\n{}", found.join("\n"));
}
