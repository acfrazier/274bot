//! Test-only transport for unchanged upstream e2e files. No product shim changes.
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::loadout::Loadout;
use host_play::{Play, ProfileOptions, SharedClientTemplate};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const FACADE: &str = include_str!("upstream_e2e/facade.js");
const CASES: &str = include_str!("../../../tools/live-map/upstream-cases.json");
const ZANARIS_BANK: &str = "useZanarisBank";
const OBSERVER: &str = "import { LoopingBot } from '#/bot/api/bot/Bot.js';\nexport default class E2EObserver extends LoopingBot { async loop() { return 1000; } }";
type Reply = Result<Value, String>;
type FrameAction = Box<dyn FnOnce(&mut client::client::Client) -> Reply + Send>;
type FrameQueue = Arc<Mutex<VecDeque<(FrameAction, mpsc::Sender<Reply>)>>>;

struct EventCapture {
    account: Option<String>,
    rows: VecDeque<Value>,
}

struct NativeEvents(Mutex<EventCapture>);

impl NativeEvents {
    const fn new() -> Self {
        Self(Mutex::new(EventCapture {
            account: None,
            rows: VecDeque::new(),
        }))
    }

    fn capture(&self, account: Option<&str>) {
        let mut capture = self.0.lock().unwrap();
        capture.account = account.map(str::to_owned);
        capture.rows.clear();
    }

    fn snapshot(&self) -> Value {
        json!(self.0.lock().unwrap().rows)
    }
}

impl api::hostlog::Sink for NativeEvents {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        let Some(account) = record.slot else {
            return;
        };
        let mut capture = self.0.lock().unwrap();
        if capture.account.as_deref() != Some(account) {
            return;
        }
        if capture.rows.len() == 1024 {
            capture.rows.pop_front();
        }
        capture.rows.push_back(json!({
            "account":account, "tick":record.tick, "source":record.source.label(),
            "level":record.level.label(), "message":record.message,
        }));
    }

    fn test_nav_trace_enabled(&self) -> bool {
        true
    }
}

static NATIVE_EVENTS: NativeEvents = NativeEvents::new();

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
            if receipt.state == script::ScriptTerminalState::Stopped {
                logs.push(RunnerLog::received(format!(
                    "stopping — {}",
                    receipt.reason
                ))?);
            }
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

fn terminal_poll(
    state: script::RunState,
    read_terminal: impl FnOnce() -> (Option<script::ScriptLifecycleReceipt>, Option<String>),
) -> Result<bool, String> {
    use script::{RunState, ScriptTerminalState};

    if !matches!(state, RunState::Idle | RunState::Error) {
        return Ok(false);
    }
    let (receipt, error) = read_terminal();
    // The native slot also keeps the reason for a requested stop in last_error.
    if state == RunState::Idle
        && receipt
            .as_ref()
            .is_some_and(|receipt| receipt.state == ScriptTerminalState::Stopped)
    {
        return Ok(true);
    }
    if let Some(error) = error {
        return Err(error);
    }
    if let Some(receipt) = receipt.filter(|receipt| receipt.state == ScriptTerminalState::Failed) {
        return Err(receipt.reason);
    }
    if state == RunState::Error {
        return Err("native script ended in Error without a message".into());
    }
    Ok(true)
}

fn scene_status(
    frame: Value,
    observer_ready: impl FnOnce(u64) -> Reply,
) -> Result<Option<Value>, String> {
    if frame["ingame"] == true && frame["scene"] == 2 {
        // The isolate can still hold the prior session's materialized snapshot.
        let session = frame["session"].as_u64().ok_or("missing client session")?;
        let ready = observer_ready(session)?
            .as_bool()
            .ok_or("invalid observer session readiness")?;
        if !ready {
            return Ok(None);
        }
    }
    Ok(Some(frame))
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
    loadouts: Vec<Loadout>,
}

impl Session {
    fn new(account: String, output: &Path) -> Result<Self, String> {
        let settings =
            script::ScriptSettingsStore::at(output.join(format!("{account}-settings.json")))
                .map_err(|error| error.to_string())?;
        Ok(Self {
            account,
            tested: false,
            observer: false,
            logs: Vec::new(),
            receipt: None,
            error: None,
            last_issue: None,
            settings,
            loadouts: Vec::new(),
        })
    }

