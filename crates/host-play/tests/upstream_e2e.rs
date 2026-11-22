//! Test-only transport for unchanged upstream e2e files. No product shim changes.
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use host_play::{Play, ProfileOptions, SharedClientTemplate};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const FACADE: &str = include_str!("upstream_e2e/facade.js");
const CASES: &str = include_str!("../../../tools/live-map/upstream-cases.json");
const OBSERVER: &str = "import { LoopingBot } from '#/bot/api/bot/Bot.js';\nexport default class E2EObserver extends LoopingBot { async loop() { return 1000; } }";
type Reply = Result<Value, String>;
type FrameAction = Box<dyn FnOnce(&mut client::client::Client) -> Reply + Send>;
type FrameQueue = Arc<Mutex<VecDeque<(FrameAction, mpsc::Sender<Reply>)>>>;

struct NativeEvents(Mutex<VecDeque<Value>>);

impl api::hostlog::Sink for NativeEvents {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        if !record.slot.is_some_and(|slot| slot.starts_with("uea")) {
            return;
        }
        let mut events = self.0.lock().unwrap();
        if events.len() == 1024 {
            events.pop_front();
        }
        events.push_back(json!({
            "account":record.slot, "tick":record.tick, "source":record.source.label(),
            "level":record.level.label(), "message":record.message,
        }));
    }

    fn test_nav_trace_enabled(&self) -> bool {
        true
    }
}

static NATIVE_EVENTS: NativeEvents = NativeEvents(Mutex::new(VecDeque::new()));

fn required(key: &str) -> Result<PathBuf, String> {
    std::env::var_os(key)
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing {key}"))
}

fn engine_port(value: Result<String, std::env::VarError>, fallback: u16) -> Result<u16, String> {
    match value {
        Ok(value) => value
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| format!("invalid engine port: {value}")),
        Err(std::env::VarError::NotPresent) => Ok(fallback),
        Err(error) => Err(error.to_string()),
    }
}

fn engine_preflight(ports: &[u16; 2]) -> Result<(), String> {
    // The upstream preflight checks the server before it creates an account.
    for &port in ports {
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let _connection = TcpStream::connect_timeout(&address, Duration::from_secs(5))
            .map_err(|error| format!("engine preflight {address}: {error}"))?;
    }
    Ok(())
}

fn account_cheat(command: &str) -> bool {
    if command.contains(['\n', '\r']) {
        return false;
    }
    matches!(
        command.split_whitespace().next(),
        Some(
            "tele"
                | "setvar"
                | "getvar"
                | "setstat"
                | "advancestat"
                | "give"
                | "givebank"
                | "clearinv"
                | "~clearinv"
                | "~maxme"
        )
    )
}

#[derive(Debug, PartialEq, Eq)]
struct RunnerLog {
    time: u64,
    msg: String,
}

impl RunnerLog {
    fn received(msg: String) -> Result<Self, String> {
        Self::at(msg, SystemTime::now())
    }

    fn at(msg: String, received_at: SystemTime) -> Result<Self, String> {
        let time = received_at
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("native log receipt time: {error}"))?
            .as_millis();
        let time = u64::try_from(time)
            .map_err(|_| "native log receipt time exceeds the u64 millisecond range".to_owned())?;
        Ok(Self { time, msg })
    }

    fn as_value(&self) -> Value {
        json!({"time":self.time, "msg":&self.msg})
    }
}

fn append_logs(logs: &mut Vec<RunnerLog>, messages: Vec<String>) -> Result<(), String> {
    for message in messages {
        logs.push(RunnerLog::received(message)?);
    }
    Ok(())
}

fn record_stop(
    logs: &mut Vec<RunnerLog>,
    previous: &mut Option<script::ScriptLifecycleReceipt>,
    receipt: Option<script::ScriptLifecycleReceipt>,
) -> Result<(), String> {
    if let Some(receipt) = receipt {
        if previous.as_ref() != Some(&receipt) {
            logs.push(RunnerLog::received(format!(
                "stopped — {}",
                receipt.reason
            ))?);
            *previous = Some(receipt);
        }
    }
    Ok(())
}

