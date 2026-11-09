//! Fresh-account live proof for the revision-377 Tutorial Island first-step banner.
use super::*;
use api::interact::Driver;
use client::client::MiniMenuAction;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::process::Command;

const GAME_PORT: u16 = 43_596;
const ASSET_PORT: u16 = 82;
const TUTORIAL_VARP: i32 = 281;
const PROGRESS_VARP: i32 = 406;
const BANNER_COMPONENT: usize = 12_227;
const PROGRESS_OVERLAY: i32 = 8_680;
const DESIGN_ACCEPT_CLIENT_CODE: i32 = 326;
const TIMER_MIN_TICKS: u32 = 49;
const TIMER_MAX_TICKS: u32 = 51;
const POST_STEP4_TICKS: u32 = 67;

struct Pins {
    engine_path: PathBuf,
    content_path: PathBuf,
    client_path: PathBuf,
    engine: String,
    content: String,
    client: String,
    host_play: String,
}

#[derive(Clone)]
struct Observation {
    tick: u32,
    tutorial_client_cache: Option<i32>,
    progress: Option<i32>,
    banner_hidden: Option<bool>,
    main_overlay: i32,
    ingame: bool,
    attached: bool,
    chat_modal: i32,
    continue_component: i32,
    dialogue: Vec<String>,
}

fn git_head(path: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| format!("read git pin at {}: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "read git pin at {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let commit = String::from_utf8(output.stdout)
        .map_err(|error| format!("git pin at {} is not UTF-8: {error}", path.display()))?
        .trim()
        .to_owned();
    if commit.is_empty() {
        return Err(format!("git pin at {} is empty", path.display()));
    }
    Ok(commit)
}

fn collect_pins(bound: &Bound377) -> Result<Pins, String> {
    let content_path = bound
        .engine_dir
        .parent()
        .ok_or("engine directory has no parent for content checkout")?
        .join("content");
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or("host-play manifest has no repository root")?
        .to_path_buf();
    let client_path = repo.join("vendor/fr-client-rust");
    Ok(Pins {
        engine: git_head(&bound.engine_dir)?,
        content: git_head(&content_path)?,
        client: git_head(&client_path)?,
        host_play: git_head(&repo)?,
        engine_path: bound.engine_dir.clone(),
        content_path,
        client_path,
    })
}

fn pin_values(pins: &Pins) -> Value {
    json!({
        "engine": pins.engine,
        "content": pins.content,
        "client": pins.client,
        "host_play": pins.host_play,
    })
}

fn pin_paths(pins: &Pins) -> Value {
    json!({
        "engine": pins.engine_path.display().to_string(),
        "content": pins.content_path.display().to_string(),
        "client": pins.client_path.display().to_string(),
    })
}

fn tutorial_step_oracle() -> Value {
    json!({
        "source": "fresh same-account `getvar tutorial` server reply, newer than the chat-sequence baseline; message-box replies are dismissed before continuing",
        "step_1": "server tutorial step 1; the initial timer baseline is normal character-design acceptance",
        "step_4": "server tutorial step 4; the Guide's first scenery-interaction stage, where the banner must stay hidden",
        "client_varp_281": "scope=perm without transmit=yes; recorded only as a non-authoritative client cache",
        "progress_varp_406": "progress-bar invariance only; not used to determine tutorial step",
    })
}

fn classify_failure(error: &str) -> &'static str {
    if error.contains("tooling gap") {
        "tooling-gap"
    } else if error.contains("timed out") || error.contains("readiness predicate failed") {
        "harness-or-environment"
    } else {
        "product-behavior-or-live-fixture"
    }
}

fn write_row(log: &mut File, row: &Value) -> Result<(), String> {
    writeln!(log, "{row}").map_err(|error| format!("write observation JSONL: {error}"))?;
    log.flush()
        .map_err(|error| format!("flush observation JSONL: {error}"))?;
    println!("{row}");
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Checkpoint rows explicitly carry identity, state, and timing evidence."
)]
fn record_observation(
    log: &mut File,
    pins: &Pins,
    account: &str,
    checkpoint: &str,
    state: &Observation,
    elapsed_ticks: Option<u32>,
    elapsed_ms: Option<u128>,
    detail: Value,
) -> Result<(), String> {
    write_row(
        log,
        &json!({
            "kind": "observation",
            "test": "tut_banner_first_step_377",
            "account": account,
            "pins": pin_values(pins),
            "checkpoint": checkpoint,
            "tick": state.tick,
            "elapsed_ticks": elapsed_ticks,
            "elapsed_ms": elapsed_ms,
            "tutorial_client_cache_varp_281": state.tutorial_client_cache,
            "progress_varp_406": state.progress,
            "component_12227_hide": state.banner_hidden,
            "main_overlay_id": state.main_overlay,
            "progress_overlay_8680_active": state.main_overlay == PROGRESS_OVERLAY,
            "ingame": state.ingame,
            "attached": state.attached,
            "chat_modal_id": state.chat_modal,
            "chat_continue_component_id": state.continue_component,
            "dialogue": state.dialogue,
            "detail": detail,
        }),
    )
}

fn record_assertion(
    log: &mut File,
    pins: &Pins,
    account: &str,
    name: &str,
    passed: bool,
    detail: Value,
) -> Result<(), String> {
    write_row(
        log,
        &json!({
            "kind": "assertion",
            "test": "tut_banner_first_step_377",
            "account": account,
            "pins": pin_values(pins),
            "name": name,
            "result": if passed { "PASS" } else { "FAIL" },
            "detail": detail,
        }),
    )
}

