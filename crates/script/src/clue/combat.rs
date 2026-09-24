use super::*;


/// The selected key-keeper step an identified membership row owns: the
/// `talk_key.keys` row whose own id is the row's, or `None` when the row is not
/// a key-hunt membership.
///
/// A sibling of `talk_step` and never a fold into it: the two families publish
/// different ids, so a talk step is never hunted and a keeper is never
/// Talked-to. The held clue stays the riddle the landed identify returned — the
/// key is not membership — and the row's own `key_id`, `keeper` and `spawn` are
/// what the arm reads.
pub(super) fn key_step(selected: Option<&SelectedGameData>, id: i32) -> Option<&TalkKeyKeyRow> {
    selected?.talk_key()?.keys.iter().find(|key| key.id == id)
}

/// The packed npc type one key-keeper row names: that keeper's own id and the
/// display name the posted npc page carries beside it, and nothing else.
///
/// Only a `type` matcher is one npc. A `category` or a bare `name` keeper
/// matches many and carries no packed id, so those rows have no type here and
/// stay idle rather than hunting the nearest anything. The matcher's own script
/// alias is never part of this identity — the posted page carries no alias at
/// all — and it is never substituted for the display name the verb carries.
pub(super) fn keeper_type(keeper: &TalkKeyKeeper) -> Option<(i32, &str)> {
    let id = keeper.id?;
    let name = keeper.name.as_deref().filter(|name| !name.is_empty())?;
    (keeper.kind == KEEPER_TYPE).then_some((id, name))
}
impl ClueRuntime {

    /// `Steady` on an identified guarded row: the first Dig, the fight, or the
    /// post-kill redig.
    ///
    /// The encounter is this row's own session state: absent until this token's
    /// first Dig went out, present until a different held row, an abort or the
    /// reset clears it. Nothing else about it is cached — every call re-reads
    /// this call's marshalled pages, and the decoded tile is re-read from the
    /// row's own selected `trail_coord` rather than kept.
    pub(super) fn guarded(
        &mut self,
        row: &TrailMembershipRow,
        tile: Tile,
        input: &Value,
        selected: Option<&SelectedGameData>,
    ) -> Value {
        if self.guardian.is_none() {
            // No encounter yet: the landed walk-then-Dig, and the Dig that
            // goes out is the spawn.
            return self.spawn(tile, input);
        }
        if self.guardian.as_ref().is_some_and(|g| g.post_kill) {
            // The kill was observed: walk back to the decoded tile and Dig
            // again, repeating while this same clue stays held.
            return self.dig(tile, input);
        }
        self.fight(guardian_names(row), tile, input, selected)
    }

    /// The guarded row's first Dig: the landed walk-then-Dig of the sibling
    /// unguarded arm, and the verb that spawns the wizard. The encounter is
    /// created from that Dig and from nothing else — a call that is still
    /// walking, has no posted `here`, or has no posted Spade waits without one
    /// (the arrived no-Spade tick as the named `supplies-needed` class) and the
    /// fight never starts early.
    pub(super) fn spawn(&mut self, tile: Tile, input: &Value) -> Value {
        match arrival(tile, input) {
            Arrival::Unknown => self.emit("wait"),
            Arrival::Walking => self.walk(tile),
            Arrival::Arrived if !spade_posted(input) => self.emit(SUPPLIES_NEEDED),
            Arrival::Arrived => {
                self.guardian = Some(Guardian {
                    owned: None,
                    post_kill: false,
                });
                self.dig_verb()
            }
        }
    }

