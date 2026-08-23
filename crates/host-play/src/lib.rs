//! `host-play`: run vaulted profiles through the host kernel. The binary
//! unlocks a vault and runs the named profiles; the `e2e` harness links
//! this library so it can poll per-slot state instead of scraping logs.

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use client::client::Client;
use client::client::ClientConfig;
use client::config::{Cache, IfType};
use client::io::JagFile;
pub use host::debug_enabled;
use host::login_queue::{LoginBackoff, LoginQueue, Permit, QueuePos};
use host::prepare_client;
pub use host::set_debug;
pub use host::Host;
use host::{PixelBuf, SlotInput};
use vault::{Profile, Vault, VaultError};

/// Connection settings shared by every spawned slot.
#[derive(Clone)]
pub struct PlayOptions {
    pub host: String,
    pub port: u16,
    pub cache_dir: String,
    pub lowmem: bool,
    /// After scene 2, queue rs2b0t `mainlandAccount` tele+setvar (no relog).
    pub mainland: bool,
}

/// Pollable per-slot view; the slot threads update it after each frame.
#[derive(Debug, Clone)]
pub struct SlotStatus {
    pub username: String,
    /// When the slot's first login handshake started (after its permit).
    pub login_started: Option<Instant>,
    pub ingame: bool,
    pub scene_state: i32,
    /// Last login error (code + message); cleared after a successful login.
    pub error: Option<String>,
    pub runenergy: i32,
    /// Accepted auto-run `set_run(true)` sends this slot has made.
    pub run_sends: u32,
    /// Local-player tile (filled from `local_player` in observe).
    pub tile_x: i32,
    pub tile_z: i32,
    /// Local-player name, empty until `PLAYER_INFO` lands.
    pub player: String,
    /// `Client.main_modal_id` (open modal interface, -1 when none).
    pub main_modal_id: i32,
    /// Queued walk target tile, -1 when idle (filled from the slot's
    /// traveller when one is wired; Task 8+).
    pub walk_x: i32,
    pub walk_z: i32,
    pub walk_level: i32,
    /// Place in the login FIFO while waiting for a permit: 1-based
    /// `position` of `total`; both -1 when not queued (same sentinel as
    /// the `walk_*` fields).
    pub queue_position: i32,
    pub queue_total: i32,
}

/// Absolute world tile from the scene origin plus the local-player route
/// head (`route_x[0]` / `route_z[0]`). Scene pixels (`lp.x` / `lp.z`) are
/// 128× these; WalkTo and the picker need world tiles.
pub fn player_world_tile(
    map_build_base_x: i32,
    map_build_base_z: i32,
    route_x: i32,
    route_z: i32,
) -> (i32, i32) {
    (map_build_base_x + route_x, map_build_base_z + route_z)
}

/// Defaults match the derived `Default` for every field except the queued
/// walk tile, which starts `-1` (none) instead of `0`.
impl Default for SlotStatus {
    fn default() -> Self {
        Self {
            username: String::new(),
            login_started: None,
            ingame: false,
            scene_state: 0,
            error: None,
            runenergy: 0,
            run_sends: 0,
            tile_x: 0,
            tile_z: 0,
            player: String::new(),
            main_modal_id: 0,
            walk_x: -1,
            walk_z: -1,
            walk_level: -1,
            queue_position: -1,
            queue_total: -1,
        }
    }
}

/// Per-slot control arm. The panel flips these to make a slot sit on the
/// title screen (no handshake) until login is armed, request a clean IF
/// logout, or stop the thread. A `None` arm at spawn means CLI/e2e: the
/// slot logs in immediately.
pub struct SlotArm {
    pub want_login: Arc<AtomicBool>,
    pub want_logout: Arc<AtomicBool>,
    pub stop: Arc<AtomicBool>,
    pub latch: Arc<AtomicBool>,
    /// The spawn-time auto-login intent (CLI `new(true)` stays armed so an
    /// unexpected DC re-handshakes; a panel one-shot arm disarms after the
    /// handshake unless the profile's auto_login was on).
    pub auto_login: Arc<AtomicBool>,
}

