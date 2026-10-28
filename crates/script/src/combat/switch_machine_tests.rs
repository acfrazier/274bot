//! PvP slice V2 (PVP-DESIGN-1026 §4, §10.1 cases 7-10, 14, 16): loadouts, the
//! switch state, the triggers, the pending Ranged need and the capabilities.
use super::pvp_tests::{fight_ops, pen_scene, player, player_attack, BOB};
use super::*;

const OPPONENT: usize = 8;

fn id(scene: &Scene, alias: &str) -> i32 {
    scene.data.item_by_alias(alias).unwrap().id
}

/// An obj worn in its own wear position.
fn worn_item(scene: &Scene, alias: &str) -> ItemView {
    let mut row = scene.held(alias, 0);
    row.container = ItemContainer::Equipment;
    row.slot = scene.data.item_by_alias(alias).unwrap().wear_position;
    row
}

/// `count` of an obj in the next free pack slot.
fn give(scene: &mut Scene, alias: &str, count: i32) {
    let mut row = scene.held(alias, scene.inventory.len() as i32);
    row.count = count;
    scene.inventory.push(row);
}

fn wear(scene: &mut Scene, alias: &str, count: i32) {
    let mut row = worn_item(scene, alias);
    row.count = count;
    scene.equipment.push(row);
}

fn kit(worn: &[(u8, i32)], carry: &[(i32, u32)]) -> Arc<CompiledKit> {
    Arc::new(CompiledKit {
        worn: Arc::from(worn),
        carry: Arc::from(carry),
        ..CompiledKit::default()
    })
}

fn request(kits: StyleKits) -> CombatRequest {
    CombatRequest {
        target: Target::Player {
            name: Arc::from(BOB),
        },
        pvp: true,
        fallback: Fallback::Fight,
        lost_radius: 32,
        kits,
        ..CombatRequest::default()
    }
}

/// The pen with `bob` one tile east, a sword worn and a bow with arrows held.
fn sword_and_bow() -> (Scene, StyleKits) {
    let mut scene = pen_scene();
    scene
        .players
        .push(player(OPPONENT, BOB, tile(3341, 3250), &fight_ops(), 60));
    wear(&mut scene, "rune_scimitar", 1);
    give(&mut scene, "maple_shortbow", 1);
    give(&mut scene, "iron_arrow", 30);
    scene.refresh();
    let kits = StyleKits::default()
        .with(
            Style::Melee,
            kit(&[(3, id(&scene, "rune_scimitar"))], &[]),
        )
        .with(
            Style::Ranged,
            kit(
                &[
                    (3, id(&scene, "maple_shortbow")),
                    (13, id(&scene, "iron_arrow")),
                ],
                &[],
            ),
        );
    (scene, kits)
}

fn effect_text(effect: &HostEffect) -> String {
    match effect {
        HostEffect::Interaction(request) => format!("{request:?}"),
        _ => "non-interaction effect".into(),
    }
}

fn rows(batch: &DrainedBatch) -> Vec<String> {
    (0..batch.len())
        .map(|index| effect_text(batch.get(index).unwrap()))
        .collect()
}

fn wear_names(batch: &DrainedBatch) -> Vec<String> {
    (0..batch.len())
        .filter_map(|index| match batch.get(index) {
            Some(HostEffect::Interaction(InteractReq::Wear { name })) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

fn has_attack(batch: &DrainedBatch) -> bool {
    (0..batch.len()).any(|index| {
        matches!(
            batch.get(index),
            Some(HostEffect::Interaction(InteractReq::Player { name, action }))
                if name == BOB && action == "Fight"
        )
    })
}

fn has_cast(batch: &DrainedBatch) -> bool {
    (0..batch.len()).any(|index| {
        matches!(
            batch.get(index),
            Some(HostEffect::Interaction(InteractReq::UseWidgetOn { .. }))
        )
    })
}

fn has_button(batch: &DrainedBatch, button: i32) -> bool {
    (0..batch.len()).any(|index| {
        matches!(
            batch.get(index),
            Some(HostEffect::Interaction(InteractReq::IfButton { component_id }))
                if *component_id == button
        )
    })
}

/// What the server does with an accepted wear: the obj leaves the pack for its
/// wear position; what was there (and, for a two-hander, the shield) goes back.
fn apply_wears(scene: &mut Scene, batch: &DrainedBatch) {
    for name in wear_names(batch) {
        let Some(at) = scene
            .inventory
            .iter()
            .position(|row| row.def.name.as_deref() == Some(name.as_str()))
        else {
            continue;
        };
        let mut item = scene.inventory.remove(at);
        let facts = scene.data.item_by_id(item.def.id).unwrap();
        let (position, second) = (facts.wear_position, facts.wear_position_2);
        for slot in [position, second] {
            if slot < 0 {
                continue;
            }
            if let Some(old) = scene.equipment.iter().position(|row| row.slot == slot) {
                let mut old = scene.equipment.remove(old);
                old.container = ItemContainer::Inventory;
                old.slot = scene.inventory.len() as i32;
                scene.inventory.push(old);
            }
        }
        item.container = ItemContainer::Equipment;
        item.slot = position;
        scene.equipment.push(item);
    }
    for (index, row) in scene.inventory.iter_mut().enumerate() {
        row.slot = index as i32;
    }
}

/// What the server does with an accepted prayer click (a toggle) or meal: the
/// varp flips, one lobster leaves the pack and heals 12.
fn apply_upkeep(scene: &mut Scene, batch: &DrainedBatch) {
    for index in 0..batch.len() {
        match batch.get(index) {
            Some(HostEffect::Interaction(InteractReq::IfButton { component_id })) => {
                if let Some(row) = scene
                    .data
                    .prayers()
                    .iter()
                    .find(|row| row.button_com == *component_id)
                {
                    let on = scene
                        .varps
                        .iter()
                        .any(|varp| varp.index == row.varp && varp.value == 1);
                    let varp = row.varp;
                    scene.prayer(varp, !on);
                }
            }
            Some(HostEffect::Interaction(InteractReq::Held { action, .. })) if action == "Eat" => {
                let lobster = id(scene, "lobster");
                if let Some(at) = scene.inventory.iter().position(|row| row.def.id == lobster) {
                    scene.inventory[at].count -= 1;
                    if scene.inventory[at].count == 0 {
                        scene.inventory.remove(at);
                    }
                    let hp = scene.stats[3].effective;
                    scene.stat(3, (hp + 12).min(40), 40);
                }
            }
            _ => {}
        }
    }
}

/// One poll with the server applied: the batch the machine emitted.
fn step(harness: &mut Harness, scene: &mut Scene, tick: u64) -> DrainedBatch {
    let polled = harness.poll(&scene.snapshot, tick);
    assert!(
        matches!(polled, Poll::Pending),
        "tick {tick}: the fight must continue, got {polled:?}"
    );
    let batch = harness.drain(None);
    apply_wears(scene, &batch);
    apply_upkeep(scene, &batch);
    scene.refresh();
    batch
}

/// Begin the fight and run the two first polls (the opening attack, then the
/// installed fight). Returns the harness; the next poll is tick 3.
fn start(scene: &mut Scene, request: CombatRequest) -> Harness {
    let mut harness = Harness::new(scene, request);
    player_attack(harness.pending(scene, 1), BOB, "Fight");
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: OPPONENT,
    });
    scene.local.player.actor.in_combat = true;
    scene.refresh();
    assert!(harness.pending(scene, 2).is_none());
    assert_eq!(harness.machine.phase, Phase::Fight);
    harness
}

fn protect(scene: &mut Scene, bits: i32) {
    scene.players[0].headicons = bits;
    scene.refresh();
}

const MELEE_PROTECT: i32 = 1 << 3;
const RANGED_PROTECT: i32 = 1 << 4;
const MAGE_PROTECT: i32 = 1 << 5;

fn frame_of<'a>(harness: &Harness, scene: &'a Scene) -> Frame<'a> {
    let evidence = harness.runtime.evidence.unwrap();
    Frame::borrow(SnapshotView::new(Some(&scene.snapshot), evidence)).unwrap()
}

