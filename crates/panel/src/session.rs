//! Panel session: owns the unlocked vault, the running slot map, the shared
//! `Focus`, and per-slot frame/input channels. The panel frame reads
//! `Session`; slot threads stay in `host_play` (spawned via `run_with_io`
//! with per-profile `FrameBuf` mailbox/`SlotInput`, keeping the login FIFO
//! and the mainland hop).
//!
//! Flat slot model (M2 Task 2b): every wall member is its own full `Client`
//! on its own slot thread — there is no channel head and no lean baton.
//! Clicking a member is [`Session::select`], which is pure `focus` bookkeeping:
//! the Game pane samples that slot's `FrameBuf`. The single-client boot still
//! holds: unlock spawns **one** Client (the focused profile); MultiBox spawns
//! the rest.

use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::snapshot::{GameSnapshot, WorldTile};
use client::client::{Client, ClientGens};
use client::render::nav_debug::{
    NavDebugCell, NavDebugColors, NavDebugHull, NavDebugPaint, CORNER_NE, CORNER_NW, CORNER_SE,
    CORNER_SW, FACE_E, FACE_N, FACE_S, FACE_W,
};
use client::sound::output::AudioOut;
use host::{FrameBuf, InputEv, SlotInput};
use host_play::audio::{AudioChange, AudioGate};
use host_play::profile::ProfileEnvironment;
use host_play::progress::{ProfileProgress, ProfileProgressObserver, ProfileProgressStage};
use host_play::{
    open_vault, run_prepared_template, run_with_io, run_with_template, Play, PlayOptions,
    ProfileOptions, ScriptNavPaint, ServerProfile, SharedClientTemplate, SlotArm, SlotStatus,
    ValidatedTemplate, WalkArm,
};
use nav::paint::{
    collision_at_with, hop_captions, hull_targets, reached, remaining_path_tiles, remaining_trail,
    select_draw_indices, trail_tones, TrailTone,
};
use nav::router::{FindOptions, Route};
use nav::tile::Tile;
use nav::traveller::TravelOptions;
use nav::world::NavWorld;
use nav::WorldState;
use vault::{Profile, ProfileSettings, Vault};

use crate::focus::{draw_for_slot, full_rate_for};
use crate::nav_settings::{from_scenario, parse_html_color, NavSettings};
use crate::wall::Wall;

/// Catalog card live_prepare stashes so the StartScript pump can
/// `script_start_load` once after seed waits. Fleet P2P golds stash one
/// entry per slot (driven + companion).
#[derive(Clone)]
struct PendingCatalogStart {
    slot: String,
    js: String,
    shape: script::LoadShape,
    bag: Option<serde_json::Map<String, serde_json::Value>>,
    siblings: Vec<(String, String)>,
    /// Scenario-owned loadouts for harness Start; empty uses operator store.
    loadouts: Vec<script::Loadout>,
    /// Compiled registry card; when set, Start uses `start_compiled`.
    compiled: Option<script::CompiledId>,
    /// Start was accepted; kept until its isolate setup settles so a setup
    /// failure still fails the core watch with its reason.
    started: bool,
}

/// Freeze the already-published prepared observation immediately before the
/// actual isolate Start call. A successful Start cannot overtake its baseline.
fn start_catalog_with_core<F>(
    watch: &host_play::catalog_core::CoreWatch,
    slot: &str,
    start: F,
) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String>,
{
    watch.begin_start(slot)?;
    let result = start();
    if let Err(error) = &result {
        watch.fail_start(slot, error.clone());
    }
    result
}

fn scenario_fixture_loadouts(settings: &scenario::ScenarioSettings) -> Vec<script::Loadout> {
    settings
        .fixture_loadouts
        .unwrap_or(&[])
        .iter()
        .map(|row| {
            row.carry
                .iter()
                .fold(script::Loadout::new(row.name), |loadout, &(item, qty)| {
                    loadout.with_carry(item, qty)
                })
        })
        .collect()
}

fn start_stashed_catalog_card(
    handle: &host_play::ScriptStartHandle,
    card: &PendingCatalogStart,
) -> Result<(), String> {
    if let Some(id) = card.compiled {
        return handle.start_compiled(&card.slot, id);
    }
    if card.loadouts.is_empty() {
        handle.start_load(
            &card.slot,
            card.js.clone(),
            card.shape,
            card.bag.clone(),
            card.siblings.clone(),
        )
    } else {
        handle.start_load_with_loadouts(
            &card.slot,
            card.js.clone(),
            card.shape,
            card.bag.clone(),
            card.siblings.clone(),
            &card.loadouts,
        )
    }
}

/// Owned inputs captured on the UI thread and consumed by the sequential
/// profile/template preparation worker.
pub(crate) struct ProfilePreparation {
    options: ProfileOptions,
    environment: ProfileEnvironment,
    saved_revision: u16,
}

impl ProfilePreparation {
    #[cfg(test)]
    pub(crate) fn run(self) -> Result<Arc<SharedClientTemplate>, String> {
        self.run_with_progress(&ProfileProgressObserver::default())
    }

    pub(crate) fn run_with_progress(
        self,
        observer: &ProfileProgressObserver,
    ) -> Result<Arc<SharedClientTemplate>, String> {
        observer.report(ProfileProgress::steps(
            ProfileProgressStage::SelectingServerProfile,
            0,
            1,
        ));
        let selection = self
            .options
            .resolve_with_env(Some(self.saved_revision), &self.environment)?;
        observer.report(ProfileProgress::steps(
            ProfileProgressStage::SelectingServerProfile,
            1,
            1,
        ));
        selection.prepare_template_with_progress(observer)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProfilePreparationCompletion {
    Installed,
    Stale,
    Failed,
}

/// When the runner is on [`scenario::StepKind::StartScript`], start every
/// stashed isolate. Returns false when Start was attempted and failed,
/// so the pump must not consume the one-tick wait. Started cards stay
/// stashed until their setup settles: Start returns before V8 setup, so a
/// setup failure fails the core watch here, with its diagnostic.
fn fire_pending_catalog_start(
    pending: &Mutex<Vec<PendingCatalogStart>>,
    handle: &Mutex<Option<host_play::ScriptStartHandle>>,
    core_watch: &Mutex<Option<host_play::catalog_core::CoreWatch>>,
    pair_watch: &Mutex<Option<host_play::paired_core::PairWatch>>,
    runner: &scenario::ScenarioRunner,
) -> bool {
    let mut pending = pending.lock().unwrap();
    if pending.iter().any(|card| card.started) {
        if let Some(h) = handle.lock().unwrap().as_ref() {
            settle_started_catalog_cards(&mut pending, h, core_watch, pair_watch);
        }
    }
    if !runner.on_start_script() {
        return true;
    }
    if pending.iter().all(|card| card.started) {
        return true;
    }
    let handle = handle.lock().unwrap();
    let Some(h) = handle.as_ref() else {
        return false;
    };
    let pair = pair_watch.lock().unwrap().clone().unwrap_or_default();
    if pair.configured() {
        if pending.len() != 2 {
            pair.fail_start("pair core requires actual scripts on both visible slots");
            return false;
        }
        match pair.barrier() {
            host_play::paired_core::StartBarrier::Wait => return false,
            host_play::paired_core::StartBarrier::RejectStartedWhileUnready => {
                pair.fail_start("pair core Start while the counterpart is unready");
                return false;
            }
            host_play::paired_core::StartBarrier::StartBoth => {}
        }
        if pair
            .begin_shared_start(&pending[0].slot, &pending[1].slot)
            .is_err()
        {
            return false;
        }
        for card in pending.iter_mut() {
            if let Err(error) = start_stashed_catalog_card(h, card) {
                pair.fail_start(error);
                return false;
            }
            card.started = true;
        }
        return true;
    }
    let watch = core_watch.lock().unwrap().clone().unwrap_or_default();
    for card in pending.iter_mut().filter(|card| !card.started) {
        if start_catalog_with_core(&watch, &card.slot, || start_stashed_catalog_card(h, card))
            .is_err()
        {
            return false;
        }
        card.started = true;
    }
    true
}

/// Drop started cards whose setup settled; a failed setup fails the watch
/// that armed its Start (the refusal path `fail_start` already covers).
fn settle_started_catalog_cards(
    pending: &mut Vec<PendingCatalogStart>,
    handle: &host_play::ScriptStartHandle,
    core_watch: &Mutex<Option<host_play::catalog_core::CoreWatch>>,
    pair_watch: &Mutex<Option<host_play::paired_core::PairWatch>>,
) {
    let mut failed = Vec::new();
    pending.retain(|card| {
        if !card.started {
            return true;
        }
        match handle.poll_start(&card.slot) {
            script::StartPoll::Pending => true,
            script::StartPoll::Settled(script::StartOutcome::Failed(error)) => {
                failed.push((card.slot.clone(), error));
                false
            }
            script::StartPoll::Settled(
                script::StartOutcome::Ready | script::StartOutcome::Cancelled,
            )
            | script::StartPoll::NotOwed => false,
        }
    });
    for (slot, error) in failed {
        let pair = pair_watch.lock().unwrap().clone().unwrap_or_default();
        if pair.configured() {
            pair.fail_start(error);
        } else if let Some(watch) = core_watch.lock().unwrap().clone() {
            watch.fail_start(&slot, error);
        }
    }
}

/// Load one exact in-tree example and return the File card selected by
/// canonical-path identity. Never looks up the shared stem.
fn load_live_example_card(
    js: &mut script::JsLibrary,
    file_name: &str,
) -> Result<script::JsCard, String> {
    let path = script::live_example_path(file_name)
        .ok_or_else(|| format!("no in-tree example {file_name}"))?;
    let loaded = js
        .load(&path)
        .map_err(|e| format!("load {file_name}: {e}"))?;
    let identity = loaded.identity_id();
    js.get(script::ScriptSource::File, &identity)
        .cloned()
        .ok_or_else(|| format!("file example {file_name} missing after identity load"))
}

/// Existing isolate Idle + lifecycle receipt reason. Not a game-chat read.
pub(crate) fn script_self_stop_observed(
    state: script::RunState,
    receipt: Option<&script::ScriptLifecycleReceipt>,
    needle: &str,
) -> bool {
    matches!(state, script::RunState::Idle)
        && receipt.is_some_and(|receipt| receipt.reason.contains(needle))
}

/// Stash StartScript isolate(s). Driven slot always. When
/// `inject_companion_as` is set on a fleet, also Start the same JS on
/// slot 1 with that key naming the driven player (P2P Trade), as the game
/// shows it ([`partner_screen_name`]).
#[allow(clippy::too_many_arguments)] // pending start packs names/companion/loadout fields
fn stash_pending_starts(
    pending: &Mutex<Vec<PendingCatalogStart>>,
    names: &[String],
    inject_companion_as: Option<&str>,
    js: String,
    shape: script::LoadShape,
    bag: Option<serde_json::Map<String, serde_json::Value>>,
    siblings: Vec<(String, String)>,
    loadouts: Vec<script::Loadout>,
) {
    let mut starts = vec![PendingCatalogStart {
        slot: names[0].clone(),
        js: js.clone(),
        shape,
        bag: bag.clone(),
        siblings: siblings.clone(),
        loadouts: loadouts.clone(),
        compiled: None,
        started: false,
    }];
    if let Some(key) = inject_companion_as {
        if names.len() > 1 {
            let mut companion_bag = bag.unwrap_or_default();
            companion_bag.insert(
                key.to_string(),
                serde_json::Value::String(partner_screen_name(&names[0])),
            );
            starts.push(PendingCatalogStart {
                slot: names[1].clone(),
                js,
                shape,
                bag: Some(companion_bag),
                siblings,
                loadouts,
                compiled: None,
                started: false,
            });
        }
    }
    *pending.lock().unwrap() = starts;
}

/// A partner setting names the player as the game shows it: minted
/// usernames (`live…_1`) display as screen names (`Live… 1`), and the
/// frozen `Players.query().name()` (like host dispatch) matches the shown
/// name exactly, ignoring case. The pair watches do the same
/// ([`host_play::paired_core::pair_settings`]).
fn partner_screen_name(username: &str) -> String {
    client::util::JString::to_screen_name(username)
}

fn stash_compiled_start(
    pending: &Mutex<Vec<PendingCatalogStart>>,
    names: &[String],
    id: script::CompiledId,
) {
    *pending.lock().unwrap() = vec![PendingCatalogStart {
        slot: names[0].clone(),
        js: String::new(),
        shape: script::LoadShape::Reject,
        bag: None,
        siblings: Vec::new(),
        loadouts: Vec::new(),
        compiled: Some(id),
        started: false,
    }];
}

#[allow(clippy::too_many_arguments)] // pair start packs names/case/watch fields
fn stash_pair_starts(
    pending: &Mutex<Vec<PendingCatalogStart>>,
    names: &[String],
    case: host_play::paired_core::PairCase,
    js: String,
    shape: script::LoadShape,
    schema: &[script::SettingDef],
    siblings: Vec<(String, String)>,
    watch: &host_play::paired_core::PairWatch,
) -> Result<(), String> {
    if names.len() != 2 {
        return Err("pair core watch supports exactly two driven slots".into());
    }
    let a_bag = host_play::paired_core::pair_settings(case, schema, 0, &names[0], &names[1])?;
    let b_bag = host_play::paired_core::pair_settings(case, schema, 1, &names[1], &names[0])?;
    watch.install_prepared_settings(&names[0], a_bag.clone(), &names[1], b_bag.clone())?;
    *pending.lock().unwrap() = vec![
        PendingCatalogStart {
            slot: names[0].clone(),
            js: js.clone(),
            shape,
            bag: Some(a_bag),
            siblings: siblings.clone(),
            loadouts: Vec::new(),
            compiled: None,
            started: false,
        },
        PendingCatalogStart {
            slot: names[1].clone(),
            js,
            shape,
            bag: Some(b_bag),
            siblings,
            loadouts: Vec::new(),
            compiled: None,
            started: false,
        },
    ];
    Ok(())
}

/// Scatter / mainland hop only on a cold world, not after a `lostCon`
/// reconnect (that would tele the re-handshaked slot on every DC).
fn seed_on_first_world(last_login_reconnect: Option<bool>) -> bool {
    last_login_reconnect != Some(true)
}

/// Loopback hosts get the debug heading / WalkTo Teleport. Public
/// `w1.rs2b2t.com` and LAN IPs do not.
pub fn is_local_engine(host: &str) -> bool {
    host_play::is_loopback_host(host)
}

/// One Teles-popup dest: button label, `CLIENT_CHEAT` body, hover text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DebugDest {
    pub label: &'static str,
    pub cheat: &'static str,
    pub tooltip: &'static str,
}

/// Engine `::getvar tutorial` replies `get tutorial: 1000`.
pub fn parse_getvar_line(text: &str) -> Option<(&str, i32)> {
    let rest = text.strip_prefix("get ")?;
    let (name, value) = rest.rsplit_once(':')?;
    Some((name.trim(), value.trim().parse().ok()?))
}

/// Debug heading buttons. TutSkip is omitted once the profile is known
/// skipped or still unknown (a `getvar` is in flight).
pub fn debug_main_buttons(show_tutskip: bool) -> Vec<&'static str> {
    debug_main_buttons_for(client::bot_target(), show_tutskip)
}

/// [`debug_main_buttons`] for an explicit target so Prod tests do not
/// flip the process-wide `OnceLock`.
pub fn debug_main_buttons_for(target: client::BotTarget, show_tutskip: bool) -> Vec<&'static str> {
    if target != client::BotTarget::Local {
        return vec!["DebugPanel"];
    }
    let mut labels = vec!["DebugPanel"];
    if show_tutskip {
        labels.push("TutSkip");
    }
    labels.extend(["Lumbridge", "maxme", "Teles"]);
    labels
}

/// Named dest cheats for the Teles popup (`[debugproc]` names).
pub fn debug_dest_cheats() -> &'static [DebugDest] {
    // Engine debugprocs are `::<debugProcChar><name>` with debugProcChar
    // default `~` (`::~home`). Engine commands (`tele`, `setvar`,
    // `setstat`) have no tilde.
    &[
        DebugDest {
            label: "Lumbridge",
            cheat: "~home",
            tooltip: "Lumbridge courtyard",
        },
        DebugDest {
            label: "Varrock",
            cheat: "~varrock",
            tooltip: "Varrock square",
        },
        DebugDest {
            label: "Falador",
            cheat: "~falador",
            tooltip: "Falador square",
        },
        DebugDest {
            label: "Draynor",
            cheat: "~draynor",
            tooltip: "Draynor Village",
        },
        DebugDest {
            label: "PortSarim",
            cheat: "~portsarim",
            tooltip: "Port Sarim docks",
        },
        DebugDest {
            label: "Rimmington",
            cheat: "~rimmington",
            tooltip: "Rimmington",
        },
        DebugDest {
            label: "AlKharid",
            cheat: "~alkharid",
            tooltip: "Al Kharid",
        },
        DebugDest {
            label: "Seers",
            cheat: "~seers",
            tooltip: "Seers' Village",
        },
        DebugDest {
            label: "Giants",
            cheat: "~giants",
            tooltip: "Hill Giants (Edgeville dungeon)",
        },
        DebugDest {
            label: "Entrana",
            cheat: "~entrana",
            tooltip: "Entrana (no weapons/armour)",
        },
        DebugDest {
            label: "Brimhaven",
            cheat: "~brimhaven",
            tooltip: "Brimhaven",
        },
        DebugDest {
            label: "Ardy",
            cheat: "~ardy",
            tooltip: "East Ardougne",
        },
        DebugDest {
            label: "Kbd",
            cheat: "~kbd",
            tooltip: "King Black Dragon lair",
        },
        DebugDest {
            label: "Elvarg",
            cheat: "~elvarg",
            tooltip: "Elvarg on Crandor",
        },
        DebugDest {
            label: "Greenland",
            cheat: "~greenland",
            tooltip: "Fields north of Tree Gnome Stronghold (not green dragons)",
        },
        DebugDest {
            label: "Gb",
            cheat: "~gb",
            tooltip: "Gnome Ball pitch",
        },
        DebugDest {
            label: "Ma",
            cheat: "~ma",
            tooltip: "Mage Arena (Wilderness)",
        },
        DebugDest {
            label: "Pvp",
            cheat: "~pvp",
            tooltip: "PvP / bounty area",
        },
        DebugDest {
            label: "Duel",
            cheat: "~duel",
            tooltip: "Duel Arena",
        },
        DebugDest {
            label: "Trawler",
            cheat: "~trawler",
            tooltip: "Fishing Trawler",
        },
        DebugDest {
            label: "Gamesroom",
            cheat: "~gamesroom",
            tooltip: "Burthorpe Games Room",
        },
        DebugDest {
            label: "Mortton",
            cheat: "~mortton",
            tooltip: "Mort'ton",
        },
    ]
}

/// `setstat <skill> 99` for the skills `[debugproc,maxme]` advances.
pub fn debug_maxme_cheats() -> &'static [&'static str] {
    api::interact::MAXME_SETSTATS
}

/// Engine `::tele` body for a WalkTo tile.
pub fn walkto_tele_cmd(tile: Tile) -> String {
    api::interact::tele_args(tile.level, tile.x, tile.z)
}

/// Cooldown between cpal open retries after a device failure: a machine
/// without an audio device must not re-open (and re-log) every 20 ms frame.
const AUDIO_OPEN_RETRY: Duration = Duration::from_secs(5);

/// Vault path used by panel-play (`~/.274bot/vault` local, `vault-prod` on
/// `--prod`). The same helper host-play and tui-play use.
pub fn default_vault_path() -> PathBuf {
    host_play::default_vault_path()
}

fn default_cache_dir() -> String {
    client::cache_dir().display().to_string()
}

/// Panel-side per-slot IO: the frame mailbox the slot stores each rendered
/// `FrameOutput` into while its renderer is on (the panel `take`s it and
/// packs the `PixMap` or reads the `Texture` back at the consume site),
/// and the input channel it drains only while capture is on.
pub struct SlotIo {
    pub input: Arc<SlotInput>,
    pub pixels: Arc<FrameBuf>,
}

