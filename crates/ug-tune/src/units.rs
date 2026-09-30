//! Starting and stopping systemd units, behind a trait so the tuner can be
//! tested without systemd.

use std::cell::RefCell;
use std::collections::BTreeSet;

pub trait Units {
    fn is_active(&self, unit: &str) -> bool;
    fn stop(&self, unit: &str) -> Result<(), String>;
    fn start(&self, unit: &str) -> Result<(), String>;
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
}

impl FakeUnits {
    pub fn with(active: &[&str]) -> Self {
        FakeUnits { active: RefCell::new(active.iter().map(|s| s.to_string()).collect()) }
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
}
