//! Engine A proof for native NPC Talk-to, Continue, and first-choice dispatch.
//! Every seed is scoped to the one disposable account; the shared engine and content stay unchanged.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::interact::{self, ActionSpec, Interactions, OpTarget, SendResult, WireCommand};
use api::snapshot::GameSnapshot;
use client::client::{Client, ClientRevision};
use client::config::if_type::ButtonType;
use host::Pump;
use host_play::{ProfileOptions, SharedClientTemplate};
use serde_json::json;

// Engine A source: `content/maps/m51_47.jm2:5030` places the captain at this tile;
// `content/scripts/quests/quest_desertrescue/configs/desertrescue.constant:3,33`
// defines stage 3 and spoke-bit 6. The talk/choice order is in
// `content/scripts/quests/quest_desertrescue/scripts/mercenary_captain.rs2:13-19,29-43`.
// Client op and button packet paths are `vendor/fr-client-rust/crates/client/src/client/client.rs:4288-4333,8644-8700`.
const DESERT_CAPTAIN_TILE: (i32, i32, i32) = (3270, 3029, 0);
const TALK_TO: &str = "Talk-to";
const FIRST_MENU_OPTION: &str = "Wow! A real captain!";
const SECOND_MENU_FIRST_OPTION: &str = "That's rude, I ought to teach you some manners.";

fn required(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("live cell requires {key}"))
}

fn required_path(key: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(key).unwrap_or_else(|| panic!("live cell requires {key}")))
}

fn required_port(key: &str) -> u16 {
    required(key)
        .parse::<u16>()
        .unwrap_or_else(|error| panic!("{key} must be a valid port: {error}"))
}

fn live_profile_options() -> ProfileOptions {
    let home = required_path("HOME");
    ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(required_port("BOT_GAME_PORT")),
        http_port: Some(required_port("BOT_HTTP_PORT")),
        cache_dir: Some(required_path("BOT_CACHE_DIR")),
        unpack_dir: Some(home.join("native-menu-unpack")),
        engine_dir: Some(required_path("ENGINE_DIR")),
        ..ProfileOptions::default()
    }
}

fn pump_once(client: &mut Client, snapshot: &mut GameSnapshot, pump: &mut Pump) {
    client.mainloop();
    host::publish_snapshot(snapshot, client, pump.drain_client(client));
}

fn wait_for(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    phase: &str,
    mut ready: impl FnMut(&Client, &GameSnapshot) -> bool,
) {
    // This bounds the test harness only; the Engine A dialogue order comes from the content below.
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        pump_once(client, snapshot, pump);
        if ready(client, snapshot) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {phase}: ingame={}, attached={}, scene_state={}, tile={:?}, continue={}, choices={:?}",
            snapshot.ingame(),
            snapshot.attached(),
            snapshot.scene_state(),
            snapshot.tile(),
            snapshot.chat_continue_component_id(),
            option_texts(snapshot),
        );
        // Match GameShell's 20 ms cycle (client/game_shell.rs:3,64).
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn option_texts(snapshot: &GameSnapshot) -> Vec<String> {
    snapshot
        .chat_options()
        .iter()
        .map(|option| option.text.clone())
        .collect()
}

