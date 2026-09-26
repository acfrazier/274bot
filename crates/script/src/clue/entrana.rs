use super::*;

/// The cap's Entrana box: membership is the identified row's own selected
/// `trail_coord`, decoded the landed way, inside this square on level 0 — the
/// frozen `isEntranaClueCoord` and never a copied `CLUE_DB`. The one selected
/// member is `trail_clue_hard_riddle027` (`3579`), whose `0_44_52_2_23`
/// decodes to `(2818, 3351, 0)`.
pub(super) const ENTRANA_X_MIN: i32 = 2_802;
pub(super) const ENTRANA_X_MAX: i32 = 2_878;
pub(super) const ENTRANA_Z_MIN: i32 = 3_329;
pub(super) const ENTRANA_Z_MAX: i32 = 3_393;
pub(super) const ENTRANA_LEVEL: i32 = 0;

/// The frozen `DDS_IDS` the strip unequips like any other restricted name but
/// never lists: the two hard-trail daggers come back with the hard kit, and
/// nothing here stocks a weapon.
pub(super) const DDS_IDS: [i32; 2] = [1231, 1215];

/// The frozen `ENTRANA_RESTRICTED_GEAR_RE` alternatives, ported whole and in
/// the frozen order — the same class of matcher the landed `keep` predicate is,
/// over **posted display names** and never a selected item family (schema 4
/// carries none, and none is invented). The three shapes the plain list cannot
/// spell are their own reads below: the `two.handed` wildcard, the
/// `gauntlets?` optional `s` (both spellings listed, which is the same language
/// under `\b`), and the `body(?!\s+rune\b)` lookahead.
pub(super) const ENTRANA_GEAR_WORDS: [&str; 57] = [
    "sword",
    "dagger",
    "scimitar",
    "longsword",
    "2h",
    "mace",
    "warhammer",
    "battleaxe",
    "axe",
    "pickaxe",
    "spear",
    "hasta",
    "halberd",
    "maul",
    "claws",
    "whip",
    "bow",
    "shortbow",
    "longbow",
    "crossbow",
    "javelin",
    "dart",
    "thrownaxe",
    "knife",
    "staff",
    "wand",
    "battlestaff",
    "cannon",
    "helmet",
    "full helm",
    "med helm",
    "coif",
    "platebody",
    "chainbody",
    "platelegs",
    "plateskirt",
    "skirt of",
    "kiteshield",
    "square shield",
    "sq shield",
    "dragon square",
    "god cape",
    "fire cape",
    "obsidian cape",
    "defender",
    "chaps",
    "vambraces",
    "gauntlet",
    "gauntlets",
    "gloves",
    "shield",
    "cape",
    "cloak",
    "snelm",
    "cowl",
    "hat",
    "hood",
];

/// The frozen `Withdraw-1` label the restore's claim rides; the bank row's own
/// posted action slots are matched by the host and never by this machine.
pub(super) const WITHDRAW_ONE: &str = "Withdraw-1";

/// The caps' own name for a bank approach that did not come up — the frozen
/// `walk to the bank failed — gear stays banked, will retry` line, logged with
/// this token and never a machine kind.
pub(super) const RESTORE_WALK_FAILED: &str = "restore-walk-failed";

/// The caps' own name for a name that would not go back on — the frozen
/// `could not re-equip … — will retry` line, logged with this token and never a
/// machine kind. `supplies-needed` stays the arrived dig's own wait-class.
pub(super) const RESTORE_INCOMPLETE: &str = "restore-incomplete";

/// The frozen `still holding Entrana-banned gear after bank prep — will retry`
/// line: the strip's own give-up, logged while the listed names stay listed and
/// the row's own arms wait for the next attempt rather than walking with the
/// gear in hand.
pub(super) const STILL_HOLDING: &str =
    "still holding Entrana-banned gear after bank prep — will retry";