fn shot_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("shot path must remain inside the evidence directory".into());
    }
    Ok(root.join(relative))
}

fn frame_call(queue: &FrameQueue, action: FrameAction) -> Reply {
    let (tx, rx) = mpsc::channel();
    queue.lock().unwrap().push_back((action, tx));
    rx.recv_timeout(Duration::from_secs(20))
        .map_err(|e| format!("client frame: {e}"))?
}

struct Session {
    account: String,
    tested: bool,
    observer: bool,
    logs: Vec<RunnerLog>,
    receipt: Option<script::ScriptLifecycleReceipt>,
    error: Option<String>,
    last_issue: Option<Value>,
    settings: script::ScriptSettingsStore,
}

struct Adapter {
    play: Play,
    library: script_js::JsLibrary,
    queue: FrameQueue,
    session: Option<Session>,
    output: PathBuf,
    engine_ports: [u16; 2],
    last_diagnostic: Value,
}

impl Adapter {
    // The only script-start helper: both registry.start and tutorial.startScript use it.
    fn start_script(&mut self, name: &str) -> Reply {
        let session = self.session.as_mut().ok_or("no session")?;
        let (js, checked, siblings, settings) = if name == "E2EObserver" {
            let source = format!("{FACADE}\n{OBSERVER}");
            let (js, checked) = script_js::compile_origin(Path::new("observer.ts"), &source)
                .map_err(|e| e.to_string())?;
            (js, checked, Vec::new(), Map::new())
        } else {
            self.library
                .ensure_js(script::ScriptSource::Catalog, name)?;
            let card = self
                .library
                .get(script::ScriptSource::Catalog, name)
                .ok_or_else(|| format!("catalog has no {name}"))?;
            if let Some(import) = &card.unloadable {
                return Err(format!("{name} is unloadable: {import}"));
            }
            let siblings = script_js::resolve_sibling_modules(
                &card.path,
                &card.origin,
                self.library.cache(),
                script_js::CacheMeta {
                    kind: card.kind,
                    source: card.source,
                    shape: None,
                    api_family: Some(card.api_family().as_str().into()),
                },
            )?;
            let settings = session.settings.merged_bag(
                script::ScriptSource::Catalog,
                name,
                &card.settings_schema,
                None,
            );
            println!(
                "{}",
                json!({"phase":"start", "card":name, "source_sha256":card.sha256, "settings":settings})
            );
            (
                format!("{FACADE}\n{}", card.js),
                card.checked,
                siblings,
                settings,
            )
        };
        if session.observer {
            self.play.script_stop(&session.account);
        }
        if name != "E2EObserver" {
            session.logs.clear();
            session.receipt = None;
            session.error = None;
        }
        self.play
            .script_start_load(&session.account, js, checked, Some(settings), siblings)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match self.play.script_poll_start(&session.account) {
                script::StartPoll::Settled(script::StartOutcome::Ready) => break,
                script::StartPoll::Settled(outcome) => {
                    return Err(format!("{name} start: {outcome:?}"));
                }
                script::StartPoll::NotOwed => return Err(format!("{name} start outcome missing")),
                script::StartPoll::Pending => {
                    if Instant::now() >= deadline {
                        return Err(format!("{name} setup deadline"));
                    }
                    frame_call(&self.queue, Box::new(|_| Ok(Value::Null)))?;
                }
            }
        }
        // Setup readiness does not imply delivery of the first native snapshot.
        let deadline = Instant::now() + Duration::from_secs(20);
        while self
            .play
            .script_probe_for_test(
                &session.account,
                "Boolean(globalThis.__rs2b0t_host?.snapshot?.stats?.length)",
            )?
            .as_bool()
            != Some(true)
        {
            if Instant::now() >= deadline {
                return Err(format!("{name} first native snapshot deadline"));
            }
            frame_call(&self.queue, Box::new(|_| Ok(Value::Null)))?;
        }
        session.observer = name == "E2EObserver";
        if !session.observer {
            session.tested = true;
        }
        Ok(Value::Null)
    }

    fn probe(&self, expression: &str) -> Reply {
        let account = &self.session.as_ref().ok_or("no session")?.account;
        self.play.script_probe_for_test(account, expression)
    }

    fn evaluate(&mut self, request: &Value) -> Reply {
        let session = self.session.as_mut().ok_or("no session")?;
        let state = self.play.script_state(&session.account);
        append_logs(
            &mut session.logs,
            self.play.script_take_pending_logs(&session.account),
        )?;
        if !session.observer {
            session.error = self.play.script_last_error(&session.account);
            record_stop(
                &mut session.logs,
                &mut session.receipt,
                self.play.script_lifecycle_receipt(&session.account),
            )?;
        }
        let native_state;
        let state_name = if session.observer {
            if session.tested {
                "stopped"
            } else {
                "idle"
            }
        } else if matches!(state, script::RunState::Paused) {
            // Native runtime state distinguishes an operator pause from a scene-load hold.
            native_state = self.play.script_probe_for_test(
                &session.account,
                "globalThis.__rs2b0t_script_runtime('state')",
            )?;
            native_state
                .as_str()
                .ok_or("invalid native runtime state")?
        } else {
            match state {
                script::RunState::Running | script::RunState::Starting => "running",
                script::RunState::Stopping => "stopping",
                script::RunState::Error => "crashed",
                _ => "stopped",
            }
        };
        let error = &session.error;
        let receipt = &session.receipt;
        let status = json!({
            "state":state_name,
            "logs":session.logs.iter().map(RunnerLog::as_value).collect::<Vec<_>>(),
            "error":error,
            "receipt":receipt,
            "walk_cancelled_by_user": self.play.script_walk_cancelled_by_user(&session.account),
        });
        if state_name == "paused" || error.is_some() {
            session.last_issue = Some(status.clone());
        }
        if !session.observer && matches!(state, script::RunState::Idle | script::RunState::Stopping)
        {
            self.start_script("E2EObserver")?;
        }
        let expression = request["expression"].as_str().ok_or("missing expression")?;
        let source =
            format!("(() => {{ globalThis.__e2eStatus = {status}; return ({expression}); }})()");
        let result = self.probe(&source)?;
        let commands = self.probe("globalThis.__e2eCommands.splice(0)")?;
        for command in commands.as_array().ok_or("invalid facade commands")? {
            let account = self.session.as_ref().unwrap().account.clone();
            match command["op"].as_str() {
                Some("start") => {
                    self.start_script(command["name"].as_str().ok_or("missing card")?)?;
                }
                Some("stop") => self.play.script_stop(&account),
                Some("pause") => self.play.script_pause(&account),
                Some("resume") => self.play.script_resume(&account),
                _ => return Err("unsupported facade command".into()),
            }
        }
        Ok(result)
    }

    fn dispatch(&mut self, request: Value) -> Reply {
        match request["op"].as_str().ok_or("missing op")? {
            "await-terminal" => {
                let account = &self.session.as_ref().ok_or("no session")?.account;
                let milliseconds = request["timeout_ms"]
                    .as_u64()
                    .filter(|value| *value > 0)
                    .ok_or("native terminal wait requires a positive timeout")?;
                let deadline = Instant::now()
                    .checked_add(Duration::from_millis(milliseconds))
                    .ok_or("native terminal deadline overflow")?;
                eprintln!("upstream adapter: waiting for native terminal state: {account}");
                loop {
                    if let Some(error) = self.play.script_last_error(account) {
                        return Err(error);
                    }
                    match self.play.script_state(account) {
                        script::RunState::Idle => return Ok(Value::Null),
                        script::RunState::Error => {
                            return Err("native script ended in Error without a message".into());
                        }
                        _ => {}
                    }
                    if Instant::now() >= deadline {
                        return Err("native terminal state deadline".into());
                    }
                    frame_call(&self.queue, Box::new(|_| Ok(Value::Null)))?;
                }
            }
            "preflight" => {
                engine_preflight(&self.engine_ports)?;
                Ok(Value::Null)
            }
            "connect" => {
                if self.session.is_some() {
                    return Err("adapter supports one account per test".into());
                }
                let user = request["user"].as_str().ok_or("missing user")?;
                let nonce = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| e.to_string())?;
                let identity = format!("{user}:{}", nonce.as_nanos());
                let account = format!(
                    "uea{}",
                    &script_js::JsCache::origin_sha(identity.as_bytes())[..9]
                );
                let uid = nonce.subsec_nanos() as i32;
                self.play.try_spawn_slot(
                    Profile {
                        username: account.clone(),
                        password: "test".into(),
                        uid,
                        settings: ProfileSettings::default(),
                    },
                    None,
                    None,
                    None,
                )?;
                let settings = script::ScriptSettingsStore::at(
                    self.output.join(format!("{account}-settings.json")),
                )
                .map_err(|error| error.to_string())?;
                self.session = Some(Session {
                    account,
                    tested: false,
                    observer: false,
                    logs: Vec::new(),
                    receipt: None,
                    error: None,
                    settings,
                    last_issue: None,
                });
                self.start_script("E2EObserver")
            }
            "evaluate" => self.evaluate(&request),
            "settings" => {
                let name = request["name"].as_str().ok_or("missing script")?;
                let bag = request["bag"].as_object().ok_or("missing settings")?;
                let settings = &mut self.session.as_mut().ok_or("no session")?.settings;
                for (key, value) in bag {
                    settings.set_value(script::ScriptSource::Catalog, name, key, value.clone());
                }
                settings.save()?;
                Ok(Value::Null)
            }
            "start" => self.start_script(request["name"].as_str().ok_or("missing script")?),
            "status" => frame_call(
                &self.queue,
                Box::new(|client| {
                    Ok(json!({
                        "ingame":client.ingame, "scene":client.scene_state,
                        "inventoryTab":client.side_icon[3],
                    }))
                }),
            ),
            "cheat" => {
                let command = request["command"]
                    .as_str()
                    .ok_or("missing command")?
                    .to_string();
                if !account_cheat(&command) {
                    return Err(format!("unsupported account-scoped cheat: {command}"));
                }
                frame_call(
                    &self.queue,
                    Box::new(move |client| {
                        if !client.ingame {
                            return Err("cheat while offline".into());
                        }
                        api::interact::cheat(client, &command);
                        Ok(json!(true))
                    }),
                )
            }
            "logout" => frame_call(
                &self.queue,
                Box::new(|client| {
                    let ifaces = Arc::clone(&client.ifaces);
                    Ok(json!(api::interact::logout(client, &ifaces)))
                }),
            ),
            "login" => {
                self.play
                    .arm(&self.session.as_ref().ok_or("no session")?.account)
                    .ok_or("missing account arm")?
                    .arm_explicit_login();
                Ok(Value::Null)
            }
            "screenshot" => {
                let path = shot_path(
                    &self.output,
                    request["path"].as_str().ok_or("missing shot path")?,
                )?;
                frame_call(
                    &self.queue,
                    Box::new(move |client| {
                        let mut renderer =
                            client::render::Renderer::new_prefer(client.config.lowmem, false);
                        let was_draw = client.draw;
                        client.set_draw(true);
                        let frame = renderer.mainredraw(client);
                        client.set_draw(was_draw);
                        let client::render::backend::FrameOutput::PixMap(pixels) = frame else {
                            return Err("no CPU framebuffer".into());
                        };
                        std::fs::create_dir_all(path.parent().ok_or("shot parent")?)
                            .map_err(|e| e.to_string())?;
                        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
                        let mut encoder =
                            png::Encoder::new(file, pixels.width as u32, pixels.height as u32);
                        encoder.set_color(png::ColorType::Rgba);
                        encoder.set_depth(png::BitDepth::Eight);
                        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
                        let rgba: Vec<u8> = pixels
                            .pixels
                            .iter()
                            .flat_map(|pixel| {
                                [(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8, 255]
                            })
                            .collect();
                        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
                        Ok(json!({"path":path}))
                    }),
                )
            }
            "diagnostics" => {
                let Some(session) = self.session.as_mut() else {
                    return Ok(self.last_diagnostic.clone());
                };
                append_logs(
                    &mut session.logs,
                    self.play.script_take_pending_logs(&session.account),
                )?;
                let facts = self.play.script_probe_for_test(&session.account,
                    "({stats:globalThis.__rs2b0t_host?.snapshot?.stats, tile:rs2b0t.reader.worldTile()})");
                Ok(json!({
                    "account":session.account,
                    "state":format!("{:?}", self.play.script_state(&session.account)),
                    "observer":session.observer,
                    "error":self.play.script_last_error(&session.account).or_else(||session.error.clone()),
                    "receipt":self.play.script_lifecycle_receipt(&session.account).or_else(||session.receipt.clone()),
                    "logs":session.logs.iter().map(RunnerLog::as_value).collect::<Vec<_>>(),
                    "facts":facts,
                    "last_issue":session.last_issue,
                    "host_events":NATIVE_EVENTS.0.lock().unwrap().iter()
                        .filter(|row| row["account"].as_str() == Some(session.account.as_str()))
                        .collect::<Vec<_>>(),
                    "walk_cancelled_by_user": self.play.script_walk_cancelled_by_user(&session.account),
                }))
            }
            "close" => {
                if self.session.is_some() {
                    self.last_diagnostic = self.dispatch(json!({"op":"diagnostics"}))?;
                }
                if let Some(session) = self.session.take() {
                    self.play.script_stop(&session.account);
                    self.play.stop_slot(&session.account);
                    std::fs::write(
                        self.output.join(format!("{}-host.json", session.account)),
                        format!("{}\n", self.last_diagnostic),
                    )
                    .map_err(|e| e.to_string())?;
                }
                self.queue.lock().unwrap().clear();
                Ok(Value::Null)
            }
            op => Err(format!("unsupported adapter operation: {op}")),
        }
    }
}

