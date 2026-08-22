//! `host-play`: run vaulted profiles through the host kernel. The binary
//! unlocks a vault and runs the named profiles; the `e2e` harness links
//! this library so it can poll per-slot state instead of scraping logs.

use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use client::client::{Client, ClientConfig};
use client::config::{Cache, IfType};
use client::io::JagFile;
use host::login_queue::{LoginBackoff, LoginQueue, Permit};
pub use host::debug_enabled;
pub use host::Host;
use vault::Profile;

/// Connection settings shared by every spawned slot.
pub struct PlayOptions {
    pub host: String,
    pub port: u16,
    pub cache_dir: String,
    pub lowmem: bool,
}

/// Pollable per-slot view; the slot threads update it after each frame.
#[derive(Debug, Clone, Default)]
pub struct SlotStatus {
    pub username: String,
    /// When the slot's first login handshake started (after its permit).
    pub login_started: Option<Instant>,
    pub ingame: bool,
    pub scene_state: i32,
    /// Last login error (code + message); cleared after a successful login.
    pub error: Option<String>,
}

/// Running slots and their shared status. Slots drive `mainloop` until the
/// process exits; callers poll [`Play::statuses`] and then exit.
pub struct Play {
    statuses: Arc<Mutex<Vec<SlotStatus>>>,
    handles: Vec<thread::JoinHandle<()>>,
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
}

/// Spawn one slot thread per profile. Each slot waits for a login-queue
/// permit, sends the handshake, then drives `mainloop` at the host cadence
/// while mirroring its state into the shared status list.
pub fn run(options: &PlayOptions, profiles: Vec<Profile>) -> Play {
    let (cache, ifaces) = load_template(&options.cache_dir);
    let cache = Arc::new(cache);
    let queue = Arc::new(Mutex::new(LoginQueue::default()));
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let mut handles = Vec::new();

    for profile in profiles {
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
        let slot_cache = Arc::clone(&cache);
        let ifaces_template = ifaces.clone();
        let slot_queue = Arc::clone(&queue);
        let slot_statuses = Arc::clone(&statuses);

        handles.push(thread::spawn(move || {
            let mut client = Client::new(config);
            client.login_uid = uid;
            client.cache = slot_cache;
            client.ifaces = ifaces_template;

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
                wait_for_permit(&slot_queue, uid);
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
                        if debug_enabled() {
                            eprintln!("[host-play] slot {username}: ingame");
                        }
                        break;
                    }
                    Err(e) => {
                        let msg = format!("code {}: {}", e.code, e.mes2);
                        if debug_enabled() {
                            eprintln!("[host-play] slot {username}: login {msg}");
                        }
                        {
                            let mut all = slot_statuses.lock().unwrap();
                            if let Some(s) =
                                all.iter_mut().find(|s| s.username == username)
                            {
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

            Host::run_client(&mut client, &username, |c, name| {
                let mut all = slot_statuses.lock().unwrap();
                for s in all.iter_mut() {
                    if s.username == name {
                        s.ingame = c.ingame;
                        s.scene_state = c.scene_state;
                    }
                }
                drop(all);
                if debug_enabled()
                    && c.ingame
                    && c.scene_state == 1
                    && c.loop_cycle % 100 == 0
                {
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
            });
        }));
    }

    Play { statuses, handles }
}

/// Unpack the config/interface jags once and share the tables across slots
/// (the client's `load_cache` is private; this mirrors it with the same
/// public `Cache::unpack` / `IfType::unpack` entry points).
fn load_template(cache_dir: &str) -> (Cache, Vec<Option<IfType>>) {
    let cache = match std::fs::read(format!("{cache_dir}/config")) {
        Ok(bytes) => std::panic::catch_unwind(AssertUnwindSafe(|| {
            Cache::unpack(&JagFile::new(bytes))
        }))
        .unwrap_or_default(),
        Err(_) => Cache::default(),
    };
    let ifaces = match std::fs::read(format!("{cache_dir}/interface")) {
        Ok(bytes) => std::panic::catch_unwind(AssertUnwindSafe(|| {
            IfType::unpack(&JagFile::new(bytes))
        }))
        .unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    (cache, ifaces)
}

/// Block until the login queue grants `uid` a handshake permit.
fn wait_for_permit(queue: &Arc<Mutex<LoginQueue>>, uid: i32) {
    loop {
        let wait = {
            let mut q = queue.lock().unwrap();
            match q.request_permit(uid, Instant::now()) {
                Permit::Grant => return,
                Permit::Wait(wait) => wait,
            }
        };
        thread::sleep(wait);
    }
}