/// A rail removal is owned by the exact arm that received its clean-logout
/// request. `io` stays here while that worker drains so re-adding the member
/// can reattach the same framebuffer and input channels without spawning a
/// second client lifetime.
struct PendingSlotRemoval {
    started: Instant,
    arm: Arc<SlotArm>,
    io: Option<SlotIo>,
}

/// Combo highlight: `None` when nothing is focused so the widget cannot
/// display index 0 as selected.
pub fn combo_index(focused: Option<&str>, names: &[String]) -> Option<usize> {
    focused.and_then(|n| names.iter().position(|x| x == n))
}

pub use crate::input_capture::{maybe_send_click, stream_capture};

/// rs2b0t disable rule: a script is active while it holds the slot, so
/// Start and Browse (and Load) are disabled for those states.
pub fn script_active(state: script::RunState) -> bool {
    matches!(
        state,
        script::RunState::Starting
            | script::RunState::Running
            | script::RunState::Paused
            | script::RunState::Stopping
    )
}

/// Pause/Resume enable rule: enabled only while Running (Pause) or Paused
/// (Resume); the button label switches to "Resume" when paused.
pub fn script_pause_enabled(state: script::RunState) -> bool {
    matches!(
        state,
        script::RunState::Starting | script::RunState::Running | script::RunState::Paused
    )
}

/// Stop enable rule: enabled while active, but not while already Stopping.
pub fn script_stop_enabled(state: script::RunState) -> bool {
    script_active(state) && state != script::RunState::Stopping
}

/// The script status-row text for a lifecycle state.
pub fn script_status_text(state: script::RunState) -> &'static str {
    match state {
        script::RunState::Idle => "idle",
        script::RunState::Starting => "starting",
        script::RunState::Running => "running",
        script::RunState::Paused => "paused",
        script::RunState::Stopping => "stopping",
        script::RunState::Error => "error",
    }
}

/// The loaded scene's tile count per side (the client's `BUILD_AREA_SIZE`):
/// the collision paint only covers the region the client has built,
/// `[base_x, base_x+104) × [base_z, base_z+104)` in world tiles.
const SCENE_TILES: i32 = 104;

/// Transport hops up to this many remaining tiles ahead get hull strokes;
/// the client only projects the locs inside its loaded scene anyway.
const HULL_WINDOW: usize = 48;

/// Prefer the live-scenario Follow/Walk route when this slot is the
/// driven client. Else a catalog `walk` Traveller (Play NavBot). WalkTo's
/// `WalkArm` is the last fallback — a script walk never writes that map,
/// so painting from it alone drops the packed path even with the toggle on.
fn live_or_walk_paint(
    driven: bool,
    live: (Option<Route>, Option<WorldTile>),
    walk: (Option<Route>, Option<WorldTile>),
    script: (Option<Route>, Option<WorldTile>),
) -> (Option<Route>, Option<WorldTile>) {
    if driven && live.0.is_some() {
        live
    } else if script.0.is_some() {
        script
    } else {
        walk
    }
}

/// Per-frame nav-paint mirror: the slot threads read it each observe to
/// publish the focused drawing slot's scene paint.
/// [`Session::pump_status`] re-copies it from `Session::nav_overlay` (live)
/// or `Session::ui.nav` every UI frame.
#[derive(Clone, Default)]
struct NavPublishCfg {
    settings: NavSettings,
}

/// Map the client's last tryMove BFS into world tiles and trim it for
/// paint. The producer (`try_move_path`) is debug-only: reaching dest or
/// leaving the path must retire it so a later off-path step or revisit
/// cannot republish the last click. `here == None` does not retire
/// (startup / network wait).
fn live_client_trail(client: &mut Client, here: Option<WorldTile>) -> Vec<WorldTile> {
    let base_x = client.map_build_base_x;
    let base_z = client.map_build_base_z;
    let trail_all: Vec<WorldTile> = client
        .try_move_path
        .iter()
        .map(|&(sx, sz)| WorldTile {
            x: base_x + sx,
            z: base_z + sz,
            level: 0,
        })
        .collect();
    let trail_world = remaining_trail(&trail_all, here);
    if here.is_some() && trail_world.is_empty() {
        client.try_move_path.clear();
    }
    trail_world
}

/// Publish the nav-debug scene paint for the focused drawing slot each
/// observe. `drawing` is the gate: only the focused slot with its renderer
/// on publishes; unfocused / skip-paint / renderer-off slots store `None`
/// so a stale paint never lingers. `world` is the baked pack, `route` the
/// armed walk route, `here` the player's observed world tile, `trail_world`
/// the local player's last `tryMove` route buffer (world tiles), `run_on`
/// the local player's run state (two-tone trail), and `click` the
/// traveller's current walk aim.
///
/// World → scene: `x - map_build_base_x`, `z - map_build_base_z`.
/// Collision covers every tile of the loaded [`SCENE_TILES`]² region the
/// `collision_fill` / `nsew_labels` toggles warrant; the path is the
/// remaining route subsampled to the 3D draw budget (the pack map keeps
/// the full path).
// The brief fixes this signature; a param struct would only shuffle names
// across the one call site.
#[allow(clippy::too_many_arguments)]
fn publish_nav_debug(
    client: &mut Client,
    world: &NavWorld,
    route: Option<&Route>,
    here: Option<WorldTile>,
    trail_world: &[WorldTile],
    run_on: bool,
    click: Option<WorldTile>,
    settings: &NavSettings,
    drawing: bool,
) {
    // Flags sidecar lifecycle: decoded once while a collision paint
    // toggle is on (the paint prefers the raw flags — visibility bits
    // the packed walk u16 drops — when the sidecar is mapped), dropped
    // when both toggles go off. Never opened for a pack the session only
    // walks.
    if settings.collision_fill || settings.nsew_labels {
        crate::picker::ensure_flags_sidecar();
    } else {
        crate::picker::drop_flags_sidecar();
    }
    if !drawing {
        client.set_nav_debug_paint(None);
        return;
    }
    let base_x = client.map_build_base_x;
    let base_z = client.map_build_base_z;
    let mut colors = NavDebugColors::default();
    colors.collision = parse_html_color(&settings.color_collision, colors.collision);
    colors.nsew = parse_html_color(&settings.color_text, colors.nsew);
    colors.path = parse_html_color(&settings.color_path, colors.path);
    colors.path_hop = parse_html_color(&settings.color_transport, colors.path_hop);
    colors.trail = parse_html_color(&settings.color_client, colors.trail);
    colors.trail_run = parse_html_color(&settings.color_client_run_alt, colors.trail_run);
    colors.hull = parse_html_color(&settings.color_transport, colors.hull);
    colors.click = parse_html_color(&settings.color_click, colors.click);
    let mut paint = NavDebugPaint {
        colors,
        show_collision: settings.collision_fill,
        show_nsew: settings.nsew_labels,
        show_path: settings.show_nav_path,
        // `show_nav_path` is the master for the path / hull / trail paints
        // (spec Display row); the layer toggles opt each one in.
        show_trail: settings.show_nav_path && settings.client_trail,
        show_hulls: settings.show_nav_path && settings.hop_labels,
        ..NavDebugPaint::default()
    };
    // Collision: every tile of the loaded scene region whose
    // `collision_fill` (blocked ground) or `nsew_labels` (face letters)
    // toggle paints it, in scene coords. Tiles outside the pack grid read
    // as open — no phantom wall at the bake's edge.
    if settings.collision_fill || settings.nsew_labels {
        let level = world.collision.origin.level;
        let ox = world.collision.origin.x;
        let oz = world.collision.origin.z;
        let (ow, oh) = (world.collision.width as i32, world.collision.height as i32);
        // Prefer the decoded flags sidecar when it matches this world's
        // grid; otherwise the walk word answers (the shared world's
        // `flags` field stays `None` — no walk-grid clone).
        let side = crate::picker::flags_sidecar_for(
            world.collision.origin,
            world.collision.width,
            world.collision.height,
        );
        // The paint-only reach bitset, baked once per world. A missing
        // bitset defaults every cell to reached — no unreached tint.
        let reach_bits = crate::picker::reach_bitset(world);
        for lz in 0..SCENE_TILES {
            for lx in 0..SCENE_TILES {
                let x = base_x + lx;
                let z = base_z + lz;
                if x < ox || z < oz || x - ox >= ow || z - oz >= oh {
                    continue;
                }
                let wt = WorldTile { x, z, level };
                let fb = collision_at_with(&world.collision, wt, side.as_deref());
                let mut bits = 0u8;
                if fb.n {
                    bits |= FACE_N;
                }
                if fb.s {
                    bits |= FACE_S;
                }
                if fb.e {
                    bits |= FACE_E;
                }
                if fb.w {
                    bits |= FACE_W;
                }
                if fb.ne {
                    bits |= CORNER_NE;
                }
                if fb.se {
                    bits |= CORNER_SE;
                }
                if fb.nw {
                    bits |= CORNER_NW;
                }
                if fb.sw {
                    bits |= CORNER_SW;
                }
                if (settings.collision_fill && (fb.blocked || bits != 0))
                    || (settings.nsew_labels && bits != 0)
                {
                    paint.collision.push(NavDebugCell {
                        lx,
                        lz,
                        bits,
                        // The client fills only blocked ground; a face-only
                        // cell keeps its NSEW letters without a fill quad.
                        blocked: fb.blocked,
                        // True when the transport network reaches the tile
                        // (or the bitset is missing): the client's reach
                        // fill draws only `show_collision && !reach`.
                        reach: reach_bits
                            .as_deref()
                            .is_none_or(|bits| reached(bits, &world.collision, wt)),
                    });
                }
            }
        }
    }
    if let Some(route) = route {
        // Path: the remaining route subsampled to the 3D draw budget.
        // Full density near, stride after, always keeping the transport
        // hops and the terminal (the pack map keeps the full path).
        if settings.show_nav_path {
            let tiles = remaining_path_tiles(route, here);
            let force: Vec<usize> = tiles
                .iter()
                .enumerate()
                .filter_map(|(i, p)| p.transport.then_some(i))
                .collect();
            paint.path = select_draw_indices(0, tiles.len(), &force)
                .into_iter()
                .map(|i| {
                    let p = tiles[i];
                    (p.tile.x - base_x, p.tile.z - base_z, p.transport)
                })
                .collect();
        }
        // Hulls: the loc-backed transport hops ahead, scene coords.
        // `show_nav_path` masters them; the hop-labels toggle also
        // strokes the transport loc hulls (the client's transport colour
        // family covers both).
        if settings.show_nav_path && settings.hop_labels {
            paint.hulls = hull_targets(route, here, HULL_WINDOW)
                .into_iter()
                .map(|h| NavDebugHull {
                    loc_id: h.loc_id,
                    scene_x: h.at.x - base_x,
                    scene_z: h.at.z - base_z,
                })
                .collect();
            paint.labels = hop_captions(route, here)
                .into_iter()
                .map(|c| (c.at.x - base_x, c.at.z - base_z, c.text))
                .collect();
        }
    }
    // Trail: client trail tones (two-tone while running). `show_nav_path`
    // is the master (spec Display row); `client_trail` opts the layer in.
    if settings.show_nav_path && settings.client_trail {
        paint.trail = trail_tones(trail_world, run_on)
            .into_iter()
            .map(|(t, tone)| (t.x - base_x, t.z - base_z, tone == TrailTone::RunAlt))
            .collect();
    }
    // Click: the traveller's current walk aim, scene coords. `show_nav_path`
    // masters it like the hulls (no nav path, no walk-target paint).
    if settings.show_nav_path {
        if let Some(aim) = click {
            paint.click = Some((aim.x - base_x, aim.z - base_z));
        }
    }
    client.set_nav_debug_paint(Some(paint));
}

/// Ease orbit yaw toward the remaining path (host-write, no client opcode).
/// Desired heading is held until it moves ≥ `TARGET_RETARGET_MIN` so a
/// corridor does not twitch; ease velocity is ours, not the keycam field
/// (`follow_camera` would double-integrate it).
fn apply_path_camera(client: &mut Client, route: Option<&Route>, here: Option<WorldTile>) {
    if client.shell.key_held[1] == 1 || client.shell.key_held[2] == 1 {
        return;
    }
    let Some(route) = route else {
        return;
    };
    let Some(here) = here else {
        return;
    };
    let tiles = remaining_path_tiles(route, Some(here));
    if tiles.is_empty() {
        return;
    }
    let world: Vec<_> = tiles.iter().map(|p| p.tile).collect();
    let hops: Vec<_> = tiles.iter().map(|p| p.transport).collect();
    let Some(sampled) = nav::camera::path_facing_yaw(here, &world, &hops, 12) else {
        return;
    };
    let mut hold = PATH_CAM.lock().unwrap();
    hold.desired = Some(nav::camera::hold_desired(hold.desired, sampled));
    let Some(target) = hold.desired else {
        return;
    };
    let (yaw, v) = nav::camera::ease_yaw(client.orbit_camera_yaw, target, hold.vel);
    hold.vel = v;
    client.orbit_camera_yaw = yaw;
    // Keycam must not also integrate our ease velocity.
    client.orbit_camera_yaw_velocity = 0;
}

struct PathCamHold {
    desired: Option<i32>,
    vel: f32,
}

static PATH_CAM: Mutex<PathCamHold> = Mutex::new(PathCamHold {
    desired: None,
    vel: 0.0,
});

pub struct Session {
    /// Shared focus policy; slot threads read it every frame (observe) to
    /// apply `client.set_draw(draw_for_slot(&focus, name))`, so only the
    /// focused slot rasters.
    pub focus: Arc<Mutex<crate::focus::Focus>>,
    pub vault: Option<Vault>,
    /// Last vault/connection error shown in the banner.
    pub error: Option<String>,
    /// Running slot threads and their shared statuses (created at unlock).
    pub play: Option<Play>,
    /// Per-username slot IO.
    pub slots: HashMap<String, SlotIo>,
    /// Rail removals waiting for clean disconnect or their bounded deadline.
    /// Each entry is bound to one arm lifetime; the UI frame only polls these,
    /// and worker joins stay in `Play`.
    pending_slot_removals: HashMap<String, PendingSlotRemoval>,
    /// The focused slot's live capture sender; `None` while capture is off,
    /// so UI send paths no-op.
    pub capture_tx: Option<Sender<InputEv>>,
    /// BOT_MAINLAND=1 / host-play --mainland; not a panel checkbox.
    pub mainland: Arc<AtomicBool>,
    /// Per-username panel log lines (status transitions), each capped at
    /// [`LOG_CAP`]. Vault / no-username lines use [`PROCESS`].
    pub log_by: Arc<Mutex<HashMap<String, Vec<String>>>>,
    /// Vault passphrase scratch buffer for the in-panel unlock prompt.
    pub pass_scratch: String,
    /// Last status poll (delta source for the log).
    pub statuses: Vec<SlotStatus>,
    /// Picker edit scratch (username/password). Empty on the strip.
    pub cred_user: String,
    pub cred_pass: String,
    /// Draft auto-login / random / lamp for a new vault row. An existing
    /// edit writes those through the vault immediately.
    pub cred_settings: ProfileSettings,
    /// Profile picker edit: `None` not editing, `Some("")` new profile,
    /// `Some(name)` editing that vault row.
    pub chooser_edit: Option<String>,
    /// Per-username walk arms; the focused slot's arm carries the armed
    /// whole-world route (polled from `start_play` `per_frame` via
    /// [`nav::traveller::Traveller::follow`]).
    pub travellers: SlotTravellers,
    /// Catalog `walk` overlay; filled after [`Session::start_play`].
    script_nav_paint: Arc<Mutex<Option<ScriptNavPaint>>>,
    /// Per-username gating facts for WalkTo routing: each slot thread
    /// publishes its live `GameSnapshot` (rebuilt incrementally — views
    /// copy only when a family's gen moved) plus the [`WorldState`]
    /// derived from it, so the UI thread can prove payable edges (a toll
    /// with coins on the player routes). A slot that has not published
    /// yet (no player decoded) is absent and routing fails closed on the
    /// empty state.
    pub nav_states: Arc<Mutex<HashMap<String, (GameSnapshot, WorldState)>>>,
    /// Host publication cursor per slot. PLAYER_INFO's tick edge is tracked
    /// separately from the snapshot's family generations.
    frontend_gens: Arc<Mutex<HashMap<String, ClientGens>>>,
    /// The tile the user last picked for WalkTo; `None` until armed. Read
    /// by [`Session::walk_status_text`] so the status row stays honest even
    /// when no route could be found.
    pub walk_dest: Option<Tile>,
    /// Slot threads set this when a traveller returns Arrived/Budget so
    /// [`Session::pump_status`] can clear [`Session::walk_dest`].
    walk_clear: Arc<AtomicBool>,
    /// Last `(gens.player, here)` ticked per username; skip until either
    /// changes so we do not re-send walk every 20 ms frame.
    tick_latch: Arc<Mutex<HashMap<String, (u64, Tile)>>>,
    /// WalkTo picker open flag; the picker window lands in Task 10.
    pub walkto_open: bool,
    /// Tile highlighted in the WalkTo picker; armed only on confirm.
    pub picker_sel: Option<Tile>,
    /// Nav config window open flag (non-modal, same as General config).
    pub nav_settings_open: bool,
    /// Non-modal settings window (renderer / capture / mem).
    pub global_settings_open: bool,
    /// Usernames we already sent `getvar tutorial` for this session.
    tutorial_getvar_sent: HashSet<String>,
    /// Forgotten-password confirm: delete the vault file (locked only).
    pub vault_reset_open: bool,
    pub vault_reset_understood: bool,
    /// Chooser ✕ waiting on the same confirm popup; `None` not pending.
    pub pending_profile_delete: Option<String>,
    pub delete_understood: bool,
    /// Live-harness overlay: the scenario's `NavSettings` for this session
    /// without writing prefs. `None` = operator `ui.nav`.
    pub nav_overlay: Option<NavSettings>,
    /// Optional headed live paint override; never persisted with operator UI.
    nav_paints_override: Option<bool>,
    /// Optional headed/live memory override; never persisted with operator UI.
    memory_override: Option<bool>,
    /// Per-frame nav-paint mirror the slot threads publish from each
    /// observe (see [`publish_nav_debug`]); `pump_status` re-copies it
    /// from `ui.nav` / [`Session::nav_overlay`] every UI frame.
    nav_publish: Arc<Mutex<NavPublishCfg>>,
    /// Overlay generation: bumped whenever the focused traveller's route
    /// can change (a new arm, or the focused profile switching). The path
    /// overlay rebuilds immediately on a bump instead of waiting for its
    /// 1 s raster cadence.
    route_gen: u64,
    mainland_sent: Arc<Mutex<HashSet<String>>>,
    /// Flat-model scatter: after scene 2, `::tele` each slot to a shuffled
    /// walkable tile (every slot is a full Client and seeds itself).
    scatter: Arc<AtomicBool>,
    options: PlayOptions,
    /// Immutable checked server/profile binding for production panel sessions.
    /// `None` exists only for legacy unit-test constructors.
    pub(crate) server_profile: Option<Arc<ServerProfile>>,
    /// Cache/interface/nav resources decoded once for every slot in this process.
    template: Option<Arc<SharedClientTemplate>>,
    /// Startup inputs retained until unlock/live boot. `None` is the explicit
    /// legacy unit-test path.
    pub(crate) profile_options: Option<ProfileOptions>,
    /// Ambient profile inputs captured once when production CLI options arrive.
    profile_environment: Option<ProfileEnvironment>,
    /// Monotonic identity for captured preparation work. Any pre-bind profile
    /// selection change invalidates an older worker completion.
    profile_generation: u64,
    /// Plain UI status only; preparation remains owned by the app worker.
    profile_preparing: bool,
    /// Interactive Unlock clicked before preparation/final validation finished.
    requested_unlock: Option<String>,
    /// One-use final resource validation proof consumed by `start_play`.
    validated_template: Option<ValidatedTemplate>,
    /// Multibox wall membership (chooser / latch / bulk ops). The UI reads
    /// it for the chooser and rail; [`Session`] methods drive it.
    pub wall: Wall,
    /// MultiBox toggle: rail (or grid) policy is up. `Focus.wall_open`
    /// mirrors this so extra rasters only run while the wall is visible.
    pub multibox: bool,
    /// Persisted panel prefs (last focus + per-profile collapsed sections).
    pub ui: crate::ui_state::PanelUiState,
    /// The script picked in Browse (compiled id or loaded JS card);
    /// `None` until one is selected. Selecting never Starts — Start is the
    /// section button. This is the focused heading; per-profile pending
    /// Browse lives in [`Session::pending_browse`].
    pub script_sel: Option<script::ScriptSel>,
    /// Per-profile Browse selection, never treated as last successful Start.
    pub pending_browse: HashMap<String, script::ScriptSel>,
    pub last_bulk_script_report: Option<String>,
    /// Load Starts whose isolate setup has not settled, by profile. The
    /// assignment and the load diagnostic are committed when it does.
    pub(crate) pending_starts: HashMap<String, crate::profile_script::PendingStart>,
    /// The last Start all report, kept so a member whose setup fails after
    /// the click moves from started to failed.
    pub(crate) bulk_start: Option<crate::profile_script::BulkStart>,
    pub reload_warning: Option<crate::profile_script::ReloadWarning>,
    /// Bound prepare/warn record for Reload or catalog Refresh confirm.
    pub pending_reload: Option<crate::profile_script::PendingReload>,
    /// Next catalog Refresh confirms a previously shown running/paused warning.
    pub catalog_refresh_confirm: bool,
    pub catalog_refresh_report: Option<String>,
    pub reload_generation: u64,
    /// Test-only: fail replacement Start for this profile after it passed
    /// eligibility and was stopped. Not a cancellation fixture.
    #[cfg(test)]
    pub fail_reload_start_for: Option<String>,
    /// Test-only: prepare wall/runner state with fake arms and no client
    /// thread. Preparation-state tests must not contact an update server.
    #[cfg(test)]
    skip_slot_spawn: bool,
    /// Catalog warmup: at most one `ensure_js` per armed frame.
    pub transpile_queue: VecDeque<(script::ScriptSource, String)>,
    pub(crate) transpile_armed: bool,
    pub transpile_done: usize,
    pub transpile_total: usize,
    /// Browse picker open flag (the Scripts window in `app.rs`).
    pub script_browse_open: bool,
    /// Load file browser open flag (the Load window in `app.rs`).
    pub script_load_open: bool,
    /// Current directory in the out-of-tree Load file browser.
    pub script_load_dir: PathBuf,
    /// Selected row in the Load file browser.
    pub script_load_sel: usize,
    /// Search box in the shared file dialog (Load or Import catalog).
    pub script_dialog_search: String,
    /// Shared file-dialog sort column.
    pub script_dialog_sort: crate::script_picker::DialogSort,
    pub script_dialog_sort_desc: bool,
    /// First-run rs2b0t clone-root folder picker (when `$RS2B0T` unset).
    pub rs2b0t_catalog_open: bool,
    /// Show **Not now** (first-run only; later Import catalog… does not defer).
    pub rs2b0t_catalog_defer_ok: bool,
    /// Current directory in the rs2b0t folder picker.
    pub rs2b0t_catalog_dir: PathBuf,
    /// Script Browse category filter (`None` = all categories).
    pub browse_category_filter: Option<String>,
    /// Persisted operator script-parameter overrides.
    pub script_settings: script::ScriptSettingsStore,
    /// Parameters editor modal open.
    /// Modal typed editors moved to Script prefs window (0.1.6).
    pub script_prefs_open: bool,
    /// Loadouts CRUD window (non-modal).
    pub loadouts_open: bool,
    /// Selected row in the loadouts list.
    pub loadouts_sel: usize,
    /// Working copy of the selected preset. Store/file change only on save.
    pub loadouts_draft: Option<script::Loadout>,
    /// Visible save/error feedback for the editor.
    pub loadouts_status: String,
    /// Item search box for the active slot or supply row.
    pub loadouts_search: String,
    /// Equipment slot currently picking an item.
    pub loadouts_search_slot: Option<String>,
    /// Supply row currently picking an item.
    pub loadouts_search_supply: Option<usize>,
    /// Per-supply quantity text buffers (stable across frames while editing).
    pub loadouts_qty_bufs: Vec<String>,
    /// Process-wide equipment/inventory presets.
    pub loadouts: script::LoadoutsStore,
    /// Scenario/live inject merged last on Start (Task 12 fills this).
    pub script_settings_inject: Option<serde_json::Map<String, serde_json::Value>>,
    /// The out-of-tree JS library (`~/.274bot/js-scripts.json`). Loaded
    /// cards appear in Browse and Start spawns their isolate.
    pub js: script::JsLibrary,
    /// True once the `$RS2B0T` catalog has been parsed for this session
    /// (first Load/Browse). Keeps the ambient env/persisted root out of
    /// boot and of every `Session::new`.
    pub(crate) rs2b0t_filled: bool,
    /// Shared `--live script_*` harness runner (Task 6): the slot thread
    /// ticks it from the per-frame hook (sends go through the slot's own
    /// `Client`), the UI frame reads its status/evidence. `None` when no
    /// scenario is live.
    pub scenario: Arc<Mutex<Option<scenario::ScenarioRunner>>>,
    /// Headed wait for [`scenario::ScenarioSettings::wait_script_stop`].
    pub(crate) live_script_stop_wait_started: Option<Instant>,
    /// Catalog card `live_prepare_script` stashes; the live pump Starts
    /// it once on [`scenario::StepKind::StartScript`].
    pending_script: Arc<Mutex<Vec<PendingCatalogStart>>>,
    /// Isolate-start handle the per-frame hook uses (filled after Play).
    script_start_handle: Arc<Mutex<Option<host_play::ScriptStartHandle>>>,
    /// Shared full-core observer handle, filled from Play and configured only
    /// by the catalog_watch executable.
    catalog_core_watch: Arc<Mutex<Option<host_play::catalog_core::CoreWatch>>>,
    catalog_core_enabled: bool,
    paired_core_watch: Arc<Mutex<Option<host_play::paired_core::PairWatch>>>,
    pair_core_enabled: bool,
    external_core_watch: Arc<Mutex<Option<host_play::external_loader::ExternalWatch>>>,
    external_core_enabled: bool,
    external_ts: Option<PathBuf>,
    /// Focused-slot speaker gate: at most one cpal speaker, owned by the
    /// focused slot while its Music/SFX toggle is on. `lowmem` (toggle
    /// off) never opens cpal; slot threads reconcile on their frame loop.
    audio: Arc<AudioGate<AudioOut>>,
    /// Whether focus/multibox writes land on disk prefs. `true` in
    /// `Session::new`; every `live_prepare_*` flips it off so an ephemeral
    /// live boot never touches the operator's `last_focus`.
    pub persist_ui: bool,
    /// Explicit PREPARE / RUN-PREPARED fixture path (default live is unchanged).
    pub fixture_mode: scenario::FixtureMode,
    /// Optional identity receipt path; default `~/.274bot/fixtures/<scenario>.json`.
    pub fixture_path: Option<PathBuf>,
}

