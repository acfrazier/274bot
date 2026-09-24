//! Identify the first held trail step over the posted pack page: a casket pass
//! then a clue pass, both in posted order. Takes the family directly so a
//! missing family is not an empty page.
//! The V8 wrapper is the only production caller.

use crate::clue_facts::TRAILS_UNAVAILABLE;
use crate::game_data::{TrailFacts, TrailMembershipRow};

/// Nothing held. A named error, never `ok` null: an append-only page of
/// unrelated items is a normal state, not a missed row.
pub const NONE_HELD: &str = "none-held";

/// The two decoder roles in the order they win. A clue earlier in the pack
/// does not beat a later casket.
const ROLES: [&str; 2] = ["casket", "clue"];

/// The first held step: first held casket, else first held clue, else
/// `none-held`. `held` is the posted `(obj id, count)` page in posted order,
/// which is the only held order — the membership table is catalog order.
///
/// A pair is held only when `count > 0`, and only an `i32` id equal to a row
/// id holds: no name, noted, cert, or alias match. A held id that is not a
/// membership row is skipped, not `unknown-id`; the challenge answers stay
/// unread. Duplicate ids would resolve to the first row with that id and the
/// role of the pass.
pub fn identify_step<'a>(
    held: &[(i32, i32)],
    facts: Option<&'a TrailFacts>,
) -> Result<&'a TrailMembershipRow, &'static str> {
    let Some(facts) = facts else {
        return Err(TRAILS_UNAVAILABLE);
    };
    for role in ROLES {
        for (id, count) in held {
            if *count <= 0 {
                continue;
            }
            if let Some(row) = facts
                .rows
                .iter()
                .find(|row| row.id == *id && row.role == role)
            {
                return Ok(row);
            }
        }
    }
    Err(NONE_HELD)
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;

    const CASKET: i32 = 3531;
    const CLUE: i32 = 3554;

    fn facts(rev: ClientRevision) -> std::sync::Arc<crate::game_data::SelectedGameData> {
        crate::game_data::for_revision(rev).expect("selected data")
    }

    #[test]
    fn identify_step_takes_a_casket_that_is_later_in_the_pack() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let data = facts(rev);
            let trails = data.trails();
            let page = [(CLUE, 1), (CASKET, 1)];
            let row = identify_step(&page, trails).expect("casket");
            assert_eq!(row.id, CASKET, "{rev:?}");
            assert_eq!(row.role, "casket", "{rev:?}");
            assert_eq!(row.alias, "trail_clue_hard_sextant016_casket", "{rev:?}");
            assert!(row.params.is_empty(), "{rev:?}");
            // The easy-map clue is not a casket even though it names one.
            let page = [(2713, 1), (2714, 1)];
            assert_eq!(identify_step(&page, trails).expect("casket").id, 2714);
        }
    }

    #[test]
    fn identify_step_uses_posted_order_not_table_order() {
        // 3531 sorts before 3554 and follows 2714 in the table: the first held
        // casket on the page wins even when a later casket is the lower id.
        let data = facts(ClientRevision::R274);
        let trails = data.trails();
        let page = [(999_999, 1), (CASKET, 1), (2714, 1)];
        assert_eq!(identify_step(&page, trails).expect("casket").id, CASKET);
        // The clue pass is posted order too.
        let page = [(999_999, 1), (CLUE, 1), (2713, 1)];
        assert_eq!(identify_step(&page, trails).expect("clue").id, CLUE);
    }

    #[test]
    fn identify_step_skips_unheld_and_non_membership_pairs() {
        let data = facts(ClientRevision::R274);
        let trails = data.trails().expect("trails");
        // Zero and negative counts are not held, so the pair after them wins.
        for count in [0, -1] {
            let page = [(CASKET, count), (CLUE, 1)];
            assert_eq!(identify_step(&page, Some(trails)).expect("clue").id, CLUE);
        }
        // Noted, cert, alias, and name are not identity: a held id that is not
        // a row is skipped, and the six challenge rows are not membership.
        let challenge = trails.challenge_answers.first().expect("challenge row").id;
        let page = [(challenge, 1), (0, 1), (CASKET, 0), (CLUE, 1)];
        assert_eq!(identify_step(&page, Some(trails)).expect("clue").id, CLUE);
        // Nothing held, nothing held-but-unrelated, and an empty page are all
        // the same named error.
        for page in [
            Vec::new(),
            vec![(challenge, 1)],
            vec![(0, 5)],
            vec![(CASKET, 0)],
        ] {
            assert_eq!(identify_step(&page, Some(trails)).err(), Some(NONE_HELD));
        }
    }

    #[test]
    fn identify_step_absent_family_is_not_an_empty_page() {
        assert_eq!(identify_step(&[], None).err(), Some(TRAILS_UNAVAILABLE));
        assert_eq!(
            identify_step(&[(CASKET, 1)], None).err(),
            Some(TRAILS_UNAVAILABLE)
        );
        assert_eq!(
            identify_step(&[], None).err(),
            Some("family-unavailable:trails")
        );
    }

    #[test]
    fn identify_step_returns_the_landed_row_for_the_constrained_clue() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let data = facts(rev);
            let row = identify_step(&[(CLUE, 1)], data.trails()).expect("clue");
            assert_eq!(row.id, CLUE, "{rev:?}");
            assert_eq!(row.alias, "trail_clue_hard_sextant028", "{rev:?}");
            assert_eq!(row.role, "clue", "{rev:?}");
            assert_eq!(row.params.len(), 4, "{rev:?}");
            assert_eq!(row.access.as_deref(), Some("constrained"), "{rev:?}");
        }
    }
}
