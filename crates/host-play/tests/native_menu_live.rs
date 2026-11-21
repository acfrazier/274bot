//! Engine A proof for native NPC Talk-to, Continue, and first-choice dispatch.
//! Every seed is scoped to the one disposable account; the shared engine and content stay unchanged.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::interact::{
    self, ActionSpec, Driver, Interactions, OpTarget, PacketCheckpoint, SendResult, WireCommand,
};
use api::query::SceneQuery;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::{Client, ClientRevision};
use client::config::if_type::ButtonType;
use client::io::{client_prot_289::map_client_prot, ClientProt};
use host::Pump;
use host_play::{ProfileOptions, SharedClientTemplate};
use serde_json::json;

// Engine A `content/maps/m49_53.jm2:7715,10214` places the bank booth between
// this public stand and banker1. `interface_bank/configs/banker.npc:1-29`
// makes that banker stationary. `interface_bank/scripts/banker.rs2:7-12`
// permits Talk-to at range 2; lines 21-33 define the dialogue order below.
// Client packet paths are `client/client.rs:4288-4333,8644-8700`.
const BANKER_HOME: WorldTile = WorldTile {
    x: 3187,
    z: 3436,
    level: 0,
};
const BANKER_TALK_RANGE: i32 = 2;
const BANKER_STAND: (i32, i32, i32) = (
    BANKER_HOME.x - BANKER_TALK_RANGE,
    BANKER_HOME.z,
    BANKER_HOME.level,
);
const TALK_TO: &str = "Talk-to";
const FIRST_MENU_OPTION: &str = "I'd like to access my bank account, please.";
const EXPLAIN_BANK_OPTION: &str = "What is this place?";
const SECOND_MENU_FIRST_OPTION: &str = "And what do you do?";

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
            "timed out waiting for {phase}: ingame={}, attached={}, scene_state={}, tile={:?}, continue={}, choices={:?}, outgoing_bytes={}, chat={:?}, npcs={:?}",
            snapshot.ingame(),
            snapshot.attached(),
            snapshot.scene_state(),
            snapshot.tile(),
            snapshot.chat_continue_component_id(),
            option_texts(snapshot),
            client.out.pos,
            snapshot
                .chat_lines()
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            snapshot
                .npcs()
                .iter()
                .map(|npc| (npc.index, npc.r#type, npc.tile, npc.distance))
                .collect::<Vec<_>>(),
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
    let banker_type = selected
        .npc_by_config("banker1")
        .expect("Engine A stationary banker config")
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

    // Confirm only this account's tutorial seed before its teleport.
    // ClientCheatHandler.ts:192-244,279-318,558-591 checks access and applies both.
    assert!(
        interact::cheat(&mut client, "setvar tutorial 1000").is_sent(),
        "tutorial seed was refused"
    );
    assert!(
        interact::cheat(&mut client, "getvar tutorial").is_sent(),
        "tutorial seed query was refused"
    );
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        "account tutorial seed",
        |_, state| {
            state
                .chat_lines()
                .iter()
                .any(|line| line.text == "get tutorial: 1000")
        },
    );

    assert!(
        interact::cheat(
            &mut client,
            &interact::tele_args(BANKER_STAND.2, BANKER_STAND.0, BANKER_STAND.1),
        )
        .is_sent(),
        "account-only banker teleport was refused"
    );
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        "stationary banker stand",
        |_, state| {
            state.ingame()
                && state.scene_state() == 2
                && state.tile() == Some(BANKER_STAND)
                && state.npcs().iter().any(|npc| {
                    npc.r#type == Some(banker_type)
                        && npc.tile == BANKER_HOME
                        && npc.actions.first().and_then(Option::as_deref) == Some(TALK_TO)
                })
        },
    );
    let stand = WorldTile {
        x: BANKER_STAND.0,
        z: BANKER_STAND.1,
        level: BANKER_STAND.2,
    };
    assert!(
        SceneQuery::new(snapshot.scene(), None).walkable(stand),
        "the account-only fixture stand must be walkable"
    );
    let (banker_slot, banker_tile, talk_result_record) = {
        let banker = snapshot
            .npcs()
            .iter()
            .find(|npc| {
                npc.r#type == Some(banker_type)
                    && npc.tile == BANKER_HOME
                    && npc.actions.first().and_then(Option::as_deref) == Some(TALK_TO)
            })
            .expect("stationary banker with Talk-to at the authored position");
        assert_eq!(
            banker.distance, BANKER_TALK_RANGE,
            "the account stand must be in the banker's authored Talk-to range"
        );
        let checkpoint = PacketCheckpoint::capture(&client);
        let local_route = client
            .local_player
            .as_ref()
            .map(|player| (player.route_x[0], player.route_z[0]));
        let talk_result = Interactions::new(&snapshot, &mut client)
            .interact(OpTarget::Npc(banker), ActionSpec::Label(TALK_TO.into()));
        let mut opcodes = Vec::new();
        assert!(
            client.trace_packets(checkpoint, &mut |opcode| opcodes.push(opcode)),
            "the emitted Talk-to frames must be complete"
        );
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
        let expected_opcode =
            u8::try_from(map_client_prot(client.revision(), ClientProt::OPNPC1).id)
                .expect("OPNPC1 is a one-byte opcode in the active revision");
        assert_eq!(
            opcodes,
            [expected_opcode],
            "Talk-to must emit only OPNPC1 at the banker's authored range"
        );
        eprintln!(
            "native-menu Talk-to account={account} player={:?} npc_slot={} npc_tile={:?} result={talk_result:?} opcodes={opcodes:?} local_route={local_route:?} path={:?} flag=({},{}) map_base=({},{})",
            snapshot.tile(),
            banker.index,
            banker.tile,
            client.try_move_path,
            client.minimap_flag_x,
            client.minimap_flag_z,
            client.map_build_base_x,
            client.map_build_base_z,
        );
        (banker.index, banker.tile, format!("{talk_result:?}"))
    };
    wait_for(
        &mut client,
        &mut snapshot,
        &mut pump,
        "banker Continue prompt",
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
        "banker first choice menu",
        |_, state| {
            state
                .chat_options()
                .first()
                .is_some_and(|option| option.text == FIRST_MENU_OPTION)
        },
    );
    let first_menu = option_texts(&snapshot);
    assert_eq!(
        first_menu.len(),
        2,
        "expected the banker's two choices: {first_menu:?}"
    );
    assert_eq!(first_menu[1], EXPLAIN_BANK_OPTION);

    let choice_component = snapshot
        .chat_options()
        .get(1)
        .expect("bank explanation choice component")
        .component_id;
    let choice_result_record = {
        let choice_result = Interactions::new(&snapshot, &mut client).answer_choice(2);
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
                } if *component_id == choice_component
            ),
            "native bank explanation choice was refused: {choice_result:?}"
        );
        format!("{choice_result:?}")
    };
    // The explanation has one player line and one NPC line before its next
    // two-choice menu (banker.rs2:31-33). Wait for each newly received pause control.
    let mut response_continues = Vec::with_capacity(2);
    for phase in ["banker player reply", "banker NPC reply"] {
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
        "response to bank explanation choice",
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
        "the explanation should open the expected two-choice menu"
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
                "tutorial": 1000,
                "tile": BANKER_STAND,
                "banker_home": BANKER_HOME,
                "talk_range": BANKER_TALK_RANGE,
            },
            "target": {
                "npc_config": "banker1",
                "npc_type": banker_type,
                "npc_slot": banker_slot,
                "npc_tile": banker_tile,
                "operation": TALK_TO,
            },
            "talk_result": talk_result_record,
            "continue_component": continue_id,
            "continue_result": continue_result_record,
            "first_menu": first_menu,
            "selected_choice": 2,
            "choice_component": choice_component,
            "choice_result": choice_result_record,
            "response_continues": response_continues,
            "next_menu": next_menu,
        }),
    );
    assert!(receipt.is_file(), "runtime receipt was not written");
}
