//! Arrav membership from the owned journal, never colour, role tags or hidden varps.
use super::pair::Gang;
use api::quest_progress::JournalRead;
use api::selected::{FactKey, Gap, InclusiveRange, Knowledge, SignalRange, SourceSpan, Truth};
use std::sync::{Arc, LazyLock};

// Selected 289 quest_blackarmgang.constant:4,12. Each journal branch's
// opposite-gang outer condition is blackarm < 3 or phoenix < 9 (:16,114).
const PHOENIX_JOINED: i32 = 9;
const BLACKARM_JOINED: i32 = 3;
static UNKNOWN: LazyLock<Gap> = LazyLock::new(|| Gap {
    code: Arc::from("gang-journal-ambiguous"),
    sources: Arc::from([SourceSpan {
        file: Arc::from("scripts/quests/quest_blackarmgang/scripts/blackarmgang_journal.rs2"),
        first: 7,
        last: 198,
    }]),
});

pub struct GangEvidence {
    pub signals: Arc<[SignalRange]>,
    pub gang: Knowledge<Option<Gang>>,
    pub complete: Truth,
}
fn signal(name: &str, min: i32, max: Option<i32>) -> SignalRange {
    SignalRange {
        signal: FactKey::new(name),
        values: InclusiveRange {
            min: Some(min),
            max,
        },
    }
}

pub fn resolve(read: &JournalRead) -> GangEvidence {
    let text = super::progress::normalize_journal(&read.lines);
    resolve_normalized(&read.quest, &text)
}

pub(crate) fn resolve_normalized(quest: &FactKey, text: &str) -> GangEvidence {
    if quest.0.as_ref() != "blackarmgang" {
        return unknown();
    }
    let phoenix = text.contains("allowed me to join the phoenix gang");
    let blackarm = text.contains("allowed me to join the black arm gang");
    let signals: Arc<[SignalRange]> =
        match (phoenix, blackarm) {
            (true, false) => Arc::from([
                signal("phoenixgang", PHOENIX_JOINED, None),
                signal("blackarmgang", 0, Some(BLACKARM_JOINED - 1)),
            ]),
            (false, true) => Arc::from([
                signal("phoenixgang", 0, Some(PHOENIX_JOINED - 1)),
                signal("blackarmgang", BLACKARM_JOINED, None),
            ]),
            (true, true) => return unknown(),
            (false, false) => {
                // Unique pre-join active lines; the pinned enclosing conditions prove
                // BOTH ranges below joining even if both introductions were started.
                // Source :8-10,18,22,26,31,44-46,118-122,134-135.
                let not_started = text.contains("i can start this quest by speaking to reldo")
                    && text.contains("or by speaking to the tramp");
                let prejoin = [
                "reldo says there is a quest hidden in one of the books",
                "i should ask reldo in the varrock palace library",
                "reldo told me that the fur trader",
                "i should find them and join",
                "join the gang in return for killing jonny the beard",
                "speak to their leader, katrine, about joining",
                "he directed me to speak to katrine",
                "she offered to let me join the gang in return for stealing two phoenix crossbows",
            ].iter().any(|needle| text.contains(needle));
                if not_started {
                    Arc::from([
                        signal("phoenixgang", 0, Some(0)),
                        signal("blackarmgang", 0, Some(0)),
                    ])
                } else if prejoin {
                    Arc::from([
                        signal("phoenixgang", 0, Some(PHOENIX_JOINED - 1)),
                        signal("blackarmgang", 0, Some(BLACKARM_JOINED - 1)),
                    ])
                } else {
                    return unknown();
                }
            }
        };
    let gang = from_signals(&signals);
    let complete = if text.contains("and he rewarded me for helping to return the shield of arrav")
    {
        Truth::True
    } else {
        Truth::False
    };
    GangEvidence {
        signals,
        gang,
        complete,
    }
}
fn unknown() -> GangEvidence {
    GangEvidence {
        signals: Arc::from([]),
        gang: Knowledge::Unknown(UNKNOWN.clone()),
        complete: Truth::Unknown,
    }
}
pub fn from_signals(signals: &[SignalRange]) -> Knowledge<Option<Gang>> {
    let joined = |name: &str, minimum: i32| {
        signals
            .iter()
            .find(|row| row.signal.0.as_ref() == name)
            .map_or(Truth::Unknown, |row| {
                InclusiveRange {
                    min: Some(minimum),
                    max: None,
                }
                .test(&row.values)
            })
    };
    match (
        joined("phoenixgang", PHOENIX_JOINED),
        joined("blackarmgang", BLACKARM_JOINED),
    ) {
        (Truth::True, Truth::False) => Knowledge::Known(Some(Gang::Phoenix)),
        (Truth::False, Truth::True) => Knowledge::Known(Some(Gang::BlackArm)),
        (Truth::False, Truth::False) => Knowledge::Known(None),
        _ => Knowledge::Unknown(UNKNOWN.clone()),
    }
}