/// Keep each per-name panel log bounded.
const LOG_CAP: usize = 200;
const SLOT_REMOVE_TIMEOUT: Duration = Duration::from_secs(10);

/// Log bucket for vault errors and lines with no username.
pub const PROCESS: &str = "*";

/// Append `line` under `name`, dropping from the front past [`LOG_CAP`].
fn push_log(map: &mut HashMap<String, Vec<String>>, name: &str, line: String) {
    if host::debug_enabled() {
        eprintln!("[panel] {name}: {line}");
    }
    let vec = map.entry(name.to_string()).or_default();
    vec.push(line);
    while vec.len() > LOG_CAP {
        vec.remove(0);
    }
}

pub(crate) fn external_loader_fixture() -> scenario::Scenario {
    use api::interact::cheat;
    use host_play::external_loader::{BONES_COUNT, PREP_DEADLINE, PREREQ_SHOT};
    use scenario::{Proof, Scenario, ScenarioSettings, Seed, Step, StepKind, Wait};
    Scenario {
        name: host_play::external_loader::LIVE_SCENARIO,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps: vec![
            Step {
                name: "prepare twenty-five carried bones",
                kind: StepKind::Perform {
                    send: Box::new(|c, _| {
                        cheat(c, "setvar tutorial 1000");
                        cheat(c, "getvar tutorial");
                        cheat(c, "give bones 25");
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Chat {
                        needle: "get tutorial: 1000",
                    },
                    budget_ticks: 200,
                },
            },
            Step {
                name: "relog so the inv tab binds",
                kind: StepKind::Relog,
                wait: Wait {
                    arm: Proof::SideTabAvailable { index: 3 },
                    budget_ticks: 600,
                },
            },
        ],
        proof: Proof::Item {
            name: "Bones",
            count: BONES_COUNT,
        },
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: PREP_DEADLINE,
            start_script: None,
            terminal_shot: Some(PREREQ_SHOT),
            nav: scenario::ScenarioNav::default().with_tick_ms(600),
            ..Default::default()
        },
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

/// The slot's incremental nav snapshot for walk follow. The observe path
/// already rebuilt this entry; hop ticks must not allocate fresh.
fn nav_snapshot_for_follow<'a>(
    states: &'a HashMap<String, (GameSnapshot, WorldState)>,
    name: &str,
) -> Option<&'a GameSnapshot> {
    states.get(name).map(|(snap, _)| snap)
}

type SlotTravellers = Arc<Mutex<HashMap<String, Arc<Mutex<WalkArm>>>>>;

fn reset_frontend_slot_session(
    name: &str,
    travellers: &SlotTravellers,
    tick_latch: &Arc<Mutex<HashMap<String, (u64, Tile)>>>,
) -> bool {
    tick_latch.lock().unwrap().remove(name);
    travellers.lock().unwrap().remove(name).is_some()
}

/// A slot spawn/removal is the ownership boundary for username reuse. Client
/// session generations are per-client, so they cannot distinguish a fresh slot
/// from the previous client that happened to use the same name.
fn reset_frontend_slot_lifetime(
    name: &str,
    gens: &Arc<Mutex<HashMap<String, ClientGens>>>,
    states: &Arc<Mutex<HashMap<String, (GameSnapshot, WorldState)>>>,
    travellers: &SlotTravellers,
    tick_latch: &Arc<Mutex<HashMap<String, (u64, Tile)>>>,
) -> bool {
    gens.lock().unwrap().remove(name);
    states.lock().unwrap().remove(name);
    reset_frontend_slot_session(name, travellers, tick_latch)
}

/// Publish facts at the same session watermark as the production host and
/// cancel externally armed work before any local-player/Guardian early return.
fn publish_frontend_slot(
    name: &str,
    client: &Client,
    gens: &Arc<Mutex<HashMap<String, ClientGens>>>,
    states: &Arc<Mutex<HashMap<String, (GameSnapshot, WorldState)>>>,
    travellers: &SlotTravellers,
    tick_latch: &Arc<Mutex<HashMap<String, (u64, Tile)>>>,
) -> bool {
    let mut gens_guard = gens.lock().unwrap();
    let last = gens_guard.entry(name.to_string()).or_default();
    let mut state_guard = states.lock().unwrap();
    let slot = state_guard
        .entry(name.to_string())
        .or_insert_with(|| (GameSnapshot::new(), WorldState::empty()));
    let publication = host::Host::publish_frontend_snapshot(last, &mut slot.0, client);
    if publication.changed {
        slot.1 = WorldState::from_snapshot(&slot.0);
    }
    drop(state_guard);
    if publication.session_boundary {
        reset_frontend_slot_session(name, travellers, tick_latch);
    }
    publication.session_boundary
}

impl Session {
    /// Empty session: no vault, no slots, default `PlayOptions` (same engine
    /// defaults as the host-play CLI). Unlock via [`Session::unlock`].
    pub fn new() -> Self {
        #[cfg(test)]
        script::IsolatedEnv::ensure_thread();
        let ui = crate::ui_state::load();
        let capture_pref = ui.capture;
        Self {
            focus: Arc::new(Mutex::new(crate::focus::Focus {
                focused: None,
                renderer: true,
                game_pane_open: true,
                capture: capture_pref,
                only_render_selected: true,
                sidecar_50: false,
                live_full_rate: false,
                focused_50: true,
                wall_open: false,
                wall: Vec::new(),
                renderer_by: HashMap::new(),
            })),
            vault: None,
            error: None,
            play: None,
            slots: HashMap::new(),
            pending_slot_removals: HashMap::new(),
            capture_tx: None,
            mainland: Arc::new(AtomicBool::new(
                env::var("BOT_MAINLAND").as_deref() == Ok("1"),
            )),
            log_by: Arc::new(Mutex::new(HashMap::new())),
            pass_scratch: String::new(),
            statuses: Vec::new(),
            cred_user: String::new(),
            cred_pass: String::new(),
            cred_settings: ProfileSettings::default(),
            chooser_edit: None,
            travellers: Arc::new(Mutex::new(HashMap::new())),
            script_nav_paint: Arc::new(Mutex::new(None)),
            nav_states: Arc::new(Mutex::new(HashMap::new())),
            frontend_gens: Arc::new(Mutex::new(HashMap::new())),
            walk_dest: None,
            walk_clear: Arc::new(AtomicBool::new(false)),
            tick_latch: Arc::new(Mutex::new(HashMap::new())),
            walkto_open: false,
            picker_sel: None,
            nav_settings_open: false,
            global_settings_open: false,
            tutorial_getvar_sent: HashSet::new(),
            vault_reset_open: false,
            vault_reset_understood: false,
            pending_profile_delete: None,
            delete_understood: false,
            nav_overlay: None,
            nav_paints_override: None,
            memory_override: None,
            nav_publish: Arc::new(Mutex::new(NavPublishCfg::default())),
            route_gen: 0,
            mainland_sent: Arc::new(Mutex::new(HashSet::new())),
            scatter: Arc::new(AtomicBool::new(false)),
            wall: Wall::default(),
            multibox: false,
            ui,
            script_sel: None,
            pending_browse: HashMap::new(),
            last_bulk_script_report: None,
            pending_starts: HashMap::new(),
            bulk_start: None,
            reload_warning: None,
            pending_reload: None,
            catalog_refresh_confirm: false,
            catalog_refresh_report: None,
            reload_generation: 0,
            #[cfg(test)]
            fail_reload_start_for: None,
            #[cfg(test)]
            skip_slot_spawn: false,
            transpile_queue: VecDeque::new(),
            transpile_armed: false,
            transpile_done: 0,
            transpile_total: 0,
            script_browse_open: false,
            script_load_open: false,
            script_load_dir: crate::script_picker::default_load_browse_dir(None),
            script_load_sel: 0,
            script_dialog_search: String::new(),
            script_dialog_sort: crate::script_picker::DialogSort::Name,
            script_dialog_sort_desc: false,
            rs2b0t_catalog_open: false,
            rs2b0t_catalog_defer_ok: false,
            rs2b0t_catalog_dir: crate::script_picker::default_load_browse_dir(None),
            browse_category_filter: None,
            script_settings: script::ScriptSettingsStore::with_default_path(),
            script_prefs_open: false,
            loadouts_open: false,
            loadouts_sel: 0,
            loadouts_draft: None,
            loadouts_status: String::new(),
            loadouts_search: String::new(),
            loadouts_search_slot: None,
            loadouts_search_supply: None,
            loadouts_qty_bufs: Vec::new(),
            loadouts: script::LoadoutsStore::with_default_path(),
            script_settings_inject: None,
            js: {
                let mut js = script::JsLibrary::new(script::default_js_store());
                let _ = js.restore(); // missing/broken store is not fatal here
                js
            },
            rs2b0t_filled: false,
            scenario: Arc::new(Mutex::new(None)),
            live_script_stop_wait_started: None,
            pending_script: Arc::new(Mutex::new(Vec::new())),
            script_start_handle: Arc::new(Mutex::new(None)),
            catalog_core_watch: Arc::new(Mutex::new(None)),
            catalog_core_enabled: false,
            paired_core_watch: Arc::new(Mutex::new(None)),
            pair_core_enabled: false,
            external_core_watch: Arc::new(Mutex::new(None)),
            external_core_enabled: false,
            external_ts: None,
            audio: Arc::new(AudioGate::new()),
            persist_ui: true,
            fixture_mode: scenario::FixtureMode::Default,
            fixture_path: None,
            options: {
                let (host, port) = host_play::play_endpoint_for(client::bot_target());
                PlayOptions {
                    host,
                    port,
                    cache_dir: default_cache_dir(),
                    lowmem: true,
                    // Panel per_frame queues hop from Session.mainland (env);
                    // spawn-time PlayOptions.mainland stays false.
                    mainland: false,
                }
            },
            server_profile: None,
            template: None,
            profile_options: None,
            profile_environment: None,
            profile_generation: 0,
            profile_preparing: false,
            requested_unlock: None,
            validated_template: None,
        }
    }

    /// Enable full-core proof only for the dedicated catalog_watch entry.
    pub fn set_catalog_core_enabled(&mut self, enabled: bool) {
        self.catalog_core_enabled = enabled;
    }

    /// Apply a session-only memory choice to every slot, including companions.
    pub fn set_memory_override(&mut self, memory_override: Option<bool>) {
        self.memory_override = memory_override;
    }

    /// Clone the Play-owned proof handle without exposing slot snapshots.
    pub fn catalog_core_watch(&self) -> Option<host_play::catalog_core::CoreWatch> {
        self.catalog_core_watch.lock().unwrap().clone()
    }

    pub(crate) fn install_catalog_core_watch(
        &self,
        watch: Option<host_play::catalog_core::CoreWatch>,
    ) {
        *self.catalog_core_watch.lock().unwrap() = watch;
    }

    /// Enable pair proof only for the dedicated pair_watch entry.
    pub fn set_pair_core_enabled(&mut self, enabled: bool) {
        self.pair_core_enabled = enabled;
    }

    pub fn paired_core_watch(&self) -> Option<host_play::paired_core::PairWatch> {
        self.paired_core_watch.lock().unwrap().clone()
    }

    pub(crate) fn install_paired_core_watch(
        &self,
        watch: Option<host_play::paired_core::PairWatch>,
    ) {
        *self.paired_core_watch.lock().unwrap() = watch;
    }

    pub fn set_external_core_enabled(&mut self, enabled: bool) {
        self.external_core_enabled = enabled;
    }

    pub fn external_core_enabled(&self) -> bool {
        self.external_core_enabled
    }

    pub fn set_external_ts(&mut self, path: Option<PathBuf>) {
        self.external_ts = path;
    }

    /// Configure the optional prepare / run-prepared fixture path for the next live boot.
    pub fn set_fixture_boot(&mut self, mode: scenario::FixtureMode, path: Option<PathBuf>) {
        self.fixture_mode = mode;
        self.fixture_path = path;
    }

    /// Resolve the identity receipt path for `scenario`.
    pub fn fixture_identity_path(&self, scenario: &str) -> PathBuf {
        self.fixture_path
            .clone()
            .unwrap_or_else(|| scenario::default_fixture_path(scenario))
    }

    pub fn external_core_watch(&self) -> Option<host_play::external_loader::ExternalWatch> {
        self.external_core_watch.lock().unwrap().clone()
    }

    pub(crate) fn install_external_core_watch(
        &self,
        watch: Option<host_play::external_loader::ExternalWatch>,
    ) {
        *self.external_core_watch.lock().unwrap() = watch;
    }

    pub fn pump_external_loader(&mut self) {
        use host_play::external_loader::{
            apply_harmless_whitespace, source_sha256, Operation, BONES_ID, PRAYER_STAT_ID,
            SCRIPT_NAME,
        };
        let Some(watch) = self.external_core_watch() else {
            return;
        };
        if !watch.configured() {
            return;
        }
        let now = Instant::now();
        watch.poll_deadlines(now);
        let account = watch.account();
        let name = self.focused_name().unwrap_or_else(|| account.clone());
        if name == account {
            if let Some(slot) = self.statuses.iter().find(|s| s.username == name) {
                watch.note_scene(slot.ingame, slot.scene_state);
                watch.note_paint(slot.script_paint.is_some());
            }
            if let Some((snap, _)) = self.nav_states.lock().unwrap().get(&name) {
                let bones: i32 = snap
                    .inventory()
                    .iter()
                    .filter(|item| item.def.id == BONES_ID)
                    .map(|item| item.count)
                    .sum();
                let prayer = snap
                    .stats()
                    .iter()
                    .find(|stat| stat.index == PRAYER_STAT_ID)
                    .map(|stat| stat.xp)
                    .unwrap_or(0);
                watch.note_inventory(now, &account, bones, prayer);
            }
        }
        if watch.status() == host_play::external_loader::ExternalWatchStatus::Failed {
            let state = self
                .play
                .as_ref()
                .map(|play| play.script_state(&account))
                .unwrap_or(script::RunState::Idle);
            let idle = matches!(state, script::RunState::Idle);
            let mut stop_invoked = false;
            if !idle && !matches!(state, script::RunState::Stopping) {
                if let Some(play) = self.play.as_ref() {
                    play.script_stop(&account);
                }
                stop_invoked = true;
            }
            watch.note_cleanup_progress(now, idle, stop_invoked);
            return;
        }
        match watch.dispatch_operation() {
            Some(Operation::LoadRawTs) => {
                let path = watch.source_path();
                self.load_js(&path);
                if let Some(error) = self.error.clone() {
                    watch.fail(format!("load: {error}"));
                    return;
                }
                let lookup = match &self.script_sel {
                    Some(script::ScriptSel::Loaded(_, lookup)) => lookup.clone(),
                    _ => path.to_string_lossy().into_owned(),
                };
                if let Err(error) = self.js.ensure_js(script::ScriptSource::File, &lookup) {
                    watch.fail(format!("transpile: {error}"));
                    return;
                }
                let Some(card) = self
                    .js
                    .get(script::ScriptSource::File, &lookup)
                    .or_else(|| {
                        self.js
                            .get(script::ScriptSource::File, &path.display().to_string())
                    })
                else {
                    watch.fail("external loader load did not produce a File card");
                    return;
                };
                let count = self
                    .js
                    .cards()
                    .iter()
                    .filter(|c| {
                        c.source == script::ScriptSource::File
                            && (c.identity_key() == card.identity_key() || c.name == SCRIPT_NAME)
                    })
                    .count();
                let want =
                    script::ScriptSel::Loaded(script::ScriptSource::File, card.identity_id());
                let selected = self.script_sel.as_ref() == Some(&want)
                    || self.pending_browse.get(&account) == Some(&want);
                let running = self.play.as_ref().is_some_and(|play| {
                    !matches!(play.script_state(&account), script::RunState::Idle)
                });
                watch.note_load(
                    count,
                    &card.name,
                    &card.path,
                    &card.identity_key(),
                    &card.sha256,
                    selected,
                    running,
                );
            }
            Some(Operation::Start) => {
                if watch.begin_start(now).is_err() {
                    return;
                }
                self.script_start_selected();
                if let Some(error) = self.error.clone() {
                    watch.fail_start(error);
                }
            }
            Some(Operation::Stop) => {
                let state = self
                    .play
                    .as_ref()
                    .map(|play| play.script_state(&account))
                    .unwrap_or(script::RunState::Idle);
                let paint = self
                    .statuses
                    .iter()
                    .find(|s| s.username == account)
                    .is_some_and(|s| s.script_paint.is_some());
                match state {
                    script::RunState::Idle => watch.note_stop(now, true, paint),
                    script::RunState::Stopping => watch.note_stop(now, false, paint),
                    _ => {
                        self.script_stop();
                        watch.note_stop(now, false, paint);
                    }
                }
            }
            Some(Operation::ReloadUnchanged) => match self.script_reload_clicked() {
                crate::profile_script::ReloadOutcome::NothingChanged => {
                    watch.note_reload_unchanged(host_play::external_loader::NOTHING_CHANGED);
                }
                other => watch.fail(format!("unchanged reload: {other:?}")),
            },
            Some(Operation::ReloadChanged) => {
                let path = watch.source_path();
                let before_bytes = match std::fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        watch.fail(format!("changed reload read {}: {error}", path.display()));
                        return;
                    }
                };
                let source_before = source_sha256(&before_bytes);
                let compiled_before = self
                    .js
                    .get(script::ScriptSource::File, &path.display().to_string())
                    .map(|c| c.sha256.clone())
                    .unwrap_or_default();
                if let Err(error) = apply_harmless_whitespace(&path) {
                    watch.fail(error);
                    return;
                }
                let after_bytes = match std::fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        watch.fail(format!("changed reload reread {}: {error}", path.display()));
                        return;
                    }
                };
                let source_after = source_sha256(&after_bytes);
                let outcome = self.script_reload(true);
                let (applied, nothing_changed) = match &outcome {
                    crate::profile_script::ReloadOutcome::Applied { .. } => (true, false),
                    crate::profile_script::ReloadOutcome::NothingChanged => (false, true),
                    crate::profile_script::ReloadOutcome::Failed(error) => {
                        watch.fail(format!("changed reload: {error}"));
                        return;
                    }
                    other => {
                        watch.fail(format!("changed reload: {other:?}"));
                        return;
                    }
                };
                let Some(card) = self
                    .js
                    .get(script::ScriptSource::File, &path.display().to_string())
                else {
                    watch.fail("external loader changed reload lost the File card");
                    return;
                };
                let count = self
                    .js
                    .cards()
                    .iter()
                    .filter(|c| {
                        c.source == script::ScriptSource::File
                            && (c.identity_key() == card.identity_key() || c.name == SCRIPT_NAME)
                    })
                    .count();
                let want =
                    script::ScriptSel::Loaded(script::ScriptSource::File, card.identity_id());
                let selected = self.script_sel.as_ref() == Some(&want)
                    || self.pending_browse.get(&account) == Some(&want);
                let running = self.play.as_ref().is_some_and(|play| {
                    !matches!(play.script_state(&account), script::RunState::Idle)
                });
                watch.note_reload_changed(
                    count,
                    applied,
                    nothing_changed,
                    &card.path,
                    &card.identity_key(),
                    &source_before,
                    &source_after,
                    &compiled_before,
                    &card.sha256,
                    selected,
                    running,
                );
            }
            _ => {}
        }
    }

    /// Install production launch inputs while the locked panel still exposes
    /// its saved revision selector. Resolution is deferred until unlock/live boot.
    pub fn configure_profile(&mut self, options: ProfileOptions) -> Result<(), String> {
        if self.server_profile.is_some() {
            return Err("server profile is already bound; restart to change revision".into());
        }
        self.profile_options = Some(options);
        self.profile_environment = Some(ProfileEnvironment::capture());
        self.profile_generation = self.profile_generation.wrapping_add(1);
        Ok(())
    }

    pub(crate) fn profile_generation(&self) -> u64 {
        self.profile_generation
    }

    pub(crate) fn profile_preparation(&self) -> Result<(u64, ProfilePreparation), String> {
        let options = self
            .profile_options
            .clone()
            .ok_or_else(|| "no production server profile was configured".to_string())?;
        let environment = self
            .profile_environment
            .clone()
            .unwrap_or_else(ProfileEnvironment::capture);
        Ok((
            self.profile_generation,
            ProfilePreparation {
                options,
                environment,
                saved_revision: self.ui.server_revision,
            },
        ))
    }

    pub(crate) fn finish_profile_preparation(
        &mut self,
        generation: u64,
        result: Result<Arc<SharedClientTemplate>, String>,
    ) -> ProfilePreparationCompletion {
        self.profile_preparing = false;
        if generation != self.profile_generation {
            return ProfilePreparationCompletion::Stale;
        }
        match result {
            Ok(template) => {
                self.install_prepared_template(template);
                ProfilePreparationCompletion::Installed
            }
            Err(error) => {
                self.error = Some(error);
                ProfilePreparationCompletion::Failed
            }
        }
    }

    fn install_prepared_template(&mut self, template: Arc<SharedClientTemplate>) {
        let profile = Arc::clone(template.profile());
        crate::picker::set_navflags_binding(
            profile.nav_flags().to_path_buf(),
            profile
                .nav_identity()
                .and_then(|identity| identity.flags_sha256.clone()),
            profile.nav_flags_origin().is_bundled(),
        );
        if let Some(world) = profile.world() {
            crate::picker::set_reach_binding(
                profile.reach(),
                world.collision.origin,
                world.collision.width,
                world.collision.height,
                profile.nav_origin().is_bundled(),
            );
        } else {
            crate::picker::set_reach_binding(
                None,
                api::snapshot::WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                0,
                0,
                false,
            );
        }
        self.options = PlayOptions {
            host: profile.client().game_host().to_string(),
            port: profile.client().game_port(),
            cache_dir: profile.client().cache_dir().display().to_string(),
            lowmem: true,
            mainland: false,
        };
        self.server_profile = Some(profile);
        self.template = Some(template);
        self.error = None;
    }

    pub(crate) fn template_for_validation(&self) -> Result<Arc<SharedClientTemplate>, String> {
        self.template
            .clone()
            .ok_or_else(|| "server profile is not prepared".to_string())
    }

    pub(crate) fn install_validated_template(
        &mut self,
        validated: ValidatedTemplate,
    ) -> Result<(), String> {
        let Some(template) = self.template.as_ref() else {
            return Err("server profile is not prepared".into());
        };
        if !Arc::ptr_eq(template, validated.template()) {
            return Err("validated template does not match the active server profile".into());
        }
        self.validated_template = Some(validated);
        Ok(())
    }

    pub(crate) fn set_profile_preparing(&mut self, preparing: bool) {
        self.profile_preparing = preparing;
    }

    pub(crate) fn profile_preparing(&self) -> bool {
        self.profile_preparing
    }

    pub(crate) fn request_unlock(&mut self, pass: String) {
        self.requested_unlock = Some(pass);
    }

    pub(crate) fn take_requested_unlock(&mut self) -> Option<String> {
        self.requested_unlock.take()
    }

    pub(crate) fn resolve_profile(&self) -> Result<host_play::ProfileSelection, String> {
        let options = self
            .profile_options
            .as_ref()
            .ok_or_else(|| "no production server profile was configured".to_string())?;
        let env = self
            .profile_environment
            .as_ref()
            .cloned()
            .unwrap_or_else(ProfileEnvironment::capture);
        options.resolve_with_env(Some(self.ui.server_revision), &env)
    }

    /// Resolve and freeze the panel's process profile before vault mutation.
    pub fn bind_profile(&mut self) -> Result<(), String> {
        let env = self
            .profile_environment
            .clone()
            .unwrap_or_else(ProfileEnvironment::capture);
        self.bind_profile_with_env(&env)
    }

    fn bind_profile_with_env(
        &mut self,
        env: &host_play::profile::ProfileEnvironment,
    ) -> Result<(), String> {
        if self.server_profile.is_some() {
            return Err("server profile is already bound; restart to change revision".into());
        }
        let options = self
            .profile_options
            .as_ref()
            .ok_or_else(|| "no production server profile was configured".to_string())?;
        let template = options
            .resolve_with_env(Some(self.ui.server_revision), env)?
            .prepare_template()?;
        self.install_prepared_template(template);
        Ok(())
    }

    pub fn ensure_profile_bound(&mut self) -> Result<(), String> {
        if self.server_profile.is_none() && self.profile_options.is_some() {
            self.bind_profile()?;
        }
        Ok(())
    }

    pub fn require_bot_operation(&self) -> Result<(), String> {
        self.server_profile
            .as_ref()
            .map_or(Ok(()), |profile| profile.require_bot_operation())
    }

    pub fn set_server_revision(&mut self, revision: u16) -> Result<(), String> {
        if self.server_profile.is_some() {
            return Err("server profile is already bound; restart to change revision".into());
        }
        if !matches!(revision, 274 | 289) {
            return Err(format!("unsupported revision {revision}; use 274 or 289"));
        }
        self.ui.server_revision = revision;
        self.profile_generation = self.profile_generation.wrapping_add(1);
        crate::ui_state::save(&self.ui);
        Ok(())
    }

    pub fn profile_bound(&self) -> bool {
        self.server_profile.is_some()
    }

    pub fn server_label(&self) -> String {
        if let Some(profile) = &self.server_profile {
            return profile.label();
        }
        self.profile_options.as_ref().map_or_else(
            || "legacy local-274 · revision 274".into(),
            |_| {
                self.resolve_profile().map_or_else(
                    |error| format!("invalid profile: {error}"),
                    |profile| profile.label(),
                )
            },
        )
    }

    pub fn app_title(&self) -> String {
        self.effective_revision_label()
            .map_or_else(|_| "bot".into(), |revision| format!("{revision}bot"))
    }

    pub fn effective_revision_label(&self) -> Result<String, String> {
        if let Some(profile) = &self.server_profile {
            return Ok(profile.revision().as_i32().to_string());
        }
        match self.profile_options.as_ref() {
            Some(_) => self
                .resolve_profile()
                .map(|selection| selection.revision().as_i32().to_string()),
            None => Ok(self.ui.server_revision.to_string()),
        }
    }

    pub(crate) fn target(&self) -> client::BotTarget {
        self.server_profile
            .as_ref()
            .map_or_else(client::bot_target, |profile| profile.target())
    }

    fn vault_path(&self) -> Result<PathBuf, String> {
        if let Some(profile) = &self.server_profile {
            return Ok(profile.vault_path().to_path_buf());
        }
        match self.profile_options.as_ref() {
            Some(_) => self
                .resolve_profile()
                .map(|selection| selection.vault_path().to_path_buf()),
            None => Ok(default_vault_path()),
        }
    }

    pub fn play_options(&self) -> &PlayOptions {
        &self.options
    }

    /// Unlock (or first-run create) the default vault and start the play.
    pub fn unlock(&mut self, pass: &str) -> bool {
        let path = match self.vault_path() {
            Ok(path) => path,
            Err(error) => {
                self.error = Some(format!("server profile: {error}"));
                return false;
            }
        };
        self.unlock_at(&path, pass)
    }

    /// Unlock (or first-run create) the vault at `path` and start the play.
    /// Only the focused profile is spawned as a slot; other vault rows stay
    /// parked until selected (select keeps a slot once it has run).
    pub fn unlock_at(&mut self, path: &Path, pass: &str) -> bool {
        if self.start_vault(path, pass) {
            self.focus_first_profile();
            true
        } else {
            false
        }
    }

    /// Whether the default vault file exists (locked prompt: Unlock vs Create).
    pub fn default_vault_exists(&self) -> Result<bool, String> {
        self.vault_path().map(|path| path.is_file())
    }

    /// Delete `path` while locked. Refuses if a vault is open so a running
    /// session cannot clobber the file. Forgotten-password recovery.
    pub fn reset_vault_at(&mut self, path: &Path) -> bool {
        if self.vault.is_some() {
            self.error = Some("reset vault: unlock / close the session first".into());
            return false;
        }
        match Vault::reset_file(path) {
            Ok(()) => {
                self.error = None;
                self.vault_reset_open = false;
                self.vault_reset_understood = false;
                true
            }
            Err(e) => {
                self.error = Some(format!("reset vault: {e}"));
                false
            }
        }
    }

    pub fn reset_vault(&mut self) -> bool {
        let path = match self.vault_path() {
            Ok(path) => path,
            Err(error) => {
                self.error = Some(format!("server profile: {error}"));
                return false;
            }
        };
        self.reset_vault_at(&path)
    }

    /// Open the vault and attach an empty [`Play`]. Does **not** spawn a
    /// slot — the boot spawns the focused profile after this; MultiBox
    /// spawns the wall members.
    fn start_vault(&mut self, path: &Path, pass: &str) -> bool {
        if let Err(msg) = self
            .ensure_profile_bound()
            .and_then(|()| self.require_bot_operation())
        {
            self.error = Some(msg);
            return false;
        }
        match open_vault(path, pass) {
            Ok(vault) => {
                self.error = None;
                match self.start_play(vault) {
                    Ok(()) => true,
                    Err(msg) => {
                        push_log(
                            &mut self.log_by.lock().unwrap(),
                            PROCESS,
                            format!("profile: {msg}"),
                        );
                        self.error = Some(msg);
                        false
                    }
                }
            }
            Err(e) => {
                let msg = e.to_string();
                push_log(
                    &mut self.log_by.lock().unwrap(),
                    PROCESS,
                    format!("vault: {msg}"),
                );
                self.error = Some(msg);
                false
            }
        }
    }

    /// Live `null_raster` setup: temp vault with `test`/`test2`, multibox
    /// wall of both, only-render-selected + focus `test`, renderer on,
    /// then `login_all`. Slot threads keep using real `Focus` → `set_draw`.
    pub fn live_prepare_null_raster(&mut self) -> Result<(), String> {
        self.persist_ui = false;
        let pass = host_play::live_vault_passphrase_for(self.target());
        let entries = null_raster_live_entries_for_target(self.target());
        let entry_refs: Vec<(&str, &str)> = entries
            .iter()
            .map(|(u, p)| (u.as_str(), p.as_str()))
            .collect();
        let path = temp_live_vault_from(&entry_refs, 274_000_001, &pass, true);
        if !self.unlock_at(&path, &pass) {
            return Err(self
                .error
                .clone()
                .unwrap_or_else(|| "unlock_at failed".into()));
        }
        self.set_multibox(true);
        self.scatter.store(false, Ordering::Relaxed);
        // First MultiBox-on opens the chooser; live already loaded both
        // names. Leave the window usable (operator may click the rail).
        self.wall.chooser_open = false;
        self.load("test");
        self.load("test2");
        self.focus.lock().unwrap().only_render_selected = true;
        self.select("test");
        self.set_renderer(true);
        self.login_all();
        Ok(())
    }

    #[cfg(feature = "memory-profile")]
    pub fn prepare_memory(&mut self, run: &host_play::memory::Run) -> Result<(), String> {
        self.persist_ui = false;
        self.mainland.store(true, Ordering::Relaxed);
        self.scatter.store(false, Ordering::Relaxed);
        if !self.start_vault(&run.vault, &run.pass) {
            return Err(self
                .error
                .clone()
                .unwrap_or_else(|| "benchmark vault failed".into()));
        }
        run.bind_seed_nav(host_play::memory::SeedNav::FromPlay(
            self.play.as_ref().and_then(|p| p.world()),
        ))?;
        self.set_multibox(true);
        for name in &run.names {
            let _ = self.wall.load(name);
            self.ensure_slot(name, self.arm_for_profile(name), false);
        }
        self.sync_wall_focus();
        self.wall.chooser_open = false;
        self.memory_focus(run);
        self.login_all();
        Ok(())
    }

    #[cfg(feature = "memory-profile")]
    pub fn memory_focus(&mut self, run: &host_play::memory::Run) {
        let index = run.focus_index();
        let name = &run.names[index];
        if self.focus.lock().unwrap().focused.as_ref() != Some(name) {
            self.select(name);
        }
        let mut focus = self.focus.lock().unwrap();
        crate::focus::memory_draw_policy(&mut focus, &run.names, run.render_policy);
    }

    /// Live `stress50` RAM watch: temp vault `s00`…`s49` (password =
    /// username, uids `274_000_100 + i`). Every member is a full `Client`
    /// (flat model — no lean extras / channel-head). `s00` is FIFO head +
    /// focus, MultiBox rail with only-render-selected (cap-only: Game
    /// paints at focused 50 fps, rail skip-paint so 50 blits do not melt
    /// RAM/GPU), scatter-seed after scene 2, chooser closed, `login_all`.
    /// Run headed with `cargo run --release` — debug 50-heads spike RAM.
    pub fn live_prepare_stress50(&mut self) -> Result<(), String> {
        warn_stress50_debug();
        self.live_prepare_stress(50, false)
    }

    /// Live `stress50_full`: same 50-head wall as [`live_prepare_stress50`],
    /// but every wall member draws (only-render-selected off) at the live
    /// full-rate overlay (Game + sidecar 50 fps). RAM/GPU explosion check
    /// after `stress50` holds. Still `--release`.
    pub fn live_prepare_stress50_full(&mut self) -> Result<(), String> {
        warn_stress50_debug();
        self.live_prepare_stress(50, true)
    }

    /// Headed flat wall of `n` profiles (`s00`…`s{n-1}`). `full_rate`
    /// paints every member at 50 fps; otherwise cap-only RAM watch.
    fn live_prepare_stress(&mut self, n: usize, full_rate: bool) -> Result<(), String> {
        self.persist_ui = false;
        let n = n.max(1);
        let names = stress_live_entries_for_target(n, self.target());
        let entries: Vec<(&str, &str)> = names
            .iter()
            .map(|(u, p)| (u.as_str(), p.as_str()))
            .collect();
        let pass = host_play::live_vault_passphrase_for(self.target());
        let path = temp_live_vault_from(&entries, 274_000_100, &pass, true);
        // Empty Play first: do not spawn last_focus before s00 focuses.
        if !self.start_vault(&path, &pass) {
            return Err(self
                .error
                .clone()
                .unwrap_or_else(|| "start_vault failed".into()));
        }
        // RAM watch: only s00 (Game pane) may grow a GPU head. Rail members
        // stay raster Off so a flipped only-render-selected cannot attach
        // 49 extra RenderWorlds (~1 GB of loc Model clones each). Full-rate
        // keeps Gpu on every member on purpose.
        if let Some(vault) = self.vault.as_mut() {
            for (i, (name, _)) in names.iter().enumerate() {
                if let Some(mut p) = vault.get(name).cloned() {
                    p.settings.lowmem = true;
                    p.settings.raster = if full_rate || i == 0 {
                        vault::RasterMode::Gpu
                    } else {
                        vault::RasterMode::Off
                    };
                    let _ = vault.upsert(p);
                }
            }
        }
        self.set_multibox(true);
        self.scatter.store(true, Ordering::Relaxed);
        self.wall.chooser_open = false;
        {
            let mut f = self.focus.lock().unwrap();
            f.only_render_selected = !full_rate;
        }
        // Spawn every member onto the wall without applying focus on each
        // load (that would demote/promote raster/mem and join `maininit`
        // on the UI thread — the window never presents). Raster/mem flips
        // are drop+reattach now, but focus apply still joins handshakes.
        // s00 is focused last so it is FIFO head.
        for (name, _) in &names {
            let _ = self.wall.load(name);
            self.ensure_slot(name, self.arm_for_profile(name), false);
        }
        self.sync_wall_focus();
        self.wall.chooser_open = false;
        self.select(&names[0].0);
        // NEVER assign sidecar_50 — it stays the operator knob. The live
        // overlay raises every *drawing* slot, Game included.
        self.set_live_full_rate(full_rate);
        self.sync_sidecar_cadence();
        self.login_all();
        Ok(())
    }

    /// Live `script_<name>` setup: temp vault with the scenario's seed
    /// profiles minted to unique per-run usernames (the engine
    /// auto-registers unknown names, so a live boot never logs the shared
    /// `test` account — player saves accumulate under the engine's
    /// `player/` dir, wipe it to reset), mainland hop per the seed,
    /// single-client boot (the MultiBox wall for a fleet — more than one
    /// seed profile — so the sidecar rail pops out and every bot is
    /// visible), and the shared [`scenario::ScenarioRunner`] installed for
    /// the slot thread's per-frame hook. The UI frame reads the runner's
    /// status/evidence.
    pub fn live_prepare_script(&mut self, scenario: scenario::Scenario) -> Result<(), String> {
        // Ephemeral boot: never persist focus/last_focus from a live run.
        self.persist_ui = false;
        // Offline prepare never boots live — refuse here so a miswired path fails closed.
        if matches!(self.fixture_mode, scenario::FixtureMode::Prepare) {
            return Err(
                "FixtureMode::Prepare is offline-only; use panel --prepare-fixture (no live boot)"
                    .into(),
            );
        }
        // Apply run-prepared rewrite (strips mainland/setup cheats) before seed.
        let scenario = scenario::apply_fixture_mode(scenario, self.fixture_mode)?;
        if matches!(self.fixture_mode, scenario::FixtureMode::RunPrepared)
            && scenario::run_prepared_has_setup_cheats(&scenario)
        {
            return Err(format!(
                "run-prepared scenario {:?} still has setup cheats after rewrite",
                scenario.name
            ));
        }
        // Copy the view knobs before `scenario` moves into the runner.
        let view = scenario.settings.clone();
        let scenario_name = scenario.name.to_string();
        let profile_count = scenario.seed.profiles.len();
        let target = self.target();
        // Run-prepared reuses the exact identity receipt (no fresh random suffix).
        // Default live still mints.
        let (names, entries, pass, auto_login) = match self.fixture_mode {
            scenario::FixtureMode::RunPrepared => {
                let path = self.fixture_identity_path(&scenario_name);
                let identity = scenario::FixtureIdentity::read_from(&path)?;
                identity.validate_for(&scenario_name, profile_count)?;
                // Install .sav into engine players dir when the operator points one.
                if let Ok(players) = std::env::var("BOT_PLAYERS_DIR") {
                    let players = PathBuf::from(players);
                    let installed = identity.install_into_players_dir(&players, true)?;
                    for p in &installed {
                        println!("[panel] installed fixture save -> {}", p.display());
                    }
                } else {
                    println!(
                        "[panel] run-prepared: BOT_PLAYERS_DIR unset; ensure engine already has {}",
                        identity
                            .accounts
                            .iter()
                            .map(|a| a.sav_path.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                let names = identity.usernames();
                let entries = identity.entries();
                let pass = identity.vault_passphrase.clone();
                (names, entries, pass, true)
            }
            scenario::FixtureMode::Default => {
                let names = host_play::mint_live_names(profile_count);
                let entries = host_play::mint_live_entries_for_target(&names, target);
                let pass = host_play::live_vault_passphrase_for(target);
                (names, entries, pass, true)
            }
            scenario::FixtureMode::Prepare => unreachable!("prepare rejected above"),
        };
        if self.catalog_core_enabled && self.pair_core_enabled {
            return Err("catalog core and pair core watches are mutually exclusive".into());
        }
        if self.external_core_enabled && (self.catalog_core_enabled || self.pair_core_enabled) {
            return Err(
                "external loader watch is mutually exclusive with catalog/pair core".into(),
            );
        }
        let core_case = if self.catalog_core_enabled {
            if names.len() != 1 {
                return Err("catalog core watch supports exactly one driven slot".into());
            }
            Some(host_play::catalog_core::CoreCase::parse(scenario.name)?)
        } else {
            None
        };
        let pair_case = if self.pair_core_enabled {
            if names.len() != 2 {
                return Err("pair core watch supports exactly two driven slots".into());
            }
            Some(host_play::paired_core::PairCase::parse(scenario.name)?)
        } else {
            None
        };
        let mut inject = scenario::settings_inject_map(view.script_settings_inject);
        if pair_case.is_none() {
            if let Some(key) = view.inject_companion_as {
                if names.len() > 1 {
                    inject.get_or_insert_with(serde_json::Map::new).insert(
                        key.to_string(),
                        serde_json::Value::String(partner_screen_name(&names[1])),
                    );
                }
            }
        }
        self.set_script_settings_inject(inject);
        let path = temp_live_vault_from(&entries, 274_000_001, &pass, auto_login);
        if !self.unlock_at(&path, &pass) {
            return Err(self
                .error
                .clone()
                .unwrap_or_else(|| "unlock_at failed".into()));
        }
        if let Some(case) = core_case {
            let watch = self
                .catalog_core_watch()
                .ok_or_else(|| "catalog core watch handle unavailable".to_string())?;
            watch.configure(case, names[0].clone());
        }
        if let Some(case) = pair_case {
            let watch = self
                .paired_core_watch()
                .ok_or_else(|| "pair core watch handle unavailable".to_string())?;
            watch.configure(case, names[0].clone(), names[1].clone());
            if case == host_play::paired_core::PairCase::Duel {
                let weapon_id = self
                    .play
                    .as_ref()
                    .and_then(|play| play.game_data())
                    .and_then(|data| {
                        data.item_by_alias(host_play::paired_core::DUEL_WEAPON_ALIAS)
                            .map(|item| item.id)
                    })
                    .ok_or_else(|| "selected cache has no bronze_scimitar".to_string())?;
                watch.install_duel_weapon(weapon_id)?;
            }
        }
        if self.external_core_enabled {
            let frozen = host_play::external_loader::resolve_source(self.external_ts.as_deref())?;
            let owned = host_play::external_loader::materialize_owned_source(&frozen)?;
            let watch = host_play::external_loader::ExternalWatch::default();
            watch.configure(
                names[0].clone(),
                owned,
                host_play::external_loader::FROZEN_SHA256.into(),
            );
            self.install_external_core_watch(Some(watch));
        }
        self.mainland
            .store(scenario.seed.mainland, Ordering::Relaxed);
        self.scatter.store(false, Ordering::Relaxed);
        // Fleet scenario (2+ seed profiles): open the MultiBox wall like
        // `live_prepare_null_raster`/`live_prepare_stress`, so the rail
        // pops out and every bot is visible.
        if scenario.seed.profiles.len() > 1 {
            self.set_multibox(true);
        }
        self.wall.chooser_open = false;
        for name in &names {
            self.load(name);
        }
        self.select(&names[0]);
        // Apply the scenario's view knobs now the wall is up; the slot
        // threads re-read the focus within a frame.
        self.set_renderer(view.renderer);
        self.focus.lock().unwrap().only_render_selected = view.only_render_selected;
        self.set_capture(view.capture);
        self.set_live_full_rate(view.full_rate);
        // Scenario nav bag is session-only — never ui_state::save'd.
        self.nav_overlay = Some(from_scenario(&view.nav));
        // NEVER assign sidecar_50 — it stays the operator knob.
        self.sync_sidecar_cadence();
        let world = self.play.as_ref().and_then(|play| play.world());
        let mut runner = scenario::ScenarioRunner::with_world(scenario, world);
        if let Some(play) = &self.play {
            runner.set_map_members(play.map_members());
        }
        if let Some(budget) = scenario::budget_s_from_env() {
            runner.set_deadline(budget);
        }
        // The runner drives/companions the minted names, never the seed's
        // `test`/`test2` (the vault holds the fresh accounts).
        runner.set_live_names(&names);
        if let Some(play) = &self.play {
            runner.set_obj_names(play.obj_names());
        }
        *self.scenario.lock().unwrap() = Some(runner);
        self.live_script_stop_wait_started = None;
        // A scenario that names a script card (`start_script`) selects
        // the script; Start waits for [`scenario::StepKind::StartScript`]
        // after seed. With `inject_companion_as` on a fleet, the same JS
        // Starts on slot 1 too (reciprocal partner). Compiled registry
        // ids (currently `Sherlock`) start that port; catalog cards come
        // from `$RS2B0T`; in-tree file fixtures load from
        // `crates/script/tests/fixtures/`. Exact example files (`start_file`)
        // Load through normal File provenance and select by identity_id.

        if let Some(file_name) = view.start_file {
            let card = load_live_example_card(&mut self.js, file_name)?;
            let identity = card.identity_id();
            self.script_sel = Some(script::ScriptSel::Loaded(
                script::ScriptSource::File,
                identity.clone(),
            ));
            let bag = self.pending_settings_bag(
                script::ScriptSource::File,
                &identity,
                &card.settings_schema,
            );
            let siblings = self.sibling_modules_for_card(&card)?;
            stash_pending_starts(
                &self.pending_script,
                &names,
                view.inject_companion_as,
                card.js.clone(),
                card.shape,
                bag,
                siblings,
                scenario_fixture_loadouts(&view),
            );
        } else if let Some(card_name) = view.start_script {
            if let Some(id) = script::compiled_id(card_name) {
                self.script_sel = Some(script::ScriptSel::Compiled(id));
                stash_compiled_start(&self.pending_script, &names, id);
            } else if let Some(fixture) = script::live_file_fixture_path(card_name) {
                let stem = script::live_file_fixture_stem(card_name)
                    .ok_or_else(|| format!("no file stem for live fixture {card_name}"))?;
                self.js
                    .load(&fixture)
                    .map_err(|e| format!("load {card_name} fixture: {e}"))?;
                let card = self
                    .js
                    .get(script::ScriptSource::File, stem)
                    .cloned()
                    .ok_or_else(|| format!("file fixture {card_name} missing after load"))?;
                self.script_sel = Some(script::ScriptSel::Loaded(
                    script::ScriptSource::File,
                    stem.to_string(),
                ));
                let bag = self.pending_settings_bag(
                    script::ScriptSource::File,
                    stem,
                    &card.settings_schema,
                );
                let siblings = self.sibling_modules_for_card(&card)?;
                if let Some(case) = pair_case {
                    let watch = self
                        .paired_core_watch()
                        .ok_or_else(|| "pair core watch handle unavailable".to_string())?;
                    stash_pair_starts(
                        &self.pending_script,
                        &names,
                        case,
                        card.js.clone(),
                        card.shape,
                        &card.settings_schema,
                        siblings,
                        &watch,
                    )?;
                } else {
                    stash_pending_starts(
                        &self.pending_script,
                        &names,
                        view.inject_companion_as,
                        card.js.clone(),
                        card.shape,
                        bag,
                        siblings,
                        scenario_fixture_loadouts(&view),
                    );
                }
            } else {
                self.fill_rs2b0t_cards_once();
                self.js
                    .ensure_js(script::ScriptSource::Catalog, card_name)
                    .map_err(|e| format!("transpile {card_name}: {e}"))?;
                let card = self
                    .js
                    .get(script::ScriptSource::Catalog, card_name)
                    .cloned()
                    .ok_or_else(|| {
                        format!("$RS2B0T catalog has no {card_name} card (is $RS2B0T set?)")
                    })?;
                self.script_sel = Some(script::ScriptSel::Loaded(
                    script::ScriptSource::Catalog,
                    card_name.to_string(),
                ));
                let bag = self.pending_settings_bag(
                    script::ScriptSource::Catalog,
                    card_name,
                    &card.settings_schema,
                );
                let siblings = self.sibling_modules_for_card(&card)?;
                if let Some(case) = pair_case {
                    let watch = self
                        .paired_core_watch()
                        .ok_or_else(|| "pair core watch handle unavailable".to_string())?;
                    stash_pair_starts(
                        &self.pending_script,
                        &names,
                        case,
                        card.js.clone(),
                        card.shape,
                        &card.settings_schema,
                        siblings,
                        &watch,
                    )?;
                } else {
                    stash_pending_starts(
                        &self.pending_script,
                        &names,
                        view.inject_companion_as,
                        card.js.clone(),
                        card.shape,
                        bag,
                        siblings,
                        scenario_fixture_loadouts(&view),
                    );
                }
            }
        }
        self.login_all();
        Ok(())
    }

    /// Empty `Play` (shared cache + FIFO + per-frame hook) then spawn the
    /// first focused profile only. Parked names are started from [`select`].
    fn start_play(&mut self, vault: Vault) -> Result<(), String> {
        let focus = Arc::clone(&self.focus);
        let log_by = Arc::clone(&self.log_by);
        let mainland = Arc::clone(&self.mainland);
        let mainland_sent = Arc::clone(&self.mainland_sent);
        let scatter = Arc::clone(&self.scatter);
        let travellers = Arc::clone(&self.travellers);
        let script_nav_paint = Arc::clone(&self.script_nav_paint);
        let nav_states = Arc::clone(&self.nav_states);
        let frontend_gens = Arc::clone(&self.frontend_gens);
        let tick_latch = Arc::clone(&self.tick_latch);
        let walk_clear = Arc::clone(&self.walk_clear);
        let scenario = Arc::clone(&self.scenario);
        let pending_script = Arc::clone(&self.pending_script);
        let script_start_handle = Arc::clone(&self.script_start_handle);
        let catalog_core_watch = Arc::clone(&self.catalog_core_watch);
        let paired_core_watch = Arc::clone(&self.paired_core_watch);
        let audio = Arc::clone(&self.audio);
        let nav_publish = Arc::clone(&self.nav_publish);
        // Last failed device-open `(slot, when)`; a machine without an
        // audio device must not re-open cpal (or re-log) every frame.
        let audio_fail: Arc<Mutex<Option<(String, Instant)>>> = Arc::new(Mutex::new(None));
        let options = self.options.clone();
        let scatter_template = self.template.clone();
        let map_members = self
            .template
            .as_ref()
            .map(|t| t.profile().map_members())
            .unwrap_or(false);
        let per_frame = move |c: &mut client::client::Client, name: &str, hold: bool| {
            let session_boundary = publish_frontend_slot(
                name,
                c,
                &frontend_gens,
                &nav_states,
                &travellers,
                &tick_latch,
            );
            if session_boundary && focus.lock().unwrap().focused.as_deref() == Some(name) {
                walk_clear.store(true, Ordering::Relaxed);
            }
            // Flat model: every slot is a full Client; draw gates the
            // slot's renderer per the wall policy (focused always,
            // members when only-render-selected is off).
            let (focused, draw) = {
                let f = focus.lock().unwrap();
                (f.focused.clone(), draw_for_slot(&f, name))
            };
            c.set_draw(draw);
            // Nav-debug scene paint: only the focused drawing slot
            // publishes; a slot that stops drawing stores None so a
            // stale paint cannot linger.
            let layers = nav_publish.lock().unwrap().settings.clone();
            let drawing = focused.as_deref() == Some(name) && draw;
            let walk = match travellers.lock().unwrap().get(name).cloned() {
                Some(arm) => {
                    let arm = arm.lock().unwrap();
                    (arm.route.clone(), arm.traveller.current_aim())
                }
                None => (None, None),
            };
            let (driven, live) = match scenario.lock().unwrap().as_ref() {
                Some(runner) if runner.drives(name) => {
                    (true, (runner.armed_route().cloned(), runner.current_aim()))
                }
                _ => (false, (None, None)),
            };
            let script = script_nav_paint
                .lock()
                .unwrap()
                .as_ref()
                .map(|h| h.of(name))
                .unwrap_or((None, None));
            let (route, click) = live_or_walk_paint(driven, live, walk, script);
            match crate::picker::pack() {
                Some(world) => {
                    let here = c.local_player.as_ref().map(|lp| WorldTile {
                        x: c.map_build_base_x + lp.route_x[0],
                        z: c.map_build_base_z + lp.route_z[0],
                        level: 0,
                    });
                    // Run orb (varp 173 / 274 overlay), not the run
                    // animation — the anim is only true while a run
                    // cycle plays.
                    let run_on = c.run_enabled();
                    // Full tryMove BFS (every scene tile, src→dest),
                    // not the entity walk buffer (capped at 9) or the
                    // MOVE waypoint list (capped at 25).
                    let trail_world = live_client_trail(c, here);
                    publish_nav_debug(
                        c,
                        &world,
                        route.as_ref(),
                        here,
                        &trail_world,
                        run_on,
                        click,
                        &layers,
                        drawing,
                    );
                    if drawing && layers.camera_follow {
                        apply_path_camera(c, route.as_ref(), here);
                    }
                }
                None => c.set_nav_debug_paint(None),
            }
            // Focused-slot speaker: at most one cpal speaker, fed by
            // this slot's Client audio state (midi/waves/fade), gated
            // on focus + the Music/SFX toggle — `lowmem` (toggle off)
            // never opens cpal. The gate reconciles every frame; the
            // open closure runs on this slot's thread.
            let change = audio.frame(name, focused.as_deref(), || {
                let now = Instant::now();
                if let Some((who, at)) = audio_fail.lock().unwrap().as_ref() {
                    if who == name && now.duration_since(*at) < AUDIO_OPEN_RETRY {
                        return None;
                    }
                }
                match AudioOut::try_open(c.midi.clone(), c.waves.clone(), c.fade.clone()) {
                    Ok(out) => {
                        *audio_fail.lock().unwrap() = None;
                        push_log(
                            &mut log_by.lock().unwrap(),
                            name,
                            format!("audio: speaker open ({} Hz)", out.sample_rate),
                        );
                        Some(out)
                    }
                    Err(e) => {
                        *audio_fail.lock().unwrap() = Some((name.to_string(), now));
                        push_log(&mut log_by.lock().unwrap(), name, format!("audio: {e}"));
                        None
                    }
                }
            });
            if change == AudioChange::Closed {
                push_log(
                    &mut log_by.lock().unwrap(),
                    name,
                    "audio: speaker closed".into(),
                );
            }
            // Reconcile the client's actual `lowmem` mode to the
            // Music/SFX gate (toggle on = highmem): a lowmem spawn
            // skipped the sound load, so flipping the toggle
            // mid-session must re-run it live, not on the next
            // respawn. `set_lowmem` is idempotent — per-frame is cheap.
            c.set_lowmem(!audio.music_on(name));
            if c.ingame
                && c.scene_state == 2
                && seed_on_first_world(c.last_login_reconnect)
                && mainland_sent.lock().unwrap().insert(name.to_string())
            {
                if scatter.load(Ordering::Relaxed) {
                    let t = scatter_template.as_ref().map_or_else(
                        || host_play::scatter_tile_for(c.login_uid),
                        |template| template.scatter_tile_for(c.login_uid),
                    );
                    api::interact::seed_at(c, t.level, t.x, t.z);
                    push_log(
                        &mut log_by.lock().unwrap(),
                        name,
                        format!("{name}: scatter seed {} {} {}", t.level, t.x, t.z),
                    );
                } else if mainland.load(Ordering::Relaxed) {
                    api::interact::mainland_hop(c);
                    push_log(
                        &mut log_by.lock().unwrap(),
                        name,
                        format!("{name}: mainland hop queued"),
                    );
                }
            }

            // Shared `--live script_*` runner: tick the scenario's
            // driven slot and its companion slots, before the
            // local-player gate (seeding must observe frames with no
            // player decode yet). Hold freezes scenario follow the
            // same way `step_nav_bot` freezes (route stays latched).
            if let Some(runner) = scenario.lock().unwrap().as_mut() {
                if runner.drives(name) {
                    if fire_pending_catalog_start(
                        &pending_script,
                        &script_start_handle,
                        &catalog_core_watch,
                        &paired_core_watch,
                        runner,
                    ) {
                        runner.tick_with_hold(c, hold);
                    }
                } else if let Some(index) = runner.companion_for(name) {
                    runner.companion_tick(index, c);
                }
            }

            let (rx, rz) = match &c.local_player {
                Some(lp) => (lp.route_x[0], lp.route_z[0]),
                None => return,
            };
            let here = Tile {
                x: c.map_build_base_x + rx,
                z: c.map_build_base_z + rz,
                level: c.minusedlevel,
            };
            // Guardian hold freezes WalkArm follow; the armed route
            // stays latched and resumes when hold lifts.
            if !WalkArm::may_follow(hold) {
                return;
            }
            let Some(arm) = travellers.lock().unwrap().get(name).cloned() else {
                return;
            };
            {
                let mut latch = tick_latch.lock().unwrap();
                if latch.get(name) == Some(&(c.gens.player, here)) {
                    return;
                }
                latch.insert(name.to_string(), (c.gens.player, here));
            }
            let finished = {
                let states = nav_states.lock().unwrap();
                let Some(snapshot) = nav_snapshot_for_follow(&states, name) else {
                    return;
                };
                let mut arm = arm.lock().unwrap();
                let world = crate::picker::pack();
                // BankBudget session first. Walk follows the stand
                // sub-route; Open / Deposit / Withdraw / Wear / Close
                // freeze follow (never mid-session final_route).
                if arm.bank_fetch.is_some() {
                    host_play::step_walk_arm_bank_fetch(
                        c,
                        snapshot,
                        &mut arm,
                        world.as_deref(),
                        Some((here.x, here.z, here.level)),
                        map_members,
                    );
                    if host_play::walk_arm_bank_fetch_freezes_follow(&arm) {
                        return;
                    }
                }
                let Some(route) = arm.route.clone() else {
                    return;
                };
                let walking_stand = arm.bank_fetch.as_ref().is_some_and(|p| {
                    matches!(
                        p.steps.front(),
                        Some(nav::bank_fetch::BankStep::Walk { x, z, level })
                            if route.dest.x == *x
                                && route.dest.z == *z
                                && route.dest.level == *level
                    )
                });
                // The follow surface reads the canonical base + route-head
                // tile from a snapshot rebuilt off the same client; the
                // run is polled one step per player-info tick. The packed
                // teleport list rides along so a jewellery rub hop can
                // answer the destination dialog's choice for its landing.
                let mut options = TravelOptions {
                    // Exact arrival: the armed dest must be stood on
                    // before the route clears (the v1 traveller arrived
                    // the same way).
                    close_enough: 0,
                    teleports: world.as_ref().map(|w| w.graph.teleports.as_slice()),
                    edges: world.as_ref().map(|w| w.graph.edges.as_slice()),
                    ..TravelOptions::default()
                };
                let outcome = arm.traveller.follow(c, snapshot, route, &mut options);
                if walking_stand
                    && matches!(
                        &outcome,
                        Some(o) if !matches!(o, nav::traveller::TravelOutcome::Arrived { .. })
                    )
                {
                    // Stand Walk stalled / refused → NoPath.
                    arm.bank_fetch = None;
                    arm.route = None;
                    return;
                }
                if outcome.is_some() {
                    arm.route = None;
                    true
                } else {
                    false
                }
            };
            if finished {
                walk_clear.store(true, Ordering::Relaxed);
            }
        };
        let play = match self.validated_template.take() {
            Some(validated) => run_prepared_template(
                validated,
                options.mainland,
                Vec::new(),
                |_| (None, None),
                per_frame,
            )?,
            None => match self.template.clone() {
                Some(template) => run_with_template(
                    template,
                    options.mainland,
                    Vec::new(),
                    |_| (None, None),
                    per_frame,
                )?,
                None => run_with_io(&options, Vec::new(), |_| (None, None), per_frame),
            },
        };
        *self.script_start_handle.lock().unwrap() = Some(play.script_start_handle());
        self.install_catalog_core_watch(Some(play.catalog_core_watch()));
        self.install_paired_core_watch(Some(play.paired_core_watch()));
        *self.script_nav_paint.lock().unwrap() = Some(play.script_nav_paint());
        self.play = Some(play);
        crate::picker::set_pack(self.play.as_ref().and_then(|p| p.world()));
        self.statuses = self.play.as_ref().map(|p| p.statuses()).unwrap_or_default();
        self.vault = Some(vault);
        Ok(())
    }

    /// After unlock/`spawn_all`: restore `last_focus` when it is still a
    /// vault/slot name; otherwise focus the first so the combo and renderer
    /// are not stuck on `None`.
    fn focus_first_profile(&mut self) {
        let names = self.profile_names();
        let last = crate::ui_state::load().last_focus;
        if let Some(name) = crate::ui_state::pick_focus(&names, last.as_deref()) {
            self.select(&name);
        }
    }

    /// Advance clean rail removals without sleeping or joining on the UI
    /// thread. A disconnected slot stops immediately; a connected one gets
    /// the bounded clean-logout window before Stop is signalled.
    fn pump_slot_removals(&mut self) {
        self.pump_slot_removals_at(Instant::now());
    }

    fn pump_slot_removals_at(&mut self, now: Instant) {
        if let Some(play) = self.play.as_mut() {
            play.reap_finished_workers();
            play.reap_stopped_slots();
        }
        if self.pending_slot_removals.is_empty() {
            return;
        }
        let statuses = self.play.as_ref().map(Play::statuses).unwrap_or_default();
        let ready: Vec<(String, bool)> = self
            .pending_slot_removals
            .iter()
            .map(|(name, pending)| {
                let owns_current_lifetime = self
                    .play
                    .as_ref()
                    .and_then(|play| play.arm(name))
                    .is_some_and(|arm| Arc::ptr_eq(&arm, &pending.arm));
                let disconnected = !statuses
                    .iter()
                    .any(|status| status.username == name.as_str() && status.connected);
                let timed_out =
                    now.saturating_duration_since(pending.started) >= SLOT_REMOVE_TIMEOUT;
                (
                    name.clone(),
                    owns_current_lifetime && (disconnected || timed_out),
                )
            })
            .collect();
        for (name, stop) in ready {
            if stop {
                if let Some(play) = self.play.as_mut() {
                    play.begin_stop_slot(&name);
                }
            }
            if stop
                || !self
                    .play
                    .as_ref()
                    .and_then(|play| play.arm(&name))
                    .is_some_and(|arm| {
                        self.pending_slot_removals
                            .get(&name)
                            .is_some_and(|pending| Arc::ptr_eq(&arm, &pending.arm))
                    })
            {
                self.pending_slot_removals.remove(&name);
            }
        }
    }

    /// Cancel a pending rail removal in response to an operator action.
    /// The retained IO is reusable only when no replacement arm exists or
    /// when the current arm is the exact lifetime that owned the removal.
    fn cancel_slot_removal(&mut self, name: &str) {
        let Some(mut pending) = self.pending_slot_removals.remove(name) else {
            return;
        };
        let current = self.play.as_ref().and_then(|play| play.arm(name));
        let may_restore_io = current
            .as_ref()
            .is_none_or(|arm| Arc::ptr_eq(arm, &pending.arm));
        if may_restore_io {
            if let Some(io) = pending.io.take() {
                self.slots.entry(name.to_string()).or_insert(io);
            }
        }
    }

    /// Poll slot statuses and append log lines for transitions (slot up,
    /// login errors, ingame, scene changes). Call once per UI frame.
    pub fn pump_status(&mut self) {
        self.pump_slot_removals();
        // Per-frame mirrors that must not lag a focus/renderer/wall change:
        // the sidecar-50 cadence latch, and the speaker teardown when the
        // owning slot is no longer running.
        self.sync_sidecar_cadence();
        self.sync_nav_publish();
        if let Some(owner) = self.audio.owner() {
            if !self.slots.contains_key(&owner) {
                self.audio.release(&owner);
            }
        }
        let Some(current) = self.play.as_ref().map(|p| p.statuses()) else {
            return;
        };
        // Start/Stop return before the isolate is up or reaped: resolve
        // them here for every slot (an offline or queued slot has no
        // observe of its own) and commit the Starts that settled.
        if let Some(play) = &self.play {
            play.pump_script_lifecycles();
        }
        self.settle_script_starts();
        self.ingest_tutorial_chat(&current);
        self.maybe_getvar_tutorial(&current);
        if let Some(play) = &self.play {
            let mut log_by = self.log_by.lock().unwrap();
            for s in &current {
                for line in play.script_take_pending_logs(&s.username) {
                    if let Some(watch) = self.external_core_watch() {
                        if watch.configured() && watch.account() == s.username {
                            watch.note_logs(
                                Instant::now(),
                                &s.username,
                                std::slice::from_ref(&line),
                            );
                        }
                    }
                    push_log(&mut log_by, &s.username, format!("script: {line}"));
                }
            }
        }
        {
            let mut log_by = self.log_by.lock().unwrap();
            for s in &current {
                let name = s.username.as_str();
                let prev = self.statuses.iter().find(|p| p.username == s.username);
                match prev {
                    None => {
                        push_log(&mut log_by, name, format!("{name}: slot up"));
                        if let Some(e) = &s.error {
                            push_log(&mut log_by, name, format!("{name}: login {e}"));
                        }
                    }
                    Some(p) => {
                        if p.error.is_none() && s.error.is_some() {
                            push_log(
                                &mut log_by,
                                name,
                                format!("{name}: login {}", s.error.as_deref().unwrap_or_default()),
                            );
                        }
                        if !p.ingame && s.ingame {
                            push_log(&mut log_by, name, format!("{name}: ingame"));
                        }
                        if p.scene_state != s.scene_state {
                            push_log(
                                &mut log_by,
                                name,
                                format!("{name}: scene {}", s.scene_state),
                            );
                        }
                        if p.welcome_notice != s.welcome_notice {
                            if let Some(line) = s.welcome_notice.as_deref() {
                                push_log(&mut log_by, name, format!("{name}: {line}"));
                            }
                        }
                        if p.welcome_failure.is_none() && s.welcome_failure.is_some() {
                            push_log(
                                &mut log_by,
                                name,
                                format!(
                                    "{name}: {}",
                                    s.welcome_failure.as_deref().unwrap_or_default()
                                ),
                            );
                        }
                    }
                }
            }
        }
        self.statuses = current;
        self.sync_walk_status();
    }

    /// Copy each slot's walk-arm dest into `walk_*` (−1 if none) and
    /// clear [`Session::walk_dest`] after Arrived.
    fn sync_walk_status(&mut self) {
        for s in &mut self.statuses {
            let queued = self
                .travellers
                .lock()
                .unwrap()
                .get(&s.username)
                .and_then(|a| a.lock().unwrap().queued_tile());
            if queued.is_some() {
                apply_queued_walk(s, queued);
            }
        }
        if self.walk_clear.swap(false, Ordering::Relaxed) {
            let focused = self.focused_name();
            let keep = focused.as_ref().and_then(|n| {
                self.travellers
                    .lock()
                    .unwrap()
                    .get(n)
                    .and_then(|a| a.lock().unwrap().queued_tile())
            });
            if keep.is_none() {
                self.walk_dest = None;
                if let Some(name) = focused.as_deref() {
                    if let Some(s) = self.statuses.iter_mut().find(|s| s.username == name) {
                        apply_queued_walk(s, None);
                    }
                }
            }
        }
    }

    /// Snapshot of every slot's status (for the status section).
    pub fn statuses(&self) -> Vec<SlotStatus> {
        self.statuses.clone()
    }

    /// Vault usernames plus any running slot outside the vault.
    pub fn profile_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .vault
            .as_ref()
            .map(|v| v.profiles().map(|p| p.username.clone()).collect())
            .unwrap_or_default();
        if let Some(play) = &self.play {
            for s in play.statuses() {
                if !names.contains(&s.username) {
                    names.push(s.username);
                }
            }
        }
        names
    }

    pub fn focused_name(&self) -> Option<String> {
        self.focus.lock().unwrap().focused.clone()
    }

    fn map_members(&self) -> bool {
        self.play.as_ref().map(|p| p.map_members()).unwrap_or(false)
    }

    /// Queue a `CLIENT_CHEAT` on the focused slot. No-op without play/focus.
    pub fn cheat_focused(&self, cmd: &str) {
        let Some(play) = self.play.as_ref() else {
            return;
        };
        let Some(name) = self.focused_name() else {
            return;
        };
        play.cheat(&name, cmd);
    }

    /// True when this session's world host is a local engine.
    pub fn debug_ui(&self) -> bool {
        is_local_engine(&self.options.host)
    }

    /// Cached TutSkip for the focused profile: `None` unknown, `Some(true)`
    /// skipped, `Some(false)` still in tutorial.
    pub fn focused_tutorial_skipped(&self) -> Option<bool> {
        let name = self.focused_name()?;
        self.vault
            .as_ref()
            .and_then(|v| v.get(&name))
            .and_then(|p| p.settings.tutorial_skipped)
    }

    /// Persist TutSkip on the focused vault profile (debugprefs).
    pub fn mark_tutorial_skipped(&mut self) {
        let Some(name) = self.focused_name() else {
            return;
        };
        self.cache_tutorial(&name, true);
    }

    fn cache_tutorial(&mut self, name: &str, skipped: bool) {
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        let Some(mut profile) = vault.get(name).cloned() else {
            return;
        };
        if profile.settings.tutorial_skipped == Some(skipped) {
            return;
        }
        profile.settings.tutorial_skipped = Some(skipped);
        if let Err(e) = vault.upsert(profile) {
            self.error = Some(format!("tutorial: {e}"));
        }
    }

    fn ingest_tutorial_chat(&mut self, statuses: &[SlotStatus]) {
        for s in statuses {
            if let Some((var, value)) = parse_getvar_line(&s.chat_head) {
                if var == "tutorial" {
                    self.cache_tutorial(&s.username, value >= 1000);
                }
            }
        }
    }

    fn maybe_getvar_tutorial(&mut self, statuses: &[SlotStatus]) {
        if !self.debug_ui() {
            return;
        }
        let Some(name) = self.focused_name() else {
            return;
        };
        if self.focused_tutorial_skipped().is_some() {
            return;
        }
        let ready = statuses
            .iter()
            .any(|s| s.username == name && s.ingame && s.scene_state == 2);
        if !ready || !self.tutorial_getvar_sent.insert(name.clone()) {
            return;
        }
        if let Some(play) = self.play.as_ref() {
            play.cheat(&name, "getvar tutorial");
        }
    }

    /// Frames for the Game pane (the focused slot's mailbox). Every
    /// wall member owns its own `FrameBuf` in the flat model; fall back to
    /// the first spawned slot when nothing is focused.
    pub fn focused_pixels(&self) -> Option<Arc<FrameBuf>> {
        if let Some(slot) = self.focused_slot() {
            return Some(Arc::clone(&slot.pixels));
        }
        self.slots.values().next().map(|s| Arc::clone(&s.pixels))
    }

    /// Username of the focused slot (the sampled one, the old TV). Falls
    /// back to the first spawned slot when the focus has no slot yet.
    pub fn tv_name(&self) -> Option<String> {
        self.focused_name()
            .filter(|n| self.slots.contains_key(n))
            .or_else(|| self.slots.keys().next().cloned())
    }

    fn focused_slot(&self) -> Option<&SlotIo> {
        let name = self.focused_name()?;
        self.slots.get(&name)
    }

    /// Switch focus only onto an already-live slot. Capture sequencing uses
    /// this instead of [`Self::select`] so a missing actor cannot spawn.
    pub fn focus_existing(&mut self, name: &str) -> Result<(), String> {
        if !self.slots.contains_key(name) {
            return Err(format!("pair capture actor {name} is not a live slot"));
        }
        self.apply_focus(name);
        Ok(())
    }

    /// Switch the focused profile. A parked vault name is spawned on first
    /// select (login FIFO); already-running slots stay up so the picker can
    /// change focus. Capture follows the new focus when the single capture
    /// toggle is on (never two keyboards). The picker edit fields follow.
    /// New slots inherit the vault profile's auto-login (and logout latch).
    ///
    /// Flat model: clicking a member is pure focus — the Game pane samples
    /// that slot's `FrameBuf`. No socket is swapped (the channel-head baton
    /// is gone); every slot keeps running.
    pub fn select(&mut self, name: &str) {
        self.cancel_slot_removal(name);
        let arm = self.arm_for_profile(name);
        self.ensure_slot(name, arm, false);
        self.apply_focus(name);
        self.restore_script_heading(name);
    }

    fn apply_focus(&mut self, name: &str) {
        if self.persist_ui {
            // Reload so an injected/disk collapsed map is not clobbered.
            let mut ui = crate::ui_state::load();
            ui.last_focus = Some(name.to_string());
            crate::ui_state::save(&ui);
            self.ui = ui;
        }
        let mut focus = self.focus.lock().unwrap();
        if focus.focused.as_deref() == Some(name) {
            return;
        }
        let old = focus.focused.clone();
        focus.focused = Some(name.to_string());
        let capture = focus.capture;
        drop(focus);
        // Mirror onto the play: which slot the panel samples (host-play
        // keeps it as pure bookkeeping — no socket adopt/park).
        if let Some(play) = self.play.as_mut() {
            play.focus(name);
            // The draw state of both the outgoing and incoming slot can
            // change (draw_for_slot follows the focus); kick both so a
            // parked thread re-reads it within a frame, not at the next
            // game-tick park timeout.
            if let Some(old) = old.as_deref() {
                play.wake(old);
            }
        }
        // The overlay follows the focused traveller: switching focus may
        // show a different (or no) route, so force a rebuild.
        self.route_gen += 1;
        if capture {
            if let Some(old) = old.clone() {
                if let Some(slot) = self.slots.get(&old) {
                    slot.input.set_enabled(false);
                }
            }
            self.capture_on(name);
        } else {
            self.capture_tx = None;
        }
        // Credentials fields follow the newly focused profile; the General
        // config mirrors the profile's raster/mem so the pane shows what the
        // slot actually runs (display only — no write-back, no re-role).
        if let Some(vault) = &self.vault {
            if let Some(p) = vault.get(name) {
                self.cred_user = p.username.clone();
                self.cred_pass = p.password.clone();
                self.ui.raster = p.settings.raster;
                self.ui.lowmem = p.settings.lowmem;
            }
        }
    }

    /// Renderer checkbox. Writes both the focused checkbox (`Focus.renderer`)
    /// and `renderer_by[focused]` so per-slot draw policy stays in sync.
    /// Slot threads apply `set_draw` from the focus in their per-frame hook.
    pub fn set_renderer(&mut self, on: bool) {
        let mut focus = self.focus.lock().unwrap();
        focus.renderer = on;
        let name = focus.focused.clone();
        if let Some(name) = &name {
            focus.renderer_by.insert(name.clone(), on);
        }
        drop(focus);
        // The focused slot's draw state flips with the checkbox; kick it so
        // a parked thread applies `set_draw` within a frame.
        if let Some(name) = name {
            if let Some(play) = self.play.as_ref() {
                play.wake(&name);
            }
        }
    }

    /// Sidecar-50 pref: wall/grid members render at 50 fps instead of the
    /// 1 fps watch cadence (a render-cadence knob, not the idle park).
    /// `pump_status` mirrors it onto each slot's frame-loop latch within a
    /// frame; kick the parked members so the raise is not held up by the
    /// 1 s watch bound.
    pub fn set_sidecar_50(&mut self, on: bool) {
        self.focus.lock().unwrap().sidecar_50 = on;
        self.wake_all_slots();
    }

    /// Game-pane 50 fps for whoever is focused. Does not follow that
    /// client onto the rail.
    pub fn set_focused_50(&mut self, on: bool) {
        self.focus.lock().unwrap().focused_50 = on;
        self.wake_all_slots();
    }

    /// Ephemeral live overlay: every drawing slot at 50 fps, focused
    /// included (the scenario's `full_rate` knob). Not sidecar-50, not
    /// persisted; raising the frame cadence needs a kick so a parked
    /// member re-reads it within a frame.
    pub fn set_live_full_rate(&mut self, on: bool) {
        self.focus.lock().unwrap().live_full_rate = on;
        self.wake_all_slots();
    }

    /// Mirror the sidecar-50 pref onto every slot's frame-cadence latch
    /// (`SlotInput::set_full_rate`). Runs every UI frame so a focus,
    /// renderer, or wall-policy change lands within a frame.
    fn sync_sidecar_cadence(&mut self) {
        let focus = self.focus.lock().unwrap();
        for (name, slot) in &self.slots {
            slot.input.set_full_rate(full_rate_for(&focus, name));
        }
    }

    /// Mirror the effective nav-paint config onto the slot threads (they
    /// publish the focused drawing slot's paint every observe). Runs every
    /// UI frame so a modal edit or live-overlay flip lands within a frame.
    fn sync_nav_publish(&self) {
        *self.nav_publish.lock().unwrap() = NavPublishCfg {
            settings: self.effective_nav(),
        };
    }

    /// Live overlay when a scenario armed one, else the operator prefs.
    pub fn effective_nav(&self) -> NavSettings {
        let settings = self
            .nav_overlay
            .clone()
            .unwrap_or_else(|| self.ui.nav.clone());
        crate::nav_settings::apply_paint_override(&settings, self.nav_paints_override)
    }

    /// Set the headed live paint choice without changing saved preferences.
    pub fn set_nav_paints_override(&mut self, value: Option<bool>) {
        self.nav_paints_override = value;
        self.sync_nav_publish();
    }

    /// Game window `.build()` Some/None. Closing the pane drops the live
    /// drain but leaves the capture pref (`should_capture` is draw-gated).
    /// Reopening with capture on re-attaches the channel.
    pub fn set_game_pane_open(&mut self, open: bool) {
        let mut focus = self.focus.lock().unwrap();
        let was = focus.game_pane_open;
        focus.game_pane_open = open;
        let name = focus.focused.clone();
        let capture = focus.capture;
        drop(focus);
        if was && !open {
            if capture {
                self.capture_off();
            }
        } else if !was && open && capture {
            if let Some(name) = name.as_deref() {
                self.capture_on(name);
            }
        }
        // draw_for_slot gates on the pane; kick the focused slot so a
        // parked thread sees the change within a frame.
        if let Some(name) = name {
            if let Some(play) = self.play.as_ref() {
                play.wake(&name);
            }
        }
    }

    /// Capture checkbox. On: attach a fresh channel and enable the focused
    /// slot's drain. Off: disable the drain and drop the sender so the UI
    /// cannot enqueue (the slot thread does no `try_recv` while disabled).
    pub fn set_capture(&mut self, on: bool) {
        self.focus.lock().unwrap().capture = on;
        if self.persist_ui {
            self.ui.capture = on;
            crate::ui_state::save(&self.ui);
        }
        let name = self.focused_name();
        if on {
            match name.as_deref() {
                Some(name) => self.capture_on(name),
                None => self.capture_tx = None,
            }
        } else {
            self.capture_off();
        }
        // Capture flips the slot's idle classification (capture → frame
        // loop); kick it so the change lands within a frame.
        if let Some(name) = name {
            if let Some(play) = self.play.as_ref() {
                play.wake(&name);
            }
        }
    }

    fn capture_on(&mut self, name: &str) {
        if let Some(slot) = self.slots.get(name) {
            let (tx, rx) = mpsc::channel();
            slot.input.connect_rx(rx);
            slot.input.set_enabled(true);
            self.capture_tx = Some(tx);
        } else {
            self.capture_tx = None;
        }
    }

    fn capture_off(&mut self) {
        if let Some(slot) = self.focused_slot() {
            slot.input.set_enabled(false);
        }
        self.capture_tx = None;
    }

    /// Control arm for a vault profile: auto-login remains a saved policy,
    /// while a persisted wall logout latch starts the worker in an explicit
    /// logged-out hold.
    fn arm_for_profile(&self, name: &str) -> Option<Arc<SlotArm>> {
        let profile = self.vault.as_ref().and_then(|v| v.get(name))?;
        let auto_login = profile.settings.auto_login;
        let arm = SlotArm::new(profile.uid, auto_login);
        arm.random_events
            .store(profile.settings.random_events, Ordering::Relaxed);
        arm.lamp_auto
            .store(profile.settings.lamp_auto, Ordering::Relaxed);
        *arm.lamp_skill.lock().unwrap() = profile.settings.lamp_skill.clone();
        if self.wall.latch.contains(name) {
            arm.hold_logged_out();
        }
        Some(arm)
    }

    /// Return the profile values used for a client spawn without mutating the
    /// persisted vault profile.
    fn profile_with_memory_override(
        mut profile: Profile,
        memory_override: Option<bool>,
    ) -> Profile {
        if let Some(lowmem) = memory_override {
            profile.settings.lowmem = lowmem;
        }
        profile
    }

    /// Register per-slot IO and spawn via [`Play::try_spawn_slot`] when a play
    /// is live. Without `play` (unit tests / pre-unlock) only the IO map is
    /// filled so focus can attach. `arm` carries the spawn's login intent:
    /// `None` logs in immediately (CLI/e2e); panel paths pass
    /// [`Session::arm_for_profile`] so auto-login / latch are respected.
    /// Existing IO with no arm is a preserved terminal lifetime and restarts
    /// only when `restart_terminal` is true for explicit Log in.
    ///
    /// Flat model: every profile spawns **one** full `Client` slot with its
    /// own input + framebuffer (no lean channel, no render-all guard — a
    /// headless member just has its draw off).
    fn ensure_slot(&mut self, username: &str, arm: Option<Arc<SlotArm>>, restart_terminal: bool) {
        if self
            .play
            .as_ref()
            .is_some_and(|play| play.arm(username).is_some())
        {
            return;
        }
        if self.slots.contains_key(username) && (self.play.is_none() || !restart_terminal) {
            return;
        }
        let Some(profile) = self.vault.as_ref().and_then(|v| v.get(username)).cloned() else {
            return;
        };
        if reset_frontend_slot_lifetime(
            username,
            &self.frontend_gens,
            &self.nav_states,
            &self.travellers,
            &self.tick_latch,
        ) {
            self.walk_dest = None;
        }
        let existing_io = self
            .slots
            .get(username)
            .map(|slot| (Arc::clone(&slot.input), Arc::clone(&slot.pixels)));
        let (input, pixels) = existing_io.unwrap_or_else(|| (SlotInput::new(), FrameBuf::new()));
        // Raster comes from the vault profile (the same source as
        // `bot_client_config`); a focus change never re-roles a live slot.
        let raster = profile.settings.raster;
        let lowmem = self.memory_override.unwrap_or(profile.settings.lowmem);
        // Apply the session-only choice to the disposable profile used for
        // initial client construction. Never write this override to the vault.
        let profile = Self::profile_with_memory_override(profile, self.memory_override);
        input.set_prefer_cpu(raster == vault::RasterMode::Cpu);
        {
            let mut f = self.focus.lock().unwrap();
            f.renderer_by
                .insert(username.to_string(), raster != vault::RasterMode::Off);
        }
        self.audio.set_music(username, !lowmem);
        #[cfg(test)]
        if self.skip_slot_spawn {
            if let (Some(play), Some(arm)) = (self.play.as_mut(), arm.as_ref()) {
                play.attach_arm(username, Arc::clone(arm));
            }
            self.slots
                .insert(username.to_string(), SlotIo { input, pixels });
            return;
        }
        if let Some(play) = &mut self.play {
            if let Err(error) = play.try_spawn_slot(
                profile,
                Some(Arc::clone(&input)),
                Some(Arc::clone(&pixels)),
                arm,
            ) {
                self.error = Some(error);
                return;
            }
        }
        self.slots
            .insert(username.to_string(), SlotIo { input, pixels });
    }

    /// Credentials Log in: clear the logout latch, arm an explicit one-shot
    /// handshake, then select (spawn if needed).
    pub fn login(&mut self, name: &str) {
        self.cancel_slot_removal(name);
        self.wall.clear_latch(name);
        if let Some(play) = self.play.as_mut() {
            play.reap_finished_workers();
        }
        if let Some(arm) = self.play.as_ref().and_then(|play| play.arm(name)) {
            arm.arm_explicit_login();
        } else {
            let arm = self.arm_for_profile(name);
            if let Some(arm) = arm.as_ref() {
                arm.arm_explicit_login();
            }
            self.ensure_slot(name, arm, true);
        }
        self.select(name);
    }

    /// Log out one member (the credentials Logout button): latch it so
    /// auto-login is blocked until [`Session::login_all`], then arm a clean
    /// IF logout. The slot stays up and focused; only the login intent
    /// changes.
    pub fn logout(&mut self, name: &str) {
        self.wall.latch_logout(name);
        if let Some(arm) = self.play.as_ref().and_then(|p| p.arm(name)) {
            arm.request_logout();
        }
        // The logout press lives in the probe (per-tick); kick a parked
        // slot so the clean logout goes out within a frame.
        if let Some(play) = self.play.as_ref() {
            play.wake(name);
        }
    }

    /// Persist the focused profile's auto-login checkbox to the vault
    /// (`ProfileSettings.auto_login`) and mirror it onto a running slot's
    /// `arm.auto_login`. Never spawns or stops a slot.
    pub fn set_auto_login(&mut self, name: &str, on: bool) -> bool {
        let Some(vault) = self.vault.as_mut() else {
            self.error = Some("auto-login: vault locked".into());
            return false;
        };
        let Some(mut profile) = vault.get(name).cloned() else {
            self.error = Some(format!("auto-login: no profile {name}"));
            return false;
        };
        profile.settings.auto_login = on;
        match vault.upsert(profile) {
            Ok(()) => self.error = None,
            Err(e) => {
                self.error = Some(format!("auto-login: {e}"));
                return false;
            }
        }
        if let Some(play) = self.play.as_ref() {
            if let Some(arm) = play.arm(name) {
                arm.set_auto_login(on);
            }
        }
        if let Some(play) = self.play.as_ref() {
            play.wake(name);
        }
        true
    }

    /// Persist the focused profile's guardian settings (`random_events`,
    /// `lamp_skill`, `lamp_auto`) to the vault — the same upsert path as
    /// auto-login — and mirror `random_events`, `lamp_auto`, and
    /// `lamp_skill` onto a running slot's arm so toggle-off never
    /// acts/holds without a respawn. Never spawns or stops a slot.
    pub fn set_random_settings(
        &mut self,
        name: &str,
        random_events: bool,
        lamp_skill: &str,
        lamp_auto: bool,
    ) -> bool {
        let Some(vault) = self.vault.as_mut() else {
            self.error = Some("random: vault locked".into());
            return false;
        };
        let Some(mut profile) = vault.get(name).cloned() else {
            self.error = Some(format!("random: no profile {name}"));
            return false;
        };
        profile.settings.random_events = random_events;
        profile.settings.lamp_skill = lamp_skill.to_string();
        profile.settings.lamp_auto = lamp_auto;
        match vault.upsert(profile) {
            Ok(()) => self.error = None,
            Err(e) => {
                self.error = Some(format!("random: {e}"));
                return false;
            }
        }
        if let Some(arm) = self.play.as_ref().and_then(|p| p.arm(name)) {
            arm.random_events.store(random_events, Ordering::Relaxed);
            arm.lamp_auto.store(lamp_auto, Ordering::Relaxed);
            *arm.lamp_skill.lock().unwrap() = lamp_skill.to_string();
        }
        true
    }

    /// Game-pane lowmem (General config). Follows the focused slot's
    /// Music/SFX gate — the per-frame driver of the spawned `Client`'s
    /// `config.lowmem` — so the status row can never show "lowmem" while
    /// audio plays (a live boot's throwaway profile defaults lowmem, but
    /// the slot runs what the gate says). Falls back to the session's
    /// `ui.lowmem` when no slot is focused.
    pub fn focused_lowmem(&self) -> bool {
        let Some(name) = self.focused_name() else {
            return self.ui.lowmem;
        };
        !self.audio.music_on(&name)
    }

    /// Game-pane none/GPU/CPU (General config). Rail members stay GPU
    /// (CPU/none only as fallback via `set_draw` / `prefer_cpu`).
    pub fn focused_raster(&self) -> vault::RasterMode {
        self.ui.raster
    }

    fn persist_game_render_prefs(&mut self) {
        if self.persist_ui {
            crate::ui_state::save(&self.ui);
        }
        let Some(name) = self.focused_name() else {
            return;
        };
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        if let Some(mut p) = vault.get(&name).cloned() {
            p.settings.raster = self.ui.raster;
            p.settings.lowmem = self.ui.lowmem;
            if let Err(e) = vault.upsert(p) {
                self.error = Some(format!("render prefs: {e}"));
            }
        }
    }

    /// Apply Game-pane raster. Off is `set_draw` on the focused client
    /// only. Gpu↔Cpu flips the slot's `prefer_cpu` latch; the host drops
    /// the `Renderer` and reattaches the right backend on the **same**
    /// client — never a restart. Rail members are not touched.
    pub fn set_focused_raster(&mut self, raster: vault::RasterMode) -> bool {
        self.ui.raster = raster;
        self.persist_game_render_prefs();
        self.set_renderer(raster != vault::RasterMode::Off);
        let Some(name) = self.focused_name() else {
            return true;
        };
        if raster == vault::RasterMode::Off {
            return true;
        }
        let want_cpu = raster == vault::RasterMode::Cpu;
        if let Some(slot) = self.slots.get(&name) {
            slot.input.set_prefer_cpu(want_cpu);
        }
        true
    }

    pub fn set_focused_lowmem(&mut self, lowmem: bool) -> bool {
        self.ui.lowmem = lowmem;
        self.persist_game_render_prefs();
        self.error = None;
        let Some(name) = self.focused_name() else {
            return true;
        };
        // The audio gate is the slot threads' lowmem channel: each frame
        // the slot applies `c.set_lowmem(!audio.music_on(name))`, and the
        // host drops the `Renderer` when `config.lowmem` changes so the
        // next paint attaches with the new mode. No restart.
        self.audio.set_music(&name, !lowmem);
        true
    }

    /// Status-row copy for the focused profile's mem mode.
    pub fn mem_status_text(lowmem: bool) -> &'static str {
        if lowmem {
            "lowmem"
        } else {
            "highmem"
        }
    }

    /// GPU↔CPU (not Off) on a spawned slot is a drop+reattach — the
    /// `Client` and its socket stay up, so no logout/restart confirm is
    /// ever required (Off is `set_draw`; mem flips are live too).
    pub fn raster_switch_needs_confirm(
        _next: vault::RasterMode,
        _prefer_cpu: bool,
        _slot_spawned: bool,
    ) -> bool {
        false
    }

    /// Pick a raster. Off is `set_draw`. GPU↔CPU on the Game-pane client
    /// drops + reattaches the renderer (never a logout). Rail members stay
    /// GPU/lowmem.
    pub fn request_focused_raster(&mut self, raster: vault::RasterMode) {
        if self.focused_raster() == raster {
            return;
        }
        let _ = self.set_focused_raster(raster);
    }

    /// Pick highmem/lowmem for the Game pane. The live `Client` flips mem
    /// (the host drops + reattaches the renderer); never a restart. Rail
    /// members stay lowmem.
    pub fn request_focused_lowmem(&mut self, lowmem: bool) {
        if self.focused_lowmem() == lowmem {
            return;
        }
        let _ = self.set_focused_lowmem(lowmem);
    }

    /// Load a wall member: ensure its slot and select it. Auto-login
    /// follows the vault profile setting unless the member's logout latch
    /// blocks it (`SlotArm::new(should_auto_login)`); a latched member is
    /// spawned holding the title screen until [`Session::login_all`].
    /// Returns whether the name was newly added to the wall.
    pub fn load(&mut self, name: &str) -> bool {
        self.cancel_slot_removal(name);
        let newly = self.wall.load(name);
        let auto_login = self
            .vault
            .as_ref()
            .and_then(|v| v.get(name))
            .map(|p| p.settings.auto_login)
            .unwrap_or(false);
        let want_login = self.wall.should_auto_login(name, auto_login);
        if let Some(play) = self.play.as_ref() {
            // Already running (re-click): re-apply saved auto intent while a
            // latched logout remains parked.
            if let Some(arm) = play.arm(name) {
                arm.set_auto_login(auto_login);
                if want_login && arm.login_latched() {
                    // A clean logout requested solely for a cancelled rail
                    // removal must not hold an auto-login profile parked.
                    arm.arm_explicit_login();
                }
            } else {
                self.ensure_slot(name, self.arm_for_profile(name), false);
            }
        } else {
            self.ensure_slot(name, self.arm_for_profile(name), false);
        }
        // Load all / chooser rows spawn onto the rail and focus the member
        // (the flat model's "click" — the Game pane samples this slot).
        self.apply_focus(name);
        self.sync_wall_focus();
        newly
    }

    /// Load every profile (vault plus running slots) that is not already a
    /// wall member — the chooser's "Load all". Returns how many were newly
    /// added. Login intent still follows each profile's auto-login setting.
    pub fn load_all(&mut self) -> usize {
        let names = self.profile_names();
        let mut added = 0;
        for name in names {
            if self.load(&name) {
                added += 1;
            }
        }
        added
    }

    /// Chooser row ✕: delete the vault profile only. A live wall member is
    /// **not** logged out or dropped; the row just disappears from the
    /// chooser (credentials Save re-creates it). Returns whether a row was
    /// removed; failures set [`Session::error`].
    pub fn vault_remove(&mut self, name: &str) -> bool {
        let Some(vault) = self.vault.as_mut() else {
            self.error = Some("chooser: vault locked".into());
            return false;
        };
        match vault.remove(name) {
            Ok(removed) => {
                if removed {
                    self.error = None;
                }
                removed
            }
            Err(e) => {
                self.error = Some(format!("chooser: {e}"));
                false
            }
        }
    }

    /// Mirror `wall.members` into `Focus.wall` so `draw_for_slot` can paint
    /// unfocused tiles when only-render-selected is off. Call whenever
    /// membership changes: load, load_all, rail_remove, or the seed path.
    fn sync_wall_focus(&mut self) {
        let members = self.wall.members.clone();
        self.focus.lock().unwrap().wall = members;
    }

    /// Kick every slot thread after a wall-policy change (`only render
    /// selected` toggling flips every member's draw state; a parked thread
    /// must re-read it within a frame, not at the game-tick timeout).
    pub fn wake_all_slots(&self) {
        if let Some(play) = self.play.as_ref() {
            play.wake_all();
        }
    }

    /// Log in every wall member: clear their latches and arm a login so
    /// title-screen slots handshake. One-shot unless the profile's
    /// auto-login is set (which keeps the arm armed after the handshake).
    /// Queue membership follows worker arrival at Queueing; the focused slot
    /// is the sole priority exception.
    pub fn login_all(&mut self) {
        if let (Some(play), Some(head)) = (self.play.as_ref(), self.tv_name()) {
            play.prefer_login(&head);
        }
        for name in self.wall.members.clone() {
            self.wall.clear_latch(&name);
            if let Some(arm) = self.play.as_ref().and_then(|play| play.arm(&name)) {
                arm.arm_explicit_login();
            }
        }
        if let Some(play) = self.play.as_ref() {
            play.wake_all();
        }
    }

    /// Log out every wall member: record the latch (blocks auto-login
    /// until the next [`Session::login_all`]) and arm a clean IF logout.
    /// `want_login` is cleared too so a title-screen member does not
    /// handshake right back in.
    pub fn logout_all(&mut self) {
        let mut names = self.wall.members.clone();
        if let Some(play) = &self.play {
            for s in play.statuses() {
                if !names.iter().any(|n| n == &s.username) {
                    names.push(s.username);
                }
            }
        }
        for name in names {
            self.wall.latch_logout(&name);
            if let Some(arm) = self.play.as_ref().and_then(|p| p.arm(&name)) {
                arm.request_logout();
            }
        }
        if let Some(play) = self.play.as_ref() {
            play.wake_all();
        }
    }

    /// MultiBox toggle. On: seed the wall with every already-running slot
    /// (first on this process opens the chooser) and open the wall draw
    /// policy (`Focus.wall_open`), which stays true for rail **or** grid.
    /// Off: clear the grid and any open chooser and stop extra rasters
    /// (`wall_open = false`) without logging anyone out.
    pub fn set_multibox(&mut self, on: bool) {
        self.multibox = on;
        if on {
            let running: Vec<String> = self
                .play
                .as_ref()
                .map(|p| p.statuses().iter().map(|s| s.username.clone()).collect())
                .unwrap_or_default();
            self.wall.on_multibox_on(&running);
            // After seed: if focus is missing or not a wall member, restore
            // last_focus when it is on the wall, else the first member.
            let focused = self.focused_name();
            let need = match focused.as_deref() {
                None => true,
                Some(f) => !self.wall.members.iter().any(|m| m == f),
            };
            if need {
                // Live boots never restore the operator's disk last_focus;
                // the first wall member / later `select(&names[0])` wins.
                let last = if self.persist_ui {
                    crate::ui_state::load().last_focus
                } else {
                    None
                };
                if let Some(name) = crate::ui_state::pick_focus(&self.wall.members, last.as_deref())
                {
                    self.select(&name);
                }
            }
        } else {
            self.wall.on_multibox_off();
            self.cancel_edit_profile();
        }
        self.focus.lock().unwrap().wall_open = on;
        self.sync_wall_focus();
        // The wall policy change flips every member's draw state; kick all
        // so parked threads re-read it within a frame.
        if let Some(play) = self.play.as_ref() {
            play.wake_all();
        }
    }

    /// Grid submode of MultiBox: hides the rail in the Game pane. A no-op
    /// while MultiBox is off.
    pub fn set_grid(&mut self, on: bool) {
        if self.multibox {
            self.wall.grid = on;
        }
    }

    /// Remove a member from the rail and return immediately. Connected slots
    /// get a bounded clean-logout window advanced by [`Session::pump_status`];
    /// disconnected workers are stopped asynchronously. Neither path sleeps
    /// or joins on the UI call stack.
    pub fn rail_remove(&mut self, name: &str) {
        self.rail_remove_at(name, Instant::now());
    }

    fn rail_remove_at(&mut self, name: &str, now: Instant) {
        let focused = self.focused_name();
        let neighbour = self.wall.focus_neighbour(name, focused.as_deref());
        self.wall.rail_remove(name);
        self.wall.clear_latch(name);
        let connected = self.play.as_ref().is_some_and(|play| {
            play.statuses()
                .iter()
                .any(|status| status.username == name && status.connected)
        });
        let retained_io = self.slots.remove(name).or_else(|| {
            self.pending_slot_removals
                .remove(name)
                .and_then(|pending| pending.io)
        });
        let arm = self.play.as_ref().and_then(|play| play.arm(name));
        if connected {
            if let (Some(play), Some(arm)) = (self.play.as_ref(), arm) {
                // Clean logout only — Stop follows disconnect or timeout.
                arm.request_logout();
                play.wake(name);
                self.pending_slot_removals.insert(
                    name.to_string(),
                    PendingSlotRemoval {
                        started: now,
                        arm,
                        io: retained_io,
                    },
                );
            } else if let Some(play) = self.play.as_mut() {
                play.begin_stop_slot(name);
            }
        } else {
            self.pending_slot_removals.remove(name);
            if let Some(play) = self.play.as_mut() {
                play.begin_stop_slot(name);
            }
        }
        if reset_frontend_slot_lifetime(
            name,
            &self.frontend_gens,
            &self.nav_states,
            &self.travellers,
            &self.tick_latch,
        ) {
            self.walk_dest = None;
        }
        self.audio.release(name);
        self.sync_wall_focus();
        if focused.as_deref() == Some(name) {
            match neighbour {
                Some(n) => self.select(&n),
                None => {
                    self.focus.lock().unwrap().focused = None;
                    self.capture_tx = None;
                }
            }
        }
    }

    /// Arm a walk to `dest`. The picked dest is always stored so the status
    /// row shows what the user asked for even when no route could be found.
    /// Routing needs the player's observed tile and a loaded pack; the
    /// picker routes via [`Session::arm_walk_on`] when it has both.
    pub fn arm_walk(&mut self, dest: Tile) {
        self.walk_dest = Some(dest);
        self.walk_clear.store(false, Ordering::Relaxed);
    }

    /// The gating facts for the focused slot's WalkTo route: the slot's
    /// last published [`WorldState`] (inv/equipment/stats/varps/quests
    /// from its live snapshot), or the fail-closed empty state when the
    /// slot has not published yet (still logging in / no player decoded).
    fn focused_walk_state(&self) -> WorldState {
        self.focused_name()
            .and_then(|name| {
                self.nav_states
                    .lock()
                    .unwrap()
                    .get(&name)
                    .map(|(_, w)| w.clone().with_map_members(self.map_members()))
            })
            .unwrap_or_else(|| WorldState::empty().with_map_members(self.map_members()))
    }

    /// Open bank rows (obj id, count) from the focused slot's last
    /// published snapshot — empty when the bank is closed or no slot is
    /// focused (BankBudget has no closed-bank inventory).
    fn focused_walk_bank(&self) -> Vec<(i32, i32)> {
        self.focused_name()
            .and_then(|name| {
                self.nav_states
                    .lock()
                    .unwrap()
                    .get(&name)
                    .map(|(snap, _)| snap.bank().iter().map(|it| (it.def.id, it.count)).collect())
            })
            .unwrap_or_default()
    }

    /// Arm a walk to `dest` and route it on `world` from `from` (the
    /// player's observed tile). On a found route the focused username's
    /// walk arm stores the route so the observe tick can step it via
    /// [`nav::traveller::Traveller::follow`]; on `NoPath` only the dest is
    /// stored and `error` carries a short message. The Nav settings'
    /// [`FindOptions`] apply: `ui.nav.allow_teleports` unions the any-tile
    /// teleport layer in and `ui.nav.allow_wilderness` allows entering the
    /// wilderness. The gating [`WorldState`] is the focused slot's last
    /// published snapshot facts (see [`Session::nav_states`]); a slot that
    /// has not published yet falls back to the fail-closed empty state.
    /// The routing and arm latching live in
    /// [`host_play::arm_walk_on`] (shared with the TUI) — this wrapper
    /// only stores the picked dest, applies the panel nav settings, and
    /// reflects the outcome.
    /// Callers that do not know the player's tile fall back to
    /// [`Session::arm_walk`].
    pub fn arm_walk_on(&mut self, world: &NavWorld, from: Tile, dest: Tile) {
        self.walk_dest = Some(dest);
        self.walk_clear.store(false, Ordering::Relaxed);
        let name = self.focused_name();
        let state = self.focused_walk_state();
        let bank = self.focused_walk_bank();
        let routed = host_play::arm_walk_on(
            world,
            from,
            dest,
            FindOptions {
                allow_teleports: self.ui.nav.allow_teleports,
                allow_wilderness: self.ui.nav.allow_wilderness,
                allow_bank_fetch: self.ui.nav.allow_bank_fetch,
                ..FindOptions::default()
            },
            &state,
            &bank,
            &self.travellers,
            name.as_deref(),
        );
        match routed {
            Ok(_) => {
                self.error = None;
                if let Some(name) = name {
                    self.tick_latch.lock().unwrap().remove(&name);
                    // Rising edge: the overlay must paint the new route on
                    // this frame, not after the 1 s raster cadence.
                    self.route_gen += 1;
                }
            }
            Err(_) => {
                self.error = Some(format!("no path to {} {} {}", dest.x, dest.z, dest.level));
            }
        }
    }

    /// Arm the current [`Session::picker_sel`] on `world`. Returns false
    /// when nothing is selected. Clears the selection either way so a
    /// second confirm does not re-fire.
    pub fn confirm_picker_walk(&mut self, world: &NavWorld) -> bool {
        let Some(tile) = self.picker_sel.take() else {
            return false;
        };
        match self.focused_tile() {
            Some((fx, fz, fl)) => {
                let from = Tile {
                    x: fx,
                    z: fz,
                    level: fl,
                };
                self.arm_walk_on(world, from, tile);
            }
            None => self.arm_walk(tile),
        }
        true
    }

    /// The focused slot's valid observed tile `(x, z, level)`. A disconnected
    /// or not-ready slot and negative/out-of-plane producer sentinels have no
    /// routing origin.
    pub fn focused_tile(&self) -> Option<(i32, i32, i32)> {
        let name = self.focused_name()?;
        self.statuses()
            .iter()
            .find(|s| s.username == name)
            .and_then(SlotStatus::ready_tile)
    }

    /// The named slot's valid login-FIFO place `(position, total)` while
    /// it waits for a permit, else `None`. A grant, missing row, or malformed
    /// producer tuple clears that slot's overlay.
    pub fn queue_for(&self, name: &str) -> Option<(i32, i32)> {
        self.statuses
            .iter()
            .find(|s| s.username == name)
            .filter(|s| {
                s.queue_position >= 1 && s.queue_total >= 1 && s.queue_position <= s.queue_total
            })
            .map(|s| (s.queue_position, s.queue_total))
    }

    /// Whether the focused slot has an authenticated client session. Control
    /// actions such as Logout use this rather than the game-action gate.
    pub fn focused_connected(&self) -> bool {
        let Some(name) = self.focused_name() else {
            return false;
        };
        self.statuses()
            .iter()
            .any(|s| s.username == name && s.connected)
    }

    /// Whether the focused slot has a scene/player observation safe for game
    /// actions.
    pub fn focused_ingame(&self) -> bool {
        let Some(name) = self.focused_name() else {
            return false;
        };
        self.statuses()
            .iter()
            .any(|s| s.username == name && s.ingame)
    }

    /// Generated facts for the bound profile, when the cache identity matches.
    pub fn selected_game_data(&self) -> Option<std::sync::Arc<api::game_data::SelectedGameData>> {
        let profile = self.server_profile.as_ref()?;
        profile.game_data()
    }

    /// Observed equipment names/ids for the focused ingame character.
    /// `None` when no usable character is available.
    pub fn focused_equipment_items(&self) -> Option<Vec<(String, i32)>> {
        if !self.focused_ingame() {
            return None;
        }
        let name = self.focused_name()?;
        let states = self.nav_states.lock().ok()?;
        let (snap, _) = states.get(&name)?;
        Some(
            snap.equipment()
                .iter()
                .filter_map(|item| {
                    let name = item.def.name.as_deref()?.trim();
                    (!name.is_empty()).then(|| (name.to_string(), item.def.id))
                })
                .collect(),
        )
    }

    /// The focused slot's script lifecycle state; `Idle` when nothing is
    /// focused or the slot has no script. The script section's disable
    /// rules key off this.
    pub fn focused_script_state(&self) -> script::RunState {
        let Some(name) = self.focused_name() else {
            return script::RunState::Idle;
        };
        self.play
            .as_ref()
            .map(|p| p.script_state(&name))
            .unwrap_or(script::RunState::Idle)
    }

    /// Whether the focused script is Idle with the named self-stop reason.
    pub fn script_self_stop_observed(&self, needle: &str) -> bool {
        let Some(name) = self.focused_name() else {
            return false;
        };
        let Some(play) = self.play.as_ref() else {
            return false;
        };
        script_self_stop_observed(
            play.script_state(&name),
            play.script_lifecycle_receipt(&name).as_ref(),
            needle,
        )
    }

    /// The focused slot's script `last_error`; `None` when the slot has no
    /// script error (or nothing is focused).
    pub fn focused_script_last_error(&self) -> Option<String> {
        let name = self.focused_name()?;
        self.play.as_ref()?.script_last_error(&name)
    }

    /// Merged operator bag for the Browse-selected JS card (schema defaults,
    /// persisted overrides, optional scenario inject).
    pub fn merged_settings_bag(
        &self,
        source: script::ScriptSource,
        name: &str,
        schema: &[script::SettingDef],
    ) -> serde_json::Map<String, serde_json::Value> {
        self.script_settings
            .merged_bag(source, name, schema, self.script_settings_inject.as_ref())
    }

    /// Inject overrides merged last when the selected script Starts (live gold).
    pub fn set_script_settings_inject(
        &mut self,
        inject: Option<serde_json::Map<String, serde_json::Value>>,
    ) {
        self.script_settings_inject = inject;
    }

    /// Schema defaults + overrides + inject. Empty schema still keeps
    /// inject keys (Thiever `target: Guard`). `None` only when the merged
    /// bag is empty.
    fn pending_settings_bag(
        &self,
        source: script::ScriptSource,
        name: &str,
        schema: &[script::SettingDef],
    ) -> Option<serde_json::Map<String, serde_json::Value>> {
        let merged = self.merged_settings_bag(source, name, schema);
        if merged.is_empty() {
            None
        } else {
            Some(merged)
        }
    }

    /// Start the Browse-selected script (compiled or loaded JS) on the
    /// focused slot. The rs2b0t rule is enforced here too: while the slot's
    /// script is active the call is refused (the Start button is disabled,
    /// so this is the no-call backstop). Errors set [`Session::error`].
    pub fn script_start_selected(&mut self) {
        let Some(name) = self.focused_name() else {
            self.error = Some("script: no focused profile".into());
            return;
        };
        match self.script_start_profile_with(&name, true) {
            Ok(()) => self.error = None,
            Err(e) => self.error = Some(format!("script: {e}")),
        }
    }

    /// Pause the focused slot's script, or Resume when it is Paused (the
    /// button label follows [`script_pause_enabled`]).
    pub fn script_toggle_pause(&mut self) {
        let Some(name) = self.focused_name() else {
            return;
        };
        let Some(play) = self.play.as_ref() else {
            return;
        };
        if play.script_state(&name) == script::RunState::Paused {
            play.script_resume(&name);
        } else {
            play.script_pause(&name);
        }
    }

    /// Stop the focused slot's script (teardown hook, instance dropped).
    pub fn script_stop(&mut self) {
        let Some(name) = self.focused_name() else {
            return;
        };
        if let Some(play) = self.play.as_ref() {
            play.script_stop(&name);
        }
    }

    /// One-shot script-local paint button on the focused slot.
    pub fn script_paint_click(&mut self, id: &str, generation: u64) {
        let Some(name) = self.focused_name() else {
            return;
        };
        if let Some(play) = self.play.as_ref() {
            play.script_paint_click(&name, id, generation);
        }
    }

    /// Persistent strip/rail/tabs selection on the focused slot.
    pub fn script_paint_select(&mut self, key: &str, select_name: &str, generation: u64) {
        let Some(name) = self.focused_name() else {
            return;
        };
        if let Some(play) = self.play.as_ref() {
            play.script_paint_select(&name, key, select_name, generation);
        }
    }

    /// Overlay generation for the path overlay's rising-edge refresh.
    pub fn route_gen(&self) -> u64 {
        self.route_gen
    }

    /// The status-row walk cell: `"—"` when nothing is queued, else the
    /// queued dest as `"x z level"`.
    pub fn walk_status_text(&self) -> String {
        match self.walk_dest {
            Some(d) => format!("{} {} {}", d.x, d.z, d.level),
            None => "—".into(),
        }
    }
}

