//! Bounded observation of replies following host-issued debug commands.
//! CLIENT_CHEAT has no request id or acknowledgement: these are explicitly
//! temporal reply candidates, never claims that a command succeeded.
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use api::hostlog::{Category, Level};
use api::snapshot::GameSnapshot;

const WINDOW: Duration = Duration::from_secs(15);
const MAX_PENDING: usize = 32;

struct Pending {
    command: String,
    until: Instant,
}

#[derive(Default)]
pub(crate) struct DebugReplies {
    pending: VecDeque<Pending>,
    sequence: Option<i32>,
    iface_generation: u64,
    modal: u64,
}

impl DebugReplies {
    pub(crate) fn sent(&mut self, name: &str, command: String, snapshot: &GameSnapshot) {
        self.sent_at(name, command, snapshot, Instant::now());
    }

    fn sent_at(&mut self, name: &str, command: String, snapshot: &GameSnapshot, now: Instant) {
        if self.pending.is_empty() {
            self.sequence = snapshot.chat_lines().first().map(|line| line.sequence);
            self.modal = modal_fingerprint(snapshot);
            self.iface_generation = snapshot.gens().iface;
        }
        if self.pending.len() == MAX_PENDING {
            if let Some(old) = self.pending.pop_front() {
                api::host_log!(
                    Category::Lifecycle,
                    Level::Info,
                    slot = name,
                    "debug ::{}: reply observation capacity reached; no success inferred",
                    old.command
                );
            }
        }
        api::host_log!(
            Category::Lifecycle,
            Level::Info,
            slot = name,
            "debug ::{command}: sent; observing chat/modal replies (not an acknowledgement)"
        );
        self.pending.push_back(Pending {
            command,
            until: now + WINDOW,
        });
    }

    pub(crate) fn observe(&mut self, name: &str, snapshot: &GameSnapshot) {
        self.observe_at(name, snapshot, Instant::now());
    }

    fn observe_at(&mut self, name: &str, snapshot: &GameSnapshot, now: Instant) {
        if self.pending.is_empty() {
            return;
        }
        if !snapshot.ingame() {
            self.pending.clear();
            self.sequence = None;
            return;
        }
        while self
            .pending
            .front()
            .is_some_and(|pending| pending.until <= now)
        {
            let pending = self.pending.pop_front().expect("checked front");
            api::host_log!(
                Category::Lifecycle,
                Level::Info,
                slot = name,
                "debug ::{}: reply observation ended; command success is not inferred",
                pending.command
            );
        }
        if self.pending.is_empty() {
            return;
        }
        let lines = snapshot.chat_lines();
        let fresh = lines
            .iter()
            .take_while(|line| Some(line.sequence) != self.sequence)
            .count();
        if fresh != 0 {
            let commands = self.pending_command_list();
            for line in lines[..fresh].iter().rev().filter(|line| line.type_ == 0) {
                api::host_log!(
                    Category::Lifecycle,
                    Level::Info,
                    slot = name,
                    "debug ::[{}]: chat reply candidate: {}",
                    commands,
                    line.text
                );
            }
        }
        self.sequence = lines.first().map(|line| line.sequence);
        if snapshot.gens().iface == self.iface_generation {
            return;
        }
        self.iface_generation = snapshot.gens().iface;
        let modal = modal_fingerprint(snapshot);
        if modal != self.modal {
            let commands = self.pending_command_list();
            for text in snapshot
                .main_modal_texts()
                .iter()
                .chain(snapshot.chat_modal_texts())
            {
                if !text.is_empty() {
                    api::host_log!(
                        Category::Lifecycle,
                        Level::Info,
                        slot = name,
                        "debug ::[{}]: modal reply candidate: {}",
                        commands,
                        text
                    );
                }
            }
            self.modal = modal;
        }
    }

    fn pending_command_list(&self) -> String {
        let mut commands = String::new();
        for (index, pending) in self.pending.iter().enumerate() {
            if index != 0 {
                commands.push_str(", ");
            }
            commands.push_str(&pending.command);
        }
        commands
    }
}