fn selected_cases<'a>(
    metadata: &'a Value,
    requested: Option<&str>,
) -> Result<Vec<&'a Value>, String> {
    let cases = metadata["cases"]
        .as_array()
        .ok_or("missing upstream cases")?;
    let ids: Vec<_> = requested
        .into_iter()
        .flat_map(|value| value.split(','))
        .filter(|id| !id.is_empty())
        .collect();
    for id in &ids {
        if !cases.iter().any(|case| case["id"] == *id) {
            return Err(format!("unknown upstream case {id}"));
        }
    }
    let selected: Vec<_> = cases
        .iter()
        .filter(|case| {
            if ids.is_empty() {
                case["eligibility"] == "adopted"
            } else {
                ids.iter().any(|id| case["id"] == *id)
            }
        })
        .collect();
    if selected.is_empty() {
        return Err("no upstream cases selected".into());
    }
    Ok(selected)
}

fn validate_verdict(metadata: &Value, selected: &[&Value], verdict: &Value) -> Result<(), String> {
    for key in ["pin", "abi_pin"] {
        if verdict[key] != metadata[key] || !verdict[key].is_string() {
            return Err(format!("upstream verdict has the wrong {key}"));
        }
    }
    let commit = verdict["host_commit"]
        .as_str()
        .ok_or("missing host commit")?;
    if commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("upstream verdict requires a full host commit".into());
    }
    let rows = verdict["results"]
        .as_array()
        .ok_or("missing upstream results")?;
    if rows.len() != selected.len() {
        return Err("upstream verdict does not cover the selected cases".into());
    }
    let mut failures = Vec::new();
    for case in selected {
        let id = case["id"].as_str().ok_or("upstream case has no id")?;
        let mut matches = rows.iter().filter(|row| row["id"] == id);
        let row = matches
            .next()
            .ok_or_else(|| format!("missing upstream result for {id}"))?;
        if matches.next().is_some() {
            return Err(format!("duplicate upstream result for {id}"));
        }
        for key in [
            "path",
            "family",
            "capabilities",
            "deadline_seconds",
            "gate_class",
            "eligibility",
        ] {
            if row[key] != case[key] {
                return Err(format!("upstream verdict metadata differs for {id}: {key}"));
            }
        }
        if row["eligibility_reason"] != case["reason"] {
            return Err(format!(
                "upstream verdict eligibility reason differs for {id}"
            ));
        }
        match (case["gate_class"].as_str(), row["verdict"].as_str()) {
            (Some("gating" | "non-gating"), Some("PASS")) => {}
            (Some("non-gating"), Some("FAIL" | "UNSUPPORTED")) => {}
            (Some("gating"), Some("FAIL" | "UNSUPPORTED")) => {
                failures.push(format!("{id}: {} ({})", row["verdict"], row["reason"]));
            }
            _ => return Err(format!("invalid upstream verdict for {id}")),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

fn read_verdict(metadata: &Value, selected: &[&Value], output: &Path) -> Result<(), String> {
    let path = output.join("verdict.json");
    let verdict = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let verdict: Value = serde_json::from_slice(&verdict).map_err(|error| error.to_string())?;
    validate_verdict(metadata, selected, &verdict)
}

fn run_corpus() -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Err("requires LIVE=1".into());
    }
    let upstream = required("UEP_UPSTREAM")?;
    let output = required("UEP_OUTPUT")?;
    std::fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    if output.join("verdict.json").exists() {
        return Err("UEP_OUTPUT already contains verdict.json; use a new run directory".into());
    }
    let metadata: Value = serde_json::from_str(CASES).map_err(|error| error.to_string())?;
    let requested = std::env::var("UEP_CASES").ok();
    let selected = selected_cases(&metadata, requested.as_deref())?;
    let seconds = selected.iter().try_fold(60u64, |total, case| {
        let seconds = case["deadline_seconds"]
            .as_u64()
            .ok_or("invalid case deadline")?;
        if seconds == 0 {
            return Err("case deadline must be positive");
        }
        total.checked_add(seconds).ok_or("corpus deadline overflow")
    })?;
    if !api::hostlog::install_sink(&NATIVE_EVENTS) {
        return Err("upstream adapter requires its own host event sink".into());
    }
    let engine_ports = [
        engine_port(std::env::var("WORLD_GAME_PORT"), 44594)?,
        engine_port(std::env::var("WORLD_HTTP_PORT"), 1080)?,
    ];
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(engine_ports[0]),
        http_port: Some(engine_ports[1]),
        engine_dir: Some(required("ENGINE_A_DIR")?),
        nav_pack: Some(required("NAV_PACK_289")?),
        nav_flags: Some(required("NAV_FLAGS_289")?),
        vault_path: Some(output.join("vault")),
        catalog_root: Some(upstream.clone()),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind()?;
    let template = SharedClientTemplate::load(profile)?;
    let queue: FrameQueue = Arc::new(Mutex::new(VecDeque::new()));
    let frame_queue = Arc::clone(&queue);
    let play = host_play::run_with_template(
        template,
        true,
        vec![],
        |_| (None, None),
        move |client, _, _| {
            let pending = frame_queue.lock().unwrap().pop_front();
            if let Some((action, reply)) = pending {
                let _ = reply.send(action(client));
            }
        },
    )?;
    let mut library =
        script_js::JsLibrary::with_cache(output.join("scripts.json"), output.join("js-cache"));
    library.register_rs2b0t(&upstream, &output.join("catalog-path"))?;
    let mut adapter = Adapter {
        play,
        library,
        queue,
        engine_ports,
        session: None,
        last_diagnostic: Value::Null,
        output: output.clone(),
    };
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut stream) = connection else {
                break;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
            let mut line = String::new();
            if BufReader::new(&stream)
                .take(1024 * 1024)
                .read_line(&mut line)
                .is_err()
            {
                continue;
            }
            if line.trim() == "shutdown" {
                break;
            }
            let (reply_tx, reply_rx) = mpsc::channel();
            if tx.send((line, reply_tx)).is_err() {
                break;
            }
            let result = reply_rx
                .recv_timeout(Duration::from_secs(120))
                .unwrap_or_else(|e| Err(format!("adapter dispatch: {e}")));
            let response = match result {
                Ok(value) => json!({"ok":true,"value":value}),
                Err(error) => json!({"ok":false,"error":error}),
            };
            let _ = writeln!(stream, "{response}");
        }
    });
    let runner = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/upstream_e2e/runner.ts");
    let mut child = Command::new("bun")
        .arg(runner)
        .env("UEP_ENDPOINT", address.to_string())
        .env("UEP_UPSTREAM", &upstream)
        .env("UEP_OUTPUT", &output)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let result = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break read_verdict(&metadata, &selected, &output).and_then(|()| {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("upstream corpus exited {status}; see verdict.json"))
                }
            });
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break Err("upstream corpus deadline exceeded".into());
        }
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok((line, reply)) => {
                let result = serde_json::from_str(&line)
                    .map_err(|e| e.to_string())
                    .and_then(|request| adapter.dispatch(request));
                let _ = reply.send(result);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => break Err(error.to_string()),
        }
    };
    let _ = adapter.dispatch(json!({"op":"close"}));
    if let Ok(mut stream) = TcpStream::connect(address) {
        let _ = writeln!(stream, "shutdown");
    }
    server.join().map_err(|_| "RPC server panicked")?;
    result
}