fn observe(client: &Client, snapshot: &GameSnapshot) -> Observation {
    Observation {
        tick: snapshot.tick(),
        tutorial_client_cache: snapshot
            .varps()
            .iter()
            .find(|varp| varp.index == TUTORIAL_VARP)
            .map(|varp| varp.value),
        progress: snapshot
            .varps()
            .iter()
            .find(|varp| varp.index == PROGRESS_VARP)
            .map(|varp| varp.value),
        banner_hidden: client.if_(BANNER_COMPONENT).map(|component| component.hide),
        main_overlay: client.main_overlay_id,
        ingame: snapshot.ingame(),
        attached: snapshot.attached(),
        chat_modal: snapshot.modals().chat,
        continue_component: snapshot.chat_continue_component_id(),
        dialogue: snapshot.chat_modal_texts().to_vec(),
    }
}

#[derive(Clone)]
struct FreshTutorialReply {
    value: i32,
    source: &'static str,
    chat_sequence: Option<i32>,
    modal_text: Option<String>,
}

struct TutorialReadback {
    value: i32,
    reply_tick: u32,
    chat_sequence_baseline: i32,
    reply_source: &'static str,
    chat_sequence: Option<i32>,
    message_box_dismissed: bool,
    message_box_dismissal_tick: Option<u32>,
}

fn fresh_tutorial_reply(
    snapshot: &GameSnapshot,
    chat_baseline: i32,
    modal_baseline: &[String],
) -> Option<FreshTutorialReply> {
    if let Some((value, text)) = snapshot
        .chat_modal_texts()
        .iter()
        .filter(|text| !modal_baseline.contains(*text))
        .find_map(|text| {
            scenario::tutorial::parse_tutorial_reply(text).map(|value| (value, text.clone()))
        })
    {
        return Some(FreshTutorialReply {
            value,
            source: "message_box",
            chat_sequence: None,
            modal_text: Some(text),
        });
    }

    snapshot
        .chat_lines()
        .iter()
        .filter(|line| line.sequence > chat_baseline)
        .filter_map(|line| {
            scenario::tutorial::parse_tutorial_reply(&line.text)
                .map(|value| (line.sequence, value))
        })
        .max_by_key(|&(sequence, _)| sequence)
        .map(|(sequence, value)| FreshTutorialReply {
            value,
            source: "chat_line",
            chat_sequence: Some(sequence),
            modal_text: None,
        })
}

fn read_tutorial_server(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    log: &mut File,
    pins: &Pins,
    account: &str,
    phase: &str,
) -> Result<TutorialReadback, String> {
    let chat_baseline = scenario::tutorial::chat_baseline(snapshot);
    let modal_baseline = snapshot.chat_modal_texts().to_vec();
    let wait_started = snapshot.tick();
    let wait_wall = Instant::now();
    if !interact::cheat(client, scenario::tutorial::TUTORIAL_GETVAR).is_sent() {
        return Err(format!(
            "tooling gap: {phase} server get tutorial read was refused"
        ));
    }
    let mut reply = None;
    pump_until(
        client,
        snapshot,
        pump,
        phase,
        Duration::from_secs(15),
        |client, snapshot| {
            log_tick(
                log,
                pins,
                account,
                phase,
                client,
                snapshot,
                wait_started,
                wait_wall,
            )
        },
        |_client, snapshot| {
            if let Some(fresh) =
                fresh_tutorial_reply(snapshot, chat_baseline, &modal_baseline)
            {
                reply = Some(fresh);
                Ok(true)
            } else {
                Ok(false)
            }
        },
    )?;
    let reply = reply.ok_or_else(|| format!("tooling gap: {phase} no fresh get tutorial reply"))?;
    let reply_tick = snapshot.tick();
    let mut message_box_dismissal_tick = None;
    if let Some(modal_text) = reply.modal_text.as_ref() {
        if snapshot
            .chat_modal_texts()
            .iter()
            .any(|text| text == modal_text)
        {
            if snapshot.chat_continue_component_id() == -1 {
                return Err(format!(
                    "tooling gap: {phase} get tutorial message box has no dismiss control"
                ));
            }
            accepted(
                Interactions::new(snapshot, client).continue_dialog(None),
                "dismiss get tutorial message box",
            )?;
            let dismissal_started = snapshot.tick();
            let dismissal_wall = Instant::now();
            pump_until(
                client,
                snapshot,
                pump,
                phase,
                Duration::from_secs(10),
                |client, snapshot| {
                    log_tick(
                        log,
                        pins,
                        account,
                        phase,
                        client,
                        snapshot,
                        dismissal_started,
                        dismissal_wall,
                    )
                },
                |_client, snapshot| {
                    Ok(!snapshot
                        .chat_modal_texts()
                        .iter()
                        .any(|text| text == modal_text))
                },
            )?;
            message_box_dismissal_tick = Some(snapshot.tick());
        }
    }

    let readback = TutorialReadback {
        value: reply.value,
        reply_tick,
        chat_sequence_baseline: chat_baseline,
        reply_source: reply.source,
        chat_sequence: reply.chat_sequence,
        message_box_dismissed: message_box_dismissal_tick.is_some(),
        message_box_dismissal_tick,
    };
    write_row(
        log,
        &json!({
            "kind": "server_readback",
            "test": "tut_banner_first_step_377",
            "account": account,
            "pins": pin_values(pins),
            "checkpoint": phase,
            "command": scenario::tutorial::TUTORIAL_GETVAR,
            "server_tutorial_step": readback.value,
            "reply_tick": readback.reply_tick,
            "chat_sequence_baseline": readback.chat_sequence_baseline,
            "chat_sequence": readback.chat_sequence,
            "reply_source": readback.reply_source,
            "message_box_text": reply.modal_text,
            "message_box_dismissed": readback.message_box_dismissed,
            "message_box_dismissal_tick": readback.message_box_dismissal_tick,
        }),
    )?;
    Ok(readback)
}

