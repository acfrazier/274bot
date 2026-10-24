//! Per-slot recovery hints retained across surface restarts.
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HintTile {
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

#[derive(Default, Debug)]
struct Hints {
    pending: bool,
    anchor: Option<HintTile>,
}

/// The slot's recovery hints, shared with each isolate it spawns.
#[derive(Default, Debug)]
pub struct RecoveryHintsCell {
    hints: Mutex<Hints>,
}

impl RecoveryHintsCell {
    pub fn new() -> Self {
        Self::default()
    }

    fn with<R>(&self, f: impl FnOnce(&mut Hints) -> R) -> R {
        f(&mut self
            .hints
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()))
    }

    /// A watchdog restart is recreating the script: frozen StallGuard's
    /// `RecoveryHints.pendingRecovery = true` (`StallGuard.ts:37`).
    pub fn note_restart(&self) {
        self.with(|hints| hints.pending = true);
    }

    /// Frozen `takeAnchor()` (`RecoveryHints.ts:7–13`).
    pub fn take_anchor(&self) -> Option<HintTile> {
        self.with(|hints| {
            if !hints.pending {
                return None;
            }
            hints.pending = false;
            hints.anchor
        })
    }

    pub fn anchor(&self) -> Option<HintTile> {
        self.with(|hints| hints.anchor)
    }

    pub fn set_anchor(&self, anchor: Option<HintTile>) {
        self.with(|hints| hints.anchor = anchor);
    }

    pub fn pending(&self) -> bool {
        self.with(|hints| hints.pending)
    }

    pub fn set_pending(&self, pending: bool) {
        self.with(|hints| hints.pending = pending);
    }

    /// Frozen `clear()` (`RecoveryHints.ts:15–18`).
    pub fn clear(&self) {
        self.with(|hints| *hints = Hints::default());
    }

    /// Frozen `ScriptRunner.ts:221–223`, after a successful `onStart`.
    pub fn startup_complete(&self) {
        self.with(|hints| {
            if hints.pending {
                *hints = Hints::default();
            }
        });
    }
}