fn converging_to(harness: &Harness) -> Option<Style> {
    harness.machine.switch_state().pending()
}

// --- Case 7: the switch on protection (T2) ----------------------------------

#[test]
fn t2_needs_two_protected_frames_then_converges_and_restyles() {
    let (mut scene, kits) = sword_and_bow();
    let mut harness = start(&mut scene, request(kits));
    let bow = id(&scene, "maple_shortbow");
    // One protected frame: nothing.
    protect(&mut scene, MELEE_PROTECT);
    step(&mut harness, &mut scene, 3);
    assert_eq!(harness.machine.switch_state(), SwitchState::Idle);
    // The second protected frame starts the switch; `active` has not moved.
    let batch = step(&mut harness, &mut scene, 4);
    assert_eq!(converging_to(&harness), Some(Style::Ranged), "{:?}", rows(&batch));
    assert_eq!(harness.machine.active_style(), Style::Melee);
    assert_eq!(wear_names(&batch), ["Maple shortbow", "Iron arrow"], "{:?}", rows(&batch));
    assert_eq!(batch.len(), 3, "[Wear bow, Wear arrows, Attack]: {:?}", rows(&batch));
    assert!(has_attack(&batch));
    // Both observed worn on the next frame: complete on that tick.
    let batch = step(&mut harness, &mut scene, 5);
    assert_eq!(harness.machine.active_style(), Style::Ranged);
    assert_eq!(harness.machine.switches(), 1);
    assert!(matches!(
        harness.machine.switch_state(),
        SwitchState::Settling { rate_old: 4, .. }
    ));
    assert_eq!(harness.machine.ranged().ammo_pick, id(&scene, "iron_arrow"));
    assert!(wear_names(&batch).is_empty());
    // The style row waits for the observed combat tab of the new weapon.
    let tab = scene.tables.weapon_style(bow).unwrap().tab.unwrap();
    let rapid = scene
        .data
        .ranged_modes()
        .iter()
        .find(|row| row.tab == tab as u8 && row.mode == RangedMode::Rapid as u8)
        .unwrap()
        .button;
    assert!(!has_button(&batch, rapid), "no style row before the tab");
    scene.combat_tab(scene.tables.combat_tab_root(tab).unwrap());
    let batch = step(&mut harness, &mut scene, 6);
    assert!(has_button(&batch, rapid), "{:?}", rows(&batch));
    assert!(has_attack(&batch), "[Style rapid, Attack]: {:?}", rows(&batch));
}

#[test]
fn t2_ignores_one_frame_the_same_mask_after_a_settle_and_a_flip_before_it() {
    let (mut scene, kits) = sword_and_bow();
    let mut harness = start(&mut scene, request(kits));
    // One protected frame, then the protect drops: never a switch.
    protect(&mut scene, MELEE_PROTECT);
    step(&mut harness, &mut scene, 3);
    protect(&mut scene, 0);
    step(&mut harness, &mut scene, 4);
    step(&mut harness, &mut scene, 5);
    assert_eq!(harness.machine.switches(), 0);
    assert_eq!(harness.machine.switch_state(), SwitchState::Idle);
    // Two protected frames: the switch converges and completes.
    protect(&mut scene, MELEE_PROTECT);
    let mut tick = 6;
    while harness.machine.switches() == 0 {
        step(&mut harness, &mut scene, tick);
        tick += 1;
        assert!(tick < 20, "the switch must complete");
    }
    assert_eq!(harness.machine.active_style(), Style::Ranged);
    let done = tick - 1;
    // The opponent flips to the new style before the settle ends: no switch back.
    protect(&mut scene, RANGED_PROTECT);
    for t in done + 1..done + 3 {
        step(&mut harness, &mut scene, t);
        assert!(
            matches!(harness.machine.switch_state(), SwitchState::Settling { .. }),
            "tick {t}: inside the settle"
        );
    }
    assert_eq!(harness.machine.switches(), 1);
    // The settle ends after rate_old + SETTLE_SLACK; the changed mask (bit 4 against bit 3 recorded at completion) switches once.
    let mut guard = 0;
    let mut t = done + 3;
    while harness.machine.switch_state().pending().is_none() {
        step(&mut harness, &mut scene, t);
        t += 1;
        guard += 1;
        assert!(guard < 30, "the changed mask must start a switch");
    }
    assert_eq!(harness.machine.active_style(), Style::Ranged);
    assert_eq!(harness.machine.switch_state().pending(), Some(Style::Melee));
}

// --- The named failure-mode tests of §4.6 -----------------------------------

#[test]
fn switch_zero_wear_completes_same_tick() {
    // A NoWeapons duel: slot 3 is locked, melee is unarmed, the mage kit is the
    // runes alone (manual casts).
    let mut scene = pen_scene();
    scene
        .players
        .push(player(OPPONENT, BOB, tile(3341, 3250), &fight_ops(), 60));
    give(&mut scene, "mindrune", 50);
    give(&mut scene, "airrune", 50);
    scene.refresh();
    let kits = StyleKits::default()
        .with(Style::Melee, kit(&[], &[]))
        .with(
            Style::Mage,
            kit(
                &[],
                &[(id(&scene, "mindrune"), 50), (id(&scene, "airrune"), 50)],
            ),
        );
    let mut req = request(kits);
    req.allow.locked_slots = 1 << 3;
    let mut harness = start(&mut scene, req);
    assert_eq!(harness.machine.active_style(), Style::Melee);
    protect(&mut scene, MELEE_PROTECT);
    let mut all = Vec::new();
    all.extend(rows(&step(&mut harness, &mut scene, 3)));
    // The trigger tick: completes with no wear row.
    let batch = step(&mut harness, &mut scene, 4);
    all.extend(rows(&batch));
    assert_eq!(harness.machine.active_style(), Style::Mage, "{all:?}");
    assert_eq!(harness.machine.switches(), 1);
    assert!(matches!(
        harness.machine.switch_state(),
        SwitchState::Settling { rate_old: 4, .. }
    ));
    assert!(matches!(harness.machine.style_state, StyleState::Magic(_)));
    assert!(wear_names(&batch).is_empty());
    // The mage kit casts by hand: a Cast row is planned on this or the next plan.
    let mut cast = has_cast(&batch);
    for t in 5..9 {
        let batch = step(&mut harness, &mut scene, t);
        assert!(wear_names(&batch).is_empty());
        cast |= has_cast(&batch);
    }
    assert!(cast, "the manual cast follows the zero-wear switch: {all:?}");
}

