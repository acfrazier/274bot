//! Picker identity through the real library and profile writer: two files
//! that show one name stay two cards; their paths, settings bags and
//! assignments follow the card identity through edits, rename and refresh;
//! strict manifest values follow the shared dynamic-value rules.

use serde_json::json;

use super::{bag, fixture, Fixture};
use crate::scripts::{CardSource, SchemaRef, ScriptChoice, Scripts};

const COUNT_SETTINGS: &str = "{ count: { type: 'number', default: 5, min: 2, max: 64 } }";

/// A manifest-form source named `name` whose Start does nothing.
fn manifest_source(name: &str, settings: &str, body: &str) -> String {
    script_js::NativeFixture::new(name)
        .settings(settings)
        .source(body)
}

/// Write `source` as `<fixture dir>/<folder>/miner.ts` and load it.
fn load_in(f: &mut Fixture, folder: &str, source: &str) -> script_js::JsCard {
    let dir = f.dir.join(folder);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("miner.ts");
    std::fs::write(&path, source).unwrap();
    f.scripts.js_mut().load(&path).unwrap()
}

fn merged(f: &mut Fixture, card: &script_js::JsCard) -> serde_json::Map<String, serde_json::Value> {
    f.scripts
        .merged_profile_bag(
            &mut f.core,
            "alice",
            CardSource::from_script(card.source),
            &card.name,
            &card.path,
            SchemaRef::new(&card.settings_schema),
        )
        .unwrap()
}

