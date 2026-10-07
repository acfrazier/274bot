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
/// Selected-revision Kalphite Queen drops which cannot be confused with the
/// seeded loadout.  Ammunition is deliberately excluded: recovered arrows
/// are not boss-loot evidence.
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
        let in_pen = pair_near(
            snap,
            WorldTile {
                x: 3374,
                z: 3250,
                level: 0,
            },
            12,
        );
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
    loot_ground: Option<i32>,
    loot_collected: bool,
    loot_banked: bool,
    bank_returns: u8,
    bank_return_latched: bool,
    safely_banked: bool,
}

#[derive(Default)]
struct KqFleetWitness {
    started: bool,
    slots: [KqSlotWitness; 4],
    restock_overlap: bool,
    premature_departure: bool,
    leader_had_two_ropes: bool,
    surface_rope_count: Option<i32>,
    surface_rope_valid: bool,
    upper_rope_valid: bool,
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
        slot.survived &= pair_effective_stat(snap, 3) > 0;
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

    fn first_loot_bank(&self) -> bool {
        self.slots
            .iter()
            .all(|slot| slot.disappearances >= 1 && slot.bank_returns >= 1)
            && self.slots.iter().any(|slot| slot.kills >= 1)
            && self
                .slots
                .iter()
                .any(|slot| slot.loot_ground.is_some() && slot.loot_collected && slot.loot_banked)
    }

    fn second_descent(&self) -> bool {
        self.slots
            .iter()
            .all(|slot| slot.lair_entries >= 2 && slot.second_attack)
    }

    fn complete(&self) -> bool {
        self.first_descent()
            && self.first_combat()
            && self.first_loot_bank()
            && self.second_descent()
            && self.restock_overlap
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
            // The live hosted-title pump normally delivers an off-world
            // observe. The session-generation edge remains the fallback when
            // ownership moves directly into a queued handshake before that
            // frame is observed. last_login_reconnect may already be true from
            // an earlier grant and must not admit stale pre-logout scene2.
            if pair_relog_seen(
                slot.logout_sent,
                slot.saw_logout,
                snap.ingame(),
                slot.logout_session,
                c.gens.session,
            ) {
                slot.saw_logout = true;
                if !snap.ingame() {
                    return;
                }
            }
            if slot.saw_logout && snap.ingame() && snap.scene_state() == 2 && inv_tab {
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
        witness
            .lock()
            .expect("KQ witness mutex")
            .observe(slot.profile, &snap);
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
/// 954". Eight covers every descent the deadline allows; Beta 1
/// qualification starts from here.
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
    ScenarioSettings {
        full_rate: true,
        only_render_selected: false,
        require_mainland_base: true,
        deadline: SCRIPT_GOLD_DEADLINE,
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
    steps.push(start_catalog_step());
    Scenario {
        name: "flax_runner",
        seed: pair_fleet_seed(),
        steps,
        proof: Proof::Stat { id: 16, min: 0 },
        companions: vec![pair_companion(PairCompanionKind::FlaxSpinner)],
        settings: pair_watch_settings("flax_runner", "FlaxRunner"),
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
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks,
        },
    }
}

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
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Four-account max-stat JiveKQ gold: all members share one minted roster,
/// provision from Shantay, descend together, kill both Queen phases, collect
/// one non-loadout drop, and carry it through the next fresh bank. JiveKQ is
/// not Start-enabled in 0.1.9 (its catalog card is dim); this witness Starts
/// it past the dim for the Beta 1 qualification (see [`KQ_LEADER_ROPES`]).
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
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 2400,
        },
    });
    let loot_witness = Arc::clone(&witness);
    steps.push(Step {
        name: "observe alive-zero-absent kill and natural ground-inventory-bank loot",
        kind: StepKind::Await {
            evidence: "kq_kill_natural_non_arrow_ground_inventory_bank",
            ready: Box::new(move |snap| {
                let mut witness = loot_witness.lock().expect("KQ witness mutex");
                witness.observe(0, snap);
                witness.first_loot_bank()
            }),
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
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: 3600,
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
            deadline: Duration::from_secs(1_800),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod prep_barrier_tests {
    use super::{post_seed_scene_ready, PairPrepBarrier};

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
