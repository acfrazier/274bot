//! `tui-play`: the headless operator panel binary. Same flag spirit as
//! `host-play` / `panel-play` (`--vault`, `--vault-pass` / `BOT_VAULT_PASS`,
//! `--host`, `--port`, `--cache`, `--user`, `--live script_<name>`).
//!
//! The binary owns the `host_play::Play` session: a per-frame hook
//! publishes each slot's snapshot and steps the focused slot's walk arm,
//! and the UI loop polls statuses, refreshes [`TuiApp`], routes
//! keys/clicks, and dispatches the returned [`AppAction`] onto the play —
//! map Walk-confirm routes through `host_play::arm_walk_on`, chat
//! Continue/Answer and WASD walks go through `host_play::WireCmd`, and
//! the settings popup writes `ProfileSettings` on the focused profile.
//!
//! **Raster Off:** every profile is spawned with `RasterMode::Off` and the
//! TUI never attaches a `Renderer` (no `panel` / imgui / wgpu anywhere).
//! `--live script_<name>` mints per-run usernames into an ephemeral vault
//! (like the panel) and PASS/FAIL comes from the scenario runner, not a
//! screenshot.

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind, MouseEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use host_play::{
    arm_walk_on, mint_live_names, open_vault, run_with_io, Play, PlayOptions, SlotArm, WalkArm,
    WireCmd,
};
use nav::router::FindOptions;
use nav::tile::Tile;
use nav::traveller::TravelOptions;
use nav::WorldState;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use vault::{Profile, Vault};

use crate::app::{AppAction, ChatData, TuiApp};
use crate::chat::ChatAction;

/// The default engine port (same as host-play / panel).
const DEFAULT_PORT: u16 = 43594;

/// What `tui-play` should do this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunMode {
    /// Interactive operator panel.
    Interactive,
    /// `--live script_<name>`: PASS/FAIL from the scenario runner.
    Live(String),
}

/// Parsed `tui-play` flags.
#[derive(Debug, Clone)]
pub struct Args {
    pub vault: PathBuf,
    pub pass: Option<String>,
    pub host: String,
    pub port: u16,
    pub cache: String,
    pub users: Vec<String>,
    pub live: Option<String>,
}

fn usage() -> ! {
    eprintln!(
        "usage: tui-play [--vault PATH] [--vault-pass PASS] \
         [--host HOST] [--port PORT] [--cache DIR] \
         [--live script_<name>] [--user USER]... (default user: first vault profile)"
    );
    std::process::exit(2);
}

fn value(it: &mut std::iter::Skip<env::Args>) -> String {
    it.next().unwrap_or_else(|| usage())
}

fn default_vault() -> PathBuf {
    match env::var("HOME") {
        Ok(home) => PathBuf::from(format!("{home}/.274bot/vault")),
        Err(_) => PathBuf::from(".274bot/vault"),
    }
}

fn default_cache_dir() -> String {
    client::cache_dir().display().to_string()
}

/// `--live NAME` wins over `BOT_LIVE`; empty env is ignored.
/// `--help`/`-h` print the usage line (exit 2, the CLI family's
/// convention).
pub fn parse_args() -> Args {
    let mut args = Args {
        vault: default_vault(),
        pass: env::var("BOT_VAULT_PASS").ok(),
        host: host_play::default_world_host(),
        port: DEFAULT_PORT,
        cache: default_cache_dir(),
        users: Vec::new(),
        live: env::var("BOT_LIVE").ok().filter(|s| !s.is_empty()),
    };
    let mut it = env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--vault" => args.vault = PathBuf::from(value(&mut it)),
            "--vault-pass" => args.pass = Some(value(&mut it)),
            "--host" => args.host = value(&mut it),
            "--port" => args.port = value(&mut it).parse().unwrap_or_else(|_| usage()),
            "--cache" => args.cache = value(&mut it),
            "--user" => args.users.push(value(&mut it)),
            "--live" => args.live = Some(value(&mut it)),
            "--help" | "-h" => usage(),
            other => {
                eprintln!("tui-play: unknown {other}");
                usage();
            }
        }
    }
    args
}