/// The machine's own window for one Entrana bank approach or one strip: how
/// long the walk to the nearest stand, the posted booth's own open and one
/// deposit or claim have to settle before the attempt logs its named failure
/// and re-arms. Armed per attempt, never once per trail, and freeze-honored
/// like every other window here, so a frozen session never spends it.
pub(super) const ENTRANA_WAIT_MS: u64 = 30_000;

/// The frozen `ENTRANA_RESTRICTED_GEAR_RE` match over one posted display name:
/// the alternatives under their own `\b…\b`, ASCII-folded, exactly as the
/// frozen regex spells them. The vectors the frozen `entranaGear.test.ts`
/// binds are the whole of it: `Dragonhide body`, `Coif` and `Dragon dagger(p)`
/// are refused, and `Amulet of glory`, `Rune arrow`, `Body rune`, `Shark`,
/// `Clue scroll`, `Spade`, `Sextant` and `Coins` are let through.
pub(super) fn entrana_restricted_gear(name: &str) -> bool {
    let folded = name.to_ascii_lowercase();
    ENTRANA_GEAR_WORDS
        .iter()
        .any(|word| word_hit(&folded, word))
        || two_handed_hit(&folded)
        || body_hit(&folded)
}

/// One frozen alternative under its own `\b…\b`: the word occurs with a
/// non-word byte — or the name's own start or end — on both sides.
pub(super) fn word_hit(name: &str, word: &str) -> bool {
    let bytes = name.as_bytes();
    let mut from = 0;
    while let Some(at) = name[from..].find(word) {
        let start = from + at;
        let end = start + word.len();
        if boundary_before(bytes, start) && boundary_after(bytes, end) {
            return true;
        }
        from = start + 1;
    }
    false
}

/// The frozen `two.handed` alternative: `two`, exactly one byte, `handed`, all
/// of it under the same word boundaries. `Two-handed` is the spelled form and
/// `twohanded` is not this alternative — the `.` is one byte and not zero.
pub(super) fn two_handed_hit(name: &str) -> bool {
    let bytes = name.as_bytes();
    let mut from = 0;
    while let Some(at) = name[from..].find("two") {
        let start = from + at;
        let end = start + 4 + "handed".len();
        if end <= bytes.len()
            && &bytes[start + 4..end] == b"handed"
            && boundary_before(bytes, start)
            && boundary_after(bytes, end)
        {
            return true;
        }
        from = start + 1;
    }
    false
}

/// The frozen `body(?!\s+rune\b)` alternative: the word `body` under its own
/// boundaries and not followed by whitespace and then the word `rune`. `Body
/// rune` is the composed rune and is let through; `Body runes` and `Rune body`
/// are not.
pub(super) fn body_hit(name: &str) -> bool {
    let bytes = name.as_bytes();
    let mut from = 0;
    while let Some(at) = name[from..].find("body") {
        let start = from + at;
        let end = start + "body".len();
        if boundary_before(bytes, start)
            && boundary_after(bytes, end)
            && !rune_word_after(name, end)
        {
            return true;
        }
        from = start + 1;
    }
    false
}

/// The `(?!\s+rune\b)` lookahead over the rest of the name: whitespace, then
/// the word `rune`. A rest with no whitespace at all, or with anything but
/// `rune` after it, is not this lookahead.
pub(super) fn rune_word_after(name: &str, at: usize) -> bool {
    let rest = &name[at..];
    let trimmed = rest.trim_start();
    if trimmed.len() == rest.len() {
        return false;
    }
    let Some(tail) = trimmed.strip_prefix("rune") else {
        return false;
    };
    !tail.starts_with(|ch: char| ch.is_ascii_alphanumeric() || ch == '_')
}

/// The frozen `\b`'s own `\w`: ASCII alphanumerics and the underscore. A byte
/// that is none of those is a boundary, a non-ASCII byte included — the frozen
/// regex is a JavaScript one without the `u` flag.
pub(super) fn word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

pub(super) fn boundary_before(bytes: &[u8], at: usize) -> bool {
    at == 0 || !word_byte(bytes[at - 1])
}

