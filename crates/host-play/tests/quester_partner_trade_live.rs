//! Live proof of the native Quester partner trade between two real accounts.
//!
//! Two accounts stand on the Varrock Tramp tile (the rendezvous the Shield of
//! Arrav guide authors for the Phoenix key handoff). Only each account's own
//! state is seeded before Start: the Phoenix account is at the joined-gang
//! stage and carries the Phoenix key; the Black Arm account is at the stage
//! that takes it. Both Questers then run the guide's own `p-key` and `b-key`
//! steps (`crates/script/src/quester/families/partner.rs`): the real
//! `Trade with`, the offer, both accepts of the offer screen, the engine's
//! confirm screen (`trade.rs2:75-154`), both accepts of the confirm screen,
//! and the engine's item move (`trade.rs2:257-277`). The cell passes only when
//! the key is in the Black Arm account's backpack and gone from Phoenix's,
//! and both accounts then log out.
//!
//! The Path is the released guide's header, partner declaration and progress
//! rules with every step but the key handoff removed
//! (`fixtures/quester-partner-trade/blackarmgang.json`), served through the
//! folder Path registry. It exists because the pair runner is the only way to
//! reach the native `TradeMachine`.
//!
//! ```text
//! LIVE=1 BOT_CPU=1 BOT_NAV_BUILD=skip BOT_LIVE_NAME_PREFIX=tcs BOT_LIVE_PARTNER_NAME_PREFIX=tct \
//!   WORLD_GAME_PORT=47594 WORLD_HTTP_PORT=3080 WORLD_NAV_PACK=<nav-pack> WORLD_ENGINE_DIR=<engine-dir> \
//!   RS2B0T=<catalog-root> BOT_CACHE_DIR=<cache> LIVE_EVIDENCE_DIR=<evidence> \
//!   isohome cargo test -p host-play --features live-harness,test-support --test quester_partner_trade_live \
//!   -- --ignored --exact live_quester_partner_trade_moves_the_phoenix_key --nocapture --test-threads=1
//! ```
#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use std::path::{Path, PathBuf};
use std::time::Duration;

use api::game_data::SelectedGameData;
use api::selected::ClientRevision;
use api::snapshot::{GameSnapshot, WorldTile};
use quester_live::{Cell, Mode, PairCell, PairMode};
use scenario::quester::{quester_role_stage, QuesterStage};
use script::quester::pair::Gang;
use serde_json::{json, Value};

const PATH_JSON: &str = include_str!("fixtures/quester-partner-trade/blackarmgang.json");
const QUEST: &str = "blackarmgang";
const DISPLAY: &str = "Shield of Arrav";
const KEY_ALIAS: &str = "phoenixkey2";
/// The guide's rendezvous for `p-key` and `b-key` (`maps/m50_52.jm2:8228`).
const STAND: WorldTile = WorldTile {
    x: 3208,
    z: 3391,
    level: 0,
};
/// One reciprocal role's fixture: the authored stage, the content variable that
/// puts the account there (`gang.rs`: `PHOENIX_JOINED`; the Black Arm journal's
/// "in return for stealing" line) and the items the account owns.
struct RoleSeed {
    label: &'static str,
    stage: &'static str,
    seed: (&'static str, i32),
    items: &'static [(&'static str, i32)],
}
const PHOENIX_ROLE: RoleSeed = RoleSeed {
    label: "phoenix-gives-key",
    stage: "p:9",
    seed: ("phoenixgang", 9),
    items: &[(KEY_ALIAS, 1)],
};
const BLACK_ARM_ROLE: RoleSeed = RoleSeed {
    label: "blackarm-takes-key",
    stage: "b:2",
    seed: ("blackarmgang", 2),
    items: &[],
};

fn key_count(snapshot: &GameSnapshot, key: i32) -> i32 {
    snapshot
        .inventory()
        .iter()
        .filter(|item| item.def.id == key && !item.def.noted)
        .map(|item| item.count)
        .sum()
}

fn role_cell(
    selected: &SelectedGameData,
    path: &script::quester::path::PathDocument,
    gang: Gang,
    key: i32,
) -> Result<Cell, String> {
    let identity = selected
        .quest_identity()
        .ok_or("selected R289 quest identity")?
        .rows
        .iter()
        .find(|row| row.id == QUEST)
        .ok_or("selected R289 has no Shield of Arrav identity")?;
    let role = match gang {
        Gang::Phoenix => PHOENIX_ROLE,
        Gang::BlackArm => BLACK_ARM_ROLE,
    };
    let (label, stage, seed, items) = (role.label, role.stage, role.seed, role.items);
    let mut fixture = quester_role_stage(
        QuesterStage {
            name: QUEST,
            quest_display: DISPLAY,
            path,
            identity,
            selected,
            stage,
            loadout: None,
            extra_items: items,
            stand: STAND,
        },
        gang,
        seed.0,
        &[seed],
    )?;
    fixture.scenario.settings.deadline = Duration::from_secs(420);
    fixture
        .start_settings
        .insert("quests".into(), json!([QUEST]));
    let expect_key = i32::from(gang == Gang::Phoenix);
    let seed_record = fixture.seed.clone();
    Ok(Cell {
        quest: QUEST,
        display: DISPLAY,
        label: label.to_owned(),
        scenario: fixture.scenario,
        start_settings: fixture.start_settings,
        mode: Mode::Clean,
        observe_start: Some(Box::new(move |snapshot| {
            let held = key_count(snapshot, key);
            if held != expect_key {
                return Err(format!(
                    "{gang:?} must hold {expect_key} Phoenix key before Start, holds {held}"
                ));
            }
            seed_record.observe_start(snapshot)
        })),
    })
}

