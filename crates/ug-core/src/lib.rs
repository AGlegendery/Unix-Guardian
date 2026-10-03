//! Configuration, distribution detection and logging setup.

/// Where bug reports go; shown by every interface next to errors.
pub const BUG_REPORT_EMAIL: &str = "unixguardian@atomicmail.io";

pub mod distro;
pub mod gpu_default;
pub mod playtime;
pub mod logging;
pub mod paths;
pub mod profile;