#[test]
#[ignore = "requires LIVE=1, local Engine A, UEP_UPSTREAM and UEP_OUTPUT"]
fn upstream_e2e_corpus() {
    if let Err(error) = run_corpus() {
        panic!("{error}");
    }
}

#[test]
fn preflight_checks_both_real_loopback_endpoints_without_a_client() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    assert!(engine_preflight(&[port, port]).is_ok());
    for ports in [[port, 0], [0, port]] {
        let error = engine_preflight(&ports).unwrap_err();
        assert!(error.contains("127.0.0.1:0"), "{error}");
    }
}

#[test]
fn screenshots_remain_in_the_selected_directory() {
    let root = Path::new("evidence");
    assert_eq!(
        shot_path(root, "docs/e2e/proof.png").unwrap(),
        root.join("docs/e2e/proof.png")
    );
    assert!(shot_path(root, "../private.png").is_err());
    assert!(shot_path(root, "/private.png").is_err());
    assert!(shot_path(root, "docs/../../private.png").is_err());
}

#[test]
fn facade_runs_in_the_real_load_isolate_and_keeps_actions_explicit() {
    let isolate = script_js::LoadIsolate::spawn(
        format!("{FACADE}\n{OBSERVER}"),
        script_js::LoadShape::CompatClass,
        Vec::new(),
    )
    .unwrap();
    assert_eq!(isolate.probe("rs2b0t.runner.state").unwrap(), json!("idle"));
    assert_eq!(
        isolate
            .probe("rs2b0t.runner.start(rs2b0t.registry.get('Alcher')); __e2eCommands")
            .unwrap(),
        json!([{"op":"start","name":"Alcher"}])
    );
    assert_eq!(isolate.probe("rs2b0t.runner.state").unwrap(), json!("idle"));
    assert!(isolate.probe("rs2b0t.runner.start(null)").is_err());
    assert!(isolate.probe("__rs2b0t.Inventory.nonexistent()").is_err());
    isolate.join();
}