/// `(username, password)` pairs for live `null_raster` (`test`/`test2`).
fn null_raster_live_entries_for_target(target: client::BotTarget) -> Vec<(String, String)> {
    ["test", "test2"]
        .iter()
        .map(|user| {
            let user = user.to_string();
            (user.clone(), host_play::profile_password_for(&user, target))
        })
        .collect()
}

/// `(username, password)` pairs for live stress walls (`s00`…`s{n-1}`).
fn stress_live_entries_for_target(n: usize, target: client::BotTarget) -> Vec<(String, String)> {
    (0..n.max(1))
        .map(|i| {
            let name = format!("s{i:02}");
            (name.clone(), host_play::profile_password_for(&name, target))
        })
        .collect()
}

/// Throwaway encrypted vault for live prepare (e2e `temp_vault` pattern,
/// kept panel-private so panel does not depend on the e2e crate).
/// Null raster keeps base uid `274_000_001`. Accepts `&str` or `String`
/// entries (live scripts mint per-run usernames).
/// Same as a single-base [`temp_live_vault_from`] (`274_000_001`).
fn temp_live_vault_from<S: AsRef<str>>(
    entries: &[(S, S)],
    uid_base: i32,
    vault_pass: &str,
    auto_login: bool,
) -> PathBuf {
    // Unique per call: parallel tests boot several scenarios and must not
    // race on one temp vault path.
    static SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "274bot-panel-live-{}-{}-{}-{serial}",
        std::process::id(),
        entries.len(),
        uid_base
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault");
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    let mut vault = Vault::create(&path, vault_pass).unwrap();
    for (i, (user, pass)) in entries.iter().enumerate() {
        vault
            .upsert(Profile {
                username: user.as_ref().into(),
                password: pass.as_ref().into(),
                uid: uid_base + i as i32,
                settings: vault::ProfileSettings {
                    // Relog leaves run_client when !ingame; auto_login
                    // keeps want_login armed so the FIFO handshakes again
                    // instead of sitting on the title ("logging in…").
                    // Prepare-fixture turns this off so LoggedOut holds.
                    auto_login,
                    ..vault::ProfileSettings::default()
                },
            })
            .unwrap();
    }
    path
}