pub(super) fn boundary_after(bytes: &[u8], at: usize) -> bool {
    at >= bytes.len() || !word_byte(bytes[at])
}

/// Whether the identified row arms the strip: the row's own selected
/// `trail_coord`, decoded by the landed decoder, inside the cap's box on level
/// 0 — and never a casket. The param is the whole membership: no copied
/// `CLUE_DB` coordinate, no frozen coordinate table and no second classify.
pub(super) fn entrana_coord(row: &TrailMembershipRow) -> bool {
    if row.role == CASKET_ROLE {
        return false;
    }
    let coord = row
        .params
        .iter()
        .find(|param| param.key == "trail_coord")
        .map(|param| param.value.as_str());
    let Some(tile) = coord.and_then(decode_trail_coord) else {
        return false;
    };
    tile.level == ENTRANA_LEVEL
        && (ENTRANA_X_MIN..=ENTRANA_X_MAX).contains(&tile.x)
        && (ENTRANA_Z_MIN..=ENTRANA_Z_MAX).contains(&tile.z)
}

/// One posted worn row as this machine reads it: the display name the frozen
/// matcher folds and the packed id the DDS exclusion joins. A row the page
/// posted without a name is not a worn row here — the matcher is names, and no
/// id is ever folded into one.
pub(super) struct Worn<'a> {
    pub(super) name: &'a str,
    pub(super) id: Option<i32>,
}

/// The first posted worn row this call's page carries whose name the frozen
/// matcher folds, in posted order. The page is the wrapper's marshalling of
/// `host().snapshot.equipment`, the raw rows with their own `slot`, and never
/// `Equipment.items()` — which drops the slot — and never a scan of the item
/// table.
pub(super) fn pick_worn(input: &Value) -> Option<Worn<'_>> {
    input
        .get("equipment")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|row| {
            let name = row.get("name").and_then(Value::as_str)?;
            entrana_restricted_gear(name).then_some(Worn {
                name,
                id: posted_i32(row, "id"),
            })
        })
}

/// The first posted pack row whose display name the frozen matcher folds: the
/// strip's deposit candidate, listed or not. The row is the frozen
/// `depositAllMatching`'s own read of the pack — a positive count and the
/// display name — and nothing about which pass put it there is consulted, so
/// an unequipped helm, a dagger id that is never listed and a restricted item
/// the player was carrying are all the same candidate.
pub(super) fn pick_pack_restricted(input: &Value) -> Option<String> {
    input
        .get("inv")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|row| {
            let name = row.get("name").and_then(Value::as_str)?;
            let count = posted_i32(row, "count")?;
            (count > 0 && entrana_restricted_gear(name)).then(|| name.to_string())
        })
}

/// The frozen make-room deposit's own row pick over this call's posted pack
/// page: the first row with a posted non-zero count, a posted name and a posted
/// id the selected trail facts do not name, that is not one of the restore's
/// own listed names and not a row this attempt has already tried.
///
/// The identity the frozen predicate protects is `CLUE_DB[id]` and
/// `CASKET_IDS[id]`: the selected `trails` facts are this revision's own answer
/// to both — the membership rows are the clue scrolls, the caskets and the
/// probes, and the `challenge_answers` rows are the challenge scrolls — so a
/// clue the trail still owns is never banked here. A row the page posted no id
/// for is not a row this deposit can identify, and it is skipped rather than
/// banked blind. No selected data at all is the same refusal: this machine
/// makes no room it cannot check.
pub(super) fn make_room_row(
    selected: Option<&SelectedGameData>,
    input: &Value,
    listed: &[String],
    tried: &[String],
) -> Option<String> {
    let facts = selected?.trails()?;
    input
        .get("inv")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|row| {
            let name = row.get("name").and_then(Value::as_str)?;
            let id = posted_i32(row, "id")?;
            if posted_i32(row, "count")? <= 0 {
                return None;
            }
            if listed
                .iter()
                .any(|listed| listed.eq_ignore_ascii_case(name))
            {
                return None;
            }
            if tried.iter().any(|tried| tried.eq_ignore_ascii_case(name)) {
                return None;
            }
            if facts.rows.iter().any(|row| row.id == id) {
                return None;
            }
            if facts.challenge_answers.iter().any(|row| row.id == id) {
                return None;
            }
            Some(name.to_string())
        })
}

