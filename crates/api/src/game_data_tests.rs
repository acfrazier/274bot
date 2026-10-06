use super::*;

const RAW_289: &[u8] = include_bytes!("../data/game-data/289.json");

#[test]
fn selected_pin_is_shared_and_unbound_facts_fail_closed() {
    let data = for_revision(ClientRevision::R289).unwrap();
    let first = data.selected_pin().unwrap();
    let second = data.selected_pin().unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let legacy =
        SelectedGameData::decode(minimal_json("").as_bytes(), ClientRevision::R274).unwrap();
    assert!(matches!(
        legacy.selected_pin(),
        Err(FactError::FamilyUnavailable(_))
    ));
    let unbound: SelectedGameData = serde_json::from_slice(RAW_289).unwrap();
    assert!(matches!(
        unbound.selected_pin(),
        Err(FactError::FamilyUnavailable(_))
    ));
}

#[test]
fn selected_pin_refuses_changed_assets_and_mismatched_manifest_identity() {
    let data = for_revision(ClientRevision::R289).unwrap();
    let mut changed = RAW_289.to_vec();
    changed.push(b' ');
    assert_eq!(
        data.bind_pin(
            changed.len() as u64,
            Sha256::digest(&changed).into(),
            MANIFEST,
            ClientRevision::R289,
        ),
        Err(FactError::PinMismatch)
    );
    // Same length but changed bytes must fail too.
    changed = RAW_289.to_vec();
    changed[0] = b' ';
    assert_eq!(
        data.bind_pin(
            changed.len() as u64,
            Sha256::digest(&changed).into(),
            MANIFEST,
            ClientRevision::R289,
        ),
        Err(FactError::PinMismatch)
    );
    for field in ["nav_sha256", "flags_sha256", "content_id", "cache_id"] {
        let mut manifest: serde_json::Value = serde_json::from_slice(MANIFEST).unwrap();
        manifest["revisions"][1]["cache_identity"][field] = "00".repeat(32).into();
        assert_eq!(
            data.bind_pin(
                RAW_289.len() as u64,
                Sha256::digest(RAW_289).into(),
                &serde_json::to_vec(&manifest).unwrap(),
                ClientRevision::R289,
            ),
            Err(FactError::PinMismatch),
            "{field}"
        );
    }
    assert_eq!(
        data.bind_pin(
            RAW_289.len() as u64,
            Sha256::digest(RAW_289).into(),
            MANIFEST,
            ClientRevision::R274,
        ),
        Err(FactError::PinMismatch)
    );
}

#[test]
fn streamed_pin_covers_trailing_whitespace_and_refuses_truncated_input() {
    // Cross the decode buffer boundary with valid JSON whitespace: it must
    // count toward the original-byte pin, even though it adds no facts.
    let bytes = RAW_289
        .iter()
        .copied()
        .chain(std::iter::repeat_n(b' ', 128 * 1024));
    let changed: Vec<u8> = bytes.collect();
    let data = SelectedGameData::decode(changed.as_slice(), ClientRevision::R289).unwrap();
    assert_eq!(data.selected_pin(), Err(FactError::PinMismatch));
    assert!(SelectedGameData::decode(&RAW_289[..RAW_289.len() / 2], ClientRevision::R289).is_err());
}

#[test]
fn family_preparation_runs_off_the_callers_thread() {
    let caller = std::thread::current().id();
    let prepared = crate::selected::FamilyPreparation::run(|worker| {
        let data = for_revision(ClientRevision::R289).unwrap();
        assert!(matches!(
            data.prepare_quests(worker),
            Err(FactError::FamilyUnavailable(_))
        ));
        (std::thread::current().id(), data.selected_pin().unwrap())
    })
    .unwrap()
    .join()
    .unwrap();
    assert_ne!(prepared.0, caller);
    assert!(Arc::ptr_eq(
        &prepared.1,
        &for_revision(ClientRevision::R289)
            .unwrap()
            .selected_pin()
            .unwrap()
    ));
}

#[test]
fn unbound_selected_data_refuses_the_gathering_family() {
    let (cached, refused) = crate::selected::FamilyPreparation::run(|worker| {
        let unbound = SelectedGameData::decode(minimal_json("").as_bytes(), ClientRevision::R274)
            .expect("schema 4 without provenance commits still decodes");
        (
            unbound.try_gathering().is_some(),
            unbound.prepare_gathering(worker).map(|_| ()),
        )
    })
    .unwrap()
    .join()
    .unwrap();
    assert!(!cached, "an unbound pin never hits the cache");
    assert!(
        matches!(refused, Err(FactError::FamilyUnavailable(_))),
        "{refused:?}"
    );
}

fn minimal_json(tail: &str) -> String {
    format!(
        r#"{{
                "schema_version": 4,
                "revision": 274,
                "provenance": {{
                    "cache_identity": {{"cache_id": "x"}},
                    "inputs": [],
                    "content_inputs": [],
                    "decoder_sources": []
                }},
                "items": [],
                "consumption": [],
                "pickpocket": []
                {tail}
            }}"#
    )
}

#[test]
fn named_sites_are_selected_core_rows_with_skill_key_closure() {
    for (revision, counts) in [
        (ClientRevision::R274, [243, 33, 32]),
        (ClientRevision::R289, [246, 33, 32]),
    ] {
        let data = for_revision(revision).unwrap();
        let mut ids = std::collections::HashSet::new();
        let mut labels = std::collections::HashSet::new();
        for (skill, expected) in ["woodcutting", "mining", "fishing"].into_iter().zip(counts) {
            assert_eq!(data.gather_sites_for(skill).count(), expected);
            for row in data.gather_sites_for(skill) {
                assert!(ids.insert(row.id.as_str()));
                assert!(labels.insert((row.skill.as_str(), row.label.as_str())));
                assert!(row.region.min_z < 6400);
                assert!(!row
                    .label
                    .as_bytes()
                    .windows(4)
                    .any(|window| window.iter().all(u8::is_ascii_digit)));
                assert!(!row.keys.is_empty());
                for key in &row.keys {
                    assert!(key.count > 0);
                    let option = data.gather_option(skill, &key.key).unwrap();
                    assert_eq!(option.key, key.key);
                }
            }
        }
        for (skill, id, label) in [
            (
                "fishing",
                "fishing.catherby",
                "Catherby · Harpoon, Net, Bait, Cage",
            ),
            (
                "fishing",
                "fishing.musa_point",
                "Musa Point · Bait, Cage, Harpoon, Net",
            ),
            (
                "fishing",
                "fishing.barbarian_village",
                "Barbarian Village · Bait, Lure",
            ),
            (
                "fishing",
                "fishing.agility_training_area.sw",
                "Agility Training Area SW19 · Bait, Lure",
            ),
            (
                "mining",
                "mining.varrock_east.se",
                "Varrock East SE50 · Copper ore 9, Tin ore 6, Iron ore 4",
            ),
            (
                "woodcutting",
                "woodcutting.draynor",
                "Draynor · Logs 25, Willow logs 5, Oak logs 4",
            ),
        ] {
            assert_eq!(data.gather_site(skill, id).unwrap().label, label);
        }
        let baxtorian = data
            .gather_site(" Fishing ", " FISHING.BAXTORIAN_FALLS ")
            .unwrap();
        assert_eq!(
            (baxtorian.region.min_x, baxtorian.region.min_z),
            (2527, 3403)
        );
        assert_eq!(
            (baxtorian.region.max_x, baxtorian.region.max_z),
            (2537, 3412)
        );
        assert!(data.gather_site("mining", "fishing.catherby").is_none());
        assert!(data
            .gather_site("fishing", "fishing.rimmington.sw")
            .is_none());
        assert!(data
            .gather_site("fishing", "fishing.cooks_guild.w")
            .is_none());
    }
}