#[test]
fn switch_failed_keeps_old_style_state() {
    let (mut scene, kits) = sword_and_bow();
    let mut harness = start(&mut scene, request(kits));
    protect(&mut scene, MELEE_PROTECT);
    // The server never equips: every wear row times out.
    let mut saw_converging = false;
    let mut tick = 3;
    while harness.machine.switch_failures() == 0 {
        let batch = step_without_server(&mut harness, &mut scene, tick);
        saw_converging |= converging_to(&harness) == Some(Style::Ranged);
        // Every Attack row of the convergence targets the engaged row.
        for index in 0..batch.len() {
            if let Some(HostEffect::Interaction(InteractReq::Player { name, action })) =
                batch.get(index)
            {
                assert_eq!((name.as_str(), action.as_str()), (BOB, "Fight"));
            }
        }
        tick += 1;
        assert!(tick < 60, "two refusals must fail the switch");
    }
    assert!(saw_converging);
    assert_eq!(harness.machine.active_style(), Style::Melee);
    assert_eq!(harness.machine.switches(), 0);
    assert_eq!(harness.machine.switch_failures(), 1);
    assert!(matches!(
        harness.machine.switch_state(),
        SwitchState::Settling { .. }
    ));
    assert!(matches!(harness.machine.style_state, StyleState::Melee(_)));
    // The style is not tried again this fight, and the fight goes on.
    for t in tick..tick + 20 {
        step_without_server(&mut harness, &mut scene, t);
        assert_ne!(converging_to(&harness), Some(Style::Ranged));
    }
    assert_eq!(harness.machine.end, None);
}

/// A poll whose emitted rows the server accepts but never acts on.
fn step_without_server(harness: &mut Harness, scene: &mut Scene, tick: u64) -> DrainedBatch {
    let polled = harness.poll(&scene.snapshot, tick);
    assert!(
        matches!(polled, Poll::Pending),
        "tick {tick}: the fight must continue, got {polled:?}"
    );
    let batch = harness.drain(None);
    scene.refresh();
    batch
}

#[test]
fn switch_right_hand_vanishes_before_completion() {
    let (mut scene, kits) = sword_and_bow();
    let mut harness = start(&mut scene, request(kits));
    protect(&mut scene, MELEE_PROTECT);
    step(&mut harness, &mut scene, 3);
    // Tick 4 plans [Wear bow, Wear arrows, Attack]; only the quiver is applied.
    let polled = harness.poll(&scene.snapshot, 4);
    assert!(matches!(polled, Poll::Pending));
    let batch = harness.drain(None);
    assert_eq!(wear_names(&batch), ["Maple shortbow", "Iron arrow"]);
    let arrows = scene
        .inventory
        .iter()
        .position(|row| row.def.id == id(&scene, "iron_arrow"))
        .unwrap();
    let mut quiver = scene.inventory.remove(arrows);
    quiver.container = ItemContainer::Equipment;
    quiver.slot = 13;
    scene.equipment.push(quiver);
    scene.refresh();
    assert_eq!(converging_to(&harness), Some(Style::Ranged));
    // The quiver is observed first; the bow is still held: no completion.
    step_without_server(&mut harness, &mut scene, 5);
    assert_eq!(harness.machine.switches(), 0);
    // The bow disappears from the pack (a server-side removal).
    let bow = scene
        .inventory
        .iter()
        .position(|row| row.def.id == id(&scene, "maple_shortbow"))
        .unwrap();
    scene.inventory.remove(bow);
    scene.refresh();
    step_without_server(&mut harness, &mut scene, 6);
    assert_eq!(harness.machine.switch_failures(), 1);
    assert_eq!(harness.machine.switches(), 0);
    assert_eq!(harness.machine.active_style(), Style::Melee);
}

#[test]
fn switch_settle_covers_old_cooldown() {
    // A rate-7 two-hander against a rate-3 (rapid) bow.
    let mut scene = pen_scene();
    scene
        .players
        .push(player(OPPONENT, BOB, tile(3341, 3250), &fight_ops(), 60));
    wear(&mut scene, "bronze_2h_sword", 1);
    give(&mut scene, "maple_shortbow", 1);
    give(&mut scene, "iron_arrow", 30);
    scene.refresh();
    let kits = StyleKits::default()
        .with(Style::Melee, kit(&[(3, id(&scene, "bronze_2h_sword"))], &[]))
        .with(
            Style::Ranged,
            kit(
                &[
                    (3, id(&scene, "maple_shortbow")),
                    (13, id(&scene, "iron_arrow")),
                ],
                &[],
            ),
        );
    let mut harness = start(&mut scene, request(kits));
    protect(&mut scene, MELEE_PROTECT);
    let mut tick = 3;
    while harness.machine.switches() == 0 {
        step(&mut harness, &mut scene, tick);
        tick += 1;
        assert!(tick < 20);
    }
    let since = tick - 1;
    assert_eq!(
        harness.machine.switch_state(),
        SwitchState::Settling { since: since as u16, rate_old: 7 }
    );
    // The opponent protects against the new style on frames since + 3 and since + 4
    // and keeps it: no own swing is observed.
    for t in since + 1..since + 9 {
        if t == since + 3 {
            protect(&mut scene, RANGED_PROTECT);
        }
        step(&mut harness, &mut scene, t);
        assert!(
            matches!(harness.machine.switch_state(), SwitchState::Settling { .. }),
            "tick {t} is inside rate_old + 2 = 9 of the completion"
        );
    }
    // The first Idle tick evaluates the trigger (R2's `2 * rate(new) + 2 = 8`
    // cap would have switched a tick earlier).
    step(&mut harness, &mut scene, since + 9);
    assert_eq!(converging_to(&harness), Some(Style::Melee));
}

#[test]
fn switch_t2_rotates_past_two_styles() {
    for (opening, expected) in [
        (
            Style::Melee,
            [Style::Melee, Style::Ranged, Style::Mage, Style::Melee],
        ),
        (
            Style::Ranged,
            [Style::Ranged, Style::Mage, Style::Melee, Style::Ranged],
        ),
    ] {
        let mut scene = pen_scene();
        scene
            .players
            .push(player(OPPONENT, BOB, tile(3341, 3250), &fight_ops(), 60));
        if opening == Style::Melee {
            wear(&mut scene, "rune_scimitar", 1);
            give(&mut scene, "maple_shortbow", 1);
        } else {
            wear(&mut scene, "maple_shortbow", 1);
            give(&mut scene, "rune_scimitar", 1);
        }
        give(&mut scene, "air_battlestaff", 1);
        give(&mut scene, "iron_arrow", 30);
        give(&mut scene, "mindrune", 100);
        scene.refresh();
        if opening == Style::Ranged {
            // The arrows start in the quiver.
            let mut row = scene.inventory.remove(
                scene
                    .inventory
                    .iter()
                    .position(|row| row.def.id == id(&scene, "iron_arrow"))
                    .unwrap(),
            );
            row.container = ItemContainer::Equipment;
            row.slot = 13;
            scene.equipment.push(row);
            // A ranged opening waits for the exact combat tab and the style echo.
            let bow = id(&scene, "maple_shortbow");
            let tab = scene.tables.weapon_style(bow).unwrap().tab.unwrap();
            let rapid = scene
                .data
                .ranged_modes()
                .iter()
                .find(|row| row.tab == tab as u8 && row.mode == RangedMode::Rapid as u8)
                .unwrap()
                .slot;
            scene.varps.push(VarpView {
                index: scene.data.ranged_mode_varp().unwrap(),
                value: i32::from(rapid),
            });
            scene.refresh();
            scene.combat_tab(scene.tables.combat_tab_root(tab).unwrap());
        }
        let kits = StyleKits::default()
            .with(Style::Melee, kit(&[(3, id(&scene, "rune_scimitar"))], &[]))
            .with(
                Style::Ranged,
                kit(
                    &[
                        (3, id(&scene, "maple_shortbow")),
                        (13, id(&scene, "iron_arrow")),
                    ],
                    &[],
                ),
            )
            .with(
                Style::Mage,
                kit(
                    &[(3, id(&scene, "air_battlestaff"))],
                    &[(id(&scene, "mindrune"), 100)],
                ),
            );
        let mut req = request(kits);
        req.style = opening;
        req.switch = SwitchPolicy::for_style(opening);
        req.switch.prefer = if opening == Style::Ranged {
            [Style::Ranged, Style::Mage, Style::Melee]
        } else {
            [Style::Melee, Style::Ranged, Style::Mage]
        };
        let mut harness = start(&mut scene, req);
        let mut seen = vec![harness.machine.active_style()];
        for tick in 3..400u64 {
            // The opponent protects against the style we executed last tick.
            scene.players[0].headicons = harness.machine.active_style().protect_bit() as i32;
            scene.refresh();
            step(&mut harness, &mut scene, tick);
            // T3 never fires while a protect bit is on.
            if let SwitchState::Converging { cause, .. } = harness.machine.switch_state() {
                assert_ne!(cause, SwitchCause::Prefer);
            }
            let active = harness.machine.active_style();
            if seen.last() != Some(&active) {
                seen.push(active);
            }
            if seen.len() == expected.len() {
                break;
            }
        }
        assert_eq!(seen, expected, "opening {opening:?}");
    }
}