/// Whether this call's posted pack page holds a row with this display name and
/// a positive count: the observation the strip's deposit and the restore's
/// claim both read. A page that posted no such row holds nothing.
pub(super) fn pack_holds_name(input: &Value, name: &str) -> bool {
    input
        .get("inv")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter().any(|row| {
                row.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|posted| posted.eq_ignore_ascii_case(name))
                    && posted_i32(row, "count").is_some_and(|count| count > 0)
            })
        })
}

/// Whether this call's posted worn page holds a row with this display name: the
/// restore's own completion read, one listed name at a time.
pub(super) fn worn_name(input: &Value, name: &str) -> bool {
    input
        .get("equipment")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter().any(|row| {
                row.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|posted| posted.eq_ignore_ascii_case(name))
            })
        })
}

/// The posted nearest Use-quickly booth: the identity the strip's and the
/// restore's own `open-booth` click rides, exactly as the wrapper marshalled
/// it. A page that posted no booth has nothing to open, and no tile is invented
/// for one.
pub(super) struct Booth<'a> {
    pub(super) x: i32,
    pub(super) z: i32,
    pub(super) level: i32,
    pub(super) id: i32,
    pub(super) name: Option<&'a str>,
    pub(super) action: Option<&'a str>,
}

pub(super) fn nearest_booth(input: &Value) -> Option<Booth<'_>> {
    let row = input.get("nearest_booth")?;
    Some(Booth {
        x: posted_i32(row, "x")?,
        z: posted_i32(row, "z")?,
        level: posted_i32(row, "level")?,
        id: posted_i32(row, "id")?,
        name: row.get("name").and_then(Value::as_str),
        action: row.get("op").and_then(Value::as_str),
    })
}

/// Whether this call's posted `here` stands on the posted booth's own level
/// inside the frozen `ARRIVE_RADIUS`: the same arrival rule every walk arm here
/// reads, applied to the booth page rather than to a decoded tile.
pub(super) fn booth_arrival(booth: &Booth<'_>, input: &Value) -> Arrival {
    let Some(here) = input.get("here").and_then(posted_tile) else {
        return Arrival::Unknown;
    };
    let tile = Tile {
        x: booth.x,
        z: booth.z,
        level: booth.level,
    };
    if here.level != booth.level || chebyshev(here, tile) > i64::from(ARRIVE_RADIUS) {
        Arrival::Walking
    } else {
        Arrival::Arrived
    }
}

/// This call's posted bank interface. Only a posted `true` is an open one: an
/// omitted slot is unobserved rather than closed, so no deposit or claim is
/// ever sent on an invented open bank.
pub(super) fn bank_open(input: &Value) -> bool {
    input.get("bank_open").and_then(Value::as_bool) == Some(true)
}

/// The landed `unequip` step: the host's worn-row `Remove` by resolved display
/// name, which is the only verb that can take a worn restricted row off. The
/// landed `wear` resolves inventory rows alone, so it can put a name back on
/// and can never take one off. No row id and no slot: the host resolves the
/// name it is handed.
pub(super) fn unequip_verb(name: &str, token: u64) -> Value {
    json!({ "kind": "unequip", "token": token, "name": name })
}

/// The landed `wear` step: the landed equip-from-pack verb the shim's own
/// `Equipment.equip` queues, and the one the restore's wear-back rides. No row
/// id and no slot: the host resolves the name it is handed.
pub(super) fn wear_verb(name: &str, token: u64) -> Value {
    json!({ "kind": "wear", "token": token, "name": name })
}

/// The landed `deposit` step, one restricted name at a time: the frozen
/// `Bank.depositAllMatching` cut to the regex-matching names this strip put in
/// the pack — and to any other regex-matching row the pack holds — and never
/// the ordinary loot deposit.
pub(super) fn deposit_verb(name: &str, token: u64) -> Value {
    json!({ "kind": "deposit", "token": token, "name": name })
}

