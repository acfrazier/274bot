//! The Path's own NPC conversation, recorded while one of its steps holds it
//! and carried across an operator Stop/Start in the same session.
//!
//! Ownership is recorded, never inferred from the scene at Start. While a
//! `talk` step (or an `acquire` recipe's `talk` child) is mid-conversation with
//! the NPC its own Talk-to opened, the runner keeps a [`HeldConversation`] in
//! the slot's retained memory (`QuesterRetained::conversation`): the Path, the
//! exact step, the NPC and the last chat page that step saw. An operator Stop
//! moves only this record to the slot ([`CarriedConversation::stop`]); every
//! other Stop, a session boundary (logout, relog, world hop) and any other
//! Path drop it.
//!
//! From the Stop on, the slot checks every observed frame against the record
//! ([`HeldConversation::observe_page`]): the conversation may stay on the page
//! the step last saw, or move once to the menu that page leads to, which is
//! then pinned. A closed chat, a page that is neither, a second menu, or a
//! frame out of game ends the record. The restarted runner answers only the
//! pinned menu, with the recorded step's own text answers (`Quester::held_menu`).

use super::families::dialogue::DialogueOptions;
use api::selected::FactKey;
use api::snapshot::{chat_page_fingerprint, ChatOptionView, GameSnapshot};

/// One observed chat page: its root, the fingerprint of its texts and option
/// rows (the driver's own page identity, `chat_page_fingerprint`), and the
/// script tick it was observed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatPage {
    pub root: i32,
    pub fingerprint: u64,
    pub tick: u64,
    /// An open choice: options and no continue.
    pub menu: bool,
}

impl ChatPage {
    /// The open chat page, or `None` when the chat is closed.
    pub fn observe(
        root: i32,
        continue_component_id: i32,
        texts: &[String],
        options: &[ChatOptionView],
        tick: u64,
    ) -> Option<Self> {
        if root == -1 && continue_component_id < 0 {
            return None;
        }
        Some(Self {
            root,
            fingerprint: chat_page_fingerprint(
                texts,
                options
                    .iter()
                    .map(|option| (option.component_id, option.text.as_str())),
            ),
            tick,
            menu: continue_component_id < 0 && !options.is_empty(),
        })
    }

    /// The same page: root and content, whatever its continue latch shows.
    pub fn same(&self, other: &Self) -> bool {
        self.root == other.root && self.fingerprint == other.fingerprint
    }

    /// The same root on the same tick: the page's rows are still arriving under
    /// the client's 5-packets-per-frame read (`PAGE_SETTLE_TICKS`), not a new page.
    fn arriving(&self, other: &Self) -> bool {
        self.root == other.root && self.tick == other.tick
    }
}

/// A conversation one of the Path's steps holds with the NPC it talked to.
pub struct HeldConversation {
    /// The Path and its compiled digest: any other Path never answers.
    pub path: FactKey,
    pub digest: [u8; 32],
    /// The root step, and the recipe child that talks when it is an `acquire`.
    pub step: FactKey,
    pub child: Option<FactKey>,
    pub npc_type: i32,
    pub npc_index: i32,
    /// The step's authored answers, narrowed to page text
    /// ([`DialogueOptions::by_text_only`]).
    pub options: DialogueOptions,
    /// The last page the step saw.
    pub page: ChatPage,
    /// The menu the conversation is waiting on: the step's own page when it
    /// was a menu, or the first menu after it.
    pub menu: Option<ChatPage>,
    /// Set by an operator Stop. A record the live step maintains is not
    /// carried; the runner drops it once that step no longer holds it.
    pub carried: bool,
}

impl HeldConversation {
    /// Whether `step`/`child`/`npc_index` is this record's conversation.
    pub fn holds(&self, step: &FactKey, child: Option<&FactKey>, npc_index: i32) -> bool {
        self.step == *step && self.child.as_ref() == child && self.npc_index == npc_index
    }

    /// The live step saw `page` (`None`: the chat is closed). Keeps the menu
    /// pinned while the step waits on it.
    pub fn saw(&mut self, page: Option<ChatPage>) {
        let Some(page) = page else {
            self.menu = None;
            return;
        };
        self.page = page;
        self.menu = page.menu.then_some(page);
    }

    /// One frame after the Stop: `true` keeps the record. The conversation may
    /// stay on the step's last page, or move once to the menu after it, which
    /// is then pinned. A page whose rows are still arriving on the same root
    /// and tick replaces the page it completes.
    pub fn observe_page(&mut self, page: Option<ChatPage>) -> bool {
        let Some(page) = page else {
            return false;
        };
        if let Some(menu) = self.menu.as_mut() {
            if page.same(menu) {
                return true;
            }
            if page.menu && page.arriving(menu) {
                *menu = page;
                return true;
            }
            return false;
        }
        if page.same(&self.page) {
            return true;
        }
        if page.arriving(&self.page) {
            self.page = page;
            self.menu = page.menu.then_some(page);
            return true;
        }
        if page.menu {
            self.menu = Some(page);
            return true;
        }
        false
    }

