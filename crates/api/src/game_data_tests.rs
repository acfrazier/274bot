use super::*;

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
fn missing_gather_methods_is_absent_not_an_empty_list() {
    let data = SelectedGameData::decode(minimal_json("").as_bytes(), ClientRevision::R274)
        .expect("schema 4 without the field still decodes");
    assert!(data.gather_methods().is_none());
}

#[test]
fn bare_gather_methods_vec_does_not_decode() {
    let error = SelectedGameData::decode(
        minimal_json(r#", "gather_methods": []"#).as_bytes(),
        ClientRevision::R274,
    )
    .expect_err("a bare vec must not decode as an empty family");
    assert!(
        error.contains("gather_methods") || error.contains("decode"),
        "unexpected error: {error}"
    );
}

#[test]
fn empty_gather_methods_object_is_not_success() {
    let error = SelectedGameData::decode(
        minimal_json(r#", "gather_methods": {"woods": [], "mining": [], "fishing": []}"#)
            .as_bytes(),
        ClientRevision::R274,
    )
    .expect_err("Some with no extracted rows is not success");
    assert!(
        error.contains("no extracted rows"),
        "unexpected error: {error}"
    );
}

#[test]
fn published_placements_decode() {
    let data = SelectedGameData::decode(
            minimal_json(
                r#", "gather_placements": {"rows": [{"loc_id": 1306, "x": 2735, "z": 3582, "plane": 0}], "coverage": [{"class": "unknown", "family": "mining", "reason": "no selected published-ore set"}]}"#,
            )
            .as_bytes(),
            ClientRevision::R274,
        )
        .expect("published placements decode");
    let facts = data
        .gather_placements()
        .expect("a present family is not absence");
    assert_eq!(facts.rows.len(), 1);
    assert_eq!(facts.rows[0].loc_id, 1306);
    assert_eq!(
        (facts.rows[0].x, facts.rows[0].z, facts.rows[0].plane),
        (2735, 3582, 0)
    );
    assert_eq!(facts.coverage[0].class, "unknown");
    assert_eq!(facts.coverage[0].family, "mining");
}

#[test]
fn missing_gather_placements_is_absent_not_an_empty_list() {
    let data = SelectedGameData::decode(minimal_json("").as_bytes(), ClientRevision::R274)
        .expect("schema 4 without the field still decodes");
    assert!(data.gather_placements().is_none());
}

#[test]
fn bare_gather_placements_vec_does_not_decode() {
    let error = SelectedGameData::decode(
        minimal_json(r#", "gather_placements": []"#).as_bytes(),
        ClientRevision::R274,
    )
    .expect_err("a bare vec must not decode as an empty family");
    assert!(
        error.contains("gather_placements") || error.contains("decode"),
        "unexpected error: {error}"
    );
}

#[test]
fn empty_gather_placements_object_is_not_success() {
    let error = SelectedGameData::decode(
            minimal_json(
                r#", "gather_placements": {"rows": [], "coverage": [{"class": "unknown", "family": "mining", "reason": "no selected published-ore set"}]}"#,
            )
            .as_bytes(),
            ClientRevision::R274,
        )
        .expect_err("Some with no world rows is not success");
    assert!(error.contains("no world rows"), "unexpected error: {error}");
}

#[test]
fn coverage_less_gather_placements_does_not_decode() {
    let error = SelectedGameData::decode(
            minimal_json(
                r#", "gather_placements": {"rows": [{"loc_id": 1306, "x": 2735, "z": 3582, "plane": 0}], "coverage": []}"#,
            )
            .as_bytes(),
            ClientRevision::R274,
        )
        .expect_err("a present family must carry its coverage");
    assert!(error.contains("no coverage"), "unexpected error: {error}");
}

#[test]
fn missing_quest_identity_is_absent_not_an_empty_list() {
    let data = SelectedGameData::decode(minimal_json("").as_bytes(), ClientRevision::R274)
        .expect("schema 4 without the field still decodes");
    assert!(data.quest_identity().is_none());
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
