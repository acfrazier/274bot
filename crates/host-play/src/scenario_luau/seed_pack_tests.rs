//! Failure modes covered before implementation:
//! 1. Unknown TOML keys and parameter kinds fail closed.
//! 2. Templates reject literal arguments and multiple commands.
//! 3. Expanded commands above the engine's 80-byte limit fail before send.
//! 4. Oversize files and duplicate operation names are rejected.
//! 5. Typed names that do not resolve in the selected table are rejected.
//! 6. Confirmation labels never claim confirmed-operation.
//! 7. Confirmed-only admission rejects none and reply-only contracts.
//! 8. World scope requires an explicit grant identity.
//! 9. Different pack bytes produce different identities.
//! 10. A matching pre-send chat line cannot satisfy a reply confirmation.

use super::*;
use api::{
    identity::{SessionKey, SlotKey},
    scenario_observe::ScenarioObserveWatermark,
    snapshot::{ChatLineView, GameSnapshot},
};
use client::{
    client::{CheatAdmission, Client, ClientConfig, ClientPlayer},
    config::Cache,
    io::{ClientRevision, ClientStream, Isaac},
};
use serde_json::{json, Map, Value};
use std::{
    fs,
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

const SIMPLE_PACK: &str = r#"
version = 1

[[operations]]
name = "echo"
template = "echo {value}"
effects = ["the selected state changes"]
precondition = "none"
scope = "own-account"
confirm = { reply = { exact = "Done." } }

[operations.params.value]
kind = "int"
min = 0
max = 9
"#;

fn parse(source: &str) -> Result<SeedPack, String> {
    SeedPack::from_bytes(source.as_bytes())
}

fn name_pack() -> String {
    r#"
version = 1

[[operations]]
name = "inspect"
template = "inspect {target}"
effects = ["the selected target is inspected"]
precondition = "none"
scope = "own-account"
confirm = { state = [{ kind = "tile", x = 1, z = 2, level = 0 }] }

[operations.params.target]
kind = "name"
table = "item"
"#
    .to_owned()
}

struct MissingName;

impl PackNameLookup for MissingName {
    fn resolve(&self, _table: NameTable, _name: &str) -> Result<ResolvedName, String> {
        Err("name is absent from the selected table".into())
    }
}

fn test_client_and_snapshot() -> (Client, GameSnapshot, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
    let port = listener.local_addr().expect("listener address").port();
    let stream = ClientStream::connect("127.0.0.1", port).expect("connect test client");
    let (server, _) = listener.accept().expect("accept test client");
    let config = ClientConfig {
        host: "127.0.0.1".into(),
        port: 1,
        cache_dir: String::new(),
        members: true,
        lowmem: false,
    };
    let mut client = Client::from_shared_with_revision(
        config,
        Arc::new(Cache::default()),
        Arc::new(Vec::new()),
        Arc::new(Vec::new()),
        ClientRevision::R289,
    );
    client.stream = Some(stream);
    client.ingame = true;
    client.scene_state = 2;
    client.local_player = Some(ClientPlayer::at(5, 5));
    client.out.random = Some(Isaac::new(&[1, 2, 3, 4]));
    client.set_cheat_admission(CheatAdmission::Granted);
    client.gens.player = 1;
    client.gens.scene = 1;
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(&client);
    snapshot.seed_tick(0);
    (client, snapshot, server)
}

fn test_watermark() -> ScenarioObserveWatermark {
    ScenarioObserveWatermark {
        session: SessionKey::new(SlotKey::from_incarnation(1).expect("slot key"), 0),
        client_session: 0,
        source_serial: 0,
        application_serial: 0,
    }
}

fn chat_line(sequence: i32) -> ChatLineView {
    ChatLineView {
        type_: 0,
        username: None,
        text: "Done.".into(),
        sequence,
    }
}

#[test]
fn scenario_luau_seed_pack_rejects_unknown_keys_and_bad_kinds() {
    let unknown_key = SIMPLE_PACK.replace(
        "scope = \"own-account\"",
        "scope = \"own-account\"\nunknown = \"value\"",
    );
    assert!(parse(&unknown_key).is_err());

    let bad_kind = SIMPLE_PACK.replace("kind = \"int\"", "kind = \"lua\"");
    assert!(parse(&bad_kind).is_err());
}

#[test]
fn scenario_luau_seed_pack_rejects_raw_template_text_and_second_command() {
    let raw_argument = SIMPLE_PACK.replace("echo {value}", "echo fixed {value}");
    assert!(parse(&raw_argument).is_err());

    let second_command = SIMPLE_PACK.replace("echo {value}", "echo {value}; other");
    assert!(parse(&second_command).is_err());
}

#[test]
fn scenario_luau_seed_pack_rejects_expansion_over_80_bytes() {
    let pack = parse(SIMPLE_PACK).expect("valid bounded pack");
    let operation = pack.operations.get("echo").expect("echo operation");
    let template = parse_template(&operation.template, &operation.params).expect("template");
    let values = BTreeMap::from([("value".to_owned(), "x".repeat(77))]);
    assert!(template.expand(&values).is_err());
}

#[test]
fn scenario_luau_seed_pack_rejects_oversize_file_and_duplicate_operations() {
    static NEXT_FILE: AtomicU64 = AtomicU64::new(1);
    let suffix = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf = std::env::temp_dir().join(format!(
        "host-play-seed-pack-{}-{suffix}.toml",
        std::process::id()
    ));
    fs::write(&path, vec![b' '; MAX_PACK_BYTES as usize + 1]).expect("write oversize pack");
    let result = SeedPack::load(&path);
    fs::remove_file(&path).expect("remove oversize pack");
    assert!(result.is_err());

    let duplicate = format!("{SIMPLE_PACK}\n{SIMPLE_PACK}");
    assert!(parse(&duplicate).is_err());
}

#[test]
fn scenario_luau_seed_pack_rejects_unresolved_names() {
    let pack = parse(&name_pack()).expect("valid name pack");
    let operation = pack.operations.get("inspect").expect("inspect operation");
    let mut args = Map::new();
    args.insert("target".into(), Value::String("missing_item".into()));
    assert!(SeedPack::resolve_operation(operation, &args, &MissingName).is_err());
}

#[test]
fn scenario_luau_seed_pack_classifies_only_declared_confirmation() {
    let none = parse(&SIMPLE_PACK.replace("{ reply = { exact = \"Done.\" } }", "\"none\""))
        .expect("none pack");
    assert_eq!(none.operations["echo"].confirmation.label(), "unconfirmed");

    let reply = parse(SIMPLE_PACK).expect("reply pack");
    assert_eq!(reply.operations["echo"].confirmation.label(), "reply-seen");
    assert!(!reply.operations["echo"]
        .confirmation
        .label()
        .contains("confirmed-operation"));

    let state = parse(&name_pack()).expect("state pack");
    assert_eq!(
        state.operations["inspect"].confirmation.label(),
        "established-state"
    );
}

#[test]
fn scenario_luau_seed_pack_confirmed_only_rejects_weak_confirmations() {
    let none = parse(&SIMPLE_PACK.replace("{ reply = { exact = \"Done.\" } }", "\"none\""))
        .expect("none pack");
    assert!(none.validate_admission(true, None).is_err());

    let reply = parse(SIMPLE_PACK).expect("reply pack");
    assert!(reply.validate_admission(true, None).is_err());

    let state = parse(&name_pack()).expect("state pack");
    assert!(state.validate_admission(true, None).is_ok());
}

#[test]
fn scenario_luau_seed_pack_world_scope_requires_grant_identity() {
    let world = SIMPLE_PACK.replace("own-account", "world");
    let pack = parse(&world).expect("world pack");
    assert!(pack.validate_admission(false, None).is_err());
    assert!(pack
        .validate_admission(false, Some("operator-review-17"))
        .is_ok());
}

#[test]
fn scenario_luau_seed_pack_hash_changes_with_input_bytes() {
    let first = parse(SIMPLE_PACK).expect("first pack");
    let second = parse(&format!("{SIMPLE_PACK}\n# changed bytes\n")).expect("second pack");
    assert_ne!(first.sha256(), second.sha256());
}

#[test]
fn scenario_luau_seed_pack_poll_requires_fresh_reply_after_single_send() {
    let pack = parse(SIMPLE_PACK).expect("valid pack");
    let mut args = Map::new();
    args.insert("value".into(), json!(4));
    let (mut client, mut snapshot, _server) = test_client_and_snapshot();
    snapshot.seed_chat_lines(vec![chat_line(10)]);
    let before = client.cheat_packets_sent();
    let watermark = test_watermark();
    let mut pending = pack
        .start(
            &mut client,
            &snapshot,
            &MissingName,
            PackSeedRequest {
                operation_name: "echo",
                args: &args,
                why: "test exact reply handling",
                policy: SeedPolicy::AllowUnconfirmed,
                watermark,
                grant_identity: None,
            },
        )
        .expect("one send");
    assert_eq!(client.cheat_packets_sent(), before + 1);

    snapshot.seed_tick(1);
    assert!(pending
        .poll(&mut client, &snapshot, watermark, &[])
        .expect("old line is not fresh")
        .is_none());

    snapshot.seed_chat_lines(vec![chat_line(11)]);
    let receipt = pending
        .poll(&mut client, &snapshot, watermark, &[])
        .expect("fresh reply poll")
        .expect("fresh reply receipt");
    assert_eq!(receipt["status"], "reply-seen");
    assert_eq!(receipt["command"], "echo 4");
    assert_ne!(receipt["confirmation"], "confirmed-operation");
    assert_eq!(client.cheat_packets_sent(), before + 1);
}
