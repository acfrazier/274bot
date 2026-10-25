mod travel;
use super::production::*;
use crate::*;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc, Mutex,
};
pub(super) use travel::{follow_step, nav_kit_steps, tele_step};

/// Lumbridge courtyard stand where the two-bot trade meets.
const TRADE_COURTYARD: WorldTile = WorldTile {
    x: 3220,
    z: 3220,
    level: 0,
};

const KQ_BANK: WorldTile = WorldTile {
    x: 3308,
    z: 3120,
    level: 0,
};
const KQ_LAIR: WorldTile = WorldTile {
    x: 3508,
    z: 9493,
    level: 0,
};
/// Whole-run watchdog for `jive_kq_four` (JIVE-KQ-1024).
///
/// Measured passing receipts (counted ticks / elapsed seconds):
/// diagnosis 2,643/1,641.616, 2,818/1,739.422, 2,343/1,490.066;
/// first fix series 2,426/1,501.433, 2,531/1,566.269,
/// 2,471/1,531.477; final series 2,283/1,416.660, 2,545/1,573.469,
/// 2,857/1,763.554, 2,441/1,512.262. Every completed pass fits
/// under 1,800 s. Two oracle stalls were censored at this deadline
/// (2,893 ticks / 1,800.016 s and 2,904 ticks / 1,800.009 s). A
/// censored time is a lower bound, not a completion time, so it
/// does not size this watchdog. Those stalls waited for
/// `restock_overlap`, a chance interleaving that is now an
/// observation, not a gate (see [`KqFleetWitness::complete`]).
/// A run that meets every gated term no longer waits for it.
/// Only completed runs of the observation-only witness size this value:
/// round 2 (`319e12b74`, `JIVE-KQ-1024/fix-r2/watchdog-r2.json`) passed in
/// 1,483.755, 1,601.719, 1,744.640 and 1,576.639 s. 2,094 s is the slowest
/// of those plus 20 %, rounded up (1,744.640 × 1.2 = 2,093.568); 1,800 s
/// left 3 % headroom (Grok review round 2).
/// The content gives no hard upper bound: fight length and the
/// local-visibility witness terms can require a third trip.
const KQ_WATCHDOG: Duration = Duration::from_secs(2_094);
/// Selected-revision Kalphite Queen drops which cannot be confused with the
/// seeded loadout. Ammunition is deliberately excluded: recovered arrows
/// are not boss-loot evidence. Banking a listed drop is an observation,
/// never a gate: the Queen's table rolls `random(128)` and 27 of those
/// rolls drop items outside this list: mithril, iron and rune arrows (15),
/// law runes (5) and the amulet of power (7); the last two are also in the
/// seeded loadout (`lostcity-289/content/scripts/drop
/// tables/scripts/kalphite_queen.rs2:82-87,96-97,102-103`), so a sequence of
/// legitimate kills can bank no listed item (JIVE-KQ-1024 round 3).
const KQ_LOOT_IDS: &[i32] = &[
    1245, 565, 1452, 560, 3140, 1249, 554, 985, 987, 3053, 379, 561, 1462, 413, 1359, 1113, 830,
    1247, 1347, 2366, 1617, 1621, 1619, 1623, 246,
];

#[derive(Clone)]
struct PairPrepBarrier {
    ready: Arc<AtomicU8>,
    expected: u8,
}

impl PairPrepBarrier {
    fn new(companions: usize) -> Self {
        assert!((1..=7).contains(&companions));
        Self {
            ready: Arc::new(AtomicU8::new(0)),
            expected: ((1_u16 << (companions + 1)) - 2) as u8,
        }
    }

    fn mark_ready(&self, profile: usize) {
        self.ready.fetch_or(1 << profile, Ordering::Release);
    }

    fn all_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire) & self.expected == self.expected
    }
}
#[derive(Default)]
struct ClueSlotWitness {
    name: Option<String>,
    initial_inv: Vec<(i32, i32)>,
    hp_baseline: Option<i32>,
    hp_unchanged: bool,
    saw_peer: bool,
    lobby: bool,
    offer: bool,
    confirm: bool,
    pen: bool,
    casket: bool,
    clue_consumed: bool,
    forfeit_yes: bool,
    win: bool,
    returned: bool,
}

#[derive(Default)]
struct ClueFleetWitness {
    started: bool,
    slots: [ClueSlotWitness; 2],
    reward_ids: Vec<i32>,
    reward_banked: bool,
}

impl ClueFleetWitness {
    fn start(&mut self) {
        self.started = true;
    }

    fn observe(&mut self, slot_index: usize, snap: &GameSnapshot) {
        if !self.started || !snap.ingame() || snap.scene_state() != 2 {
            return;
        }
        // The counterpart by its own name, not any named player nearby.
        let saw_peer = self.slots[1 - slot_index]
            .name
            .as_deref()
            .is_some_and(|peer| {
                snap.players().iter().any(|player| {
                    player
                        .actor
                        .name
                        .as_deref()
                        .is_some_and(|name| same_player(name, peer))
                })
            });
        let slot = &mut self.slots[slot_index];
        if slot.name.is_none() {
            slot.name = pair_local_name(snap).map(str::to_owned);
            slot.initial_inv = snap.inv().to_vec();
            slot.hp_baseline = Some(pair_effective_stat(snap, 3));
            slot.hp_unchanged = true;
        }
        if let Some(baseline) = slot.hp_baseline {
            slot.hp_unchanged &= pair_effective_stat(snap, 3) == baseline;
        }
        slot.saw_peer |= saw_peer;
        slot.lobby |= pair_near(snap, DUEL_CHALLENGE, 8);
        let exact_obstacles = pair_varp(snap, 286) == Some(1024);
        slot.offer |= snap.modals().main == 6575
            && exact_obstacles
            && snap.widgets().iter().any(|widget| {
                widget.component_id == 6671
                    && widget
                        .text
                        .as_deref()
                        .is_some_and(|text| !text.trim().is_empty())
            });
        slot.confirm |= snap.modals().main == 6412 && exact_obstacles;
        // 289 duelarena.dbrow:7-11 lets the server choose any obstacle pen
        // and any square in it; a radius around the first pen misses its edge.
        let in_pen = snap.tile().is_some_and(|(x, z, level)| {
            level == 0 && DUEL_FIGHT_PENS[3..].iter().any(|pen| within(*pen, x, z))
        });
        slot.pen |= in_pen;
        slot.casket |= pair_inv_id(snap, 3555) > 0;
        slot.clue_consumed |= slot.casket && pair_inv_id(snap, 3554) == 0;
        slot.forfeit_yes |= snap
            .chat_options()
            .iter()
            .any(|option| option.text.trim().eq_ignore_ascii_case("yes"));
        slot.win |= snap.modals().main == 6733;
        slot.returned |= slot.pen && !in_pen && pair_inv_id(snap, 3554) == 0;

        if slot_index == 0 && slot.casket && pair_inv_id(snap, 3555) == 0 {
            for (id, count) in snap.inv() {
                let initial = slot
                    .initial_inv
                    .iter()
                    .filter(|(initial_id, _)| initial_id == id)
                    .map(|(_, initial_count)| *initial_count)
                    .sum::<i32>();
                if *count > initial && !self.reward_ids.contains(id) {
                    self.reward_ids.push(*id);
                }
            }
        }
        if slot_index == 0 && snap.bank_component_id() >= 0 && snap.bank_loaded() {
            self.reward_banked |= self.reward_ids.iter().any(|id| pair_bank_id(snap, *id) > 0);
        }
    }

    fn identities_ready(&self) -> bool {
        let names = self
            .slots
            .iter()
            .filter_map(|slot| slot.name.as_deref())
            .collect::<Vec<_>>();
        names.len() == 2 && names[0] != names[1] && self.slots.iter().all(|slot| slot.saw_peer)
    }

    fn handshake_complete(&self) -> bool {
        self.identities_ready()
            && self
                .slots
                .iter()
                .all(|slot| slot.lobby && slot.offer && slot.confirm && slot.pen)
    }

    fn trail_complete(&self) -> bool {
        let solver = &self.slots[0];
        solver.casket
            && solver.clue_consumed
            && solver.forfeit_yes
            && solver.returned
            && !self.reward_ids.is_empty()
            && self.slots[1].win
            && self.slots[1].returned
    }

    fn complete(&self) -> bool {
        self.handshake_complete()
            && self.trail_complete()
            && self.reward_banked
            && self.slots.iter().all(|slot| slot.hp_unchanged)
    }
}

/// The first observation that cleared one slot's `survived` latch: the
/// snapshot tick, the observed hitpoints, the world tile, and the newest
/// chat line in that snapshot, if any. Recorded once, at the transition,
/// so a real death is diagnosable from the receipt alone. The earlier
/// witness kept only the false latch, and one run's receipt could not
/// distinguish a death from an unreceived stat (INT-BATCH-1024E).
struct KqSurvivalClear {
    tick: u32,
    hp: i32,
    tile: Option<(i32, i32, i32)>,
    chat: Option<String>,
}

#[derive(Default)]
struct KqSlotWitness {
    name: Option<String>,
    survived: bool,
    strength_baseline: Option<i32>,
    ranged_baseline: Option<i32>,
    pack_ready: bool,
    kit_equipped: bool,
    pass_ready: bool,
    pass_bank_seen: bool,
    pass_shop_seen: bool,
    pass_source_complete: bool,
    bank_activity: bool,
    bank_departed: bool,
    at_bank_now: bool,
    surface_rope: bool,
    upper_rope: bool,
    surface_party: bool,
    upper_party: bool,
    lair_entries: u8,
    was_in_lair: bool,
    in_lair_now: bool,
    fighting_now: bool,
    ground_alive: bool,
    flying_alive: bool,
    flying_zero: bool,
    flying_was_visible: bool,
    kills: u8,
    disappearances: u8,
    melee_formation: bool,
    ranged_formation: bool,
    strength_gain: bool,
    ranged_gain: bool,
    second_entry_xp: Option<(i32, i32)>,
    second_attack: bool,
    food_eaten: bool,
    food_eaten_at: Option<(i32, i32)>,
    xp_after_food: bool,
    boosted: bool,
    camelot_escape: bool,
    duel_escape: bool,
    ring_spent: bool,
    /// Listed-loot observations only (see [`KQ_LOOT_IDS`]). Which drop a
    /// kill rolls is a chance outcome, so these fields never gate the
    /// run. The fleet value reaches the receipt through
    /// [`KqFleetWitness::terminal_summary`].
    loot_ground: Option<i32>,
    loot_collected: bool,
    loot_banked: bool,
    /// The first received-stats observation of hitpoints 0, if one ever
    /// cleared `survived`. Read-only reporting state.
    survival_cleared: Option<KqSurvivalClear>,
    bank_returns: u8,
    bank_return_latched: bool,
    safely_banked: bool,
    /// This slot was observed at least once after the safe finish
    /// began ([`KqFleetWitness::begin_finish`]).
    finish_observed: bool,
    /// This slot's latest post-finish observation showed its bank
    /// closed. Recomputed from every post-finish snapshot, never
    /// latched from an earlier one and never from the historical
    /// `safely_banked` flag: a bank seen open after the finish
    /// revokes it until a later snapshot shows the bank closed.
    finish_closed: bool,
    /// Last observed state for the terminal failure summary:
    /// world tile, snapshot game tick, and bank component from this
    /// slot's most recent snapshot. Read-only reporting state; no
    /// witness latch reads it.
    last_tile: Option<(i32, i32, i32)>,
    last_tick: u32,
    last_bank_component: i32,
}

#[derive(Default)]
struct KqFleetWitness {
    started: bool,
    slots: [KqSlotWitness; 4],
    /// One member was restocking at the bank while another was
    /// fighting in the lair. That stagger follows food use,
    /// hitpoint loss, and the Queen's targeting, so it is a chance
    /// outcome. It is an observation recorded in the receipt
    /// through [`KqFleetWitness::terminal_summary`]. It never gates
    /// the run.
    restock_overlap: bool,
    premature_departure: bool,
    leader_had_two_ropes: bool,
    surface_rope_count: Option<i32>,
    surface_rope_valid: bool,
    upper_rope_valid: bool,
    /// The coordinated safe finish has begun: every KQ witness was met
    /// and the fleet's scripts were stopped through the product Stop
    /// path. Bank observations before this point do not count toward
    /// the finish.
    finish_started: bool,
}

impl KqFleetWitness {
    fn start(&mut self) {
        self.started = true;
    }