/// Like `step`, but only the wears `keep` names reach the server.
fn step_keeping(
    harness: &mut Harness,
    scene: &mut Scene,
    tick: u64,
    keep: &[&str],
) -> DrainedBatch {
    let polled = harness.poll(&scene.snapshot, tick);
    assert!(
        matches!(polled, Poll::Pending),
        "tick {tick}: the fight must continue, got {polled:?}"
    );
    let batch = harness.drain(None);
    let names: Vec<String> = wear_names(&batch)
        .into_iter()
        .filter(|name| keep.contains(&name.as_str()))
        .collect();
    let mut filtered = DrainedBatch::default();
    for name in names {
        filtered.effects[filtered.len] = Some(HostEffect::Interaction(InteractReq::Wear { name }));
        filtered.len += 1;
    }
    apply_wears(scene, &filtered);
    scene.refresh();
    batch
}

fn item_name(scene: &Scene, alias: &str) -> String {
    scene
        .data
        .item_by_alias(alias)
        .unwrap()
        .name
        .clone()
        .unwrap()
}

// --- Case 7g: the pending Ranged need (Codex F1) ----------------------------

#[test]
fn switch_thrownaxe_needs_no_quiver() {
    let mut scene = pen_scene();
    scene
        .players
        .push(player(OPPONENT, BOB, tile(3341, 3250), &fight_ops(), 60));
    wear(&mut scene, "rune_scimitar", 1);
    give(&mut scene, "rune_thrownaxe", 20);
    scene.refresh();
    let axe = id(&scene, "rune_thrownaxe");
    let kits = StyleKits::default()
        .with(Style::Melee, kit(&[(3, id(&scene, "rune_scimitar"))], &[]))
        .with(Style::Ranged, kit(&[(3, axe)], &[]));
    let mut harness = start(&mut scene, request(kits));
    {
        let frame = frame_of(&harness, &scene);
        assert_eq!(
            harness.machine.pending_ranged_need(Style::Ranged, &frame),
            Some(RangedNeed::Thrown { weapon: axe })
        );
        assert_eq!(harness.machine.wear_set(Style::Ranged, &frame), 1 << 3);
    }
    protect(&mut scene, MELEE_PROTECT);
    step(&mut harness, &mut scene, 3);
    let batch = step(&mut harness, &mut scene, 4);
    assert_eq!(wear_names(&batch), [item_name(&scene, "rune_thrownaxe")]);
    assert_eq!(converging_to(&harness), Some(Style::Ranged));
    // Slot 3 shows the axe, slot 13 is still empty: complete.
    step(&mut harness, &mut scene, 5);
    assert!(scene.equipment.iter().all(|row| row.slot != 13));
    assert_eq!(harness.machine.active_style(), Style::Ranged);
    assert_eq!(harness.machine.ranged().ammo_pick, axe);
    assert_eq!(harness.machine.switches(), 1);
}

fn bow_and_iron_arrows_scene() -> (Scene, CombatRequest) {
    let mut scene = pen_scene();
    scene
        .players
        .push(player(OPPONENT, BOB, tile(3341, 3250), &fight_ops(), 60));
    wear(&mut scene, "rune_scimitar", 1);
    give(&mut scene, "maple_shortbow", 1);
    give(&mut scene, "iron_arrow", 30);
    scene.refresh();
    // The kit names steel arrows; only iron arrows are held.
    let kits = StyleKits::default()
        .with(Style::Melee, kit(&[(3, id(&scene, "rune_scimitar"))], &[]))
        .with(
            Style::Ranged,
            kit(
                &[
                    (3, id(&scene, "maple_shortbow")),
                    (13, id(&scene, "steel_arrow")),
                ],
                &[],
            ),
        );
    let req = request(kits);
    (scene, req)
}

#[test]
fn switch_bow_uses_held_fallback_ammo() {
    let (mut scene, req) = bow_and_iron_arrows_scene();
    let (bow, iron) = (id(&scene, "maple_shortbow"), id(&scene, "iron_arrow"));
    let mut harness = start(&mut scene, req);
    {
        let frame = frame_of(&harness, &scene);
        assert_eq!(
            harness.machine.pending_ranged_need(Style::Ranged, &frame),
            Some(RangedNeed::Quiver {
                weapon: bow,
                ammo: iron
            })
        );
        assert_eq!(
            harness.machine.wear_set(Style::Ranged, &frame),
            (1 << 3) | (1 << 13)
        );
    }
    protect(&mut scene, MELEE_PROTECT);
    step(&mut harness, &mut scene, 3);
    let batch = step(&mut harness, &mut scene, 4);
    assert_eq!(
        wear_names(&batch),
        [
            item_name(&scene, "maple_shortbow"),
            item_name(&scene, "iron_arrow")
        ],
        "the bow first, then the iron arrows"
    );
    step(&mut harness, &mut scene, 5);
    assert_eq!(harness.machine.active_style(), Style::Ranged);
    assert_eq!(harness.machine.ranged().ammo_pick, iron);
    assert_eq!(harness.machine.switches(), 1);
}

#[test]
fn switch_refused_mandatory_ammo_fails() {
    let (mut scene, req) = bow_and_iron_arrows_scene();
    let scimitar = item_name(&scene, "rune_scimitar");
    let bow = item_name(&scene, "maple_shortbow");
    let mut harness = start(&mut scene, req);
    protect(&mut scene, MELEE_PROTECT);
    let keep = [bow.as_str(), scimitar.as_str()];
    step_keeping(&mut harness, &mut scene, 3, &keep);
    let mut tick = 4;
    let mut rewore_scimitar = false;
    while harness.machine.switch_failures() == 0 {
        let batch = step_keeping(&mut harness, &mut scene, tick, &keep);
        assert!(!has_cast(&batch));
        // The sword is only worn again once the switch has failed.
        rewore_scimitar |= wear_names(&batch).contains(&scimitar);
        tick += 1;
        assert!(tick < 60, "two refusals of the iron arrows must fail the switch");
    }
    // The bow is worn, the quiver never was: a bow with no arrows must not be a completion.
    assert_eq!(harness.machine.active_style(), Style::Melee);
    assert_eq!(harness.machine.switches(), 0);
    assert!(matches!(harness.machine.style_state, StyleState::Melee(_)));
    for t in tick..tick + 12 {
        let batch = step_keeping(&mut harness, &mut scene, t, &keep);
        rewore_scimitar |= wear_names(&batch).contains(&scimitar);
    }
    assert!(rewore_scimitar, "the ordinary repair wears the sword again");
    assert!(scene
        .equipment
        .iter()
        .any(|row| row.slot == 3 && row.def.id == id(&scene, "rune_scimitar")));
}