/// The landed `withdraw` step for one missing listed name: the frozen
/// `Bank.withdraw(name, 'Withdraw-1')`, one chunk of one.
pub(super) fn withdraw_verb(name: &str, token: u64) -> Value {
    json!({ "kind": "withdraw", "token": token, "name": name, "action": WITHDRAW_ONE })
}

/// The landed `walk-nearest-bank` step: Rust picks the stand from the packed
/// world, so this machine sends no tile of its own and never a frozen one.
pub(super) fn walk_nearest_bank_verb(token: u64) -> Value {
    json!({ "kind": "walk-nearest-bank", "token": token })
}

/// The landed `close` step: the open bank interface's own close, once the
/// strip's deposit or the restore's claim is done with it.
pub(super) fn close_verb(token: u64) -> Value {
    json!({ "kind": "close", "token": token })
}

/// The landed `open-booth` step over the posted booth: its own tile, id and —
/// when the page posted them — name and action, the way the landed bank
/// helpers enqueue it. Nothing about it is invented and no stand is picked
/// here.
pub(super) fn open_booth_verb(booth: &Booth<'_>, token: u64) -> Value {
    let mut step = json!({
        "kind": "open-booth",
        "token": token,
        "x": booth.x,
        "z": booth.z,
        "level": booth.level,
        "id": booth.id,
    });
    if let Some(name) = booth.name {
        step["name"] = json!(name);
    }
    if let Some(action) = booth.action {
        step["action"] = json!(action);
    }
    step
}

/// The frozen `could not re-equip … — will retry` line, named the way the caps
/// do (`restore-incomplete`): a log through `callback.log`, never a machine
/// kind, and the names it carries stay listed.
pub(super) fn restore_incomplete(names: &str) -> String {
    format!("{RESTORE_INCOMPLETE}: could not re-equip {names} — will retry")
}

