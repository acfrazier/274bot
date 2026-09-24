use super::*;


/// The selected talk step an identified membership row owns: the
/// `talk_key.talk` row whose own id is the row's, or `None` when the row is not
/// a talk membership. The key keepers, the puzzle-box extras and every other
/// identified row are not this arm's, and nothing is ever played as if they
/// were.
pub(super) fn talk_step(selected: Option<&SelectedGameData>, id: i32) -> Option<&TalkKeyTalkRow> {
    selected?.talk_key()?.talk.iter().find(|talk| talk.id == id)
}

/// The challenge seam: `identify_step` reads no challenge id, so the page the
/// server leaves once a parent clue swaps for its own challenge scroll is
/// `none-held`. A selected `challenge_answers` id with a positive posted count
/// joins its **parent** talk membership row through the `_challenge` strip, and
/// the parent — never the scroll id — is the step the token keeps.
///
/// The first posted selected id wins, so two scrolls joined by one page resolve
/// in posted order. Nothing else is admitted: an empty page, a zero count, an
/// id no selected challenge answers, a strip that names no selected row, and a
/// parent the talk family does not publish are all `none-held`.
pub(super) fn challenge_step<'a>(
    selected: Option<&'a SelectedGameData>,
    input: &Value,
) -> Option<&'a TrailMembershipRow> {
    let facts = selected?.trails()?;
    for (id, count) in posted_page(input) {
        if count <= 0 {
            continue;
        }
        let Some(challenge) = facts
            .challenge_answers
            .iter()
            .find(|challenge| challenge.id == id)
        else {
            continue;
        };
        let Some(alias) = challenge.alias.strip_suffix(CHALLENGE_SUFFIX) else {
            continue;
        };
        let Some(parent) = facts.rows.iter().find(|row| row.alias == alias) else {
            continue;
        };
        if talk_step(selected, parent.id).is_none() {
            continue;
        }
        return Some(parent);
    }
    None
}

/// The selected answer this talk step's own challenge scroll carries: the
/// `challenge_answers` row whose `_challenge`-stripped alias is the step's own
/// alias, parsed as the non-negative `i32` the count dialog wants.
///
/// `None` when the step has no challenge, and `None` when the selected string
/// is not one — an unparsed, negative or absent answer never clicks, so no
/// count is ever invented.
pub(super) fn challenge_answer(selected: Option<&SelectedGameData>, talk: &TalkKeyTalkRow) -> Option<i32> {
    for challenge in &selected?.trails()?.challenge_answers {
        if challenge.alias.strip_suffix(CHALLENGE_SUFFIX) != Some(talk.alias.as_str()) {
            continue;
        }
        let Ok(value) = challenge.answer.parse::<i32>() else {
            continue;
        };
        if value >= 0 {
            return Some(value);
        }
    }
    None
}
impl ClueRuntime {

    /// `Steady` on an identified talk membership: the selected
    /// `talk_key.talk` row this held step owns, walked to and then Talk-to'd,
    /// one verb per call.
    ///
    /// The membership is the caller's, resolved from the selected `talk_key.talk`
    /// family alone: a key keeper the family does not publish never reaches this
    /// arm, so its `wait` is never a key hunt's fallthrough and no keeper is
    /// ever Talked-to.
    ///
    /// The arm opens on the posted chat facts, because an open chat is not a
    /// tick to Talk-to again: a posted `count_dialog_open` is answered — with
    /// this step's own selected challenge answer and nothing else — and a
    /// landed `dialog_ready` waits the tick out. An unobserved slot is neither.
    ///
    /// A step with a published spawn walks to that `{x, z, plane}` tile and
    /// Talks-to only a posted npc of this step's identity standing on it; the
    /// steps without one take the nearest posted npc of that identity. Either
    /// way a page with no posted match, no posted `here`, no talk action or no
    /// postable identity is a `wait` with the token live: nothing is invented,
    /// nothing is Cleared, and no second target is chased.
    pub(super) fn talk(
        &mut self,
        talk: &TalkKeyTalkRow,
        input: &Value,
        selected: Option<&SelectedGameData>,
    ) -> Value {
        if count_open(input) {
            // The count dialog this step's own challenge scroll opens: the
            // selected answer, never a computed or remembered one. A step the
            // pin answered nothing for waits the tick out.
            return match challenge_answer(selected, talk) {
                Some(value) => json!({
                    "kind": "answer-count",
                    "token": self.token,
                    "value": value,
                }),
                None => self.emit("wait"),
            };
        }
        if dialog_ready(input) {
            // Owned open chat: no walk and no Talk-to goes out behind it, the
            // same way the landed dialog sequencer refuses to press a second
            // option. The token lives and the next call re-reads the page.
            return self.emit("wait");
        }
        let Some(page) = input.get("npcs").and_then(Value::as_array) else {
            // No posted npc page this call: no spawn can be observed, so there
            // is nothing to Talk-to.
            return self.emit("wait");
        };
        match &talk.spawn {
            Some(spawn) => {
                let tile = Tile {
                    x: spawn.x,
                    z: spawn.z,
                    level: spawn.plane,
                };
                match arrival(tile, input) {
                    Arrival::Unknown => self.emit("wait"),
                    Arrival::Walking => self.walk(tile),
                    Arrival::Arrived => {
                        let npc = NpcIdentity::Talk {
                            id: talk.npc.id,
                            name: &talk.npc.name,
                        };
                        match pick_at_spawn(&npc, page, tile) {
                            Some(pick) => self.npc_verb(&pick),
                            // Arrived with no posted row of this identity on the
                            // tile: stay there and wait, never chase a wanderer and
                            // never take a second target.
                            None => self.emit("wait"),
                        }
                    }
                }
            }
            None => {
                let Some(here) = posted_here(input) else {
                    // No posted tile: there is no arrival claim to make and no
                    // distance to measure a posted row by, so this tick waits
                    // rather than walking blind.
                    return self.emit("wait");
                };
                let npc = NpcIdentity::Talk {
                    id: talk.npc.id,
                    name: &talk.npc.name,
                };
                match pick_talk(&npc, page, here) {
                    Some(pick) if pick.distance <= i64::from(ARRIVE_RADIUS) => self.npc_verb(&pick),
                    // Posted but out of reach: walk to the row's own posted
                    // tile and re-pick from the arrival. A row that posted no
                    // tile is unmeasured, so this tick waits.
                    Some(pick) => match pick.tile {
                        Some(tile) => self.walk(tile),
                        None => self.emit("wait"),
                    },
                    None => self.emit("wait"),
                }
            }
        }
    }
}
