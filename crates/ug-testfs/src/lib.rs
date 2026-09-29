//! Synthetic `/sys` and `/proc` trees.
//!
//! Every hardware-reading crate takes a root path instead of hard-coding `/sys`,
//! which is what makes this possible: a test points the reader at a temporary
//! directory laid out like a real machine and asserts on the parsed result. That
//! keeps the hardware layer testable in CI, on a laptop with the wrong GPU, and
//! on a contributor's machine that has neither.
//!
//! Fixtures are captured from real machines with `scripts/capture-fixture.sh` and
//! replayed here, so the parsers are exercised against the messy shapes hardware
//! actually produces rather than shapes we imagined.
//!
//! ```
//! # use ug_testfs::TestFs;
//! let fs = TestFs::new();
//! fs.hwmon(0, "coretemp").temp(1, "Package id 0", 47_000);
//! // `fs.root()` now looks enough like /sys for a reader to work on.
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Path to a fixture captured from a real machine with
/// `scripts/capture-fixture.sh`, for use as a read-only root.
///
/// | Fixture               | Machine                                                  |
/// |-----------------------|----------------------------------------------------------|
/// | `dev-laptop-rtx3050`  | i5-12450HX, Intel UHD (i915) + RTX 3050 Laptop (nvidia 550), Parrot 7.3 |
///
/// Panics with the list of available fixtures if `name` is unknown, so a typo
/// fails loudly rather than as a confusing "no hardware found".
pub fn fixture(name: &str) -> PathBuf {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let p = base.join(name);
    if !p.is_dir() {
        let available: Vec<String> = fs::read_dir(&base)
            .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        panic!("no fixture named {name:?}; available: {available:?}");
    }
    p
}

/// A temporary directory shaped like `/sys` + `/proc`, deleted on drop.
pub struct TestFs {
    dir: TempDir,
}

impl Default for TestFs {
    fn default() -> Self {
        Self::new()
    }
}

impl TestFs {
    /// Creates an empty tree. Add only the nodes a given test needs — a reader
    /// that copes with a machine missing `power1_input` should be tested against
    /// a tree that is actually missing it.
    pub fn new() -> Self {
        TestFs {
            dir: TempDir::new().expect("create tempdir"),
        }
    }

    /// The path to pass to a reader in place of `/`.
    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    /// Writes a file, creating parent directories. Trailing newline included,
    /// because sysfs has one and parsers must tolerate it.
    pub fn write(&self, rel: impl AsRef<Path>, contents: &str) -> &Self {
        let p = self.dir.path().join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(&p, format!("{contents}\n")).expect("write fixture file");
        self
    }

    /// Creates a directory, including parents.
    pub fn dir(&self, rel: impl AsRef<Path>) -> &Self {
        fs::create_dir_all(self.dir.path().join(rel)).expect("create dir");
        self
    }