impl SlotArm {
    pub fn new(want_login: bool) -> Arc<Self> {
        Arc::new(Self {
            want_login: Arc::new(AtomicBool::new(want_login)),
            want_logout: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            latch: Arc::new(AtomicBool::new(false)),
            auto_login: Arc::new(AtomicBool::new(want_login)),
        })
    }
}

/// Whether the slot may start a login handshake: on the title (not ingame)
/// and the arm wants a login that is not latched by an intentional logout.
fn should_handshake(arm: &SlotArm, ingame: bool) -> bool {
    !ingame
        && arm.want_login.load(Ordering::Relaxed)
        && !arm.latch.load(Ordering::Relaxed)
}

/// After a successful handshake: stay armed only when this slot was spawned
/// with auto-login (an unexpected DC re-handshakes); a one-shot Log in /
/// Login all disarms until the next explicit arm.
fn on_login_success(arm: &SlotArm) {
    arm.want_login.store(
        arm.auto_login.load(Ordering::Relaxed) && !arm.latch.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
}

/// Per-frame arm handling in the 20 ms body: press the CC_LOGOUT iface when
/// the panel armed a logout on an ingame slot, then report whether the
/// thread must stop (rail ✕). The press is the only place a clean logout
/// can go out while the slot is inside [`Host::run_client`].
fn tick_flags(client: &mut Client, ifaces: &[Option<IfType>], arm: &SlotArm) -> bool {
    if arm.want_logout.load(Ordering::Relaxed) && client.ingame {
        api::interact::logout(client, ifaces);
        arm.want_logout.store(false, Ordering::Relaxed);
        arm.latch.store(true, Ordering::Relaxed);
        arm.want_login.store(false, Ordering::Relaxed);
    }
    arm.stop.load(Ordering::Relaxed)
}

/// Running slots and their shared status. Slots drive `mainloop` until the
/// process exits; callers poll [`Play::statuses`] and then exit.
///
/// [`Play::spawn_slot`] can add a profile after the initial [`run_with_io`]
/// call; later slots share the same login FIFO, cache, and per-frame hook.
pub struct Play {
    statuses: Arc<Mutex<Vec<SlotStatus>>>,
    handles: Vec<thread::JoinHandle<()>>,
    options: PlayOptions,
    cache: Arc<Cache>,
    ifaces: Vec<Option<IfType>>,
    queue: Arc<Mutex<LoginQueue>>,
    per_frame: Arc<dyn Fn(&mut Client, &str) + Send + Sync>,
    spawned: HashSet<String>,
    arms: HashMap<String, Arc<SlotArm>>,
}

impl Play {
    /// Snapshot of every slot's status.
    pub fn statuses(&self) -> Vec<SlotStatus> {
        self.statuses.lock().unwrap().clone()
    }

    /// Blocks until every slot thread exits (slot threads run forever, so
    /// this only returns if a slot panicked).
    pub fn join(self) {
        for handle in self.handles {
            let _ = handle.join();
        }
    }

    /// The control arm for a running slot, `None` when the name is not
    /// running. The panel flips the arm's flags to login/logout/stop.
    pub fn arm(&self, name: &str) -> Option<Arc<SlotArm>> {
        self.arms.get(name).cloned()
    }

    /// Spawn one more slot on this play's FIFO. No-op if `username` is
    /// already in the status list (already running). `None` arm behaves as
    /// [`SlotArm::new(true)`] — the slot logs in immediately (CLI/e2e);
    /// the panel passes a real arm so it can sit on the title.
    pub fn spawn_slot(
        &mut self,
        profile: Profile,
        input: Option<Arc<SlotInput>>,
        pixels: Option<Arc<PixelBuf>>,
        arm: Option<Arc<SlotArm>>,
    ) {
        if !self.spawned.insert(profile.username.clone()) {
            return;
        }
        let arm = arm.unwrap_or_else(|| SlotArm::new(true));
        self.arms.insert(profile.username.clone(), Arc::clone(&arm));
        spawn_slot_thread(
            &self.options,
            profile,
            input,
            pixels,
            arm,
            Arc::clone(&self.cache),
            self.ifaces.clone(),
            Arc::clone(&self.queue),
            Arc::clone(&self.statuses),
            Arc::clone(&self.per_frame),
            &mut self.handles,
        );
    }
}

/// Spawn one slot thread per profile. Each slot waits for a login-queue
/// permit, sends the handshake, then drives `mainloop` at the host cadence
/// while mirroring its state into the shared status list. Slots run with no
/// input and no pixel output; [`run_with_io`] adds per-slot channels.
pub fn run(options: &PlayOptions, profiles: Vec<Profile>) -> Play {
    run_with_io(options, profiles, |_| (None, None), |_, _| {})
}

/// Like [`run`], but each slot gets the `SlotInput`/`PixelBuf` returned by
/// `per_slot` (called synchronously, keyed by username), and `per_frame`
/// runs inside the observe hook on every 20 ms frame so callers can mirror
/// panel state (e.g. `client.set_draw`) into the slot thread. The FIFO
/// login queue and mainland hop are shared by every slot.
pub fn run_with_io<F, G>(
    options: &PlayOptions,
    profiles: Vec<Profile>,
    per_slot: F,
    per_frame: G,
) -> Play
where
    F: Fn(&str) -> (Option<Arc<SlotInput>>, Option<Arc<PixelBuf>>),
    G: Fn(&mut Client, &str) + Send + Sync + 'static,
{
    let (cache, ifaces) = load_template(&options.cache_dir);
    let mut play = Play {
        statuses: Arc::new(Mutex::new(Vec::new())),
        handles: Vec::new(),
        options: options.clone(),
        cache: Arc::new(cache),
        ifaces,
        queue: Arc::new(Mutex::new(LoginQueue::default())),
        per_frame: Arc::new(per_frame),
        spawned: HashSet::new(),
        arms: HashMap::new(),
    };
    for profile in profiles {
        let (slot_input, slot_pixels) = per_slot(&profile.username);
        play.spawn_slot(profile, slot_input, slot_pixels, None);
    }
    play
}

fn spawn_slot_thread(
    options: &PlayOptions,
    profile: Profile,
    slot_input: Option<Arc<SlotInput>>,
    slot_pixels: Option<Arc<PixelBuf>>,
    arm: Arc<SlotArm>,
    slot_cache: Arc<Cache>,
    ifaces_template: Vec<Option<IfType>>,
    slot_queue: Arc<Mutex<LoginQueue>>,
    slot_statuses: Arc<Mutex<Vec<SlotStatus>>>,
    slot_frame: Arc<dyn Fn(&mut Client, &str) + Send + Sync>,
    handles: &mut Vec<thread::JoinHandle<()>>,
) {
    let username = profile.username.clone();
    let uid = profile.uid;
    let password = profile.password.clone();
    let config = ClientConfig {
        host: options.host.clone(),
        port: options.port,
        cache_dir: options.cache_dir.clone(),
        members: true,
        lowmem: options.lowmem,
    };
    let mainland = options.mainland;

    handles.push(thread::spawn(move || {
        let mut client = prepare_client(config, uid, slot_cache, ifaces_template.clone());

        {
            let mut all = slot_statuses.lock().unwrap();
            all.push(SlotStatus {
                username: username.clone(),
                ..SlotStatus::default()
            });
        }
        if debug_enabled() {
            eprintln!("[host-play] slot {username}: thread up");
        }

        // Jag/anim/model/map prefetch (mirrors client-play; the scene
        // cannot reach scene_state 2 until the loc models are in).
        client.maininit();
        if client.error_loading {
            if debug_enabled() {
                eprintln!("[host-play] slot {username}: maininit failed");
            }
        }

        let mut backoff = LoginBackoff::new();
        loop {
            if arm.stop.load(Ordering::Relaxed) {
                slot_queue.lock().unwrap().leave(uid);
                return;
            }
            if !should_handshake(&arm, client.ingame) {
                // Sit on the title: no permit request, no handshake, until
                // the arm wants a login (auto-login / Log in / Login all).
                thread::sleep(Duration::from_millis(20));
                continue;
            }
            wait_for_permit(&slot_queue, &slot_statuses, &username, uid);
            let login_started = Instant::now();
            {
                let mut all = slot_statuses.lock().unwrap();
                if let Some(s) = all.iter_mut().find(|s| s.username == username) {
                    // First attempt only: retries must not move the
                    // handshake-start metric the harness asserts on.
                    if s.login_started.is_none() {
                        s.login_started = Some(login_started);
                    }
                    s.error = None;
                }
            }
            match client.login(&username, &password, false) {
                Ok(()) => {
                    backoff.reset();
                    // Auto-login slots stay armed so an unexpected DC
                    // re-handshakes; a one-shot Log in / Login all disarms
                    // until the next explicit arm.
                    on_login_success(&arm);
                    if debug_enabled() {
                        eprintln!("[host-play] slot {username}: ingame");
                    }
                    let mut mainland_sent = false;
                    Host::run_client(
                        &mut client,
                        &username,
                        slot_input.clone(),
                        slot_pixels.clone(),
                        |c, name, run_sends| {
                            slot_frame(c, name);
                            if !mainland_sent && mainland && c.ingame && c.scene_state == 2 {
                                api::interact::mainland_hop(c);
                                mainland_sent = true;
                                if debug_enabled() {
                                    eprintln!("[host-play] slot {name}: queued mainland tele+setvar (scene 2)");
                                }
                            }
                            let mut all = slot_statuses.lock().unwrap();
                            for s in all.iter_mut() {
                                if s.username == name {
                                    s.ingame = c.ingame;
                                    s.scene_state = c.scene_state;
                                    s.runenergy = c.runenergy;
                                    s.run_sends = run_sends;
                                    s.main_modal_id = c.main_modal_id;
                                    if let Some(lp) = &c.local_player {
                                        let (tx, tz) = player_world_tile(
                                            c.map_build_base_x,
                                            c.map_build_base_z,
                                            lp.route_x[0],
                                            lp.route_z[0],
                                        );
                                        s.tile_x = tx;
                                        s.tile_z = tz;
                                        s.player = lp.name.clone().unwrap_or_default();
                                    }
                                }
                            }
                            drop(all);
                            if debug_enabled() && c.ingame && c.scene_state == 1 && c.loop_cycle % 100 == 0 {
                                let od = c
                                    .on_demand
                                    .as_ref()
                                    .map(|od| {
                                        format!(
                                            "remaining={} fail={} msg={:?}",
                                            od.remaining(),
                                            od.fail_count,
                                            od.message
                                        )
                                    })
                                    .unwrap_or_else(|| "ondemand=none".into());
                                let ground = c
                                    .map_build_ground_data
                                    .iter()
                                    .filter(|d| d.is_some())
                                    .count();
                                let locs = c
                                    .map_build_location_data
                                    .iter()
                                    .filter(|d| d.is_some())
                                    .count();
                                eprintln!(
                                    "[host-play] slot {name}: scene loading {od} \
                                     ground={}/{} loc={}/{} await_pi={}",
                                    ground,
                                    c.map_build_ground_data.len(),
                                    locs,
                                    c.map_build_location_data.len(),
                                    c.awaiting_player_info
                                );
                            }
                        },
                        |c| {
                            // Leave the 20 ms body on `stop`, or once the
                            // client is back on the title (clean IF logout
                            // / DC) so this control loop decides the next
                            // handshake. `tick_flags` presses CC_LOGOUT
                            // while the arm asks for a logout.
                            tick_flags(c, &ifaces_template, &arm) || !c.ingame
                        },
                    );
                }
                Err(e) => {
                    let msg = format!("code {}: {}", e.code, e.mes2);
                    if debug_enabled() {
                        eprintln!("[host-play] slot {username}: login {msg}");
                    }
                    {
                        let mut all = slot_statuses.lock().unwrap();
                        if let Some(s) = all.iter_mut().find(|s| s.username == username) {
                            s.error = Some(msg);
                        }
                    }
                    // Response 16 (world full) escalates; response 5 is
                    // the engine's login-limit message ("Try again in
                    // 60 secs") and waits that long; other codes (wrong
                    // credentials, RSA mismatch, ...) retry slower.
                    let wait = match e.code {
                        16 => backoff.delay(),
                        5 => Duration::from_secs(60),
                        _ => Duration::from_secs(5),
                    };
                    thread::sleep(wait);
                }
            }
        }
    }));
}

/// Unlock `path`, or create it (and parent dirs) when missing. Any other
/// unlock error (`WrongPassphrase`, `Corrupt`, `EmptyPassphrase`) is
/// returned as-is so the CLI can print it instead of falling through to
/// `AlreadyExists`.
pub fn open_vault(path: &Path, passphrase: &str) -> Result<Vault, VaultError> {
    match Vault::unlock(path, passphrase) {
        Ok(v) => Ok(v),
        Err(VaultError::NotFound(_)) => {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            Vault::create(path, passphrase)
        }
        Err(e) => Err(e),
    }
}

/// Unpack the config/interface jags once and share the tables across slots
/// (the client's `load_cache` is private; this mirrors it with the same
/// public `Cache::unpack` / `IfType::unpack` entry points).
fn load_template(cache_dir: &str) -> (Cache, Vec<Option<IfType>>) {
    let cache = match std::fs::read(format!("{cache_dir}/config")) {
        Ok(bytes) => {
            std::panic::catch_unwind(AssertUnwindSafe(|| Cache::unpack(&JagFile::new(bytes))))
                .unwrap_or_default()
        }
        Err(_) => Cache::default(),
    };
    let ifaces = match std::fs::read(format!("{cache_dir}/interface")) {
        Ok(bytes) => {
            std::panic::catch_unwind(AssertUnwindSafe(|| IfType::unpack(&JagFile::new(bytes))))
                .unwrap_or_default()
        }
        Err(_) => Vec::new(),
    };
    (cache, ifaces)
}

/// Copy a login-queue snapshot onto every `SlotStatus` row named `name`;
/// `None` (granted or not queued) clears both fields back to -1.
fn apply_queue_wait(rows: &mut [SlotStatus], name: &str, pos: Option<QueuePos>) {
    let (position, total) = match pos {
        Some(p) => (p.position as i32, p.total as i32),
        None => (-1, -1),
    };
    for s in rows.iter_mut().filter(|s| s.username == name) {
        s.queue_position = position;
        s.queue_total = total;
    }
}

/// Block until the login queue grants `uid` a handshake permit, mirroring
/// the queue position onto the slot's status row while it waits.
fn wait_for_permit(
    queue: &Arc<Mutex<LoginQueue>>,
    statuses: &Arc<Mutex<Vec<SlotStatus>>>,
    username: &str,
    uid: i32,
) {
    loop {
        let wait = {
            let mut q = queue.lock().unwrap();
            match q.request_permit(uid, Instant::now()) {
                Permit::Grant => {
                    drop(q);
                    let mut all = statuses.lock().unwrap();
                    apply_queue_wait(&mut all, username, None);
                    return;
                }
                Permit::Wait(wait) => {
                    let pos = q.status(uid);
                    drop(q);
                    let mut all = statuses.lock().unwrap();
                    apply_queue_wait(&mut all, username, pos);
                    drop(all);
                    wait
                }
            }
        };
        thread::sleep(wait);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use client::client::ClientConfig;
    use client::config::Cache;

    fn tmp_vault(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("274bot-host-play-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("nested").join("vault")
    }

    #[test]
    fn open_vault_creates_missing_parent_dirs() {
        let path = tmp_vault("create");
        assert!(!path.exists());
        let v = open_vault(&path, "bot").unwrap();
        drop(v);
        assert!(path.exists());
    }

    #[test]
    fn open_vault_wrong_pass_is_not_already_exists() {
        let path = tmp_vault("wrong");
        open_vault(&path, "bot").unwrap();
        match open_vault(&path, "nope") {
            Err(VaultError::WrongPassphrase) => {}
            Err(e) => panic!("expected WrongPassphrase, got {e}"),
            Ok(_) => panic!("expected WrongPassphrase, unlocked"),
        }
    }

    #[test]
    fn run_with_io_empty_profiles_starts_no_slots() {
        let play = run_with_io(
            &PlayOptions {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: "/tmp".into(),
                lowmem: true,
                mainland: false,
            },
            vec![],
            |_| (None, None),
            |_, _| {},
        );
        assert!(play.statuses().is_empty());
    }

    #[test]
    fn prepare_client_from_shared_template() {
        let cache = Arc::new(Cache::default());
        let cfg = ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: true,
        };
        let a = prepare_client(cfg, 1, Arc::clone(&cache), vec![]);
        assert!(Arc::ptr_eq(&a.cache, &cache));
        assert!(!a.error_loading);
    }

    #[test]
    fn slot_status_walk_defaults_cleared() {
        let s = SlotStatus::default();
        assert_eq!((s.walk_x, s.walk_z, s.walk_level), (-1, -1, -1));
    }

    #[test]
    fn apply_queue_wait_writes_k_of_n_and_grant_clears() {
        let mut rows = vec![
            SlotStatus { username: "a".into(), queue_position: -1, queue_total: -1, ..SlotStatus::default() },
            SlotStatus { username: "b".into(), queue_position: -1, queue_total: -1, ..SlotStatus::default() },
        ];
        apply_queue_wait(&mut rows, "b", Some(host::login_queue::QueuePos { position: 2, total: 2 }));
        assert_eq!(rows[1].queue_position, 2);
        assert_eq!(rows[1].queue_total, 2);
        apply_queue_wait(&mut rows, "b", None);
        assert_eq!(rows[1].queue_position, -1);
        assert_eq!(rows[1].queue_total, -1);
    }

    #[test]
    fn spawn_without_auto_login_does_not_handshake() {
        let arm = SlotArm::new(false);
        assert!(!should_handshake(&arm, false));
        arm.want_login.store(true, Ordering::Relaxed);
        assert!(should_handshake(&arm, false));
        arm.latch.store(true, Ordering::Relaxed);
        assert!(!should_handshake(&arm, false));
        arm.latch.store(false, Ordering::Relaxed);
        assert!(should_handshake(&arm, false));
        assert!(!should_handshake(&arm, true));
    }

    #[test]
    fn login_success_keeps_auto_login_armed_but_disarms_one_shot() {
        // CLI: `new(true)` (auto_login true) stays armed so an unexpected
        // DC re-handshakes.
        let arm = SlotArm::new(true);
        on_login_success(&arm);
        assert!(should_handshake(&arm, false));

        // Panel Log in / Login all: armed explicitly, then disarmed after
        // the handshake — a DC sits on the title.
        let arm = SlotArm::new(false);
        arm.want_login.store(true, Ordering::Relaxed);
        on_login_success(&arm);
        assert!(!should_handshake(&arm, false));

        // The intentional-logout latch blocks even an auto-login slot.
        let arm = SlotArm::new(true);
        arm.latch.store(true, Ordering::Relaxed);
        on_login_success(&arm);
        assert!(!should_handshake(&arm, false));
    }

    #[test]
    fn tick_flags_presses_logout_when_ingame_and_reports_stop() {
        let cfg = ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: true,
        };
        let mut ifaces = vec![None; 10];
        let mut com = IfType::default();
        com.client_code = api::interact::CC_LOGOUT;
        ifaces[7] = Some(com);
        let mut client = prepare_client(cfg, 1, Arc::new(Cache::default()), ifaces.clone());
        client.ingame = true;
        let arm = SlotArm::new(false);
        arm.want_logout.store(true, Ordering::Relaxed);

        // want_logout + ingame presses CC_LOGOUT, latches, and does not stop.
        assert!(!tick_flags(&mut client, &ifaces, &arm));
        assert!(!arm.want_logout.load(Ordering::Relaxed));
        assert!(arm.latch.load(Ordering::Relaxed));
        assert!(!arm.want_login.load(Ordering::Relaxed));
        assert_eq!(
            client.out.data()[0],
            client::io::ClientProt::IF_BUTTON.id as u8
        );

        // A title slot never presses; `stop` still reports.
        client.ingame = false;
        arm.want_logout.store(true, Ordering::Relaxed);
        arm.stop.store(true, Ordering::Relaxed);
        assert!(tick_flags(&mut client, &ifaces, &arm));
        assert!(
            arm.want_logout.load(Ordering::Relaxed),
            "no CC_LOGOUT press on the title; the flag stays for the panel"
        );
    }

    #[test]
    fn player_world_tile_adds_build_base_to_route_head() {
        // Lumbridge courtyard: base 3200,3200 + route 22,20 → 3222,3220.
        assert_eq!(player_world_tile(3200, 3200, 22, 20), (3222, 3220));
        // Catherby range door from: base 2752,3392 + route 64,45 → 2816,3437.
        assert_eq!(player_world_tile(2752, 3392, 64, 45), (2816, 3437));
        assert_ne!(
            player_world_tile(3200, 3200, 22, 20),
            (22 * 128, 20 * 128),
            "must not report scene pixels"
        );
    }
}
