use super::*;
use crate::combat::CastNpc;
use api::snapshot::{WidgetKind, WidgetRoot, WidgetView};

pub(super) fn magic_scene(staff: Option<&str>, level: i32) -> Scene {
    let mut scene = Scene::new("khazard_warlord");
    scene.stat(6, level, level);
    scene
        .varps
        .iter_mut()
        .find(|row| row.index == super::super::super::OPTION_NODEF)
        .unwrap()
        .value = 1;
    if let Some(staff) = staff {
        let mut worn = scene.held(staff, 3);
        worn.container = ItemContainer::Equipment;
        scene.equipment.push(worn);
    }
    for (slot, alias) in [
        "airrune",
        "firerune",
        "waterrune",
        "earthrune",
        "mindrune",
        "chaosrune",
        "deathrune",
        "bloodrune",
    ]
    .into_iter()
    .enumerate()
    {
        let mut rune = scene.held(alias, slot as i32);
        rune.count = 100;
        scene.inventory.push(rune);
    }
    let controls = scene.data.autocast_controls().unwrap();
    scene.varps.push(VarpView {
        index: controls.magic_varp,
        value: controls.armed_value,
    });
    refresh_magic(&mut scene, 3, false);
    scene
}
pub(super) fn refresh_magic(scene: &mut Scene, value: i32, panel: bool) {
    let controls = scene.data.autocast_controls().unwrap();
    let (varp, root) = (
        controls.magic_varp,
        if panel {
            controls.spell_panel_root
        } else {
            controls.staff_tab_root
        },
    );
    scene
        .varps
        .iter_mut()
        .find(|row| row.index == varp)
        .unwrap()
        .value = value;
    scene.refresh();
    scene.combat_tab(root);
}
fn request(scene: &Scene, spells: Option<&[&str]>, fallback: bool) -> CombatRequest {
    CombatRequest {
        style: Style::Mage,
        spells: spells.map(|rows| {
            rows.iter()
                .map(|alias| SpellRef {
                    alias: Arc::from(*alias),
                })
                .collect::<Vec<_>>()
                .into()
        }),
        fallback_spells: fallback,
        retaliate: false,
        ..scene.request()
    }
}
fn press(effect: Option<HostEffect>, expected: i32) {
    assert!(
        matches!(effect, Some(HostEffect::Interaction(InteractReq::IfButton { component_id })) if component_id == expected)
    );
}
fn cast(effect: Option<HostEffect>, expected: i32) {
    assert!(
        matches!(effect, Some(HostEffect::Interaction(InteractReq::UseWidgetOn { component_id, index: Some(7), .. })) if component_id == expected)
    );
}
fn component(scene: &Scene, alias: &str) -> i32 {
    let index = magic::spell_index(&scene.tables, alias).unwrap();
    scene.data.spells()[usize::from(index)].component_id
}
pub(super) fn spend(scene: &mut Scene, alias: &str) {
    let index = magic::spell_index(&scene.tables, alias).unwrap();
    let spell = &scene.data.spells()[usize::from(index)];
    let rhand = scene
        .equipment
        .iter()
        .find(|row| row.slot == 3)
        .map_or(-1, |row| row.def.id);
    for rune in &spell.runes {
        if !magic::staff_provides(&scene.tables, rhand, rune.id, true) {
            scene
                .inventory
                .iter_mut()
                .find(|row| row.def.id == rune.id)
                .unwrap()
                .count -= rune.count;
        }
    }
}
fn set_count(scene: &mut Scene, alias: &str, count: i32) {
    let id = scene.data.item_by_alias(alias).unwrap().id;
    scene
        .inventory
        .iter_mut()
        .find(|row| row.def.id == id)
        .unwrap()
        .count = count;
}
fn arm(harness: &mut Harness, scene: &mut Scene, start: u64, name: &str) {
    let controls = scene.data.autocast_controls().unwrap();
    let (choose, toggle, selected) = (
        controls.choose_com,
        controls.toggle_com,
        scene.data.spell_button_com(name),
    );
    press(harness.pending(scene, start), choose);
    refresh_magic(scene, 3, true);
    press(harness.pending(scene, start + 1), selected);
    refresh_magic(scene, 2, false);
    press(harness.pending(scene, start + 2), toggle);
    refresh_magic(scene, 3, false);
    attack(harness.pending(scene, start + 3));
}

