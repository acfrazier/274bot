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

impl Play {
    /// Queue `cmd` (the `::` part only) for `user`'s slot: its own thread
    /// writes `CLIENT_CHEAT` through the slot's Driver and flushes. No-op
    /// when the user is not a running slot, or when the target is Prod.
    pub fn cheat(&self, user: &str, cmd: &str) {
        if self.connection.require_bot_operation().is_err()
            || !api::interact::cheat_allowed(self.connection.target())
        {
            return;
        }
        let statuses = self.statuses.lock().unwrap();
        if !statuses
            .iter()
            .any(|status| status.username == user && status.ingame)
        {
            return;
        }
        if let Some(q) = self.cheats.lock().unwrap().get_mut(user) {
            q.push_back(cmd.to_string());
        }
        drop(statuses);
        self.wake(user);
    }

    /// Queue a chat or one-tile movement command for a connected slot.
    /// Hold the published-session lock through enqueue so a disconnect reset
    /// cannot clear the queue and then receive an old producer's command.
    pub fn queue_wire(&self, user: &str, cmd: WireCmd) {
        if self.connection.require_bot_operation().is_err() {
            return;
        }
        let statuses = self.statuses.lock().unwrap();
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