    /// [`Self::observe_page`] on a host frame. Out of game ends the record.
    pub fn observe(&mut self, snapshot: Option<&GameSnapshot>, tick: u64) -> bool {
        let Some(snapshot) = snapshot.filter(|snapshot| snapshot.ingame()) else {
            return false;
        };
        self.observe_page(ChatPage::observe(
            snapshot.modals().chat,
            snapshot.chat_continue_component_id(),
            snapshot.chat_modal_texts(),
            snapshot.chat_options(),
            tick,
        ))
    }
}

/// The slot's side of a conversation an operator Stop carried. It lives
/// outside the retained cell so that Stop, a session boundary and the
/// per-frame watch never wait on the cell's lock, which a preparing Start's
/// factory may hold; each compiled tick lends it to the cell and takes it
/// back ([`Self::lend`], [`Self::reclaim`]).
#[derive(Default)]
pub struct CarriedConversation(Option<Box<HeldConversation>>);

impl CarriedConversation {
    /// Operator Stop: the conversation the live step recorded in `cell` is
    /// carried; with none recorded (or the cell busy, `None`), the one already
    /// carried stays.
    pub fn stop(&mut self, cell: Option<&mut super::QuesterRetained>) {
        if let Some(mut held) = cell.and_then(|cell| cell.conversation.take()) {
            held.carried = true;
            self.0 = Some(held);
        }
    }

    /// Any other Stop, or a session boundary: nothing is carried.
    pub fn clear(&mut self) {
        self.0 = None;
    }

    /// One observed frame ([`HeldConversation::observe`]); a frame that ends
    /// the conversation drops it.
    pub fn watch(&mut self, snapshot: Option<&GameSnapshot>, tick: u64) {
        if self
            .0
            .as_mut()
            .is_some_and(|held| !held.observe(snapshot, tick))
        {
            self.0 = None;
        }
    }

    /// Before a compiled tick: the runner reads the carried record from the
    /// cell.
    pub fn lend(&mut self, cell: &mut super::QuesterRetained) {
        if let Some(held) = self.0.take() {
            cell.conversation = Some(held);
        }
    }

    /// After the tick: a carried record the runner neither answered nor
    /// replaced with its live step's comes back to the slot's watch.
    pub fn reclaim(&mut self, cell: &mut super::QuesterRetained) {
        if cell.conversation.as_ref().is_some_and(|held| held.carried) {
            self.0 = cell.conversation.take();
        }
    }

    pub fn get_mut(&mut self) -> Option<&mut HeldConversation> {
        self.0.as_deref_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(root: i32, fingerprint: u64, tick: u64, menu: bool) -> ChatPage {
        ChatPage {
            root,
            fingerprint,
            tick,
            menu,
        }
    }

    fn held(last: ChatPage) -> HeldConversation {
        HeldConversation {
            path: FactKey::new("vampire"),
            digest: [0; 32],
            step: FactKey::new("stake"),
            child: Some(FactKey::new("buy-harlow-beer")),
            npc_type: 1,
            npc_index: 304,
            options: DialogueOptions::default(),
            page: last,
            menu: last.menu.then_some(last),
            carried: true,
        }
    }

    #[test]
    fn the_step_page_then_one_menu_is_kept_and_pinned() {
        let npc = page(4882, 1, 10, false);
        let menu = page(2469, 2, 11, true);
        let mut record = held(npc);
        assert!(record.observe_page(Some(npc)), "the step's own page stays");
        assert!(record.observe_page(Some(menu)), "the menu it leads to");
        assert_eq!(record.menu, Some(menu));
        assert!(record.observe_page(Some(page(2469, 2, 14, true))), "pinned");
    }

    #[test]
    fn anything_else_after_the_stop_ends_the_record() {
        let npc = page(4882, 1, 10, false);
        let menu = page(2469, 2, 11, true);
        for (label, frames) in [
            ("closed chat", vec![None]),
            ("another continue page", vec![Some(page(968, 3, 11, false))]),
            (
                "a second menu",
                vec![Some(menu), Some(page(2459, 4, 12, true))],
            ),
            (
                "the pinned menu replaced on its root a tick later",
                vec![Some(menu), Some(page(2469, 5, 12, true))],
            ),
            ("closed, then the menu", vec![None, Some(menu)]),
        ] {
            let mut record = held(npc);
            // The watch drops the record at the first frame that ends it.
            let kept = frames.into_iter().all(|frame| record.observe_page(frame));
            assert!(!kept, "{label}");
        }
    }

    #[test]
    fn rows_still_arriving_on_the_same_tick_complete_the_page() {
        let partial = page(2469, 7, 11, true);
        let complete = page(2469, 8, 11, true);
        let mut record = held(page(4882, 1, 10, false));
        assert!(record.observe_page(Some(partial)));
        assert!(record.observe_page(Some(complete)));
        assert_eq!(record.menu, Some(complete));
    }
}