// --- Case 7b/7c/7e/7f: upkeep, refusals, two-handers, armour ----------------

#[test]
fn a_client_refused_first_wear_keeps_converging_and_retries_next_tick() {
    let (mut scene, kits) = sword_and_bow();
    let mut harness = start(&mut scene, request(kits));
    protect(&mut scene, MELEE_PROTECT);
    step(&mut harness, &mut scene, 3);
    // Tick 4: the first row (the bow) is refused by the client; the batch fail-stops.
    assert!(matches!(harness.poll(&scene.snapshot, 4), Poll::Pending));
    let batch = harness.drain(Some(0));
    assert_eq!(wear_names(&batch).len(), 2, "{:?}", rows(&batch));
    assert_eq!(converging_to(&harness), Some(Style::Ranged));
    assert_eq!(harness.machine.switch_failures(), 0);
    // Tick 5: the wear is planned again.
    let batch = step(&mut harness, &mut scene, 5);
    assert_eq!(
        wear_names(&batch).first().map(String::as_str),
        Some("Maple shortbow"),
        "{:?}",
        rows(&batch)
    );
    step(&mut harness, &mut scene, 6);
    assert_eq!(harness.machine.active_style(), Style::Ranged);
}

#[test]
fn a_full_armour_change_wears_the_right_hand_first_over_two_plans() {
    let mut scene = pen_scene();
    scene
        .players
        .push(player(OPPONENT, BOB, tile(3341, 3250), &fight_ops(), 60));
    wear(&mut scene, "rune_scimitar", 1);
    for (alias, count) in [
        ("maple_shortbow", 1),
        ("iron_arrow", 30),
        ("rune_full_helm", 1),
        ("rune_platebody", 1),
        ("rune_platelegs", 1),
    ] {
        give(&mut scene, alias, count);
    }
    scene.refresh();
    let kits = StyleKits::default()
        .with(Style::Melee, kit(&[(3, id(&scene, "rune_scimitar"))], &[]))
        .with(
            Style::Ranged,
            kit(
                &[
                    (3, id(&scene, "maple_shortbow")),
                    (13, id(&scene, "iron_arrow")),
                    (0, id(&scene, "rune_full_helm")),
                    (4, id(&scene, "rune_platebody")),
                    (7, id(&scene, "rune_platelegs")),
                ],
                &[],
            ),
        );
    let mut harness = start(&mut scene, request(kits));
    protect(&mut scene, MELEE_PROTECT);
    step(&mut harness, &mut scene, 3);
    let first = step(&mut harness, &mut scene, 4);
    assert_eq!(
        wear_names(&first),
        [
            item_name(&scene, "maple_shortbow"),
            item_name(&scene, "iron_arrow"),
            item_name(&scene, "rune_full_helm")
        ],
        "right hand, quiver, then slot order"
    );
    assert_eq!(harness.machine.switches(), 0);
    // A wear paces the next one by two ticks (`OpKind::Wear` admission delay).
    assert!(wear_names(&step(&mut harness, &mut scene, 5)).is_empty());
    let second = step(&mut harness, &mut scene, 6);
    assert_eq!(
        wear_names(&second),
        [
            item_name(&scene, "rune_platebody"),
            item_name(&scene, "rune_platelegs")
        ]
    );
    assert_eq!(harness.machine.switches(), 0, "not before the last slot is observed");
    step(&mut harness, &mut scene, 7);
    assert_eq!(harness.machine.active_style(), Style::Ranged);
    assert_eq!(harness.machine.switches(), 1);
}

#[test]
fn a_shield_waits_for_the_right_hand_when_a_two_hander_gives_way() {
    let mut scene = pen_scene();
    scene
        .players
        .push(player(OPPONENT, BOB, tile(3341, 3250), &fight_ops(), 60));
    wear(&mut scene, "rune_2h_sword", 1);
    give(&mut scene, "air_battlestaff", 1);
    give(&mut scene, "wooden_shield", 1);
    give(&mut scene, "mindrune", 100);
    scene.refresh();
    let kits = StyleKits::default()
        .with(Style::Melee, kit(&[(3, id(&scene, "rune_2h_sword"))], &[]))
        .with(
            Style::Mage,
            kit(
                &[
                    (3, id(&scene, "air_battlestaff")),
                    (5, id(&scene, "wooden_shield")),
                ],
                &[(id(&scene, "mindrune"), 100)],
            ),
        );
    let mut req = request(kits);
    req.switch.prefer = [Style::Melee, Style::Mage, Style::Ranged];
    let mut harness = start(&mut scene, req);
    protect(&mut scene, MELEE_PROTECT);
    step(&mut harness, &mut scene, 3);
    let first = step(&mut harness, &mut scene, 4);
    assert_eq!(
        wear_names(&first),
        [item_name(&scene, "air_battlestaff")],
        "the shield is not in the plan of the two-hander's replacement: {:?}",
        rows(&first)
    );
    assert!(wear_names(&step(&mut harness, &mut scene, 5)).is_empty());
    let second = step(&mut harness, &mut scene, 6);
    assert_eq!(wear_names(&second), [item_name(&scene, "wooden_shield")]);
    assert_eq!(harness.machine.switches(), 0);
    step(&mut harness, &mut scene, 7);
    assert_eq!(harness.machine.active_style(), Style::Mage);
}

// --- Case 7d: out of reach while melee (T4) ---------------------------------

fn far_opponent(scene: &mut Scene, at: api::WorldTile) {
    scene.players[0].actor.tile = at;
    scene.players[0].network = at;
    scene.refresh();
}

#[test]
fn t4_switches_after_two_attack_intervals_out_of_reach() {
    let (mut scene, kits) = sword_and_bow();
    let mut harness = start(&mut scene, request(kits));
    far_opponent(&mut scene, tile(3345, 3250));
    // 2 * rate(4) = 8 observed ticks out of reach with no swing onset of ours.
    for tick in 3..10 {
        step(&mut harness, &mut scene, tick);
        assert_eq!(converging_to(&harness), None, "tick {tick}");
    }
    let batch = step(&mut harness, &mut scene, 10);
    assert_eq!(converging_to(&harness), Some(Style::Ranged), "{:?}", rows(&batch));
    assert!(matches!(
        harness.machine.switch_state(),
        SwitchState::Converging {
            cause: SwitchCause::Reach,
            ..
        }
    ));
    step(&mut harness, &mut scene, 11);
    assert_eq!(harness.machine.active_style(), Style::Ranged);
}