    /// Starts a hwmon chip at `/sys/class/hwmon/hwmon{index}` with the given
    /// `name` file, and returns a builder for its sensors.
    pub fn hwmon(&self, index: u32, name: &str) -> Hwmon<'_> {
        let rel = PathBuf::from(format!("sys/class/hwmon/hwmon{index}"));
        self.write(rel.join("name"), name);
        Hwmon { fs: self, rel }
    }

    /// Adds a logical CPU with a cpufreq policy.
    ///
    /// `cur_khz` mirrors `scaling_cur_freq`, which is in kHz — the unit mismatch
    /// against every other frequency in the codebase is a real sysfs quirk and
    /// tests should keep hitting it.
    pub fn cpu(&self, n: u32, governor: &str, driver: &str, cur_khz: u64) -> &Self {
        let base = format!("sys/devices/system/cpu/cpu{n}/cpufreq");
        self.write(format!("{base}/scaling_governor"), governor);
        self.write(format!("{base}/scaling_driver"), driver);
        self.write(format!("{base}/scaling_cur_freq"), &cur_khz.to_string());
        self
    }

    /// Gives CPU `n` an L3 cache entry, which is how L3 domains (and therefore
    /// AMD CCDs) are discovered.
    pub fn cpu_l3(&self, n: u32, id: u32, size: &str, shared_list: &str) -> &Self {
        let base = format!("sys/devices/system/cpu/cpu{n}/cache/index3");
        self.write(format!("{base}/level"), "3");
        self.write(format!("{base}/type"), "Unified");
        self.write(format!("{base}/size"), size);
        self.write(format!("{base}/id"), &id.to_string());
        self.write(format!("{base}/shared_cpu_list"), shared_list);
        self
    }

    /// Adds a DRM card node with its PCI identity.
    pub fn drm_card(&self, index: u32, pci: &str, vendor: u16, device: u16, boot_vga: bool) -> &Self {
        let base = format!("sys/class/drm/card{index}/device");
        self.write(format!("{base}/vendor"), &format!("0x{vendor:04x}"));
        self.write(format!("{base}/device"), &format!("0x{device:04x}"));
        self.write(format!("{base}/boot_vga"), if boot_vga { "1" } else { "0" });
        self.write(format!("{base}/uevent"), &format!("PCI_SLOT_NAME={pci}"));
        self
    }

    /// Writes `/proc/stat` with one aggregate line and `cores` per-CPU lines.
    ///
    /// Fields are the seven the reader uses; a real kernel writes more, and the
    /// parser must not depend on the count, so tests that care should write a
    /// longer line by hand.
    pub fn proc_stat(&self, cores: u32, user: u64, idle: u64) -> &Self {
        let mut s = format!("cpu  {user} 0 0 {idle} 0 0 0 0 0 0\n");
        for i in 0..cores {
            s.push_str(&format!("cpu{i} {user} 0 0 {idle} 0 0 0 0 0 0\n"));
        }
        s.push_str("intr 0\nctxt 0\n");
        let p = self.dir.path().join("proc/stat");
        fs::create_dir_all(p.parent().unwrap()).expect("create /proc");
        fs::write(p, s).expect("write /proc/stat");
        self
    }

    /// Writes a `/proc/meminfo` with the four fields the memory reader needs.
    pub fn proc_meminfo(&self, total_kb: u64, available_kb: u64, swap_total_kb: u64, swap_free_kb: u64) -> &Self {
        self.write(
            "proc/meminfo",
            &format!(
                "MemTotal:       {total_kb} kB\n\
                 MemFree:        {available_kb} kB\n\
                 MemAvailable:   {available_kb} kB\n\
                 SwapTotal:      {swap_total_kb} kB\n\
                 SwapFree:       {swap_free_kb} kB"
            ),
        )
    }
}

/// Builder for one hwmon chip's sensor files.
pub struct Hwmon<'a> {
    fs: &'a TestFs,
    rel: PathBuf,
}

impl Hwmon<'_> {
    /// Adds `tempN_label` and `tempN_input`. Input is millidegrees, as in sysfs.
    pub fn temp(self, n: u32, label: &str, millidegrees: i64) -> Self {
        self.fs.write(self.rel.join(format!("temp{n}_label")), label);
        self.fs
            .write(self.rel.join(format!("temp{n}_input")), &millidegrees.to_string());
        self
    }

    /// Adds `tempN_input` with no label, which is how many chips report.
    pub fn temp_unlabelled(self, n: u32, millidegrees: i64) -> Self {
        self.fs
            .write(self.rel.join(format!("temp{n}_input")), &millidegrees.to_string());
        self
    }

    /// Adds `fanN_input`, in RPM.
    pub fn fan(self, n: u32, rpm: u32) -> Self {
        self.fs.write(self.rel.join(format!("fan{n}_input")), &rpm.to_string());
        self
    }

    /// Adds `powerN_average`, in microwatts.
    pub fn power(self, n: u32, microwatts: u64) -> Self {
        self.fs
            .write(self.rel.join(format!("power{n}_average")), &microwatts.to_string());
        self
    }

    /// Adds an arbitrary file to this chip, for shapes the helpers do not cover.
    pub fn file(self, name: &str, contents: &str) -> Self {
        self.fs.write(self.rel.join(name), contents);
        self
    }
}
