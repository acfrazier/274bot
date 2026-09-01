//! Random-event data types shared across the crate boundary: `host`
//! detects and guards random events, `script` answers the `on_random`
//! knock, and `host-play`/`panel`/`tui` bind the status row. The
//! detect/act machine stays in `host`; this module only carries the
//! cross-crate contracts (guardian spec `2026-09-01-random-event-guardian-design.md`).

/// The kind of random event one detection found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RandomKind {
    Dialog,
    Pick,
    Evade,
    Maze,
    Mime,
    Box,
    Lamp,
    Hazard,
    LostTool,
    LostGear,
}

/// One detected random event on the current snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedRandom {
    pub kind: RandomKind,
    pub name: String,
    pub ours: bool,
    pub npc_index: Option<usize>,
}

/// Who handles a detected random event. `Host` (the default) lets the
/// host guardian act and hold; `Handle` means the running script owns the
/// event — ticks and follow keep running and the host does not act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RandomClaim {
    Host,
    Handle,
}