#[test]
fn named_site_wire_rejects_unknown_fields_and_legacy_core_defaults_empty() {
    let row = serde_json::json!({
        "id": "mining.fixture", "skill": "mining", "label": "Fixture · Copper ore 1",
        "region": {"min_x": 1, "min_z": 2, "max_x": 1, "max_z": 2, "level": 0},
        "keys": [{"key": "copper", "count": 1}]
    });
    assert!(serde_json::from_value::<GatherSiteOption>(row.clone()).is_ok());
    for changed in [
        {
            let mut changed = row.clone();
            changed["anchor"] = serde_json::json!({"x": 1, "z": 2, "level": 0});
            changed
        },
        {
            let mut changed = row.clone();
            changed["region"]["plane"] = serde_json::json!(0);
            changed
        },
        {
            let mut changed = row;
            changed["keys"][0]["resource"] = serde_json::json!("copper");
            changed
        },
    ] {
        assert!(serde_json::from_value::<GatherSiteOption>(changed).is_err());
    }
    let legacy =
        SelectedGameData::decode(minimal_json("").as_bytes(), ClientRevision::R274).unwrap();
    assert!(legacy.gather_sites().is_empty());
}

#[test]
fn selected_prayer_layout_rejects_noncontiguous_or_oversized_rows() {
    let mut gap: SelectedGameData = serde_json::from_slice(RAW_289).unwrap();
    gap.prayers[0].varp += 1;
    let error = validate_prayer_layout(&gap, ClientRevision::R289).unwrap_err();
    assert!(error.contains("revision 289"));
    assert!(error.contains("row 0"));

    let mut oversized: SelectedGameData = serde_json::from_slice(RAW_289).unwrap();
    let extra_rows = oversized.prayers.clone();
    oversized.prayers.extend(extra_rows);
    let error = validate_prayer_layout(&oversized, ClientRevision::R289).unwrap_err();
    assert!(error.contains("revision 289"));
    assert!(error.contains("supported count"));
}