fn seed_autocast_widget_text(
    scene: &mut Scene,
    text: Option<&str>,
    component_id: i32,
    root_component_id: i32,
    active: bool,
    visible: bool,
    hidden: bool,
) {
    let widgets = text
        .map(|text| {
            vec![WidgetView {
                kind: WidgetKind::Widget,
                component_id,
                layer_id: 0,
                parent_id: root_component_id,
                root_component_id,
                root: WidgetRoot::Side,
                type_: 4,
                button_type: 0,
                client_code: 0,
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                scroll_height: 0,
                inventory_ready: false,
                scroll_position: 0,
                hidden,
                text: Some(text.into()),
                alternate_text: None,
                button_text: None,
                target_verb: None,
                target_base: None,
                target_mask: 0,
                model_type: 0,
                model_id: 0,
                alternate_model_type: 0,
                alternate_model_id: 0,
                scripts: None,
                script_comparators: None,
                script_operands: None,
                varp_bindings: Vec::new(),
                colour: 0,
                actions: Vec::new(),
                items: Vec::new(),
            }]
        })
        .unwrap_or_default();
    scene.snapshot.seed_side_tabs(
        vec![SideTabView {
            index: 0,
            root_component_id,
            available: true,
            active,
            visible,
            widgets,
        }],
        if active { 0 } else { 1 },
    );
}

fn is_arm_control(effect: Option<HostEffect>, choose: i32, toggle: i32, spell: i32) -> bool {
    match effect {
        Some(HostEffect::Interaction(InteractReq::SideTab { tab: 0 })) => true,
        Some(HostEffect::Interaction(InteractReq::IfButton { component_id })) => {
            [choose, toggle, spell].contains(&component_id)
        }
        _ => false,
    }
}

#[test]
fn fresh_autocast_combat_adopts_visible_matching_armed_spell() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = magic_scene(Some("staff_of_fire"), 35);
    let controls = *scene.data.autocast_controls().unwrap();
    let spell_component = scene.data.spell_button_com("Fire Bolt");
    seed_autocast_widget_text(
        &mut scene,
        Some("Fire Bolt"),
        controls.spell_text_component,
        controls.staff_tab_root,
        true,
        true,
        false,
    );
    let mut harness = Harness::new(&scene, request(&scene, None, false));

    let effect = harness.pending(&scene, 1);
    assert!(
        !is_arm_control(
            effect,
            controls.choose_com,
            controls.toggle_com,
            spell_component
        ),
        "matching armed autocast state was rearmed"
    );
    let selected = harness.machine.magic().selected.unwrap();
    assert_eq!(scene.data.spells()[usize::from(selected)].name, "Fire Bolt");
    assert!(harness.machine.magic().armed);
    assert!(harness.machine.magic().arm.is_none());
    assert!(harness.machine.magic().initial_arm_checked);
}

#[test]
fn wrong_missing_hidden_stale_or_unarmed_autocast_state_starts_serial_arm() {
    if !api::test_support::game_data_available() {
        return;
    }
    let cases = [
        ("wrong spell", Some("Wind Strike"), 3, true, true, false),
        ("missing spell text", None, 3, true, true, false),
        ("hidden spell text", Some("Fire Bolt"), 3, true, true, true),
        (
            "invisible stale text",
            Some("Fire Bolt"),
            3,
            true,
            false,
            false,
        ),
        ("unarmed varp", Some("Fire Bolt"), 2, true, true, false),
    ];
    for (case, text, varp, active, visible, hidden) in cases {
        let mut scene = magic_scene(Some("staff_of_fire"), 35);
        refresh_magic(&mut scene, varp, false);
        let controls = *scene.data.autocast_controls().unwrap();
        seed_autocast_widget_text(
            &mut scene,
            text,
            controls.spell_text_component,
            controls.staff_tab_root,
            active,
            visible,
            hidden,
        );
        let mut harness = Harness::new(&scene, request(&scene, None, false));

        press(harness.pending(&scene, 1), controls.choose_com);
        let selected = harness.machine.magic().selected.unwrap();
        assert_eq!(
            scene.data.spells()[usize::from(selected)].name,
            "Fire Bolt",
            "{case}"
        );
        assert!(!harness.machine.magic().armed, "{case}");
        assert!(harness.machine.magic().arm.is_some(), "{case}");
        assert!(harness.machine.magic().initial_arm_checked, "{case}");
    }
}

#[test]
fn inactive_or_wrong_staff_tree_cannot_adopt_autocast_state() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut inactive = magic_scene(Some("staff_of_fire"), 35);
    let controls = *inactive.data.autocast_controls().unwrap();
    seed_autocast_widget_text(
        &mut inactive,
        Some("Fire Bolt"),
        controls.spell_text_component,
        controls.staff_tab_root,
        false,
        false,
        false,
    );
    let mut harness = Harness::new(&inactive, request(&inactive, None, false));
    assert!(matches!(
        harness.pending(&inactive, 1),
        Some(HostEffect::Interaction(InteractReq::SideTab { tab: 0 }))
    ));
    assert!(!harness.machine.magic().armed);
    assert!(harness.machine.magic().arm.is_some());

    let mut wrong_root = magic_scene(Some("staff_of_fire"), 35);
    let controls = *wrong_root.data.autocast_controls().unwrap();
    seed_autocast_widget_text(
        &mut wrong_root,
        Some("Fire Bolt"),
        controls.spell_text_component,
        controls.staff_tab_root + 1,
        true,
        true,
        false,
    );
    let mut harness = Harness::new(&wrong_root, request(&wrong_root, None, false));
    let _ = harness.ready(&wrong_root, 1);
    assert_eq!(
        harness.machine.end,
        Some(CombatEnd::Aborted(AbortReason::PrepFailed(PrepItem::Staff)))
    );
    assert!(!harness.machine.magic().armed);
}