#[test]
fn t4_never_fires_beside_the_opponent_or_without_an_alternative() {
    // Adjacent: never.
    let (mut scene, kits) = sword_and_bow();
    let mut harness = start(&mut scene, request(kits));
    for tick in 3..40 {
        step(&mut harness, &mut scene, tick);
    }
    assert_eq!(harness.machine.switch_state(), SwitchState::Idle);
    // Out of reach but no non-melee kit: never.
    let (mut scene, mut kits) = sword_and_bow();
    kits.ranged = None;
    let mut harness = start(&mut scene, request(kits));
    far_opponent(&mut scene, tile(3345, 3250));
    for tick in 3..40 {
        step(&mut harness, &mut scene, tick);
    }
    assert_eq!(harness.machine.switch_state(), SwitchState::Idle);
    assert_eq!(harness.machine.switches(), 0);
}

#[test]
fn t4_waits_out_a_swing_onset_inside_the_window() {
    let (mut scene, kits) = sword_and_bow();
    let mut harness = start(&mut scene, request(kits));
    far_opponent(&mut scene, tile(3345, 3250));
    let seq = scene.melee_seq();
    for tick in 3..14 {
        if tick == 6 {
            // One swing of ours at tick 6 restarts the count.
            scene.local.player.actor.animation = seq;
            scene.local.player.actor.animation_frame = 0;
        } else {
            scene.local.player.actor.animation = -1;
        }
        scene.refresh();
        step(&mut harness, &mut scene, tick);
        assert_eq!(converging_to(&harness), None, "tick {tick}");
    }
    scene.local.player.actor.animation = -1;
    scene.refresh();
    step(&mut harness, &mut scene, 14);
    assert_eq!(converging_to(&harness), Some(Style::Ranged));
}

// --- Case 7b: upkeep leaves room for one wear; a drink admits none ----------

/// `bob` swings at us in melee while wearing Protect from Melee, we are hurt, hold
/// food and have prayer points: the arbiter has rows of its own every tick.
fn upkeep(scene: &mut Scene) {
    give(scene, "lobster", 5);
    let lobster = scene.inventory.len() - 1;
    scene.inventory[lobster].actions = vec![Some("Eat".into())];
    scene.stat(3, 12, 40);
    scene.stat(5, 50, 50);
    scene.players[0].actor.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: scene.local.player.index,
    });
    scene.players[0].actor.animation = scene.melee_seq();
    scene.players[0].actor.in_combat = true;
    scene.refresh();
}

fn has_held(batch: &DrainedBatch, verb: &str) -> bool {
    (0..batch.len()).any(|index| {
        matches!(
            batch.get(index),
            Some(HostEffect::Interaction(InteractReq::Held { action, .. })) if action == verb
        )
    })
}

#[test]
fn an_emergency_eat_and_a_protect_leave_room_for_one_wear() {
    let (mut scene, kits) = sword_and_bow();
    let mut harness = start(&mut scene, request(kits));
    upkeep(&mut scene);
    protect(&mut scene, MELEE_PROTECT);
    let mut tick = 3;
    let mut first_wear = None;
    let mut all = Vec::new();
    while first_wear.is_none() {
        let batch = step(&mut harness, &mut scene, tick);
        all.push((tick, rows(&batch)));
        eprintln!("DBGEAT t={tick} {:?} state={:?}", rows(&batch), harness.machine.switch_state());
        {
            let frame = frame_of(&harness, &scene);
            let m = &harness.machine;
            eprintln!(
                "DBGEAT   opp={} prev={} trig={:?} usable={} wearset={:b} wearready={} phase={:?}",
                m.opp_protect,
                m.opp_protect_prev,
                m.switch_trigger(&frame, false),
                m.style_usable(Style::Ranged, &frame),
                m.wear_set(Style::Ranged, &frame),
                m.schedule.ready(OpKind::Wear, tick as u16),
                m.phase
            );
        }
        if !wear_names(&batch).is_empty() {
            first_wear = Some((tick, batch));
        }
        tick += 1;
        assert!(tick < 30, "the switch must start: {all:?}");
    }
    let (at, batch) = first_wear.unwrap();
    assert_eq!(
        wear_names(&batch),
        ["Maple shortbow"],
        "one wear under upkeep at tick {at}: {all:?}"
    );
    assert!(
        batch.len() <= 4,
        "[upkeep, wear, attack] stays inside the five-event budget: {all:?}"
    );
    assert_eq!(harness.machine.switches(), 0);
    // The quiver follows on a later plan and only then does the switch complete.
    let mut quiver_tick = None;
    for t in tick..tick + 12 {
        let batch = step(&mut harness, &mut scene, t);
        if wear_names(&batch) == ["Iron arrow"] && quiver_tick.is_none() {
            quiver_tick = Some(t);
            assert_eq!(harness.machine.switches(), 0, "not after the first wear only");
        }
    }
    assert!(quiver_tick.is_some(), "the quiver is worn on a later plan");
    assert_eq!(harness.machine.active_style(), Style::Ranged);
}

#[test]
fn a_drink_on_the_trigger_tick_admits_no_wear_and_starts_nothing() {
    let (mut scene, kits) = sword_and_bow();
    let mut harness = start(&mut scene, request(kits));
    give(&mut scene, "1doseprayerrestore", 1);
    scene.stat(5, 0, 50);
    scene.players[0].actor.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: scene.local.player.index,
    });
    scene.players[0].actor.animation = scene.melee_seq();
    scene.players[0].actor.in_combat = true;
    scene.refresh();
    protect(&mut scene, MELEE_PROTECT);
    let mut drank = false;
    let mut started = None;
    for tick in 3..14 {
        let batch = step(&mut harness, &mut scene, tick);
        if has_held(&batch, "Drink") {
            drank = true;
            assert!(wear_names(&batch).is_empty(), "no wear in a drink plan");
            assert_eq!(converging_to(&harness), None, "no switch under a drink plan");
            // The sip lands: points are back, the vial is gone.
            scene.stat(5, 30, 50);
            let vial = id(&scene, "1doseprayerrestore");
            scene.inventory.retain(|row| row.def.id != vial);
            scene.refresh();
        } else if drank && converging_to(&harness).is_some() && started.is_none() {
            started = Some(tick);
        }
    }
    assert!(drank, "the scene must have a drink due");
    assert!(started.is_some(), "the switch starts once the drink is out of the way");
}

// --- Case 8: exhaustion (T1) -----------------------------------------------

/// A bow with two arrows in the quiver, a sword in the pack, a ranged opening.
fn ranged_opening(with_melee_kit: bool) -> (Scene, CombatRequest) {
    let mut scene = pen_scene();
    scene
        .players
        .push(player(OPPONENT, BOB, tile(3341, 3250), &fight_ops(), 60));
    wear(&mut scene, "maple_shortbow", 1);
    wear(&mut scene, "iron_arrow", 2);
    give(&mut scene, "rune_scimitar", 1);
    let bow = id(&scene, "maple_shortbow");
    let tab = scene.tables.weapon_style(bow).unwrap().tab.unwrap();
    let rapid = scene
        .data
        .ranged_modes()
        .iter()
        .find(|row| row.tab == tab as u8 && row.mode == RangedMode::Rapid as u8)
        .unwrap()
        .slot;
    scene.varps.push(VarpView {
        index: scene.data.ranged_mode_varp().unwrap(),
        value: i32::from(rapid),
    });
    scene.refresh();
    scene.combat_tab(scene.tables.combat_tab_root(tab).unwrap());
    let mut kits = StyleKits::default().with(
        Style::Ranged,
        kit(
            &[(3, bow), (13, id(&scene, "iron_arrow"))],
            &[],
        ),
    );
    if with_melee_kit {
        kits = kits.with(Style::Melee, kit(&[(3, id(&scene, "rune_scimitar"))], &[]));
    }
    let mut req = request(kits);
    req.style = Style::Ranged;
    req.switch = SwitchPolicy::for_style(Style::Ranged);
    (scene, req)
}