/// The `--live` scenario, `Err` when the name is not a `script_<name>`.
/// Unknown names fail the run (same usage contract as panel-play).
pub fn live_scenario(name: &str) -> Result<scenario::Scenario, String> {
    let script = name
        .strip_prefix("script_")
        .ok_or_else(|| format!("tui-play: --live {name}: only script_<name> is supported"))?;
    scenario::get(script).ok_or_else(|| format!("tui-play: --live {name}: unknown scenario"))
}

/// Throwaway encrypted vault for `--live` (minted names, `bot` pass).
fn temp_live_vault(entries: &[(String, String)]) -> PathBuf {
    static SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = env::temp_dir().join(format!(
        "274bot-tui-live-{}-{}-{serial}",
        std::process::id(),
        entries.len()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("vault");
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    let mut vault = Vault::create(&path, "bot").unwrap();
    for (i, (user, pass)) in entries.iter().enumerate() {
        vault
            .upsert(Profile {
                username: user.clone(),
                password: pass.clone(),
                uid: 274_000_001 + i as i32,
                settings: vault::ProfileSettings {
                    auto_login: true,
                    ..vault::ProfileSettings::default()
                },
            })
            .unwrap();
    }
    path
}

/// The walk-arm step latch key: `(player gen, here)` per username, so a
/// hop is sent once per server tick, not every 20 ms frame.
type NavStepLatch = HashMap<String, (u64, (i32, i32, i32))>;

/// The TUI session: the running play, the vault, the per-slot snapshot
/// publication, and the per-username walk arms (the same `WalkArm` map
/// `host_play::arm_walk_on` latches and the panel drives).
pub struct TuiSession {
    play: Option<Play>,
    vault: Option<Vault>,
    pub error: Option<String>,
    /// All profile names (for the strip's slot list), in vault order.
    names: Vec<String>,
    options: PlayOptions,
    /// The username the settings popup currently edits; reload
    /// `ProfileSettings` into the app when it changes.
    last_focused: Option<String>,
    /// Per-username snapshots, rebuilt by the per-frame hook.
    snapshots: Arc<Mutex<HashMap<String, api::snapshot::GameSnapshot>>>,
    /// Per-username walk arms; the focused arm's route paints the map and
    /// the per-frame hook steps it via `Traveller::follow`.
    travellers: Arc<Mutex<HashMap<String, Arc<Mutex<WalkArm>>>>>,
    /// Last `(player gen, here)` ticked per username, so a walk hop is
    /// sent once per server tick, not every 20 ms frame.
    tick_latch: Arc<Mutex<NavStepLatch>>,
    /// Slot threads set this when a traveller returns Arrived/Budget so
    /// the UI can clear the picked walk dest.
    walk_clear: Arc<AtomicBool>,
    /// The shared nav world (`Play::world`), read by the per-frame walk
    /// step and the app thread's route arms.
    nav_world: Arc<Mutex<Option<Arc<nav::world::NavWorld>>>>,
    /// The shared `--live script_*` runner (slot threads tick it from the
    /// per-frame hook; the UI loop reads its status/evidence).
    scenario: Arc<Mutex<Option<scenario::ScenarioRunner>>>,
    /// The `--live` run's scenario name, for the PASS/FAIL label.
    live_name: Option<String>,
    /// The out-of-tree JS library: the Browse picker's cards and the
    /// Load/Start source for the focused slot (same store the panel
    /// persists to).
    js: script::JsLibrary,
    /// The `$RS2B0T` registry cards were filled into `js` once (first
    /// Browse/Load, like the panel).
    rs2b0t_filled: bool,
}

impl TuiSession {
    /// Empty session over the default engine options.
    fn new(options: PlayOptions) -> Self {
        let mut js = script::JsLibrary::new(script::default_js_store());
        let _ = js.restore(); // missing/broken store is not fatal here
        Self {
            play: None,
            vault: None,
            error: None,
            names: Vec::new(),
            options,
            last_focused: None,
            snapshots: Arc::new(Mutex::new(HashMap::new())),
            travellers: Arc::new(Mutex::new(HashMap::new())),
            tick_latch: Arc::new(Mutex::new(HashMap::new())),
            walk_clear: Arc::new(AtomicBool::new(false)),
            nav_world: Arc::new(Mutex::new(None)),
            scenario: Arc::new(Mutex::new(None)),
            live_name: None,
            js,
            rs2b0t_filled: false,
        }
    }

    /// Unlock (or first-run create) the vault at `path` and start the play.
    fn unlock_at(&mut self, path: &Path, pass: &str) -> Result<(), String> {
        let vault = open_vault(path, pass).map_err(|e| e.to_string())?;
        self.start_play(vault);
        Ok(())
    }

    /// Empty `Play` (shared cache + FIFO + per-frame hook), then spawn the
    /// focused profile only; `m` spawns the rest.
    fn start_play(&mut self, vault: Vault) {
        let snapshots = Arc::clone(&self.snapshots);
        let travellers = Arc::clone(&self.travellers);
        let tick_latch = Arc::clone(&self.tick_latch);
        let walk_clear = Arc::clone(&self.walk_clear);
        let nav_world = Arc::clone(&self.nav_world);
        let scenario = Arc::clone(&self.scenario);
        let options = self.options.clone();
        let play = run_with_io(
            &options,
            Vec::new(),
            |_| (None, None),
            move |c, name, hold| {
                // Publish the slot's snapshot (chat ring / modal, inv /
                // stats / locs) for the UI thread; the rebuild is
                // incremental, so a quiet frame publishes nothing new.
                let mut all = snapshots.lock().unwrap();
                let snap = all.entry(name.to_string()).or_default();
                snap.rebuild(c);

                // The shared `--live script_*` runner: tick the driven
                // slot and its companions before the local-player gate
                // (seeding must observe frames with no player decode).
                // Hold freezes scenario follow like `step_nav_bot`.
                if let Some(runner) = scenario.lock().unwrap().as_mut() {
                    if runner.drives(name) {
                        runner.tick_with_hold(c, hold);
                    } else if let Some(index) = runner.companion_for(name) {
                        runner.companion_tick(index, c);
                    }
                }

                let (rx, rz) = match &c.local_player {
                    Some(lp) => (lp.route_x[0], lp.route_z[0]),
                    None => return,
                };
                let here = (c.map_build_base_x + rx, c.map_build_base_z + rz, 0);
                // Guardian hold freezes WalkArm follow; the armed route
                // stays latched and resumes when hold lifts.
                if !WalkArm::may_follow(hold) {
                    return;
                }
                // Step the armed walk route one leg per player-info tick
                // (the panel's `tick_latch` pattern — a hop is sent once
                // per server tick, not re-sent every 20 ms frame).
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
                    let mut arm = arm.lock().unwrap();
                    let Some(route) = arm.route.clone() else {
                        return;
                    };
                    let world = nav_world.lock().unwrap().clone();
                    let mut options = TravelOptions {
                        // Exact arrival: the armed dest must be stood on
                        // before the route clears (the panel's contract).
                        close_enough: 0,
                        teleports: world.as_ref().map(|w| w.graph.teleports.as_slice()),
                        edges: world.as_ref().map(|w| w.graph.edges.as_slice()),
                        ..TravelOptions::default()
                    };
                    let outcome = arm.traveller.follow(c, snap, route, &mut options);
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
            },
        );
        self.nav_world.lock().unwrap().clone_from(&play.world());
        self.play = Some(play);
        self.vault = Some(vault);
    }

    /// Spawn `name`'s profile as a slot thread. `RasterMode::Off` always
    /// (the TUI never attaches a `Renderer`); the slot logs in
    /// immediately and re-handshakes after a DC only when the profile's
    /// `auto_login` is on.
    fn spawn(&mut self, name: &str) -> bool {
        let Some(mut profile) = self.vault.as_ref().and_then(|v| v.get(name)).cloned() else {
            return false;
        };
        profile.settings.raster = vault::RasterMode::Off;
        let auto_login = profile.settings.auto_login;
        let arm = SlotArm::new(profile.uid, true);
        arm.auto_login.store(auto_login, Ordering::Relaxed);
        arm.random_events
            .store(profile.settings.random_events, Ordering::Relaxed);
        if let Some(play) = self.play.as_mut() {
            play.spawn_slot(profile, None, None, Some(arm));
            true
        } else {
            false
        }
    }

    /// Spawn every vault profile that is not running yet (the `m` key).
    fn spawn_all(&mut self) -> usize {
        let names: Vec<String> = self
            .vault
            .as_ref()
            .map(|v| v.profiles().map(|p| p.username.clone()).collect())
            .unwrap_or_default();
        let mut spawned = 0;
        for name in names {
            if self.play.as_ref().is_some_and(|p| p.arm(&name).is_some()) {
                continue;
            }
            if self.spawn(&name) {
                spawned += 1;
            }
        }
        spawned
    }

    /// Focus `name` in the play (pure bookkeeping in the flat model).
    fn focus(&mut self, name: &str) {
        if let Some(play) = self.play.as_mut() {
            play.focus(name);
        }
    }

    /// `--live script_*` boot: minted ephemeral vault + spawn + runner.
    fn live_prepare_script(&mut self, scenario: scenario::Scenario) -> Result<(), String> {
        let name = scenario.name.to_string();
        let names = mint_live_names(scenario.seed.profiles.len());
        let entries: Vec<(String, String)> = names.iter().map(|u| (u.clone(), u.clone())).collect();
        let path = temp_live_vault(&entries);
        self.unlock_at(&path, "bot")?;
        self.live_name = Some(name);
        let mut runner = scenario::ScenarioRunner::new(scenario);
        runner.set_live_names(&names);
        if let Some(play) = &self.play {
            runner.set_obj_names(play.obj_names());
        }
        *self.scenario.lock().unwrap() = Some(runner);
        self.names = names.clone();
        for n in &names {
            self.spawn(n);
        }
        self.focus(&names[0]);
        Ok(())
    }

    /// The focused slot's last published [`WorldState`] (inv/equipment/
    /// stats/varps/quests), or the fail-closed empty state when the slot
    /// has not published yet.
    fn focused_walk_state(&self, name: &Option<String>) -> WorldState {
        name.as_deref()
            .and_then(|n| {
                self.snapshots
                    .lock()
                    .unwrap()
                    .get(n)
                    .map(WorldState::from_snapshot)
            })
            .unwrap_or_else(WorldState::empty)
    }

    /// Map Walk-confirm: store the picked dest, then route and arm the
    /// focused slot's walk arm when the player tile and nav world are
    /// known (`host_play::arm_walk_on`, the same shared arm the panel
    /// uses — the two views cannot drift).
    fn arm_walk_on(&mut self, app: &mut TuiApp, dest: Tile) {
        app.walk_dest = Some(dest);
        self.walk_clear.store(false, Ordering::Relaxed);
        let name = app.focused_name();
        let from = app.here.map(|h| Tile {
            x: h.x,
            z: h.z,
            level: h.level,
        });
        let world = self.nav_world.lock().unwrap().clone();
        let (Some(world), Some(from)) = (world, from) else {
            return; // no player tile / no pack: dest stored only
        };
        let state = self.focused_walk_state(&name);
        let routed = arm_walk_on(
            &world,
            from,
            dest,
            FindOptions::default(),
            &state,
            &self.travellers,
            name.as_deref(),
        );
        app.error = match routed {
            Ok(_) => None,
            Err(_) => Some(format!("no path to {} {} {}", dest.x, dest.z, dest.level)),
        };
    }

    /// WASD one-tile walk: a direct `try_move` through the slot's wire
    /// queue (adjacent world tile; the client pathfinds the step).
    fn wasd_walk(&self, app: &TuiApp, dest: Tile) {
        let Some(name) = app.focused_name() else {
            return;
        };
        if let Some(play) = &self.play {
            play.queue_wire(
                &name,
                WireCmd::Walk {
                    x: dest.x,
                    z: dest.z,
                    level: dest.level,
                },
            );
        }
    }

    /// Chat modal advance: queue `continue_dialog` / `answer_choice` on
    /// the focused slot (the guardian's hold still lets chat through).
    fn chat_send(&self, app: &TuiApp, action: ChatAction) {
        let Some(name) = app.focused_name() else {
            return;
        };
        if let Some(play) = &self.play {
            match action {
                ChatAction::Continue => play.queue_wire(&name, WireCmd::Continue),
                ChatAction::Answer(option) => {
                    play.queue_wire(&name, WireCmd::Answer(option as i32))
                }
                ChatAction::None => {}
            }
        }
    }

    /// Fill the JS library's cards from the `$RS2B0T` registry once
    /// (first Browse/Load, like the panel). Errors are debug-only.
    fn fill_rs2b0t_cards_once(&mut self) {
        if self.rs2b0t_filled {
            return;
        }
        self.rs2b0t_filled = true;
        if let Some(root) = script::rs2b0t_root() {
            if let Err(e) = self
                .js
                .register_rs2b0t(&root, &script::default_rs2b0t_path_file())
            {
                if std::env::var("BOT_DEBUG").is_ok() {
                    eprintln!("[tui-play] $RS2B0T registry: {e}");
                }
            }
        }
    }

    /// Start the Browse-selected card (a loaded JS bot) on the focused
    /// slot: `Play::script_start_load` with the card's source and shape
    /// (the same dispatch the panel uses). Errors land on the strip.
    fn script_start(&mut self, app: &mut TuiApp, card: &str) {
        let Some(name) = app.focused_name() else {
            app.error = Some("script: no focused profile".into());
            return;
        };
        let result = match &self.play {
            Some(play) => match self.js.get(card) {
                Some(card) => play.script_start_load(&name, card.source.clone(), card.shape),
                None => Err(format!("no loaded script: {card}")),
            },
            None => Err("no play".to_string()),
        };
        app.error = match result {
            Ok(()) => None,
            Err(e) => Some(format!("script: {e}")),
        };
    }

    /// Pause the focused slot's script (no-op without a focused slot).
    fn script_pause(&mut self, app: &mut TuiApp) {
        let Some(name) = app.focused_name() else {
            return;
        };
        if let Some(play) = &self.play {
            play.script_pause(&name);
        }
    }

    /// Stop the focused slot's script.
    fn script_stop(&mut self, app: &mut TuiApp) {
        let Some(name) = app.focused_name() else {
            return;
        };
        if let Some(play) = &self.play {
            play.script_stop(&name);
        }
    }

    /// Load a local JS bot file into the library, select it for Start,
    /// and persist the store. Errors land on the strip.
    fn script_load(&mut self, app: &mut TuiApp, path: &str) {
        match self.js.load(Path::new(path)) {
            Ok(card) => {
                app.script_sel = Some(card.name);
                app.error = None;
            }
            Err(e) => app.error = Some(format!("script: {e}")),
        }
    }

    /// Persist the settings popup's changes onto the focused vault
    /// profile (the operator vault; `--live`'s temp vault is ephemeral)
    /// and mirror `random_events` onto a running slot's arm.
    fn persist_settings(&mut self, app: &TuiApp) {
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        let Some(name) = app.focused_name() else {
            return;
        };
        let Some(mut profile) = vault.get(&name).cloned() else {
            return;
        };
        profile.settings = app.settings.clone();
        let random_events = profile.settings.random_events;
        let _ = vault.upsert(profile);
        if let Some(arm) = self.play.as_ref().and_then(|p| p.arm(&name)) {
            arm.random_events.store(random_events, Ordering::Relaxed);
        }
    }

    /// Copy the focused slot's views into the app and poll the runner.
    fn pump(&mut self, app: &mut TuiApp) {
        let statuses = self.play.as_ref().map(|p| p.statuses()).unwrap_or_default();
        // Running slots join the strip even when they are not in the
        // vault (live minted names).
        let mut names = self.names.clone();
        for s in &statuses {
            if !names.contains(&s.username) {
                names.push(s.username.clone());
            }
        }
        app.names = names;
        app.statuses = statuses;
        // The script pane's Browse picker lists the library cards
        // (persisted store + filled $RS2B0T registry cards).
        app.script_names = self.js.cards().iter().map(|c| c.name.clone()).collect();
        // The map paints `Play`'s packed collision, not the live loc
        // snapshot. Copy the session world each pump (an `Arc` clone)
        // so a loaded pack is not stuck behind the empty-state title.
        app.world = self.nav_world.lock().unwrap().clone();
        app.refresh();

        // The settings popup edits the focused profile: reload when the
        // focus changes (a fresh focus must not show the old slot's
        // random toggle).
        let focused = app.focused_name();
        if self.last_focused.as_deref() != focused.as_deref() {
            self.last_focused = focused.clone();
            app.settings = focused
                .as_deref()
                .and_then(|n| self.vault.as_ref().and_then(|v| v.get(n)))
                .map(|p| p.settings.clone())
                .unwrap_or_default();
            app.settings_state.open = false;
        }

        if let Some(name) = &focused {
            // The paint-as-chat toggle is operator state: rebuild the
            // pane data without losing it across pumps.
            let show_game_chat = app.chat_data.show_game_chat;
            // Borrow the focused slot's snapshot under the lock and copy
            // the small views the panes need (GameSnapshot is not Clone).
            let snap = self.snapshots.lock().unwrap();
            match snap.get(name) {
                Some(s) => {
                    app.chat_data = chat_data_from(s);
                    app.chat_data.show_game_chat = show_game_chat;
                    app.inv_items = s
                        .inventory()
                        .iter()
                        .map(|i| (i.def.name.clone().unwrap_or_else(|| "?".into()), i.count))
                        .collect();
                    app.stats_rows = s
                        .stats()
                        .iter()
                        .filter(|st| st.used)
                        .map(|st| (st.name.clone(), st.effective))
                        .collect();
                    let here = app.here;
                    app.locs_near = s
                        .locs()
                        .iter()
                        .filter_map(|l| {
                            let name = l.name.as_deref()?.to_string();
                            let d = here
                                .map(|h| (l.tile.x - h.x).abs().max((l.tile.z - h.z).abs()))
                                .unwrap_or(l.distance);
                            Some((d, name))
                        })
                        .collect();
                    app.locs_near.sort_by_key(|(d, _)| *d);
                    app.locs_near.truncate(3);
                }
                None => {
                    // A slot with no published snapshot must not show the
                    // previous focused slot's chat / inventory.
                    app.chat_data = ChatData::default();
                    app.chat_data.show_game_chat = show_game_chat;
                    app.inv_items.clear();
                    app.stats_rows.clear();
                    app.locs_near.clear();
                }
            }
            drop(snap);
            // The script's paint frame rides the status row (copied from
            // the isolate each observe); the chat pane shows it in place
            // of the game chat while it is non-empty.
            app.chat_data.script_paint =
                app.focused_status().and_then(|st| st.script_paint.clone());
            // Stop drops the isolate (and its paint with it); reset the
            // operator's game-chat toggle when no paint is showing so a
            // fresh Start shows the new paint by default instead of
            // hiding it until `p` again.
            if app.chat_data.script_paint.is_none() {
                app.chat_data.show_game_chat = false;
            }
            app.route = self
                .travellers
                .lock()
                .unwrap()
                .get(name)
                .and_then(|a| a.lock().unwrap().route.clone());
            if self.walk_clear.swap(false, Ordering::Relaxed) {
                app.walk_dest = None;
            }
            if let Some(play) = &self.play {
                app.script_state = play.script_state(name);
            }
        }
        if app.settings_dirty {
            self.persist_settings(app);
            app.settings_dirty = false;
        }
    }

    /// The `--live` terminal state: `Some(exit code)` when the runner
    /// passed (0) or failed (1); `None` while it runs.
    fn live_status(&self) -> Option<i32> {
        let name = self.live_name.as_deref().unwrap_or("script");
        let status = self.scenario.lock().unwrap().as_ref().map(|r| r.status());
        let evidence = self
            .scenario
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|r| r.evidence().cloned())
            .map(|ev| ev.to_json())
            .unwrap_or_default();
        match status {
            Some(scenario::RunnerStatus::Passed) => {
                println!("PASS: live {name} {evidence}");
                Some(0)
            }
            Some(scenario::RunnerStatus::Failed(msg)) => {
                eprintln!("FAIL: live {name} {evidence}");
                eprintln!("FAIL: {msg}");
                Some(1)
            }
            _ => None,
        }
    }
}

/// The chat pane's owned data, copied from the focused snapshot each pump.
fn chat_data_from(s: &api::snapshot::GameSnapshot) -> ChatData {
    ChatData {
        lines: s.chat_lines().to_vec(),
        modal_texts: s.chat_modal_texts().to_vec(),
        options: s.chat_options().to_vec(),
        has_continue: s.chat_continue_component_id() != -1,
        // The paint rides the status row, not the snapshot; the pump
        // patches it in, and the toggle is operator state.
        script_paint: None,
        show_game_chat: false,
    }
}

/// Run the interactive (or `--live`) TUI: unlock, spawn, event loop.
fn run(args: &Args, mode: RunMode) -> Result<i32, String> {
    let mut session = TuiSession::new(PlayOptions {
        host: args.host.clone(),
        port: args.port,
        cache_dir: args.cache.clone(),
        lowmem: true,
        mainland: false,
    });

    match mode {
        RunMode::Live(name) => {
            let scenario = live_scenario(&name)?;
            session.options.mainland = scenario.seed.mainland;
            session.live_prepare_script(scenario)?;
            let mut app = TuiApp::new(format!("tui-play --live {name}"));
            app.names = session.names.clone();
            app.focused = Some(0);
            run_loop(session, app)
        }
        RunMode::Interactive => {
            let Some(pass) = args.pass.clone() else {
                return Err("no vault passphrase (set BOT_VAULT_PASS or --vault-pass)".into());
            };
            let vault_exists = args.vault.is_file();
            if let Err(e) = session.unlock_at(&args.vault, &pass) {
                return Err(format!("vault {}: {e}", args.vault.display()));
            }
            if !vault_exists {
                // First run: create the default `test`/`test` profile so
                // unlock is not a dead end (host-play CLI convention).
                session.create_profile("test");
            }
            // `--user` names may not exist: create them like host-play
            // does (`password = username`, fresh uid).
            for u in &args.users {
                if session.vault.as_ref().is_none_or(|v| v.get(u).is_none()) {
                    session.create_profile(u);
                }
            }
            session.names = session
                .vault
                .as_ref()
                .map(|v| v.profiles().map(|p| p.username.clone()).collect())
                .unwrap_or_default();
            let focus = args
                .users
                .first()
                .cloned()
                .or_else(|| session.names.first().cloned());
            let Some(focus) = focus else {
                return Err("vault has no profiles (create one with host-play --user)".into());
            };
            session.spawn(&focus);
            session.focus(&focus);
            let mut app = TuiApp::new("274bot headless");
            app.names = session.names.clone();
            app.focused = session.names.iter().position(|n| n == &focus);
            run_loop(session, app)
        }
    }
}

impl TuiSession {
    /// Create a missing profile (host-play CLI convention: password =
    /// username, uid one past the vault's max, from the 274M base).
    fn create_profile(&mut self, username: &str) {
        let uid = self
            .vault
            .as_ref()
            .map(|v| v.profiles().map(|p| p.uid).max().unwrap_or(274_000_000) + 1)
            .unwrap_or(274_000_001);
        let profile = Profile {
            username: username.into(),
            password: username.into(),
            uid,
            settings: vault::ProfileSettings::default(),
        };
        if let Some(vault) = self.vault.as_mut() {
            let _ = vault.upsert(profile);
        }
    }
}

/// The crossterm event loop. `--live` runs headed when a controlling
/// terminal is available (the operator watches the panes) and degrades to
/// a headless pump loop otherwise, so the PASS/FAIL still lands in CI.
fn run_loop(mut session: TuiSession, mut app: TuiApp) -> Result<i32, String> {
    if enable_raw_mode().is_err() {
        // No controlling terminal: pump the runner without drawing.
        loop {
            session.pump(&mut app);
            if let Some(code) = session.live_status() {
                return Ok(code);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )
    .map_err(|e| format!("terminal setup: {e}"))?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;

    let result = (|| loop {
        session.pump(&mut app);
        if let Some(code) = session.live_status() {
            return Ok(code);
        }
        terminal
            .draw(|frame| app.draw(frame))
            .map_err(|e| e.to_string())?;
        if event::poll(Duration::from_millis(50)).map_err(|e| e.to_string())? {
            match event::read().map_err(|e| e.to_string())? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    let action = app.on_key(k);
                    dispatch(&mut session, &mut app, action);
                }
                Event::Mouse(m) => {
                    if let MouseEventKind::Down(_) = m.kind {
                        let action = app.on_click(m.column, m.row);
                        dispatch(&mut session, &mut app, action);
                    }
                }
                _ => {}
            }
        }
        if app.quit {
            return Ok(0);
        }
    })();

    disable_raw_mode().ok();
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::event::DisableMouseCapture,
        LeaveAlternateScreen
    )
    .ok();
    result
}

