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

/// One poll with the server applied: the batch the machine emitted.
fn step(harness: &mut Harness, scene: &mut Scene, tick: u64) -> DrainedBatch {
    let polled = harness.poll(&scene.snapshot, tick);
    assert!(
        matches!(polled, Poll::Pending),
        "tick {tick}: the fight must continue, got {polled:?}"
    );
    let batch = harness.drain(None);
    apply_wears(scene, &batch);
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

#[test]
fn layout_report() {
    eprintln!(
        "LAYOUT Combat={} CombatReport={} CombatRequest={} SwitchState={} StyleKits={}",
        std::mem::size_of::<Combat>(),
        std::mem::size_of::<CombatReport>(),
        std::mem::size_of::<CombatRequest>(),
        std::mem::size_of::<SwitchState>(),
        std::mem::size_of::<StyleKits>()
    );
}