#[test]
fn exhausted_arrows_switch_to_melee_instead_of_aborting() {
    let (mut scene, req) = ranged_opening(true);
    let mut harness = start(&mut scene, req);
    assert_eq!(harness.machine.active_style(), Style::Ranged);
    // The last arrows are gone.
    scene.equipment.retain(|row| row.slot != 13);
    scene.refresh();
    let batch = step(&mut harness, &mut scene, 3);
    assert_eq!(converging_to(&harness), Some(Style::Melee), "{:?}", rows(&batch));
    assert!(matches!(
        harness.machine.switch_state(),
        SwitchState::Converging {
            cause: SwitchCause::Exhausted,
            ..
        }
    ));
    assert_eq!(wear_names(&batch), ["Rune scimitar"]);
    step(&mut harness, &mut scene, 4);
    assert_eq!(harness.machine.active_style(), Style::Melee);
    assert_eq!(harness.machine.switches(), 1);
    assert_eq!(harness.machine.end, None);
}

#[test]
fn exhausted_arrows_with_no_alternative_abort_as_before() {
    let (mut scene, req) = ranged_opening(false);
    let mut harness = start(&mut scene, req);
    scene.equipment.retain(|row| row.slot != 13);
    scene.refresh();
    let mut ended = None;
    for tick in 3..12 {
        if let Poll::Ready(result) = harness.poll(&scene.snapshot, tick) {
            ended = Some(result.unwrap().end);
            break;
        }
        harness.drain(None);
    }
    assert_eq!(
        ended,
        Some(CombatEnd::Aborted(AbortReason::Unprotected(Unprotected::NoAmmo)))
    );
}

#[test]
fn exhaustion_of_the_target_during_a_convergence_fails_the_switch() {
    let (mut scene, kits) = sword_and_bow();
    let mut harness = start(&mut scene, request(kits));
    protect(&mut scene, MELEE_PROTECT);
    step_without_server(&mut harness, &mut scene, 3);
    step_without_server(&mut harness, &mut scene, 4);
    assert_eq!(converging_to(&harness), Some(Style::Ranged));
    // The arrows vanish before anything is worn.
    let arrows = id(&scene, "iron_arrow");
    scene.inventory.retain(|row| row.def.id != arrows);
    scene.refresh();
    step_without_server(&mut harness, &mut scene, 5);
    assert_eq!(harness.machine.switch_failures(), 1);
    assert_eq!(harness.machine.active_style(), Style::Melee);
    assert!(matches!(
        harness.machine.switch_state(),
        SwitchState::Settling { .. }
    ));
}

#[test]
fn the_same_protect_mask_never_switches_twice() {
    // Melee -> Ranged on Protect from Melee; the arrows run out and T1 returns
    // to melee while the opponent still protects against melee. The mask is the
    // one recorded at the last completed switch: no rotation to the staff.
    let (mut scene, mut kits) = sword_and_bow();
    give(&mut scene, "air_battlestaff", 1);
    give(&mut scene, "mindrune", 100);
    kits = kits.with(
        Style::Mage,
        kit(
            &[(3, id(&scene, "air_battlestaff"))],
            &[(id(&scene, "mindrune"), 100)],
        ),
    );
    let mut harness = start(&mut scene, request(kits));
    protect(&mut scene, MELEE_PROTECT);
    let mut tick = 3;
    while harness.machine.switches() == 0 {
        step(&mut harness, &mut scene, tick);
        tick += 1;
        assert!(tick < 20);
    }
    assert_eq!(harness.machine.active_style(), Style::Ranged);
    scene.equipment.retain(|row| row.slot != 13);
    scene.refresh();
    while harness.machine.switches() < 2 {
        step(&mut harness, &mut scene, tick);
        tick += 1;
        assert!(tick < 60, "T1 must return to melee");
    }
    assert_eq!(harness.machine.active_style(), Style::Melee);
    for t in tick..tick + 30 {
        step(&mut harness, &mut scene, t);
        assert_eq!(
            converging_to(&harness),
            None,
            "tick {t}: the opponent's mask did not change"
        );
    }
    assert_eq!(harness.machine.switches(), 2);
}

// --- Case 9: locked slots and no equipment ----------------------------------

#[test]
fn no_equipment_allowance_means_no_wear_and_no_switch_evaluation() {
    let (mut scene, kits) = sword_and_bow();
    let mut req = request(kits);
    req.allow.equipment = false;
    let mut harness = start(&mut scene, req);
    protect(&mut scene, MELEE_PROTECT);
    for tick in 3..25 {
        let batch = step(&mut harness, &mut scene, tick);
        assert!(wear_names(&batch).is_empty());
        assert_eq!(harness.machine.switch_state(), SwitchState::Idle);
    }
    assert_eq!(harness.machine.active_style(), Style::Melee);
}

#[test]
fn a_locked_right_hand_is_never_worn_and_the_switch_completes_on_the_other_slots() {
    let mut scene = pen_scene();
    scene
        .players
        .push(player(OPPONENT, BOB, tile(3341, 3250), &fight_ops(), 60));
    give(&mut scene, "air_battlestaff", 1);
    give(&mut scene, "rune_full_helm", 1);
    give(&mut scene, "mindrune", 100);
    give(&mut scene, "airrune", 100);
    scene.refresh();
    let kits = StyleKits::default()
        .with(Style::Melee, kit(&[], &[]))
        .with(
            Style::Mage,
            kit(
                &[
                    (3, id(&scene, "air_battlestaff")),
                    (0, id(&scene, "rune_full_helm")),
                ],
                &[(id(&scene, "mindrune"), 100), (id(&scene, "airrune"), 100)],
            ),
        );
    let mut req = request(kits);
    req.allow.locked_slots = 1 << 3;
    let mut harness = start(&mut scene, req);
    protect(&mut scene, MELEE_PROTECT);
    step(&mut harness, &mut scene, 3);
    let batch = step(&mut harness, &mut scene, 4);
    assert_eq!(wear_names(&batch), [item_name(&scene, "rune_full_helm")]);
    step(&mut harness, &mut scene, 5);
    assert_eq!(harness.machine.active_style(), Style::Mage);
    assert!(scene.equipment.iter().all(|row| row.slot != 3));
}

// --- Case 10: the category and tier pick ------------------------------------

fn pick_scene(attack: i32, items: &[&str]) -> Harness {
    let mut scene = Scene::new("cow");
    scene.stat(0, attack, attack);
    for alias in items {
        give(&mut scene, alias, 1);
    }
    scene.refresh();
    Harness::new(&scene, scene.request())
}

fn melee_pick(harness: &Harness, scene_alias: &str) -> bool {
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    harness.machine.rhand_pick[Style::Melee.index()] == data.item_by_alias(scene_alias).unwrap().id
}

#[test]
fn the_melee_pick_is_the_highest_tier_the_attack_level_allows() {
    let steel_and_rune = ["steel_scimitar", "rune_scimitar"];
    assert!(melee_pick(&pick_scene(30, &steel_and_rune), "steel_scimitar"));
    assert!(melee_pick(&pick_scene(40, &steel_and_rune), "rune_scimitar"));
}