#[test]
fn sibling_widget_with_matching_text_cannot_adopt_armed_autocast() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = magic_scene(Some("staff_of_fire"), 35);
    let controls = *scene.data.autocast_controls().unwrap();
    seed_autocast_widget_text(
        &mut scene,
        Some("Fire Bolt"),
        controls.choose_com,
        controls.staff_tab_root,
        true,
        true,
        false,
    );
    let mut harness = Harness::new(&scene, request(&scene, None, false));

    press(harness.pending(&scene, 1), controls.choose_com);
    assert!(!harness.machine.magic().armed);
    assert!(harness.machine.magic().arm.is_some());
}

#[test]
fn autocast_state_is_not_adopted_after_initial_arm_decision() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = magic_scene(Some("staff_of_fire"), 35);
    let controls = *scene.data.autocast_controls().unwrap();
    seed_autocast_widget_text(
        &mut scene,
        Some("Wind Strike"),
        controls.spell_text_component,
        controls.staff_tab_root,
        true,
        true,
        false,
    );
    let mut harness = Harness::new(&scene, request(&scene, None, false));
    press(harness.pending(&scene, 1), controls.choose_com);

    seed_autocast_widget_text(
        &mut scene,
        Some("Fire Bolt"),
        controls.spell_text_component,
        controls.staff_tab_root,
        true,
        true,
        false,
    );
    let _ = harness.pending(&scene, 2);
    assert!(!harness.machine.magic().armed);
    assert!(harness.machine.magic().arm.is_some());
}

#[test]
fn case17_autocast_initial_armed_spell_is_replaced_and_rune_out_rearms_strike() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = magic_scene(Some("staff_of_fire"), 35);
    let mut harness = Harness::new(&scene, request(&scene, None, false));
    arm(&mut harness, &mut scene, 1, "Fire Bolt");
    scene.install();
    scene.refresh();
    refresh_magic(&mut scene, 3, false);
    assert!(harness.pending(&scene, 5).is_none());
    set_count(&mut scene, "chaosrune", 0);
    refresh_magic(&mut scene, 0, false);
    arm(&mut harness, &mut scene, 6, "Fire Strike");
    assert_eq!(harness.machine.magic().mode, Some(CastMode::Autocast));
    assert_eq!(harness.machine.magic().rejected, 0);
}

#[test]
fn mage_fight_enables_only_the_protect_prayer() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = magic_scene(Some("staff_of_fire"), 35);
    scene.stat(5, 43, 43);
    scene.install();
    scene.face_us();
    scene.npcs[0].animation = scene.melee_seq();
    scene.npcs[0].animation_frame = 0;
    scene.refresh();

    let protect = scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == "Protect from Melee")
        .unwrap();
    let mut harness = Harness::new(&scene, request(&scene, Some(&["fire_bolt"]), false));
    let frame = Frame::borrow(SnapshotView::new(
        Some(&scene.snapshot),
        EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        },
    ))
    .unwrap();
    harness.machine.engage(
        ActorRef {
            kind: ActorKind::Npc,
            index: 7,
        },
        &frame,
    );
    harness.machine.phase = Phase::Fight;
    assert_ne!(harness.machine.flags & BOOST_WORTH, 0);

    harness.pending_batch(&scene, 1);
    let mut prayers = harness
        .machine
        .plan
        .iter()
        .filter(|row| row.kind == RowKind::Prayer);
    assert_eq!(prayers.next().map(|row| row.id), Some(protect.button_com));
    assert!(
        prayers.next().is_none(),
        "mage combat enabled a melee offensive prayer"
    );
}

#[test]
fn case24_manual_order_with_armed_staff_never_arms_or_attacks_and_casts_five_ticks_apart() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = magic_scene(Some("staff_of_fire"), 60);
    let order = ["wind_blast", "water_blast", "earth_blast", "fire_blast"];
    let mut harness = Harness::new(&scene, request(&scene, Some(&order), false));
    for (offset, alias) in order.iter().enumerate() {
        let tick = 1 + offset as u64 * 5;
        cast(harness.pending(&scene, tick), component(&scene, alias));
        spend(&mut scene, alias);
        for next in tick + 1..tick + 5 {
            refresh_magic(&mut scene, 3, false);
            assert!(
                harness.pending(&scene, next).is_none(),
                "unexpected upkeep or cast at {next}"
            );
        }
    }
    assert_eq!(harness.machine.casts(), 4);
    assert_eq!(harness.machine.magic().mode, Some(CastMode::Manual));
    assert!(harness.machine.magic().arm.is_none());
}