/// One bounded owned Arrav journal transaction shared by queue selection and re-admission.
#[derive(Default)]
pub(crate) struct GangRead {
    handle: Option<crate::native::ActionHandle<crate::quest_journal::JournalMachine>>,
    since: Option<std::time::Duration>,
}
impl GangRead {
    pub(crate) fn cancel(&mut self) {
        self.handle = None;
        self.since = None;
    }
    pub(crate) fn poll(
        &mut self,
        tick: &mut crate::native::NativeTick<'_>,
        facts: &Arc<api::quest_facts::QuestCatalog>,
    ) -> std::task::Poll<Result<Knowledge<Option<Gang>>, crate::native::ActionError>> {
        use crate::native::ActionError;
        use crate::quest_journal::{JournalMachine, JournalRequest};
        use std::task::Poll;
        let Some(port) = tick.pairs else {
            return Poll::Ready(Err(ActionError::Unavailable(Arc::from(
                "partner capability is not installed in this Play",
            ))));
        };
        let since = *self.since.get_or_insert(tick.cx.active_now());
        if tick.cx.active_now().saturating_sub(since) >= std::time::Duration::from_secs(16) {
            self.cancel();
            return Poll::Ready(Err(ActionError::Blocked(Arc::from(
                "owned Arrav admission journal could not be read",
            ))));
        }
        if self.handle.is_none() {
            match tick.actions.begin::<JournalMachine>(
                JournalRequest {
                    quest: FactKey::new("blackarmgang"),
                    facts: Arc::clone(facts),
                },
                &mut tick.cx,
            ) {
                Ok(handle) => self.handle = Some(handle),
                Err(ActionError::Busy | ActionError::Held | ActionError::BudgetExhausted) => {
                    return Poll::Pending
                }
                Err(error) => return Poll::Ready(Err(error)),
            }
        }
        match tick
            .actions
            .poll(self.handle.as_ref().unwrap(), &mut tick.cx)
        {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => {
                self.cancel();
                Poll::Ready(Err(error))
            }
            Poll::Ready(Ok(read)) => {
                self.cancel();
                Poll::Ready(
                    port.observe_gang(&read)
                        .map_err(super::pair::PairError::action),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ambiguous_ranges_and_two_memberships_never_choose_a_gang() {
        assert!(matches!(from_signals(&[]), Knowledge::Unknown(_)));
        assert!(matches!(
            from_signals(&[
                signal("phoenixgang", 0, Some(9)),
                signal("blackarmgang", 0, Some(2)),
            ]),
            Knowledge::Unknown(_)
        ));
        assert!(matches!(
            from_signals(&[
                signal("phoenixgang", 9, None),
                signal("blackarmgang", 3, None),
            ]),
            Knowledge::Unknown(_)
        ));
        assert!(matches!(
            from_signals(&[
                signal("phoenixgang", 0, Some(8)),
                signal("blackarmgang", 0, Some(2)),
            ]),
            Knowledge::Known(None)
        ));
    }
}
