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

/// The frozen `Withdraw-1` label the restore's claim rides; the bank row's own
/// posted action slots are matched by the host and never by this machine.
pub(super) const WITHDRAW_ONE: &str = "Withdraw-1";

/// The native clue's one bank approach/open attempt failed its existing
/// walk or readiness deadline. Sherlock maps this terminal reason to Blocked.
pub(crate) const BANK_APPROACH_FAILED: &str = "bank-approach-failed";

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

/// The Entrana strip/restore window after an observed bank is ready: how long
/// a gear operation has to settle before its existing named failure path runs.
/// Bank approach and open use the shared bank-open walk/readiness bounds below.
/// Freeze-honored like every other window here.
pub(super) const ENTRANA_WAIT_MS: u64 = 30_000;

/// The monk's restriction from `content/scripts/areas/area_port_sarim/scripts/monk_of_entrana.rs2:26-50`.
/// Categories at lines 27–49 apply to both the pack and worn equipment; the
/// `cannon_parts` category at line 50 applies to the pack alone.
pub(super) fn restricted_item(
    selected: Option<&SelectedGameData>,
    id: i32,
    inventory: bool,
) -> bool {
    let Some(category) = selected
        .and_then(|data| data.item_by_id(id))
        .and_then(|item| item.category.as_deref())
    else {
        return false;
    };
    match category {
        "armour_hands" | "weapon_staff" | "armour_helmet" | "armour_body" | "armour_legs"
        | "armour_shield" | "armour_cape" | "armour_godcape" | "weapon_slash" | "weapon_blunt"
        | "weapon_stab" | "weapon_crossbow" | "weapon_axe" | "weapon_pickaxe"
        | "weapon_javelin" | "weapon_2h_sword" | "weapon_spear" | "weapon_spiked"
        | "weapon_thrown" | "weapon_scythe" | "weapon_bow" | "weapon_claws" | "weapon_polearm" => {
            true
        }
        "cannon_parts" => inventory,
        _ => false,
    }
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

/// One posted worn row selected by the item's selected category. Its display
/// name is retained only for the host's unequip and reclaim actions.
pub(super) struct Worn<'a> {
    pub(super) name: &'a str,
}

/// The first posted worn row whose selected item's category is restricted, in
/// posted order. The page is the wrapper's marshalling of
/// `host().snapshot.equipment`, the raw rows with their own `slot`, and never
/// `Equipment.items()` — which drops the slot — and never a scan of the item
/// table.
pub(super) fn pick_worn<'a>(
    selected: Option<&SelectedGameData>,
    input: &'a Value,
) -> Option<Worn<'a>> {
    input
        .get("equipment")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|row| {
            let id = posted_i32(row, "id")?;
            let name = row.get("name").and_then(Value::as_str)?;
            restricted_item(selected, id, false).then_some(Worn { name })
        })
}