#[test]
fn fixture_cheats_cannot_mutate_the_world_or_tick_speed() {
    for command in [
        "tele 0,50,50,20,20",
        "setstat herblore 20",
        "advancestat thieving 50",
        "~clearinv",
        "~maxme",
        "givebank pot_empty 30",
    ] {
        assert!(account_cheat(command), "{command}");
    }
    for command in [
        "speed 200",
        "npcadd goblin",
        "locadd tree",
        "give coins 1\nspeed 200",
        "",
    ] {
        assert!(!account_cheat(command), "{command}");
    }
}

#[test]
fn runner_log_time_is_native_unix_epoch_milliseconds() {
    let log = RunnerLog::at(
        "banked 13 rings".into(),
        UNIX_EPOCH + Duration::from_millis(1_725_000_123_456),
    )
    .unwrap();
    assert_eq!(
        log.as_value(),
        json!({"time":1_725_000_123_456u64, "msg":"banked 13 rings"})
    );
}

#[test]
fn native_stop_receipts_reach_the_runner_log_once() {
    let mut logs = vec![RunnerLog::at("script output".into(), UNIX_EPOCH).unwrap()];
    let mut previous = None;
    let receipt = script::ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: script::ScriptTerminalState::Stopped,
        tick: 8,
        reason: "native stop reason".into(),
    };
    record_stop(&mut logs, &mut previous, Some(receipt.clone())).unwrap();
    record_stop(&mut logs, &mut previous, Some(receipt.clone())).unwrap();
    record_stop(&mut logs, &mut previous, None).unwrap();
    assert_eq!(
        logs.iter().map(|log| log.msg.as_str()).collect::<Vec<_>>(),
        ["script output", "stopped — native stop reason"]
    );
    assert_eq!(previous, Some(receipt));
}

