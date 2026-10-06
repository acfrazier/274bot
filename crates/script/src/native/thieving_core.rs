use std::time::Duration;

pub(crate) const PICKPOCKET: &str = "Pickpocket";
pub(crate) const STEAL_FROM: &str = "Steal-from";

pub(crate) fn level_ready(effective: Option<i32>, required: i32) -> Option<bool> {
    effective.map(|level| level >= required)
}

#[cfg(test)]
pub(crate) const DEFAULT_ACTION_DEADLINE: Duration = Duration::from_secs(60);
const ATTEMPT_WINDOW: Duration = Duration::from_millis(2_500);
const RETRY_GAP: Duration = Duration::from_millis(600);
const STUN_LOCK: Duration = Duration::from_secs(9);
const RECEIPT_DEADLINE: Duration = Duration::from_millis(2_500);

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ChatEvidence {
    pub response: bool,
    pub failure: bool,
    pub stunned: bool,
    pub level_refusal: bool,
    pub through_sequence: i32,
}

impl ChatEvidence {
    pub(crate) fn observe(&mut self, action: &str, text: &str, sequence: i32) {
        self.through_sequence = self.through_sequence.max(sequence);
        if contains_ascii(text, "attempt to pick")
            || contains_ascii(text, "attempt to steal")
            || contains_ascii(text, "can't reach")
            || contains_ascii(text, "cannot reach")
        {
            return;
        }
        if contains_ascii(text, "stunned") {
            self.stunned = true;
            return;
        }
        if contains_ascii(text, "need to be at level") || contains_ascii(text, "need to be a level")
        {
            self.level_refusal = true;
            self.failure = true;
            return;
        }
        if contains_ascii(text, "fail")
            || contains_ascii(text, "cannot")
            || contains_ascii(text, "can't")
        {
            self.failure = true;
            return;
        }
        let pickpocket = action.eq_ignore_ascii_case(PICKPOCKET)
            && (contains_ascii(text, "you pick the")
                || contains_ascii(text, "you steal")
                || contains_ascii(text, "you find"));
        let steal = action.eq_ignore_ascii_case(STEAL_FROM) && contains_ascii(text, "you steal");
        self.response |= pickpocket || steal;
    }
}