#[test]
fn case25_manual_specific_rejection_is_permanent_but_not_no_runes() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = magic_scene(Some("ibanstaff"), 60);
    let order = ["ibans_blast", "fire_bolt"];
    let mut harness = Harness::new(&scene, request(&scene, Some(&order), false));
    cast(harness.pending(&scene, 1), component(&scene, "ibans_blast"));
    let iban = magic::spell_index(&scene.tables, "ibans_blast").unwrap();
    harness
        .machine
        .magic_refusal("You have no charges left on the staff.");
    assert_ne!(harness.machine.magic().rejected & (1 << iban), 0);
    for tick in 2..6 {
        refresh_magic(&mut scene, 3, false);
        assert!(harness.pending(&scene, tick).is_none());
    }
    cast(harness.pending(&scene, 6), component(&scene, "fire_bolt"));
    assert_eq!(harness.machine.end, None);
    spend(&mut scene, "fire_bolt");
    for tick in 7..11 {
        refresh_magic(&mut scene, 3, false);
        assert!(harness.pending(&scene, tick).is_none());
    }
    cast(harness.pending(&scene, 11), component(&scene, "fire_bolt"));
}

#[test]
fn case25_crumble_is_only_castable_against_undead_and_specific_refusal_is_not_runes() {
    if !api::test_support::game_data_available() {
        return;
    }
    let scene = magic_scene(None, 41);
    let frame = Frame::borrow(SnapshotView::new(
        Some(&scene.snapshot),
        EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        },
    ))
    .unwrap();
    let crumble = &scene.data.spells()
        [usize::from(magic::spell_index(&scene.tables, "crumble_undead").unwrap())];
    let target = Some(ActorRef {
        kind: ActorKind::Npc,
        index: 7,
    });
    assert!(!magic::castable(crumble, &frame, &scene.tables, target));
    assert_eq!(
        magic::refusal(
            "This spell only affects skeletons, zombies, ghosts and shades.",
            crumble
        ),
        Some(Refusal::Spell)
    );
    assert_eq!(
        magic::refusal(
            "You do not have enough Chaos Runes to cast this spell.",
            crumble
        ),
        Some(Refusal::Runes)
    );
}

#[test]
fn case32_iban_staff_defaults_to_selectable_fire_blast_but_explicit_iban_is_manual() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = magic_scene(Some("ibanstaff"), 60);
    let mut automatic = Harness::new(&scene, request(&scene, None, false));
    arm(&mut automatic, &mut scene, 1, "Fire Blast");
    let mut manual = Harness::new(&scene, request(&scene, Some(&["ibans_blast"]), false));
    for offset in 0..4 {
        let tick = 1 + offset * 5;
        cast(
            manual.pending(&scene, tick),
            component(&scene, "ibans_blast"),
        );
        spend(&mut scene, "ibans_blast");
        for next in tick + 1..tick + 5 {
            refresh_magic(&mut scene, 3, false);
            assert!(manual.pending(&scene, next).is_none());
        }
    }
    assert_eq!(manual.machine.casts(), 4);
}

#[test]
fn manual_order_fallback_flag_is_required_at_rune_exhaustion() {
    if !api::test_support::game_data_available() {
        return;
    }
    for fallback in [false, true] {
        let mut scene = magic_scene(Some("staff_of_fire"), 35);
        set_count(&mut scene, "chaosrune", 1);
        refresh_magic(&mut scene, 3, false);
        let mut harness = Harness::new(&scene, request(&scene, Some(&["fire_bolt"]), fallback));
        cast(harness.pending(&scene, 1), component(&scene, "fire_bolt"));
        spend(&mut scene, "fire_bolt");
        refresh_magic(&mut scene, 3, false);
        if fallback {
            for tick in 2..6 {
                assert!(harness.pending(&scene, tick).is_none());
            }
            cast(harness.pending(&scene, 6), component(&scene, "fire_strike"));
            assert_eq!(harness.machine.magic().mode, Some(CastMode::Manual));
        } else {
            assert_eq!(
                harness.ready(&scene, 2).end,
                CombatEnd::Aborted(AbortReason::Unprotected(Unprotected::NoRunes))
            );
        }
    }
}

#[test]
fn manual_cast_timeout_retries_once_then_reports_unresponsive() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = magic_scene(Some("staff_of_fire"), 35);
    let mut harness = Harness::new(&scene, request(&scene, Some(&["fire_bolt"]), false));
    cast(harness.pending(&scene, 1), component(&scene, "fire_bolt"));

    for tick in 2..11 {
        refresh_magic(&mut scene, 3, false);
        assert!(harness.pending(&scene, tick).is_none());
    }
    assert_eq!(harness.machine.magic().rejected, 0);
    cast(harness.pending(&scene, 11), component(&scene, "fire_bolt"));
    assert_eq!(harness.machine.end, None);

    for tick in 12..21 {
        refresh_magic(&mut scene, 3, false);
        assert!(harness.pending(&scene, tick).is_none());
    }
    assert_eq!(
        harness.ready(&scene, 21).end,
        CombatEnd::Aborted(AbortReason::Unresponsive)
    );
}