/// Route one [`AppAction`] onto the session: map walks arm through
/// `host_play::arm_walk_on`, chat and WASD go through [`WireCmd`], the
/// script actions dispatch `Play::script_start_load` / pause / stop /
/// the JS library, and the settings popup persists on the next pump.
fn dispatch(session: &mut TuiSession, app: &mut TuiApp, action: AppAction) {
    match action {
        AppAction::Quit => app.quit = true,
        AppAction::Focus(name) => session.focus(&name),
        AppAction::ArmWalk(tile) => session.arm_walk_on(app, tile),
        AppAction::WalkTile(tile) => session.wasd_walk(app, tile),
        AppAction::Chat(action) => session.chat_send(app, action),
        AppAction::SpawnAll => multibox_key(session, app),
        AppAction::ScriptStart(card) => session.script_start(app, &card),
        AppAction::ScriptPause => session.script_pause(app),
        AppAction::ScriptStop => session.script_stop(app),
        AppAction::ScriptBrowse => session.fill_rs2b0t_cards_once(),
        AppAction::ScriptLoad(path) => session.script_load(app, &path.to_string_lossy()),
        AppAction::None => {}
    }
}

/// The `m` key spawns the rest of the MultiBox wall.
fn multibox_key(session: &mut TuiSession, app: &mut TuiApp) {
    let spawned = session.spawn_all();
    if spawned > 0 {
        app.error = Some(format!("spawned {spawned} slot(s)"));
    }
}