    /// The fight after the spawn: the frozen mid-fight hitpoints wait, this
    /// call's posted-npc observation, and only then the Protect from Magic
    /// overlay and the Attack.
    ///
    /// The npc page is this call's own marshalling of `host().snapshot.npcs`,
    /// so nothing is cached and no scene is scanned. The spawn is observed
    /// **before** anything is raised: the row this token already owns, or —
    /// before that Attack — a posted index of the row family's cap-documented
    /// names inside the frozen radius. A call with neither waits with no click
    /// and no Attack, so the click is only ever enqueued behind a matching
    /// spawn and the nearest anything is never Attacked. The encounter then
    /// owns the Attacked index: the kill is the owned row posted at zero
    /// health while the page still shows this token's fight on it, or the
    /// owned index leaving the page inside the frozen grace. An index this
    /// token never Attacked is never a kill, a disappearance outside the grace
    /// is a `wait`, and this machine has no `guardian-lost`.
    pub(super) fn fight(
        &mut self,
        names: &'static [&'static str],
        tile: Tile,
        input: &Value,
        selected: Option<&SelectedGameData>,
    ) -> Value {
        let Some(page) = input.get("npcs").and_then(Value::as_array) else {
            // No posted npc page this call: no spawn is posted, so there is
            // nothing to observe and nothing to raise the overlay for. The
            // spawn wait and the kill wait are the same `wait`.
            return self.emit("wait");
        };
        let self_slot = posted_i32(input, "self_slot");
        let now = self.clock.now();
        let owned = self
            .guardian
            .as_ref()
            .and_then(|g| g.owned.as_ref())
            .map(|owned| owned.index);
        if let Some(index) = owned {
            // The wizard this token Attacked, read before the overlay: a kill
            // never clicks, and a page that no longer posts the spawn is not a
            // reason to raise the prayer.
            let Some(row) = page
                .iter()
                .find(|row| posted_i32(row, "index") == Some(index))
            else {
                // The owned index left the page. Inside the frozen grace that
                // is this token's kill; outside it the wizard is gone without
                // ever being seen at zero health, so the encounter is lost and
                // the token dies with it — never a redig.
                let within = self
                    .guardian
                    .as_ref()
                    .and_then(|g| g.owned.as_ref())
                    .is_some_and(|owned| {
                        now.saturating_duration_since(owned.seen_at)
                            < Duration::from_millis(KILL_GRACE_MS)
                    });
                if !within {
                    return self.ended(GUARDIAN_LOST);
                }
                self.kill();
                return self.dig(tile, input);
            };
            // Posted: this call's observation is the last-seen the grace reads.
            if let Some(owned) = self.guardian.as_mut().and_then(|g| g.owned.as_mut()) {
                owned.seen_at = now;
            }
            if died_owned(row, self_slot, self_target(input)) {
                self.kill();
                return self.dig(tile, input);
            }
            // Posted and alive: the fight is on, and the Attack is enqueued
            // once per owned index — this call only reads the overlay below
            // and never re-Attacks.
        }
        // The spawn this call raises the overlay for: the owned wizard just
        // observed, or — before that Attack — the posted family wizard the
        // Attack is about to own. A call with neither is the spawn wait.
        let spawn = if owned.is_some() {
            None
        } else {
            pick_npc(names, page, self_slot, posted_here(input))
        };
        if owned.is_none() && spawn.is_none() {
            return self.emit("wait");
        }
        let Some(prayer) = selected.and_then(|data| api::prayer::lookup(data, PROTECT_FROM_MAGIC))
        else {
            // The one selected row this fight raises is not in the pin:
            // nothing is invented in its place and the fight waits.
            return self.emit("wait");
        };
        if posted_i32(input, "varp95") != Some(1) {
            // Not a proven on: the landed generic `if-button` raises the
            // row's own selected component. An overlay already up skips the
            // click, an unobserved one never Attacks, and nothing here nests
            // the prayer isolate or waits a toggle out.
            return json!({
                "kind": "if-button",
                "token": self.token,
                "component_id": prayer.button_com,
            });
        }
        match spawn {
            // The overlay reads up and the posted spawn is now this token's:
            // the one Attack for this index, carrying the posted name and the
            // posted scene index and nothing else.
            Some((index, name)) => {
                if let Some(guardian) = self.guardian.as_mut() {
                    guardian.owned = Some(Owned {
                        index,
                        seen_at: now,
                    });
                }
                json!({
                    "kind": "npc",
                    "token": self.token,
                    "name": name,
                    "action": ATTACK,
                    // The posted scene index, always present: the host
                    // matches that identity and refuses a stale one.
                    "index": index,
                })
            }
            // Owned and still posted: the fight waits for its kill.
            None => self.emit("wait"),
        }
    }

    /// The kill was observed: the owned index is dropped and the walk back
    /// with its Dig replaces the fight. Only an owned, settled read reaches
    /// here.
    pub(super) fn kill(&mut self) {
        if let Some(guardian) = self.guardian.as_mut() {
            guardian.owned = None;
            guardian.post_kill = true;
        }
    }

    /// `Steady` on an identified key-keeper row: the one key the keeper the
    /// selected family names drops for this clue, walked to, Attacked and
    /// Taken, one verb per call.
    ///
    /// The membership is the selected `talk_key.keys` row whose own id is this
    /// step's, read on this arm alone: a talk step belongs to the arm above and
    /// a keeper is never Talked-to. The held clue stays the riddle the landed
    /// identify returned, so this is a sibling family and never a second
    /// identify — and the key in hand is not trail completion.
    ///
    /// Inside the arm the hunt is: the key already on the posted pack page ends
    /// it with the idle `wait` the original riddle keeps; the keeper this
    /// token's `Attack` went out for is observed first, and only its kill lets
    /// the pickup run; the walk goes to the published `{x, z, plane}` tile and
    /// repeats until this call's posted `here` holds; the one `Attack` is only
    /// ever a posted npc of the keeper's packed type standing on that tile
    /// carrying the posted `Attack`; and the Take is only ever a posted ground
    /// row of the key's own id at that same tile. A page with no posted match,
    /// no posted `here`, a missing slot count or a full pack waits with the
    /// token live: nothing is invented, nothing is Dropped, no prayer is raised
    /// and no completion kind is ever emitted here.
    pub(super) fn keys(
        &mut self,
        row: &TrailMembershipRow,
        input: &Value,
        selected: Option<&SelectedGameData>,
    ) -> Value {
        let Some(key) = key_step(selected, row.id) else {
            // Not a selected key-keeper membership: the desc-only riddles no
            // keeper names, the empty-params 2722 and every other identified
            // row keep the idle they had.
            return self.emit("wait");
        };
        if holds(input, key.key_id) {
            // The key this keeper drops is already on the posted pack page: the
            // hunt is over and the original riddle idles. No Attack, no gate
            // and no completion kind — and a key banked but not held is not
            // observed at all, because the bank is not a posted page.
            return self.emit("wait");
        }
        let Some(spawn) = key.spawn.as_ref() else {
            // No published spawn: there is no tile this arm may walk to, and no
            // coordinate is invented for a keeper the family covered instead.
            return self.emit("wait");
        };
        let Some((keeper_id, keeper_name)) = keeper_type(&key.keeper) else {
            // A `category` or a bare `name` keeper names no packed npc type, so
            // its row hunts nothing and idles the way it always did.
            return self.emit("wait");
        };
        let tile = Tile {
            x: spawn.x,
            z: spawn.z,
            level: spawn.plane,
        };
        let now = self.clock.now();
        if let Some(index) = self
            .keeper
            .as_ref()
            .and_then(|keeper| keeper.owned.as_ref())
            .map(|owned| owned.index)
        {
            // The keeper this token Attacked, read before anything walks: only
            // a settled read of that index ends the hunt, and the Attack is
            // never issued twice for one owned index.
            let Some(page) = input.get("npcs").and_then(Value::as_array) else {
                // No posted npc page this call: the owned keeper cannot be
                // observed at all, so this call waits.
                return self.emit("wait");
            };
            match page
                .iter()
                .find(|posted| posted_i32(posted, "index") == Some(index))
            {
                Some(posted) => {
                    // Posted: this call's observation is the last-seen the
                    // grace reads.
                    if let Some(owned) = self.keeper.as_mut().and_then(|k| k.owned.as_mut()) {
                        owned.seen_at = now;
                    }
                    if !died_owned(posted, posted_i32(input, "self_slot"), self_target(input)) {
                        // Posted and alive: the hunt waits for its kill.
                        return self.emit("wait");
                    }
                }
                None => {
                    // The owned index left the page. Inside the frozen grace
                    // that is this token's kill; outside it the keeper is gone
                    // without ever being seen at zero health, which is this
                    // hunt's `wait` and never the wizard encounter's
                    // `guardian-lost`.
                    let within = self
                        .keeper
                        .as_ref()
                        .and_then(|keeper| keeper.owned.as_ref())
                        .is_some_and(|owned| {
                            now.saturating_duration_since(owned.seen_at)
                                < Duration::from_millis(KILL_GRACE_MS)
                        });
                    if !within {
                        return self.emit("wait");
                    }
                }
            }
            if let Some(keeper) = self.keeper.as_mut() {
                keeper.owned = None;
                keeper.post_kill = true;
            }
        }
        let after_kill = self.keeper.as_ref().is_some_and(|keeper| keeper.post_kill);
        match arrival(tile, input) {
            // No posted `here`: there is no arrival claim to make and no walk
            // to measure, so this tick waits rather than walking blind.
            Arrival::Unknown => self.emit("wait"),
            Arrival::Walking => self.walk(tile),
            Arrival::Arrived if after_kill => {
                // The kill is observed: the key lies on its own spawn tile, and
                // the hunt Takes it.
                match pick_key(input, key.key_id, tile) {
                    Some(drop) => {
                        let Some(size) = posted_inv_size(input) else {
                            // No posted slot count: the pack's fullness is not
                            // invented for it.
                            return self.emit("wait");
                        };
                        if occupied(input) >= i64::from(size) {
                            // A full pack waits — this arm Drops no food to make
                            // room, unlike the casket's own Take.
                            return self.emit("wait");
                        }
                        json!({
                            "kind": "obj",
                            "token": self.token,
                            "x": drop.tile.x,
                            "z": drop.tile.z,
                            "level": drop.tile.level,
                            "name": drop.name,
                            "action": TAKE,
                        })
                    }
                    // Arrived with nothing of the key posted on the tile: wait
                    // and re-read the page next call.
                    None => self.emit("wait"),
                }
            }
            Arrival::Arrived => {
                match input
                    .get("npcs")
                    .and_then(Value::as_array)
                    .and_then(|page| pick_keeper(keeper_id, keeper_name, page, tile))
                {
                    Some((index, name)) => {
                        self.keeper = Some(Keeper {
                            owned: Some(Owned {
                                index,
                                seen_at: now,
                            }),
                            post_kill: false,
                        });
                        json!({
                            "kind": "npc",
                            "token": self.token,
                            "name": name,
                            "action": ATTACK,
                            // The posted scene index, always present: the host
                            // matches that identity and refuses a stale one.
                            "index": index,
                        })
                    }
                    // Arrived with no posted row of this keeper's type: stay on
                    // the tile and wait, never chasing a wanderer.
                    None => self.emit("wait"),
                }
            }
        }
    }
}