fn require_server_tutorial_step(
    readback: &TutorialReadback,
    expected: i32,
    phase: &str,
) -> Result<(), String> {
    if readback.value != expected {
        return Err(format!(
            "{phase}: server get tutorial returned {}, expected {expected}",
            readback.value
        ));
    }
    Ok(())
}

/// Client cache varp 281 is `scope=perm` and is not transmitted; use the fresh server readback for tutorial steps.
fn require_progress_state(
    state: &Observation,
    progress: i32,
    phase: &str,
) -> Result<(), String> {
    require_in_game(state, phase)?;
    match state.progress {
        Some(value) if value == progress => {}
        Some(value) => {
            return Err(format!(
                "{phase}: progress varp {PROGRESS_VARP} was {value}, expected unchanged value {progress}"
            ));
        }
        None => {
            return Err(format!(
                "tooling gap: {phase} cannot observe varp {PROGRESS_VARP}"
            ))
        }
    }
    if state.main_overlay != PROGRESS_OVERLAY {
        return Err(format!(
            "{phase}: progress overlay was {}, expected {PROGRESS_OVERLAY}",
            state.main_overlay
        ));
    }
    Ok(())
}

fn require_in_game(state: &Observation, phase: &str) -> Result<(), String> {
    if !state.ingame || !state.attached {
        return Err(format!(
            "{phase}: expected an attached in-game client, got ingame={} attached={}",
            state.ingame, state.attached
        ));
    }
    Ok(())
}


fn require_banner(state: &Observation, hidden: bool, phase: &str) -> Result<(), String> {
    match state.banner_hidden {
        Some(value) if value == hidden => Ok(()),
        Some(value) => Err(format!(
            "{phase}: component {BANNER_COMPONENT} hide flag was {value}, expected {hidden}"
        )),
        None => Err(format!(
            "tooling gap: {phase} cannot observe component {BANNER_COMPONENT} hide flag"
        )),
    }
}