pub fn main() -> ExitCode {
    let args = parse_args();
    let mode = match args.live.clone() {
        Some(name) => RunMode::Live(name),
        None => RunMode::Interactive,
    };
    match run(&args, mode) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("tui-play: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nav::grid::StepGrid;
    use nav::world::NavWorld;

    fn dummy_options() -> PlayOptions {
        PlayOptions {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        }
    }

    /// The map pane reads `TuiApp::world`. The session holds the pack on
    /// `nav_world` after `Play` loads it; pump must copy that Arc so a
    /// running script's loc list is not the only live world view.
    #[test]
    fn pump_copies_nav_world_onto_the_app() {
        let mut session = TuiSession::new(dummy_options());
        *session.nav_world.lock().unwrap() =
            Some(Arc::new(NavWorld::from_grid(&StepGrid::fixture_open_3x3())));
        let mut app = TuiApp::new("274bot headless");
        assert!(app.world.is_none(), "fresh app has no pack");
        session.pump(&mut app);
        assert!(
            app.world.is_some(),
            "pump copies the session nav world onto the map"
        );
    }

    #[test]
    fn pump_leaves_app_world_none_when_no_pack_loaded() {
        let mut session = TuiSession::new(dummy_options());
        let mut app = TuiApp::new("274bot headless");
        session.pump(&mut app);
        assert!(
            app.world.is_none(),
            "no session pack stays the empty-state title"
        );
    }

    /// Task 13 fix: the paint-as-chat toggle must not stick across a
    /// Stop → new Start. A slot whose script has no paint (stopped, or
    /// not painted yet) resets the toggle, so the fresh paint is visible
    /// by default instead of hidden behind the game-chat toggle.
    #[test]
    fn pump_resets_the_paint_toggle_when_the_paint_is_gone() {
        let mut session = TuiSession::new(dummy_options());
        session.names = vec!["test".into()];
        let mut app = TuiApp::new("274bot headless");
        app.focused = Some(0);
        // The operator toggled to game chat while the old script painted.
        app.chat_data.show_game_chat = true;
        session.pump(&mut app);
        assert!(
            !app.chat_data.show_game_chat,
            "a stopped/not-yet-painted slot must fall back to showing paint by default"
        );
    }
}
