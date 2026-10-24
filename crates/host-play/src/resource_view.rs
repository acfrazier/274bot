//! Live-slot facts the operator session's resource meter samples
//! (`frontend-core` owns the meter itself), the background-bot count, and
//! the background-bots notice preference. The shared `panel-ui.json`
//! preferences are the process local-state service's `preferences` domain
//! (T5.2 r3, [`crate::local_state::Preferences`]); nothing here reads or
//! writes the file.

#[cfg(test)]
use std::io;

use crate::local_state::Preferences;
use crate::Play;

/// One live worker lifetime at sample time. Terminal history rows are omitted
/// by the Play snapshot that produces these facts.
#[derive(Debug, Clone, Copy)]
pub struct LiveSlot<'a> {
    pub name: &'a str,
    /// [`crate::SlotArm::lifetime_id`]: a respawn of the same name is a
    /// different lifetime.
    pub lifetime: u64,
    pub ingame: bool,
    /// `bytes_in + bytes_out` of the current stream.
    pub traffic_bytes: u64,
    /// The row's [`crate::SlotStatus::stream_epoch`]: it changes whenever
    /// the byte counters restart.
    pub stream: u32,
}

impl Play {
    /// Visit each worker lifetime that is still owned (`arm` present), in
    /// one pass over the status rows. Logged-out, queued, connecting, and
    /// disconnected workers count; terminal history rows and retired slots
    /// do not. A worker whose row is not published yet has no traffic.
    pub fn for_each_live_slot(&self, mut visit: impl FnMut(LiveSlot<'_>)) {
        let statuses = crate::lock_statuses(&self.statuses);
        let mut visited = 0;
        for row in statuses.iter() {
            if let Some(arm) = self.arms.get(&row.username) {
                visited += 1;
                visit(LiveSlot {
                    name: &row.username,
                    lifetime: arm.lifetime_id(),
                    ingame: row.ingame,
                    traffic_bytes: row.bytes_in.wrapping_add(row.bytes_out),
                    stream: row.stream_epoch,
                });
            }
        }
        if visited == self.arms.len() {
            return;
        }
        // A spawn publishes its row at once, so this scan is for arms
        // attached without a worker (fixtures) only.
        for (name, arm) in &self.arms {
            if !statuses.iter().any(|row| &row.username == name) {
                visit(LiveSlot {
                    name,
                    lifetime: arm.lifetime_id(),
                    ingame: false,
                    traffic_bytes: 0,
                    stream: 0,
                });
            }
        }
    }

    pub fn background_bot_count(&self, focused: Option<&str>) -> usize {
        self.arms
            .keys()
            .filter(|name| focused != Some(name.as_str()))
            .count()
    }
}

/// Live slots that are not the focused profile.
pub fn background_bot_count<I, S>(live: I, focused: Option<&str>) -> usize
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    live.into_iter()
        .filter(|name| focused != Some(name.as_ref()))
        .count()
}

const BACKGROUND_BOTS_ACK_KEY: &str = "background_bots_ack";