#[test]
fn upstream_verdict_requires_complete_structured_results() {
    let metadata: Value = serde_json::from_str(CASES).unwrap();
    let case = metadata["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["eligibility"] == "adopted" && case["gate_class"] == "gating")
        .unwrap();
    let mut row = case.clone();
    row["eligibility_reason"] = case["reason"].clone();
    row["verdict"] = json!("PASS");
    row["reason"] = json!("unchanged upstream test passed");
    let mut verdict = json!({
        "pin":metadata["pin"], "abi_pin":metadata["abi_pin"],
        "host_commit":"0123456789012345678901234567890123456789", "results":[row]
    });
    assert!(validate_verdict(&metadata, &[case], &verdict).is_ok());
    verdict["results"][0]["verdict"] = json!("UNSUPPORTED");
    assert!(validate_verdict(&metadata, &[case], &verdict).is_err());
    verdict["results"][0]["verdict"] = json!("PASS");
    let duplicate = verdict["results"][0].clone();
    verdict["results"].as_array_mut().unwrap().push(duplicate);
    assert!(validate_verdict(&metadata, &[case], &verdict).is_err());
    verdict["results"] = json!([]);
    assert!(validate_verdict(&metadata, &[case], &verdict).is_err());
    assert!(selected_cases(&metadata, Some("not-an-upstream-case")).is_err());
}

#[test]
fn engine_ports_follow_the_cell_runner_without_falling_back_on_invalid_values() {
    assert_eq!(
        engine_port(Err(std::env::VarError::NotPresent), 44594),
        Ok(44594)
    );
    assert_eq!(engine_port(Ok("47594".into()), 44594), Ok(47594));
    for value in ["0", "65536", "bad"] {
        assert!(engine_port(Ok(value.into()), 44594).is_err());
    }
    assert!(engine_port(Err(std::env::VarError::NotUnicode("bad".into())), 44594).is_err());
}
