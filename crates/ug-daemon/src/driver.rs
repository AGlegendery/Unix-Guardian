//! `org.unixguardian.Daemon1.Driver`: the driver troubleshooter's changes.
//!
//! Diagnosis runs in the caller (it needs no root); the daemon only plans,
//! applies and undoes fixes, each behind polkit with a password every time.
//! A development daemon on the real system never changes drivers; with
//! `--sysroot` it changes the scratch tree and records the programs it
//! would run.

use crate::polkit::{action, Gate};
use std::sync::Arc;
use ug_driver::apply::Executor;
use ug_hw::SysRoot;
use ug_proto::{DriverOp, DriverPlan};

pub struct DriverIface {
    pub root: SysRoot,
    pub exec: Arc<Executor>,
    pub gate: Gate,
    /// Why changes are refused here (development on the real system).
    pub refused: Option<String>,
}

impl DriverIface {
    fn plan_now(&self, fix_id: &str) -> DriverPlan {
        let facts = ug_driver::inventory::facts(&self.root, &Default::default());
        let mut p = ug_driver::fixes::plan(fix_id, &self.root, &facts);
        if let (Some(r), true) = (&self.refused, p.refused.is_empty()) {
            p.refused = r.clone();
        }
        p
    }
}

#[zbus::interface(name = "org.unixguardian.Daemon1.Driver")]
impl DriverIface {
    async fn plan(&self, fix_id: &str) -> DriverPlan {
        self.plan_now(fix_id)
    }

    async fn apply(&self, #[zbus(header)] hdr: zbus::message::Header<'_>, fix_id: &str) -> zbus::fdo::Result<String> {
        if let Some(r) = &self.refused {
            return Err(zbus::fdo::Error::NotSupported(r.clone()));
        }
        let facts = ug_driver::inventory::facts(&self.root, &Default::default());
        let fix = ug_driver::fixes::auto(fix_id, &self.root, &facts).map_err(zbus::fdo::Error::NotSupported)?;
        self.gate.authorize(&hdr, action::DRIVER).await?;
        let id = self.exec.start(&fix).map_err(zbus::fdo::Error::Failed)?;
        let (exec, op) = (Arc::clone(&self.exec), id.clone());
        tokio::task::spawn_blocking(move || exec.run(&op, &fix));
        Ok(id)
    }

    async fn ops(&self) -> Vec<DriverOp> {
        self.exec.ops()
    }

    /// An empty id undoes the newest change still in place.
    async fn rollback(&self, #[zbus(header)] hdr: zbus::message::Header<'_>, op_id: &str) -> zbus::fdo::Result<()> {
        if let Some(r) = &self.refused {
            return Err(zbus::fdo::Error::NotSupported(r.clone()));
        }
        self.gate.authorize(&hdr, action::DRIVER).await?;
        let exec = Arc::clone(&self.exec);
        let id = (!op_id.is_empty()).then(|| op_id.to_string());
        tokio::task::spawn_blocking(move || exec.rollback(id.as_deref()))
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map(|_| ())
            .map_err(zbus::fdo::Error::Failed)
    }
}