/// The frozen `walk to the bank failed — gear stays banked, will retry` line,
/// named the way the caps do (`restore-walk-failed`): a log through
/// `callback.log`, never a machine kind.
pub(super) fn restore_walk_failed() -> String {
    format!("{RESTORE_WALK_FAILED}: the bank did not come up — will retry")
}
impl ClueRuntime {
    /// The Entrana restore the collect's exit owes while `strippedGear` is
    /// non-empty: the frozen `restoreStrippedGear`, one verb per call over this
    /// call's posted pages.
    ///
    /// In order: every listed name this call's posted worn page already shows
    /// is done with; every listed name that page does not show but the posted
    /// pack holds goes back on (`wear`, the landed equip-from-pack verb); and
    /// what is left is claimed at the bank — the walk to the nearest stand, the
    /// posted booth's own `open-booth`, the frozen make-room deposit while the
    /// pack is too full to take the claim, one `Withdraw-1` per missing name,
    /// and the interface's close — after which the wear pass runs again. The
    /// read is by display name, the way the frozen `Equipment.contains` and
    /// `Bank.withdraw` read it, and by nothing else.
    ///
    /// `None` is the restore's own completion: the list is empty and the
    /// caller's next step may go out. One attempt is bounded by this machine's
    /// own `ENTRANA_WAIT_MS` window; past it the names that are still not back
    /// on are the named `restore-incomplete` log and the next call starts a
    /// fresh attempt, so a name that will not go back on stays listed, is never
    /// a machine kind, and never lets the finish latch report the trail solved.
    pub(super) fn restore(
        &mut self,
        selected: Option<&SelectedGameData>,
        input: &Value,
    ) -> Option<Value> {
        if self.stripped.is_empty() {
            return None;
        }
        // Every listed name is worn again: the frozen list empties and the
        // caller's own completion step may go out. Read before the attempt, so
        // a page that came back on its own needs no verb at all.
        if self.stripped.iter().all(|name| worn_name(input, name)) {
            self.stripped.clear();
            self.restore = None;
            self.clock.deadline = None;
            return None;
        }
        let mut state = match self.restore.take() {
            Some(state) => state,
            None => {
                // A fresh attempt: the previous one's window is not this one's.
                self.clock.arm(ENTRANA_WAIT_MS);
                Restore::default()
            }
        };
        // The wear pass: a listed name the posted worn page does not show but
        // the posted pack holds goes back on now, one per call — and only after
        // this attempt's own bank interface is closed, so nothing is equipped
        // behind an open bank. A wear verb the page never settles is the whole
        // attempt's window running out: the frozen `could not re-equip … — will
        // retry` over the names that are still not back on.
        if let Some(name) = self
            .stripped
            .iter()
            .find(|name| !worn_name(input, name) && pack_holds_name(input, name))
            .cloned()
        {
            if state.bank.opened && bank_open(input) {
                self.restore = Some(state);
                return Some(close_verb(self.token));
            }
            if self.clock.bound_reached() {
                return Some(self.incomplete(input));
            }
            self.restore = Some(state);
            return Some(wear_verb(&name, self.token));
        }
        // The claims: the names this call's posted pack page does not hold
        // either. One in flight at a time — a claim this call no longer reads as
        // missing has landed, and one that is still missing was not observed to
        // land, so this attempt gives up on that name. The next attempt starts
        // with an empty tried list and claims it again.
        let missing = |name: &String| !worn_name(input, name) && !pack_holds_name(input, name);
        if let Some(sent) = state.sent.clone() {
            if !missing(&sent) {
                state.sent = None;
            } else {
                state.tried.push(sent);
                state.sent = None;
            }
        }
        let claim = self
            .stripped
            .iter()
            .find(|name| missing(name) && !state.tried.contains(name))
            .cloned();
        match claim {
            Some(name) => {
                // The bank this claim needs is the shared approach: the walk to
                // the nearest stand, the posted booth's own open, and then the
                // claim itself.
                if let Some(step) = self.bank_approach(&mut state.bank, input) {
                    self.restore = Some(state);
                    return Some(step);
                }
                // The frozen make-room deposit, between the open bank and the
                // claim: a pack that cannot take the withdrawn name banks what
                // the frozen predicate takes first, so a pack that filled up
                // over the trail does not make the reclaim wait forever.
                if let Some(step) = self.make_room(selected, &mut state, input) {
                    self.restore = Some(state);
                    return Some(step);
                }
                state.sent = Some(name.clone());
                self.restore = Some(state);
                Some(withdraw_verb(&name, self.token))
            }
            None if bank_open(input) => {
                // Nothing left to claim and the interface is still up: the close
                // is this call's verb, and the names that did not come back are
                // read on the call after it.
                self.restore = Some(state);
                Some(close_verb(self.token))
            }
            // Nothing left to claim with the interface down: the attempt is
            // over and the names that are still missing are the named log.
            None => Some(self.incomplete(input)),
        }
    }

