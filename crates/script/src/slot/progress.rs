//! One-attempt progress policy over the shared Path resolvers and JournalMachine.
use super::*;
use crate::api_progress::{ProgressPage, QuestProgressRow};
use crate::native::{ActionError, ActionHandle, NativeActions};
use crate::quest_journal::{title_matches, JournalMachine, JournalRequest, ROOT_289, TITLE_289};
use crate::quester::{
    compile::CompiledPath,
    progress::{quest_colour, resolve_colour, resolve_journal},
};
use api::quest_facts::QuestCatalog;
use api::selected::{FamilyPreparation, Knowledge, SelectedPin};
use api::snapshot::QuestListStatus;
use std::{task::Poll, time::Duration};

pub(super) type ProgressJob = std::thread::JoinHandle<Result<PreparedProgress, Arc<str>>>;

pub(super) struct PreparedProgress {
    selected: Arc<api::game_data::SelectedGameData>,
    pin: Arc<SelectedPin>,
    path: Arc<CompiledPath>,
    quests: Arc<QuestCatalog>,
}

#[derive(Default)]
pub(super) enum ProgressSeat {
    #[default]
    Idle,
    Preparing {
        token: u64,
        run: RunKey,
        job: ProgressJob,
    },
    Reading {
        token: u64,
        run: RunKey,
        prepared: PreparedProgress,
        since: Option<Duration>,
        // Only journal-rule reads pay for the large machine's storage.
        journal: Option<Box<ActionHandle<JournalMachine>>>,
        discarded: bool,
    },
}

impl ProgressSeat {
    pub(super) fn token(&self) -> Option<u64> {
        match self {
            Self::Idle => None,
            Self::Preparing { token, .. } | Self::Reading { token, .. } => Some(*token),
        }
    }
    pub(super) fn selected(&self) -> Option<Arc<api::game_data::SelectedGameData>> {
        match self {
            Self::Reading { prepared, .. } => Some(Arc::clone(&prepared.selected)),
            _ => None,
        }
    }
    pub(super) fn reset(&mut self, retiring: &mut Vec<ProgressJob>) {
        if let Self::Preparing { job, .. } = std::mem::take(self) {
            retiring.push(job);
        }
    }
    pub(super) fn rekey(&mut self, session: u64) {
        match self {
            Self::Preparing { run, .. } | Self::Reading { run, .. } => run.session = session,
            Self::Idle => {}
        }
    }
}

fn error_reason(error: ActionError) -> Arc<str> {
    match error {
        ActionError::Busy | ActionError::Held | ActionError::BudgetExhausted => "busy".into(),
        ActionError::Stale => "stale".into(),
        ActionError::Cancelled => "cancelled".into(),
        ActionError::UserInput => "manual-movement".into(),
        ActionError::Unavailable(reason) => format!("unavailable:{reason}").into(),
        ActionError::Failed(reason) | ActionError::Blocked(reason) => {
            format!("failed:{reason}").into()
        }
        ActionError::NeedsEvidence(gates) => format!("needs-evidence:{gates:?}").into(),
    }
}

fn compile_error_reason(error: crate::quester::compile::CompileError) -> Arc<str> {
    match error.detail.as_deref() {
        Some(detail) => format!("failed:{}: {detail}", error.code).into(),
        None => format!("failed:{}", error.code).into(),
    }
}

fn row(
    prepared: &PreparedProgress,
    progress: api::quest_progress::QuestProgress,
    colour: QuestListStatus,
    journal_read: bool,
) -> Arc<QuestProgressRow> {
    let knowledge = |value: Knowledge<api::selected::FactKey>| match value {
        Knowledge::Known(value) => Knowledge::Known(value.0),
        Knowledge::Unknown(gap) => Knowledge::Unknown(gap),
        Knowledge::Partial { gaps, .. } => {
            Knowledge::Unknown(gaps.first().cloned().unwrap_or_else(|| api::selected::Gap {
                code: "partial-progress".into(),
                sources: Arc::from([]),
            }))
        }
    };
    Arc::new(QuestProgressRow {
        quest: progress.quest.0,
        display: Arc::clone(&prepared.path.display_name),
        colour,
        stage: knowledge(progress.stage),
        complete: progress.complete,
        rule: knowledge(progress.rule),
        flags: progress.flags,
        evidence: progress.evidence,
        journal_read,
        binding: progress.binding.0,
        role: progress.role.map(|role| role.0),
    })
}

