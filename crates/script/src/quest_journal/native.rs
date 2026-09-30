//! One click, selected title/root validation, capture, one close, observed close.
use crate::native::{ActionContext, ActionError, NativeMachine, QuietReadLease};
use crate::shim::InteractReq;
use api::quest_facts::QuestCatalog;
use api::quest_progress::{EvidenceStamp, JournalRead};
use api::selected::{ClientRevision, FactKey, SelectedPin};
use std::{sync::Arc, task::Poll, time::Duration};

// Selected 289 interface.pack: questjournal_scroll and :ifquestname. The
// generic player/quest_journal.rs2 opens this root for every roster journal.
const ROOT_289: i32 = 8134;
const TITLE_289: i32 = 8144;
const WINDOW: Duration = Duration::from_secs(3);
const OVERALL: Duration = Duration::from_secs(8);
pub struct JournalRequest {
    pub quest: FactKey,
    pub facts: Arc<QuestCatalog>,
}

enum Phase {
    Click,
    Acquire { request: u64 },
    Adopt { before: EvidenceStamp },
    Close,
    Closing { request: u64 },
}

pub struct JournalMachine {
    quest: FactKey,
    title: Arc<str>,
    component: i32,
    colour: i32,
    pin: Arc<SelectedPin>,
    lease: Option<QuietReadLease>,
    phase: Phase,
    deadline: Duration,
    overall_deadline: Duration,
    acquired: Option<EvidenceStamp>,
    lines: Arc<[Arc<str>]>,
}

fn failure(reason: &'static str) -> ActionError {
    ActionError::Failed(Arc::from(reason))
}

fn title_matches(actual: &str, expected: &str) -> bool {
    // The selected server prefixes the title with @dre@. Do not infer a title
    // from body position or accept a different quest on the same root.
    actual.strip_prefix("@dre@").unwrap_or(actual).trim() == expected
}

fn strictly_later(actual: EvidenceStamp, before: EvidenceStamp) -> bool {
    actual.meets(before) && actual != before
}

fn display_title(actual: &str) -> &str {
    actual.strip_prefix("@dre@").unwrap_or(actual).trim()
}

fn foreign_modal_failure(root: i32, texts: &[String]) -> Option<ActionError> {
    if root >= 0 {
        let text = texts.iter().find(|text| !text.is_empty());
        return Some(match text {
            Some(text) => ActionError::Failed(Arc::from(format!(
                "journal blocked by modal root {root} ({text})"
            ))),
            None => ActionError::Failed(Arc::from(format!(
                "journal blocked by modal root {root}"
            ))),
        });
    }
    texts
        .iter()
        .find(|text| !text.is_empty())
        .map(|text| {
            ActionError::Failed(Arc::from(format!(
                "journal blocked by modal text ({text})"
            )))
        })
}

impl NativeMachine for JournalMachine {
    type Args = JournalRequest;
    type Output = JournalRead;