#[test]
fn two_files_with_one_display_name_keep_paths_bags_and_assignment_through_rename_and_refresh() {
    let mut f = fixture("picker-identity-same-name", &["alice"]);
    let a = load_in(
        &mut f,
        "one",
        &manifest_source("Miner", COUNT_SETTINGS, "export function tick(api) {}"),
    );
    let b = load_in(
        &mut f,
        "two",
        &manifest_source(
            "Miner",
            COUNT_SETTINGS,
            "export function tick(api) { api.log('b'); }",
        ),
    );
    assert_eq!(a.name, b.name, "one stem");
    assert_ne!(a.identity_id(), b.identity_id());
    assert_ne!(a.identity_key(), b.identity_key());

    // The shared picker model lists both with distinct provenance, and marks
    // the shared display name so a front end shows it.
    let rows: Vec<_> = f
        .scripts
        .browse_cards_marked()
        .filter(|(card, _)| card.display_name() == "Miner")
        .collect();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|(_, shared)| *shared), "both are marked");
    let ids: Vec<String> = rows.iter().map(|(card, _)| card.identity_id()).collect();
    assert_eq!(ids, [a.identity_id(), b.identity_id()]);
    let paths: Vec<_> = rows.iter().map(|(card, _)| card.path().unwrap()).collect();
    assert_eq!(paths, [a.path.as_path(), b.path.as_path()]);
    for (card, _) in &rows {
        let provenance = card.provenance();
        assert!(provenance.starts_with("File: "), "{provenance}");
        assert!(
            provenance.ends_with(&card.path().unwrap().display().to_string()),
            "{provenance}"
        );
        // Selected-card details: version, author, API family and declared
        // capabilities, with no permission row.
        let details = card.details();
        assert_eq!(details.value("Source"), Some("File"));
        assert_eq!(
            details.value("Path"),
            Some(card.path().unwrap().to_str().unwrap())
        );
        assert_eq!(details.value("Version"), Some("test"));
        assert_eq!(details.value("Author"), Some("test"));
        assert_eq!(details.value("API"), Some("v2 (native)"));
        assert_eq!(details.value("Declares"), Some("no host capabilities"));
    }
    // A name nobody else shows is not marked.
    assert!(f
        .scripts
        .browse_cards_marked()
        .filter(|(card, _)| card.display_name() != "Miner")
        .all(|(_, shared)| !shared));

    // Each card has its own profile bag, keyed by its identity. A number
    // within the range is kept exactly as typed.
    f.set("alice", &a, "count", json!(7));
    f.set("alice", &b, "count", json!(12.5));
    f.core.flush_writes();
    assert_eq!(
        f.saved_bag("alice", &a.identity_key()),
        Some(bag(&[("count", json!(7))]))
    );
    assert_eq!(
        f.saved_bag("alice", &b.identity_key()),
        Some(bag(&[("count", json!(12.5))]))
    );
    let refused = f
        .scripts
        .set_profile_setting(
            &mut f.core,
            "alice",
            CardSource::File,
            &b.name,
            &b.path,
            "count",
            json!(65),
        )
        .expect_err("a value above the maximum is refused, not clipped");
    assert!(refused.contains("outside its declared range"), "{refused}");
    assert_eq!(merged(&mut f, &b).get("count"), Some(&json!(12.5)));

    // The assignment names the second file's path.
    f.assign("alice", &b);
    let assignment = Scripts::assignment(&f.core, "alice").unwrap();
    assert_eq!(assignment.identity, b.identity_id());
    assert_eq!(assignment.source_kind, "file");

    // Rename the second card in its manifest and refresh it by identity;
    // change the first and refresh it by identity too. Neither refresh may
    // find the other card by its stem.
    std::fs::write(
        &b.path,
        manifest_source(
            "Miner (renamed)",
            COUNT_SETTINGS,
            "export function tick(api) { api.log('renamed'); }",
        ),
    )
    .unwrap();
    f.scripts
        .js_mut()
        .refresh(script::ScriptSource::File, &b.identity_id())
        .expect("refresh by the file identity");
    std::fs::write(
        &a.path,
        manifest_source(
            "Miner",
            COUNT_SETTINGS,
            "export function tick(api) { api.log('a2'); }",
        ),
    )
    .unwrap();
    f.scripts
        .js_mut()
        .refresh(script::ScriptSource::File, &a.identity_id())
        .expect("refresh of the other file by its identity");

    let renamed = f
        .scripts
        .card(CardSource::File, &b.identity_id())
        .expect("the renamed card is still found by its identity");
    assert_eq!(renamed.display_name(), "Miner (renamed)");
    assert_eq!(renamed.identity_id(), b.identity_id());
    assert_eq!(renamed.identity_key(), b.identity_key());
    assert!(renamed.is_transpiled());
    let first = f.scripts.card(CardSource::File, &a.identity_id()).unwrap();
    assert_eq!(first.display_name(), "Miner");
    assert_eq!(
        first.legacy_lookup_name(),
        renamed.legacy_lookup_name(),
        "the legacy lookup name stays the stem and is not the identity"
    );
    assert_ne!(first.legacy_lookup_name(), first.identity_id());
    let renamed_origin = f
        .scripts
        .js()
        .get(script::ScriptSource::File, &b.identity_id())
        .unwrap()
        .origin
        .clone();
    assert!(renamed_origin.contains("renamed"));
    let first_origin = f
        .scripts
        .js()
        .get(script::ScriptSource::File, &a.identity_id())
        .unwrap()
        .origin
        .clone();
    assert!(first_origin.contains("a2") && !first_origin.contains("renamed"));

    // Bags and the assignment are untouched.
    assert_eq!(merged(&mut f, &a).get("count"), Some(&json!(7)));
    assert_eq!(merged(&mut f, &b).get("count"), Some(&json!(12.5)));
    assert_eq!(
        Scripts::assignment(&f.core, "alice").unwrap().identity,
        b.identity_id()
    );
    assert_eq!(
        f.saved_bag("alice", &b.identity_key()),
        Some(bag(&[("count", json!(12.5))]))
    );
    // Selection by the identity token resolves to the card, not the stem's
    // first match.
    let choice = ScriptChoice::loaded(CardSource::File, &b.identity_id());
    assert_eq!(
        choice.loaded_card(),
        Some((CardSource::File, b.identity_id().as_str()))
    );
    assert_eq!(
        f.scripts
            .card(CardSource::File, &b.identity_id())
            .map(|card| card.display_name()),
        Some("Miner (renamed)")
    );
}

