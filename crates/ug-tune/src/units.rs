//! Starting and stopping systemd units, and Wi-Fi power saving, behind a
//! trait so the tuner can be tested without systemd or a radio.

use std::cell::RefCell;
use std::collections::BTreeSet;

pub trait Units {
    fn is_active(&self, unit: &str) -> bool;
    fn stop(&self, unit: &str) -> Result<(), String>;
    fn start(&self, unit: &str) -> Result<(), String>;
    /// Whether power saving is on for a Wi-Fi interface; `None` when it
    /// cannot be read (no `iw`, a driver without the setting).
    fn wifi_power_save(&self, _iface: &str) -> Option<bool> {
        None
    }
    fn set_wifi_power_save(&self, iface: &str, _on: bool) -> Result<(), String> {
        Err(format!("cannot change power saving on {iface}"))
    }
}

/// A machine without systemd: nothing is ever active, so nothing is paused.
pub struct NoUnits;

impl Units for NoUnits {
    fn is_active(&self, _: &str) -> bool {
        false
    }
    fn stop(&self, u: &str) -> Result<(), String> {
        Err(format!("no service manager to stop {u}"))
    }
    fn start(&self, u: &str) -> Result<(), String> {
        Err(format!("no service manager to start {u}"))
    }
}

/// An in-memory service manager for tests.
#[derive(Default)]
pub struct FakeUnits {
    pub active: RefCell<BTreeSet<String>>,
    /// Wi-Fi interfaces with power saving on.
    pub wifi_saving: RefCell<BTreeSet<String>>,
}

impl FakeUnits {
    pub fn with(active: &[&str]) -> Self {
        FakeUnits { active: RefCell::new(active.iter().map(|s| s.to_string()).collect()), ..Default::default() }
    }
}

impl Units for FakeUnits {
    fn is_active(&self, unit: &str) -> bool {
        self.active.borrow().contains(unit)
    }
    fn stop(&self, unit: &str) -> Result<(), String> {
        self.active.borrow_mut().remove(unit);
        Ok(())
    }
    fn start(&self, unit: &str) -> Result<(), String> {
        self.active.borrow_mut().insert(unit.to_string());
        Ok(())
    }
    fn wifi_power_save(&self, iface: &str) -> Option<bool> {
        Some(self.wifi_saving.borrow().contains(iface))
    }
    fn set_wifi_power_save(&self, iface: &str, on: bool) -> Result<(), String> {
        if on {
            self.wifi_saving.borrow_mut().insert(iface.to_string());
        } else {
            self.wifi_saving.borrow_mut().remove(iface);
        }
        Ok(())
    }
}