fn write_folder_path(evidence: &Path) -> Result<PathBuf, String> {
    let folder = evidence.join("path-folder");
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    std::fs::write(folder.join(format!("{QUEST}.json")), PATH_JSON)
        .map_err(|error| error.to_string())?;
    Ok(folder)
}

#[test]
#[ignore = "requires LIVE=1, an explicitly selected local 289 engine and an isolated HOME; see the module docs"]
fn live_quester_partner_trade_moves_the_phoenix_key() {
    if !api::test_support::game_data_available() {
        return;
    }
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let evidence = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));
    let folder = write_folder_path(&evidence).expect("write the folder Path");
    let source = scenario::quester::reload_quester_path_source(QUEST, Some(&folder))
        .expect("publish the folder Path");
    // A folder Path the bundled index does not list is served as a draft.
    assert!(
        matches!(source, "folder" | "draft"),
        "the cell must run its own Path document, not a bundled one (source {source})"
    );
    let (path, _) = scenario::quester::load_quester_path_document(QUEST).expect("load the Path");
    let selected = api::game_data::for_revision(ClientRevision::R289).expect("selected R289 data");
    let key = selected
        .item_by_alias(KEY_ALIAS)
        .expect("selected R289 Phoenix key")
        .id;

    let roles = [
        role_cell(&selected, &path, Gang::Phoenix, key).expect("Phoenix role fixture"),
        role_cell(&selected, &path, Gang::BlackArm, key).expect("Black Arm role fixture"),
    ];
    let oracle = Box::new(
        move |snapshots: [&GameSnapshot; 2]| -> Result<Option<Value>, String> {
            let [phoenix, black_arm] = [key_count(snapshots[0], key), key_count(snapshots[1], key)];
            if phoenix == 0 && black_arm == 1 {
                return Ok(Some(json!({
                    "phoenix_keys": phoenix,
                    "blackarm_keys": black_arm,
                    "phoenix_tile": snapshots[0].tile(),
                    "blackarm_tile": snapshots[1].tile(),
                })));
            }
            // While the trade is open the key sits in the offer container, so
            // neither backpack holds it; a second copy is the only impossible state.
            if phoenix + black_arm > 1 {
                return Err(format!(
                    "the key was duplicated: phoenix={phoenix} blackarm={black_arm}"
                ));
            }
            Ok(None)
        },
    );
    let receipt = quester_live::run_pair_observed(
        PairCell {
            roles,
            mode: PairMode::Clean,
        },
        oracle,
    )
    .unwrap_or_else(|error| panic!("partner trade cell failed: {error}"));
    assert_eq!(
        receipt["logged_out"],
        json!([true, true]),
        "both accounts must log out"
    );
    assert!(
        receipt["pair_receipt"]["blackarm_keys"] == 1
            && receipt["pair_receipt"]["phoenix_keys"] == 0,
        "the key must move from Phoenix to Black Arm"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use script::quester::compile::{compile_path_for_gang, prepare_for_test};

    /// The embedded fixture is a valid released-shape Path for both reciprocal
    /// roles: each role authors exactly its half of the key handoff.
    #[test]
    fn the_partner_trade_fixture_compiles_for_both_roles() {
        if !api::test_support::game_data_available() {
            return;
        }
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let quests = std::sync::Arc::new(
            api::quest_facts::QuestCatalog::from_identity(selected.quest_identity()).unwrap(),
        );
        for gang in [Gang::Phoenix, Gang::BlackArm] {
            let selected = std::sync::Arc::clone(&selected);
            let quests = std::sync::Arc::clone(&quests);
            let compiled = prepare_for_test(move |worker| {
                compile_path_for_gang(
                    PATH_JSON.as_bytes(),
                    &selected,
                    &quests,
                    worker,
                    Some(gang),
                    &script::local_state::OperatorLoadouts::absent(),
                    None,
                )
            });
            assert!(compiled.is_ok(), "{gang:?}: {:?}", compiled.err());
        }
    }

    #[test]
    fn the_oracle_names_both_halves_of_the_handoff_and_nothing_else() {
        let document: Value = serde_json::from_str(PATH_JSON).unwrap();
        let steps: Vec<(String, String)> = document["roles"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|role| role["sequences"].as_array().unwrap())
            .flat_map(|sequence| sequence["steps"].as_array().unwrap())
            .map(|step| {
                (
                    step["id"].as_str().unwrap().to_owned(),
                    step["kind"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(
            steps,
            [
                ("p-key".to_owned(), "partner".to_owned()),
                ("b-key".to_owned(), "partner".to_owned())
            ]
        );
    }
}