    fn observe(&mut self, slot_index: usize, snap: &GameSnapshot) {
        if !self.started || !snap.ingame() || snap.scene_state() != 2 {
            return;
        }
        let slot = &mut self.slots[slot_index];
        if slot.name.is_none() {
            slot.name = pair_local_name(snap).map(str::to_owned);
            slot.survived = true;
            slot.strength_baseline = Some(pair_xp(snap, 2));
            slot.ranged_baseline = Some(pair_xp(snap, 4));
        }
        // Survival clears only on hitpoints observed from stats the
        // server has sent since login. An absent row, or the client's
        // zero-filled arrays before the first stat packets, is a
        // missing observation, never a death (`pair_observed_hp`).
        // The first clearing observation is kept for the receipt.
        if let Some(hp) = pair_observed_hp(snap).filter(|&hp| hp <= 0) {
            if slot.survived && slot.survival_cleared.is_none() {
                slot.survival_cleared = Some(KqSurvivalClear {
                    tick: snap.tick(),
                    hp,
                    tile: snap.tile(),
                    // The chat ring is newest first
                    // (`snapshot.rs`, `chat_lines`).
                    chat: snap
                        .chat_lines()
                        .first()
                        .map(|line| line.text.clone())
                        .filter(|text| !text.is_empty()),
                });
            }
            slot.survived = false;
        }
        slot.last_tile = snap.tile();
        slot.last_tick = snap.tick();
        slot.last_bank_component = snap.bank_component_id();
        let strength_xp = pair_xp(snap, 2);
        let ranged_xp = pair_xp(snap, 4);
        slot.strength_gain |= slot
            .strength_baseline
            .is_some_and(|baseline| strength_xp > baseline);
        slot.ranged_gain |= slot
            .ranged_baseline
            .is_some_and(|baseline| ranged_xp > baseline);

        let at_bank = pair_near(snap, KQ_BANK, 10);
        slot.at_bank_now = at_bank;
        slot.bank_activity |= at_bank
            && (snap.bank_component_id() >= 0 || snap.shop().open || !snap.equipment().is_empty());
        let sharks = pair_inv_named(snap, "Shark");
        let equipped_arrows = snap
            .equipment()
            .iter()
            .filter(|item| item.def.id == 892)
            .map(|item| item.count)
            .sum::<i32>();
        let ropes = pair_inv_named(snap, "Rope");
        let wanted_sharks = if slot_index == 0 { 14 } else { 16 };
        let exact_pack = equipped_arrows == 250
            && sharks == wanted_sharks
            && ropes == if slot_index == 0 { 2 } else { 0 };
        slot.kit_equipped |=
            pair_equipped_ids(snap, &[1434, 1163, 2503, 2497, 2491, 1731, 1061, 2550]);
        slot.pass_ready |= pair_inv_id(snap, 1854) > 0;
        if slot_index <= 1 {
            slot.pass_bank_seen |=
                snap.bank_component_id() >= 0 && snap.bank_loaded() && pair_bank_id(snap, 1854) > 0;
            slot.pass_source_complete |= slot.pass_bank_seen && slot.pass_ready;
        } else {
            slot.pass_shop_seen |= snap.shop().open;
            slot.pass_source_complete |= slot.pass_shop_seen && slot.pass_ready;
        }
        slot.pack_ready |= at_bank && exact_pack && slot.kit_equipped && slot.pass_source_complete;
        slot.bank_departed |= slot.pack_ready && !at_bank;
        slot.boosted |= pair_effective_stat(snap, 0) > 99
            && pair_effective_stat(snap, 1) > 99
            && pair_effective_stat(snap, 2) > 99;
        if slot_index == 0 {
            self.leader_had_two_ropes |= ropes == 2;
            if self.leader_had_two_ropes && slot.surface_rope && (1..=2).contains(&ropes) {
                self.surface_rope_count.get_or_insert(ropes);
                self.surface_rope_valid = true;
            }
            if let Some(surface_count) = self.surface_rope_count {
                self.upper_rope_valid |= slot.upper_rope
                    && ropes <= surface_count
                    && ropes >= surface_count.saturating_sub(1);
            }
        }

        let peers_visible = pair_visible_peers(snap) >= 3;
        let at_surface = pair_near(
            snap,
            WorldTile {
                x: 3226,
                z: 3108,
                level: 0,
            },
            10,
        );
        let at_upper = pair_near(
            snap,
            WorldTile {
                x: 3508,
                z: 9497,
                level: 2,
            },
            12,
        );
        slot.surface_party |= at_surface && peers_visible;
        slot.upper_party |= at_upper && peers_visible;
        slot.surface_rope |= snap.locs().iter().any(|loc| loc.id == 3828);
        slot.upper_rope |= snap.locs().iter().any(|loc| loc.id == 3831);

        let in_lair = pair_near(snap, KQ_LAIR, 45);
        slot.in_lair_now = in_lair;
        if in_lair && !slot.was_in_lair {
            slot.lair_entries = slot.lair_entries.saturating_add(1);
            if slot.lair_entries == 2 {
                slot.second_entry_xp = Some((strength_xp, ranged_xp));
            }
            slot.flying_was_visible = false;
            slot.flying_zero = false;
        }
        slot.was_in_lair = in_lair;
        if slot.lair_entries >= 2 {
            slot.second_attack |= slot
                .second_entry_xp
                .is_some_and(|(strength, ranged)| strength_xp > strength || ranged_xp > ranged);
        }

        let ground = snap.npcs().iter().find(|npc| npc.r#type == Some(1158));
        let flying = snap.npcs().iter().find(|npc| npc.r#type == Some(1160));
        slot.fighting_now = in_lair
            && ground
                .map(|npc| npc.health > 0)
                .or_else(|| flying.map(|npc| npc.health > 0))
                .unwrap_or(false);
        slot.ground_alive |= in_lair && ground.is_some_and(|npc| npc.health > 0);
        slot.flying_alive |= in_lair && flying.is_some_and(|npc| npc.health > 0);
        slot.flying_zero |= in_lair && flying.is_some_and(|npc| npc.health <= 0);
        slot.flying_was_visible |= in_lair && flying.is_some();
        if in_lair && slot.flying_was_visible && flying.is_none() && ground.is_none() {
            slot.disappearances = slot.disappearances.saturating_add(1);
            if slot.flying_zero {
                slot.kills = slot.kills.saturating_add(1);
            }
            slot.flying_was_visible = false;
            slot.flying_zero = false;
        }
        let formation_at = |centre: WorldTile, size: i32, ranged: bool| {
            snap.tile().is_some_and(|(x, z, level)| {
                let centre_x = centre.x + size / 2;
                let centre_z = centre.z + size / 2;
                let (along, across) = match slot_index {
                    0 => (centre_x - x, (z - centre_z).abs()),
                    1 => (x - centre_x, (z - centre_z).abs()),
                    2 => (z - centre_z, (x - centre_x).abs()),
                    _ => (centre_z - z, (x - centre_x).abs()),
                };
                level == centre.level
                    && across == 0
                    && if ranged {
                        (3..=9).contains(&along)
                    } else {
                        along == size / 2 + 1
                    }
            })
        };
        let magic_prayer = pair_varp(snap, 95).is_some_and(|value| value != 0);
        slot.melee_formation |= in_lair
            && magic_prayer
            && ground
                .is_some_and(|npc| npc.health > 0 && formation_at(npc.network, npc.size, false));
        slot.ranged_formation |= in_lair
            && magic_prayer
            && flying
                .is_some_and(|npc| npc.health > 0 && formation_at(npc.network, npc.size, true));

        if slot.pack_ready && sharks < wanted_sharks && slot.food_eaten_at.is_none() {
            slot.food_eaten = true;
            slot.food_eaten_at = Some((strength_xp, ranged_xp));
        }
        slot.xp_after_food |= slot
            .food_eaten_at
            .is_some_and(|(strength, ranged)| strength_xp > strength || ranged_xp > ranged);

        if slot.lair_entries > 0 {
            slot.camelot_escape |= pair_near(
                snap,
                WorldTile {
                    x: 2757,
                    z: 3478,
                    level: 0,
                },
                4,
            );
            let at_duel_arena = pair_near(
                snap,
                WorldTile {
                    x: 3315,
                    z: 3235,
                    level: 0,
                },
                4,
            );
            slot.duel_escape |= at_duel_arena;
            slot.ring_spent |= at_duel_arena
                && [2554, 2556, 2558, 2560, 2562, 2564, 2566]
                    .iter()
                    .any(|id| pair_inv_id(snap, *id) > 0);
        }

        if in_lair {
            if let Some(item) = snap
                .ground_items()
                .iter()
                .find(|item| KQ_LOOT_IDS.contains(&item.def.id))
            {
                slot.loot_ground = Some(item.def.id);
            }
        }
        if let Some(id) = slot.loot_ground {
            slot.loot_collected |= pair_inv_id(snap, id) > 0;
            if snap.bank_component_id() >= 0 && snap.bank_loaded() {
                slot.loot_banked |= pair_bank_id(snap, id) > 0;
            }
        }

        let fresh_return = slot.lair_entries > slot.bank_returns
            && at_bank
            && snap.bank_component_id() >= 0
            && snap.bank_loaded();
        if fresh_return && !slot.bank_return_latched {
            slot.bank_returns = slot.bank_returns.saturating_add(1);
            slot.bank_return_latched = true;
        }
        if !at_bank {
            slot.bank_return_latched = false;
        }
        slot.safely_banked |= slot.bank_returns >= 2 && at_bank && snap.bank_component_id() < 0;

        // Safe-finish oracle: only snapshots observed after the finish
        // began count, and the closed test is the current bank state
        // (`Proof::BankClosed`: no bank component and no loaded bank;
        // `observe` already required in-game, scene 2). The latest
        // post-finish observation decides: an open bank revokes this
        // slot's credit until a later snapshot shows it closed. A
        // reopen is not latched, because one bank interaction the
        // stopped script had already sent can still land after the
        // stop; the finish's own close sends close it again, and only
        // the fresh closed observation after that counts. A script
        // that kept cycling its bank would keep revoking credit.
        if self.finish_started {
            slot.finish_observed = true;
            slot.finish_closed = snap.bank_component_id() < 0 && !snap.bank_loaded();
        }

        self.restock_overlap |= self
            .slots
            .iter()
            .any(|member| member.bank_returns >= 1 && member.at_bank_now)
            && self.slots.iter().any(|member| member.fighting_now);
        self.premature_departure |= self.slots.iter().any(|member| member.bank_departed)
            && !self.slots.iter().all(|member| member.bank_activity);
    }

    fn identities_ready(&self) -> bool {
        let names = self
            .slots
            .iter()
            .filter_map(|slot| slot.name.as_deref())
            .collect::<Vec<_>>();
        names.len() == 4
            && names
                .iter()
                .enumerate()
                .all(|(index, name)| !name.is_empty() && !names[..index].contains(name))
    }

    fn first_descent(&self) -> bool {
        self.identities_ready()
            && !self.premature_departure
            && self.leader_had_two_ropes
            && self.surface_rope_valid
            && self.upper_rope_valid
            && self.slots.iter().all(|slot| {
                slot.pack_ready
                    && slot.kit_equipped
                    && slot.pass_ready
                    && slot.pass_source_complete
                    && slot.bank_departed
                    && slot.surface_rope
                    && slot.upper_rope
                    && slot.surface_party
                    && slot.upper_party
                    && slot.lair_entries >= 1
            })
    }

    fn first_combat(&self) -> bool {
        self.slots.iter().all(|slot| {
            slot.ground_alive
                && slot.flying_alive
                && slot.strength_gain
                && slot.ranged_gain
                && slot.melee_formation
                && slot.ranged_formation
                && slot.boosted
        }) && self
            .slots
            .iter()
            .any(|slot| slot.food_eaten && slot.xp_after_food)
    }

    /// The first witnessed kill and every member's first fresh bank
    /// return. Banking a listed drop is not a term here: which drop a
    /// kill rolls is a chance outcome (see [`KQ_LOOT_IDS`]), so the
    /// fleet records it through [`Self::loot_banked_observed`] and
    /// never waits for it.
    fn first_kill_bank(&self) -> bool {
        self.slots
            .iter()
            .all(|slot| slot.disappearances >= 1 && slot.bank_returns >= 1)
            && self.slots.iter().any(|slot| slot.kills >= 1)
    }

    /// Any member was seen banking a listed Queen drop it had observed
    /// on the ground and collected. An observation for the receipt,
    /// like `restock_overlap`. It never gates the run.
    fn loot_banked_observed(&self) -> bool {
        self.slots
            .iter()
            .any(|slot| slot.loot_ground.is_some() && slot.loot_collected && slot.loot_banked)
    }

    fn second_descent(&self) -> bool {
        self.slots
            .iter()
            .all(|slot| slot.lair_entries >= 2 && slot.second_attack)
    }

    /// Every gated fleet outcome holds. `restock_overlap` and
    /// `loot_banked` are not terms here: both depend on chance, so the
    /// run records them as observations and never waits for either.
    /// The gates stay the kills, the escapes, the disappearances, and
    /// the second safe bank. The coordinated finish follows in the
    /// scenario steps.
    fn complete(&self) -> bool {
        self.first_descent()
            && self.first_combat()
            && self.first_kill_bank()
            && self.second_descent()
            && self.slots.iter().all(|slot| {
                slot.camelot_escape
                    && slot.duel_escape
                    && slot.ring_spent
                    && slot.disappearances >= 2
                    && slot.safely_banked
                    && slot.survived
            })
            && self.slots.iter().any(|slot| slot.kills >= 2)
    }

    /// Begin the coordinated safe finish. The scenario calls this only
    /// after [`Self::complete`] held and the fleet's scripts were
    /// stopped, so no slot is stopped while a teammate still needs the
    /// party, and no bank state from before the stop counts.
    fn begin_finish(&mut self) {
        self.finish_started = true;
    }

    fn finish_started(&self) -> bool {
        self.finish_started
    }

    /// The companion finish may prove only when every slot has been
    /// observed since the finish began and every companion slot's
    /// latest post-finish observation showed its bank closed. The
    /// driven slot's bank is not judged here: the scenario closes it
    /// in the next step, whose `BankClosed` arm and the scenario's
    /// terminal proof judge its real current state. Judging the
    /// companions first gives an in-flight driven bank interaction
    /// the whole companion finish to land before that close step's
    /// repeated sends run.
    fn finish_ready(&self) -> bool {
        self.finish_started
            && self.slots.iter().all(|slot| slot.finish_observed)
            && self.slots[1..].iter().all(|slot| slot.finish_closed)
    }

    /// Read-only terminal summary for the receipt: every required
    /// term still unmet, the `restock_overlap` and `loot_banked`
    /// observations, the first survival-clearing observation per
    /// slot, then each slot's last observed tile, snapshot tick, and
    /// bank component with the witness latches it still lacks. Two
    /// deadline failures (this task's first series run 1 and
    /// SCRIPT-SPLIT-INT's candidate run) stalled on the final
    /// composite witness with no retained record of which term was
    /// false. This summary rides the receipt on a failure and on a
    /// pass, so the next failure is diagnosable from the receipt
    /// alone and a pass still records its observation values.
    /// `restock_overlap` and `loot_banked` appear only as observed
    /// values. Neither appears in the unmet list or in a slot's
    /// missing list, because neither is a gate.
    ///
    /// The unmet list names the unmet fleet phases and, for the
    /// per-slot terms of [`Self::complete`] that no phase already
    /// implies, one `s{index}:{term}` entry per slot that lacks it.
    /// One integration receipt printed `unmet=[]` while `complete()`
    /// was false on one slot's `survived` (INT-BATCH-1024E). With
    /// the per-slot entries, the unmet list is empty only when
    /// `complete()` holds and the finish is not outstanding. It
    /// reads witness state only and changes nothing.
    fn terminal_summary(&self) -> String {
        let mut unmet: Vec<String> = Vec::new();
        if !self.identities_ready() {
            unmet.push("identities".to_string());
        }
        if self.premature_departure {
            unmet.push("premature_departure".to_string());
        }
        if !self.first_descent() {
            unmet.push("first_descent".to_string());
        }
        if !self.first_combat() {
            unmet.push("first_combat".to_string());
        }
        if !self.first_kill_bank() {
            unmet.push("first_kill_bank".to_string());
        }
        if !self.second_descent() {
            unmet.push("second_descent".to_string());
        }
        if !self.slots.iter().any(|slot| slot.kills >= 2) {
            unmet.push("any_kills>=2".to_string());
        }
        if self.finish_started && !self.finish_ready() {
            unmet.push("safe_finish".to_string());
        }
        // The per-slot conjuncts of `complete()`, qualified by slot.
        // The phases above already imply the other per-slot latches
        // (packs, ropes, parties, combat gains, second attacks), so
        // those stay in each slot's own missing list below.
        for (index, slot) in self.slots.iter().enumerate() {
            for (met, name) in [
                (slot.camelot_escape, "camelot_escape"),
                (slot.duel_escape, "duel_escape"),
                (slot.ring_spent, "ring_spent"),
                (slot.disappearances >= 2, "disappearances>=2"),
                (slot.safely_banked, "safely_banked"),
                (slot.survived, "survived"),
            ] {
                if !met {
                    unmet.push(format!("s{index}:{name}"));
                }
            }
        }
        let mut survival_cleared: Vec<String> = Vec::new();
        for (index, slot) in self.slots.iter().enumerate() {
            if let Some(clear) = &slot.survival_cleared {
                let tile = match clear.tile {
                    Some((x, z, level)) => format!("{x},{z},{level}"),
                    None => "?".to_string(),
                };
                survival_cleared.push(format!(
                    "s{index}:tick={}:hp={}:tile={tile}:chat={:?}",
                    clear.tick, clear.hp, clear.chat,
                ));
            }
        }
        let survival_cleared = if survival_cleared.is_empty() {
            "none".to_string()
        } else {
            survival_cleared.join(";")
        };
        let mut slots = String::new();
        for (index, slot) in self.slots.iter().enumerate() {
            let mut missing: Vec<String> = Vec::new();
            let mut want = |met: bool, name: &str| {
                if !met {
                    missing.push(name.to_string());
                }
            };
            want(slot.pack_ready, "pack");
            want(slot.kit_equipped, "kit");
            want(slot.pass_ready, "pass");
            want(slot.pass_source_complete, "pass_source");
            want(slot.bank_departed, "departed");
            want(slot.surface_rope, "surface_rope");
            want(slot.upper_rope, "upper_rope");
            want(slot.surface_party, "surface_party");
            want(slot.upper_party, "upper_party");
            want(slot.ground_alive, "ground_seen");
            want(slot.flying_alive, "flying_seen");
            want(slot.strength_gain, "strength_xp");
            want(slot.ranged_gain, "ranged_xp");
            want(slot.melee_formation, "melee_formation");
            want(slot.ranged_formation, "ranged_formation");
            want(slot.boosted, "boosted");
            want(slot.second_attack, "second_attack");
            want(slot.camelot_escape, "camelot_escape");
            want(slot.duel_escape, "duel_escape");
            want(slot.ring_spent, "ring_spent");
            want(slot.disappearances >= 2, "disappearances>=2");
            want(slot.safely_banked, "safely_banked");
            want(slot.survived, "survived");
            if self.finish_started && !slot.finish_closed {
                missing.push("finish_bank_closed".to_string());
            }
            let (tile, bank) = match slot.last_tile {
                Some((x, z, level)) => (
                    format!("{x},{z},{level}"),
                    slot.last_bank_component.to_string(),
                ),
                None => ("?".to_string(), "?".to_string()),
            };
            slots.push_str(&format!(
                " s{index}<{}> tile={tile} tick={} bank={bank} entries={} returns={} disp={} kills={} missing=[{}]",
                slot.name.as_deref().unwrap_or("?"),
                slot.last_tick,
                slot.lair_entries,
                slot.bank_returns,
                slot.disappearances,
                slot.kills,
                missing.join(" "),
            ));
        }
        format!(
            "kq_witness unmet=[{}] restock_overlap={} loot_banked={} survival_cleared={} ropes(leader2={} surface={} upper={}){}",
            unmet.join(" "),
            self.restock_overlap,
            self.loot_banked_observed(),
            survival_cleared,
            self.leader_had_two_ropes,
            self.surface_rope_valid,
            self.upper_rope_valid,
            slots,
        )
    }
}

/// Per-frame state for the trade-accept companion (profile 1).
#[derive(Default)]
struct TradeAcceptSlot {
    scene2_seen: bool,
    tele_sent: bool,
}

/// The `script_trade` scenario: a two-bot fleet — both profiles Start the
/// in-tree `TradeBot` fixture (Compat `Trade.request` / `offerAll` /
/// `accept`) with reciprocal `partner` inject; profile 1 also rust-teles
/// beside the driven bot after the mainland hop and presses Accept on
/// both trade screens. Proof: the driven slot holds zero Coins after
/// offering the seeded stack of twenty-five.
pub(crate) fn script_trade_scenario() -> Scenario {
    let courtyard = TRADE_COURTYARD;
    Scenario {
        name: "script_trade",
        seed: Seed {
            profiles: vec![("test", "test"), ("test2", "test2")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "stick tutorial skip and seed twenty-five coins",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "setvar tutorial 1000");
                        cheat(c, "getvar tutorial");
                        cheat(c, "give coins 25");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Chat {
                        needle: "get tutorial: 1000",
                    },
                    budget_ticks: 200,
                },
            },
            Step {
                name: "relog so the inv tab binds",
                kind: StepKind::Relog,
                wait: Wait {
                    arm: Proof::SideTabAvailable { index: 3 },
                    budget_ticks: 600,
                },
            },
            Step {
                name: "tele to Lumbridge courtyard",
                kind: StepKind::Perform {
                    send: Box::new(move |c, _| {
                        cheat(c, &tele_args(courtyard.level, courtyard.x, courtyard.z));
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Arrived {
                        x: courtyard.x,
                        z: courtyard.z,
                        level: courtyard.level,
                    },
                    budget_ticks: 120,
                },
            },
            start_catalog_step(),
            Step {
                name: "watch the trade consume coins",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::ItemAtMost {
                        name: "Coins",
                        count: 0,
                    },
                    budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
                },
            },
        ],
        proof: Proof::ItemAtMost {
            name: "Coins",
            count: 0,
        },
        companions: vec![Companion {
            profile: 1,
            per_frame: {
                let mut slot = TradeAcceptSlot::default();
                Box::new(move |c| trade_acceptor_frame(c, &mut slot))
            },
        }],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("TradeBot"),
            inject_companion_as: Some("partner"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

fn trade_acceptor_frame(c: &mut Client, s: &mut TradeAcceptSlot) {
    let Some(lp) = &c.local_player else {
        if debug_enabled() {
            eprintln!("[trade-companion] no local_player scene={}", c.scene_state);
        }
        return;
    };
    let here = WorldTile {
        x: c.map_build_base_x + lp.route_x[0],
        z: c.map_build_base_z + lp.route_z[0],
        level: 0,
    };
    if stage_trade_companion_tele(c, here, s) {
        return;
    }
    press_trade_accept(c);
}

/// Host-play queues `mainland_hop` after the first `scene_state == 2`
/// companion frame; cheat-tele beside the driven bot once ingame on a
/// mainland build base (the runner seed gate), not only when `here.x > 3100`.
fn stage_trade_companion_tele(c: &mut Client, here: WorldTile, s: &mut TradeAcceptSlot) -> bool {
    if at_trade_courtyard(here) || s.tele_sent {
        return false;
    }
    if c.scene_state != 2 {
        return false;
    }
    if !s.scene2_seen {
        s.scene2_seen = true;
        return false;
    }
    if c.map_build_base_x < 3000 {
        if debug_enabled() {
            eprintln!(
                "[trade-companion] waiting mainland base, here={here:?} base_x={}",
                c.map_build_base_x
            );
        }
        return false;
    }
    if debug_enabled() {
        eprintln!(
            "[trade-companion] tele {} from {here:?}",
            tele_args(TRADE_COURTYARD.level, TRADE_COURTYARD.x, TRADE_COURTYARD.z)
        );
    }
    cheat(
        c,
        &tele_args(TRADE_COURTYARD.level, TRADE_COURTYARD.x, TRADE_COURTYARD.z),
    );
    s.tele_sent = true;
    true
}

fn at_trade_courtyard(here: WorldTile) -> bool {
    here.x == TRADE_COURTYARD.x
        && here.z == TRADE_COURTYARD.z
        && here.level == TRADE_COURTYARD.level
}

/// Fail-closed when the trade screen is open but the posted accept
/// component id is absent (companion cannot press Accept).
pub(crate) fn trade_accept_missing_block(trade: &api::snapshot::TradeView) -> Option<&'static str> {
    if (trade.offer_open || trade.confirm_open) && trade.accept_component_id < 0 {
        Some("BLOCKED: missing trade accept com")
    } else {
        None
    }
}

fn press_trade_accept(c: &mut Client) {
    let mut snap = GameSnapshot::default();
    snap.rebuild(c);
    let trade = snap.trade();
    if let Some(msg) = trade_accept_missing_block(trade) {
        fail(msg);
    }
    if !trade.offer_open && !trade.confirm_open {
        return;
    }
    let mut ix = Interactions::new(&snap, c);
    let ctx = ReadContext::new(&snap);
    let Some(widget) = ctx.component(trade.accept_component_id) else {
        return;
    };
    match ix.press(widget) {
        SendResult::Sent { .. } | SendResult::Refused { .. } => {}
    }
}

#[derive(Clone, Copy)]
enum PairCompanionKind {
    AirRunner,
    MuleMule,
    FlaxSpinner,
    DuelPeer,
    ClueHelper,
    KqMember,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PairPrepStage {
    WaitMainland,
    TutSkip,
    Relog,
    WaitRelog,
    ReseedTutorial,
    WaitReseedConfirm,
    Seed,
    WaitSeed,
    AckBank,
    CloseBank,
    FirstLoad,
    WaitReady,
    Wear,
    WaitWear,
    Idle,
}

struct PairCompanionSlot {
    kind: PairCompanionKind,
    profile: usize,
    stage: PairPrepStage,
    logout_sent: bool,
    kq_has_pass: bool,
    saw_logout: bool,
    logout_session: u64,
    seed_sent: bool,
    seed_scene_generation: Option<u64>,
    first_load_sent: bool,
    last_action: Instant,
    clue_witness: Option<Arc<Mutex<ClueFleetWitness>>>,
    kq_witness: Option<Arc<Mutex<KqFleetWitness>>>,
    prep_barrier: Option<PairPrepBarrier>,
    tutorial_reseed: Option<crate::tutorial::PostRelogTutorial>,
}

impl PairCompanionSlot {
    fn new(kind: PairCompanionKind) -> Self {
        Self {
            profile: 1,
            kind,
            stage: PairPrepStage::WaitMainland,
            kq_has_pass: true,
            logout_sent: false,
            saw_logout: false,
            logout_session: 0,
            seed_sent: false,
            seed_scene_generation: None,
            first_load_sent: false,
            last_action: Instant::now() - Duration::from_secs(1),
            clue_witness: None,
            kq_witness: None,
            prep_barrier: None,
            tutorial_reseed: None,
        }
    }
}

fn pair_companion_frame(c: &mut Client, slot: &mut PairCompanionSlot) {
    let mut snap = GameSnapshot::default();
    snap.rebuild(c);
    let now = Instant::now();
    let inv_tab = snap
        .side_tabs()
        .iter()
        .any(|tab| tab.index == 3 && tab.available);
    match slot.stage {
        PairPrepStage::WaitMainland => {
            if c.ingame && c.scene_state == 2 && c.map_build_base_x >= 3000 {
                slot.stage = PairPrepStage::TutSkip;
            }
        }
        PairPrepStage::TutSkip => {
            if !send_ok(slot, now) {
                return;
            }
            cheat(c, crate::tutorial::TUTORIAL_SETVAR);
            cheat(c, crate::tutorial::TUTORIAL_GETVAR);
            slot.last_action = now;
            slot.stage = PairPrepStage::Relog;
        }
        PairPrepStage::Relog => {
            if c.ingame && !slot.logout_sent {
                if !send_ok(slot, now) {
                    return;
                }
                let ifaces = std::sync::Arc::clone(&c.ifaces);
                if logout(c, ifaces.as_slice()) {
                    slot.logout_sent = true;
                    slot.logout_session = c.gens.session;
                    slot.last_action = now;
                    slot.stage = PairPrepStage::WaitRelog;
                }
            }
        }
        PairPrepStage::WaitRelog => {
            // The 289 engine retries pending `player_logout` saves every
            // 15s (`engine/src/engine/World.ts:795-803`) and rejects a login
            // while that save is pending (`World.ts:812-821`). The host paces a
            // same-world clean relog from its exit and probes response 5 at 5s
            // intervals within a 30s budget, under shared admission limits
            // (`crates/host-play/src/play_login.rs`, `clean_release_admission_at`
            // and `defer_clean_release_at`); generic fallback still waits 60s.
            // Sending logout or sleeping is therefore not release evidence.
            // Require the successful new session to publish scene2 and the
            // inventory tab before seeding.
            let logout_observed = pair_relog_seen(
                slot.logout_sent,
                slot.saw_logout,
                snap.ingame(),
                slot.logout_session,
                c.gens.session,
            );
            if logout_observed {
                slot.saw_logout = true;
                if !snap.ingame() {
                    return;
                }
            }
            if pair_relog_ready(slot.saw_logout, snap.ingame(), snap.scene_state(), inv_tab) {
                if matches!(slot.kind, PairCompanionKind::DuelPeer) {
                    slot.stage = PairPrepStage::ReseedTutorial;
                } else {
                    slot.stage = PairPrepStage::Seed;
                }
            }
        }
        PairPrepStage::ReseedTutorial => {
            if !send_ok(slot, now) {
                return;
            }
            // Fresh-account post-relog reseed: the kit-close queue has
            // already run in the new session. The DuelPeer wields a bronze
            // scimitar below, which needs `tutorial > 400` for the tab.
            let baseline = crate::tutorial::chat_baseline(&snap);
            cheat(c, crate::tutorial::TUTORIAL_SETVAR);
            cheat(c, crate::tutorial::TUTORIAL_GETVAR);
            slot.tutorial_reseed = Some(crate::tutorial::PostRelogTutorial::new(baseline));
            slot.last_action = now;
            slot.stage = PairPrepStage::WaitReseedConfirm;
        }
        PairPrepStage::WaitReseedConfirm => {
            let confirmed = match slot.tutorial_reseed.as_ref() {
                Some(reseed) => match reseed.check(&snap) {
                    Ok(confirmed) => confirmed,
                    Err(error) => {
                        // No fresh reply within the helper's bound: re-arm
                        // with a new baseline rather than wedging. The
                        // driven slot's own fresh seed still gates Start.
                        println!("{error}");
                        slot.stage = PairPrepStage::ReseedTutorial;
                        return;
                    }
                },
                None => false,
            };
            if confirmed {
                println!("{}", crate::tutorial::confirmation_log());
                slot.stage = PairPrepStage::Seed;
            } else if send_ok(slot, now) {
                cheat(c, crate::tutorial::TUTORIAL_GETVAR);
                slot.last_action = now;
            }
        }
        PairPrepStage::Seed => {
            if !send_ok(slot, now) {
                return;
            }
            if !slot.seed_sent {
                // Capture the last pre-teleport scene generation. Tile/stat
                // updates can arrive before the teleport's scene rebuild; the
                // paired Start barrier must not admit that transient snapshot.
                slot.seed_scene_generation = Some(c.gens.scene);
                cheat(c, "~clearinv");
                match slot.kind {
                    PairCompanionKind::AirRunner => {
                        cheat(c, &format!("givebank blankrune {RUNE_ESSENCE_SEED}"));
                        cheat(
                            c,
                            &tele_args(
                                FALADOR_EAST_BANK.level,
                                FALADOR_EAST_BANK.x,
                                FALADOR_EAST_BANK.z,
                            ),
                        );
                    }
                    PairCompanionKind::MuleMule => {
                        cheat(c, &format!("givebank blankrune {RUNE_ESSENCE_SEED}"));
                        cheat(
                            c,
                            &tele_args(
                                FALADOR_EAST_BANK.level,
                                FALADOR_EAST_BANK.x,
                                FALADOR_EAST_BANK.z,
                            ),
                        );
                    }
                    PairCompanionKind::FlaxSpinner => {
                        cheat(c, "setstat crafting 10");
                        cheat(c, &tele_args(FLAX_MEET.level, FLAX_MEET.x, FLAX_MEET.z));
                    }
                    PairCompanionKind::DuelPeer => {
                        cheat(c, "give bronze_scimitar 1");
                        cheat(
                            c,
                            &tele_args(DUEL_CHALLENGE.level, DUEL_CHALLENGE.x, DUEL_CHALLENGE.z),
                        );
                    }
                    PairCompanionKind::ClueHelper => {
                        cheat(
                            c,
                            &tele_args(DUEL_CHALLENGE.level, DUEL_CHALLENGE.x, DUEL_CHALLENGE.z),
                        );
                    }
                    PairCompanionKind::KqMember => {
                        seed_kq_profile(c, slot.kq_has_pass, KQ_MEMBER_ROPES)
                    }
                }
                slot.seed_sent = true;
                slot.last_action = now;
            }
            slot.stage = PairPrepStage::WaitSeed;
        }
        PairPrepStage::WaitSeed => match slot.kind {
            PairCompanionKind::AirRunner | PairCompanionKind::MuleMule => {
                if pair_near(&snap, FALADOR_EAST_BANK, 8) {
                    slot.stage = PairPrepStage::AckBank;
                }
            }
            PairCompanionKind::FlaxSpinner => {
                if pair_near(&snap, FLAX_MEET, 8)
                    && pair_stat(&snap, CRAFTING_STAT) >= 10
                    && pair_inv_id(&snap, FLAX_ID) == 0
                    && pair_inv_id(&snap, BOW_STRING_ID) == 0
                {
                    slot.stage = PairPrepStage::Idle;
                }
            }
            PairCompanionKind::DuelPeer => {
                if pair_near(&snap, DUEL_CHALLENGE, 8) && pair_inv_any(&snap) {
                    slot.stage = PairPrepStage::Wear;
                }
            }
            PairCompanionKind::ClueHelper => {
                if post_seed_scene_ready(slot.seed_scene_generation, c.gens.scene, c.scene_state)
                    && pair_near(&snap, DUEL_CHALLENGE, 8)
                {
                    slot.stage = PairPrepStage::Idle;
                }
            }
            PairCompanionKind::KqMember => {
                if post_seed_scene_ready(slot.seed_scene_generation, c.gens.scene, c.scene_state)
                    && pair_near(&snap, KQ_BANK, 8)
                    && pair_stat(&snap, 0) >= 99
                {
                    slot.stage = PairPrepStage::Idle;
                }
            }
        },
        PairPrepStage::AckBank => {
            if pair_bank_id(&snap, RUNE_ESSENCE_ID) >= RUNE_ESSENCE_SEED
                && snap.bank_loaded()
                && snap.bank_component_id() >= 0
                && pair_near(&snap, FALADOR_EAST_BANK, 8)
            {
                slot.stage = PairPrepStage::CloseBank;
                return;
            }
            if !send_ok(slot, now) {
                return;
            }
            match Interactions::new(&snap, c).open_nearest_booth() {
                SendResult::Sent { .. } | SendResult::Refused { .. } => {}
            }
            slot.last_action = now;
        }
        PairPrepStage::CloseBank => {
            if snap.bank_component_id() < 0 {
                slot.stage = PairPrepStage::FirstLoad;
                return;
            }
            if !send_ok(slot, now) {
                return;
            }
            match Interactions::new(&snap, c).close_modal() {
                SendResult::Sent { .. } | SendResult::Refused { .. } => {}
            }
            slot.last_action = now;
        }
        PairPrepStage::FirstLoad => {
            if !send_ok(slot, now) {
                return;
            }
            if !slot.first_load_sent {
                let n = match slot.kind {
                    PairCompanionKind::AirRunner => PAIR_AIR_FIRST_LOAD,
                    PairCompanionKind::MuleMule => PAIR_MULE_FIRST_LOAD,
                    PairCompanionKind::FlaxSpinner
                    | PairCompanionKind::DuelPeer
                    | PairCompanionKind::ClueHelper
                    | PairCompanionKind::KqMember => 0,
                };
                cheat(c, &format!("give blankrune {n}"));
                cheat(
                    c,
                    &tele_args(
                        MULECRAFTER_AIR_RUINS.level,
                        MULECRAFTER_AIR_RUINS.x,
                        MULECRAFTER_AIR_RUINS.z,
                    ),
                );
                slot.first_load_sent = true;
                slot.last_action = now;
            }
            slot.stage = PairPrepStage::WaitReady;
        }
        PairPrepStage::WaitReady => {
            let want = match slot.kind {
                PairCompanionKind::AirRunner => PAIR_AIR_FIRST_LOAD,
                PairCompanionKind::MuleMule => PAIR_MULE_FIRST_LOAD,
                PairCompanionKind::FlaxSpinner
                | PairCompanionKind::DuelPeer
                | PairCompanionKind::ClueHelper
                | PairCompanionKind::KqMember => 0,
            };
            if pair_near(&snap, MULECRAFTER_AIR_RUINS, 8)
                && pair_inv_id(&snap, RUNE_ESSENCE_ID) >= want
                && snap.bank_component_id() < 0
            {
                slot.stage = PairPrepStage::Idle;
            }
        }
        PairPrepStage::Wear => {
            if !send_ok(slot, now) {
                return;
            }
            pair_wear_first_inv(c, &snap);
            slot.last_action = now;
            slot.stage = PairPrepStage::WaitWear;
        }
        PairPrepStage::WaitWear => {
            if pair_near(&snap, DUEL_CHALLENGE, 8)
                && pair_weapon_equipped(&snap)
                && snap.modals().main < 0
            {
                slot.stage = PairPrepStage::Idle;
            } else if send_ok(slot, now) {
                pair_wear_first_inv(c, &snap);
                slot.last_action = now;
            }
        }
        PairPrepStage::Idle => {}
    }
    if slot.stage == PairPrepStage::Idle {
        if let Some(barrier) = &slot.prep_barrier {
            barrier.mark_ready(slot.profile);
        }
    }
    if let Some(witness) = &slot.clue_witness {
        witness
            .lock()
            .expect("clue witness mutex")
            .observe(1, &snap);
    }
    if let Some(witness) = &slot.kq_witness {
        let finish_started = {
            let mut witness = witness.lock().expect("KQ witness mutex");
            witness.observe(slot.profile, &snap);
            witness.finish_started()
        };
        // During the safe finish the fleet's scripts are stopped, so a
        // bank this companion still has open stays open until it is
        // closed here. Before the finish this frame never sends a
        // close: the running script owns its bank.
        if finish_started && snap.bank_component_id() >= 0 && send_ok(slot, now) {
            match Interactions::new(&snap, c).close_modal() {
                SendResult::Sent { .. } | SendResult::Refused { .. } => {}
            }
            slot.last_action = now;
        }
    }
}

fn post_seed_scene_ready(
    seed_scene_generation: Option<u64>,
    scene_generation: u64,
    scene_state: i32,
) -> bool {
    seed_scene_generation.is_some_and(|seed| scene_generation != seed) && scene_state == 2
}

fn send_ok(slot: &PairCompanionSlot, now: Instant) -> bool {
    now.duration_since(slot.last_action) >= Duration::from_millis(400)
}

/// Ropes the leader's bank holds; it withdraws two for every descent. With
/// four banked, KQ witness 14 (0.1.9) failed: its second group retreated on
/// the food reserve before the second kill, a third descent still fit the
/// 1800 s deadline, and the leader stopped on "KQ bank needs 2 more of item
/// 954". Eight still covers [`KQ_WATCHDOG`]: the fastest observed full
/// descent cycle is 908 leader ticks (diagnosis run 3, trip 2: bank 364,
/// walk 38, surface rope 17, upper cave 177, lair 206, escape 22, return
/// walk 84), and the first lair entry was observed at leader tick 567, so
/// a fifth entry needs at least 567 + 4×908 = 4,199 ticks, past the
/// watchdog's 3,490 ticks (2,094 s at 600 ms). Four descents at two ropes each is
/// the most the watchdog allows; Beta 1 qualification starts from here.
const KQ_LEADER_ROPES: u32 = 8;
/// The other members never withdraw ropes.
const KQ_MEMBER_ROPES: u32 = 4;

fn seed_kq_profile(c: &mut Client, has_pass: bool, ropes: u32) {
    cheat(c, "~clearinv");
    cheat(c, "setvar heroquest 15");
    for skill in [
        "attack",
        "strength",
        "defence",
        "ranged",
        "hitpoints",
        "prayer",
        "magic",
    ] {
        cheat(c, &format!("setstat {skill} 99"));
    }
    for (item, count) in [
        ("dragon_mace", 2),
        ("rune_full_helm", 2),
        ("black_dragonhide_body", 2),
        ("black_dragonhide_chaps", 2),
        ("black_dragon_vambraces", 2),
        ("amulet_of_power", 2),
        ("leather_boots", 2),
        ("ring_of_recoil", 4),
        ("rune_arrow", 1000),
        ("magic_shortbow", 2),
        ("rope", ropes),
        ("4doseprayerrestore", 8),
        ("4dose2antipoison", 4),
        ("4dose2attack", 4),
        ("4dose2strength", 4),
        ("4dose2defense", 4),
        ("ring_of_dueling_8", 4),
        ("airrune", 100),
        ("lawrune", 20),
        ("shark", 100),
        ("coins", 1000),
    ] {
        cheat(c, &format!("givebank {item} {count}"));
    }
    if has_pass {
        cheat(c, "givebank shantay_pass 4");
    }
    cheat(c, &tele_args(KQ_BANK.level, KQ_BANK.x, KQ_BANK.z));
}

fn pair_relog_seen(
    logout_sent: bool,
    saw_logout: bool,
    ingame: bool,
    logout_session: u64,
    session: u64,
) -> bool {
    saw_logout || !ingame || (logout_sent && session != logout_session)
}

fn pair_relog_ready(saw_logout: bool, ingame: bool, scene_state: i32, inv_tab: bool) -> bool {
    saw_logout && ingame && scene_state == 2 && inv_tab
}

fn pair_near(snap: &GameSnapshot, dest: WorldTile, radius: i32) -> bool {
    snap.tile().is_some_and(|(x, z, level)| {
        level == dest.level && (x - dest.x).abs() <= radius && (z - dest.z).abs() <= radius
    })
}

fn pair_inv_id(snap: &GameSnapshot, id: i32) -> i32 {
    snap.inv()
        .iter()
        .filter(|(item_id, _)| *item_id == id)
        .map(|(_, count)| *count)
        .sum()
}

fn pair_inv_any(snap: &GameSnapshot) -> bool {
    snap.inv().iter().any(|(_, count)| *count > 0)
}

fn pair_weapon_equipped(snap: &GameSnapshot) -> bool {
    snap.equipment()
        .iter()
        .any(|item| item.count > 0 && item.def.id > 0)
}

fn pair_wear_first_inv(c: &mut Client, snap: &GameSnapshot) {
    let Some((id, _)) = snap.inv().iter().copied().find(|(_, count)| *count > 0) else {
        return;
    };
    match Interactions::new(snap, c).wear(id) {
        SendResult::Sent { .. } | SendResult::Refused { .. } => {}
    }
}

fn pair_bank_id(snap: &GameSnapshot, id: i32) -> i32 {
    snap.bank()
        .iter()
        .filter(|row| row.def.id == id)
        .map(|row| row.count)
        .sum()
}

fn pair_stat(snap: &GameSnapshot, id: i32) -> i32 {
    snap.stats()
        .iter()
        .find(|row| row.index == id)
        .map(|row| row.base)
        .unwrap_or(0)
}
fn pair_local_name(snap: &GameSnapshot) -> Option<&str> {
    snap.local_player()
        .and_then(|local| local.player.actor.name.as_deref())
}

/// Two posted player names are the same account: case and `_`/space
/// spelling differ between the login name and the displayed one.
fn same_player(a: &str, b: &str) -> bool {
    fn fold(name: &str) -> impl Iterator<Item = char> + '_ {
        name.trim().chars().map(|c| {
            if c == '_' {
                ' '
            } else {
                c.to_ascii_lowercase()
            }
        })
    }
    fold(a).eq(fold(b))
}

fn pair_effective_stat(snap: &GameSnapshot, id: i32) -> i32 {
    snap.stats()
        .iter()
        .find(|row| row.index == id)
        .map(|row| row.effective)
        .unwrap_or(0)
}

/// Hitpoints (stat 3) observed from stats the server has sent since
/// login, or `None` when this snapshot carries no such observation.
///
/// The received signal is the snapshot's own: the native snapshot
/// treats the stat table as observed only when it is non-empty and no
/// used skill posts base 0 (`crates/api/src/snapshot/native.rs`,
/// `NativeSnapshot::stats`). That test separates a sent table from the
/// client's initial state, because the client arrays start at 0
/// (`vendor/fr-client-rust/crates/client/src/client/client.rs`,
/// `stat_base_level: vec![0; Skill::count]`) and a stat packet writes
/// a base of at least 1 for the skill it carries (the same file's
/// `UPDATE_STAT` arm: `let mut base: i32 = 1`). A stat rebuild alone
/// does not prove receipt: it copies all 25 rows from those arrays
/// (`crates/api/src/snapshot/decode.rs`, `rebuild_stat`), zeros
/// included, and a scene rebuild re-arms it. A dead account still
/// passes the signal: death sets hitpoints effective 0, never its
/// base. Unlike [`pair_effective_stat`], an absent hitpoints row is
/// missing data here, not 0.
fn pair_observed_hp(snap: &GameSnapshot) -> Option<i32> {
    let stats = snap.stats();
    if stats.is_empty() || stats.iter().any(|row| row.used && row.base <= 0) {
        return None;
    }
    stats
        .iter()
        .find(|row| row.index == 3 && row.used && row.base > 0)
        .map(|row| row.effective)
}

fn pair_xp(snap: &GameSnapshot, id: i32) -> i32 {
    snap.stats()
        .iter()
        .find(|row| row.index == id)
        .map(|row| row.xp)
        .unwrap_or(0)
}

fn pair_varp(snap: &GameSnapshot, id: i32) -> Option<i32> {
    snap.varps()
        .iter()
        .find(|varp| varp.index == id)
        .map(|varp| varp.value)
}

fn pair_inv_named(snap: &GameSnapshot, name: &str) -> i32 {
    snap.inventory()
        .iter()
        .filter(|item| item.def.name.as_deref() == Some(name))
        .map(|item| item.count)
        .sum()
}

fn pair_equipped_ids(snap: &GameSnapshot, ids: &[i32]) -> bool {
    ids.iter().all(|id| {
        snap.equipment()
            .iter()
            .any(|item| item.def.id == *id && item.count > 0)
    })
}

fn pair_visible_peers(snap: &GameSnapshot) -> usize {
    snap.players()
        .iter()
        .filter(|player| {
            player
                .actor
                .name
                .as_deref()
                .is_some_and(|name| !name.is_empty())
        })
        .count()
}

fn pair_fleet_seed() -> Seed {
    Seed {
        profiles: vec![("test", "test"), ("test2", "test2")],
        mainland: true,
    }
}

fn pair_watch_settings(name: &'static str, start_script: &'static str) -> ScenarioSettings {
    pair_watch_settings_with_deadline(name, start_script, SCRIPT_GOLD_DEADLINE)
}

fn pair_watch_settings_with_deadline(
    name: &'static str,
    start_script: &'static str,
    deadline: Duration,
) -> ScenarioSettings {
    ScenarioSettings {
        full_rate: true,
        only_render_selected: false,
        require_mainland_base: true,
        deadline,
        start_script: Some(start_script),
        terminal_shot: Some(name),
        nav: gold_script_nav(),
        ..Default::default()
    }
}

fn pair_companion(kind: PairCompanionKind) -> Companion {
    Companion {
        profile: 1,
        per_frame: {
            let mut slot = PairCompanionSlot::new(kind);
            Box::new(move |c| pair_companion_frame(c, &mut slot))
        },
    }
}
fn pair_prep_companion(kind: PairCompanionKind, prep_barrier: PairPrepBarrier) -> Companion {
    Companion {
        profile: 1,
        per_frame: {
            let mut slot = PairCompanionSlot::new(kind);
            slot.prep_barrier = Some(prep_barrier);
            Box::new(move |c| pair_companion_frame(c, &mut slot))
        },
    }
}
fn clue_witness_companion(
    witness: Arc<Mutex<ClueFleetWitness>>,
    prep_barrier: PairPrepBarrier,
) -> Companion {
    Companion {
        profile: 1,
        per_frame: {
            let mut slot = PairCompanionSlot::new(PairCompanionKind::ClueHelper);
            slot.clue_witness = Some(witness);
            slot.prep_barrier = Some(prep_barrier);
            Box::new(move |c| pair_companion_frame(c, &mut slot))
        },
    }
}

fn kq_witness_companion(
    profile: usize,
    witness: Arc<Mutex<KqFleetWitness>>,
    prep_barrier: PairPrepBarrier,
) -> Companion {
    Companion {
        profile,
        per_frame: {
            let mut slot = PairCompanionSlot::new(PairCompanionKind::KqMember);
            slot.profile = profile;
            slot.kq_has_pass = profile <= 1;
            slot.kq_witness = Some(witness);
            slot.prep_barrier = Some(prep_barrier);
            Box::new(move |c| pair_companion_frame(c, &mut slot))
        },
    }
}

/// NatureCrafter Air Master/Runner: two visible slots, shared Start after
/// both native preps. Pair watch supplies complementary mode/partner bags.
pub(crate) fn nature_crafter_air_scenario() -> Scenario {
    let ruins = MULECRAFTER_AIR_RUINS;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Air master talisman at ruins before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "give air_talisman 1");
                cheat(c, &tele_args(ruins.level, ruins.x, ruins.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: ruins.x,
                z: ruins.z,
                level: ruins.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Air talisman in pack before Start",
            Proof::ItemId {
                id: AIR_TALISMAN_ID,
                count: 1,
            },
        ),
        (
            "confirm no seeded essence in pack before Start",
            Proof::ItemIdAtMost {
                id: RUNE_ESSENCE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted essence in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_ESSENCE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded Air runes in pack before Start",
            Proof::ItemIdAtMost {
                id: AIR_RUNE_ID,
                count: 0,
            },
        ),
        ("confirm seed bank closed before Start", Proof::BankClosed),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    Scenario {
        name: "nature_crafter_air",
        seed: pair_fleet_seed(),
        steps,
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![pair_companion(PairCompanionKind::AirRunner)],
        settings: pair_watch_settings("nature_crafter_air", "NatureCrafter"),
    }
}

/// MuleCrafter Air Crafter/Mule: two visible slots, shared Start after both
/// native preps. bankFill=true is the pair_settings default, not this cell.
pub(crate) fn mule_crafter_air_scenario() -> Scenario {
    let ruins = MULECRAFTER_AIR_RUINS;
    let mut steps = script_live_seed_steps();
    steps.push(native_bank_seed(
        "seed Mule crafter's exact noted essence bank before Start",
        FALADOR_EAST_BANK,
        vec![NativeSeed {
            unnoted_id: RUNE_ESSENCE_ID,
            debug_alias: "blankrune",
            note_alias: Some("cert_blankrune"),
            quantity: RUNE_ESSENCE_SEED,
            note_id: Some(NOTED_ESSENCE_ID),
        }],
        "runecraft",
        1,
    ));
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the Mule crafter essence bank",
        Proof::BankItemIdAtMost {
            id: NOTED_ESSENCE_ID,
            count: 0,
        },
    ));
    steps.extend(native_bank_deposit(
        "deposit the Mule crafter noted essence through the bank window",
        vec![NativeSeed {
            unnoted_id: RUNE_ESSENCE_ID,
            debug_alias: "blankrune",
            note_alias: Some("cert_blankrune"),
            quantity: RUNE_ESSENCE_SEED,
            note_id: Some(NOTED_ESSENCE_ID),
        }],
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(Step {
        name: "seed Mule crafter talisman and first 27 essence at ruins before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "give air_talisman 1");
                cheat(c, &format!("give blankrune {PAIR_MULE_FIRST_LOAD}"));
                cheat(c, &tele_args(ruins.level, ruins.x, ruins.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: ruins.x,
                z: ruins.z,
                level: ruins.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Air talisman in pack before Start",
            Proof::ItemId {
                id: AIR_TALISMAN_ID,
                count: 1,
            },
        ),
        (
            "confirm exactly one unnoted 1436 load of 27 before Start",
            Proof::ItemId {
                id: RUNE_ESSENCE_ID,
                count: PAIR_MULE_FIRST_LOAD,
            },
        ),
        (
            "confirm no extra unnoted essence beyond the first load",
            Proof::ItemIdAtMost {
                id: RUNE_ESSENCE_ID,
                count: PAIR_MULE_FIRST_LOAD,
            },
        ),
        (
            "confirm no seeded noted essence in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_ESSENCE_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded Air runes in pack before Start",
            Proof::ItemIdAtMost {
                id: AIR_RUNE_ID,
                count: 0,
            },
        ),
        ("confirm seed bank closed before Start", Proof::BankClosed),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    Scenario {
        name: "mule_crafter_air",
        seed: pair_fleet_seed(),
        steps,
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![pair_companion(PairCompanionKind::MuleMule)],
        settings: pair_watch_settings("mule_crafter_air", "MuleCrafter"),
    }
}

/// FlaxRunner Runner/Spinner: empty packs, runner at the field, spinner at
/// the meet. First flax pack is picked after shared Start.
pub(crate) fn flax_runner_scenario() -> Scenario {
    let field = FLAX_FIELD;
    let prep_barrier = PairPrepBarrier::new(1);
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed empty Flax runner pack at the field before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &tele_args(field.level, field.x, field.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: field.x,
                z: field.z,
                level: field.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm no seeded flax in pack before Start",
            Proof::ItemIdAtMost {
                id: FLAX_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded bow string in pack before Start",
            Proof::ItemIdAtMost {
                id: BOW_STRING_ID,
                count: 0,
            },
        ),
        ("confirm seed bank closed before Start", Proof::BankClosed),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    let prep_wait = prep_barrier.clone();
    steps.push(Step {
        name: "wait for spinner successful relog and fixture readiness before Start",
        kind: StepKind::Await {
            evidence: "flax_spinner_relogged_and_prepared_before_start",
            ready: Box::new(move |_| prep_wait.all_ready()),
            summary: None,
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 600,
        },
    });
    steps.push(start_catalog_step());
    Scenario {
        name: "flax_runner",
        seed: pair_fleet_seed(),
        steps,
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![pair_prep_companion(
            PairCompanionKind::FlaxSpinner,
            prep_barrier,
        )],
        settings: pair_watch_settings_with_deadline(
            "flax_runner",
            "FlaxRunner",
            FLAX_RUNNER_DEADLINE,
        ),
    }
}

/// DuelArena both actors: bronze scimitar at the challenge anchor, wear it,
/// then shared Start. Counterpart identity is native witness-owned; the
/// trainer defaults include the helper mode's inert empty partner field.
pub(crate) fn duel_arena_scenario() -> Scenario {
    let dest = DUEL_CHALLENGE;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Duel bronze scimitar at the challenge anchor before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "give bronze_scimitar 1");
                cheat(c, &tele_args(dest.level, dest.x, dest.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: dest.x,
                z: dest.z,
                level: dest.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "confirm bronze scimitar in pack before Start",
        Proof::Item {
            name: "Bronze scimitar",
            count: 1,
        },
    ));
    steps.push(Step {
        name: "wear the seeded bronze scimitar before Start",
        kind: StepKind::Repeat {
            send: Box::new(|c, snap| {
                pair_wear_first_inv(c, snap);
                true
            }),
        },
        wait: Wait {
            arm: Proof::ItemAtMost {
                name: "Bronze scimitar",
                count: 0,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    Scenario {
        name: "duel_arena",
        seed: pair_fleet_seed(),
        steps,
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![pair_companion(PairCompanionKind::DuelPeer)],
        settings: pair_watch_settings("duel_arena", "Duel Arena Combat Trainer"),
    }
}

/// The six Duel Arena fight pens as `(min_x, max_x, min_z, max_z)`, level 0:
/// the frozen `api/duel/Duel.ts` `DUEL_FIGHT_ARENAS`.
const DUEL_FIGHT_PENS: [(i32, i32, i32, i32); 6] = [
    (3333, 3357, 3244, 3258),
    (3364, 3388, 3225, 3239),
    (3333, 3357, 3206, 3220),
    (3364, 3388, 3244, 3258),
    (3333, 3357, 3225, 3239),
    (3364, 3388, 3206, 3220),
];

/// The frozen `DuelArenaLogic.ts` `DUEL_ZONE`: the arena and its lobby.
const DUEL_ZONE: (i32, i32, i32, i32) = (3328, 3393, 3203, 3325);

fn within(rect: (i32, i32, i32, i32), x: i32, z: i32) -> bool {
    (rect.0..=rect.1).contains(&x) && (rect.2..=rect.3).contains(&z)
}

/// One slot's completed-duel witness, from its own snapshots alone: it stood
/// in a fight pen and was later seen back in the arena's lobby. The server
/// sends both fighters to the lobby when a duel ends, so this is the same
/// transition the card counts as a finished duel, and it needs a real
/// counterpart: a lone slot never enters a pen.
#[derive(Default)]
struct DuelRoundTrip {
    fought: bool,
}

impl DuelRoundTrip {
    fn observe(&mut self, snap: &GameSnapshot) -> bool {
        if !snap.ingame() || snap.scene_state() != 2 {
            return false;
        }
        let Some((x, z, level)) = snap.tile() else {
            return false;
        };
        if level != 0 {
            return false;
        }
        if DUEL_FIGHT_PENS.iter().any(|pen| within(*pen, x, z)) {
            self.fought = true;
            return false;
        }
        self.fought && within(DUEL_ZONE, x, z)
    }
}

/// Fleet qualification for the frozen DuelArena card: after Start, this slot
/// must finish one duel against another fleet member. The slots pair among
/// themselves; nothing here spawns or changes the world. The witness holds
/// per-slot state, so build one step per slot.
pub fn duel_arena_completed_duel_step(budget_ticks: u32) -> Step {
    let witness = Mutex::new(DuelRoundTrip::default());
    Step {
        name: "watch this slot finish a duel with another fleet member",
        kind: StepKind::Await {
            evidence: "duel_arena_pen_entered_and_lobby_returned",
            ready: Box::new(move |snap| witness.lock().expect("duel witness mutex").observe(snap)),
            summary: None,
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks,
        },
    }
}

/// ClueSolver carries a `loadout` setting and Start refuses when it
/// cannot select a row. The cell seeds fifteen Sharks in the solver's
/// pack, so the posted fixture row names that same food; both fleet
/// slots receive the row at Start.
const CLUE_DUEL_FIXTURE_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Scenario Clue food",
    carry: &[("Shark", 15)],
}];

/// Two-account clue 3554 gold: the solver and dedicated Duel helper start
/// together, complete the no-stakes handshake, obtain/open the casket, and
/// return through a fresh bank before closing it.
pub(crate) fn clue_duel_3554_scenario() -> Scenario {
    let witness = Arc::new(Mutex::new(ClueFleetWitness::default()));
    let prep_barrier = PairPrepBarrier::new(1);
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed hard clue 3554 kit at the Duel Arena",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "~clearinv");
                for skill in ["attack", "strength", "defence", "hitpoints", "prayer"] {
                    cheat(c, &format!("setstat {skill} 99"));
                }
                cheat(c, "setvar zanaris 6");
                cheat(c, "setvar trail_status 133");
                for item in [
                    "trail_clue_hard_sextant028 1",
                    "trail_sextant 1",
                    "trail_watch 1",
                    "trail_chart 1",
                    "spade 1",
                    "dragon_dagger_p 1",
                    "4dose2antipoison 1",
                    "shark 15",
                    "coins 1000",
                ] {
                    cheat(c, &format!("give {item}"));
                }
                cheat(
                    c,
                    &tele_args(DUEL_CHALLENGE.level, DUEL_CHALLENGE.x, DUEL_CHALLENGE.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ItemId { id: 3554, count: 1 },
            budget_ticks: 240,
        },
    });
    let prep_wait = prep_barrier.clone();
    steps.push(Step {
        name: "wait for the clue helper fixture before Start",
        kind: StepKind::Await {
            evidence: "clue_helper_prepared_before_start",
            ready: Box::new(move |_| prep_wait.all_ready()),
            summary: None,
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 600,
        },
    });
    steps.push(start_catalog_step());
    let start_witness = Arc::clone(&witness);
    steps.push(Step {
        name: "begin two-slot clue-duel witness after Start",
        kind: StepKind::Await {
            evidence: "clue_duel_two_distinct_visible_accounts",
            ready: Box::new(move |snap| {
                let mut witness = start_witness.lock().expect("clue witness mutex");
                witness.start();
                witness.observe(0, snap);
                witness.identities_ready()
            }),
            summary: None,
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 300,
        },
    });
    let handshake_witness = Arc::clone(&witness);
    steps.push(Step {
        name: "observe both no-stake exact-rule duel handshakes and pen entry",
        kind: StepKind::Await {
            evidence: "clue_duel_both_offer_confirm_rules_1024_pen",
            ready: Box::new(move |snap| {
                let mut witness = handshake_witness.lock().expect("clue witness mutex");
                witness.observe(0, snap);
                witness.handshake_complete()
            }),
            summary: None,
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 900,
        },
    });
    let trail_witness = Arc::clone(&witness);
    steps.push(Step {
        name: "observe dig, casket, forfeit, helper reset, and reward",
        kind: StepKind::Await {
            evidence: "clue_3554_casket_forfeit_helper_reset_reward",
            ready: Box::new(move |snap| {
                let mut witness = trail_witness.lock().expect("clue witness mutex");
                witness.observe(0, snap);
                witness.trail_complete()
            }),
            summary: None,
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 900,
        },
    });
    let bank_witness = Arc::clone(&witness);
    steps.push(Step {
        name: "observe new reward in a fresh bank with both HP totals unchanged",
        kind: StepKind::Await {
            evidence: "clue_reward_bank_delta_and_both_hp_unchanged",
            ready: Box::new(move |snap| {
                let mut witness = bank_witness.lock().expect("clue witness mutex");
                witness.observe(0, snap);
                witness.complete()
            }),
            summary: None,
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 900,
        },
    });
    steps.push(Step {
        name: "watch ClueSolver close the reward bank",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::BankClosed,
            budget_ticks: 120,
        },
    });
    Scenario {
        name: "clue_duel_3554",
        seed: pair_fleet_seed(),
        steps,
        proof: Proof::BankClosed,
        companions: vec![clue_witness_companion(witness, prep_barrier)],
        settings: ScenarioSettings {
            full_rate: true,
            only_render_selected: false,
            require_mainland_base: true,
            deadline: Duration::from_secs(360),
            start_script: Some("ClueSolver"),
            fleet_start: Some(FleetStart::ClueDuel),
            script_settings_inject: Some(&[
                ScriptSettingInject {
                    id: "useTeleports",
                    value: ScriptInjectValue::Bool(false),
                },
                ScriptSettingInject {
                    id: "restorePrayer",
                    value: ScriptInjectValue::Bool(false),
                },
            ]),
            fixture_loadouts: Some(CLUE_DUEL_FIXTURE_LOADOUTS),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// The KQ witness steps' read-only terminal summary closure: one
/// shared witness, read at terminal time only, on a pass and on a
/// failure alike.
fn kq_terminal_summary(
    witness: &Arc<Mutex<KqFleetWitness>>,
) -> Option<Box<dyn Fn() -> String + Send + Sync>> {
    let witness = Arc::clone(witness);
    Some(Box::new(move || {
        witness.lock().expect("KQ witness mutex").terminal_summary()
    }))
}

/// Four-account max-stat JiveKQ gold: all members share one minted roster,
/// provision from Shantay, descend together, kill both Queen phases, and
/// return through the next fresh bank. A listed drop banked on the way is
/// recorded in the receipt, never required (see [`KQ_LOOT_IDS`]). JiveKQ
/// is not Start-enabled in 0.1.9 (its catalog card is dim); this witness
/// Starts it past the dim for the Beta 1 qualification (see
/// [`KQ_LEADER_ROPES`]).
pub(crate) fn jive_kq_four_scenario() -> Scenario {
    let witness = Arc::new(Mutex::new(KqFleetWitness::default()));
    let prep_barrier = PairPrepBarrier::new(3);
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed four-account KQ banks and stage at Shantay",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                seed_kq_profile(c, true, KQ_LEADER_ROPES);
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: KQ_BANK.x,
                z: KQ_BANK.z,
                level: KQ_BANK.level,
                radius: 8,
            },
            budget_ticks: 240,
        },
    });
    let prep_wait = prep_barrier.clone();
    steps.push(Step {
        name: "wait for all three KQ companion fixtures before Start",
        kind: StepKind::Await {
            evidence: "kq_companions_prepared_before_start",
            ready: Box::new(move |_| prep_wait.all_ready()),
            summary: None,
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 600,
        },
    });
    let arm_witness = Arc::clone(&witness);
    steps.push(Step {
        name: "arm KQ witness before the delayed leader Start",
        kind: StepKind::Perform {
            send: Box::new(move |_, _| {
                arm_witness.lock().expect("KQ witness mutex").start();
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 20,
        },
    });
    steps.push(start_catalog_step());
    let start_witness = Arc::clone(&witness);
    steps.push(Step {
        name: "begin four-slot KQ witness after Start",
        kind: StepKind::Await {
            evidence: "kq_four_distinct_account_sessions",
            ready: Box::new(move |snap| {
                let mut witness = start_witness.lock().expect("KQ witness mutex");
                witness.start();
                witness.observe(0, snap);
                witness.identities_ready()
            }),
            summary: kq_terminal_summary(&witness),
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 400,
        },
    });
    let descent_witness = Arc::clone(&witness);
    steps.push(Step {
        name: "observe pass sources, exact packs, peer holds, consumed ropes, and first descent",
        kind: StepKind::Await {
            evidence: "kq_pass_paths_packs_peer_hold_two_ropes_first_descent",
            ready: Box::new(move |snap| {
                let mut witness = descent_witness.lock().expect("KQ witness mutex");
                witness.observe(0, snap);
                witness.first_descent()
            }),
            summary: kq_terminal_summary(&witness),
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 2400,
        },
    });
    let combat_witness = Arc::clone(&witness);
    steps.push(Step {
        name: "observe both forms and each member's protected cardinal combat contribution",
        kind: StepKind::Await {
            evidence: "kq_both_forms_four_xp_food_resume_cardinal_formation",
            ready: Box::new(move |snap| {
                let mut witness = combat_witness.lock().expect("KQ witness mutex");
                witness.observe(0, snap);
                witness.first_combat()
            }),
            summary: kq_terminal_summary(&witness),
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 2400,
        },
    });
    let kill_bank_witness = Arc::clone(&witness);
    steps.push(Step {
        name: "observe alive-zero-absent kill and first fresh bank return",
        kind: StepKind::Await {
            evidence: "kq_kill_first_fresh_bank_return",
            ready: Box::new(move |snap| {
                let mut witness = kill_bank_witness.lock().expect("KQ witness mutex");
                witness.observe(0, snap);
                witness.first_kill_bank()
            }),
            summary: kq_terminal_summary(&witness),
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 3600,
        },
    });
    let reentry_witness = Arc::clone(&witness);
    steps.push(Step {
        name: "observe all four restock, descend again, and attack the respawn",
        kind: StepKind::Await {
            evidence: "kq_four_fresh_banks_second_descent_respawn_xp",
            ready: Box::new(move |snap| {
                let mut witness = reentry_witness.lock().expect("KQ witness mutex");
                witness.observe(0, snap);
                witness.second_descent()
            }),
            summary: kq_terminal_summary(&witness),
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 3600,
        },
    });
    let safe_witness = Arc::clone(&witness);
    steps.push(Step {
        name: "observe escapes, overlapping restock, second kill, and safe bank teardown",
        kind: StepKind::Await {
            evidence: "kq_escape_restock_second_kill_four_survivors_safely_banked",
            ready: Box::new(move |snap| {
                let mut witness = safe_witness.lock().expect("KQ witness mutex");
                witness.observe(0, snap);
                witness.complete()
            }),
            summary: kq_terminal_summary(&witness),
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 3600,
        },
    });
    // Coordinated safe finish (JIVE-KQ-1024). The step above ends only
    // when every KQ witness holds for the whole fleet, so party
    // coordination stays intact until no teammate still needs it.
    // Only then does the scenario stop the fleet: `StopScript` runs
    // the product Stop path for every stashed fleet slot
    // (`host-play/src/live_start.rs`, the `on_stop_script` branch stops
    // each pending card), and its `ScriptIdle` arm holds only on the
    // host's observed idle driven slot. The 10-tick budget is the
    // existing Stop guard (`quester.rs`, "stop Quester mid-quest").
    // With the scripts stopped, the finish closes every bank the
    // bots still have open and then proves the current closed state.
    // Each companion closes its own bank in `pair_companion_frame`
    // throughout the finish, and the Await below requires every
    // companion's latest post-finish observation to show it closed
    // (`KqFleetWitness::finish_ready`): an open observation revokes
    // that slot until a later closed one, so a bank interaction the
    // stopped script had already sent may land and be closed again
    // without counting stale. The driven bot's close sends run last,
    // after that Await, so its in-flight interaction has the whole
    // companion finish to land before the repeated sends start; its
    // `BankClosed` arm and the scenario proof then judge the driven
    // bank's real current state. One close send is acted on by the
    // next game tick and the companion send pacing is 400 ms
    // (`send_ok`), under one 600 ms tick; the 120-tick budgets are
    // the guards this file already uses for a `BankClosed` watch
    // (`clue_duel_3554_scenario`), not expected durations. The
    // sticky `safely_banked` latch never proves the finish.
    steps.push(Step {
        name: "stop the KQ fleet after every witness is met",
        kind: StepKind::StopScript,
        wait: Wait {
            arm: Proof::ScriptIdle,
            budget_ticks: 10,
        },
    });
    let finish_witness = Arc::clone(&witness);
    steps.push(Step {
        name: "begin the KQ safe finish after the fleet stop",
        kind: StepKind::Perform {
            send: Box::new(move |_, _| {
                finish_witness
                    .lock()
                    .expect("KQ witness mutex")
                    .begin_finish();
                true
            }),
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 20,
        },
    });
    let finish_ready_witness = Arc::clone(&witness);
    steps.push(Step {
        name: "observe every companion bank closed after the fleet stop",
        kind: StepKind::Await {
            evidence: "kq_safe_finish_fleet_stopped_companion_banks_closed",
            ready: Box::new(move |snap| {
                let mut witness = finish_ready_witness.lock().expect("KQ witness mutex");
                witness.observe(0, snap);
                witness.finish_ready()
            }),
            summary: kq_terminal_summary(&witness),
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 120,
        },
    });
    steps.push(Step {
        name: "close the driven bot's bank after the fleet stop",
        kind: StepKind::Repeat {
            send: Box::new(|c, snapshot| {
                matches!(
                    Interactions::new(snapshot, c).close_modal(),
                    SendResult::Sent { .. } | SendResult::Refused { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::BankClosed,
            budget_ticks: 120,
        },
    });
    Scenario {
        name: "jive_kq_four",
        seed: Seed {
            profiles: vec![
                ("test", "test"),
                ("test2", "test2"),
                ("test3", "test3"),
                ("test4", "test4"),
            ],
            mainland: true,
        },
        steps,
        proof: Proof::BankClosed,
        companions: (1..4)
            .map(|profile| {
                kq_witness_companion(profile, Arc::clone(&witness), prep_barrier.clone())
            })
            .collect(),
        settings: ScenarioSettings {
            full_rate: true,
            only_render_selected: false,
            require_mainland_base: true,
            start_script: Some("JiveKQ"),
            fleet_start: Some(FleetStart::JiveKq),
            deadline: KQ_WATCHDOG,
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod prep_barrier_tests {
    use super::{flax_runner_scenario, pair_relog_ready, post_seed_scene_ready, PairPrepBarrier};
    use crate::StepKind;

    #[test]
    fn fleet_start_waits_for_every_companion_fixture() {
        let barrier = PairPrepBarrier::new(3);
        assert!(!barrier.all_ready());
        barrier.mark_ready(1);
        barrier.mark_ready(3);
        assert!(!barrier.all_ready());
        barrier.mark_ready(2);
        assert!(barrier.all_ready());
    }

    #[test]
    fn flax_spinner_is_not_ready_until_relogin_scene_and_inventory_tab_are_fresh() {
        let barrier = PairPrepBarrier::new(1);
        for (saw_logout, ingame, scene_state, inv_tab) in [
            (false, true, 2, true),
            (true, false, 2, true),
            (true, true, 1, true),
            (true, true, 2, false),
        ] {
            if pair_relog_ready(saw_logout, ingame, scene_state, inv_tab) {
                barrier.mark_ready(1);
            }
            assert!(!barrier.all_ready());
        }
        if pair_relog_ready(true, true, 2, true) {
            barrier.mark_ready(1);
        }
        assert!(barrier.all_ready());
    }

    #[test]
    fn flax_runner_waits_for_spinner_fixture_before_shared_start() {
        let scenario = flax_runner_scenario();
        let ready = scenario
            .steps
            .iter()
            .position(|step| {
                matches!(
                    &step.kind,
                    StepKind::Await { evidence, .. }
                        if *evidence == "flax_spinner_relogged_and_prepared_before_start"
                )
            })
            .expect("spinner fixture readiness step");
        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(&step.kind, StepKind::StartScript))
            .expect("shared Start step");
        assert!(ready < start, "fixture readiness must precede shared Start");
    }

    #[test]
    fn seed_fixture_waits_for_ready_scene_after_teleport_generation() {
        assert!(!post_seed_scene_ready(None, 7, 2));
        assert!(!post_seed_scene_ready(Some(7), 7, 2));
        assert!(!post_seed_scene_ready(Some(7), 8, 1));
        assert!(post_seed_scene_ready(Some(7), 8, 2));
    }
}

#[cfg(test)]
mod clue_witness_tests {
    use super::ClueFleetWitness;
    use api::snapshot::GameSnapshot;
    use client::client::{Client, ClientPlayer};
    use client::io::ServerProt;

    /// An in-game snapshot of `local` with `others` in view.
    fn seen(local: &str, others: &[&str]) -> GameSnapshot {
        let mut client = Client::new(client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        });
        client.ingame = true;
        client.scene_state = 2;
        client.local_player = Some(ClientPlayer {
            name: Some(local.into()),
            ..ClientPlayer::at(20, 20)
        });
        client.player_count = others.len() as i32;
        for (index, name) in others.iter().enumerate() {
            client.player_ids[index] = index as i32;
            client.players[index] = Some(Box::new(ClientPlayer {
                name: Some((*name).into()),
                ..ClientPlayer::at(22, 20)
            }));
        }
        client.bump_gens(ServerProt::PLAYER_INFO);
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        snapshot
    }

    /// The clue witness's peer is the other slot's own account: another
    /// named player in view while the counterpart is away is not it.
    #[test]
    fn saw_peer_needs_the_named_counterpart() {
        let mut witness = ClueFleetWitness::default();
        witness.start();
        witness.observe(0, &seen("Solver", &[]));
        witness.observe(1, &seen("Helper_One", &["Stranger"]));
        assert!(!witness.slots[1].saw_peer, "a stranger is not the solver");
        witness.observe(0, &seen("Solver", &["Stranger"]));
        assert!(!witness.slots[0].saw_peer, "a stranger is not the helper");
        witness.observe(0, &seen("Solver", &["Stranger", "helper one"]));
        assert!(witness.slots[0].saw_peer, "the helper by its own name");
    }

    #[test]
    fn clue_pen_witness_covers_each_engine_obstacle_arena_boundary() {
        // 289 game_duelarena/configs/duelarena.dbrow:7-11.
        // The server chooses the first obstacle pen with fewer than 24 players.
        // It then chooses any walkable square inside that pen's bounds.
        for (min_x, max_x, min_z, max_z) in [
            (3364, 3388, 3244, 3258),
            (3333, 3357, 3225, 3239),
            (3364, 3388, 3206, 3220),
        ] {
            for (x, z) in [
                (min_x, min_z),
                (min_x, max_z),
                (max_x, min_z),
                (max_x, max_z),
            ] {
                let mut snapshot = GameSnapshot::new();
                snapshot.seed_ingame(2);
                snapshot.seed_tile(api::WorldTile { x, z, level: 0 });
                let mut witness = ClueFleetWitness::default();
                witness.start();
                witness.observe(1, &snapshot);
                assert!(witness.slots[1].pen, "legal obstacle pen tile ({x},{z})");
                assert!(!witness.slots[1].returned, "pen entry is not a return");
                snapshot.seed_tile(api::WorldTile {
                    x: 3368,
                    z: 3274,
                    level: 0,
                });
                witness.observe(1, &snapshot);
                assert!(
                    witness.slots[1].returned,
                    "the helper returned to the lobby"
                );
            }
        }
    }

    #[test]
    fn clue_pen_witness_rejects_non_obstacle_tiles_other_levels_and_loading() {
        for (x, z, level, scene) in [
            (3362, 3250, 0, 2), // close to the old centre, outside every pen
            (3340, 3250, 0, 2), // an unobstructed arena is not this duel's pen
            (3368, 3274, 0, 2), // lobby
            (3374, 3250, 1, 2), // correct coordinates on another level
            (3374, 3250, 0, 1), // unloaded scene
        ] {
            let mut snapshot = GameSnapshot::new();
            snapshot.seed_ingame(scene);
            snapshot.seed_tile(api::WorldTile { x, z, level });
            let mut witness = ClueFleetWitness::default();
            witness.start();
            witness.observe(1, &snapshot);
            assert!(
                !witness.slots[1].pen && !witness.slots[1].returned,
                "not an observed obstacle pen: ({x},{z},{level}), scene {scene}"
            );
        }
    }
}

#[cfg(test)]
mod duel_round_trip_tests {
    use super::DuelRoundTrip;
    use api::snapshot::GameSnapshot;
    use client::client::{Client, ClientPlayer};
    use client::io::ServerProt;

    const BASE_X: i32 = 3296;
    const BASE_Z: i32 = 3200;

    /// The local player standing on world tile `(x, z)`; `scene_state` 2 is a
    /// loaded scene.
    fn at(x: i32, z: i32, level: i32, scene_state: i32) -> GameSnapshot {
        let mut client = Client::new(client::client::ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        });
        client.ingame = true;
        client.scene_state = scene_state;
        client.map_build_base_x = BASE_X;
        client.map_build_base_z = BASE_Z;
        client.minusedlevel = level;
        client.local_player = Some(ClientPlayer::at(x - BASE_X, z - BASE_Z));
        client.bump_gens(ServerProt::PLAYER_INFO);
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(&client);
        snapshot
    }

    /// A duel is the pen visit and the return, not either alone: a slot
    /// waiting in the lobby, or one still fighting, has not finished one.
    #[test]
    fn a_duel_needs_a_pen_visit_and_a_return_to_the_lobby() {
        let mut witness = DuelRoundTrip::default();
        assert!(!witness.observe(&at(3368, 3274, 0, 2)), "lobby only");

        assert!(!witness.observe(&at(3340, 3250, 0, 2)), "fighting in a pen");
        assert!(!witness.observe(&at(3340, 3250, 0, 2)), "still fighting");
        assert!(
            !witness.observe(&at(3368, 3274, 0, 1)),
            "a scene that is still loading is not an observation"
        );
        assert!(witness.observe(&at(3368, 3274, 0, 2)), "back in the lobby");
    }

    /// Leaving the pens somewhere that is not the arena lobby (a logout to the
    /// spawn, another level) is not a finished duel, and a pen tile on another
    /// level is not a pen.
    #[test]
    fn only_the_arena_lobby_after_a_pen_completes_a_duel() {
        let mut witness = DuelRoundTrip::default();
        assert!(
            !witness.observe(&at(3340, 3250, 1, 2)),
            "upstairs is no pen"
        );
        assert!(!witness.observe(&at(3368, 3274, 0, 2)), "so no duel yet");

        assert!(!witness.observe(&at(3340, 3250, 0, 2)));
        assert!(
            !witness.observe(&at(3222, 3218, 0, 2)),
            "Lumbridge is not the lobby"
        );
        assert!(witness.observe(&at(3372, 3270, 0, 2)));
    }
}

#[cfg(test)]
mod kq_finish_tests {
    use super::{jive_kq_four_scenario, KqFleetWitness, KQ_BANK};
    use crate::{Proof, StepKind};
    use api::snapshot::{ChatLineView, GameSnapshot, StatView};

    /// One fleet member at the Shantay bank in a loaded scene, with its
    /// bank open (component 5382, loaded) or closed. The closed form
    /// also satisfies the scenario's real `BankClosed` proof.
    fn at_bank(bank_open: bool) -> GameSnapshot {
        let mut snap = GameSnapshot::new();
        snap.seed_ingame(2);
        snap.seed_tile(api::WorldTile {
            x: KQ_BANK.x,
            z: KQ_BANK.z,
            level: KQ_BANK.level,
        });
        if bank_open {
            snap.seed_bank_observation(5382, 1, Some(Vec::new()), Vec::new());
        } else {
            snap.seed_bank_observation(-1, 1, None, Vec::new());
        }
        snap
    }

    fn started_witness() -> KqFleetWitness {
        let mut witness = KqFleetWitness::default();
        witness.start();
        witness
    }

    /// One bank snapshot carrying a stat table with the given
    /// hitpoints row (`Some((effective, base))`), or no hitpoints row
    /// at all (`None`). An attack row is present in both forms, as a
    /// received table posts every used skill.
    fn with_hitpoints(hp: Option<(i32, i32)>) -> GameSnapshot {
        fn row(index: i32, name: &str, effective: i32, base: i32) -> StatView {
            StatView {
                index,
                name: name.to_string(),
                effective,
                base,
                xp: 14_391_160,
                used: true,
            }
        }
        let mut snap = at_bank(false);
        let mut rows = vec![row(0, "Attack", 99, 99)];
        if let Some((effective, base)) = hp {
            rows.push(row(3, "Hitpoints", effective, base));
        }
        snap.seed_stats(rows);
        snap
    }

    /// A started witness whose slot `index` has its identity and lives,
    /// the state a member is in once its name is observed after Start.
    fn living_witness(index: usize) -> KqFleetWitness {
        let mut witness = started_witness();
        witness.slots[index].name = Some(format!("member{index}"));
        witness.slots[index].survived = true;
        witness
    }

    /// Pass direction: after the finish begins, every slot is
    /// observed and each companion's bank is seen open (the state
    /// the stopped script left it in) and then seen closed. The
    /// companion finish is ready only after the last companion's
    /// closed observation. The driven bot's bank is judged by the
    /// close step and the scenario proof, whose real state is the
    /// closed fixture here.
    #[test]
    fn safe_finish_passes_when_every_bank_closes_after_the_finish() {
        assert!(
            Proof::BankClosed.check(&at_bank(false), None),
            "the closed fixture is the real BankClosed state"
        );
        assert!(
            !Proof::BankClosed.check(&at_bank(true), None),
            "the open fixture is not BankClosed"
        );
        let mut witness = started_witness();
        witness.begin_finish();
        for slot in 1..4 {
            witness.observe(slot, &at_bank(false));
        }
        assert!(
            !witness.finish_ready(),
            "the driven slot has not been observed since the finish began"
        );
        witness.observe(0, &at_bank(false));
        assert!(
            witness.finish_ready(),
            "every companion bank was observed closed after the finish began"
        );
        for slot in 1..4 {
            witness.observe(slot, &at_bank(true));
            assert!(
                !witness.finish_ready(),
                "slot {slot} still has its bank open"
            );
        }
        for slot in 1..3 {
            witness.observe(slot, &at_bank(false));
            assert!(
                !witness.finish_ready(),
                "slot {} has not been seen closed yet",
                slot + 1
            );
        }
        witness.observe(3, &at_bank(false));
        assert!(witness.finish_ready());
    }

    /// Fail direction, reopen: a companion that opens its bank again
    /// after it was seen closed during the finish revokes its
    /// credit, and the finish does not prove while that bank is
    /// open. Only a fresh closed observation after the reopen (the
    /// state after the finish's own close sends close an in-flight
    /// interaction the stopped script had already sent) restores
    /// it: the first, stale closed observation never counts again
    /// on its own.
    #[test]
    fn safe_finish_fails_when_a_bot_reopens_its_bank_after_the_finish() {
        let mut witness = started_witness();
        witness.begin_finish();
        for slot in 0..4 {
            witness.observe(slot, &at_bank(false));
        }
        assert!(witness.finish_ready());
        witness.observe(2, &at_bank(true));
        assert!(
            !witness.finish_ready(),
            "slot 2 reopened its bank after the finish"
        );
        witness.observe(2, &at_bank(true));
        assert!(
            !witness.finish_ready(),
            "the stale closed observation does not count while the bank is open"
        );
        witness.observe(2, &at_bank(false));
        assert!(
            witness.finish_ready(),
            "only the fresh closed observation after the reopen counts"
        );
    }

    /// Fail direction, too early: the finish does not prove while a
    /// bank is still open, from a scene that is still loading, or
    /// from bank states observed before the finish began. The last
    /// case is the JIVE-KQ-1024 race: a bank that was closed earlier
    /// (the sticky `safely_banked` latch) says nothing about the
    /// bank's state at the finish.
    #[test]
    fn safe_finish_does_not_prove_before_the_banks_close_after_the_finish() {
        let mut witness = started_witness();
        for slot in 0..4 {
            witness.observe(slot, &at_bank(false));
        }
        assert!(
            !witness.finish_ready(),
            "closed banks before the finish do not prove the finish"
        );
        witness.begin_finish();
        assert!(
            !witness.finish_ready(),
            "no slot has been observed since the finish began"
        );
        for slot in 0..4 {
            witness.observe(slot, &at_bank(true));
        }
        assert!(
            !witness.finish_ready(),
            "every bank is still open after the finish began"
        );
        let mut loading = at_bank(false);
        loading.seed_ingame(1);
        for slot in 0..4 {
            witness.observe(slot, &loading);
        }
        assert!(
            !witness.finish_ready(),
            "a loading scene is not a closed-bank observation"
        );
        for slot in 0..4 {
            witness.observe(slot, &at_bank(false));
        }
        assert!(witness.finish_ready());
    }

    /// The scenario's finish is ordered: the fleet witness completes
    /// first, then the fleet stops, then the finish begins, then
    /// the companion banks prove closed, then the driven bank
    /// closes last. The terminal proof stays the real `BankClosed`.
    #[test]
    fn jive_kq_stops_the_fleet_then_proves_the_safe_finish() {
        let scenario = jive_kq_four_scenario();
        let complete = scenario
            .steps
            .iter()
            .position(|step| {
                matches!(
                    &step.kind,
                    StepKind::Await { evidence, .. }
                        if *evidence
                            == "kq_escape_restock_second_kill_four_survivors_safely_banked"
                )
            })
            .expect("fleet completion step");
        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .expect("fleet Start step");
        let stop = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StopScript))
            .expect("fleet Stop step");
        assert!(start < complete && complete < stop, "Start, witness, Stop");
        assert_eq!(
            scenario
                .steps
                .iter()
                .filter(|step| matches!(step.kind, StepKind::StopScript))
                .count(),
            1,
            "exactly one coordinated stop, never a per-bot early stop"
        );
        assert_eq!(scenario.steps[stop].wait.arm, Proof::ScriptIdle);
        assert!(
            matches!(scenario.steps[stop + 1].kind, StepKind::Perform { .. }),
            "the finish begins right after the stop"
        );
        for step in [&scenario.steps[complete], &scenario.steps[stop + 2]] {
            assert!(matches!(
                &step.kind,
                StepKind::Await {
                    summary: Some(_),
                    ..
                }
            ));
        }
        let finish = &scenario.steps[stop + 2];
        assert!(matches!(
            &finish.kind,
            StepKind::Await { evidence, .. }
                if *evidence == "kq_safe_finish_fleet_stopped_companion_banks_closed"
        ));
        let close = &scenario.steps[stop + 3];
        assert!(
            matches!(close.kind, StepKind::Repeat { .. }),
            "the driven bank close repeats until it lands"
        );
        assert_eq!(close.wait.arm, Proof::BankClosed);
        assert_eq!(
            scenario.steps.len(),
            stop + 4,
            "the driven close is the last step before the proof"
        );
        assert_eq!(scenario.proof, Proof::BankClosed);
        assert_eq!(
            scenario.settings.deadline,
            std::time::Duration::from_secs(2_094),
            "watchdog is the slowest round-2 completed pass plus 20 % (KQ_WATCHDOG)"
        );
    }

    /// The terminal summary names the unmet fleet phases and per-slot
    /// terms, each account's last tile, tick, and bank state, and the
    /// latches that account still lacks. Terms drop out as they are
    /// met: a met term must not stay in the summary, and an unmet one
    /// must not vanish from it.
    #[test]
    fn terminal_summary_names_unmet_witnesses_and_last_slot_states() {
        let mut witness = started_witness();
        let summary = witness.terminal_summary();
        for unmet in [
            "identities",
            "first_descent",
            "first_combat",
            "first_kill_bank",
            "second_descent",
            "any_kills>=2",
            "s0:survived",
            "s3:safely_banked",
        ] {
            assert!(summary.contains(unmet), "unmet {unmet}: {summary}");
        }
        let unmet_list = summary
            .split("unmet=[")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("unmet list in summary");
        assert!(
            !unmet_list.contains("restock_overlap"),
            "restock overlap is an observation, never an unmet term: {summary}"
        );
        assert!(
            !unmet_list.contains("loot"),
            "banked loot is an observation, never an unmet term: {summary}"
        );
        assert!(
            summary.contains("restock_overlap=false"),
            "the summary records the observed value: {summary}"
        );
        assert!(
            summary.contains("loot_banked=false"),
            "the summary records the observed value: {summary}"
        );
        assert!(
            summary.contains("survival_cleared=none"),
            "no slot has an observed death: {summary}"
        );
        assert!(summary.contains("s1<?> tile=? tick=0 bank=?"), "{summary}");
        assert!(summary.contains("safely_banked"), "{summary}");
        assert!(summary.contains("disp=0"), "{summary}");

        let mut snap = at_bank(false);
        snap.seed_tick(2_893);
        witness.observe(1, &snap);
        let summary = witness.terminal_summary();
        let slot1 = summary.split("s1<").nth(1).expect("slot 1 in summary");
        assert!(
            slot1.starts_with("?> tile=3308,3120,0 tick=2893 bank=-1"),
            "{summary}"
        );

        witness.slots[1].safely_banked = true;
        witness.slots[1].disappearances = 2;
        witness.slots[1].kills = 2;
        let summary = witness.terminal_summary();
        let slot1 = summary
            .split("s1<")
            .nth(1)
            .and_then(|rest| rest.split(" s2<").next())
            .expect("slot 1 segment");
        assert!(!slot1.contains("safely_banked"), "met latch: {slot1}");
        assert!(slot1.contains("disp=2"), "{slot1}");
        assert!(slot1.contains("kills=2"), "{slot1}");
        assert!(
            !summary.contains("any_kills>=2"),
            "fleet kills term met: {summary}"
        );
        let slot2 = summary.split("s2<").nth(1).expect("slot 2 in summary");
        assert!(
            slot2.contains("safely_banked"),
            "other slots still report their unmet terms: {slot2}"
        );
    }

    /// A fleet witness set to every gated term of
    /// [`KqFleetWitness::complete`]: two descents, combat in both
    /// forms, both escapes, two disappearances, a second witnessed
    /// kill, and the second safe bank. The listed-loot fields are set
    /// too, so the receipt observation reads true.
    fn complete_witness() -> KqFleetWitness {
        let mut witness = started_witness();
        witness.leader_had_two_ropes = true;
        witness.surface_rope_valid = true;
        witness.upper_rope_valid = true;
        for (index, slot) in witness.slots.iter_mut().enumerate() {
            slot.name = Some(format!("member{index}"));
            slot.survived = true;
            slot.pack_ready = true;
            slot.kit_equipped = true;
            slot.pass_ready = true;
            slot.pass_source_complete = true;
            slot.bank_departed = true;
            slot.surface_rope = true;
            slot.upper_rope = true;
            slot.surface_party = true;
            slot.upper_party = true;
            slot.lair_entries = 2;
            slot.ground_alive = true;
            slot.flying_alive = true;
            slot.strength_gain = true;
            slot.ranged_gain = true;
            slot.melee_formation = true;
            slot.ranged_formation = true;
            slot.boosted = true;
            slot.food_eaten = true;
            slot.xp_after_food = true;
            slot.disappearances = 2;
            slot.bank_returns = 2;
            slot.kills = 2;
            slot.loot_ground = Some(565);
            slot.loot_collected = true;
            slot.loot_banked = true;
            slot.second_attack = true;
            slot.camelot_escape = true;
            slot.duel_escape = true;
            slot.ring_spent = true;
            slot.safely_banked = true;
        }
        witness
    }

    /// Restock overlap is a chance interleaving, not a gate: a
    /// witness with every gated term and no overlap completes and
    /// its summary records `restock_overlap=false`. The real gates
    /// still gate in both overlap states.
    #[test]
    fn completion_passes_without_restock_overlap_and_records_it() {
        let mut witness = complete_witness();
        assert!(!witness.restock_overlap);
        assert!(
            witness.complete(),
            "every gated term holds; the chance overlap must not gate"
        );
        let summary = witness.terminal_summary();
        assert!(
            summary.starts_with(
                "kq_witness unmet=[] restock_overlap=false loot_banked=true survival_cleared=none"
            ),
            "{summary}"
        );

        witness.restock_overlap = true;
        assert!(witness.complete());
        assert!(
            witness.terminal_summary().starts_with(
                "kq_witness unmet=[] restock_overlap=true loot_banked=true survival_cleared=none"
            ),
            "{}",
            witness.terminal_summary()
        );

        witness.slots[2].safely_banked = false;
        assert!(
            !witness.complete(),
            "the second safe bank still gates, overlap or not"
        );
        witness.slots[2].safely_banked = true;
        for slot in &mut witness.slots {
            slot.kills = 1;
        }
        assert!(
            !witness.complete(),
            "the second witnessed kill still gates, overlap or not"
        );
        for slot in &mut witness.slots {
            slot.kills = 2;
        }
        witness.slots[0].duel_escape = false;
        assert!(
            !witness.complete(),
            "each escape still gates, overlap or not"
        );
        witness.slots[0].duel_escape = true;
        witness.slots[3].disappearances = 1;
        assert!(
            !witness.complete(),
            "two disappearances still gate, overlap or not"
        );
        witness.slots[3].disappearances = 2;
        assert!(witness.complete());
    }

    /// Banked listed loot is a chance outcome, not a gate: a witness
    /// with every gated term and no listed loot at all completes and
    /// its summary records `loot_banked=false`. The kill and the
    /// first bank return still gate in both loot states.
    #[test]
    fn completion_passes_without_listed_loot_and_records_it() {
        let mut witness = complete_witness();
        for slot in &mut witness.slots {
            slot.loot_ground = None;
            slot.loot_collected = false;
            slot.loot_banked = false;
        }
        assert!(
            witness.complete(),
            "every gated term holds; the chance drop must not gate"
        );
        let summary = witness.terminal_summary();
        assert!(
            summary.starts_with("kq_witness unmet=[] restock_overlap=false loot_banked=false"),
            "{summary}"
        );
        assert_eq!(
            summary.matches("loot_banked").count(),
            1,
            "loot appears only as the fleet observation, never as a slot term: {summary}"
        );

        witness.slots[1].loot_ground = Some(565);
        witness.slots[1].loot_collected = true;
        witness.slots[1].loot_banked = true;
        assert!(witness.complete());
        assert!(
            witness
                .terminal_summary()
                .contains("loot_banked=true survival_cleared=none"),
            "{}",
            witness.terminal_summary()
        );

        for slot in &mut witness.slots {
            slot.kills = 0;
        }
        assert!(
            !witness.first_kill_bank() && !witness.complete(),
            "a witnessed kill still gates, banked loot or not"
        );
        for slot in &mut witness.slots {
            slot.kills = 2;
        }
        witness.slots[2].bank_returns = 0;
        assert!(
            !witness.first_kill_bank() && !witness.complete(),
            "every first fresh bank return still gates, banked loot or not"
        );
        witness.slots[2].bank_returns = 2;
        assert!(witness.complete());
    }

    /// Survival clears only on hitpoints 0 observed from stats the
    /// server has sent. A missing hitpoints row, the client's
    /// zero-filled row, and a snapshot with no stat table at all are
    /// missing observations and leave survival set, with no clear
    /// record. Positive received hitpoints leave it set. Received
    /// hitpoints 0 clears it and keeps the first clearing
    /// observation for the receipt.
    #[test]
    fn survival_clears_only_on_received_zero_hitpoints() {
        for snap in [
            at_bank(false),
            with_hitpoints(None),
            with_hitpoints(Some((0, 0))),
            with_hitpoints(Some((50, 99))),
        ] {
            let mut witness = living_witness(1);
            witness.observe(1, &snap);
            assert!(
                witness.slots[1].survived,
                "no received hitpoints 0 in this snapshot"
            );
            assert!(
                witness.slots[1].survival_cleared.is_none(),
                "a missing observation is not a death record"
            );
            assert!(
                witness.terminal_summary().contains("survival_cleared=none"),
                "{}",
                witness.terminal_summary()
            );
        }

        let mut dead = with_hitpoints(Some((0, 99)));
        dead.seed_tick(3_481);
        dead.seed_chat_lines(vec![ChatLineView {
            type_: 0,
            username: None,
            text: "Oh dear, you are dead!".to_string(),
            sequence: 7,
        }]);
        let mut witness = living_witness(1);
        witness.observe(1, &dead);
        assert!(
            !witness.slots[1].survived,
            "received hitpoints 0 clears survival"
        );
        let clear = witness.slots[1]
            .survival_cleared
            .as_ref()
            .expect("the clearing observation is recorded");
        assert_eq!(clear.tick, 3_481);
        assert_eq!(clear.hp, 0);
        assert_eq!(clear.tile, Some((KQ_BANK.x, KQ_BANK.z, KQ_BANK.level)));
        assert_eq!(clear.chat.as_deref(), Some("Oh dear, you are dead!"));
        let summary = witness.terminal_summary();
        assert!(
            summary.contains(
                "survival_cleared=s1:tick=3481:hp=0:tile=3308,3120,0:chat=Some(\"Oh dear, you are dead!\")"
            ),
            "{summary}"
        );

        // Only the first clearing observation is kept.
        let mut later = with_hitpoints(Some((0, 99)));
        later.seed_tick(3_500);
        witness.observe(1, &later);
        let clear = witness.slots[1]
            .survival_cleared
            .as_ref()
            .expect("the record stays");
        assert_eq!(clear.tick, 3_481, "the first clear is the record");
    }

    /// The unmet list names the per-slot term that blocks completion,
    /// so it is never empty while `complete()` is false on a per-slot
    /// term alone. The integration receipt printed `unmet=[]` with
    /// one slot's `survived` false (INT-BATCH-1024E).
    #[test]
    fn terminal_summary_names_the_per_slot_term_that_blocks_completion() {
        let mut witness = complete_witness();
        assert!(witness.complete());
        witness.slots[3].survived = false;
        assert!(!witness.complete(), "survival still gates");
        let summary = witness.terminal_summary();
        let unmet_list = summary
            .split("unmet=[")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("unmet list in summary");
        assert_eq!(
            unmet_list, "s3:survived",
            "the one false term is named with its slot: {summary}"
        );

        witness.slots[3].survived = true;
        witness.slots[0].camelot_escape = false;
        witness.slots[2].disappearances = 1;
        assert!(!witness.complete());
        let summary = witness.terminal_summary();
        let unmet_list = summary
            .split("unmet=[")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .expect("unmet list in summary");
        assert_eq!(
            unmet_list, "s0:camelot_escape s2:disappearances>=2",
            "every false per-slot term is named with its slot: {summary}"
        );

        witness.slots[0].camelot_escape = true;
        witness.slots[2].disappearances = 2;
        assert!(witness.complete());
        assert!(
            witness
                .terminal_summary()
                .starts_with("kq_witness unmet=[]"),
            "the list is empty only once completion holds: {}",
            witness.terminal_summary()
        );
    }
}