    fn begin(args: JournalRequest, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        if cx.pin().revision != ClientRevision::R289 {
            return Err(ActionError::Unavailable(Arc::from(
                "journal interface unavailable for selected revision",
            )));
        }
        let facts = args
            .facts
            .quest(args.quest.0.as_ref())
            .map_err(|_| failure("unknown journal quest"))?;
        let title = facts
            .journal_title
            .as_ref()
            .ok_or_else(|| failure("journal title unavailable"))?;
        let snapshot = cx.snapshot();
        let pair = snapshot.main_modal().ok_or(ActionError::Busy)?;
        let chat = snapshot.chat_modal().ok_or(ActionError::Busy)?;
        let adopted_before = if chat.value.root != -1 || !chat.value.texts.is_empty() {
            return Err(foreign_modal_failure(chat.value.root, chat.value.texts)
                .unwrap_or(ActionError::Busy));
        } else if pair.value.root == ROOT_289 {
            match snapshot.journal_widgets(ROOT_289, TITLE_289) {
                Some(page) if title_matches(page.value.title, title) => Some(page.stamp),
                Some(page) => {
                    return Err(ActionError::Failed(Arc::from(format!(
                        "journal blocked by quest journal '{}'",
                        display_title(page.value.title)
                    ))));
                }
                None => return Err(ActionError::Busy),
            }
        } else if pair.value.root != -1 || !pair.value.texts.is_empty() {
            return Err(foreign_modal_failure(pair.value.root, pair.value.texts)
                .unwrap_or(ActionError::Busy));
        } else {
            None
        };
        let rows = snapshot
            .quest_statuses()
            .ok_or_else(|| failure("quest tab unavailable"))?;
        let row = rows
            .value
            .iter()
            .find(|row| row.name.eq_ignore_ascii_case(&facts.display))
            .filter(|row| row.component_id >= 0)
            .ok_or_else(|| failure("quest row unavailable"))?;
        let component = row.component_id;
        let colour = row.colour;
        let title = Arc::clone(title);
        let lease = cx.begin_quiet_read(cx.action_id())?;
        Ok(Self {
            quest: args.quest,
            title,
            component,
            colour,
            pin: Arc::new(cx.pin().clone()),
            lease: Some(lease),
            phase: adopted_before
                .map(|before| Phase::Adopt { before })
                .unwrap_or(Phase::Click),
            deadline: cx.active_now() + WINDOW,
            overall_deadline: cx.active_now() + OVERALL,
            acquired: None,
            lines: Arc::from([]),
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<JournalRead, ActionError>> {
        if cx.active_now() >= self.overall_deadline {
            return Poll::Ready(Err(failure("journal transaction timeout")));
        }
        if !self.lease.as_ref().is_some_and(QuietReadLease::live) {
            return Poll::Ready(Err(failure("journal quiet lease expired")));
        }
        if cx.active_now() >= self.deadline {
            return Poll::Ready(Err(failure("journal modal timeout")));
        }
        let snapshot = cx.snapshot();
        let Some(pair) = snapshot.main_modal() else {
            return Poll::Pending;
        };
        match self.phase {
            Phase::Click => {
                if pair.value.root != -1
                    || !pair.value.texts.is_empty()
                    || snapshot
                        .chat_modal()
                        .is_none_or(|chat| chat.value.root != -1)
                {
                    return Poll::Ready(Err(ActionError::Busy));
                }
                match cx.emit(InteractReq::IfButton {
                    component_id: self.component,
                }) {
                    Ok(request) => {
                        self.phase = Phase::Acquire { request };
                        self.deadline = cx.active_now() + WINDOW;
                    }
                    Err(ActionError::BudgetExhausted) => {}
                    Err(error) => return Poll::Ready(Err(error)),
                }
            }
            Phase::Acquire { request } => {
                let Some(receipt) = cx.interaction_receipt(request) else {
                    return Poll::Pending;
                };
                if !receipt.accepted {
                    return Poll::Ready(Err(failure("journal click refused")));
                }
                if !pair.stamp.meets(receipt.evidence) || pair.stamp == receipt.evidence {
                    return Poll::Pending;
                }
                if pair.value.root == -1 {
                    return Poll::Pending;
                }
                if pair.value.root != ROOT_289 {
                    return Poll::Ready(Err(failure("journal root changed")));
                }
                let Some(page) = snapshot.journal_widgets(ROOT_289, TITLE_289) else {
                    return Poll::Pending;
                };
                if !title_matches(page.value.title, &self.title) {
                    return Poll::Ready(Err(failure("journal title changed")));
                }
                self.lines = page.value.lines().map(Arc::<str>::from).collect();
                self.acquired = Some(page.stamp);
                self.phase = Phase::Close;
                self.deadline = cx.active_now() + WINDOW;
            }
            Phase::Adopt { before } => {
                if snapshot.chat_modal().is_none_or(|chat| {
                    chat.value.root != -1 || !chat.value.texts.is_empty()
                }) {
                    return Poll::Ready(Err(failure("journal ownership lost before close")));
                }
                let Some(page) = snapshot.journal_widgets(ROOT_289, TITLE_289) else {
                    return Poll::Ready(Err(failure("journal ownership lost before close")));
                };
                if !strictly_later(page.stamp, before)
                    || !title_matches(page.value.title, &self.title)
                {
                    if !strictly_later(page.stamp, before) {
                        return Poll::Pending;
                    }
                    return Poll::Ready(Err(failure("journal ownership lost before close")));
                }
                self.lines = page.value.lines().map(Arc::<str>::from).collect();
                self.acquired = Some(page.stamp);
                self.phase = Phase::Close;
                self.deadline = cx.active_now() + WINDOW;
            }
            Phase::Close => {
                if snapshot.chat_modal().is_none_or(|chat| {
                    chat.value.root != -1 || !chat.value.texts.is_empty()
                }) {
                    return Poll::Ready(Err(failure("journal ownership lost before close")));
                }
                let Some(page) = snapshot.journal_widgets(ROOT_289, TITLE_289) else {
                    return Poll::Ready(Err(failure("journal ownership lost before close")));
                };
                if !title_matches(page.value.title, &self.title)
                    || !page.value.lines().eq(self.lines.iter().map(AsRef::as_ref))
                {
                    return Poll::Ready(Err(failure("journal ownership lost before close")));
                }
                match cx.emit(InteractReq::CloseModal) {
                    Ok(request) => {
                        self.phase = Phase::Closing { request };
                        self.deadline = cx.active_now() + WINDOW;
                    }
                    Err(ActionError::BudgetExhausted) => {}
                    Err(error) => return Poll::Ready(Err(error)),
                }
            }
            Phase::Closing { request } => {
                let Some(receipt) = cx.interaction_receipt(request) else {
                    return Poll::Pending;
                };
                if !receipt.accepted {
                    return Poll::Ready(Err(failure("journal close refused")));
                }
                if !pair.stamp.meets(receipt.evidence) || pair.stamp == receipt.evidence {
                    return Poll::Pending;
                }
                if snapshot.chat_modal().is_none_or(|chat| {
                    chat.value.root != -1 || !chat.value.texts.is_empty()
                }) {
                    return Poll::Ready(Err(failure("journal ownership lost while closing")));
                }
                if pair.value.root == -1 && pair.value.texts.is_empty() {
                    let closed = pair.stamp;
                    if let Some(lease) = self.lease.take() {
                        cx.end_quiet_read(lease);
                    }
                    return Poll::Ready(Ok(JournalRead {
                        quest: self.quest.clone(),
                        root: ROOT_289,
                        lines: Arc::clone(&self.lines),
                        colour: Some(self.colour),
                        acquired: self.acquired.expect("captured before close"),
                        closed,
                        pin: Arc::clone(&self.pin),
                    }));
                }
                if pair.value.root != ROOT_289 {
                    return Poll::Ready(Err(failure("journal ownership lost while closing")));
                }
            }
        }
        Poll::Pending
    }

    fn cancel(&mut self) {
        // Revocation precedes this callback. No compensating close can target
        // somebody else's modal; releasing the lease is unconditional.
        self.lease = None;
    }
}

#[cfg(test)]
#[path = "native_tests.rs"]
pub(crate) mod tests;