#[test]
fn browse_filter_searches_name_description_category_and_tags() {
    let mut f = fixture("picker-identity-filter", &["alice"]);
    let source = "export const manifest = { apiVersion: 2, name: 'Rune Miner', version: '1.2', \
        author: 'Alice', description: 'Digs ore at the guild', category: 'Skilling', \
        tags: ['mining', 'ore-run'], settings: {}, capabilities: [] };\n\
        export function tick(api) {}\n";
    let card = load_in(&mut f, "one", source);
    let matching = |query: &str| {
        f.scripts
            .browse_cards_matching(query)
            .any(|row| row.identity_id() == card.identity_id())
    };
    assert!(matching(""), "an empty filter lists every card");
    assert!(matching("rune"), "name");
    assert!(matching("RUNE miner"), "every term, any case");
    assert!(matching("guild"), "description");
    assert!(matching("skilling"), "category");
    assert!(matching("ore-run"), "tag");
    assert!(
        matching("mining guild"),
        "terms may come from different fields"
    );
    assert!(!matching("fishing"));
    assert!(!matching("rune fishing"), "every term must match");
}

#[test]
fn strict_dynamic_values_follow_the_shared_start_and_edit_rules() {
    let mut f = fixture("picker-identity-dynamic", &["alice"]);
    let bank = api::named_banks::bank_catalog()
        .ok()
        .and_then(|banks| banks.first())
        .map(|bank| bank.name.to_owned());
    let Some(bank) = bank else {
        return;
    };
    let settings = "{ \
        count: { type: 'number', default: 12.5, min: 2, max: 64 }, \
        banks: { type: 'string[]', default: [], optionsFrom: 'named-banks' }, \
        other: { type: 'string', default: '', optionsFrom: 'loadouts' }, \
        bank: { type: 'string', default: '', optionsFrom: 'named-banks' }, \
        loadout: { type: 'string', default: '', optionsFrom: 'loadouts' } }";
    let card = load_in(
        &mut f,
        "one",
        &manifest_source("Dynamic", settings, "export function tick(api) {}"),
    );
    let selection = ScriptChoice::loaded(CardSource::File, &card.identity_id());
    let overrides = |pairs: &[(&str, serde_json::Value)]| bag(pairs);

    // A blank named-banks value is invalid, an empty multiselection is legal,
    // a blank loadouts value that is not `loadout` stays blank: the first
    // failure is the blank bank, keyed by its setting.
    match f
        .scripts
        .validate_manifest_profile_settings(&f.core, &card, &overrides(&[]))
    {
        Err(script::StartLoadError::SettingValue(error)) => {
            assert_eq!(error.setting, "bank");
            assert_eq!(
                error.kind,
                script::SavedSettingValueErrorKind::InvalidOption
            );
        }
        other => panic!("a blank bank is refused before Start, got {other:?}"),
    }
    // The effective bag is checked field by field for the pickers.
    let effective = merged(&mut f, &card);
    assert_eq!(effective.get("bank"), Some(&json!("")));
    assert_eq!(
        effective.get("loadout"),
        Some(&json!("")),
        "the sentinel stays blank"
    );
    let issue = |field: &str| {
        f.scripts
            .manifest_value_issue(&f.core, &selection, &effective, field)
    };
    assert!(issue("bank").is_some_and(|text| text.contains("setting `bank`")));
    assert_eq!(issue("banks"), None, "an empty multiselection is legal");
    assert_eq!(
        issue("other"),
        None,
        "another loadouts field may stay blank"
    );
    assert_eq!(issue("count"), None, "12.5 inside 2..=64 is valid as typed");
    assert_eq!(
        issue("loadout"),
        None,
        "blank is the default-loadout sentinel"
    );

    // With a bank chosen, Start goes on to the sentinel: no stored loadout
    // exists, so it is refused as NoLoadouts rather than replaced by a name.
    let with_bank = overrides(&[("bank", json!(bank))]);
    match f
        .scripts
        .validate_manifest_profile_settings(&f.core, &card, &with_bank)
    {
        Err(script::StartLoadError::NoLoadouts(text)) => {
            assert!(text.contains("loadout"), "{text}");
        }
        other => panic!("expected NoLoadouts, got {other:?}"),
    }
    // A number outside the range is refused by key, never clipped.
    let high = overrides(&[("bank", json!(bank)), ("count", json!(64.5))]);
    match f
        .scripts
        .validate_manifest_profile_settings(&f.core, &card, &high)
    {
        Err(script::StartLoadError::SettingValue(error)) => {
            assert_eq!(error.setting, "count");
            assert_eq!(error.kind, script::SavedSettingValueErrorKind::OutsideRange);
        }
        other => panic!("64.5 is above the maximum, got {other:?}"),
    }
}