    fn set_settings(&mut self, name: &str, bag: &Map<String, Value>) -> Reply {
        for (key, value) in bag {
            let value = match (name, key.as_str()) {
                ("Loadouts", "sets") => {
                    let raw = value
                        .as_str()
                        .ok_or("Loadouts.sets must be a JSON string")?;
                    self.loadouts = serde_json::from_str(raw)
                        .map_err(|error| format!("Loadouts.sets: {error}"))?;
                    continue;
                }
                ("Global", ZANARIS_BANK) => Value::Bool(match value {
                    Value::Bool(enabled) => *enabled,
                    Value::Number(number) => number.as_f64() == Some(1.0),
                    Value::String(raw) => {
                        let raw = raw.trim();
                        raw == "1"
                            || raw.eq_ignore_ascii_case("true")
                            || raw.eq_ignore_ascii_case("yes")
                    }
                    _ => return Err(format!("Global.{ZANARIS_BANK} must be a scalar setting")),
                }),
                _ => value.clone(),
            };
            self.settings
                .set_value(script::ScriptSource::Catalog, name, key, value);
        }
        self.settings.save()?;
        Ok(Value::Null)
    }

    fn start_settings(&self, name: &str, schema: &[script::SettingDef]) -> Map<String, Value> {
        let source = script::ScriptSource::Catalog;
        let mut overrides = self.settings.overrides(source, name);
        // Native named banks read the card's run bag, not an account-wide toggle.
        if !overrides.contains_key(ZANARIS_BANK) {
            let mut global = self.settings.overrides(source, "Global");
            if let Some(opt_in) = global.swap_remove(ZANARIS_BANK) {
                overrides.insert(ZANARIS_BANK.into(), opt_in);
            }
        }
        script::merge_bag(schema, &overrides, None)
    }
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
            let settings = session.start_settings(name, &card.settings_schema);
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
        self.play.script_start_handle().start_load_with_loadouts(
            &session.account,
            js,
            checked,
            Some(settings),
            siblings,
            &session.loadouts,
        )?;
        self.play.wake(&session.account);
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

    fn await_terminal(&self, account: &str, deadline: Instant) -> Reply {
        loop {
            if terminal_poll(self.play.script_state(account), || {
                (
                    self.play.script_lifecycle_receipt(account),
                    self.play.script_last_error(account),
                )
            })? {
                return Ok(Value::Null);
            }
            if Instant::now() >= deadline {
                return Err("native terminal state deadline".into());
            }
            frame_call(&self.queue, Box::new(|_| Ok(Value::Null)))?;
        }
    }

