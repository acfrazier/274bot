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
            until: Instant::now() + WINDOW,
        });
    }

    pub(crate) fn observe(&mut self, name: &str, snapshot: &GameSnapshot) {
        if self.pending.is_empty() {
            return;
        }
        if !snapshot.ingame() {
            self.pending.clear();
            self.sequence = None;
            return;
        }
        let now = Instant::now();
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
        for line in lines[..fresh].iter().rev().filter(|line| line.type_ == 0) {
            for pending in &self.pending {
                api::host_log!(
                    Category::Lifecycle,
                    Level::Info,
                    slot = name,
                    "debug ::{}: chat reply candidate: {}",
                    pending.command,
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
            for text in snapshot
                .main_modal_texts()
                .iter()
                .chain(snapshot.chat_modal_texts())
            {
                if !text.is_empty() {
                    for pending in &self.pending {
                        api::host_log!(
                            Category::Lifecycle,
                            Level::Info,
                            slot = name,
                            "debug ::{}: modal reply candidate: {}",
                            pending.command,
                            text
                        );
                    }
                }
            }
            self.modal = modal;
        }
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