#[test]
fn numbers_are_free_text_kept_as_typed_and_blank_choices_read_as_their_meaning() {
    use crate::scripts::{parse_parameter_text, resolve_parameter_options, Loadouts};
    use crate::test_support::scripts::TestSchema;

    // A strict number carries only its bounds: no step grid, no rounding.
    let mut count = TestSchema::setting("count", "number", Some("5"), Some("Ore count"), &[]);
    count.set_min("2").set_max("64");
    let def = count.schema_ref().field(0).unwrap();
    assert_eq!(def.bounds(), (Some("2"), Some("64")));
    assert_eq!(parse_parameter_text(def, "12.5", &[]), Ok(json!(12.5)));
    assert_eq!(parse_parameter_text(def, " 2 ", &[]), Ok(json!(2)));
    assert_eq!(parse_parameter_text(def, "64", &[]), Ok(json!(64)));
    for refused in ["1.99", "64.5", "65", "-3", "abc", "NaN", "inf", ""] {
        assert!(
            parse_parameter_text(def, refused, &[]).is_err(),
            "{refused:?} is refused, never clipped to the range"
        );
    }

    // A blank `loadout` is the default-loadout sentinel; it is shown as such
    // and is never mapped to the first stored name. A blank value of another
    // loadouts or named-banks field shows as blank; a literal option label
    // still wins.
    let loadouts = Loadouts::detached(vec![api::loadout::Loadout::new("fish")]);
    let mut schema = TestSchema::setting("loadout", "string", Some(""), Some("Loadout"), &[]);
    schema.set_options_from("loadouts");
    schema.add_setting("other", "string", Some(""), Some("Other"), &[]);
    schema.set_options_from("loadouts");
    schema.add_setting("bank", "string", Some(""), Some("Bank"), &[]);
    schema.set_options_from("named-banks");
    schema.add_setting("note", "string", Some(""), Some("Note"), &[]);
    let bag = serde_json::Map::new();
    let loadout = schema.schema_ref().field(0).unwrap();
    let options = resolve_parameter_options(loadout, &bag, &loadouts, None).unwrap();
    assert_eq!(
        options.scalar_label(loadout, ""),
        "(default: first loadout)"
    );
    assert_eq!(options.scalar_label(loadout, "fish"), "fish");
    assert_eq!(
        options.value_for(""),
        "",
        "the sentinel is not replaced by a name"
    );
    assert_ne!(options.scalar_label(loadout, ""), "None");
    let other = schema.schema_ref().field(1).unwrap();
    assert_eq!(other.blank_label(), Some("(blank)"));
    assert_eq!(
        schema.schema_ref().field(2).unwrap().blank_label(),
        Some("(blank)")
    );
    assert_eq!(schema.schema_ref().field(3).unwrap().blank_label(), None);
}