/// The first posted pack row whose selected item's category is restricted: the
/// strip's deposit candidate, listed or not. The row needs a positive count
/// and its posted display name for the host's deposit action.
pub(super) fn pick_pack_restricted(
    selected: Option<&SelectedGameData>,
    input: &Value,
) -> Option<String> {
    input
        .get("inv")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|row| {
            let id = posted_i32(row, "id")?;
            let name = row.get("name").and_then(Value::as_str)?;
            let count = posted_i32(row, "count")?;
            (count > 0 && restricted_item(selected, id, true)).then(|| name.to_string())
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
/// it. Rust-native Sherlock may also post authoritative operability/stand facts;
/// a missing `approach` key is the compatibility caller's legacy page.
pub(super) struct Booth<'a> {
    pub(super) x: i32,
    pub(super) z: i32,
    pub(super) level: i32,
    pub(super) id: i32,
    pub(super) name: Option<&'a str>,
    pub(super) action: Option<&'a str>,
    approach_present: bool,
    approach: Option<BoothApproach>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BoothApproach {
    can_operate: bool,
    dest: Option<Tile>,
}

pub(super) fn nearest_booth(input: &Value) -> Option<Booth<'_>> {
    let row = input.get("nearest_booth")?;
    let approach_page = row.get("approach");
    Some(Booth {
        x: posted_i32(row, "x")?,
        z: posted_i32(row, "z")?,
        level: posted_i32(row, "level")?,
        id: posted_i32(row, "id")?,
        name: row.get("name").and_then(Value::as_str),
        action: row.get("op").and_then(Value::as_str),
        approach_present: approach_page.is_some(),
        approach: approach_page.and_then(|facts| {
            Some(BoothApproach {
                can_operate: facts.get("can_operate")?.as_bool()?,
                dest: facts.get("dest").and_then(posted_tile),
            })
        }),
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

/// The landed `deposit` step, one restricted name at a time: the first posted
/// pack row with a positive count whose selected category is restricted under
/// the monk's rule — whether this strip just unequipped it or the player
/// already carried it — and never the ordinary loot deposit.
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

fn bank_stand_walk_verb(tile: Tile, token: u64) -> Value {
    json!({
        "kind": "walk",
        "token": token,
        "x": tile.x,
        "z": tile.z,
        "level": tile.level,
    })
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
                    if step.get("kind").and_then(Value::as_str) != Some("aborted") {
                        self.restore = Some(state);
                    }
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
    /// source category rule at
    /// `content/scripts/areas/area_port_sarim/scripts/monk_of_entrana.rs2:26-50`,
    /// followed by the bank approach's own two halves, one verb per call.
    ///
    /// Membership is this row's own selected `trail_coord`, decoded the landed
    /// way and inside the cap box — `entrana_coord` — and nothing else: a casket
    /// never arms it and no copied `CLUE_DB` is consulted. In order: every worn
    /// item with a restricted category is unequipped with the landed `unequip`
    /// verb — the worn row's own `Remove`, because the host's `wear` resolves
    /// inventory rows alone — and listed by its display name for reclaim; then
    /// every pack row with a restricted category goes to the bank, one per call,
    /// and the interface closes before the row's own arms run. `cannon_parts`
    /// applies to pack rows only, as in the content rule.
    ///
    /// `None` is the fall-through: not a box row, or a strip this step already
    /// settled. The listed names outlive the step — they are the restore's own
    /// list — and a strip that cannot finish re-arms and logs rather than
    /// walking with restricted gear in hand: the frozen `bankFirst` returns
    /// false on that same failure and the trail does not run.
    pub(super) fn strip(
        &mut self,
        row: &TrailMembershipRow,
        input: &Value,
        selected: Option<&SelectedGameData>,
    ) -> Option<Value> {
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
        if let Some(worn) = pick_worn(selected, input) {
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
        // The deposit pass selects every posted pack row in a restricted
        // category, whether it was just unequipped or was already carried.
        if let Some(name) = pick_pack_restricted(selected, input) {
            if let Some(step) = self.bank_approach(&mut state.bank, input) {
                if step.get("kind").and_then(Value::as_str) != Some("aborted") {
                    self.strip = Some(state);
                }
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

    /// One bounded Entrana bank approach shared by strip and restore. This
    /// clue is a page→verb machine, not BankOpen's `Cx` action-poll context;
    /// nesting it would transfer token ownership into that scheduler.
    /// Sherlock instead borrows its cached flood and projects the shared
    /// `booth_approach` facts in Rust.
    ///
    /// Native pages walk their proved stand directly; compatibility pages
    /// send one nearest-bank route. Each changed stand is walked once. Shared
    /// bank-open walk/readiness bounds fail closed on missing approach facts or
    /// a refused open; restore can issue another open only after success and
    /// an observed close.
    pub(super) fn bank_approach(
        &mut self,
        bank: &mut BankApproach,
        input: &Value,
    ) -> Option<Value> {
        if bank_open(input) {
            if !bank.opened {
                bank.opened = true;
                self.clock.arm(ENTRANA_WAIT_MS);
            }
            return None;
        }
        // A closed bank after a proved open starts another restore claim
        // cycle; unchanged closed evidence before success stays latched.
        if bank.opened {
            bank.opened = false;
            bank.open_issued = false;
            bank.approach_dest = None;
            self.clock.arm(crate::bank_open::WALK_BOUND_MS);
        }
        let booth = nearest_booth(input);
        if !bank.started {
            bank.started = true;
            self.clock.arm(crate::bank_open::WALK_BOUND_MS);
            if !booth.as_ref().is_some_and(|booth| booth.approach_present) {
                return Some(walk_nearest_bank_verb(self.token));
            }
        }
        if self.clock.bound_reached() {
            return Some(self.aborted(BANK_APPROACH_FAILED));
        }
        if bank.open_issued {
            return Some(self.emit("wait"));
        }
        if let Some(booth) = booth {
            if booth.approach_present {
                if let Some(approach) = booth.approach {
                    if approach.can_operate {
                        bank.open_issued = true;
                        self.clock.arm(crate::bank_open::BANK_READY_MS);
                        return Some(open_booth_verb(&booth, self.token));
                    }
                    if let Some(dest) = approach.dest {
                        if bank.approach_dest != Some(dest) {
                            bank.approach_dest = Some(dest);
                            return Some(bank_stand_walk_verb(dest, self.token));
                        }
                    }
                }
            } else if booth_arrival(&booth, input) == Arrival::Arrived {
                // Older compatibility pages do not carry the native-only
                // approach projection. Keep their existing admission rule,
                // but still latch and bound the one open request.
                bank.open_issued = true;
                self.clock.arm(crate::bank_open::BANK_READY_MS);
                return Some(open_booth_verb(&booth, self.token));
            }
        }
        Some(self.emit("wait"))
    }

    /// List one stripped item's display name for reclaim, never twice under a
    /// different case.
    pub(super) fn list(&mut self, worn: &Worn<'_>) {
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