#[test]
fn manual_classified_refusal_exhaustion_is_unresponsive_not_arm_failure() {
    if !api::test_support::game_data_available() {
        return;
    }
    let scene = magic_scene(Some("ibanstaff"), 60);
    let mut harness = Harness::new(&scene, request(&scene, Some(&["ibans_blast"]), false));
    cast(harness.pending(&scene, 1), component(&scene, "ibans_blast"));
    harness
        .machine
        .magic_refusal("You have no charges left on the staff.");

    assert_eq!(
        harness.ready(&scene, 2).end,
        CombatEnd::Aborted(AbortReason::Unresponsive)
    );
}

#[test]
fn splash_resource_settle_counts_cast_once_without_damage_or_shorter_cadence() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = magic_scene(Some("staff_of_fire"), 35);
    let mut harness = Harness::new(&scene, request(&scene, Some(&["fire_bolt"]), false));
    cast(harness.pending(&scene, 1), component(&scene, "fire_bolt"));
    spend(&mut scene, "fire_bolt");
    let splash = scene
        .data
        .failed_spell_impact()
        .expect("selected splash fact");
    scene.npcs[0].spot_animation = splash;
    scene.npcs[0].spot_animation_stamp = 30;
    for tick in 2..6 {
        refresh_magic(&mut scene, 3, false);
        assert!(harness.pending(&scene, tick).is_none());
        assert_eq!(harness.machine.casts(), 1);
        assert_eq!(harness.machine.counters.damage, 0);
        assert_eq!(scene.npcs[0].health, 30);
    }
    cast(harness.pending(&scene, 6), component(&scene, "fire_bolt"));
}

#[test]
fn arm_failure_is_prep_failed_and_native_side_tab_admission_is_serial() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = magic_scene(Some("staff_of_fire"), 35);
    scene.snapshot.seed_side_tabs(
        vec![SideTabView {
            index: 0,
            root_component_id: scene.data.autocast_controls().unwrap().staff_tab_root,
            available: true,
            active: false,
            visible: true,
            widgets: Vec::new(),
        }],
        1,
    );
    let mut harness = Harness::new(&scene, request(&scene, None, false));
    let interaction = harness.machine.schedule.interaction;
    assert!(matches!(
        harness.pending(&scene, 1),
        Some(HostEffect::Interaction(InteractReq::SideTab { tab: 0 }))
    ));
    assert_eq!(harness.machine.interaction(), interaction);
    for tick in 2..5 {
        assert!(harness.pending(&scene, tick).is_none());
        assert_eq!(harness.machine.schedule.interaction, interaction);
    }
    assert_eq!(
        harness.ready(&scene, 5).end,
        CombatEnd::Aborted(AbortReason::PrepFailed(PrepItem::Arm))
    );
}