#[test]
fn the_all_kinds_manifest_projects_into_the_shared_form_model_and_persists_every_kind() {
    let mut f = fixture("picker-identity-all-kinds", &["alice"]);
    let card = load_in(
        &mut f,
        "one",
        &crate::test_support::scripts::all_kinds_native_fixture(),
    );
    let schema = SchemaRef::new(&card.settings_schema);
    let ids: Vec<&str> = schema.iter().map(|def| def.id()).collect();
    assert_eq!(
        ids,
        ["enabled", "count", "ore", "veins", "spot", "runeNote", "loadout"],
        "definition order is kept"
    );
    let field = |id: &str| schema.get(id).unwrap_or_else(|| panic!("field {id}"));
    for (id, ty) in [
        ("enabled", "boolean"),
        ("count", "number"),
        ("ore", "string"),
        ("veins", "string[]"),
        ("spot", "tile"),
        ("loadout", "string"),
    ] {
        assert_eq!(field(id).ty(), ty, "{id}");
    }
    // Bounds: a number carries its range and nothing like a step.
    assert_eq!(field("count").bounds(), (Some("2"), Some("64")));
    assert_eq!(field("count").default(), Some("12.5"));
    // Options and their labels, parallel and by value.
    assert_eq!(field("ore").options(), ["copper", "tin", "rune"]);
    assert_eq!(
        field("ore").option_labels(),
        ["Copper ore", "Tin ore", "Runite ore"]
    );
    assert_eq!(field("veins").options(), ["copper", "tin", "rune"]);
    assert_eq!(
        field("veins").option_labels(),
        ["Copper vein", "Tin vein", "Rune vein"]
    );
    // Grouping, labels and help.
    let groups: Vec<Option<&str>> = schema.iter().map(|def| def.group()).collect();
    assert_eq!(
        groups,
        [
            Some("General"),
            Some("Mining"),
            Some("Mining"),
            Some("Mining"),
            Some("Mining"),
            Some("Mining"),
            Some("Supplies")
        ]
    );
    assert_eq!(field("enabled").help(), Some("Turns the script on"));
    assert_eq!(field("count").help(), Some("How many ores per trip"));
    assert_eq!(field("count").display_label(), "Ore count");
    // The dynamic choice comes from its source, not from the manifest.
    assert_eq!(field("loadout").options_from(), Some("loadouts"));
    assert!(field("loadout").options().is_empty());
    assert_eq!(
        field("loadout").blank_label(),
        Some("(default: first loadout)")
    );

    // showIf follows the saved value of the key it names.
    let defaults = merged(&mut f, &card);
    assert!(!field("runeNote").visible(&defaults), "{defaults:?}");
    assert!(field("count").visible(&defaults));

    // Every kind persists under the card's identity key and reads back.
    for (id, value) in [
        ("enabled", json!(false)),
        ("count", json!(12.5)),
        ("ore", json!("rune")),
        ("veins", json!(["copper", "rune"])),
        ("spot", json!({ "x": 3201, "z": 3202, "level": 1 })),
        ("loadout", json!("")),
    ] {
        f.set("alice", &card, id, value);
    }
    f.core.flush_writes();
    let expected = bag(&[
        ("enabled", json!(false)),
        ("count", json!(12.5)),
        ("ore", json!("rune")),
        ("veins", json!(["copper", "rune"])),
        ("spot", json!({ "x": 3201, "z": 3202, "level": 1 })),
        ("loadout", json!("")),
    ]);
    assert_eq!(f.saved_bag("alice", &card.identity_key()), Some(expected));
    let after = merged(&mut f, &card);
    assert_eq!(
        after.get("runeNote"),
        Some(&json!("careful")),
        "defaults fill missing keys"
    );
    assert!(field("runeNote").visible(&after), "ore is rune now");
    assert_eq!(
        after.get("loadout"),
        Some(&json!("")),
        "the sentinel is kept"
    );
    // A value outside an option list is refused, never replaced.
    let refused = f
        .scripts
        .set_profile_setting(
            &mut f.core,
            "alice",
            CardSource::File,
            &card.name,
            &card.path,
            "ore",
            json!("mithril"),
        )
        .expect_err("not an option");
    assert!(refused.contains("setting `ore`"), "{refused}");
}