/// Whether the operator acknowledged that other profiles keep running.
pub fn background_bots_acked(preferences: &Preferences) -> bool {
    preferences
        .value(BACKGROUND_BOTS_ACK_KEY)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

pub fn background_bots_ack_error(err: &impl std::fmt::Display) -> String {
    format!("background bots: {err}")
}

pub fn clear_background_bots_ack_error(error: &mut Option<String>) {
    if error
        .as_deref()
        .is_some_and(|msg| msg.starts_with("background bots:"))
    {
        *error = None;
    }
}

/// Fixture-only durable acknowledgement. Product callers submit asynchronously.
#[cfg(test)]
pub fn persist_background_bots_ack(preferences: &Preferences) -> io::Result<()> {
    preferences.set(BACKGROUND_BOTS_ACK_KEY, serde_json::Value::Bool(true))
}

/// Submit the acknowledgement without waiting for storage.
pub fn begin_persist_background_bots_ack(
    preferences: &Preferences,
) -> Result<crate::local_state::Update, crate::local_state::PreferenceError> {
    preferences.begin_set(BACKGROUND_BOTS_ACK_KEY, serde_json::Value::Bool(true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{run_with_io, PlayOptions, SlotArm, SlotStatus, WorkerTerminal};

    fn empty_play() -> Play {
        run_with_io(
            &PlayOptions {
                host: "127.0.0.1".into(),
                transport: client::Transport::Tcp,
                port: 43594,
                cache_dir: "/tmp".into(),
                lowmem: true,
                mainland: false,
            },
            vec![],
            |_| (None, None),
            |_, _, _| {},
        )
    }

    #[test]
    fn background_count_skips_the_focused_name() {
        assert_eq!(background_bot_count(["alice", "bob"], Some("bob")), 1);
        assert_eq!(background_bot_count(["alice"], Some("alice")), 0);
        assert_eq!(background_bot_count(["alice", "bob"], None), 2);
    }

    #[test]
    fn play_live_slots_skip_terminal_and_retired_keep_logged_out() {
        let mut play = empty_play();
        play.attach_arm("alice", SlotArm::new(1, false));
        play.attach_arm("carol", SlotArm::new(3, false));
        play.statuses.lock().unwrap().extend([
            SlotStatus {
                username: "alice".into(),
                ingame: true,
                connected: true,
                ..SlotStatus::default()
            },
            SlotStatus {
                username: "bob".into(),
                ingame: false,
                worker_terminal: Some(WorkerTerminal::Failed),
                ..SlotStatus::default()
            },
            SlotStatus {
                username: "carol".into(),
                ingame: false,
                connected: false,
                login_latched: true,
                ..SlotStatus::default()
            },
            SlotStatus {
                username: "dave".into(),
                ingame: false,
                ..SlotStatus::default()
            },
        ]);
        let mut names = Vec::new();
        let mut ingame = 0;
        play.for_each_live_slot(|slot| {
            names.push(slot.name.to_string());
            if slot.ingame {
                ingame += 1;
            }
        });
        names.sort();
        assert_eq!(names, ["alice", "carol"]);
        assert_eq!(ingame, 1);
        assert_eq!(play.background_bot_count(Some("alice")), 1);
    }

    fn preferences(scratch: &script::IsolatedEnv) -> Preferences {
        crate::local_state::LocalStateService::open_process(crate::local_state::StateRoot::new(
            scratch.home.join(".274bot"),
        ))
        .preferences()
    }

    #[test]
    fn persist_merges_ack_into_panel_ui_json() {
        let scratch = script::IsolatedEnv::enter("ack-json");
        let preferences = preferences(&scratch);
        let path = preferences.path();
        vault::write_private_file(&path, br#"{"last_focus":"alice"}"#).unwrap();
        assert!(!background_bots_acked(&preferences));
        persist_background_bots_ack(&preferences).unwrap();
        assert!(background_bots_acked(&preferences));
        persist_background_bots_ack(&preferences).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["last_focus"], "alice");
        assert_eq!(v["background_bots_ack"], true);
    }

    /// T5.2 r4: a corrupt preferences file is moved aside, byte for byte,
    /// to `panel-ui.json.corrupt-<time>` (the pre-T5.2 recovery, now under
    /// the file's lock and flushed), and the acknowledgement is saved in
    /// fresh preferences.
    #[test]
    fn persist_moves_a_corrupt_panel_ui_aside_before_writing() {
        let scratch = script::IsolatedEnv::enter("ack-corrupt");
        let preferences = preferences(&scratch);
        let path = preferences.path();
        let corrupt = b"{not valid json";
        vault::write_private_file(&path, corrupt).unwrap();

        persist_background_bots_ack(&preferences).unwrap();

        assert!(background_bots_acked(&preferences));
        let mut backups = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("panel-ui.json.corrupt-")
            });
        let backup = backups.next().expect("corrupt panel-ui backup");
        assert!(backups.next().is_none(), "one backup");
        assert_eq!(std::fs::read(backup.path()).unwrap(), corrupt);
    }

    #[test]
    fn persist_fails_when_parent_is_a_file() {
        let scratch = script::IsolatedEnv::enter("ack-parent-file");
        let preferences = preferences(&scratch);
        let parent = preferences.path().parent().unwrap().to_path_buf();
        std::fs::write(&parent, b"not-a-dir").unwrap();
        assert!(persist_background_bots_ack(&preferences).is_err());
        assert!(!background_bots_acked(&preferences));
    }

    #[test]
    fn persist_fails_when_panel_ui_path_is_a_directory() {
        let scratch = script::IsolatedEnv::enter("ack-path-dir");
        let preferences = preferences(&scratch);
        std::fs::create_dir_all(preferences.path()).unwrap();
        assert!(!background_bots_acked(&preferences));
        assert!(persist_background_bots_ack(&preferences).is_err());
        assert!(!background_bots_acked(&preferences));
    }
}
