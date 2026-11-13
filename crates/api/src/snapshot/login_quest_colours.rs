use super::{QuestListStatus, QuestStatusView};

const MAX_QUEST_ROWS: usize = 256;
const WORD_BITS: usize = u64::BITS as usize;
const BITSET_WORDS: usize = MAX_QUEST_ROWS / WORD_BITS;

#[derive(Debug, Default)]
pub(super) struct LoginQuestColours {
    eligible: [u64; BITSET_WORDS],
    session: Option<u64>,
    root: i32,
    count: u16,
    armed: bool,
    initialized: bool,
}

impl LoginQuestColours {
    pub(super) fn observe_session(
        &mut self,
        session: u64,
        cold_login: bool,
        player_info_since_login: u64,
    ) {
        if self.session != Some(session) {
            self.invalidate();
            self.session = Some(session);
            if cold_login && player_info_since_login <= 1 {
                self.begin_fresh_login();
            }
        }
        if self.needs_initial_observation() && player_info_since_login > 1 {
            self.invalidate();
        }
    }

    pub(super) fn begin_fresh_login(&mut self) {
        self.eligible = [0; BITSET_WORDS];
        self.root = 0;
        self.count = 0;
        self.armed = true;
        self.initialized = false;
    }

    pub(super) fn invalidate(&mut self) {
        self.eligible = [0; BITSET_WORDS];
        self.root = 0;
        self.count = 0;
        self.armed = false;
        self.initialized = false;
    }

    pub(super) const fn needs_initial_observation(&self) -> bool {
        self.armed && !self.initialized
    }

    pub(super) fn observe(&mut self, root: i32, rows: &[QuestStatusView], stable: bool) {
        if !self.armed {
            return;
        }

        if rows.len() > MAX_QUEST_ROWS {
            self.invalidate();
            return;
        }

        if self.initialized && (root != self.root || rows.len() != self.count as usize) {
            self.invalidate();
            return;
        }

        if !stable {
            return;
        }

        if !self.initialized {
            self.root = root;
            self.count = rows.len() as u16;
            self.initialized = true;
            for (index, row) in rows.iter().enumerate() {
                if row.status() == QuestListStatus::Complete {
                    self.set_eligible(index);
                }
            }
            return;
        }

        for (index, row) in rows.iter().enumerate() {
            if row.status() != QuestListStatus::Complete {
                self.clear_eligible(index);
            }
        }
    }

    pub(super) fn eligible(&self, index: usize) -> bool {
        self.armed
            && self.initialized
            && index < self.count as usize
            && self.eligible[index / WORD_BITS] & (1 << (index % WORD_BITS)) != 0
    }

    fn set_eligible(&mut self, index: usize) {
        self.eligible[index / WORD_BITS] |= 1 << (index % WORD_BITS);
    }

    fn clear_eligible(&mut self, index: usize) {
        self.eligible[index / WORD_BITS] &= !(1 << (index % WORD_BITS));
    }
}

#[cfg(test)]
mod tests {
    use super::{LoginQuestColours, MAX_QUEST_ROWS};
    use crate::snapshot::{QuestListStatus, QuestStatusView};
    use std::mem::size_of;

    const ROOT: i32 = 7;
    const RED: i32 = 0xF80000;
    const YELLOW: i32 = 0xF8F800;
    const GREEN: i32 = 0x00F800;
    const UNKNOWN: i32 = 0x123456;

    fn rows(colours: &[i32]) -> Vec<QuestStatusView> {
        colours
            .iter()
            .enumerate()
            .map(|(index, colour)| QuestStatusView {
                component_id: index as i32,
                name: String::new(),
                colour: *colour,
            })
            .collect()
    }

    #[test]
    fn partial_and_unavailable_observations_do_not_initialize_proof() {
        let mut proof = LoginQuestColours::default();
        proof.begin_fresh_login();
        let green = rows(&[GREEN]);

        proof.observe(ROOT, &green, false);
        assert!(proof.needs_initial_observation());
        assert!(!proof.eligible(0));

        proof.observe(ROOT, &green, true);
        assert!(!proof.needs_initial_observation());
        assert!(proof.eligible(0));
    }

    #[test]
    fn fresh_login_resets_prior_proof_and_requires_first_observation() {
        let mut proof = LoginQuestColours::default();
        proof.begin_fresh_login();
        proof.observe(ROOT, &rows(&[GREEN]), true);
        assert!(proof.eligible(0));

        proof.begin_fresh_login();
        assert!(proof.needs_initial_observation());
        assert!(!proof.eligible(0));
        proof.observe(ROOT, &rows(&[RED]), true);
        proof.observe(ROOT, &rows(&[GREEN]), true);
        assert!(!proof.eligible(0));
    }