#[test]
fn spell_queue_outlives_visual_and_impact_does_not_relabel_the_new_facing_actor() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = magic_scene(Some("staff_of_fire"), 35);
    let mut mage = PlayerView {
        index: 2,
        network: tile(2605, 3200),
        actor: actor(tile(2605, 3200)),
        combat_level: 60,
        skill_level: 0,
        headicons: 0,
        weapon: Some(scene.data.item_by_alias("staff_of_fire").unwrap().id),
    };
    mage.actor.name = Some("alice".into());
    mage.actor.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: 1,
    });
    scene.players.push(mage);
    scene.refresh();
    let projectile_gfx = scene
        .data
        .style_spotanims()
        .iter()
        .find(|row| row.location == "projectile" && row.style == 4)
        .unwrap()
        .spotanim_id;
    scene
        .snapshot
        .seed_projectiles(vec![api::snapshot::ProjectileView {
            spotanim: projectile_gfx,
            level: 0,
            src: tile(2605, 3200),
            target: Some(ActorTargetView {
                kind: ActorKind::Player,
                index: 1,
            }),
            t1: 351,
            t2: 396,
        }]);
    let mut marks = [HitmarkView {
        value: 0,
        kind: 0,
        cycle: 0,
    }; 4];
    scene.snapshot.seed_hitmarks(HitmarksView {
        marks,
        loop_cycle: 300,
    });
    let evidence = |tick| EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick,
        sequence: tick,
    };
    let mut threats = ThreatSet::default();
    threats.observe(
        &Frame::borrow(SnapshotView::new(Some(&scene.snapshot), evidence(10))).unwrap(),
        &scene.tables,
        10,
    );
    let spell_source = threats.iter(10).find(|row| row.actor.index == 2).unwrap();
    assert_eq!(
        spell_source.projectile_family(),
        super::super::super::threats::ProjectileFamily::PlayerSpell
    );
    assert_eq!(spell_source.due_tick, 14);

    // A later melee onset becomes current before the old spell's visual
    // flight begins. Its future t1 must not outrank that newer evidence.
    scene.players[0].actor.animation = scene.melee_seq();
    scene.players[0].actor.animation_frame = 0;
    scene.players[0].actor.target = None;
    scene.players[0].weapon = Some(scene.data.item_by_alias("rune_scimitar").unwrap().id);
    scene.refresh();
    scene.snapshot.seed_hitmarks(HitmarksView {
        marks,
        loop_cycle: 330,
    });
    threats.observe(
        &Frame::borrow(SnapshotView::new(Some(&scene.snapshot), evidence(11))).unwrap(),
        &scene.tables,
        11,
    );
    assert_eq!(
        threats
            .iter(11)
            .find(|row| row.actor.index == 2)
            .unwrap()
            .style,
        StyleObs::Melee
    );
    scene.face_us();
    scene.local.player.actor.spot_animation = scene
        .data
        .style_spotanims()
        .iter()
        .find(|row| row.location == "on_us" && row.style == 4)
        .unwrap()
        .spotanim_id;
    scene.local.player.actor.spot_animation_stamp = 420;
    scene.refresh();
    // The projectile has visually expired, but its hit queue is still due.
    scene.snapshot.seed_projectiles(Vec::new());
    marks[0] = HitmarkView {
        value: 6,
        kind: 1,
        cycle: 490,
    };
    scene.snapshot.seed_hitmarks(HitmarksView {
        marks,
        loop_cycle: 420,
    });
    let events = threats.observe(
        &Frame::borrow(SnapshotView::new(Some(&scene.snapshot), evidence(14))).unwrap(),
        &scene.tables,
        14,
    );
    assert_eq!(events.damage, 6);
    assert_eq!(
        threats
            .iter(14)
            .find(|row| row.actor.index == 2)
            .unwrap()
            .style,
        StyleObs::Melee
    );
    assert_eq!(
        threats
            .iter(14)
            .find(|row| row.actor.kind == ActorKind::Npc)
            .unwrap()
            .style,
        StyleObs::Melee
    );
}

#[test]
fn mage_boosts_never_select_melee_attack_or_strength_potions() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = magic_scene(Some("staff_of_fire"), 35);
    let first_free = scene.inventory.len();
    for (offset, alias) in ["4dose2attack", "4dose2strength", "4dose1magic"]
        .into_iter()
        .enumerate()
    {
        scene
            .inventory
            .push(scene.held(alias, (first_free + offset) as i32));
    }
    refresh_magic(&mut scene, 3, false);
    let frame = Frame::borrow(SnapshotView::new(
        Some(&scene.snapshot),
        EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        },
    ))
    .unwrap();
    let harness = Harness::new(&scene, request(&scene, None, false));
    assert_eq!(harness.machine.boost(&frame), Some(PotionKind::Magic));
}

#[test]
fn steady_autocast_observation_and_ranking_allocate_nothing_for_two_hundred_ticks() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = magic_scene(Some("staff_of_fire"), 35);
    // Forty Fire Bolts consume 120 air runes; keep the chosen spell stable.
    set_count(&mut scene, "airrune", 150);
    refresh_magic(&mut scene, 3, false);
    let mut harness = Harness::new(&scene, request(&scene, None, false));
    arm(&mut harness, &mut scene, 1, "Fire Bolt");
    scene.install();
    refresh_magic(&mut scene, 3, false);
    assert!(harness.pending(&scene, 5).is_none());
    let mut allocations = 0;
    for tick in 6..206 {
        if (tick - 6) % 5 == 0 {
            spend(&mut scene, "fire_bolt");
        }
        // Host publication is outside the measured native machine poll.
        refresh_magic(&mut scene, 3, false);
        allocations += allocation_counter::measure(|| {
            assert!(matches!(harness.poll(&scene.snapshot, tick), Poll::Pending));
            assert!(harness.runtime.ledger.as_ref().unwrap().outbox.is_empty());
        })
        .count_total;
    }
    assert_eq!(allocations, 0);
    assert_eq!(harness.machine.casts(), 40);
}

#[test]
fn steady_manual_order_selection_allocates_nothing_between_casts() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = magic_scene(Some("staff_of_fire"), 60);
    let order = ["wind_blast", "water_blast"];
    refresh_magic(&mut scene, 3, false);
    let mut harness = Harness::new(&scene, request(&scene, Some(&order), false));
    cast(harness.pending(&scene, 1), component(&scene, order[0]));

    let mut last_cast_tick = 1;
    let mut last_alias = order[0];
    let mut next_spell = 1;
    let mut allocations = 0;
    // Run 100 ticks; measure the 80 non-emitting ticks, including rune
    // settlement and order selection. The 20 emitting ticks are unmeasured.
    for tick in 2..102 {
        if tick == last_cast_tick + 1 {
            spend(&mut scene, last_alias);
        }
        refresh_magic(&mut scene, 3, false);
        if (tick - 1) % 5 == 0 {
            last_alias = order[next_spell];
            cast(harness.pending(&scene, tick), component(&scene, last_alias));
            last_cast_tick = tick;
            next_spell = (next_spell + 1) % order.len();
        } else {
            allocations += allocation_counter::measure(|| {
                assert!(matches!(harness.poll(&scene.snapshot, tick), Poll::Pending));
                assert!(harness.runtime.ledger.as_ref().unwrap().outbox.is_empty());
            })
            .count_total;
        }
    }
    assert_eq!(allocations, 0);
}

