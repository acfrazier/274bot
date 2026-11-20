//! Test-only transport for unchanged upstream e2e files. No product shim changes.
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use host_play::{Play, ProfileOptions, SharedClientTemplate};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const FACADE: &str = include_str!("upstream_e2e/facade.js");
const OBSERVER: &str = "import { LoopingBot } from '#/bot/api/bot/Bot.js';\nexport default class E2EObserver extends LoopingBot { async loop() { return 1000; } }";
type Reply = Result<Value, String>;
type FrameAction = Box<dyn FnOnce(&mut client::client::Client) -> Reply + Send>;
type FrameQueue = Arc<Mutex<VecDeque<(FrameAction, mpsc::Sender<Reply>)>>>;

fn required(key: &str) -> Result<PathBuf, String> {
    std::env::var_os(key)
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing {key}"))
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
    logs: Vec<String>,
    settings: script::ScriptSettingsStore,
}

struct Adapter {
    play: Play,
    library: script_js::JsLibrary,
    queue: FrameQueue,
    session: Option<Session>,
    output: PathBuf,
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
        if name == "E2EObserver" {
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
                    return Err("observer first native snapshot deadline".into());
                }
                frame_call(&self.queue, Box::new(|_| Ok(Value::Null)))?;
            }
        }
        session.observer = name == "E2EObserver";
        if !session.observer {
            session.tested = true;
            session.logs.clear();
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
        session
            .logs
            .extend(self.play.script_take_pending_logs(&session.account));
        let state_name = if session.observer {
            if session.tested {
                "stopped"
            } else {
                "idle"
            }
        } else {
            match state {
                script::RunState::Running | script::RunState::Starting => "running",
                script::RunState::Paused => "paused",
                _ => "stopped",
            }
        };
        let status = json!({"state":state_name, "logs":session.logs.iter().map(|msg| json!({"msg":msg})).collect::<Vec<_>>()});
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
            "connect" => {
                if self.session.is_some() {
                    return Err("pilot supports one account per test".into());
                }
                let user = request["user"].as_str().ok_or("missing user")?;
                let account = format!(
                    "uep{}",
                    &script_js::JsCache::origin_sha(user.as_bytes())[..9]
                );
                let uid = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| e.to_string())?
                    .subsec_nanos() as i32;
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
                    settings,
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
                if command.starts_with("speed ") || command.starts_with("npcadd ") {
                    return Err("pilot refuses world and tick-speed changes".into());
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
            "close" => {
                if let Some(session) = self.session.take() {
                    self.play.script_stop(&session.account);
                    self.play.stop_slot(&session.account);
                }
                self.queue.lock().unwrap().clear();
                Ok(Value::Null)
            }
            op => Err(format!("unsupported adapter operation: {op}")),
        }
    }
}

fn run_pilot() -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Err("requires LIVE=1".into());
    }
    let upstream = required("UEP_UPSTREAM")?;
    let output = required("UEP_OUTPUT")?;
    std::fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(44594),
        http_port: Some(1080),
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
        session: None,
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
    let deadline = Instant::now() + Duration::from_secs(2700);
    let result = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break if status.success() {
                Ok(())
            } else {
                Err(format!("upstream pilot exited {status}"))
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break Err("pilot deadline exceeded".into());
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
fn upstream_e2e_pilot() {
    if let Err(error) = run_pilot() {
        panic!("{error}");
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
