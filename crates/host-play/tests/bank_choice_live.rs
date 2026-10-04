//! Ignored LIVE=1 proof of native Quester Cook's authored-bank ranking from
//! Ardougne and Catherby. The fixture reuses the production `quester_cook`
//! scenario's seed/Start path, changes only its pre-Start stand to the tested
//! origin, and captures real CpuPix3D frames with matching JSON receipts.
//!
//! Run against the shared local 289 engine with a throwaway HOME, for example:
//! `env HOME="$(mktemp -d)" LIVE=1 BOT_LIVE_NAME_PREFIX=bc BOT_CPU=1 BOT_NAV_BUILD=skip WORLD_NAV_PACK=/absolute/path/to/274bot.navpack WORLD_ENGINE_DIR=/absolute/path/to/engine RS2B0T=/absolute/path/to/rs2b0t BOT_CACHE_DIR=/absolute/path/to/cache LIVE_EVIDENCE_DIR=/Volumes/dev-scratch/274bot-evidence/BANK-CHOICE-1 cargo test --locked -p host-play --features live-harness --test bank_choice_live -- --ignored --nocapture --test-threads=1`

#![cfg(feature = "live-harness")]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::{GameSnapshot, WorldTile};
use host::Pump;
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate};
use scenario::{Proof, RunnerStatus, Scenario, ScenarioRunner, Step, StepKind, Wait};
use script::native::{NativePhase, ScriptStatus, StatusValue};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const CELL: &str = "bank_choice_cook_local";
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const BANK_ARRIVAL_DEADLINE: Duration = Duration::from_secs(300);
const COOK_ONWARD_DEADLINE: Duration = Duration::from_secs(120);
const BANK_RADIUS: i32 = 2;

#[derive(Debug, Clone, Copy)]
struct Origin {
    name: &'static str,
    start: WorldTile,
    bank: WorldTile,
    bank_name: &'static str,
}

const ORIGINS: &[Origin] = &[
    Origin {
        name: "ardougne",
        start: WorldTile {
            x: 2663,
            z: 3302,
            level: 0,
        },
        bank: WorldTile {
            x: 2655,
            z: 3283,
            level: 0,
        },
        bank_name: "Ardougne East",
    },
    Origin {
        name: "catherby",
        start: WorldTile {
            x: 2840,
            z: 3436,
            level: 0,
        },
        bank: WorldTile {
            x: 2809,
            z: 3441,
            level: 0,
        },
        bank_name: "Catherby",
    },
];

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Result<Self, String> {
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("274bot-bank-choice-{label}-{serial}"));
        std::fs::create_dir_all(&path)
            .map_err(|error| format!("create {}: {error}", path.display()))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required for {CELL} live"))
}