#[test]
fn combat_fact_rows_preserve_nullable_delays_stages_and_combat_inputs() {
    let food: ConsumptionFact = serde_json::from_str(
        r#"{
            "item":{"alias":"bread","id":1,"name":"Bread"},
            "source_row":"food","source_file":"consume_normal.dbrow",
            "effect":"consume_food","eat_delay_arg":2,"skill_delay_arg":3,
            "message_delay":null,"stat_change":[],
            "stat_heal":[{"stat":"hitpoints","base":4,"percent":0}],
            "heal_energy":[],"qualification":"fixed_hp_heal",
            "next_stage":"stale_bread","dose_family":null,"dose_count":null
        }"#,
    )
    .unwrap();
    assert_eq!(food.eat_delay_arg, Some(2));
    assert_eq!(food.skill_delay_arg, Some(3));
    assert_eq!(food.message_delay, None);
    assert_eq!(food.next_stage.as_deref(), Some("stale_bread"));
    assert_eq!(food.stat_heal[0].base, 4);
    assert_eq!(food.fixed_hp_heal(), Some(4));

    let old_food: ConsumptionFact = serde_json::from_str(
        r#"{
            "item":{"alias":"old_bread","id":2,"name":"Old bread"},
            "source_row":"food","source_file":"consume_normal.dbrow",
            "qualification":"fixed_hp_heal"
        }"#,
    )
    .unwrap();
    assert_eq!(old_food.eat_delay_arg, None);
    assert_eq!(old_food.skill_delay_arg, None);
    assert_eq!(old_food.message_delay, None);
    assert_eq!(old_food.fixed_hp_heal(), None);

    let spell: SpellFact = serde_json::from_str(
        r#"{
            "name":"Fire Strike","source_row":"magic_spell_fire_strike","ssb":3,
            "component_id":1158,"autocast_selectable":true,"level":13,
            "continue_by_autocast":true,"spellcom":"magic:fire_strike",
            "maxhit":8,"members":false,"wornrequired":"staff_of_fire","impact_spotanim":101,"runes":[]
        }"#,
    )
    .unwrap();
    assert_eq!(spell.spellcom, "magic:fire_strike");
    assert_eq!(spell.maxhit, 8);
    assert!(!spell.members);
    assert_eq!(spell.wornrequired.as_deref(), Some("staff_of_fire"));
    assert_eq!(spell.source_row, "magic_spell_fire_strike");
    assert_eq!(spell.component_id, 1158);
    assert!(spell.autocast_selectable);

    let npc: NpcNameRow = serde_json::from_str(
        r#"{
            "id":17,"config":"test_npc","display":"Test NPC","ops":["Attack"],
            "size":1,"wanderrange":0,"maxrange":1,"attackrange":1,"huntrange":1,
            "vislevel":0,"hitpoints":44,"damagetype":null,"strength":24,"ranged":3,
            "strengthbonus":8,"rangebonus":4,"undead":1,"ap_attack":true,
            "attack_kind":"mixed","forced_max_hit":24,"dragonfire":"chromatic",
            "headicon":8,
            "attackrate":6,"bespoke":true
        }"#,
    )
    .unwrap();
    assert_eq!(npc.strength, Some(24));
    assert_eq!(npc.ranged, Some(3));
    assert_eq!(npc.undead, Some(1));
    assert_eq!(npc.forced_max_hit, Some(24));
    assert_eq!(npc.dragonfire, Some(DragonfireKind::Chromatic));
    assert_eq!(npc.attackrate, Some(6));
    assert_eq!(npc.headicon, Some(8));

    let sequence: StyleSequenceFact = serde_json::from_str(r#"{"seq_id":1,"style":3}"#).unwrap();
    let spotanim: StyleSpotanimFact =
        serde_json::from_str(r#"{"spotanim_id":2,"style":8,"where":"attacker"}"#).unwrap();
    let weapon: WeaponStyleFact =
        serde_json::from_str(r#"{"obj_id":3,"style":1,"attackrate":5,"category":0,"tab":9}"#)
            .unwrap();
    let old_weapon: WeaponStyleFact =
        serde_json::from_str(r#"{"obj_id":4,"style":1,"attackrate":4,"category":0}"#).unwrap();
    let unknown_weapon: WeaponStyleFact =
        serde_json::from_str(r#"{"obj_id":5,"style":1,"attackrate":4,"category":0,"tab":null}"#)
            .unwrap();
    let combat_tab: CombatTabFact = serde_json::from_str(r#"{"tab":9,"root_id":903}"#).unwrap();
    assert_eq!((sequence.seq_id, sequence.style), (1, 3));
    assert_eq!((spotanim.spotanim_id, spotanim.style), (2, 8));
    assert_eq!(spotanim.location, "attacker");
    assert_eq!(
        (
            weapon.obj_id,
            weapon.style,
            weapon.attackrate,
            weapon.category
        ),
        (3, 1, 5, 0)
    );
    assert_eq!(weapon.tab, Some(9));
    assert_eq!(old_weapon.tab, None);
    assert_eq!(unknown_weapon.tab, None);
    assert_eq!((combat_tab.tab, combat_tab.root_id), (9, 903));
    let melee_mode: MeleeModeFact =
        serde_json::from_str(r#"{"tab":1,"slot":2,"mode":1,"button":9012}"#).unwrap();
    assert_eq!(
        (
            melee_mode.tab,
            melee_mode.slot,
            melee_mode.mode,
            melee_mode.button
        ),
        (1, 2, 1, 9012),
    );
}

#[test]
fn autocast_grid_admits_chooser_spells_and_refuses_manual_only_spells() {
    let tail = r#", "spells": [
        {"name":"Wind Strike","source_row":"magic_spell_wind_strike","ssb":0,"component_id":1152,"autocast_selectable":true,"level":1,"continue_by_autocast":true,"spellcom":"magic:wind_strike","maxhit":2,"members":false,"runes":[],"impact_spotanim":92},
        {"name":"Crumble undead","source_row":"magic_spell_crumble_undead","ssb":-1,"component_id":1171,"autocast_selectable":false,"level":39,"continue_by_autocast":true,"spellcom":"magic:crumble_undead","maxhit":8,"members":false,"runes":[],"impact_spotanim":147},
        {"name":"Iban blast","source_row":"magic_spell_iban_blast","ssb":-1,"component_id":1539,"autocast_selectable":false,"level":50,"continue_by_autocast":true,"spellcom":"magic:iban_blast","maxhit":25,"members":true,"wornrequired":"ibanstaff","worn_reqmessage":"You must wield Iban's staff to cast this spell.","runes":[],"impact_spotanim":89}
    ], "failed_spell_impact": 85, "autocast": {"staff_tab_root":328,"spell_text_component":352,"spell_panel_root":1829,"choose_com":353,"toggle_com":349,"spell_grid_base":1830,"magic_varp":108,"selected_value":2,"armed_value":3}"#;
    let data =
        SelectedGameData::decode(minimal_json(tail).as_bytes(), ClientRevision::R274).unwrap();
    // Name lookup stays case-insensitive and covers manual-only spells.
    assert_eq!(
        data.spell("wind strike").unwrap().source_row,
        "magic_spell_wind_strike"
    );
    assert_eq!(data.spell("CRUMBLE UNDEAD").unwrap().component_id, 1171);
    // The chooser spell resolves through the staff grid base ...
    assert_eq!(data.spell_button_com("Wind Strike"), 1830);
    // ... while manual-only spells keep their own widget component and are refused.
    let crumble = data.spell("Crumble undead").unwrap();
    assert!(!crumble.autocast_selectable);
    assert_eq!(crumble.ssb, -1);
    assert_eq!(crumble.component_id, 1171);
    assert_eq!(crumble.impact_spotanim, 147);
    assert_eq!(data.spell_button_com("Crumble undead"), -1);
    // Refusal text is exact selected content, not a broad fallback.
    let iban = data.spell("iban blast").unwrap();
    assert_eq!(iban.wornrequired.as_deref(), Some("ibanstaff"));
    assert_eq!(
        iban.worn_reqmessage.as_deref(),
        Some("You must wield Iban's staff to cast this spell.")
    );
    assert_eq!(data.spell_button_com("Iban blast"), -1);
    // The shared miss splash is one selected scalar, excluded from per-spell impacts.
    assert_eq!(data.failed_spell_impact(), Some(85));
    assert!(data
        .spells()
        .iter()
        .all(|spell| spell.impact_spotanim != 85));
    // Unknown names stay refused.
    assert!(data.spell("No Such Spell").is_none());
    assert_eq!(data.spell_button_com("No Such Spell"), -1);
    // A core without the scalar still decodes, with no splash published.
    let legacy =
        SelectedGameData::decode(minimal_json("").as_bytes(), ClientRevision::R274).unwrap();
    assert_eq!(legacy.failed_spell_impact(), None);
}

#[test]
fn same_name_food_facts_keep_per_item_heals_and_legacy_name_ambiguity() {
    let json = minimal_json("").replace(
        r#""consumption": []"#,
        r#""consumption": [
            {
                "item":{"alias":"cooked_karambwan","id":3144,"name":"Cooked karambwan"},
                "source_row":"cooked_karambwan","source_file":"consume_normal.dbrow",
                "effect":"consume_food","eat_delay_arg":null,"skill_delay_arg":null,
                "message_delay":2,"stat_change":[],
                "stat_heal":[{"stat":"hitpoints","base":18,"percent":0}],
                "heal_energy":[],"qualification":"fixed_hp_heal",
                "next_stage":null,"dose_family":null,"dose_count":null
            },
            {
                "item":{"alias":"harmful_karambwan","id":3142,"name":"Cooked karambwan"},
                "source_row":"harmful_karambwan","source_file":"consume_effects.dbrow",
                "effect":"consume_food","eat_delay_arg":null,"skill_delay_arg":null,
                "message_delay":2,"stat_change":[{"stat":"hitpoints","base":-5,"percent":0}],
                "stat_heal":[],"heal_energy":[],"qualification":"not_fixed_hp_heal",
                "next_stage":null,"dose_family":null,"dose_count":null
            }
        ]"#,
    );
    let data = SelectedGameData::decode(json.as_bytes(), ClientRevision::R274)
        .expect("same-name consumption facts decode");

    let cooked = data
        .consumption_facts()
        .iter()
        .find(|fact| fact.item.id == 3144)
        .expect("cooked karambwan fact");
    let harmful = data
        .consumption_facts()
        .iter()
        .find(|fact| fact.item.id == 3142)
        .expect("harmful same-name fact");
    assert_eq!(cooked.fixed_hp_heal(), Some(18));
    assert_eq!(harmful.fixed_hp_heal(), None);
    assert_eq!(data.fixed_food_heal("Cooked karambwan"), None);
}

#[test]
fn legacy_selected_data_has_no_invented_combat_controls() {
    let data = SelectedGameData::decode(minimal_json("").as_bytes(), ClientRevision::R274).unwrap();
    assert!(data.melee_modes().is_empty());
    assert_eq!(data.melee_mode_varp(), None);
    assert!(data.ranged_weapons().is_empty());
    assert!(data.ranged_ammo().is_empty());
    assert!(data.ranged_modes().is_empty());
    assert_eq!(data.ranged_mode_varp(), None);
}

#[test]
fn ranged_fact_payload_decodes_and_is_exposed_by_selected_data() {
    let json = minimal_json(
        r#", "ranged_weapons":[{"obj_id":841,"attackrange":10,"levelrequire":30,"ammo_family":"ogre_arrow"}],
        "ranged_ammo":[{"obj_id":2866,"levelrequire":30,"family":"ogre_arrow"}],
        "ranged_modes":[{"tab":12,"slot":1,"mode":1,"button":20032}],
        "ranged_mode_varp":43"#,
    );
    let data = SelectedGameData::decode(json.as_bytes(), ClientRevision::R274).unwrap();
    assert_eq!(
        data.ranged_weapons()[0].ammo_family,
        RangedAmmoFamily::OgreArrow
    );
    assert_eq!(data.ranged_weapons()[0].attackrange, 10);
    assert_eq!(data.ranged_ammo()[0].family, RangedAmmoFamily::OgreArrow);
    assert_eq!(data.ranged_modes()[0].mode, 1);
    assert_eq!(data.ranged_mode_varp(), Some(43));
}

fn debug_catalog() -> crate::debug_commands::DebugCatalog {
    crate::debug_commands::DebugCatalog::decode(
        include_bytes!("../data/game-data/289/debug.json"),
        ClientRevision::R289,
        for_revision(ClientRevision::R289).unwrap(),
    )
    .unwrap()
}

#[test]
fn debug_picker_exact_alias_precedes_substrings() {
    let data = debug_catalog();
    assert_eq!(data.search_names("npc", "man", 40)[0].alias, "man");
    assert_eq!(data.search_names("inv", "inv", 40)[0].alias, "inv");
}

#[test]
fn debug_catalog_commands_and_name_families_are_searchable() {
    let data = debug_catalog();
    assert!(data
        .commands()
        .iter()
        .any(|command| command.name == "~maxme"));
    assert_eq!(data.search_names("obj", "coins", 8)[0].alias, "coins");
    assert_eq!(data.search_names("namedobj", "995", 8)[0].id, 995);
    assert_eq!(
        data.search_names("varbit", "tutorial", 8)[0].alias,
        "tutorial"
    );
    assert!(data.search_names("npc", "", 8).is_empty());
    assert!(data.search_names("future", "guard", 8).is_empty());
    assert!(data.search_names("npc", "man", 0).is_empty());
}

fn scanned_fixed_food_heal(data: &SelectedGameData, name: &str) -> Option<i32> {
    let mut matching = data
        .consumption
        .iter()
        .filter(|fact| fact.item.name.eq_ignore_ascii_case(name));
    let heal = matching.next()?.fixed_hp_heal()?;
    matching
        .all(|fact| fact.fixed_hp_heal() == Some(heal))
        .then_some(heal)
}

#[test]
fn selected_lookup_indexes_match_straight_scans() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = for_revision(revision).expect("selected game data");
        for item in data.items() {
            let indexed = data.item_by_id(item.id).expect("indexed selected item");
            assert_eq!(indexed.id, item.id);
            assert_eq!(indexed.alias, item.alias);
        }
        assert!(data.item_by_id(-1).is_none());
        assert!(data
            .item_by_id(data.items().iter().map(|item| item.id).max().unwrap_or(0) + 1)
            .is_none());

        for fact in &data.consumption {
            let name = fact.item.name.as_str();
            let expected = scanned_fixed_food_heal(&data, name);
            assert_eq!(
                data.fixed_food_heal(name),
                expected,
                "{name} on {revision:?}"
            );
            let upper = name.to_ascii_uppercase();
            let lower = name.to_ascii_lowercase();
            assert_eq!(
                data.fixed_food_heal(&upper),
                expected,
                "upper {upper} on {revision:?}"
            );
            assert_eq!(
                data.fixed_food_heal(&lower),
                expected,
                "lower {lower} on {revision:?}"
            );
        }
        assert_eq!(data.fixed_food_heal("not a selected food"), None);
    }
}

#[test]
fn missing_quest_identity_is_absent_not_an_empty_list() {
    let data = SelectedGameData::decode(minimal_json("").as_bytes(), ClientRevision::R274)
        .expect("schema 4 without the field still decodes");
    assert!(data.quest_identity().is_none());
}

#[test]
fn generated_quest_starts_are_pinned_to_selected_content() {
    let data = for_revision(ClientRevision::R289).expect("selected game data");
    let starts = data.quest_starts().expect("generated quest start family");
    assert_eq!(starts.schema, 1);
    assert_eq!(starts.revision, 289);
    assert_eq!(Some(starts.content_id.as_str()), data.content_id());
    assert!(
        !starts.rows.is_empty(),
        "the pinned content has quest starts"
    );
    assert!(
        starts
            .rows
            .iter()
            .all(|row| !row.quest.is_empty()
                && (row.target.kind == "npc" || row.target.kind == "loc"))
    );
}

#[test]
fn quest_starts_with_mismatched_content_are_rejected() {
    let json = minimal_json(
        r#", "quest_starts": {"schema": 1, "revision": 274, "content_id": "not-the-cache", "rows": [{"quest": "cook", "target": {"kind": "npc", "id": 100}, "op": 1}]}"#,
    )
    .replace(r#""cache_id": "x""#, r#""cache_id": "x", "content_id": "the-cache""#);
    let error = SelectedGameData::decode(json.as_bytes(), ClientRevision::R274)
        .expect_err("quest starts must be pinned to the selected content");
    assert!(error.contains("content identity mismatch"), "{error}");
}

#[test]
fn bare_quest_identity_vec_does_not_decode() {
    let error = SelectedGameData::decode(
        minimal_json(r#", "quest_identity": []"#).as_bytes(),
        ClientRevision::R274,
    )
    .expect_err("a bare vec must not decode as an empty family");
    assert!(
        error.contains("quest_identity") || error.contains("decode"),
        "unexpected error: {error}"
    );
}

#[test]
fn coverage_only_quest_identity_is_not_success() {
    let error = SelectedGameData::decode(
            minimal_json(
                r#", "quest_identity": {"rows": [], "coverage": [{"class": "revision-absent", "alias": "routequest", "on_revision": 274, "other_pin_id": 387, "copied": false, "reason": "not copied"}]}"#,
            )
            .as_bytes(),
            ClientRevision::R274,
        )
        .expect_err("coverage-only is not success");
    assert!(
        error.contains("no identity rows"),
        "unexpected error: {error}"
    );
}

#[test]
fn quest_complete_range_does_not_decode() {
    let error = SelectedGameData::decode(
            minimal_json(
                r#", "quest_identity": {"rows": [{"id": "death", "component": "death", "display": "Death Plateau", "varp": "death_equiproom", "varp_id": 314, "complete": {"min": 80}, "quest_points": 1, "unknown_sides": [], "requirements": {"qualification": "partial", "skills": [], "items": [], "empty_must_have": true, "unknown_as_satisfied": false}}], "coverage": []}"#,
            )
            .as_bytes(),
            ClientRevision::R274,
        )
        .expect_err("a complete range must not decode");
    assert!(error.contains("decode"), "unexpected error: {error}");
}

#[test]
fn promoted_quest_requirements_do_not_decode() {
    let error = SelectedGameData::decode(
            minimal_json(
                r#", "quest_identity": {"rows": [{"id": "runemysteries", "component": "runemysteries", "display": "Rune Mysteries Quest", "varp": "runemysteries", "varp_id": 63, "complete": 6, "quest_points": 1, "unknown_sides": [], "requirements": {"qualification": "complete", "skills": [], "items": [], "empty_must_have": true, "unknown_as_satisfied": false}}], "coverage": []}"#,
            )
            .as_bytes(),
            ClientRevision::R274,
        )
        .expect_err("a successful join must not promote requirements");
    assert!(error.contains("partial"), "unexpected error: {error}");
}

#[test]
fn unknown_as_satisfied_quest_requirements_do_not_decode() {
    let error = SelectedGameData::decode(
            minimal_json(
                r#", "quest_identity": {"rows": [{"id": "murder", "component": "murder", "display": "Murder Mystery", "varp": "murderquest", "varp_id": 192, "complete": 2, "quest_points": 3, "unknown_sides": [], "requirements": {"qualification": "partial", "skills": [], "items": [], "empty_must_have": true, "unknown_as_satisfied": true}}], "coverage": []}"#,
            )
            .as_bytes(),
            ClientRevision::R274,
        )
        .expect_err("empty mustHave is not unknown-satisfied");
    assert!(
        error.contains("unknown-satisfied"),
        "unexpected error: {error}"
    );
}

#[test]
fn partial_quest_identity_row_decodes() {
    let data = SelectedGameData::decode(
            minimal_json(
                r#", "quest_identity": {"rows": [{"id": "cook", "component": "cook", "display": "Cook's Assistant", "varp": "cookquest", "varp_id": 29, "complete": 2, "quest_points": 1, "unknown_sides": [], "requirements": {"qualification": "partial", "skills": [], "items": [{"alias": "egg", "quantity": 1, "kind": "inv"}], "empty_must_have": false, "unknown_as_satisfied": false}}], "coverage": []}"#,
            )
            .as_bytes(),
            ClientRevision::R274,
        )
        .expect("partial requirements still decode");
    let facts = data.quest_identity().expect("present family");
    assert_eq!(facts.rows.len(), 1);
    assert_eq!(facts.rows[0].varp_id, 29);
    assert_eq!(facts.rows[0].complete, 2);
    assert_eq!(facts.rows[0].requirements.qualification, "partial");
    assert!(!facts.rows[0].requirements.unknown_as_satisfied);
    assert!(facts.coverage.is_empty());
}

#[test]
fn missing_trails_is_absent_not_an_empty_list() {
    let data = SelectedGameData::decode(minimal_json("").as_bytes(), ClientRevision::R274)
        .expect("schema 4 without the field still decodes");
    assert!(data.trails().is_none());
}

#[test]
fn bare_trails_vec_does_not_decode() {
    let error = SelectedGameData::decode(
        minimal_json(r#", "trails": []"#).as_bytes(),
        ClientRevision::R274,
    )
    .expect_err("a bare vec must not decode as an empty family");
    assert!(
        error.contains("trails") || error.contains("decode"),
        "unexpected error: {error}"
    );
}

#[test]
fn trails_without_challenge_answers_do_not_decode() {
    let error = SelectedGameData::decode(
            minimal_json(
                r#", "trails": {"rows": [{"alias": "trail_clue_easy_simple001", "id": 2677, "role": "clue", "params": [{"key": "trail_loc", "value": "^true"}]}]}"#,
            )
            .as_bytes(),
            ClientRevision::R274,
        )
        .expect_err("an omitted challenge_answers key is not an empty list");
    assert!(
        error.contains("challenge_answers"),
        "unexpected error: {error}"
    );
}

#[test]
fn empty_challenge_answers_are_not_success() {
    let error = SelectedGameData::decode(
            minimal_json(
                r#", "trails": {"rows": [{"alias": "trail_clue_easy_simple001", "id": 2677, "role": "clue", "params": []}], "challenge_answers": []}"#,
            )
            .as_bytes(),
            ClientRevision::R274,
        )
        .expect_err("Some with no selected answer is not success");
    assert!(
        error.contains("challenge answers"),
        "unexpected error: {error}"
    );
}

#[test]
fn empty_trails_rows_are_not_success() {
    let error = SelectedGameData::decode(
            minimal_json(
                r#", "trails": {"rows": [], "challenge_answers": [{"alias": "trail_clue_medium_anagram001_challenge", "id": 2842, "answer": "6859"}]}"#,
            )
            .as_bytes(),
            ClientRevision::R274,
        )
        .expect_err("a present family with no membership rows is not success");
    assert!(
        error.contains("no membership rows"),
        "unexpected error: {error}"
    );
}

#[test]
fn trail_row_without_params_does_not_decode() {
    let error = SelectedGameData::decode(
            minimal_json(
                r#", "trails": {"rows": [{"alias": "trail_clue_easy_simple001", "id": 2677, "role": "clue"}], "challenge_answers": [{"alias": "trail_clue_medium_anagram001_challenge", "id": 2842, "answer": "6859"}]}"#,
            )
            .as_bytes(),
            ClientRevision::R274,
        )
        .expect_err("params is part of the row shape, not a defaulted list");
    assert!(error.contains("params"), "unexpected error: {error}");
}

#[test]
fn guardian_trail_role_does_not_decode() {
    let error = SelectedGameData::decode(
            minimal_json(
                r#", "trails": {"rows": [{"alias": "trail_clue_hard_sextant017", "id": 3532, "role": "trail_hard2", "params": []}], "challenge_answers": [{"alias": "trail_clue_medium_anagram001_challenge", "id": 2842, "answer": "6859"}]}"#,
            )
            .as_bytes(),
            ClientRevision::R274,
        )
        .expect_err("a guardian param is not a role and membership is not support");
    assert!(error.contains("role"), "unexpected error: {error}");
}

#[test]
fn open_trail_access_does_not_decode() {
    let error = SelectedGameData::decode(
            minimal_json(
                r#", "trails": {"rows": [{"alias": "trail_clue_easy_simple001", "id": 2677, "role": "clue", "params": [], "access": "open"}], "challenge_answers": [{"alias": "trail_clue_medium_anagram001_challenge", "id": 2842, "answer": "6859"}]}"#,
            )
            .as_bytes(),
            ClientRevision::R274,
        )
        .expect_err("membership is not support and is not open access");
    assert!(error.contains("constrained"), "unexpected error: {error}");
}

#[test]
fn numeric_challenge_answer_does_not_decode() {
    let error = SelectedGameData::decode(
            minimal_json(
                r#", "trails": {"rows": [{"alias": "trail_clue_easy_simple001", "id": 2677, "role": "clue", "params": []}], "challenge_answers": [{"alias": "trail_clue_medium_anagram002_challenge", "id": 2844, "answer": 9}]}"#,
            )
            .as_bytes(),
            ClientRevision::R274,
        )
        .expect_err("a coerced answer is not a raw param string");
    assert!(error.contains("decode"), "unexpected error: {error}");
}

#[test]
fn partial_trail_facts_decode() {
    let data = SelectedGameData::decode(
            minimal_json(
                r#", "trails": {"rows": [{"alias": "trail_clue_easy_simple001", "id": 2677, "role": "clue", "params": [{"key": "trail_loc", "value": "^true"}, {"key": "trail_sextant", "value": "yes"}]}, {"alias": "trail_clue_hard_sextant016_casket", "id": 3531, "role": "casket", "params": []}, {"alias": "trail_clue_hard_sextant028", "id": 3554, "role": "clue", "params": [], "access": "constrained"}], "challenge_answers": [{"alias": "trail_clue_medium_anagram001_challenge", "id": 2842, "answer": "6859"}]}"#,
            )
            .as_bytes(),
            ClientRevision::R274,
        )
        .expect("partial trail facts still decode");
    let facts = data.trails().expect("present family");
    assert_eq!(facts.rows.len(), 3);
    assert_eq!(facts.rows[0].role, "clue");
    assert_eq!(facts.rows[0].params[0].key, "trail_loc");
    assert_eq!(facts.rows[0].params[0].value, "^true");
    assert_eq!(facts.rows[0].params[1].value, "yes");
    assert_eq!(facts.rows[1].params.len(), 0);
    assert_eq!(facts.rows[2].access.as_deref(), Some("constrained"));
    assert_eq!(facts.challenge_answers[0].id, 2842);
    assert_eq!(facts.challenge_answers[0].answer, "6859");
}

const TALK_KEY_TALK_STEP: &str = r#"{"alias": "trail_clue_medium_anagram001", "id": 2841, "npc": {"alias": "grandtree_hazelmere", "id": 669, "name": "Hazelmere"}, "spawn": {"x": 2678, "z": 3086, "plane": 1}}"#;
const TALK_KEY_TYPE_KEEPER: &str = r#"{"alias": "trail_clue_medium_riddle001", "id": 2831, "key_alias": "trail_clue_medium_riddle001_key", "key_id": 2832, "keeper": {"kind": "type", "alias": "black_heather", "id": 202, "name": "Black Heather"}, "spawn": {"x": 3039, "z": 3700, "plane": 0}}"#;
const TALK_KEY_COVERAGE_ROW: &str = r#"[{"class": "unknown", "family": "keys", "alias": "trail_clue_medium_riddle004", "reason": "keeper is a category, not one packed npc id"}]"#;

fn talk_key_tail(talk: &str, keys: &str, coverage: Option<&str>) -> String {
    let coverage = coverage
        .map(|rows| format!(r#", "coverage": {rows}"#))
        .unwrap_or_default();
    format!(r#", "talk_key": {{"talk": {talk}, "keys": {keys}{coverage}}}"#)
}

#[test]
fn missing_talk_key_is_absent_not_an_empty_list() {
    let data = SelectedGameData::decode(minimal_json("").as_bytes(), ClientRevision::R274)
        .expect("schema 4 without the field still decodes");
    assert!(data.talk_key().is_none());
}

#[test]
fn bare_talk_key_vec_does_not_decode() {
    let error = SelectedGameData::decode(
        minimal_json(r#", "talk_key": []"#).as_bytes(),
        ClientRevision::R274,
    )
    .expect_err("a bare vec must not decode as an empty family");
    assert!(
        error.contains("talk_key") || error.contains("decode"),
        "unexpected error: {error}"
    );
}

#[test]
fn talk_key_without_talk_steps_does_not_decode() {
    let tail = talk_key_tail(
        "[]",
        &format!("[{TALK_KEY_TYPE_KEEPER}]"),
        Some(TALK_KEY_COVERAGE_ROW),
    );
    let error = SelectedGameData::decode(minimal_json(&tail).as_bytes(), ClientRevision::R274)
        .expect_err("a present family with no talk steps is not success");
    assert!(error.contains("no talk steps"), "unexpected error: {error}");
}

#[test]
fn talk_key_without_key_keepers_does_not_decode() {
    let tail = talk_key_tail(
        &format!("[{TALK_KEY_TALK_STEP}]"),
        "[]",
        Some(TALK_KEY_COVERAGE_ROW),
    );
    let error = SelectedGameData::decode(minimal_json(&tail).as_bytes(), ClientRevision::R274)
        .expect_err("a present family with no key keepers is not success");
    assert!(
        error.contains("no key keepers"),
        "unexpected error: {error}"
    );
}

#[test]
fn talk_key_without_coverage_does_not_decode() {
    let tail = talk_key_tail(
        &format!("[{TALK_KEY_TALK_STEP}]"),
        &format!("[{TALK_KEY_TYPE_KEEPER}]"),
        None,
    );
    let error = SelectedGameData::decode(minimal_json(&tail).as_bytes(), ClientRevision::R274)
        .expect_err("a present family must record what it does not publish");
    assert!(
        error.contains("coverage") || error.contains("decode"),
        "unexpected error: {error}"
    );
}

#[test]
fn empty_talk_key_coverage_does_not_decode() {
    let tail = talk_key_tail(
        &format!("[{TALK_KEY_TALK_STEP}]"),
        &format!("[{TALK_KEY_TYPE_KEEPER}]"),
        Some("[]"),
    );
    let error = SelectedGameData::decode(minimal_json(&tail).as_bytes(), ClientRevision::R274)
        .expect_err("an empty coverage list must not decode as an unknown");
    assert!(error.contains("no coverage"), "unexpected error: {error}");
}

#[test]
fn null_talk_key_spawn_does_not_decode() {
    let step = TALK_KEY_TALK_STEP.replace(
        r#", "spawn": {"x": 2678, "z": 3086, "plane": 1}"#,
        r#", "spawn": null"#,
    );
    let tail = talk_key_tail(
        &format!("[{step}]"),
        &format!("[{TALK_KEY_TYPE_KEEPER}]"),
        Some(TALK_KEY_COVERAGE_ROW),
    );
    let error = SelectedGameData::decode(minimal_json(&tail).as_bytes(), ClientRevision::R274)
        .expect_err("a null spawn is neither a tile nor an omission");
    assert!(
        error.contains("must be omitted, not null"),
        "unexpected error: {error}"
    );
}

#[test]
fn omitted_talk_key_spawn_decodes_as_unknown() {
    let step = TALK_KEY_TALK_STEP.replace(r#", "spawn": {"x": 2678, "z": 3086, "plane": 1}"#, "");
    let tail = talk_key_tail(
        &format!("[{step}]"),
        &format!("[{TALK_KEY_TYPE_KEEPER}]"),
        Some(TALK_KEY_COVERAGE_ROW),
    );
    let data = SelectedGameData::decode(minimal_json(&tail).as_bytes(), ClientRevision::R274)
        .expect("a step without a unique spawn keeps its row");
    let facts = data.talk_key().expect("present family");
    assert_eq!(facts.talk.len(), 1);
    assert_eq!(facts.talk[0].npc.id, 669);
    assert!(facts.talk[0].spawn.is_none());
    assert_eq!(
        facts.keys[0].spawn.as_ref().map(|spawn| spawn.plane),
        Some(0)
    );
}

#[test]
fn talk_key_keeper_union_is_exact() {
    for (keeper, expected) in [
        (
            r#"{"kind": "category", "category": "chicken", "id": 3379}"#,
            "bare category",
        ),
        (
            r#"{"kind": "name", "name": "Man", "alias": "man"}"#,
            "bare name",
        ),
        (
            r#"{"kind": "type", "alias": "black_heather", "name": "Black Heather"}"#,
            "packed npc id",
        ),
        (
            r#"{"kind": "type", "alias": "black_heather", "id": 202, "category": "chicken"}"#,
            "packed npc id",
        ),
        (r#"{"kind": "coordinate", "x": 1}"#, "not a keeper"),
    ] {
        let key = format!(
            r#"{{"alias": "trail_clue_medium_riddle004", "id": 2837, "key_alias": "trail_clue_medium_riddle004_key", "key_id": 2838, "keeper": {keeper}}}"#
        );
        let tail = talk_key_tail(
            &format!("[{TALK_KEY_TALK_STEP}]"),
            &format!("[{key}]"),
            Some(TALK_KEY_COVERAGE_ROW),
        );
        let error = SelectedGameData::decode(minimal_json(&tail).as_bytes(), ClientRevision::R274)
            .expect_err("a keeper must be one exact union member");
        assert!(
            error.contains(expected),
            "keeper {keeper}: unexpected error: {error}"
        );
    }
}

#[test]
fn talk_key_with_type_keeper_decodes_with_spawn() {
    let tail = talk_key_tail(
        &format!("[{TALK_KEY_TALK_STEP}]"),
        &format!("[{TALK_KEY_TYPE_KEEPER}]"),
        Some(TALK_KEY_COVERAGE_ROW),
    );
    let data = SelectedGameData::decode(minimal_json(&tail).as_bytes(), ClientRevision::R274)
        .expect("a type keeper and a unique spawn decode");
    let facts = data.talk_key().expect("present family");
    assert_eq!(
        facts.talk[0]
            .spawn
            .as_ref()
            .map(|spawn| (spawn.x, spawn.z, spawn.plane)),
        Some((2678, 3086, 1))
    );
    assert_eq!(facts.keys[0].keeper.kind, "type");
    assert_eq!(facts.keys[0].keeper.alias.as_deref(), Some("black_heather"));
    assert_eq!(facts.keys[0].keeper.id, Some(202));
    assert_eq!(facts.keys[0].keeper.name.as_deref(), Some("Black Heather"));
    assert_eq!(facts.keys[0].key_alias, "trail_clue_medium_riddle001_key");
    assert_eq!(facts.coverage[0].family, "keys");
}

const TRIO_GIVER_SPAWNED: &str = r#"{"alias": "observatory_professor", "id": 488, "name": "Observatory professor", "spawn": {"x": 2438, "z": 3186, "plane": 0}}"#;
const TRIO_GIVER_UNSPAWNED: &str = r#"{"alias": "murphy", "id": 463, "name": "Murphy"}"#;
const TRIO_GIVER_COVERAGE_ROW: &str = r#"[{"class": "unknown", "family": "trio_givers", "alias": "murphy", "reason": "non-unique jm2 NPC spawn"}]"#;

fn trio_givers_tail(rows: &str, coverage: Option<&str>) -> String {
    let coverage = coverage
        .map(|rows| format!(r#", "coverage": {rows}"#))
        .unwrap_or_default();
    format!(r#", "trio_givers": {{"rows": {rows}{coverage}}}"#)
}

#[test]
fn missing_trio_givers_is_absent_not_an_empty_list() {
    let data = SelectedGameData::decode(minimal_json("").as_bytes(), ClientRevision::R274)
        .expect("schema 4 without the field still decodes");
    assert!(data.trio_givers().is_none());
}

#[test]
fn bare_trio_givers_vec_does_not_decode() {
    let error = SelectedGameData::decode(
        minimal_json(r#", "trio_givers": []"#).as_bytes(),
        ClientRevision::R274,
    )
    .expect_err("a bare vec must not decode as an empty family");
    assert!(
        error.contains("trio_givers") || error.contains("decode"),
        "unexpected error: {error}"
    );
}

#[test]
fn empty_trio_giver_rows_are_not_success() {
    let tail = trio_givers_tail("[]", Some("[]"));
    let error = SelectedGameData::decode(minimal_json(&tail).as_bytes(), ClientRevision::R274)
        .expect_err("a present family with no givers is not success");
    assert!(error.contains("no givers"), "unexpected error: {error}");
}

#[test]
fn coverage_less_trio_givers_does_not_decode() {
    let tail = trio_givers_tail(
        &format!("[{TRIO_GIVER_SPAWNED}, {TRIO_GIVER_UNSPAWNED}]"),
        None,
    );
    let error = SelectedGameData::decode(minimal_json(&tail).as_bytes(), ClientRevision::R274)
        .expect_err("an omitted coverage key is not an empty list");
    assert!(
        error.contains("coverage") || error.contains("decode"),
        "unexpected error: {error}"
    );
}

#[test]
fn null_trio_giver_spawn_does_not_decode() {
    let row = TRIO_GIVER_SPAWNED.replace(
        r#", "spawn": {"x": 2438, "z": 3186, "plane": 0}"#,
        r#", "spawn": null"#,
    );
    let tail = trio_givers_tail(&format!("[{row}]"), Some("[]"));
    let error = SelectedGameData::decode(minimal_json(&tail).as_bytes(), ClientRevision::R274)
        .expect_err("a null spawn is neither a tile nor an omission");
    assert!(
        error.contains("must be omitted, not null"),
        "unexpected error: {error}"
    );
}

#[test]
fn level_trio_giver_spawn_does_not_decode() {
    let row = TRIO_GIVER_SPAWNED.replace(r#""plane": 0"#, r#""level": 0"#);
    let tail = trio_givers_tail(&format!("[{row}]"), Some("[]"));
    let error = SelectedGameData::decode(minimal_json(&tail).as_bytes(), ClientRevision::R274)
        .expect_err("a scene level is not a plane");
    assert!(error.contains("plane"), "unexpected error: {error}");
}

#[test]
fn trio_giver_coverage_must_match_the_unspawned_rows() {
    // A spawned row recorded as unknown is not honest coverage.
    let spawned = trio_givers_tail(
        &format!("[{TRIO_GIVER_SPAWNED}, {TRIO_GIVER_UNSPAWNED}]"),
        Some(
            r#"[{"class": "unknown", "family": "trio_givers", "alias": "observatory_professor", "reason": "non-unique jm2 NPC spawn"}]"#,
        ),
    );
    let error = SelectedGameData::decode(minimal_json(&spawned).as_bytes(), ClientRevision::R274)
        .expect_err("coverage must name an unspawned giver");
    assert!(
        error.contains("exactly the givers without a unique spawn"),
        "unexpected error: {error}"
    );
    // An unspawned row left out of coverage is the same refusal.
    let uncovered = trio_givers_tail(
        &format!("[{TRIO_GIVER_SPAWNED}, {TRIO_GIVER_UNSPAWNED}]"),
        Some("[]"),
    );
    let error = SelectedGameData::decode(minimal_json(&uncovered).as_bytes(), ClientRevision::R274)
        .expect_err("an unspawned giver must be recorded");
    assert!(
        error.contains("exactly the givers without a unique spawn"),
        "unexpected error: {error}"
    );
}

#[test]
fn trio_giver_rows_decode_with_and_without_a_unique_spawn() {
    let tail = trio_givers_tail(
        &format!("[{TRIO_GIVER_SPAWNED}, {TRIO_GIVER_UNSPAWNED}]"),
        Some(TRIO_GIVER_COVERAGE_ROW),
    );
    let data = SelectedGameData::decode(minimal_json(&tail).as_bytes(), ClientRevision::R274)
        .expect("a unique spawn and an unknown spawn decode together");
    let facts = data.trio_givers().expect("present family");
    assert_eq!(facts.rows.len(), 2);
    assert_eq!(facts.rows[0].alias, "observatory_professor");
    assert_eq!(facts.rows[0].id, 488);
    assert_eq!(facts.rows[0].name, "Observatory professor");
    assert_eq!(
        facts.rows[0]
            .spawn
            .as_ref()
            .map(|spawn| (spawn.x, spawn.z, spawn.plane)),
        Some((2438, 3186, 0))
    );
    assert!(facts.rows[1].spawn.is_none());
    assert_eq!(facts.coverage[0].family, "trio_givers");
    assert_eq!(facts.coverage[0].alias, "murphy");
}

#[test]
fn unsupported_trio_giver_coverage_does_not_decode() {
    let tail = trio_givers_tail(
        &format!("[{TRIO_GIVER_SPAWNED}, {TRIO_GIVER_UNSPAWNED}]"),
        Some(
            r#"[{"class": "supported", "family": "trio_givers", "alias": "murphy", "reason": ""}]"#,
        ),
    );
    let error = SelectedGameData::decode(minimal_json(&tail).as_bytes(), ClientRevision::R274)
        .expect_err("a support class is not unknown-spawn coverage");
    assert!(error.contains("unknown"), "unexpected error: {error}");
}

#[test]
fn gather_resources_decode_and_skill_lookup() {
    let tail = r#", "gather_resources": [
        {"skill": "mining", "method": "mining.copper", "methods": ["mining.copper"], "aliases": [], "key": "copper", "resources": ["copper"], "label": "Copper ore", "level": 1, "selectable": true, "gap": null},
        {"skill": "woodcutting", "method": "woodcutting.jungle", "methods": ["woodcutting.jungle"], "aliases": [], "key": "jungle", "resources": ["jungle"], "label": "Jungle", "selectable": false, "gap": "no-resource-target"},
        {"skill": "fishing", "method": "fishing.freshfish.op1", "methods": ["fishing.freshfish.op1", "fishing.loc_2027.op1"], "aliases": ["fishing.freshfish.op1", "fishing.loc_2027.op1"], "key": "fishing.lure.tool_309.products_331_335", "resources": ["raw_salmon", "raw_trout"], "label": "Raw trout / Raw salmon — Lure (Fly fishing rod + Feather)", "level": 20, "selectable": true, "gap": null}
    ]"#;
    let data = SelectedGameData::decode(minimal_json(tail).as_bytes(), ClientRevision::R274)
        .expect("gather slice decodes");
    assert_eq!(data.gather_resources().len(), 3);
    let legacy = SelectedGameData::decode(minimal_json("").as_bytes(), ClientRevision::R274)
        .expect("a core without the slice still decodes");
    assert!(legacy.gather_resources().is_empty());
    let mining: Vec<_> = data.gather_resources_for("Mining").collect();
    assert_eq!(mining.len(), 1);
    assert_eq!(mining[0].key, "copper");
    assert_eq!(
        data.gather_option("mining", " COPPER ")
            .map(|row| row.method.as_str()),
        Some("mining.copper")
    );
    let lure = data
        .gather_option("fishing", "fishing.loc_2027.op1")
        .expect("legacy method id resolves to its group");
    assert_eq!(lure.key, "fishing.lure.tool_309.products_331_335");
    assert_eq!(
        lure.methods,
        ["fishing.freshfish.op1", "fishing.loc_2027.op1"]
    );
    assert_eq!(
        data.gather_option("fishing", "fishing.lure.tool_309.products_331_335")
            .map(|row| row.label.as_str()),
        Some("Raw trout / Raw salmon — Lure (Fly fishing rod + Feather)")
    );
    assert!(data.gather_option("mining", "coal").is_none());
    let jungle = data
        .gather_option("woodcutting", "jungle")
        .expect("a refused row stays published");
    assert!(!jungle.selectable);
    assert_eq!(jungle.gap.as_deref(), Some("no-resource-target"));
    assert_eq!(
        jungle.level, 0,
        "an omitted level defaults, it is never invented"
    );
}

#[test]
fn generated_karamja_facts_join_selected_items_npcs_and_locs() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = for_revision(revision).expect("selected game data");
        let facts = data.karamja().expect("generated Karamja facts");
        assert!(facts.crate_capacity > 0);
        assert!(facts.coin_payout > 0);
        assert!(!facts.banana_tree_configs.is_empty());
        assert!(!facts.banana_tree_spawns.is_empty());
        assert!(facts
            .banana_tree_spawns
            .iter()
            .all(|spawn| facts.banana_tree_configs.contains(&spawn.config)));
        assert!(data.item_by_alias("coins").is_some());
        assert!(data.item_by_alias("banana").is_some());

        let luthas = data
            .npc_by_config(&facts.luthas_spawn.config)
            .expect("Luthas joins the selected NPC pack");
        assert!(luthas
            .display
            .as_deref()
            .is_some_and(|display| !display.is_empty()));
        assert!(luthas
            .ops
            .iter()
            .any(|op| op.eq_ignore_ascii_case("Talk-to")));
        for config in facts
            .banana_tree_configs
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(facts.crate_spawn.config.as_str()))
        {
            assert!(data
                .loc_by_config(config)
                .is_some_and(|loc| !loc.ops.is_empty()));
        }
        assert!(!facts.dialogue.employment.is_empty());
        assert!(!facts.dialogue.paid.is_empty());
        assert!(!facts.dialogue.incomplete.is_empty());
    }
}

#[test]
fn generated_fishing_groups_have_members_aliases_and_unique_labels() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = for_revision(revision).unwrap();
        let groups = data.gather_resources_for("fishing").collect::<Vec<_>>();
        assert_eq!(groups.len(), 13, "{revision:?} fishing group count");
        let labels = groups
            .iter()
            .map(|row| row.label.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(labels.len(), groups.len(), "{revision:?} labels are unique");
        for group in &groups {
            assert!(!group.methods.is_empty(), "{group:?}");
            assert_eq!(group.aliases, group.methods, "{group:?}");
            assert!(group.key.starts_with("fishing."), "{group:?}");
            assert!(group.key.contains(".tool_"), "{group:?}");
            assert!(group.key.contains(".products_"), "{group:?}");
        }
        let freshfish = data
            .gather_option("fishing", "fishing.freshfish.op1")
            .expect("freshfish operation resolves to its group");
        let loc = data
            .gather_option("fishing", "fishing.loc_2027.op1")
            .expect("location operation resolves to its group");
        assert_eq!(freshfish.key, loc.key, "{revision:?} aliases share a key");
        assert_eq!(
            freshfish.methods.len(),
            2,
            "{revision:?} shared method group"
        );
    }
}

#[test]
fn dialogue_ui_controls_are_optional_and_refuse_invalid_identities() {
    let missing =
        SelectedGameData::decode(minimal_json("").as_bytes(), ClientRevision::R274).unwrap();
    assert!(missing.dialogue_ui().is_none());
    let valid = serde_json::json!({
        "scroll_root": 1136,
        "book_root": 837,
        "book_forward": 841,
        "book_close": 10162,
        "book_forward_marker": 842
    });
    let decode = |ids: &serde_json::Value| {
        SelectedGameData::decode(
            minimal_json(&format!(", \"dialogue_ui\": {ids}")).as_bytes(),
            ClientRevision::R274,
        )
    };
    let data = decode(&valid).unwrap();
    assert_eq!(data.dialogue_ui().unwrap().book_forward, 841);
    assert_eq!(data.dialogue_ui().unwrap().book_forward_marker, 842);
    for field in [
        "scroll_root",
        "book_root",
        "book_forward",
        "book_close",
        "book_forward_marker",
    ] {
        let mut invalid = valid.clone();
        invalid[field] = serde_json::json!(0);
        assert!(decode(&invalid).unwrap().dialogue_ui().is_none(), "{field}");
    }
    let mut duplicate = valid.clone();
    duplicate["book_forward"] = duplicate["book_close"].clone();
    assert!(decode(&duplicate).unwrap().dialogue_ui().is_none());
    let mut unknown = valid;
    unknown["debug_catalog"] = serde_json::json!(true);
    assert!(decode(&unknown).is_err());
}

#[test]
fn generated_dialogue_ui_roles_match_both_pinned_sources() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = for_revision(revision).unwrap();
        assert_eq!(
            data.dialogue_ui(),
            Some(&DialogueUiIds {
                scroll_root: 1136,
                book_root: 837,
                book_forward: 841,
                book_close: 10162,
                book_forward_marker: 842,
            }),
            "{revision:?} forward is the handler, not the visibility marker"
        );
    }
}

#[test]
fn operator_item_names_use_the_native_alias_then_case_insensitive_name_rule() {
    let data = for_revision(crate::selected::ClientRevision::R289).unwrap();
    let item = data
        .items()
        .iter()
        .find(|item| item.alias.is_some() && item.name.is_some())
        .unwrap();
    let alias = item.alias.as_deref().unwrap();
    let display = item.name.as_deref().unwrap();

    assert!(std::ptr::eq(data.resolve_item_name(alias).unwrap(), item));
    assert_eq!(
        data.resolve_item_name(&display.to_ascii_uppercase())
            .map(|resolved| resolved.id),
        Some(item.id)
    );
    assert!(data.resolve_item_name(&format!(" {display} ")).is_none());
    assert!(data.resolve_item_name("").is_none());
}

#[test]
fn duplicate_display_names_preserve_selected_order_and_exact_aliases_win() {
    fn item(alias: &str, id: i32, name: &str) -> GameItem {
        GameItem {
            alias: Some(alias.to_string()),
            id,
            name: Some(name.to_string()),
            cost: 0,
            stackable: false,
            members: false,
            certificate_link: 0,
            certificate_template: 0,
            wear_position: 0,
            wear_position_2: 0,
            wear_position_3: 0,
            tradeable: true,
            stack_variant: false,
        }
    }
    let mut data =
        SelectedGameData::decode(minimal_json("").as_bytes(), ClientRevision::R274).unwrap();
    std::sync::Arc::get_mut(&mut data)
        .expect("freshly decoded data is uniquely owned")
        .items = vec![
        item("high", 20, "Duplicate"),
        item("low", 10, "Duplicate"),
        item("Collision", 30, "Alias winner"),
        item("other", 1, "Collision"),
    ];

    assert_eq!(data.resolve_item_name("DUPLICATE").unwrap().id, 20);
    assert_eq!(data.resolve_item_name("Collision").unwrap().id, 30);
    assert_eq!(data.resolve_item_name("collision").unwrap().id, 1);
}