fn npc_cast_scene(with_runes: bool) -> Scene {
    let mut scene = Scene::new("imp");
    if with_runes {
        for (slot, alias) in ["airrune", "mindrune"].into_iter().enumerate() {
            let mut rune = scene.held(alias, slot as i32);
            rune.count = 10;
            scene.inventory.push(rune);
        }
    }
    scene.refresh();
    scene
}

fn npc_cast_request(scene: &Scene) -> CastNpcRequest {
    CastNpcRequest {
        component_id: component(scene, "wind_strike"),
        index: 7,
    }
}

fn npc_cast_runtime() -> Runtime {
    Runtime {
        retained: RetainedMemory::default(),
        ledger: None,
        budget: ledger::TickBudget::default(),
        wall: Instant::now(),
        evidence: None,
        reach: None,
        condition: None,
        poison_unknown_pulse: u8::MAX,
        cure_poison: crate::survival::CurePoison::WhenNeeded,
        wilderness: None,
    }
}

fn begin_npc_cast(scene: &Scene) -> (Runtime, ActionHandle<CastNpc>) {
    let mut runtime = npc_cast_runtime();
    let request = npc_cast_request(scene);
    let selected = Arc::clone(&scene.data);
    let handle = runtime.context(&scene.snapshot, 1, 1, |actions, cx| {
        actions.begin::<CastNpc>((request, selected), cx).unwrap()
    });
    (runtime, handle)
}

fn poll_npc_cast(
    runtime: &mut Runtime,
    scene: &Scene,
    handle: &ActionHandle<CastNpc>,
    tick: u64,
) -> Poll<Result<bool, crate::native::ActionError>> {
    runtime.context(&scene.snapshot, tick, tick, |actions, cx| {
        actions.poll(handle, cx)
    })
}

fn settle_npc_cast(runtime: &mut Runtime, accepted: bool) {
    let evidence = runtime.evidence.unwrap();
    let ledger = runtime.ledger.as_mut().unwrap();
    assert_eq!(ledger.outbox.len(), 1);
    let action = ledger.outbox.remove(0);
    let authority = action.authority();
    ledger.complete_interaction(
        &authority,
        crate::native::InteractionReceipt {
            request_id: action.request_id.get(),
            evidence,
            accepted,
            chat_since: -1,
        },
    );
}

fn spend_npc_cast_runes(scene: &mut Scene) {
    let component_id = component(scene, "wind_strike");
    let spell = scene
        .data
        .spells()
        .iter()
        .find(|spell| spell.component_id == component_id)
        .unwrap();
    for rune in &spell.runes {
        scene
            .inventory
            .iter_mut()
            .find(|item| item.def.id == rune.id)
            .unwrap()
            .count -= rune.count;
    }
    scene.refresh();
}

#[test]
fn npc_cast_requires_selected_spell_and_enough_runes_before_dispatch() {
    if !api::test_support::game_data_available() {
        return;
    }
    let scene = npc_cast_scene(false);
    let mut runtime = npc_cast_runtime();
    let request = npc_cast_request(&scene);
    let selected = Arc::clone(&scene.data);
    let result = runtime.context(&scene.snapshot, 1, 1, |actions, cx| {
        actions.begin::<CastNpc>((request, selected), cx)
    });
    assert!(matches!(
        result,
        Err(crate::native::ActionError::Unavailable(_))
    ));
    assert!(
        runtime
            .ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()),
        "unavailable cast queued a host interaction"
    );
}