/// 50-head watches are a release RAM/GPU check. Debug cargo run looks
/// frozen and spikes RSS; do not FAIL unit tests that call the shared
/// helper at N=2/3.
fn warn_stress50_debug() {
    if cfg!(debug_assertions) {
        eprintln!(
            "panel-play: stress50 is a release RAM watch — run with cargo run --release -p panel --bin panel-play -- --live stress50"
        );
    }
}

/// Fresh uid for a profile with no existing vault entry: one past the max
/// (host-play assigns uids from the same 274M base range).
fn fresh_uid(vault: &Vault) -> i32 {
    vault.profiles().map(|p| p.uid).max().unwrap_or(274_000_000) + 1
}

/// Copy a traveller dest into `SlotStatus.walk_*`; −1 when idle.
fn apply_queued_walk(status: &mut SlotStatus, queued: Option<Tile>) {
    match queued {
        Some(t) => {
            status.walk_x = t.x;
            status.walk_z = t.z;
            status.walk_level = t.level;
        }
        None => {
            status.walk_x = -1;
            status.walk_z = -1;
            status.walk_level = -1;
        }
    }
}

/// Detach the picker's nav world only after the play's slot threads are
/// joined, so no live observe can read a cleared `pack()`.
impl Drop for Session {
    fn drop(&mut self) {
        self.play = None;
        crate::picker::set_pack(None);
    }
}

mod chooser;

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