    fn evaluate(&mut self, request: &Value) -> Reply {
        let session = self.session.as_ref().ok_or("no session")?;
        let mut state = self.play.script_state(&session.account);
        if !session.observer && state == script::RunState::Stopping {
            self.await_terminal(&session.account, Instant::now() + Duration::from_secs(20))?;
            state = self.play.script_state(&session.account);
        }
        let session = self.session.as_mut().ok_or("no session")?;
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
        let error = if (session.observer || state == script::RunState::Idle)
            && session
                .receipt
                .as_ref()
                .is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Stopped)
        {
            None
        } else {
            session.error.as_deref()
        };
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
        if !session.observer && state == script::RunState::Idle {
            terminal_poll(state, || (session.receipt.clone(), session.error.clone()))?;
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
                self.await_terminal(account, deadline)
            }
            "preflight" => {
                engine_preflight(&self.engine_ports)?;
                Ok(Value::Null)
            }
            "connect" => {
                if self.session.is_some() {
                    return Err("adapter supports one account per test".into());
                }
                request["user"].as_str().ok_or("missing user")?;
                let nonce = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| e.to_string())?;
                let account = host_play::mint_live_names(1)
                    .pop()
                    .expect("one live account was requested");
                let uid = nonce.subsec_nanos() as i32;
                let session = Session::new(account, &self.output)?;
                NATIVE_EVENTS.capture(Some(&session.account));
                if let Err(error) = self.play.try_spawn_slot(
                    Profile {
                        username: session.account.clone(),
                        password: "test".into(),
                        uid,
                        settings: ProfileSettings::default(),
                    },
                    None,
                    None,
                    None,
                ) {
                    NATIVE_EVENTS.capture(None);
                    return Err(error);
                }
                self.session = Some(session);
                self.start_script("E2EObserver")
            }
            "evaluate" => self.evaluate(&request),
            "settings" => {
                let name = request["name"].as_str().ok_or("missing script")?;
                let bag = request["bag"].as_object().ok_or("missing settings")?;
                self.session
                    .as_mut()
                    .ok_or("no session")?
                    .set_settings(name, bag)
            }
            "start" => self.start_script(request["name"].as_str().ok_or("missing script")?),
            "status" => {
                let deadline = Instant::now() + Duration::from_secs(20);
                loop {
                    let frame = frame_call(
                        &self.queue,
                        Box::new(|client| {
                            Ok(json!({
                                "ingame":client.ingame, "scene":client.scene_state,
                                "inventoryTab":client.side_icon[3],
                                "session":client.gens.session,
                            }))
                        }),
                    )?;
                    if let Some(status) = scene_status(frame, |session| {
                        self.probe(&format!("__e2eSessionReady({session})"))
                    })? {
                        return Ok(status);
                    }
                    if Instant::now() >= deadline {
                        return Err("native observer session snapshot deadline".into());
                    }
                }
            }
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
                    "host_events":NATIVE_EVENTS.snapshot(),
                    "walk_cancelled_by_user": self.play.script_walk_cancelled_by_user(&session.account),
                }))
            }
            "close" => {
                if self.session.is_some() {
                    self.last_diagnostic = self.dispatch(json!({"op":"diagnostics"}))?;
                }
                if let Some(session) = self.session.take() {
                    NATIVE_EVENTS.capture(None);
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
        false, // The upstream helpers own tutorial setup. A relog must not teleport.
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
fn fixture_settings_supply_named_operator_loadout_at_start() {
    let _home = script::IsolatedEnv::enter("upstream-loadout-fixture");
    let mut session = Session::new("alice".into(), &script::bot_home()).unwrap();
    let loadouts = json!({
        "sets": r#"[{"name":"Melee","worn":{"righthand":"Rune scimitar"},"carry":[{"item":"Lobster","qty":8}]}]"#
    });
    session
        .set_settings("Loadouts", loadouts.as_object().unwrap())
        .unwrap();
    session
        .set_settings(
            "ArdyFighter",
            json!({"loadout":"Melee"}).as_object().unwrap(),
        )
        .unwrap();
    let settings = session.start_settings("ArdyFighter", &[]);
    let selected = script::loadouts_store::resolve_compat_script_loadout_setting(
        &session.loadouts,
        settings["loadout"].as_str(),
    )
    .unwrap();
    assert_eq!(
        selected,
        Some(
            &Loadout::new("Melee")
                .with_slot("righthand", "Rune scimitar")
                .with_carry("Lobster", 8)
        )
    );
    for invalid in [json!([]), json!("[{}]")] {
        assert!(session
            .set_settings("Loadouts", json!({"sets":invalid}).as_object().unwrap())
            .is_err());
        assert_eq!(session.loadouts.len(), 1);
    }
    session
        .set_settings("Loadouts", json!({"sets":"[]"}).as_object().unwrap())
        .unwrap();
    assert!(session.loadouts.is_empty());
}

#[test]
fn fixture_settings_supply_zanaris_preference_at_start() {
    let _home = script::IsolatedEnv::enter("upstream-zanaris-fixture");
    let mut session = Session::new("alice".into(), &script::bot_home()).unwrap();
    session
        .set_settings(
            "Global",
            json!({"useZanarisBank":true,"useMageBank":true})
                .as_object()
                .unwrap(),
        )
        .unwrap();
    session
        .set_settings(
            "CookBot",
            json!({"location":"Zanaris"}).as_object().unwrap(),
        )
        .unwrap();
    let settings = session.start_settings("CookBot", &[]);
    assert_eq!(settings.get("useZanarisBank"), Some(&Value::Bool(true)));
    assert_eq!(settings["location"], "Zanaris");
    assert!(!settings.contains_key("useMageBank"));

    assert!(session
        .set_settings(
            "Global",
            json!({"useZanarisBank":null}).as_object().unwrap()
        )
        .is_err());
    for (raw, enabled) in [
        (json!(1), true),
        (json!(" Yes "), true),
        (json!(false), false),
    ] {
        session
            .set_settings("Global", json!({"useZanarisBank":raw}).as_object().unwrap())
            .unwrap();
        assert_eq!(
            session.start_settings("CookBot", &[])["useZanarisBank"],
            enabled
        );
    }
    session
        .set_settings(
            "Global",
            json!({"useZanarisBank":true}).as_object().unwrap(),
        )
        .unwrap();

    session
        .set_settings(
            "CookBot",
            json!({"useZanarisBank":false}).as_object().unwrap(),
        )
        .unwrap();
    assert_eq!(
        session.start_settings("CookBot", &[])["useZanarisBank"],
        false
    );
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
fn facade_reads_bank_trips_from_the_running_compat_instance() {
    let source = format!(
        "{FACADE}\n\
         import {{ LoopingBot }} from '#/bot/api/bot/Bot.js';\n\
         export default class TripCounter extends LoopingBot {{\n\
             bankTrips = 0;\n\
             countBankTrip() {{ this.bankTrips++; }}\n\
             async loop() {{ return 1000; }}\n\
         }}"
    );
    let isolate =
        script_js::LoadIsolate::spawn(source, script_js::LoadShape::CompatClass, Vec::new())
            .unwrap();
    let predicate = "(rs2b0t.runner.bot?.bankTrips ?? 0) > 0";
    isolate.probe("__e2eStatus.state = 'running'").unwrap();
    assert_eq!(isolate.probe(predicate).unwrap(), false);
    isolate.probe("__rs_bot.countBankTrip()").unwrap();
    assert_eq!(isolate.probe(predicate).unwrap(), true);
    assert_eq!(
        isolate.probe("rs2b0t.runner.bot === __rs_bot").unwrap(),
        true
    );
    isolate.probe("__e2eStatus.state = 'stopped'").unwrap();
    assert_eq!(isolate.probe("rs2b0t.runner.bot === null").unwrap(), true);
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
fn native_host_events_follow_the_active_session_account() {
    use api::hostlog::{Level, Record, Sink, Source};

    let events = NativeEvents::new();
    let account = "cp21abc123_0";
    events.capture(Some(account));
    let record = |slot| Record {
        slot,
        tick: Some(18),
        source: Source::Script,
        level: Level::Info,
        message: "script requested stop",
    };
    events.record(&record(None));
    events.record(&record(Some("cp21def456_0")));
    events.record(&record(Some(account)));
    assert_eq!(
        events.snapshot(),
        json!([{"account":account, "tick":18, "source":"script",
                "level":"info", "message":"script requested stop"}])
    );

    events.capture(Some("cp21def456_0"));
    events.record(&record(Some(account)));
    assert_eq!(events.snapshot(), json!([]));

    events.capture(None);
    events.record(&record(Some("cp21def456_0")));
    assert_eq!(events.snapshot(), json!([]));
}

#[test]
fn native_stop_receipts_reach_the_runner_log_in_order_once() {
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
        [
            "script output",
            "stopping — native stop reason",
            "stopped — native stop reason"
        ]
    );
    assert_eq!(previous, Some(receipt));
}

#[test]
fn await_terminal_waits_for_reap_after_script_requested_stop() {
    use script::{RunState, ScriptLifecycleReceipt, ScriptTerminalState};

    let receipt = ScriptLifecycleReceipt {
        runtime_generation: 1,
        state: ScriptTerminalState::Stopped,
        tick: 18,
        reason: "no Logs left in the bank".into(),
    };
    let diagnostic = "script requested stop on tick 18: no Logs left in the bank";
    assert_eq!(
        terminal_poll(RunState::Stopping, || (
            Some(receipt.clone()),
            Some(diagnostic.into())
        )),
        Ok(false)
    );
    assert_eq!(
        terminal_poll(RunState::Idle, || (Some(receipt), Some(diagnostic.into()))),
        Ok(true)
    );
}

#[test]
fn await_terminal_does_not_hide_a_native_failure() {
    use script::{RunState, ScriptLifecycleReceipt, ScriptTerminalState};

    assert_eq!(
        terminal_poll(RunState::Error, || (None, Some("runtime failure".into()))),
        Err("runtime failure".into())
    );
    assert!(terminal_poll(RunState::Error, || (None, None)).is_err());
    assert_eq!(
        terminal_poll(RunState::Idle, || (None, Some("setup failure".into()))),
        Err("setup failure".into())
    );
    assert_eq!(
        terminal_poll(RunState::Idle, || (
            Some(ScriptLifecycleReceipt {
                runtime_generation: 1,
                state: ScriptTerminalState::Failed,
                tick: 18,
                reason: "runtime failure".into(),
            }),
            None,
        )),
        Err("runtime failure".into())
    );
}

#[test]
fn relog_scene_ready_waits_for_observer_stats() {
    let isolate = script_js::LoadIsolate::spawn(
        format!("{FACADE}\n{OBSERVER}"),
        script_js::LoadShape::CompatClass,
        Vec::new(),
    )
    .unwrap();
    let frame = json!({"ingame":true, "scene":2, "inventoryTab":149, "session":2});
    let ready = |session| isolate.probe(&format!("__e2eSessionReady({session})"));
    isolate
        .probe(
            "__rs2b0t_host.snapshot = {ingame:true, scene_state:2, client_adapter:{session_generation:2}, stats_ready:false, stats:[]}; true",
        )
        .unwrap();
    assert!(isolate.probe("__rs2b0t.Skills.xp('ranged')").is_err());
    assert_eq!(scene_status(frame.clone(), ready).unwrap(), None);

    isolate
        .probe("__rs2b0t_host.snapshot.stats = [{name:'ranged', xp:83}]; true")
        .unwrap();
    assert_eq!(scene_status(frame.clone(), ready).unwrap(), Some(frame));
    // A ready snapshot from the prior login must not release the next relog.
    let next_frame = json!({"ingame":true, "scene":2, "inventoryTab":149, "session":3});
    assert_eq!(scene_status(next_frame, ready).unwrap(), None);
    assert_eq!(isolate.probe("__rs2b0t.Skills.xp('ranged')").unwrap(), 83);
    isolate.join();
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