fn contains_ascii(text: &str, needle: &str) -> bool {
    text.as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Observation {
    pub effective_thieving: Option<i32>,
    pub required_level: Option<i32>,
    pub experience: Option<i32>,
    pub inventory_used: Option<i32>,
    pub target_count: Option<i32>,
    pub inventory_ready: bool,
    pub chat: ChatEvidence,
    pub chat_dialog_open: bool,
    pub chat_dialog_fingerprint: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    InvalidConfiguration,
    Level,
    Deadline,
    Attempts,
    DispatchRejected,
    ReceiptTimeout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Decision {
    Wait,
    Dispatch,
    Complete { held: i32 },
    AttemptResolved,
    AttemptFailed,
    AttemptTimedOut,
    Stunned,
    Dialogue,
    Failed(Failure),
}

#[derive(Clone, Copy)]
struct Attempt {
    started: Duration,
    request_id: Option<u64>,
    accepted: Option<bool>,
    before_experience: Option<i32>,
    before_inventory_used: Option<i32>,
    before_target_count: Option<i32>,
    before_chat_dialog_open: bool,
    before_chat_dialog_fingerprint: Option<u64>,
    failure_observed: bool,
}

pub(crate) struct ThieveCore {
    goal_qty: Option<i32>,
    attempts_left: Option<u8>,
    require_receipt: bool,
    deadline: Duration,
    next_attempt: Duration,
    stunned_until: Option<Duration>,
    pending: Option<Attempt>,
    chat_since: i32,
}

impl ThieveCore {
    pub(crate) fn new(
        now: Duration,
        deadline_after: Duration,
        goal_qty: Option<i32>,
        max_attempts: Option<u8>,
        require_receipt: bool,
        chat_since: i32,
    ) -> Result<Self, Failure> {
        if deadline_after.is_zero()
            || goal_qty.is_some_and(|qty| qty < 0)
            || max_attempts == Some(0)
        {
            return Err(Failure::InvalidConfiguration);
        }
        Ok(Self {
            goal_qty,
            attempts_left: max_attempts,
            require_receipt,
            deadline: now.saturating_add(deadline_after),
            next_attempt: now,
            stunned_until: None,
            pending: None,
            chat_since,
        })
    }

    pub(crate) fn chat_since(&self) -> i32 {
        self.chat_since
    }

    pub(crate) fn pending_request_id(&self) -> Option<u64> {
        self.pending?.request_id
    }

    pub(crate) fn waiting_for_receipt(&self) -> bool {
        self.pending
            .is_some_and(|attempt| attempt.request_id.is_some() && attempt.accepted.is_none())
    }

    pub(crate) fn stun_active(&self, now: Duration) -> bool {
        self.stunned_until.is_some_and(|until| now < until)
    }

    pub(crate) fn receipt(&mut self, request_id: u64, accepted: bool, chat_since: i32) {
        let Some(attempt) = self.pending.as_mut() else {
            return;
        };
        if attempt.request_id != Some(request_id) || attempt.accepted.is_some() {
            return;
        }
        attempt.accepted = Some(accepted);
        self.chat_since = self.chat_since.max(chat_since);
    }

    pub(crate) fn start_attempt(
        &mut self,
        now: Duration,
        request_id: Option<u64>,
        observation: &Observation,
        chat_since: i32,
    ) -> Result<(), Failure> {
        if self.pending.is_some()
            || self.attempts_left == Some(0)
            || self.require_receipt != request_id.is_some()
        {
            return Err(Failure::InvalidConfiguration);
        }
        if let Some(remaining) = self.attempts_left.as_mut() {
            *remaining -= 1;
        }
        self.chat_since = self.chat_since.max(chat_since);
        self.pending = Some(Attempt {
            started: now,
            request_id,
            accepted: (!self.require_receipt).then_some(true),
            before_experience: observation.experience,
            before_inventory_used: observation.inventory_used,
            before_target_count: observation.target_count,
            before_chat_dialog_open: observation.chat_dialog_open,
            before_chat_dialog_fingerprint: observation.chat_dialog_fingerprint,
            failure_observed: false,
        });
        Ok(())
    }

    pub(crate) fn poll(&mut self, now: Duration, observation: Observation) -> Decision {
        self.chat_since = self.chat_since.max(observation.chat.through_sequence);
        if now >= self.deadline {
            return Decision::Failed(Failure::Deadline);
        }
        if let Some(attempt) = self.pending {
            if self.require_receipt && attempt.accepted.is_none() {
                return if now.saturating_sub(attempt.started) >= RECEIPT_DEADLINE {
                    Decision::Failed(Failure::ReceiptTimeout)
                } else {
                    Decision::Wait
                };
            }
            if attempt.accepted == Some(false) {
                return Decision::Failed(Failure::DispatchRejected);
            }
        }
        if self
            .goal_qty
            .is_some_and(|qty| observation.target_count.is_some_and(|count| count >= qty))
        {
            return Decision::Complete {
                held: observation.target_count.unwrap_or_default(),
            };
        }
        if observation.chat.level_refusal {
            self.pending = None;
            return Decision::Failed(Failure::Level);
        }
        if observation.chat.stunned {
            let until = now.saturating_add(STUN_LOCK);
            self.stunned_until = Some(until);
            self.next_attempt = until;
            self.pending = None;
            return Decision::Stunned;
        }

        if let Some(attempt) = self.pending {
            if observation.chat.failure && !attempt.failure_observed {
                // Generic failures still have p_delay(0) calls before the stun.
                // Keep this attempt pending until that real line arrives.
                self.pending = Some(Attempt {
                    started: now,
                    failure_observed: true,
                    ..attempt
                });
                return if self.attempts_left == Some(0) {
                    Decision::Failed(Failure::Attempts)
                } else {
                    Decision::AttemptFailed
                };
            }
            if attempt.failure_observed {
                if now.saturating_sub(attempt.started) < ATTEMPT_WINDOW {
                    return Decision::Wait;
                }
                self.pending = None;
                self.next_attempt = now.saturating_add(RETRY_GAP);
                return Decision::AttemptTimedOut;
            }
            let new_dialogue = observation.chat_dialog_open
                && (!attempt.before_chat_dialog_open
                    || observation.chat_dialog_fingerprint
                        != attempt.before_chat_dialog_fingerprint);
            if new_dialogue {
                self.pending = None;
                self.next_attempt = now.saturating_add(RETRY_GAP);
                return Decision::Dialogue;
            }
            let experience_changed = observation
                .experience
                .zip(attempt.before_experience)
                .is_some_and(|(after, before)| after > before);
            let inventory_changed = observation
                .inventory_used
                .zip(attempt.before_inventory_used)
                .is_some_and(|(after, before)| after > before);
            let target_changed = observation
                .target_count
                .zip(attempt.before_target_count)
                .is_some_and(|(after, before)| after > before);
            if experience_changed
                || inventory_changed
                || target_changed
                || observation.chat.response
            {
                self.pending = None;
                self.next_attempt = now.saturating_add(RETRY_GAP);
                return Decision::AttemptResolved;
            }
            if now.saturating_sub(attempt.started) >= ATTEMPT_WINDOW {
                self.pending = None;
                self.next_attempt = now.saturating_add(RETRY_GAP);
                return if self.attempts_left == Some(0) {
                    Decision::Failed(Failure::Attempts)
                } else {
                    Decision::AttemptTimedOut
                };
            }
            return Decision::Wait;
        }

        if self.stunned_until.is_some_and(|until| now < until) || observation.chat_dialog_open {
            return Decision::Wait;
        }
        let Some(effective) = observation.effective_thieving else {
            return Decision::Wait;
        };
        let Some(required) = observation.required_level else {
            return Decision::Wait;
        };
        if level_ready(Some(effective), required) == Some(false) {
            return Decision::Failed(Failure::Level);
        }
        if !observation.inventory_ready || now < self.next_attempt {
            return Decision::Wait;
        }
        if self.attempts_left == Some(0) {
            return Decision::Failed(Failure::Attempts);
        }
        Decision::Dispatch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(level: Option<i32>, required: Option<i32>) -> Observation {
        Observation {
            effective_thieving: level,
            required_level: required,
            experience: Some(100),
            inventory_used: Some(1),
            target_count: Some(0),
            inventory_ready: true,
            chat: ChatEvidence::default(),
            chat_dialog_open: false,
            chat_dialog_fingerprint: None,
        }
    }

    fn core(goal_qty: Option<i32>, max_attempts: u8, receipt: bool) -> ThieveCore {
        ThieveCore::new(
            Duration::ZERO,
            DEFAULT_ACTION_DEADLINE,
            goal_qty,
            Some(max_attempts),
            receipt,
            0,
        )
        .unwrap()
    }

    fn start(core: &mut ThieveCore, now: Duration, request_id: Option<u64>, obs: Observation) {
        core.start_attempt(now, request_id, &obs, obs.chat.through_sequence)
            .unwrap();
    }

    #[test]
    fn effective_level_gate_fails_closed_on_unknown_and_low_evidence() {
        let mut unknown = core(Some(1), 2, true);
        assert_eq!(
            unknown.poll(Duration::ZERO, observation(None, Some(30))),
            Decision::Wait
        );

        let mut low = core(Some(1), 2, true);
        assert_eq!(
            low.poll(Duration::ZERO, observation(Some(29), Some(30))),
            Decision::Failed(Failure::Level)
        );
    }

    #[test]
    fn level_ready_distinguishes_unobserved_from_below_requirement() {
        assert_eq!(level_ready(None, 30), None);
        assert_eq!(level_ready(Some(29), 30), Some(false));
        assert_eq!(level_ready(Some(30), 30), Some(true));
    }

    #[test]
    fn pickpocket_success_line_is_a_response_but_attempt_is_not() {
        // `scripts/skill_thieving/scripts/thieving.rs2`, [proc,pick_pocket]
        // emits the attempt at line 4 and the success line at line 24.
        let mut attempt = ChatEvidence::default();
        attempt.observe("Pickpocket", "You attempt to pick the man's pocket.", 1);
        assert!(!attempt.response && !attempt.failure && !attempt.stunned);

        let mut success = ChatEvidence::default();
        success.observe("Pickpocket", "You pick the man's pocket.", 2);
        assert!(success.response);

        // `scripts/quests/quest_itexam/scripts/digsite_workman.rs2`
        // [label,pickpocket_digworkman1] reports loot as success too.
        let mut workman_success = ChatEvidence::default();
        workman_success.observe("Pickpocket", "You steal some money.", 3);
        assert!(workman_success.response);
    }

    #[test]
    fn generic_pickpocket_failure_then_delayed_stun_follow_content_order() {
        // `scripts/skill_thieving/scripts/thieving.rs2`: attempt and p_delay(0)
        // are lines 4-5; failure is line 32, then the actual stun is line 48.
        let mut core = core(None, 2, true);
        let mut observation = observation(Some(30), Some(1));
        assert_eq!(core.poll(Duration::ZERO, observation), Decision::Dispatch);
        start(&mut core, Duration::ZERO, Some(7), observation);
        core.receipt(7, true, 0);

        let mut attempt = ChatEvidence::default();
        attempt.observe("Pickpocket", "You attempt to pick the man's pocket.", 1);
        observation.chat = attempt;
        assert_eq!(
            core.poll(Duration::from_millis(600), observation),
            Decision::Wait,
            "attempt text is not the attempt outcome"
        );

        let mut failure = ChatEvidence::default();
        failure.observe("Pickpocket", "You fail to pick the man's pocket.", 2);
        assert!(failure.failure && !failure.stunned && !failure.response);
        observation.chat = failure;
        assert_eq!(
            core.poll(Duration::from_millis(1_200), observation),
            Decision::AttemptFailed
        );
        observation.chat = ChatEvidence {
            through_sequence: 2,
            ..ChatEvidence::default()
        };
        for millis in [1_800, 2_399] {
            assert_eq!(
                core.poll(Duration::from_millis(millis), observation),
                Decision::Wait,
                "the two p_delay(0) calls after failure must not admit another pick"
            );
        }

        let mut stun = ChatEvidence::default();
        stun.observe("Pickpocket", "You've been stunned!", 3);
        assert!(stun.stunned && !stun.response);
        observation.chat = stun;
        assert_eq!(
            core.poll(Duration::from_millis(2_400), observation),
            Decision::Stunned,
            "only the real stun line starts the stun lock"
        );
        observation.chat = ChatEvidence {
            through_sequence: 3,
            ..ChatEvidence::default()
        };
        assert_eq!(
            core.poll(Duration::from_millis(11_399), observation),
            Decision::Wait
        );
        assert_eq!(
            core.poll(Duration::from_millis(11_400), observation),
            Decision::Dispatch
        );
    }

    #[test]
    fn troll_guard_failure_is_a_failure_not_a_stun() {
        // `scripts/quests/quest_troll/scripts/troll_stronghold_camp_guard.rs2`,
        // [proc,troll_prison_guard_steal] attempt/failure lines 50-63 have no stun.
        let mut core = core(None, 2, false);
        let mut observation = observation(Some(30), Some(30));
        assert_eq!(core.poll(Duration::ZERO, observation), Decision::Dispatch);
        start(&mut core, Duration::ZERO, None, observation);

        let mut attempt = ChatEvidence::default();
        attempt.observe("Pickpocket", "You attempt to pick the guard's pocket.", 1);
        observation.chat = attempt;
        assert_eq!(
            core.poll(Duration::from_millis(600), observation),
            Decision::Wait
        );

        let mut failure = ChatEvidence::default();
        failure.observe("Pickpocket", "You fail to pick the guard's pocket.", 2);
        assert!(failure.failure && !failure.stunned && !failure.response);
        observation.chat = failure;
        assert_eq!(
            core.poll(Duration::from_millis(1_200), observation),
            Decision::AttemptFailed
        );
    }

    #[test]
    fn workman_level_refusal_is_terminal_not_a_pickpocket_response() {
        // `scripts/quests/quest_itexam/scripts/digsite_workman.rs2`,
        // [label,pickpocket_digworkman1] attempt/p_delay(2) lines 58-59 and
        // level refusal lines 62-64.
        let mut core = core(None, 2, false);
        let mut observation = observation(Some(30), Some(25));
        assert_eq!(core.poll(Duration::ZERO, observation), Decision::Dispatch);
        start(&mut core, Duration::ZERO, None, observation);

        let mut attempt = ChatEvidence::default();
        attempt.observe(
            "Pickpocket",
            "You attempt to pick the workman's pocket...",
            1,
        );
        observation.chat = attempt;
        assert_eq!(
            core.poll(Duration::from_millis(600), observation),
            Decision::Wait
        );
        observation.chat = ChatEvidence {
            through_sequence: 1,
            ..ChatEvidence::default()
        };
        assert_eq!(
            core.poll(Duration::from_millis(1_799), observation),
            Decision::Wait,
            "workman p_delay(2) must not admit another pick"
        );

        let mut refusal = ChatEvidence::default();
        refusal.observe(
            "Pickpocket",
            "You need to be at level 25 Thieving to pick the workman's pocket.",
            2,
        );
        assert!(refusal.failure && !refusal.response && !refusal.stunned);
        observation.chat = refusal;
        assert_eq!(
            core.poll(Duration::from_millis(1_800), observation),
            Decision::Failed(Failure::Level)
        );
    }

    #[test]
    fn workman_failure_waits_through_content_delays_for_the_real_stun() {
        // `scripts/quests/quest_itexam/scripts/digsite_workman.rs2`:
        // attempt/p_delay(2) at 58-59, failure/p_delay(0) at 69-71, stun at 78.
        let mut core = core(None, 2, false);
        let mut observation = observation(Some(30), Some(25));
        start(&mut core, Duration::ZERO, None, observation);
        let mut attempt = ChatEvidence::default();
        attempt.observe(
            "Pickpocket",
            "You attempt to pick the workman's pocket...",
            1,
        );
        observation.chat = attempt;
        assert_eq!(
            core.poll(Duration::from_millis(600), observation),
            Decision::Wait
        );
        observation.chat = ChatEvidence {
            through_sequence: 1,
            ..ChatEvidence::default()
        };
        assert_eq!(
            core.poll(Duration::from_millis(1_799), observation),
            Decision::Wait
        );
        let mut failure = ChatEvidence::default();
        failure.observe("Pickpocket", "You fail to pick the workman's pocket.", 2);
        observation.chat = failure;
        assert_eq!(
            core.poll(Duration::from_millis(1_800), observation),
            Decision::AttemptFailed
        );
        observation.chat = ChatEvidence {
            through_sequence: 2,
            ..ChatEvidence::default()
        };
        assert_eq!(
            core.poll(Duration::from_millis(2_399), observation),
            Decision::Wait
        );
        let mut stun = ChatEvidence::default();
        stun.observe("Pickpocket", "You've been stunned!", 3);
        observation.chat = stun;
        assert_eq!(
            core.poll(Duration::from_millis(2_400), observation),
            Decision::Stunned
        );
        observation.chat = ChatEvidence {
            through_sequence: 3,
            ..ChatEvidence::default()
        };
        assert_eq!(
            core.poll(Duration::from_millis(11_399), observation),
            Decision::Wait
        );
        assert_eq!(
            core.poll(Duration::from_millis(11_400), observation),
            Decision::Dispatch
        );
    }

    #[test]
    fn accepted_receipt_is_not_product_proof() {
        let mut core = core(Some(1), 2, true);
        let mut obs = observation(Some(30), Some(30));
        assert_eq!(core.poll(Duration::ZERO, obs), Decision::Dispatch);
        start(&mut core, Duration::ZERO, Some(7), obs);
        core.receipt(7, true, 4);
        obs.chat.through_sequence = 4;
        assert_eq!(core.poll(Duration::from_millis(600), obs), Decision::Wait);
        obs.target_count = Some(1);
        assert_eq!(
            core.poll(Duration::from_millis(1_200), obs),
            Decision::Complete { held: 1 }
        );
    }

    #[test]
    fn stun_locks_out_retries_until_the_bounded_window_expires() {
        let mut core = core(Some(1), 2, false);
        let mut obs = observation(Some(30), Some(30));
        assert_eq!(core.poll(Duration::ZERO, obs), Decision::Dispatch);
        start(&mut core, Duration::ZERO, None, obs);
        obs.chat.stunned = true;
        obs.chat.through_sequence = 1;
        assert_eq!(
            core.poll(Duration::from_millis(600), obs),
            Decision::Stunned
        );
        obs.chat = ChatEvidence {
            through_sequence: 1,
            ..ChatEvidence::default()
        };
        assert_eq!(core.poll(Duration::from_millis(9_599), obs), Decision::Wait);
        assert_eq!(
            core.poll(Duration::from_millis(9_600), obs),
            Decision::Dispatch
        );
    }

    #[test]
    fn fresh_failure_is_not_success_and_attempts_remain_bounded() {
        let mut core = core(None, 1, false);
        let mut obs = observation(Some(1), Some(1));
        assert_eq!(core.poll(Duration::ZERO, obs), Decision::Dispatch);
        start(&mut core, Duration::ZERO, None, obs);
        obs.chat.failure = true;
        obs.chat.through_sequence = 2;
        assert_eq!(
            core.poll(Duration::from_millis(600), obs),
            Decision::Failed(Failure::Attempts)
        );
    }

    #[test]
    fn stale_chat_modal_blocks_dispatch_and_a_fresh_page_resolves_the_attempt() {
        let mut core = core(Some(1), 2, false);
        let mut obs = observation(Some(30), Some(30));
        obs.chat_dialog_open = true;
        obs.chat_dialog_fingerprint = Some(7);
        assert_eq!(core.poll(Duration::ZERO, obs), Decision::Wait);

        obs.chat_dialog_open = false;
        obs.chat_dialog_fingerprint = None;
        assert_eq!(core.poll(Duration::ZERO, obs), Decision::Dispatch);
        start(&mut core, Duration::ZERO, None, obs);
        obs.chat_dialog_open = true;
        obs.chat_dialog_fingerprint = Some(7);
        assert_eq!(
            core.poll(Duration::from_millis(600), obs),
            Decision::Dialogue
        );
    }

    #[test]
    fn zero_goal_completes_from_observed_inventory_without_dispatch() {
        let mut core = core(Some(0), 2, true);
        assert_eq!(
            core.poll(Duration::ZERO, observation(None, None)),
            Decision::Complete { held: 0 }
        );
    }

    #[test]
    fn overall_deadline_rejects_a_late_inventory_goal() {
        let mut core = core(Some(1), 2, true);
        let mut obs = observation(Some(30), Some(30));
        obs.target_count = Some(1);
        assert_eq!(
            core.poll(DEFAULT_ACTION_DEADLINE, obs),
            Decision::Failed(Failure::Deadline)
        );
    }

    #[test]
    fn goal_cannot_hide_an_unobserved_or_rejected_dispatch() {
        let mut core = core(Some(1), 2, true);
        let mut obs = observation(Some(30), Some(30));
        start(&mut core, Duration::ZERO, Some(7), obs);
        obs.target_count = Some(1);
        assert_eq!(core.poll(Duration::from_millis(100), obs), Decision::Wait);
        core.receipt(7, false, 0);
        assert_eq!(
            core.poll(Duration::from_millis(200), obs),
            Decision::Failed(Failure::DispatchRejected)
        );
    }

    #[test]
    fn single_pick_stun_holds_the_core_until_lockout_finishes() {
        let mut core = core(None, 1, false);
        let mut obs = observation(Some(30), Some(30));
        start(&mut core, Duration::ZERO, None, obs);
        let stunned_at = Duration::from_millis(600);
        obs.chat.stunned = true;
        obs.chat.through_sequence = 1;
        assert_eq!(core.poll(stunned_at, obs), Decision::Stunned);
        obs.chat = ChatEvidence {
            through_sequence: 1,
            ..ChatEvidence::default()
        };
        assert_eq!(core.poll(stunned_at + RETRY_GAP, obs), Decision::Wait);
        assert_eq!(
            core.poll(stunned_at + STUN_LOCK, obs),
            Decision::Failed(Failure::Attempts)
        );
    }

    #[test]
    fn quest_inventory_goal_has_no_undeclared_attempt_cap() {
        let mut core = ThieveCore::new(
            Duration::ZERO,
            DEFAULT_ACTION_DEADLINE,
            Some(1),
            None,
            false,
            0,
        )
        .unwrap();
        let mut obs = observation(Some(30), Some(30));
        for attempt in 0..40 {
            let now = Duration::from_millis(attempt * 1_200);
            assert_eq!(core.poll(now, obs), Decision::Dispatch);
            start(&mut core, now, None, obs);
            obs.chat.response = true;
            obs.chat.through_sequence = i32::try_from(attempt + 1).unwrap();
            assert_eq!(
                core.poll(now + Duration::from_millis(1), obs),
                Decision::AttemptResolved
            );
            obs.chat.response = false;
        }
        obs.target_count = Some(1);
        assert_eq!(
            core.poll(Duration::from_secs(49), obs),
            Decision::Complete { held: 1 }
        );
    }
}