fn write_receipt(account: &str, value: serde_json::Value) -> PathBuf {
    let dir = required_path("LIVE_EVIDENCE_DIR");
    std::fs::create_dir_all(&dir).expect("create live evidence directory");
    let path = dir.join(format!("native-menu-{account}.json"));
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&value).expect("serialize native menu receipt"),
    )
    .expect("write native menu receipt");
    path
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, BOT_LIVE_NAME_PREFIX, BOT_CACHE_DIR, BOT_GAME_PORT, BOT_HTTP_PORT, ENGINE_DIR and LIVE_EVIDENCE_DIR"]
fn live_engine_a_native_menu_talk_continue_choice() {
    assert_eq!(required("LIVE"), "1");
    assert_eq!(required("BOT_CPU"), "1");
    let prefix = required("BOT_LIVE_NAME_PREFIX");
    assert!(
        (1..=4).contains(&prefix.len()) && prefix.bytes().all(|byte| byte.is_ascii_alphanumeric()),
        "BOT_LIVE_NAME_PREFIX must be one to four ASCII alphanumeric characters"
    );
    let evidence_dir = required_path("LIVE_EVIDENCE_DIR");
    let home = required_path("HOME");
    assert!(
        !evidence_dir.starts_with(&home),
        "live evidence must be outside the disposable HOME"
    );
    assert!(
        api::test_support::game_data_available(),
        "R289 game data is required"
    );
    let selected =
        api::game_data::for_revision(ClientRevision::R289).expect("load selected R289 game data");
    let captain_type = selected
        .npc_by_config("desertminingcaptain")
        .expect("Engine A Mercenary Captain config")
        .id as usize;

    let profile = live_profile_options()
        .resolve(None)
        .expect("resolve local Engine A profile")
        .bind()
        .expect("bind local Engine A profile");
    let template = SharedClientTemplate::load(Arc::clone(&profile))
        .expect("load Engine A R289 client template");
    let account = host_play::mint_live_names(1)
        .into_iter()
        .next()
        .expect("mint one disposable live account");
    let mut client = template
        .prepare_client(1, true)
        .expect("prepare disposable live client");
    client.draw = false;
    client.maininit();
    assert!(!client.error_loading, "client failed to load");
    assert!(interact::login(&mut client, &account, &account, false));

    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        "login and scene",
        |_, state| state.ingame() && state.attached() && state.scene_state() == 2,
    );

    // Stage 3 is `approached_captain`; clearing mechanisms leaves spoke-bit 6
    // unset, selecting the three-choice path at mercenary_captain.rs2:16-19.
    for (command, expected) in [
        ("setvar desertrescue 3", "get desertrescue: 3"),
        (
            "setvar desertrescue_map_mechanisms 0",
            "get desertrescue_map_mechanisms: 0",
        ),
    ] {
        assert!(
            interact::cheat(&mut client, command).is_sent(),
            "seed refused: {command}"
        );
        let query = command
            .strip_prefix("setvar ")
            .and_then(|rest| rest.split_whitespace().next())
            .expect("varp name in setup command");
        assert!(
            interact::cheat(&mut client, &format!("getvar {query}")).is_sent(),
            "varp query refused: {query}"
        );
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "account quest seed",
            |_, state| state.chat_lines().iter().any(|line| line.text == expected),
        );
    }

    interact::seed_at(
        &mut client,
        DESERT_CAPTAIN_TILE.2,
        DESERT_CAPTAIN_TILE.0,
        DESERT_CAPTAIN_TILE.1,
    );
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        "Captain tile",
        |_, state| {
            state.ingame()
                && state.scene_state() == 2
                && state.tile() == Some(DESERT_CAPTAIN_TILE)
                && state.npcs().iter().any(|npc| {
                    npc.r#type == Some(captain_type)
                        && npc.actions.first().and_then(Option::as_deref) == Some(TALK_TO)
                })
        },
    );
    let (captain_slot, captain_tile, talk_result_record) = {
        let captain = snapshot
            .npcs()
            .iter()
            .filter(|npc| {
                npc.r#type == Some(captain_type)
                    && npc
                        .actions
                        .iter()
                        .flatten()
                        .any(|action| action.as_str() == TALK_TO)
            })
            .min_by_key(|npc| npc.distance)
            .expect("Mercenary Captain with Talk-to in the live scene");
        let talk_result = Interactions::new(&snapshot, &mut client)
            .interact(OpTarget::Npc(captain), ActionSpec::Label(TALK_TO.into()));
        assert!(
            matches!(
                &talk_result,
                SendResult::Sent {
                    command: WireCommand::Op { operation: 1, .. },
                    ..
                }
            ),
            "native Talk-to was refused: {talk_result:?}"
        );
        (captain.index, captain.tile, format!("{talk_result:?}"))
    };
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        "Captain Continue prompt",
        |current, state| state.chat_continue_component_id() >= 0 && !current.resumed_pause_button,
    );

    let continue_id = snapshot.chat_continue_component_id();
    let continue_result_record = {
        let continue_result = Interactions::new(&snapshot, &mut client).continue_dialog(None);
        assert!(
            matches!(
                &continue_result,
                SendResult::Sent {
                    command: WireCommand::Continue { component_id },
                    ..
                } if *component_id == continue_id
            ),
            "native Continue was refused: {continue_result:?}"
        );
        format!("{continue_result:?}")
    };
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        "Captain first choice menu",
        |_, state| {
            state
                .chat_options()
                .first()
                .is_some_and(|option| option.text == FIRST_MENU_OPTION)
        },
    );
    let first_menu = option_texts(&snapshot);
    assert!(
        first_menu.len() >= 3,
        "expected the Captain's three choices: {first_menu:?}"
    );

    let first_choice_component = snapshot
        .chat_options()
        .first()
        .expect("first choice component")
        .component_id;
    let choice_result_record = {
        let choice_result = Interactions::new(&snapshot, &mut client).answer_choice(1);
        assert!(
            matches!(
                &choice_result,
                SendResult::Sent {
                    command:
                        WireCommand::Button {
                            component_id,
                            button_type: ButtonType::BUTTON_OK,
                        },
                    ..
                } if *component_id == first_choice_component
            ),
            "native first choice was refused: {choice_result:?}"
        );
        format!("{choice_result:?}")
    };
    // The chosen branch has one player line and one NPC line before multi2
    // (mercenary_captain.rs2:29-32). Wait for each newly received pause control.
    let mut response_continues = Vec::with_capacity(2);
    for phase in ["Captain player reply", "Captain NPC reply"] {
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            phase,
            |current, state| {
                state.chat_continue_component_id() >= 0 && !current.resumed_pause_button
            },
        );
        let component_id = snapshot.chat_continue_component_id();
        let result = Interactions::new(&snapshot, &mut client).continue_dialog(Some(component_id));
        assert!(
            matches!(
                &result,
                SendResult::Sent {
                    command: WireCommand::Continue { component_id: sent },
                    ..
                } if *sent == component_id
            ),
            "{phase} was refused: {result:?}"
        );
        response_continues.push(json!({
            "phase": phase,
            "component_id": component_id,
            "result": format!("{result:?}"),
        }));
    }
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        "response to choice one",
        |_, state| {
            state
                .chat_options()
                .first()
                .is_some_and(|option| option.text == SECOND_MENU_FIRST_OPTION)
        },
    );
    let next_menu = option_texts(&snapshot);
    assert_eq!(
        next_menu.len(),
        2,
        "choice one should open the expected two-choice menu"
    );

    let receipt = write_receipt(
        &account,
        json!({
            "cell": "live_engine_a_native_menu_talk_continue_choice",
            "engine": "Engine A",
            "revision": 289,
            "account": account,
            "seed_scope": "disposable account only",
            "seed": {
                "desertrescue": 3,
                "desertrescue_map_mechanisms": 0,
                "tile": DESERT_CAPTAIN_TILE,
            },
            "target": {
                "npc_config": "desertminingcaptain",
                "npc_type": captain_type,
                "npc_slot": captain_slot,
                "npc_tile": captain_tile,
                "operation": TALK_TO,
            },
            "talk_result": talk_result_record,
            "continue_component": continue_id,
            "continue_result": continue_result_record,
            "first_menu": first_menu,
            "first_choice_component": first_choice_component,
            "choice_one_result": choice_result_record,
            "response_continues": response_continues,
            "next_menu": next_menu,
        }),
    );
    assert!(receipt.is_file(), "runtime receipt was not written");
}