#[test]
fn an_obj_outside_the_old_name_family_is_picked_by_its_category() {
    // Excalibur (tier 20) is a melee-category weapon the display-name family
    // never listed; at attack 30 it beats the steel scimitar.
    assert!(melee_pick(
        &pick_scene(30, &["steel_scimitar", "excalibur"]),
        "excalibur"
    ));
}

#[test]
fn a_quest_gated_tier_and_a_slower_tie_are_not_picked() {
    // The dragon dagger's gate needs a quest varp the machine cannot see.
    assert!(melee_pick(
        &pick_scene(99, &["steel_scimitar", "dragon_dagger"]),
        "steel_scimitar"
    ));
    // Equal tier: the lower attack rate wins (scimitar 4 over two-hander 7).
    assert!(melee_pick(
        &pick_scene(10, &["bronze_2h_sword", "bronze_scimitar"]),
        "bronze_scimitar"
    ));
}

// --- Case 14: legal capabilities (the machine's half) ------------------------

#[test]
fn no_weapons_opens_unarmed_melee_and_never_wears_the_ranged_kit() {
    let (mut scene, kits) = sword_and_bow();
    let mut req = request(kits);
    req.style = Style::Ranged; // the ranged weapon cannot be worn: not legal
    req.allow.locked_slots = 1 << 3;
    let mut harness = Harness::new(&scene, req);
    assert_eq!(harness.machine.active_style(), Style::Melee);
    player_attack(harness.pending(&scene, 1), BOB, "Fight");
    scene.refresh();
    protect(&mut scene, MELEE_PROTECT);
    for tick in 2..20 {
        let batch = step(&mut harness, &mut scene, tick);
        assert!(wear_names(&batch).is_empty(), "{:?}", rows(&batch));
    }
    assert_eq!(harness.machine.active_style(), Style::Melee);
}

#[test]
fn the_kit_less_style_is_never_opened_when_kits_are_declared() {
    // NoMelee with a bow and arrows: only the ranged kit, so ranged opens even
    // though `request.style` still says melee.
    let (mut scene, mut kits) = sword_and_bow();
    kits.melee = None;
    let bow = id(&scene, "maple_shortbow");
    let tab = scene.tables.weapon_style(bow).unwrap().tab.unwrap();
    scene.combat_tab(scene.tables.combat_tab_root(tab).unwrap());
    let harness = Harness::new(&scene, request(kits));
    assert_eq!(harness.machine.active_style(), Style::Ranged);
    // NoWeapons + NoMelee with runes: manual magic, no staff to wear.
    let mut scene = pen_scene();
    scene
        .players
        .push(player(OPPONENT, BOB, tile(3341, 3250), &fight_ops(), 60));
    give(&mut scene, "mindrune", 50);
    give(&mut scene, "airrune", 50);
    scene.refresh();
    let kits = StyleKits::default().with(
        Style::Mage,
        kit(&[], &[(id(&scene, "mindrune"), 50), (id(&scene, "airrune"), 50)]),
    );
    let mut req = request(kits);
    req.allow.locked_slots = 1 << 3;
    let harness = Harness::new(&scene, req);
    assert_eq!(harness.machine.active_style(), Style::Mage);
    // A request whose only kit is of an illegal style opens nothing.
    let (scene, mut kits) = sword_and_bow();
    kits.melee = None;
    let mut req = request(kits);
    req.allow.locked_slots = 1 << 3;
    assert!(matches!(
        Harness::try_new(&scene, req, 0),
        Err(ActionError::Unavailable(_))
    ));
}

#[test]
fn a_melee_only_request_never_switches() {
    // NoRanged + NoMagic: only the melee kit.
    let (mut scene, mut kits) = sword_and_bow();
    kits.ranged = None;
    let mut harness = start(&mut scene, request(kits));
    protect(&mut scene, MELEE_PROTECT);
    for tick in 3..40 {
        step(&mut harness, &mut scene, tick);
    }
    assert_eq!(harness.machine.switch_state(), SwitchState::Idle);
    assert_eq!(harness.machine.switches(), 0);
}

// --- Case 16: memory --------------------------------------------------------

#[test]
fn the_machine_layout_stays_inside_its_tripwires() {
    eprintln!(
        "LAYOUT Combat={} CombatReport={} CombatRequest={} SwitchState={} StyleKits={} SwitchPolicy={}",
        std::mem::size_of::<Combat>(),
        std::mem::size_of::<CombatReport>(),
        std::mem::size_of::<CombatRequest>(),
        std::mem::size_of::<SwitchState>(),
        std::mem::size_of::<StyleKits>(),
        std::mem::size_of::<SwitchPolicy>(),
    );
    assert!(std::mem::size_of::<Combat>() <= MACHINE_BYTES);
    assert!(std::mem::size_of::<CombatReport>() <= 80);
}

/// Bytes of an idle `Combat` after V2 (measured: 520 + 24); the exact per-bot pin
/// is in `host-play`.
const MACHINE_BYTES: usize = 544;

#[test]
fn a_steady_pvp_fight_with_kits_allocates_nothing_for_two_hundred_ticks() {
    let (mut scene, kits) = sword_and_bow();
    let mut harness = start(&mut scene, request(kits));
    scene.local.player.actor.animation = scene.melee_seq();
    scene.local.player.actor.animation_frame = 0;
    scene.snapshot.seed_local_player(scene.local.clone());
    assert!(harness.pending(&scene, 3).is_none());
    let allocations = allocation_counter::measure(|| {
        for tick in 4..204 {
            scene.local.player.actor.animation_frame = (tick % 4) as i32;
            scene.snapshot.seed_local_player(scene.local.clone());
            assert!(matches!(harness.poll(&scene.snapshot, tick), Poll::Pending));
            assert!(harness.runtime.ledger.as_ref().unwrap().outbox.is_empty());
        }
    });
    assert_eq!(allocations.count_total, 0);
    assert_eq!(harness.machine.switch_state(), SwitchState::Idle);
}

#[test]
fn a_completed_switch_allocates_exactly_its_style_box() {
    // Melee -> Ranged: one `Box<RangedState>`.
    let (mut scene, kits) = sword_and_bow();
    let mut harness = start(&mut scene, request(kits));
    protect(&mut scene, MELEE_PROTECT);
    step(&mut harness, &mut scene, 3);
    step(&mut harness, &mut scene, 4);
    assert_eq!(converging_to(&harness), Some(Style::Ranged));
    let frame_scene = &scene;
    let frame = frame_of(&harness, frame_scene);
    let allocations = allocation_counter::measure(|| {
        harness.machine.switch_observe(&frame, 5);
    });
    assert_eq!(harness.machine.active_style(), Style::Ranged);
    assert_eq!(allocations.count_total, 1, "{allocations:?}");
    // Ranged -> Melee: the melee state is inline, no allocation at all (the
    // boxed ranged state is freed, which the counter does not count as one).
    let mut scene = scene;
    protect(&mut scene, RANGED_PROTECT);
    let mut tick = 6;
    while converging_to(&harness) != Some(Style::Melee) {
        step(&mut harness, &mut scene, tick);
        tick += 1;
        assert!(tick < 40);
    }
    let frame = frame_of(&harness, &scene);
    let allocations = allocation_counter::measure(|| {
        harness.machine.switch_observe(&frame, tick as u16);
    });
    assert_eq!(harness.machine.active_style(), Style::Melee);
    assert_eq!(allocations.count_total, 0, "{allocations:?}");
}