impl SlotScript {
    pub(super) fn start_api_progress(&mut self, token: u64, quest: &str) {
        if !self.load_active() {
            return;
        }
        let seat = self.api.get_or_insert_with(|| Box::new(ApiSeat::default()));
        seat.reap();
        if seat.progress.token() == Some(token)
            || seat
                .progress_page
                .as_ref()
                .is_some_and(|page| page.token() == token)
        {
            return;
        }
        if seat.gather.token().is_some() {
            seat.progress_page = Some(ProgressPage::Refused {
                token,
                reason: "busy".into(),
            });
            return;
        }
        if let ProgressSeat::Reading {
            journal: Some(_),
            discarded,
            ..
        } = &mut seat.progress
        {
            // Superseding the waiter must not revoke an owned journal close.
            *discarded = true;
            seat.progress_page = Some(ProgressPage::Refused {
                token,
                reason: "busy".into(),
            });
            return;
        }
        if seat.retiring.len() + seat.progress_retiring.len() >= 4 {
            seat.progress_page = Some(ProgressPage::Refused {
                token,
                reason: "busy".into(),
            });
            return;
        }
        seat.progress.reset(&mut seat.progress_retiring);
        let Some(bytes) = crate::quester::card::released_path(quest) else {
            seat.progress_page = Some(ProgressPage::Refused {
                token,
                reason: "unknown-path".into(),
            });
            return;
        };
        let Some(selected) = self
            .load_identity
            .as_ref()
            .and_then(|identity| identity.game_data.clone())
        else {
            seat.progress_page = Some(ProgressPage::Refused {
                token,
                reason: "unavailable:selected game data unavailable".into(),
            });
            return;
        };
        let Some(generation) = self
            .control_generation
            .max(self.runtime_generation)
            .checked_add(1)
        else {
            seat.progress_page = Some(ProgressPage::Refused {
                token,
                reason: "unavailable:run generation exhausted".into(),
            });
            return;
        };
        let run = RunKey {
            slot: self.incarnation,
            run: generation,
            session: self.work_epoch,
        };
        let job = FamilyPreparation::run(move |cap| {
            let pin = selected
                .selected_pin()
                .map_err(|error| Arc::from(format!("failed:{error:?}")))?;
            let quests = Arc::new(
                QuestCatalog::from_identity(selected.quest_identity())
                    .map_err(|error| Arc::from(format!("failed:{error:?}")))?,
            );
            let path = crate::quester::compile::compile_path(bytes, &selected, &quests, cap)
                .map_err(compile_error_reason)?;
            Ok(PreparedProgress {
                selected,
                pin,
                path,
                quests,
            })
        });
        match job {
            Ok(job) => {
                self.control_generation = generation;
                seat.progress = ProgressSeat::Preparing { token, run, job };
                seat.progress_page = Some(ProgressPage::Reading { token });
            }
            Err(error) => {
                seat.progress_page = Some(ProgressPage::Refused {
                    token,
                    reason: format!("unavailable:{error}").into(),
                })
            }
        }
    }