fn modal_fingerprint(snapshot: &GameSnapshot) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    snapshot.modals().main.hash(&mut hash);
    snapshot.modals().chat.hash(&mut hash);
    snapshot.main_modal_texts().hash(&mut hash);
    snapshot.chat_modal_texts().hash(&mut hash);
    hash.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn synthetic_reply_snapshot() -> GameSnapshot {
        use client::config::if_type::{ComponentType, IfType, IfTypeMut};

        let mut client = client::client::Client::new(client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: false,
        });
        client.ingame = true;
        client.scene_state = 2;
        client.main_modal_id = 100;
        client.set_iface(
            100,
            IfType {
                id: 100,
                layer_id: 100,
                r#type: ComponentType::TYPE_LAYER,
                children: Some(vec![101]),
                ..Default::default()
            },
        );
        client.set_iface(
            101,
            IfType {
                id: 101,
                layer_id: 100,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        client.set_iface_mut(
            101,
            IfTypeMut {
                text: "modal reply".into(),
                ..Default::default()
            },
        );
        client.add_chat(0, "chat reply", "alice");
        client.bump_gens(client::io::ServerProt::REBUILD_NORMAL);
        client.bump_gens(client::io::ServerProt::REBUILD_NORMAL);
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        assert_eq!(snapshot.chat_lines().len(), 1);
        assert_eq!(snapshot.main_modal_texts(), &["modal reply".to_string()]);
        snapshot
    }

    #[test]
    fn reply_candidates_are_logged_once_with_all_pending_commands() {
        let mark = crate::walk_map::test_log::mark();

        let baseline = synthetic_snapshot(true);
        let reply = synthetic_reply_snapshot();
        let start = Instant::now();
        let mut replies = DebugReplies::default();
        replies.sent_at("alice", "first".into(), &baseline, start);
        replies.sent_at("alice", "second".into(), &baseline, start);
        replies.observe_at("alice", &reply, start + Duration::from_secs(1));

        let records = crate::walk_map::test_log::records_since(mark)
            .into_iter()
            .map(|(_, message)| message)
            .filter(|message| message.contains("reply candidate"))
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 2);
        assert!(records
            .iter()
            .all(|record| record.contains("[first, second]")));
        assert!(records.iter().any(|record| record.contains("chat reply")));
        assert!(records.iter().any(|record| record.contains("modal reply")));
    }

    fn synthetic_snapshot(ingame: bool) -> GameSnapshot {
        if !ingame {
            return GameSnapshot::default();
        }

        let mut client = client::client::Client::new(client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: false,
        });
        client.ingame = true;
        client.scene_state = 2;
        client.bump_gens(client::io::ServerProt::REBUILD_NORMAL);
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        assert!(snapshot.ingame());
        snapshot
    }

    #[test]
    fn capacity_evicts_oldest_pending_command() {
        let snapshot = synthetic_snapshot(true);
        let start = Instant::now();
        let mut replies = DebugReplies::default();

        for index in 0..MAX_PENDING {
            replies.sent_at("alice", format!("command-{index}"), &snapshot, start);
        }
        assert_eq!(replies.pending.len(), MAX_PENDING);
        assert_eq!(replies.pending.front().unwrap().command, "command-0");

        replies.sent_at(
            "alice",
            "command-32".into(),
            &snapshot,
            start + Duration::from_secs(1),
        );
        assert_eq!(replies.pending.len(), MAX_PENDING);
        assert_eq!(replies.pending.front().unwrap().command, "command-1");
        assert_eq!(replies.pending.back().unwrap().command, "command-32");
    }

    #[test]
    fn pending_commands_expire_at_fifteen_seconds() {
        let snapshot = synthetic_snapshot(true);
        let start = Instant::now();
        let mut replies = DebugReplies::default();

        replies.sent_at("alice", "getcoord".into(), &snapshot, start);
        replies.observe_at("alice", &snapshot, start + WINDOW);

        assert!(replies.pending.is_empty());
    }

    #[test]
    fn disconnect_clears_pending_commands() {
        let connected = synthetic_snapshot(true);
        let disconnected = synthetic_snapshot(false);
        let start = Instant::now();
        let mut replies = DebugReplies::default();

        replies.sent_at("alice", "getcoord".into(), &connected, start);
        replies.observe_at("alice", &disconnected, start + Duration::from_secs(1));

        assert!(replies.pending.is_empty());
        assert!(replies.sequence.is_none());
    }

    #[test]
    fn session_reset_starts_with_no_pending_commands() {
        let snapshot = synthetic_snapshot(true);
        let mut replies = DebugReplies::default();
        replies.sent_at("alice", "getcoord".into(), &snapshot, Instant::now());
        assert!(!replies.pending.is_empty());

        replies = DebugReplies::default();

        assert!(replies.pending.is_empty());
        assert!(replies.sequence.is_none());
        assert_eq!(replies.iface_generation, 0);
        assert_eq!(replies.modal, 0);
    }
}
