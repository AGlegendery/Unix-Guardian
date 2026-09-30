//! Internet control for Unix Guardian: which app uses how much, and (next)
//! blocking and limiting apps and keeping ping steady.
//!
//! Apps are told apart by their cgroup, which systemd already assigns, and
//! traffic is counted by nftables in a table of our own.

pub mod account;
pub mod cgroups;
pub mod nft;
