//! A drop-in for Feral's GameMode D-Bus interface.
//!
//! Steam (via `gamemoderun`), Lutris, Heroic, gamescope and many games ship
//! `libgamemode`, which calls `com.feralinteractive.GameMode` on the session
//! bus when a game starts. Answering on that name means those launchers tell
//! us about games directly, with no guessing — and Unix Guardian's game mode
//! runs where the user already asked for GameMode.
//!
//! If Feral's own `gamemoded` is running it owns the name and we step aside;
//! detection still works from `/proc`, and the UI suggests disabling one of
//! the two so they do not both tune the machine.

use crate::iface::{lock, Shared};

/// Return values libgamemode expects.
const OK: i32 = 0;
const INACTIVE: i32 = 0;
const ACTIVE: i32 = 1;
const ACTIVE_AND_REGISTERED: i32 = 2;
const ERROR: i32 = -1;

pub struct GameModeCompat {
    pub core: Shared,
}

impl GameModeCompat {
    fn status(&self, pid: i32) -> i32 {
        let core = lock(&self.core);
        if core.is_running_pid(pid) {
            ACTIVE_AND_REGISTERED
        } else if !core.running().is_empty() {
            ACTIVE
        } else {
            INACTIVE
        }
    }

    fn register(&self, pid: i32) -> i32 {
        if pid <= 0 {
            return ERROR;
        }
        lock(&self.core).register(pid);
        OK
    }
}

#[zbus::interface(name = "com.feralinteractive.GameMode")]
impl GameModeCompat {
    async fn register_game(&self, pid: i32) -> i32 {
        self.register(pid)
    }

    /// Games end when their processes exit; detection sees that on its own, so
    /// an explicit unregister needs no action. Accepting it keeps clients happy.
    async fn unregister_game(&self, pid: i32) -> i32 {
        if pid <= 0 { ERROR } else { OK }
    }

    async fn query_status(&self, pid: i32) -> i32 {
        self.status(pid)
    }

    #[zbus(name = "RegisterGameByPID")]
    async fn register_game_by_pid(&self, _caller: i32, pid: i32) -> i32 {
        self.register(pid)
    }

    #[zbus(name = "UnregisterGameByPID")]
    async fn unregister_game_by_pid(&self, _caller: i32, pid: i32) -> i32 {
        if pid <= 0 { ERROR } else { OK }
    }

    #[zbus(name = "QueryStatusByPID")]
    async fn query_status_by_pid(&self, _caller: i32, pid: i32) -> i32 {
        self.status(pid)
    }

    async fn refresh_config(&self) -> i32 {
        OK
    }

    #[zbus(property)]
    async fn client_count(&self) -> i32 {
        lock(&self.core).running().len() as i32
    }
}