    pub(super) fn tick_api_progress(&mut self, seat: &mut ApiSeat, ctx: &mut ScriptCtx<'_>) {
        if matches!(&seat.progress, ProgressSeat::Preparing { job, .. } if job.is_finished()) {
            let ProgressSeat::Preparing {
                token,
                mut run,
                job,
            } = std::mem::take(&mut seat.progress)
            else {
                unreachable!()
            };
            match job
                .join()
                .unwrap_or_else(|_| Err("unavailable:preparation worker panic".into()))
            {
                Ok(prepared) => {
                    run.session = self.work_epoch;
                    seat.progress = ProgressSeat::Reading {
                        token,
                        run,
                        prepared,
                        since: None,
                        journal: None,
                        discarded: false,
                    };
                }
                Err(reason) => seat.progress_page = Some(ProgressPage::Refused { token, reason }),
            }
        }
        let ProgressSeat::Reading {
            token,
            run,
            prepared,
            since,
            journal,
            discarded,
        } = &mut seat.progress
        else {
            return;
        };
        if ctx.compiled.hold {
            return;
        }
        let retained = self
            .retained
            .get_or_insert_with(|| Arc::new(Mutex::new(RetainedMemory::default())));
        let mut retained = retained
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut cx = compiled::frame_context(
            ctx,
            *run,
            &prepared.pin,
            &mut retained,
            &mut self.native_runtime,
        );
        let mut actions = NativeActions { _private: () };
        let terminal = if let Some(handle) = journal.as_deref() {
            match actions.poll(handle, &mut cx) {
                Poll::Pending => return,
                Poll::Ready(Err(error)) => ProgressPage::Refused {
                    token: *token,
                    reason: error_reason(error),
                },
                Poll::Ready(Ok(read)) => {
                    #[cfg(feature = "test-hooks")]
                    {
                        seat.journal_evidence = Some((read.acquired, read.closed));
                    }
                    ProgressPage::Done {
                        token: *token,
                        row: row(
                            prepared,
                            resolve_journal(&prepared.path, &read, None),
                            QuestListStatus::InProgress,
                            true,
                        ),
                    }
                }
            }
        } else {
            let colour = quest_colour(&prepared.path, &prepared.quests, cx.snapshot());
            let Some(colour) = colour.filter(|colour| *colour != QuestListStatus::Unknown) else {
                let since = since.get_or_insert(cx.active_now());
                if cx.active_now().saturating_sub(*since) < Duration::from_secs(8) {
                    return;
                }
                if !*discarded {
                    seat.progress_page = Some(ProgressPage::Refused {
                        token: *token,
                        reason: "unavailable:quest colour".into(),
                    });
                }
                seat.progress = ProgressSeat::Idle;
                return;
            };
            if colour == QuestListStatus::InProgress && !prepared.path.progress.rules.is_empty() {
                // A modal owned by another quest is foreground occupancy, not a failed read.
                let snapshot = cx.snapshot();
                let occupied = snapshot.main_modal().is_some_and(|modal| {
                    if modal.value.root == ROOT_289 {
                        let expected = prepared
                            .quests
                            .quest(prepared.path.id.0.as_ref())
                            .ok()
                            .and_then(|facts| facts.journal_title.as_deref());
                        !expected.is_some_and(|expected| {
                            snapshot
                                .journal_widgets(ROOT_289, TITLE_289)
                                .is_some_and(|page| title_matches(page.value.title, expected))
                        })
                    } else {
                        modal.value.root != -1
                    }
                }) || snapshot
                    .chat_modal()
                    .is_some_and(|modal| modal.value.root != -1 || !modal.value.texts.is_empty());
                if occupied {
                    ProgressPage::Refused {
                        token: *token,
                        reason: "busy".into(),
                    }
                } else {
                    match actions.begin::<JournalMachine>(
                        JournalRequest {
                            quest: prepared.path.id.clone(),
                            facts: Arc::clone(&prepared.quests),
                        },
                        &mut cx,
                    ) {
                        Ok(handle) => {
                            *journal = Some(Box::new(handle));
                            return;
                        }
                        Err(error) => ProgressPage::Refused {
                            token: *token,
                            reason: error_reason(error),
                        },
                    }
                }
            } else {
                ProgressPage::Done {
                    token: *token,
                    row: row(
                        prepared,
                        resolve_colour(
                            &prepared.path,
                            colour,
                            cx.evidence(),
                            Arc::clone(&prepared.pin),
                        ),
                        colour,
                        false,
                    ),
                }
            }
        };
        if !*discarded {
            seat.progress_page = Some(terminal);
        }
        seat.progress = ProgressSeat::Idle;
    }
}

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