    /// The frozen `restoreStrippedGear`'s own make-room deposit, between the
    /// open bank and the claim: while this call's posted pack has fewer free
    /// slots than the listed names its pages do not hold, one posted pack row
    /// the frozen predicate takes goes to the bank, so the gear about to be
    /// claimed has somewhere to land. Without it a pack that filled up over the
    /// trail never takes a claim: the recoverable path that reaches the exit
    /// with a full pack is a real one — the casket reward fills it — and the
    /// reclaim would retry against the same full pack forever, holding the
    /// finish latch and starving the sibling grind.
    ///
    /// The predicate is the frozen `!want.includes(name) && CLUE_DB[id] ===
    /// undefined && CASKET_IDS[id] === undefined`: a listed name is never
    /// banked here, and a posted row whose id the selected trail facts name — a
    /// clue scroll, a casket, a challenge scroll — is left alone. Everything
    /// else in the pack, food included, is the frozen deposit's to take.
    ///
    /// The frozen `depositAllMatching` banks every match in one action; this
    /// machine banks one row per call and never the same row twice inside one
    /// attempt, so a bank that refuses the deposit ends the attempt — the
    /// claim it was making room for goes out as usual — rather than repeating
    /// the same verb forever. A page that posted no `inv_size` has not said how
    /// full the pack is, and no room is made on an invented one.
    pub(super) fn make_room(
        &self,
        selected: Option<&SelectedGameData>,
        state: &mut Restore,
        input: &Value,
    ) -> Option<Value> {
        // The row this attempt's deposit went out for: a page that still holds
        // it did not observe it land, so this attempt does not send it again.
        if let Some(sent) = state.room_sent.take() {
            if pack_holds_name(input, &sent) {
                state.room_tried.push(sent);
            }
        }
        let free = i64::from(input.get("inv_size").and_then(i32_of)?) - occupied(input);
        let missing = self
            .stripped
            .iter()
            .filter(|name| !worn_name(input, name) && !pack_holds_name(input, name))
            .count();
        if free >= missing as i64 {
            return None;
        }
        let name = make_room_row(selected, input, &self.stripped, &state.room_tried)?;
        state.room_sent = Some(name.clone());
        Some(deposit_verb(&name, self.token))
    }

    /// The `restore-incomplete` give-up: the frozen `could not re-equip … — will
    /// retry` over the names this call's posted worn page still does not show,
    /// with the attempt dropped so the next call starts a fresh one. The list
    /// keeps them, so the finish latch stays blocked and nothing downstream ever
    /// reads the reclaim as done. A log through `callback.log` and never a
    /// machine kind.
    pub(super) fn incomplete(&mut self, input: &Value) -> Value {
        let names: Vec<&str> = self
            .stripped
            .iter()
            .filter(|name| !worn_name(input, name))
            .map(String::as_str)
            .collect();
        self.restore = None;
        self.clock.deadline = None;
        json!({
            "kind": "callback.log",
            "token": self.token,
            "message": restore_incomplete(&names.join(", ")),
        })
    }

    /// The Entrana strip in front of the identified box row's own arms: the
    /// frozen `heldClueNeedsEntranaStrip` with `bankFirst`'s own two halves, one
    /// verb per call.
    ///
    /// Membership is this row's own selected `trail_coord`, decoded the landed
    /// way and inside the cap box — `entrana_coord` — and nothing else: a casket
    /// never arms it and no copied `CLUE_DB` is consulted. In order: every
    /// posted worn row whose name the frozen matcher folds is unequipped with
    /// the landed `unequip` verb — the worn row's own `Remove`, because the
    /// host's `wear` resolves inventory rows alone — and listed unless it is one
    /// of the two hard-trail dagger ids; then every posted pack row the matcher
    /// folds goes to the bank, one per call — the names the unequip put there,
    /// the dagger ids that were never listed, and a restricted item that was
    /// carried rather than worn — and the interface closes before the row's own
    /// arms run.
    ///
    /// The deposit is the frozen `depositAllMatching(name => !isKeep(name))` cut
    /// to the matcher's own rows: every regex-matching name in the pack, listed
    /// or not, and never the ordinary loot deposit.
    ///
    /// `None` is the fall-through: not a box row, or a strip this step already
    /// settled. The listed names outlive the step — they are the restore's own
    /// list — and a strip that cannot finish re-arms and logs rather than
    /// walking with the gear in hand: the frozen `bankFirst` returns false on
    /// that same failure and the trail does not run.
    pub(super) fn strip(&mut self, row: &TrailMembershipRow, input: &Value) -> Option<Value> {
        if !entrana_coord(row) {
            return None;
        }
        let mut state = match self.strip.take() {
            Some(state) => state,
            None => {
                self.clock.arm(ENTRANA_WAIT_MS);
                Strip::default()
            }
        };
        if state.settled {
            self.strip = Some(state);
            return None;
        }
        // The unequip pass. The verb repeats while the row stays on the posted
        // page — the host fails closed on an item that is already gone — and a
        // whole window of that is the frozen `still holding …` line with a
        // fresh attempt behind it.
        if let Some(worn) = pick_worn(input) {
            if self.clock.bound_reached() {
                self.clock.arm(ENTRANA_WAIT_MS);
                return Some(json!({
                    "kind": "callback.log",
                    "token": self.token,
                    "message": STILL_HOLDING,
                }));
            }
            self.list(&worn);
            self.strip = Some(state);
            return Some(unequip_verb(worn.name, self.token));
        }
        // The deposit pass: a posted pack row the frozen matcher folds goes to
        // the bank, one per call and whatever put it there — the unequip above,
        // a dagger id that is never listed, or a restricted item the player was
        // carrying rather than wearing.
        if let Some(name) = pick_pack_restricted(input) {
            if let Some(step) = self.bank_approach(&mut state.bank, input) {
                self.strip = Some(state);
                return Some(step);
            }
            if self.clock.bound_reached() {
                self.clock.arm(ENTRANA_WAIT_MS);
                return Some(json!({
                    "kind": "callback.log",
                    "token": self.token,
                    "message": STILL_HOLDING,
                }));
            }
            self.strip = Some(state);
            return Some(deposit_verb(&name, self.token));
        }
        // The exit: the interface closes once while it is up, and then the row's
        // own arms run for the rest of the step.
        if bank_open(input) {
            self.strip = Some(state);
            return Some(close_verb(self.token));
        }
        state.settled = true;
        self.strip = Some(state);
        None
    }