fn required_path(name: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(required(name)?);
    if !path.is_absolute() {
        return Err(format!(
            "{name} must be an absolute path: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn validate_inputs(
    nav_pack: &Path,
    engine_dir: &Path,
    catalog_root: &Path,
    cache_dir: &Path,
    evidence_root: &Path,
    isolated_home: &Path,
) -> Result<(), String> {
    for (name, path, is_file) in [
        ("WORLD_NAV_PACK", nav_pack, true),
        ("WORLD_ENGINE_DIR", engine_dir, false),
        ("RS2B0T", catalog_root, false),
        ("BOT_CACHE_DIR", cache_dir, false),
    ] {
        if is_file && !path.is_file() {
            return Err(format!("{name} is not a file: {}", path.display()));
        }
        if !is_file && !path.is_dir() {
            return Err(format!("{name} is not a directory: {}", path.display()));
        }
    }
    if evidence_root.starts_with(isolated_home) {
        return Err("LIVE_EVIDENCE_DIR must be outside the throwaway HOME".into());
    }
    std::fs::create_dir_all(evidence_root)
        .map_err(|error| format!("create evidence root {}: {error}", evidence_root.display()))
}

fn selected_profile(
    nav_pack: PathBuf,
    engine_dir: PathBuf,
    catalog_root: PathBuf,
    cache_dir: PathBuf,
    temp: &Path,
) -> Result<(Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>), String> {
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(
            std::env::var("WORLD_GAME_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(45594),
        ),
        http_port: Some(
            std::env::var("WORLD_HTTP_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(2080),
        ),
        cache_dir: Some(cache_dir),
        nav_pack: Some(nav_pack),
        engine_dir: Some(engine_dir),
        unpack_dir: Some(temp.join("unpack")),
        vault_path: Some(temp.join("vault")),
        catalog_root: Some(catalog_root),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind_runtime()?;
    if profile.profile_class() != host_play::ProfileClass::Local
        || profile.client().game_host() != "127.0.0.1"
    {
        return Err(format!("{CELL} requires a loopback local profile"));
    }
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    if template.world().is_none() {
        return Err("selected template has no navigation world".into());
    }
    Ok((profile, template))
}

fn tile_json(tile: Option<WorldTile>) -> Value {
    tile.map_or(
        Value::Null,
        |tile| json!({"x": tile.x, "z": tile.z, "level": tile.level}),
    )
}

fn observed_tile(snapshot: &GameSnapshot) -> Option<WorldTile> {
    snapshot
        .tile()
        .map(|(x, z, level)| WorldTile { x, z, level })
}

fn tile_distance(tile: WorldTile, target: WorldTile) -> Option<i32> {
    (tile.level == target.level).then(|| (tile.x - target.x).abs().max((tile.z - target.z).abs()))
}

fn text_field<'a>(status: &'a ScriptStatus, key: &str) -> Option<&'a str> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            StatusValue::Text(value) => Some(value.as_ref()),
            _ => None,
        })
}

fn status_json(status: &ScriptStatus) -> Value {
    let fields: Map<String, Value> = status
        .fields
        .iter()
        .map(|field| {
            let value = match &field.value {
                StatusValue::Text(value) => json!(value.as_ref()),
                StatusValue::Integer(value) => json!(value),
                StatusValue::Tile(tile) => {
                    json!({"x": tile.x, "z": tile.z, "level": tile.level})
                }
                StatusValue::Truth(value) => json!(format!("{value:?}")),
                StatusValue::Quest(value) => json!(format!("{value:?}")),
            };
            (field.key.to_owned(), value)
        })
        .collect();
    json!({
        "card": format!("{:?}", status.card),
        "phase": format!("{:?}", status.phase),
        "active_settings": status.active_settings,
        "pending_settings": status.pending_settings,
        "fields": fields,
        "failure": status.failure.as_ref().map(|failure| format!("{failure:?}")),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusKey {
    phase: String,
    quest_id: Option<String>,
    action_state: Option<String>,
    step_id: Option<String>,
    provision: Option<String>,
    sequence: Option<String>,
    step_index: Option<String>,
}

fn status_key(status: &ScriptStatus) -> StatusKey {
    let integer = |key: &str| {
        status
            .fields
            .iter()
            .find(|field| field.key == key)
            .and_then(|field| match &field.value {
                StatusValue::Integer(value) => Some(value.to_string()),
                _ => None,
            })
    };
    StatusKey {
        phase: format!("{:?}", status.phase),
        quest_id: text_field(status, "quest_id").map(str::to_owned),
        action_state: text_field(status, "action_state").map(str::to_owned),
        step_id: text_field(status, "step_id").map(str::to_owned),
        provision: text_field(status, "provision").map(str::to_owned),
        sequence: integer("sequence"),
        step_index: integer("step_index"),
    }
}

struct CaptureRequest {
    label: &'static str,
    receipt: Value,
}

struct BankChoiceState {
    runner: ScenarioRunner,
    account: String,
    origin: Origin,
    start_settings: Map<String, Value>,
    evidence_dir: PathBuf,
    started_at: Instant,
    snapshot: GameSnapshot,
    pump: Pump,
    start_handle: Option<ScriptStartHandle>,
    start_count: u32,
    start_receipt: Option<Value>,
    arrival: Option<Value>,
    arrival_at: Option<Instant>,
    arrival_status_key: Option<StatusKey>,
    arrival_status: Option<Value>,
    after_bank_outcome: Option<Value>,
    pending_capture: Option<CaptureRequest>,
    capture_paths: Vec<(String, String)>,
    capture_error: Option<String>,
    latest_status: Option<Arc<ScriptStatus>>,
    latest_status_key: Option<StatusKey>,
    native_status_history: Vec<Value>,
    position_trace: Vec<Value>,
    last_trace_tile: Option<WorldTile>,
    last_trace_at: Instant,
    error: Option<String>,
}

impl BankChoiceState {
    fn snapshot_receipt(&self, stage: &str) -> Value {
        let tile = observed_tile(&self.snapshot);
        let local_player = self.snapshot.local_player();
        let bank_open = self.snapshot.bank_component_id() >= 0;
        let inventory = self.snapshot.inventory();
        let mut raw = json!({
            "phase": stage,
            "cell": CELL,
            "origin": self.origin.name,
            "start": tile_json(Some(self.origin.start)),
            "expected_bank": {"name": self.origin.bank_name, "tile": tile_json(Some(self.origin.bank))},
            "observed_tile": tile_json(tile),
            "combat_level": local_player.map(|local| local.player.combat_level),
            "inventory_size": self.snapshot.inventory_size(),
            "inventory_entry_count": inventory.len(),
            "inventory_empty": inventory.is_empty(),
            "bank_open": bank_open,
            "bank_session_generation": self.snapshot.bank_session_generation(),
            "start_count": self.start_count,
            "start_settings": self.start_settings,
            "native_status": self.latest_status.as_deref().map(status_json),
            "elapsed_ms": self.started_at.elapsed().as_millis(),
        });
        if let Some(arrival) = self.arrival.as_ref() {
            raw["bank_arrival"] = arrival.clone();
        }
        if let Some(outcome) = self.after_bank_outcome.as_ref() {
            raw["after_bank_outcome"] = outcome.clone();
        }
        raw
    }

    fn save_capture(
        &mut self,
        client: &mut client::client::Client,
        label: &'static str,
        receipt: Value,
    ) {
        let result = save_live_capture(
            client,
            &self.evidence_dir,
            label,
            (self.capture_paths.len() + 1) as u32,
            receipt,
        );
        match result {
            Ok(path) => self
                .capture_paths
                .push((label.to_owned(), path.display().to_string())),
            Err(error) => self.capture_error = Some(error),
        }
    }

    fn queue_capture(&mut self, label: &'static str, stage: &str) {
        if self.pending_capture.is_none()
            && !self
                .capture_paths
                .iter()
                .any(|(captured, _)| captured == label)
        {
            self.pending_capture = Some(CaptureRequest {
                label,
                receipt: self.snapshot_receipt(stage),
            });
        }
    }

    fn record_after_bank(&mut self, kind: &str, status: Option<&ScriptStatus>) {
        if self.after_bank_outcome.is_some() {
            return;
        }
        let detail = status.and_then(|status| {
            text_field(status, "block_reason")
                .or_else(|| text_field(status, "last_failure"))
                .map(str::to_owned)
        });
        let possible_danger_refusal = detail.as_ref().is_some_and(|message| {
            let lower = message.to_ascii_lowercase();
            lower.contains("zone") || lower.contains("danger") || lower.contains("wilderness")
        });
        self.after_bank_outcome = Some(json!({
            "kind": kind,
            "elapsed_ms": self.started_at.elapsed().as_millis(),
            "observed_tile": tile_json(observed_tile(&self.snapshot)),
            "bank_open": self.snapshot.bank_component_id() >= 0,
            "native_status": status.map(status_json),
            "native_failure": status.and_then(|status| status.failure.as_ref().map(|failure| format!("{failure:?}"))),
            "block_detail": detail,
            "possible_danger_zone_refusal": possible_danger_refusal,
        }));
        self.queue_capture("03-after-bank", "after-bank-outcome");
    }

    fn observe_status(&mut self, status: Option<Arc<ScriptStatus>>) {
        let Some(status) = status else {
            return;
        };
        if status.card != script::CompiledId("Quester") {
            self.error = Some(format!(
                "native status belongs to {:?}, not Quester",
                status.card
            ));
            return;
        }
        let key = status_key(&status);
        if self.latest_status_key.as_ref() != Some(&key) {
            self.native_status_history.push(status_json(&status));
            if self.native_status_history.len() > 64 {
                self.native_status_history.remove(0);
            }
            self.latest_status_key = Some(key.clone());
        }
        self.latest_status = Some(status.clone());
        if status.phase == NativePhase::Blocked && self.arrival.is_none() {
            self.error = Some(format!(
                "Cook blocked before the local-bank arrival gate: {:?}",
                status.failure
            ));
            return;
        }
        if self.arrival.is_none() || text_field(&status, "quest_id") != Some("cook") {
            return;
        }
        if self.arrival_status_key.is_none() {
            self.arrival_status_key = Some(key.clone());
            self.arrival_status = Some(status_json(&status));
            if matches!(status.phase, NativePhase::Blocked | NativePhase::Complete) {
                self.record_after_bank(
                    "native terminal phase observed after local bank arrival",
                    Some(&status),
                );
            }
            return;
        }
        if status.phase == NativePhase::Blocked {
            self.record_after_bank(
                "native blocked outcome after local bank arrival",
                Some(&status),
            );
        } else if status.phase == NativePhase::Complete {
            self.record_after_bank("Cook completed after local bank arrival", Some(&status));
        }
    }

    fn start_ready(&self) -> bool {
        observed_tile(&self.snapshot) == Some(self.origin.start)
            && self
                .snapshot
                .local_player()
                .is_some_and(|local| local.player.combat_level == 3)
            && self.snapshot.inventory_size() > 0
            && self.snapshot.inventory().is_empty()
            && self.snapshot.bank_component_id() < 0
    }

    fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        if let Some(tile) = observed_tile(&self.snapshot) {
            let now = Instant::now();
            if self.last_trace_tile != Some(tile)
                || now.duration_since(self.last_trace_at) >= Duration::from_secs(1)
            {
                self.position_trace.push(json!({
                    "elapsed_ms": self.started_at.elapsed().as_millis(),
                    "tile": tile_json(Some(tile)),
                    "bank_open": self.snapshot.bank_component_id() >= 0,
                    "native_status": self.latest_status.as_deref().map(status_json),
                }));
                self.last_trace_tile = Some(tile);
                self.last_trace_at = now;
            }
        }
        if !client.ingame || client.scene_state != 2 {
            return;
        }

        if self.runner.on_start_script() && self.start_count == 0 {
            if !self.start_ready() {
                self.runner.tick_with_hold(client, true);
                return;
            }
            let receipt = self.snapshot_receipt("start");
            self.save_capture(client, "01-start", receipt.clone());
            if let Some(error) = self.capture_error.as_ref() {
                self.error = Some(format!("start evidence capture failed: {error}"));
                return;
            }
            self.start_receipt = Some(receipt);
            let Some(handle) = self.start_handle.as_ref() else {
                self.error = Some("Quester Start reached before ScriptStartHandle install".into());
                return;
            };
            match handle.start_compiled(
                &self.account,
                script::CompiledId("Quester"),
                self.start_settings.clone(),
            ) {
                Ok(()) => self.start_count = 1,
                Err(error) => {
                    self.error = Some(format!("Quester compiled Start failed: {error}"));
                    return;
                }
            }
        }

        let tile = observed_tile(&self.snapshot);
        let bank_open = self.snapshot.bank_component_id() >= 0;
        let cook_active = self
            .latest_status
            .as_ref()
            .is_some_and(|status| text_field(status, "quest_id") == Some("cook"));
        if self.start_count == 1
            && self.arrival.is_none()
            && cook_active
            && bank_open
            && tile.is_some_and(|tile| {
                tile_distance(tile, self.origin.bank)
                    .is_some_and(|distance| distance <= BANK_RADIUS)
            })
        {
            let tile = tile.expect("checked above");
            self.arrival_at = Some(Instant::now());
            self.arrival = Some(json!({
                "elapsed_ms": self.started_at.elapsed().as_millis(),
                "expected_bank": self.origin.bank_name,
                "expected_bank_tile": tile_json(Some(self.origin.bank)),
                "observed_tile": tile_json(Some(tile)),
                "distance": tile_distance(tile, self.origin.bank),
                "bank_open": true,
                "bank_session_generation": self.snapshot.bank_session_generation(),
                "native_status": self.latest_status.as_deref().map(status_json),
                "gate": "live Quester Cook status + observed player at expected local bank + open bank session",
            }));
            self.queue_capture("02-bank-arrival", "bank-arrival");
        }

        if self.arrival.is_some() && self.after_bank_outcome.is_none() {
            let status = self.latest_status.clone();
            let left_bank = tile.is_some_and(|tile| {
                tile_distance(tile, self.origin.bank).is_none_or(|distance| distance > BANK_RADIUS)
            });
            if left_bank {
                self.record_after_bank(
                    "player physically left the selected local bank",
                    status.as_deref(),
                );
            } else if let Some(status) = status.as_deref() {
                if status.phase == NativePhase::Blocked {
                    self.record_after_bank(
                        "native blocked outcome after local bank arrival",
                        Some(status),
                    );
                } else if status.phase == NativePhase::Complete {
                    self.record_after_bank("Cook completed after local bank arrival", Some(status));
                }
            }
        }

        if let Some(request) = self.pending_capture.take() {
            self.save_capture(client, request.label, request.receipt);
            if let Some(error) = self.capture_error.as_ref() {
                self.error = Some(format!(
                    "{} evidence capture failed: {error}",
                    request.label
                ));
                return;
            }
        }

        if !matches!(
            self.runner.status(),
            RunnerStatus::Passed | RunnerStatus::Failed(_)
        ) {
            self.runner.tick_with_hold(client, hold);
        }
    }

    fn receipt(&self, run_state: script::RunState, lifecycle: Option<String>) -> Value {
        json!({
            "cell": CELL,
            "origin": self.origin.name,
            "account": self.account,
            "card": "Quester",
            "authored_quest": "cook",
            "start_settings": self.start_settings,
            "start": self.start_receipt,
            "bank_arrival": self.arrival,
            "arrival_native_status": self.arrival_status,
            "after_bank_outcome": self.after_bank_outcome,
            "native_status_history": self.native_status_history,
            "position_trace": self.position_trace,
            "start_count": self.start_count,
            "run_state": format!("{run_state:?}"),
            "lifecycle": lifecycle,
            "scenario_status": format!("{:?}", self.runner.status()),
            "captures": self.capture_paths.iter().map(|(step, path)| json!({"step": step, "path": path})).collect::<Vec<_>>(),
            "capture_error": self.capture_error,
            "observed_quest_completion_required": false,
        })
    }
}

fn make_cook_scenario(origin: Origin) -> Result<(Scenario, Map<String, Value>), String> {
    let mut scenario =
        scenario::get("quester_cook").ok_or("scenario registry has no quester_cook")?;
    if scenario.name != "quester_cook" || scenario.settings.start_script != Some("Quester") {
        return Err(format!(
            "quester_cook must start Quester, got name={} start={:?}",
            scenario.name, scenario.settings.start_script
        ));
    }
    let settings = scenario::settings_inject_map(scenario.settings.script_settings_inject)
        .ok_or("quester_cook has no compiled Start settings")?;
    if settings.len() != 1 || settings.get("quests") != Some(&json!(["cook"])) {
        return Err(format!(
            "quester_cook must select only the authored Cook Path, got {settings:?}"
        ));
    }

    let reset_index = scenario
        .steps
        .iter()
        .position(|step| step.name == "reset quest stage")
        .ok_or("quester_cook scenario has no reset quest stage")?;
    scenario.steps.insert(
        reset_index,
        Step {
            name: "reset minted account to combat 3",
            kind: StepKind::Perform {
                send: Box::new(|client, _| interact::cheat(client, "minme").is_sent()),
            },
            wait: Wait {
                arm: Proof::Stat { id: 3, min: 10 },
                budget_ticks: 80,
            },
        },
    );
    let stand_index = scenario
        .steps
        .iter()
        .position(|step| step.name == "stand at the quest start")
        .ok_or("quester_cook scenario has no authored start stand")?;
    let mut stand = scenario.steps.remove(stand_index);
    stand.kind = StepKind::Perform {
        send: Box::new(move |client, _| {
            interact::cheat(
                client,
                &interact::tele_args(origin.start.level, origin.start.x, origin.start.z),
            )
            .is_sent()
        }),
    };
    stand.wait.arm = Proof::Arrived {
        x: origin.start.x,
        z: origin.start.z,
        level: origin.start.level,
    };
    // Relog triggers the host's mainland hop. Establish the tested origin
    // only after that relog, so Start cannot be followed by a Lumbridge reset.
    let start_index = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .ok_or("quester_cook scenario has no compiled Start step")?;
    scenario.steps.insert(start_index, stand);
    scenario.settings.nav.engine_speed_ms = None;
    Ok((scenario, settings))
}

fn mint_profile(account: &str, password: &str, offset: i32) -> Result<Profile, String> {
    let uid = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("clock: {error}"))?
        .as_millis()
        .checked_rem(i32::MAX as u128)
        .ok_or("uid clock overflow")? as i32;
    Ok(Profile {
        username: account.to_owned(),
        password: password.to_owned().into(),
        uid: uid.saturating_add(offset),
        settings: ProfileSettings::default(),
    })
}

fn save_live_capture(
    client: &mut client::client::Client,
    directory: &Path,
    label: &str,
    sequence: u32,
    receipt: Value,
) -> Result<PathBuf, String> {
    if !client.ingame || client.scene_state != 2 {
        return Err(format!(
            "capture {label} requires ingame && scene_state == 2, got ingame={} scene_state={}",
            client.ingame, client.scene_state
        ));
    }
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();
    let stem = format!("{epoch_ms}Z_{sequence:02}-{label}");
    let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
    let was_draw = client.draw;
    client.set_draw(true);
    let frame = renderer.mainredraw(client);
    client.set_draw(was_draw);
    let client::render::backend::FrameOutput::PixMap(pixels) = frame else {
        return Err("live capture did not return a CPU PixMap; run with BOT_CPU=1".into());
    };
    let mut rgba = Vec::with_capacity(pixels.pixels.len() * 4);
    for pixel in &pixels.pixels {
        rgba.extend_from_slice(&[
            ((pixel >> 16) & 0xff) as u8,
            ((pixel >> 8) & 0xff) as u8,
            (pixel & 0xff) as u8,
            u8::MAX,
        ]);
    }
    let png_path = directory.join(format!("{stem}.png"));
    let file = std::fs::File::create(&png_path).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(file, pixels.width as u32, pixels.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .map_err(|error| error.to_string())?
        .write_image_data(&rgba)
        .map_err(|error| error.to_string())?;
    let mut receipt = receipt;
    receipt["frame"] = json!({
        "ingame": client.ingame,
        "scene_state": client.scene_state,
        "renderer": "real Client CpuPix3D framebuffer",
        "width": pixels.width,
        "height": pixels.height,
    });
    std::fs::write(
        directory.join(format!("{stem}.json")),
        serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(png_path)
}

fn run_origin(
    profile: &Arc<host_play::ServerProfile>,
    template: &Arc<SharedClientTemplate>,
    origin: Origin,
    account: String,
    password: String,
    evidence_root: &Path,
) -> Result<(), String> {
    let (scenario, start_settings) = make_cook_scenario(origin)?;
    let scenario_deadline = scenario.settings.deadline;
    let output_serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let evidence_dir = evidence_root.join(format!(
        "{CELL}_{}_{}_{}Z",
        origin.name, account, output_serial
    ));
    std::fs::create_dir_all(&evidence_dir)
        .map_err(|error| format!("create {}: {error}", evidence_dir.display()))?;

    let mut runner = ScenarioRunner::with_world(scenario, template.world());
    runner.set_map_members(profile.map_members());
    runner.set_live_names(std::slice::from_ref(&account));
    runner.set_shot_sink(Box::new(|_, _| {}));
    let state = Arc::new(Mutex::new(BankChoiceState {
        runner,
        account: account.clone(),
        origin,
        start_settings: start_settings.clone(),
        evidence_dir: evidence_dir.clone(),
        started_at: Instant::now(),
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        start_handle: None,
        start_count: 0,
        start_receipt: None,
        arrival: None,
        arrival_at: None,
        arrival_status_key: None,
        arrival_status: None,
        after_bank_outcome: None,
        pending_capture: None,
        capture_paths: Vec::new(),
        capture_error: None,
        latest_status: None,
        latest_status_key: None,
        native_status_history: Vec::new(),
        position_trace: Vec::new(),
        last_trace_tile: None,
        last_trace_at: Instant::now(),
        error: None,
    }));
    let frame_state = Arc::clone(&state);
    let frame_account = account.clone();
    let mut play = host_play::run_with_template(
        Arc::clone(template),
        true,
        vec![],
        |_| (None, None),
        move |client, username, hold| {
            if username == frame_account {
                if let Ok(mut slot) = frame_state.lock() {
                    slot.frame(client, hold.hold);
                }
            }
        },
    )?;
    {
        let mut slot = state.lock().map_err(|_| "bank-choice state poisoned")?;
        slot.runner.set_obj_names(play.obj_names());
        slot.start_handle = Some(play.script_start_handle());
    }
    play.try_spawn_slot(mint_profile(&account, &password, 1)?, None, None, None)?;
    play.focus(&account);
    println!(
        "{}",
        json!({
            "phase": "identity",
            "cell": CELL,
            "profile": profile.label(),
            "game_port": profile.client().game_port(),
            "nav_pack": profile.nav_pack(),
            "engine_dir": std::env::var("WORLD_ENGINE_DIR").ok(),
            "catalog_root": std::env::var("RS2B0T").ok(),
            "cache_dir": std::env::var("BOT_CACHE_DIR").ok(),
            "account": account,
            "origin": origin.name,
            "start_tile": tile_json(Some(origin.start)),
            "expected_bank": {"name": origin.bank_name, "tile": tile_json(Some(origin.bank))},
            "card": "Quester",
            "start_settings": start_settings,
            "scenario": "quester_cook",
            "evidence_dir": evidence_dir,
            "scenario_deadline_seconds": scenario_deadline.as_secs(),
        })
    );

    let test_started = Instant::now();
    let result = loop {
        let status = play.script_native_status(&account);
        let run_state = play.script_state(&account);
        let lifecycle = play.script_lifecycle_receipt(&account);
        let script_error = play.script_last_error(&account);
        let (arrival_at, after_bank, state_error, scenario_status) = {
            let mut slot = state.lock().map_err(|_| "bank-choice state poisoned")?;
            if run_state == script::RunState::Running {
                slot.runner.observe_script_running();
            }
            slot.observe_status(status.clone());
            (
                slot.arrival_at,
                slot.after_bank_outcome.is_some()
                    && slot
                        .capture_paths
                        .iter()
                        .any(|(step, _)| step == "03-after-bank"),
                slot.error.clone(),
                slot.runner.status(),
            )
        };
        if let Some(error) = state_error {
            break Err(error);
        }
        if let Some(error) = script_error {
            break Err(format!("Quester lifecycle error: {error}"));
        }
        if let Some(status) = status.as_ref() {
            if status.phase == NativePhase::Blocked && arrival_at.is_none() {
                break Err(format!(
                    "Cook blocked before observed local-bank arrival: {:?}",
                    status.failure
                ));
            }
        }
        if let Some(receipt) = lifecycle.as_ref() {
            match receipt.state {
                script::ScriptTerminalState::Failed
                | script::ScriptTerminalState::Cancelled
                | script::ScriptTerminalState::Stopped => {
                    break Err(format!("Cook lifecycle ended unexpectedly: {receipt:?}"));
                }
                script::ScriptTerminalState::Completed => {}
            }
        }
        if let RunnerStatus::Failed(error) = scenario_status {
            break Err(format!("Cook scenario setup failed: {error}"));
        }
        if after_bank {
            break Ok(());
        }
        let deadline = arrival_at
            .map(|arrival| arrival + COOK_ONWARD_DEADLINE)
            .unwrap_or(test_started + BANK_ARRIVAL_DEADLINE);
        if Instant::now() >= deadline {
            let witness = state
                .lock()
                .map_err(|_| "bank-choice state poisoned")?
                .receipt(
                    run_state,
                    lifecycle.as_ref().map(|receipt| format!("{receipt:?}")),
                );
            break Err(format!(
                "{} timed out waiting for observed bank arrival and Cook onward activity (receipt: {witness})",
                origin.name
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    let final_receipt = {
        let slot = state.lock().map_err(|_| "bank-choice state poisoned")?;
        let mut receipt = slot.receipt(
            play.script_state(&account),
            play.script_lifecycle_receipt(&account)
                .as_ref()
                .map(|receipt| format!("{receipt:?}")),
        );
        receipt["result"] = json!(format!("{result:?}"));
        receipt["evidence_dir"] = json!(evidence_dir);
        receipt
    };
    std::fs::write(
        evidence_dir.join("receipt.json"),
        serde_json::to_vec_pretty(&final_receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("write final receipt in {}: {error}", evidence_dir.display()))?;
    println!("{}", final_receipt);
    play.stop_slot(&account);
    result
}

fn run_bank_choice_live() -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Ok(());
    }
    if std::env::var("BOT_LIVE_NAME_PREFIX").as_deref() != Ok("bc") {
        return Err(format!("{CELL} requires BOT_LIVE_NAME_PREFIX=bc"));
    }
    if std::env::var("BOT_CPU").as_deref() != Ok("1") {
        return Err(format!("{CELL} requires BOT_CPU=1 for scene2 PNG evidence"));
    }
    if std::env::var("BOT_NAV_BUILD").as_deref() != Ok("skip") {
        return Err(format!(
            "{CELL} requires BOT_NAV_BUILD=skip and an explicit WORLD_NAV_PACK"
        ));
    }

    let isolated = script::IsolatedEnv::enter(CELL);
    let nav_pack = required_path("WORLD_NAV_PACK")?;
    let engine_dir = required_path("WORLD_ENGINE_DIR")?;
    let catalog_root = required_path("RS2B0T")?;
    let cache_dir = required_path("BOT_CACHE_DIR")?;
    let evidence_root = required_path("LIVE_EVIDENCE_DIR")?;
    validate_inputs(
        &nav_pack,
        &engine_dir,
        &catalog_root,
        &cache_dir,
        &evidence_root,
        &isolated.home,
    )?;
    isolated.set_rs2b0t(&catalog_root);
    api::hostlog::set_debug(true);

    let temp = TempRoot::new(CELL)?;
    let (profile, template) =
        selected_profile(nav_pack, engine_dir, catalog_root, cache_dir, temp.path())?;
    let names = host_play::mint_live_names(ORIGINS.len());
    if names.len() != ORIGINS.len() {
        return Err(format!("failed to mint {} bc live accounts", ORIGINS.len()));
    }
    let entries = host_play::mint_live_entries(&names);
    if entries.len() != ORIGINS.len() {
        return Err(format!(
            "failed to mint credentials for {} bc accounts",
            ORIGINS.len()
        ));
    }

    let mut failures = Vec::new();
    for (index, origin) in ORIGINS.iter().copied().enumerate() {
        let password = entries[index].1.clone();
        if let Err(error) = run_origin(
            &profile,
            &template,
            origin,
            names[index].clone(),
            password,
            &evidence_root,
        ) {
            failures.push(format!("{}: {error}", origin.name));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

#[test]
fn cook_fixture_establishes_origin_after_last_relog_before_start() {
    for &origin in ORIGINS {
        let (scenario, _) = make_cook_scenario(origin).unwrap();
        let last_relog = scenario
            .steps
            .iter()
            .rposition(|step| matches!(step.kind, StepKind::Relog))
            .unwrap();
        let stand = scenario
            .steps
            .iter()
            .position(|step| step.name == "stand at the quest start")
            .unwrap();
        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert!(last_relog < stand);
        assert_eq!(stand + 1, start);
    }
}

#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX=bc, BOT_CPU=1, BOT_NAV_BUILD=skip, explicit WORLD_NAV_PACK/WORLD_ENGINE_DIR/RS2B0T/BOT_CACHE_DIR/LIVE_EVIDENCE_DIR and local 289 engine"]
fn quester_cook_local_bank_choice() {
    run_bank_choice_live().unwrap();
}
