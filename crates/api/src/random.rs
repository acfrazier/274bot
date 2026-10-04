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

/// The Maze random's map square (`[mapzone,0_45_71]`,
/// `[random_event_maze_zones]`), as `(x >> 6, z >> 6)` at level 0.
pub const MAZE_SQUARE: (i32, i32) = (45, 71);
/// The Mime random's stage square (`[mapzone,0_31_74]`), level 0.
pub const MIME_SQUARE: (i32, i32) = (31, 74);

/// The random event whose content map square traps a player standing on
/// world tile `(x, z, level)`: the Maze or the Mime stage. Only the event
/// itself leads off these squares, so anything but its solver is stuck
/// there.
pub fn trapped_area(x: i32, z: i32, level: i32) -> Option<RandomKind> {
    if level != 0 {
        return None;
    }
    match (x >> 6, z >> 6) {
        MAZE_SQUARE => Some(RandomKind::Maze),
        MIME_SQUARE => Some(RandomKind::Mime),
        _ => None,
    }
}
