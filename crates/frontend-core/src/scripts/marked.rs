//! What [`crate::marked`] needs from the coordinator: resolve a selected
//! card to the assignment it saves, and queue a restart behind the stop of
//! the run it replaces.

use std::path::Path;

use vault::ScriptAssignment;

use super::{arm_flags, wall_member, LogTo, Outcome, QueuedStart, Scripts};
use crate::session::OperatorSession;

impl Scripts {
    /// The assignment a selected card saves. A card that cannot start
    /// (missing file, unavailable catalog card, unloadable import) has none.
    pub(crate) fn assignment_for(
        &mut self,
        sel: &script::ScriptSel,
        catalog_root: Option<&Path>,
    ) -> Result<ScriptAssignment, String> {
        match sel {
            script::ScriptSel::Compiled(id) => script::compiled_card(*id)
                .map(|_| script::compiled_assignment(*id))
                .ok_or_else(|| "unavailable card".to_string()),
            script::ScriptSel::Loaded(source, lookup) => {
                if *source == script::ScriptSource::Catalog {
                    self.fill_catalog_once(catalog_root);
                }
                let card = self.js.get(*source, lookup).ok_or_else(|| match source {
                    script::ScriptSource::File => format!("missing file: {lookup}"),
                    _ => format!("unavailable: {lookup}"),
                })?;
                match &card.unloadable {
                    Some(reason) => Err(format!("unloadable import: {reason}")),
                    None => Ok(card.assignment()),
                }
            }
        }
    }

    /// Drop `profile`'s Browse draft: it now names its saved assignment.
    pub(crate) fn clear_pending_browse(&mut self, profile: &str) {
        self.pending_browse.remove(profile);
    }

    /// Queue `sel` to start on `profile` through the paced Start permit, in
    /// the running tally. When the slot's script is still stopping (the
    /// caller asked it to stop) the Start waits until the slot is idle.
    pub(crate) fn queue_restart<Io>(
        &mut self,
        core: &OperatorSession<Io>,
        profile: &str,
        sel: script::ScriptSel,
        stopping: bool,
    ) -> Result<(), String> {
        if core.play().is_none() {
            return Err("no play".into());
        }
        if self.admit.contains(profile) {
            return Ok(());
        }
        let (had_arm, latched) = arm_flags(core, profile);
        let entry = QueuedStart {
            profile: profile.to_string(),
            sel,
            assigned: None,
            latched,
            had_arm,
        };
        if stopping {
            self.admit.hold_until_stopped(entry);
        } else {
            self.admit.enqueue(entry);
        }
        self.tally_record(profile, Outcome::Queued, LogTo::None);
        Ok(())
    }

    /// Move restarts whose slot has stopped onto the paced queue. A member
    /// that was removed meanwhile is released too, so the grant reports it
    /// instead of waiting forever.
    pub(super) fn release_stopped_restarts<Io>(&mut self, core: &OperatorSession<Io>) {
        self.admit.release_stopped(|profile| {
            !wall_member(core, profile)
                || matches!(
                    core.play()
                        .map_or(script::RunState::Idle, |play| play.script_state(profile)),
                    script::RunState::Idle | script::RunState::Error
                )
        });
    }
}