    /// One step of the shared Entrana bank approach: the walk to the nearest
    /// stand, then the posted booth's own open, then the interface — which is
    /// the `None` that lets the caller's own deposit or claim go out.
    ///
    /// The approaching half is bounded by this machine's own `ENTRANA_WAIT_MS`
    /// window, armed when the walk goes out: past it the approach logs the named
    /// `restore-walk-failed` — the caps' own name for this bank trip — and
    /// re-arms, so a stand the page never posts is a named failure rather than a
    /// silent hang. Freeze-honored like every other window here.
    pub(super) fn bank_approach(
        &mut self,
        bank: &mut BankApproach,
        input: &Value,
    ) -> Option<Value> {
        if bank_open(input) {
            bank.opened = true;
            return None;
        }
        if !bank.walked {
            bank.walked = true;
            self.clock.arm(ENTRANA_WAIT_MS);
            return Some(walk_nearest_bank_verb(self.token));
        }
        if let Some(booth) = nearest_booth(input) {
            if booth_arrival(&booth, input) == Arrival::Arrived {
                return Some(open_booth_verb(&booth, self.token));
            }
        }
        if self.clock.bound_reached() {
            *bank = BankApproach::default();
            self.clock.arm(ENTRANA_WAIT_MS);
            return Some(json!({
                "kind": "callback.log",
                "token": self.token,
                "message": restore_walk_failed(),
            }));
        }
        Some(self.emit("wait"))
    }

    /// List one stripped name the way the frozen `strippedGear` does: never
    /// twice under a different case, and never one of the two hard-trail dagger
    /// ids, which are unequipped like any other match and left off the list.
    pub(super) fn list(&mut self, worn: &Worn<'_>) {
        if worn.id.is_some_and(|id| DDS_IDS.contains(&id)) {
            return;
        }
        if self
            .stripped
            .iter()
            .any(|name| name.eq_ignore_ascii_case(worn.name))
        {
            return;
        }
        self.stripped.push(worn.name.to_string());
    }

    /// The adapter's `ownsEquipment` read: the frozen formula's own half that
    /// exists this slice — `strippedGear` is non-empty. True means
    /// do-not-grind-equip, it is read off the machine and never off a page, and
    /// it survives the token the list was made on: a dead session still answers
    /// true while the names are unclaimed. `bankedThisSolve` is not this slice's
    /// and is never set.
    pub(super) fn owns_equipment(&self) -> Value {
        json!({
            "kind": "ownsEquipment",
            "token": self.token,
            "owns": !self.stripped.is_empty(),
        })
    }
}