#[test]
fn npc_cast_dispatch_requires_later_rune_or_exact_target_evidence() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = npc_cast_scene(true);
    let (mut runtime, handle) = begin_npc_cast(&scene);
    assert!(matches!(
        poll_npc_cast(&mut runtime, &scene, &handle, 2),
        Poll::Pending
    ));
    let npc = &scene.npcs[0];
    let expected_component = component(&scene, "wind_strike");
    let expected_type = i32::try_from(npc.r#type.unwrap()).unwrap();
    match &runtime.ledger.as_ref().unwrap().outbox[0].effect {
        HostEffect::Interaction(InteractReq::UseWidgetOn {
            component_id,
            kind,
            target_name,
            x,
            z,
            level,
            index,
            target_id,
        }) => {
            assert_eq!(*component_id, expected_component);
            assert_eq!(kind, "npc");
            assert_eq!(target_name, &npc.name);
            assert_eq!(
                (*x, *z, *level),
                (npc.network.x, npc.network.z, npc.network.level)
            );
            assert_eq!(*index, Some(7));
            assert_eq!(*target_id, Some(expected_type));
        }
        _ => panic!("unexpected NPC cast interaction"),
    }
    settle_npc_cast(&mut runtime, true);
    assert!(
        matches!(
            poll_npc_cast(&mut runtime, &scene, &handle, 3),
            Poll::Pending
        ),
        "dispatch acceptance is not cast completion"
    );

    spend_npc_cast_runes(&mut scene);
    assert_eq!(
        poll_npc_cast(&mut runtime, &scene, &handle, 4),
        Poll::Ready(Ok(true))
    );
}

#[test]
fn npc_cast_reports_consumed_runes_after_target_despawns() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = npc_cast_scene(true);
    let (mut runtime, handle) = begin_npc_cast(&scene);
    assert!(matches!(
        poll_npc_cast(&mut runtime, &scene, &handle, 2),
        Poll::Pending
    ));
    settle_npc_cast(&mut runtime, true);
    assert!(matches!(
        poll_npc_cast(&mut runtime, &scene, &handle, 3),
        Poll::Pending
    ));

    scene.npcs.clear();
    spend_npc_cast_runes(&mut scene);
    assert_eq!(
        poll_npc_cast(&mut runtime, &scene, &handle, 4),
        Poll::Ready(Ok(true))
    );
}

#[test]
fn npc_cast_uses_target_impact_or_splash_and_rejection_before_rune_success() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut impact_scene = npc_cast_scene(true);
    let (mut impact_runtime, impact_handle) = begin_npc_cast(&impact_scene);
    assert!(matches!(
        poll_npc_cast(&mut impact_runtime, &impact_scene, &impact_handle, 2),
        Poll::Pending
    ));
    settle_npc_cast(&mut impact_runtime, true);
    let spell = impact_scene
        .data
        .spells()
        .iter()
        .find(|spell| spell.component_id == component(&impact_scene, "wind_strike"))
        .unwrap();
    assert!(spell.impact_spotanim >= 0);
    impact_scene.npcs[0].spot_animation = spell.impact_spotanim;
    impact_scene.npcs[0].spot_animation_stamp = 1;
    impact_scene.refresh();
    assert_eq!(
        poll_npc_cast(&mut impact_runtime, &impact_scene, &impact_handle, 3),
        Poll::Ready(Ok(true))
    );

    let mut failed_scene = npc_cast_scene(true);
    let (mut failed_runtime, failed_handle) = begin_npc_cast(&failed_scene);
    assert!(matches!(
        poll_npc_cast(&mut failed_runtime, &failed_scene, &failed_handle, 2),
        Poll::Pending
    ));
    settle_npc_cast(&mut failed_runtime, true);
    let failed_impact = failed_scene.data.failed_spell_impact().unwrap();
    failed_scene.npcs[0].spot_animation = failed_impact;
    failed_scene.npcs[0].spot_animation_stamp = 1;
    spend_npc_cast_runes(&mut failed_scene);
    // A splash is a sent cast that missed: upstream `Game.castOnNpc` returns
    // the dispatch result (`Game.ts:177-185`), so it is `true`.
    assert_eq!(
        poll_npc_cast(&mut failed_runtime, &failed_scene, &failed_handle, 3),
        Poll::Ready(Ok(true))
    );

    let mut chat_scene = npc_cast_scene(true);
    let (mut chat_runtime, chat_handle) = begin_npc_cast(&chat_scene);
    assert!(matches!(
        poll_npc_cast(&mut chat_runtime, &chat_scene, &chat_handle, 2),
        Poll::Pending
    ));
    settle_npc_cast(&mut chat_runtime, true);
    spend_npc_cast_runes(&mut chat_scene);
    chat_scene
        .snapshot
        .seed_chat_lines(vec![api::snapshot::ChatLineView {
            type_: 0,
            username: None,
            text: "You do not have enough Air Runes to cast this spell.".into(),
            sequence: 1,
        }]);
    assert_eq!(
        poll_npc_cast(&mut chat_runtime, &chat_scene, &chat_handle, 3),
        Poll::Ready(Ok(false))
    );
}

#[test]
fn npc_cast_times_out_without_content_confirmation() {
    if !api::test_support::game_data_available() {
        return;
    }
    let scene = npc_cast_scene(true);
    let (mut runtime, handle) = begin_npc_cast(&scene);
    assert!(matches!(
        poll_npc_cast(&mut runtime, &scene, &handle, 2),
        Poll::Pending
    ));
    assert_eq!(
        poll_npc_cast(&mut runtime, &scene, &handle, 7),
        Poll::Ready(Ok(false))
    );
}
