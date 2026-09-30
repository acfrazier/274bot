use std::fmt;

use crate::play_status::lock_statuses;
use api::interact::Driver;
use api::snapshot::{GameSnapshot, WorldTile};

use super::Play;

/// One operator interaction queued from a view (the TUI) onto a slot.
/// The slot thread drains the queue in its observe hook through
/// [`api::interact::Interactions`] on its own `Client` — the same wire
/// path the scenario runner and the guardian use, so a queued send
/// respects the same preconditions and lands on the slot's live socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireCmd {
    /// `Interactions::continue_dialog`: press the chat modal's Continue
    /// button. Unsticks NPC dialogue the guardian is not handling.
    Continue,
    /// `Interactions::answer_choice(option)`: press the chat modal's
    /// `option`-th BUTTON_OK choice (1-based).
    Answer(i32),
    /// `Interactions::walk` to an adjacent world tile (WASD one-step, a
    /// direct `try_move` — not a routed walk arm).
    Walk { x: i32, z: i32, level: i32 },
}

/// Run the queued [`WireCmd`]s through `Interactions` on the slot's own
/// Driver. `hold` freezes WASD walks (the guardian's hold freezes the
/// follow too); chat sends still go out so the operator can unstick a
/// dialog the guardian is not talking through.
pub(super) fn dispatch_wires(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    cmds: Vec<WireCmd>,
    hold: bool,
) {
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    for cmd in cmds {
        match cmd {
            WireCmd::Continue => {
                ix.continue_dialog();
            }
            WireCmd::Answer(option) => {
                ix.answer_choice(option);
            }
            WireCmd::Walk { x, z, level } => {
                if !hold {
                    ix.walk(WorldTile { x, z, level });
                }
            }
        }
    }
}

/// Why a queued debug command was refused before it reached the slot queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheatRefusal {
    /// The bound profile or revision does not permit host bot operations.
    Unauthorized,
    /// The command is empty, non-ASCII, contains controls, or exceeds 80
    /// bytes.
    InvalidBody,
    /// No running slot is registered for the requested username.
    UnknownBot,
    /// The slot exists but has not reached an in-game state.
    NotInGame,
    /// The slot status exists but its command queue has already been removed.
    QueueUnavailable,
}

impl fmt::Display for CheatRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Unauthorized => "requires an authorized Local loopback profile",
            Self::InvalidBody => {
                "body must be non-empty, ASCII, control-free, and at most 80 bytes"
            }
            Self::UnknownBot => "bot is not running",
            Self::NotInGame => "bot is not in game",
            Self::QueueUnavailable => "bot command queue is unavailable",
        };
        f.write_str(message)
    }
}

impl Play {
    /// A private queue marker for host-owned cheats. `Play::cheat` rejects
    /// control bytes, so this cannot collide with an operator command.
    pub(crate) const INTERNAL_CHEAT_PREFIX: char = '\u{1}';

    /// Queue `cmd` for a running local-profile slot, returning the admission
    /// reason when it cannot be queued. The slot's client performs the final
    /// admission at the encoder.
    pub fn cheat(&self, user: &str, cmd: &str) -> Result<(), CheatRefusal> {
        self.queue_cheat(user, cmd, true)
    }

    /// Queue a host-owned cheat without making it a Debug-tab reply probe.
    /// This is used for login/session bookkeeping, not operator commands.
    pub fn cheat_internal(&self, user: &str, cmd: &str) -> Result<(), CheatRefusal> {
        self.queue_cheat(user, cmd, false)
    }

    fn queue_cheat(
        &self,
        user: &str,
        cmd: &str,
        observe_replies: bool,
    ) -> Result<(), CheatRefusal> {
        if self.connection.require_bot_operation().is_err() || !self.map_teleport_authorized() {
            return Err(CheatRefusal::Unauthorized);
        }
        if cmd.is_empty()
            || cmd.len() > 80
            || !cmd.is_ascii()
            || cmd.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(CheatRefusal::InvalidBody);
        }
        let statuses = lock_statuses(&self.statuses);
        let known = statuses.iter().any(|status| status.username == user);
        if !known {
            return Err(CheatRefusal::UnknownBot);
        }
        if !statuses
            .iter()
            .any(|status| status.username == user && status.ingame)
        {
            return Err(CheatRefusal::NotInGame);
        }
        let queued = if observe_replies {
            cmd.to_string()
        } else {
            let mut queued =
                String::with_capacity(cmd.len() + Self::INTERNAL_CHEAT_PREFIX.len_utf8());
            queued.push(Self::INTERNAL_CHEAT_PREFIX);
            queued.push_str(cmd);
            queued
        };
        if let Some(q) = self.cheats.lock().unwrap().get_mut(user) {
            q.push_back(queued);
        } else {
            return Err(CheatRefusal::QueueUnavailable);
        }
        drop(statuses);
        self.wake(user);
        Ok(())
    }

    /// Queue a chat or one-tile movement command for a connected slot.
    /// Hold the published-session lock through enqueue so a disconnect reset
    /// cannot clear the queue and then receive an old producer's command.
    pub fn queue_wire(&self, user: &str, cmd: WireCmd) {
        if self.connection.require_bot_operation().is_err() {
            return;
        }
        let statuses = lock_statuses(&self.statuses);
        if !statuses
            .iter()
            .any(|status| status.username == user && status.ingame)
        {
            return;
        }
        if let Some(q) = self.wires.lock().unwrap().get_mut(user) {
            q.push_back(cmd);
        }
        drop(statuses);
        self.wake(user);
    }
}
