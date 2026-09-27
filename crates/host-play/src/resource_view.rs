//! Live-slot facts the operator session's resource meter samples
//! (`frontend-core` owns the meter itself), the background-bot count, and
//! the shared `panel-ui.json` preferences store.

use std::io::{self, ErrorKind};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// `~/.274bot/panel-ui.json` — the shared panel/TUI prefs store.
pub fn panel_ui_path() -> PathBuf {
    script::bot_file("panel-ui.json")
}

pub fn background_bots_acked() -> bool {
    panel_ui_value(BACKGROUND_BOTS_ACK_KEY)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// One top-level value of `panel-ui.json`; `None` when the file, the key or
/// a readable JSON object is absent.
pub fn panel_ui_value(key: &str) -> Option<serde_json::Value> {
    let data = std::fs::read(panel_ui_path()).ok()?;
    let mut value: serde_json::Value = serde_json::from_slice(&data).ok()?;
    value.as_object_mut()?.remove(key)
}

pub fn background_bots_ack_error(err: &io::Error) -> String {
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

/// Set `background_bots_ack` in `panel-ui.json`, preserving other keys.
pub fn persist_background_bots_ack() -> io::Result<()> {
    persist_panel_ui_value(BACKGROUND_BOTS_ACK_KEY, serde_json::Value::Bool(true))
}

/// Set one top-level key of `panel-ui.json`, preserving every other key.
pub fn persist_panel_ui_value(key: &str, value: serde_json::Value) -> io::Result<()> {
    let path = panel_ui_path();
    let mut document = match std::fs::read(&path) {
        Ok(data) => match serde_json::from_slice(&data) {
            Ok(document) => document,
            Err(error) => {
                let stamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let backup = path.with_file_name(format!("panel-ui.json.corrupt-{stamp}"));
                eprintln!(
                    "host-play: refusing to clobber invalid {}; moving it to {}",
                    path.display(),
                    backup.display()
                );
                if backup.exists() {
                    eprintln!(
                        "host-play: refusing to overwrite existing corrupt backup {}",
                        backup.display()
                    );
                    return Err(io::Error::new(
                        ErrorKind::AlreadyExists,
                        format!(
                            "corrupt panel-ui backup already exists: {}",
                            backup.display()
                        ),
                    ));
                }
                if let Err(rename_error) = std::fs::rename(&path, &backup) {
                    eprintln!(
                        "host-play: could not preserve invalid {} as {}: {rename_error}",
                        path.display(),
                        backup.display()
                    );
                    return Err(io::Error::new(
                        ErrorKind::InvalidData,
                        format!("invalid panel-ui.json ({error}); backup failed: {rename_error}"),
                    ));
                }
                serde_json::json!({})
            }
        },
        Err(e) if e.kind() == ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(e),
    };
    let obj = document
        .as_object_mut()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "panel-ui.json is not an object"))?;
    obj.insert(key.into(), value);
    let data = serde_json::to_vec_pretty(&document)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    vault::write_private_file(&path, &data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{run_with_io, PlayOptions, SlotArm, SlotStatus, WorkerTerminal};

    fn empty_play() -> Play {
        run_with_io(
            &PlayOptions {
                host: "127.0.0.1".into(),
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

    #[test]
    fn persist_merges_ack_into_panel_ui_json() {
        let _iso = script::IsolatedEnv::enter("ack-json");
        let path = panel_ui_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"last_focus":"alice"}"#).unwrap();
        assert!(!background_bots_acked());
        persist_background_bots_ack().unwrap();
        assert!(background_bots_acked());
        persist_background_bots_ack().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["last_focus"], "alice");
        assert_eq!(v["background_bots_ack"], true);
    }

    #[test]
    fn persist_moves_corrupt_panel_ui_before_writing() {
        let _iso = script::IsolatedEnv::enter("ack-corrupt");
        let path = panel_ui_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let corrupt = b"{not valid json";
        std::fs::write(&path, corrupt).unwrap();

        persist_background_bots_ack().unwrap();

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
        assert!(backups.next().is_none(), "one backup per corrupt write");
        assert_eq!(std::fs::read(backup.path()).unwrap(), corrupt);
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(written["background_bots_ack"], true);
    }

    #[test]
    fn persist_fails_when_parent_is_a_file() {
        let _iso = script::IsolatedEnv::enter("ack-parent-file");
        let parent = panel_ui_path().parent().unwrap().to_path_buf();
        std::fs::write(&parent, b"not-a-dir").unwrap();
        assert!(persist_background_bots_ack().is_err());
        assert!(!background_bots_acked());
    }

    #[test]
    fn persist_fails_when_panel_ui_path_is_a_directory() {
        let _iso = script::IsolatedEnv::enter("ack-path-dir");
        let path = panel_ui_path();
        std::fs::create_dir_all(&path).unwrap();
        assert!(!background_bots_acked());
        assert!(persist_background_bots_ack().is_err());
        assert!(!background_bots_acked());
    }
}