fn pump_until(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    phase: &str,
    timeout: Duration,
    mut on_tick: impl FnMut(&Client, &GameSnapshot) -> Result<(), String>,
    mut ready: impl FnMut(&Client, &GameSnapshot) -> Result<bool, String>,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last_tick = snapshot.tick();
    loop {
        let started = Instant::now();
        client.mainloop();
        host::publish_snapshot(snapshot, client, pump.drain_client(client));
        let tick = snapshot.tick();
        if tick != last_tick {
            last_tick = tick;
            on_tick(client, snapshot)?;
        }
        if ready(client, snapshot)? {
            return Ok(());
        }
        if client.error_loading {
            return Err(format!(
                "{phase}: client loading failed: {}",
                client.last_progress_message
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{phase}: timed out after {} seconds",
                timeout.as_secs()
            ));
        }
        if let Some(rest) = Duration::from_millis(20).checked_sub(started.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

fn active_design_accept(client: &Client) -> Option<i32> {
    let roots = [
        client.main_modal_id,
        client.side_modal_id,
        client.chat_modal_id,
        client.tut_com_id,
        client.main_overlay_id,
    ];
    client.ifaces_merged().find_map(|component| {
        (component.client_code == DESIGN_ACCEPT_CLIENT_CODE
            && roots.contains(&component.layer_id)
            && !component.hide)
            .then_some(component.id)
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "Tick capture carries the observer, identity, phase, and timing context."
)]
fn log_tick(
    log: &mut File,
    pins: &Pins,
    account: &str,
    checkpoint: &str,
    client: &Client,
    snapshot: &GameSnapshot,
    start_tick: u32,
    wall_start: Instant,
) -> Result<(), String> {
    record_observation(
        log,
        pins,
        account,
        checkpoint,
        &observe(client, snapshot),
        Some(snapshot.tick().saturating_sub(start_tick)),
        Some(wall_start.elapsed().as_millis()),
        Value::Null,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "Each live wait needs the client pump, baseline values, and evidence context."
)]
fn wait_for_banner(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    log: &mut File,
    pins: &Pins,
    account: &str,
    checkpoint: &str,
    start_tick: u32,
    progress: i32,
) -> Result<Observation, String> {
    let wall_start = Instant::now();
    pump_until(
        client,
        snapshot,
        pump,
        checkpoint,
        Duration::from_secs(45),
        |client, snapshot| {
            log_tick(
                log, pins, account, checkpoint, client, snapshot, start_tick, wall_start,
            )
        },
        |client, snapshot| {
            let state = observe(client, snapshot);
            require_progress_state(&state, progress, checkpoint)?;
            let elapsed = state.tick.saturating_sub(start_tick);
            match state.banner_hidden {
                None => Err(format!(
                    "tooling gap: {checkpoint} cannot observe component {BANNER_COMPONENT} hide flag"
                )),
                Some(true) if elapsed <= TIMER_MAX_TICKS => Ok(false),
                Some(true) => Err(format!(
                    "{checkpoint}: banner remained hidden through {elapsed} ticks"
                )),
                Some(false) if (TIMER_MIN_TICKS..=TIMER_MAX_TICKS).contains(&elapsed) => Ok(true),
                Some(false) => Err(format!(
                    "{checkpoint}: banner appeared after {elapsed} ticks, outside {TIMER_MIN_TICKS}..={TIMER_MAX_TICKS}"
                )),
            }
        },
    )?;
    let state = observe(client, snapshot);
    let elapsed = state.tick.saturating_sub(start_tick);
    record_observation(
        log,
        pins,
        account,
        &format!("{checkpoint}_shown"),
        &state,
        Some(elapsed),
        Some(wall_start.elapsed().as_millis()),
        json!({"accepted_tick_window": [TIMER_MIN_TICKS, TIMER_MAX_TICKS]}),
    )?;
    Ok(state)
}

fn find_guide(snapshot: &GameSnapshot) -> Option<&api::snapshot::NpcView> {
    snapshot.npcs().iter().find(|npc| {
        npc.name
            .as_deref()
            .is_some_and(|name| name.to_ascii_lowercase().contains("guide"))
            && npc
                .actions
                .iter()
                .flatten()
                .any(|action| action.eq_ignore_ascii_case(TALK_ACTION))
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "Dialogue polling carries its client pump and evidence context across ticks."
)]
fn ensure_dialogue(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    log: &mut File,
    pins: &Pins,
    account: &str,
    phase: &str,
    progress: i32,
) -> Result<(), String> {
    if snapshot.chat_continue_component_id() == -1 {
        let guide = find_guide(snapshot).ok_or_else(|| {
            format!("{phase}: no visible Talk-to Guide target in the current snapshot")
        })?;
        let guide_name = guide.name.clone().unwrap_or_default();
        let guide_type = guide.r#type;
        let guide_index = guide.index;
        accepted(
            Interactions::new(snapshot, client).interact(
                OpTarget::Npc(guide),
                ActionSpec::Label(TALK_ACTION.to_owned()),
            ),
            phase,
        )?;
        let waiting_from = snapshot.tick();
        let wall_start = Instant::now();
        pump_until(
            client,
            snapshot,
            pump,
            phase,
            Duration::from_secs(20),
            |client, snapshot| {
                log_tick(
                    log,
                    pins,
                    account,
                    phase,
                    client,
                    snapshot,
                    waiting_from,
                    wall_start,
                )?;
                let state = observe(client, snapshot);
                require_progress_state(&state, progress, phase)?;
                require_banner(&state, false, phase)
            },
            |_client, snapshot| Ok(snapshot.chat_continue_component_id() != -1),
        )?;
        record_observation(
            log,
            pins,
            account,
            phase,
            &observe(client, snapshot),
            Some(snapshot.tick().saturating_sub(waiting_from)),
            Some(wall_start.elapsed().as_millis()),
            json!({
                "guide_name": guide_name,
                "guide_type": guide_type,
                "guide_index": guide_index,
                "action": TALK_ACTION,
            }),
        )?;
    }
    Ok(())
}

fn pass_assertion(
    assertions: &mut Vec<String>,
    log: &mut File,
    pins: &Pins,
    account: &str,
    name: &str,
    detail: Value,
) -> Result<(), String> {
    record_assertion(log, pins, account, name, true, detail)?;
    assertions.push(name.to_owned());
    Ok(())
}

fn write_receipt(path: &Path, receipt: &Value) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("create receipt {}: {error}", path.display()))?;
    serde_json::to_writer_pretty(&mut file, receipt)
        .map_err(|error| format!("write receipt {}: {error}", path.display()))?;
    file.write_all(b"\n")
        .map_err(|error| format!("finish receipt {}: {error}", path.display()))
}

#[test]
#[ignore = "requires the live tut-banner 377 engine, verified client cache, LIVE=1, and BOT_LIVE_NAME_PREFIX=frtb"]
fn tut_banner_first_step_377() -> Result<(), String> {
    if std::env::var("BOT_LIVE_NAME_PREFIX").ok().as_deref() != Some("frtb") {
        return Err("BOT_LIVE_NAME_PREFIX must be exactly frtb for this fresh-account cell".into());
    }

    let bound = load_bound_377_with_ports(GAME_PORT, ASSET_PORT)?;
    resource_record(&bound);
    let pins = collect_pins(&bound)?;
    let directory = required_path("TUT_BANNER_EVIDENCE_DIR")?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("create evidence directory {}: {error}", directory.display()))?;
    let observations_path = directory.join("observations.jsonl");
    let receipt_path = directory.join("receipt.json");
    let mut log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&observations_path)
        .map_err(|error| {
            format!(
                "create observations {}: {error}",
                observations_path.display()
            )
        })?;

    let account = host_play::mint_live_names(1)
        .into_iter()
        .next()
        .ok_or("could not mint one fresh live account")?;
    if !account.starts_with("frtb") {
        return Err(format!(
            "minted live account {account} did not use prefix frtb"
        ));
    }
    let mut assertions = Vec::new();
    write_row(
        &mut log,
        &json!({
            "kind": "metadata",
            "test": "tut_banner_first_step_377",
            "account": account,
            "fresh_account": true,
            "prefix": "frtb",
            "tutorial_state_seeds": [],
            "character_design": "normal client accept-design action (client code 326)",
            "game_port": GAME_PORT,
            "asset_port": ASSET_PORT,
            "pins": pin_values(&pins),
            "pin_paths": pin_paths(&pins),
            "tutorial_step_oracle": tutorial_step_oracle(),
            "client_cache_manifest": bound.cache_dir.join("SHA256SUMS").display().to_string(),
            "client_unpack_dir": bound.unpack_dir.display().to_string(),
            "observations_path": observations_path.display().to_string(),
        }),
    )?;

    let password = host_play::profile_password(&account);
    let mut client = bound.template.prepare_client(fresh_uid(24), false)?;
    assert_377_client(&client, false)?;
    if !Arc::ptr_eq(
        client
            .session_profile()
            .ok_or("prepared client has no bound session profile")?,
        bound.profile.client(),
    ) {
        return Err(
            "prepared client session is not bound to the selected tut-banner profile".into(),
        );
    }
    client.draw = false;
    client.maininit();
    if client.error_loading {
        return Err(format!(
            "377 asset initialization failed: {}",
            client.last_progress_message
        ));
    }
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    let result = (|| -> Result<(), String> {
        write_row(
            &mut log,
            &json!({
                "kind": "event",
                "event": "login_request",
                "account": account,
                "pins": pin_values(&pins),
                "password": "<redacted>",
            }),
        )?;
        if !interact::login(&mut client, &account, &password, false) {
            return Err(format!(
                "fresh-account login refused: {} {}",
                client.login_mes1, client.login_mes2
            ));
        }
        let design_wait_start = snapshot.tick();
        pump_until(
            &mut client,
            &mut snapshot,
            &mut pump,
            "character_design_wait",
            Duration::from_secs(90),
            |_client, _snapshot| Ok(()),
            |client, _snapshot| Ok(active_design_accept(client).is_some()),
        )?;
        let design_state = observe(&client, &snapshot);
        record_observation(
            &mut log,
            &pins,
            &account,
            "character_design_open",
            &design_state,
            Some(design_state.tick.saturating_sub(design_wait_start)),
            None,
            json!({
                "main_modal_id": client.main_modal_id,
                "side_modal_id": client.side_modal_id,
                "chat_modal_id": client.chat_modal_id,
                "tutorial_component_id": client.tut_com_id,
            }),
        )?;
        let accept_id = active_design_accept(&client)
            .ok_or("character-design accept button disappeared before click")?;
        let out_before = client.out.pos;
        Driver::set_menu(&mut client, 0, MiniMenuAction::IF_BUTTON, 0, 0, accept_id);
        if !Driver::do_action(&mut client, 0) {
            return Err("normal character-design accept action was refused".into());
        }
        let out_after = client.out.pos;
        if out_after == out_before {
            return Err("character-design accept action emitted no client packet".into());
        }
        record_observation(
            &mut log,
            &pins,
            &account,
            "character_design_accepted",
            &observe(&client, &snapshot),
            None,
            None,
            json!({
                "component_id": accept_id,
                "client_code": DESIGN_ACCEPT_CLIENT_CODE,
                "packet_bytes_queued": out_after.saturating_sub(out_before),
            }),
        )?;

        let first_t0 = snapshot.tick();
        let first_wall = Instant::now();
        pump_until(
            &mut client,
            &mut snapshot,
            &mut pump,
            "step1_start_wait",
            Duration::from_secs(15),
            |client, snapshot| {
                log_tick(
                    &mut log,
                    &pins,
                    &account,
                    "step1_start_wait",
                    client,
                    snapshot,
                    first_t0,
                    first_wall,
                )
            },
            |_client, snapshot| {
                Ok(snapshot.tick() > first_t0 && snapshot.ingame() && snapshot.attached())
            },
        )?;
        let first_start_before_readback = observe(&client, &snapshot);
        let progress = first_start_before_readback
            .progress
            .ok_or("tooling gap: step-1 start cannot observe progress varp 406")?;
        require_progress_state(&first_start_before_readback, progress, "step1_start")?;
        require_banner(&first_start_before_readback, true, "step1_start")?;
        let first_start_step = read_tutorial_server(
            &mut client,
            &mut snapshot,
            &mut pump,
            &mut log,
            &pins,
            &account,
            "step1_start_server_readback",
        )?;
        require_server_tutorial_step(&first_start_step, 1, "step1_start")?;
        let first_start = observe(&client, &snapshot);
        require_progress_state(&first_start, progress, "step1_start")?;
        require_banner(&first_start, true, "step1_start")?;
        record_observation(
            &mut log,
            &pins,
            &account,
            "step1_start",
            &first_start,
            Some(first_start.tick.saturating_sub(first_t0)),
            Some(first_wall.elapsed().as_millis()),
            json!({
                "timer_start_tick": first_t0,
                "server_tutorial_step": first_start_step.value,
                "server_readback_reply_tick": first_start_step.reply_tick,
                "expected_banner_hidden": true,
            }),
        )?;
        pass_assertion(
            &mut assertions,
            &mut log,
            &pins,
            &account,
            "step1_start_hidden_and_progress_bar_present",
            json!({
                "tick": first_start.tick,
                "timer_start_tick": first_t0,
                "server_tutorial_step": first_start_step.value,
                "server_readback_reply_tick": first_start_step.reply_tick,
                "tutorial_client_cache_281": first_start.tutorial_client_cache,
                "component_12227_hide": first_start.banner_hidden,
                "progress_varp_406": progress,
                "overlay_id": PROGRESS_OVERLAY,
            }),
        )?;

        let first_shown = wait_for_banner(
            &mut client,
            &mut snapshot,
            &mut pump,
            &mut log,
            &pins,
            &account,
            "first_timer",
            first_t0,
            progress,
        )?;
        let first_reminder_step = read_tutorial_server(
            &mut client,
            &mut snapshot,
            &mut pump,
            &mut log,
            &pins,
            &account,
            "first_reminder_server_readback",
        )?;
        require_server_tutorial_step(&first_reminder_step, 1, "first_reminder")?;
        let first_reminder_state = observe(&client, &snapshot);
        require_progress_state(&first_reminder_state, progress, "first_reminder")?;
        require_banner(&first_reminder_state, false, "first_reminder")?;
        pass_assertion(
            &mut assertions,
            &mut log,
            &pins,
            &account,
            "first_reminder_shown_at_50_plus_minus_1_ticks_while_server_tutorial_step_is_1",
            json!({
                "start_tick": first_t0,
                "shown_tick": first_shown.tick,
                "elapsed_ticks": first_shown.tick.saturating_sub(first_t0),
                "server_tutorial_step": first_reminder_step.value,
                "server_readback_reply_tick": first_reminder_step.reply_tick,
                "tutorial_client_cache_281": first_shown.tutorial_client_cache,
                "component_12227_hide": first_reminder_state.banner_hidden,
            }),
        )?;

        ensure_dialogue(
            &mut client,
            &mut snapshot,
            &mut pump,
            &mut log,
            &pins,
            &account,
            "first_dialogue_ready",
            progress,
        )?;
        let before_continue = observe(&client, &snapshot);
        require_progress_state(&before_continue, progress, "before_first_continue")?;
        require_banner(&before_continue, false, "before_first_continue")?;
        record_observation(
            &mut log,
            &pins,
            &account,
            "before_first_continue",
            &before_continue,
            Some(before_continue.tick.saturating_sub(first_t0)),
            Some(first_wall.elapsed().as_millis()),
            json!({"continue_component_id": before_continue.continue_component}),
        )?;
        accepted(
            Interactions::new(&snapshot, &mut client).continue_dialog(None),
            "first ordinary dialogue continue",
        )?;
        let continue_wall = Instant::now();
        pump_until(
            &mut client,
            &mut snapshot,
            &mut pump,
            "first_continue_wait",
            Duration::from_secs(15),
            |client, snapshot| {
                log_tick(
                    &mut log,
                    &pins,
                    &account,
                    "first_continue_wait",
                    client,
                    snapshot,
                    before_continue.tick,
                    continue_wall,
                )?;
                let state = observe(client, snapshot);
                require_progress_state(&state, progress, "first_continue_wait")?;
                require_banner(&state, false, "first_continue_wait")
            },
            |_client, snapshot| Ok(snapshot.tick() > before_continue.tick),
        )?;
        let after_continue_step = read_tutorial_server(
            &mut client,
            &mut snapshot,
            &mut pump,
            &mut log,
            &pins,
            &account,
            "after_first_continue_server_readback",
        )?;
        require_server_tutorial_step(&after_continue_step, 1, "after_first_continue")?;
        let after_continue = observe(&client, &snapshot);
        require_progress_state(&after_continue, progress, "after_first_continue")?;
        require_banner(&after_continue, false, "after_first_continue")?;
        record_observation(
            &mut log,
            &pins,
            &account,
            "after_first_continue",
            &after_continue,
            Some(after_continue.tick.saturating_sub(first_t0)),
            Some(first_wall.elapsed().as_millis()),
            json!({
                "continue_component_id_before": before_continue.continue_component,
                "server_tutorial_step": after_continue_step.value,
                "server_readback_reply_tick": after_continue_step.reply_tick,
            }),
        )?;
        pass_assertion(
            &mut assertions,
            &mut log,
            &pins,
            &account,
            "banner_remains_shown_after_dialogue_continue",
            json!({
                "server_tutorial_step": after_continue_step.value,
                "server_readback_reply_tick": after_continue_step.reply_tick,
                "tutorial_client_cache_281": after_continue.tutorial_client_cache,
                "component_12227_hide": after_continue.banner_hidden,
                "progress_varp_406": after_continue.progress,
            }),
        )?;

        let before_logout = observe(&client, &snapshot);
        record_observation(
            &mut log,
            &pins,
            &account,
            "before_logout",
            &before_logout,
            Some(before_logout.tick.saturating_sub(first_t0)),
            Some(first_wall.elapsed().as_millis()),
            Value::Null,
        )?;
        let ifaces = Arc::clone(&client.ifaces);
        if !interact::logout(&mut client, ifaces.as_slice()) {
            return Err("normal logout interface unavailable on the fresh account".into());
        }
        let logout_wait = snapshot.tick();
        let logout_wall = Instant::now();
        pump_until(
            &mut client,
            &mut snapshot,
            &mut pump,
            "logout_wait",
            Duration::from_secs(60),
            |client, snapshot| {
                log_tick(
                    &mut log,
                    &pins,
                    &account,
                    "logout_wait",
                    client,
                    snapshot,
                    logout_wait,
                    logout_wall,
                )?;
                let state = observe(client, snapshot);
                if state.ingame {
                    require_progress_state(&state, progress, "logout_wait")?;
                    require_banner(&state, false, "logout_wait")?;
                }
                Ok(())
            },
            |_client, snapshot| Ok(!snapshot.ingame()),
        )?;
        let after_logout = observe(&client, &snapshot);
        record_observation(
            &mut log,
            &pins,
            &account,
            "after_logout",
            &after_logout,
            None,
            Some(logout_wall.elapsed().as_millis()),
            json!({"normal_logout_button": true}),
        )?;

        if !interact::login(&mut client, &account, &password, false) {
            return Err(format!(
                "fresh-account relog refused: {} {}",
                client.login_mes1, client.login_mes2
            ));
        }
        let relog_wait = snapshot.tick();
        let relog_wall = Instant::now();
        pump_until(
            &mut client,
            &mut snapshot,
            &mut pump,
            "relogin_step1_wait",
            Duration::from_secs(90),
            |client, snapshot| {
                log_tick(
                    &mut log,
                    &pins,
                    &account,
                    "relogin_step1_wait",
                    client,
                    snapshot,
                    relog_wait,
                    relog_wall,
                )?;
                let state = observe(client, snapshot);
                if state.ingame && state.attached {
                    require_progress_state(&state, progress, "relogin_step1_wait")?;
                    require_banner(&state, true, "relogin_step1_wait")?;
                }
                Ok(())
            },
            |client, snapshot| {
                let state = observe(client, snapshot);
                Ok(state.ingame && state.attached)
            },
        )?;
        let relog_start = observe(&client, &snapshot);
        require_progress_state(&relog_start, progress, "relogin_step1_start")?;
        require_banner(&relog_start, true, "relogin_step1_start")?;
        let relog_t0 = relog_start.tick;
        let relog_timer_wall = Instant::now();
        let relog_start_step = read_tutorial_server(
            &mut client,
            &mut snapshot,
            &mut pump,
            &mut log,
            &pins,
            &account,
            "relogin_step1_server_readback",
        )?;
        require_server_tutorial_step(&relog_start_step, 1, "relogin_step1_start")?;
        let relog_start_after_readback = observe(&client, &snapshot);
        require_progress_state(&relog_start_after_readback, progress, "relogin_step1_start")?;
        require_banner(&relog_start_after_readback, true, "relogin_step1_start")?;
        record_observation(
            &mut log,
            &pins,
            &account,
            "relogin_step1_start",
            &relog_start_after_readback,
            Some(relog_start_after_readback.tick.saturating_sub(relog_t0)),
            Some(relog_timer_wall.elapsed().as_millis()),
            json!({
                "timer_start_tick": relog_t0,
                "server_tutorial_step": relog_start_step.value,
                "server_readback_reply_tick": relog_start_step.reply_tick,
                "expected_banner_hidden": true,
            }),
        )?;
        pass_assertion(
            &mut assertions,
            &mut log,
            &pins,
            &account,
            "relogin_step1_login_is_hidden",
            json!({
                "tick": relog_t0,
                "server_tutorial_step": relog_start_step.value,
                "server_readback_reply_tick": relog_start_step.reply_tick,
                "tutorial_client_cache_281": relog_start.tutorial_client_cache,
                "component_12227_hide": relog_start_after_readback.banner_hidden,
                "progress_varp_406": relog_start_after_readback.progress,
            }),
        )?;
        let relog_shown = wait_for_banner(
            &mut client,
            &mut snapshot,
            &mut pump,
            &mut log,
            &pins,
            &account,
            "relogin_timer",
            relog_t0,
            progress,
        )?;
        let relog_reminder_step = read_tutorial_server(
            &mut client,
            &mut snapshot,
            &mut pump,
            &mut log,
            &pins,
            &account,
            "relogin_reminder_server_readback",
        )?;
        require_server_tutorial_step(&relog_reminder_step, 1, "relogin_reminder")?;
        let relog_reminder_state = observe(&client, &snapshot);
        require_progress_state(&relog_reminder_state, progress, "relogin_reminder")?;
        require_banner(&relog_reminder_state, false, "relogin_reminder")?;
        pass_assertion(
            &mut assertions,
            &mut log,
            &pins,
            &account,
            "relogin_reminder_shown_at_50_plus_minus_1_ticks_while_server_tutorial_step_is_1",
            json!({
                "start_tick": relog_t0,
                "shown_tick": relog_shown.tick,
                "elapsed_ticks": relog_shown.tick.saturating_sub(relog_t0),
                "server_tutorial_step": relog_reminder_step.value,
                "server_readback_reply_tick": relog_reminder_step.reply_tick,
                "tutorial_client_cache_281": relog_shown.tutorial_client_cache,
                "component_12227_hide": relog_reminder_state.banner_hidden,
            }),
        )?;

        ensure_dialogue(
            &mut client,
            &mut snapshot,
            &mut pump,
            &mut log,
            &pins,
            &account,
            "guide_dialogue_ready",
            progress,
        )?;
        let mut tutorial_step = relog_reminder_step;
        require_server_tutorial_step(&tutorial_step, 1, "guide_dialogue_start")?;
        let mut dialogue_pages = 0_u32;
        loop {
            let before = observe(&client, &snapshot);
            require_progress_state(&before, progress, "guide_dialogue_continue")?;
            require_server_tutorial_step(&tutorial_step, 1, "guide_dialogue_continue")?;
            require_banner(&before, false, "guide_dialogue_continue")?;
            if before.continue_component == -1 {
                ensure_dialogue(
                    &mut client,
                    &mut snapshot,
                    &mut pump,
                    &mut log,
                    &pins,
                    &account,
                    "guide_dialogue_reopened",
                    progress,
                )?;
                continue;
            }
            dialogue_pages += 1;
            if dialogue_pages > 12 {
                return Err(
                    "Guide dialogue exceeded 12 Continue pages without tutorial step 4".into(),
                );
            }
            record_observation(
                &mut log,
                &pins,
                &account,
                "guide_dialogue_before_continue",
                &before,
                Some(before.tick.saturating_sub(relog_t0)),
                Some(relog_timer_wall.elapsed().as_millis()),
                json!({
                    "page": dialogue_pages,
                    "server_tutorial_step": tutorial_step.value,
                }),
            )?;
            accepted(
                Interactions::new(&snapshot, &mut client).continue_dialog(None),
                "Guide dialogue continue",
            )?;
            let page_wall = Instant::now();
            pump_until(
                &mut client,
                &mut snapshot,
                &mut pump,
                "guide_dialogue_progress",
                Duration::from_secs(20),
                |client, snapshot| {
                    log_tick(
                        &mut log,
                        &pins,
                        &account,
                        "guide_dialogue_progress",
                        client,
                        snapshot,
                        before.tick,
                        page_wall,
                    )?;
                    let state = observe(client, snapshot);
                    require_progress_state(&state, progress, "guide_dialogue_progress")
                },
                |_client, snapshot| {
                    Ok(snapshot.tick() > before.tick
                        && (snapshot.chat_modal_texts() != before.dialogue.as_slice()
                            || snapshot.chat_continue_component_id()
                                != before.continue_component))
                },
            )?;
            let readback_phase = format!("guide_step_after_continue_{dialogue_pages}");
            let next_step = read_tutorial_server(
                &mut client,
                &mut snapshot,
                &mut pump,
                &mut log,
                &pins,
                &account,
                &readback_phase,
            )?;
            let after = observe(&client, &snapshot);
            require_progress_state(&after, progress, "guide_dialogue_next_page")?;
            match next_step.value {
                4 => {
                    require_banner(&after, true, "step4_advanced")?;
                    record_observation(
                        &mut log,
                        &pins,
                        &account,
                        "step4_advanced",
                        &after,
                        Some(after.tick.saturating_sub(before.tick)),
                        Some(page_wall.elapsed().as_millis()),
                        json!({
                            "guide_continue_pages": dialogue_pages,
                            "server_tutorial_step": next_step.value,
                            "server_readback_reply_tick": next_step.reply_tick,
                        }),
                    )?;
                    pass_assertion(
                        &mut assertions,
                        &mut log,
                        &pins,
                        &account,
                        "banner_hidden_when_server_tutorial_step_advances_to_4",
                        json!({
                            "tick": after.tick,
                            "server_tutorial_step": next_step.value,
                            "server_readback_reply_tick": next_step.reply_tick,
                            "tutorial_client_cache_281": after.tutorial_client_cache,
                            "component_12227_hide": after.banner_hidden,
                            "progress_varp_406": after.progress,
                        }),
                    )?;
                    break;
                }
                1 => {
                    require_banner(&after, false, "guide_dialogue_next_page")?;
                    record_observation(
                        &mut log,
                        &pins,
                        &account,
                        "guide_dialogue_next_page",
                        &after,
                        Some(after.tick.saturating_sub(before.tick)),
                        Some(page_wall.elapsed().as_millis()),
                        json!({
                            "page": dialogue_pages,
                            "server_tutorial_step": next_step.value,
                            "server_readback_reply_tick": next_step.reply_tick,
                        }),
                    )?;
                    tutorial_step = next_step;
                }
                value => {
                    return Err(format!(
                        "Guide dialogue advanced server tutorial step to unexpected value {value}"
                    ));
                }
            }
        }

        let step4 = observe(&client, &snapshot);
        require_progress_state(&step4, progress, "step4_hidden_watch_start")?;
        require_banner(&step4, true, "step4_hidden_watch_start")?;
        let step4_tick = step4.tick;
        let step4_wall = Instant::now();
        pump_until(
            &mut client,
            &mut snapshot,
            &mut pump,
            "step4_hidden_watch",
            Duration::from_secs(90),
            |client, snapshot| {
                log_tick(
                    &mut log,
                    &pins,
                    &account,
                    "step4_hidden_watch",
                    client,
                    snapshot,
                    step4_tick,
                    step4_wall,
                )?;
                let state = observe(client, snapshot);
                require_progress_state(&state, progress, "step4_hidden_watch")?;
                require_banner(&state, true, "step4_hidden_watch")
            },
            |client, snapshot| {
                let state = observe(client, snapshot);
                require_progress_state(&state, progress, "step4_hidden_watch")?;
                require_banner(&state, true, "step4_hidden_watch")?;
                Ok(state.tick.saturating_sub(step4_tick) >= POST_STEP4_TICKS
                    && step4_wall.elapsed() >= Duration::from_secs(40))
            },
        )?;
        let final_step = read_tutorial_server(
            &mut client,
            &mut snapshot,
            &mut pump,
            &mut log,
            &pins,
            &account,
            "step4_final_server_readback",
        )?;
        require_server_tutorial_step(&final_step, 4, "step4_hidden_after_40_seconds")?;
        let final_state = observe(&client, &snapshot);
        require_progress_state(&final_state, progress, "step4_hidden_after_40_seconds")?;
        require_banner(&final_state, true, "step4_hidden_after_40_seconds")?;
        let final_elapsed = final_state.tick.saturating_sub(step4_tick);
        let final_wall_ms = step4_wall.elapsed().as_millis();
        record_observation(
            &mut log,
            &pins,
            &account,
            "step4_hidden_after_40_seconds",
            &final_state,
            Some(final_elapsed),
            Some(final_wall_ms),
            json!({
                "minimum_game_ticks": POST_STEP4_TICKS,
                "minimum_wall_seconds": 40,
                "server_tutorial_step": final_step.value,
                "server_readback_reply_tick": final_step.reply_tick,
            }),
        )?;
        pass_assertion(
            &mut assertions,
            &mut log,
            &pins,
            &account,
            "banner_remains_hidden_for_40_seconds_after_step4",
            json!({
                "elapsed_ticks": final_elapsed,
                "elapsed_ms": final_wall_ms,
                "server_tutorial_step": final_step.value,
                "server_readback_reply_tick": final_step.reply_tick,
                "tutorial_client_cache_281": final_state.tutorial_client_cache,
                "component_12227_hide": final_state.banner_hidden,
            }),
        )?;
        pass_assertion(
            &mut assertions,
            &mut log,
            &pins,
            &account,
            "progress_overlay_and_varp406_unchanged_throughout",
            json!({"overlay_id": final_state.main_overlay, "progress_varp_406": final_state.progress, "expected_progress_varp_406": progress, "per_tick_checks": true}),
        )?;
        Ok(())
    })();

    if client.ingame {
        client.logout();
    }
    let error = result.as_ref().err().cloned();
    let receipt = json!({
        "test": "tut_banner_first_step_377",
        "result": if result.is_ok() { "PASS" } else { "FAIL" },
        "failure_class": error.as_deref().map(classify_failure),
        "error": error,
        "account": account,
        "fresh_account": true,
        "tutorial_state_seeds": [],
        "tutorial_step_oracle": tutorial_step_oracle(),
        "pins": pin_values(&pins),
        "pin_paths": pin_paths(&pins),
        "assertions_passed": assertions,
        "observations": observations_path.display().to_string(),
    });
    write_receipt(&receipt_path, &receipt)?;
    result?;
    Ok(())
}