    #[test]
    fn warm_session_boundary_invalidation_clears_proof() {
        let mut proof = LoginQuestColours::default();
        proof.begin_fresh_login();
        proof.observe(ROOT, &rows(&[GREEN]), true);
        proof.invalidate();
        proof.begin_fresh_login();

        assert!(!proof.eligible(0));
        assert!(proof.needs_initial_observation());
    }

    #[test]
    fn initial_non_green_and_later_non_green_never_gain_or_retain_proof() {
        let mut proof = LoginQuestColours::default();
        proof.begin_fresh_login();
        proof.observe(ROOT, &rows(&[RED, YELLOW, UNKNOWN]), true);
        proof.observe(ROOT, &rows(&[GREEN, GREEN, GREEN]), true);
        assert!(!proof.eligible(0));
        assert!(!proof.eligible(1));
        assert!(!proof.eligible(2));

        proof.begin_fresh_login();
        proof.observe(ROOT, &rows(&[GREEN]), true);
        assert!(proof.eligible(0));
        proof.observe(ROOT, &rows(&[YELLOW]), true);
        proof.observe(ROOT, &rows(&[GREEN]), true);
        assert!(!proof.eligible(0));
    }

    #[test]
    fn root_or_count_change_invalidates_all_proof_until_fresh_login() {
        let mut proof = LoginQuestColours::default();
        proof.begin_fresh_login();
        proof.observe(ROOT, &rows(&[GREEN]), true);
        proof.observe(ROOT + 1, &rows(&[GREEN]), true);
        assert!(!proof.eligible(0));
        assert!(!proof.needs_initial_observation());

        proof.begin_fresh_login();
        proof.observe(ROOT, &rows(&[GREEN]), true);
        proof.observe(ROOT, &rows(&[GREEN, GREEN]), true);
        assert!(!proof.eligible(0));
        assert!(!proof.eligible(1));
    }

    #[test]
    fn only_a_new_cold_session_arms_proof() {
        let mut proof = LoginQuestColours::default();
        let green = rows(&[GREEN]);
        proof.observe_session(1, true, 1);
        proof.observe(ROOT, &green, true);
        assert!(proof.eligible(0));

        proof.observe(ROOT + 1, &green, true);
        proof.observe_session(1, true, 1);
        proof.observe(ROOT, &green, true);
        assert!(!proof.eligible(0));

        proof.observe_session(2, false, 1);
        proof.observe(ROOT, &green, true);
        assert!(!proof.eligible(0));

        proof.observe_session(3, true, 1);
        proof.observe(ROOT, &green, true);
        assert!(proof.eligible(0));
    }

    #[test]
    fn missing_the_first_player_info_does_not_prove_login_colour() {
        let mut proof = LoginQuestColours::default();
        let green = rows(&[GREEN]);
        proof.observe_session(1, true, 2);
        proof.observe(ROOT, &green, true);
        assert!(!proof.eligible(0));

        proof.observe_session(2, true, 0);
        proof.observe(ROOT, &green, false);
        proof.observe_session(2, true, 2);
        proof.observe(ROOT, &green, true);
        assert!(!proof.eligible(0));

        proof.observe_session(3, true, 1);
        proof.observe(ROOT, &green, true);
        proof.observe_session(3, true, 50);
        assert!(proof.eligible(0));
    }

    #[test]
    fn overflow_fails_closed_and_indices_are_bounded() {
        let mut proof = LoginQuestColours::default();
        proof.begin_fresh_login();
        let oversized = rows(&vec![GREEN; MAX_QUEST_ROWS + 1]);
        proof.observe(ROOT, &oversized, true);

        assert!(!proof.needs_initial_observation());
        assert!(!proof.eligible(0));
        assert!(!proof.eligible(MAX_QUEST_ROWS));
    }

    #[test]
    fn maximum_row_count_fits_fixed_bitset() {
        let mut proof = LoginQuestColours::default();
        proof.begin_fresh_login();
        let full = rows(&vec![GREEN; MAX_QUEST_ROWS]);
        proof.observe(ROOT, &full, true);

        assert!(proof.eligible(MAX_QUEST_ROWS - 1));
        assert!(!proof.eligible(MAX_QUEST_ROWS));
        // Measured 56 B plus the default 64 B headroom (AGENTS.md tripwire rule).
        assert!(size_of::<LoginQuestColours>() <= 120);
    }

    #[test]
    fn complete_status_matches_only_green_colour() {
        assert_eq!(rows(&[GREEN])[0].status(), QuestListStatus::Complete);
        assert_ne!(rows(&[RED])[0].status(), QuestListStatus::Complete);
    }
}
