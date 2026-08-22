//! `host-play`: run vaulted profiles through the host kernel. The binary
//! unlocks a vault and runs the named profiles; the `e2e` harness links
//! this library so it can poll per-slot state instead of scraping logs.

use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use client::client::Client;
use client::client::ClientConfig;
use client::config::{Cache, IfType};
use client::io::JagFile;
pub use host::debug_enabled;
use host::login_queue::{LoginBackoff, LoginQueue, Permit};
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
#[derive(Debug, Clone, Default)]
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
    let cache = Arc::new(cache);
    let queue = Arc::new(Mutex::new(LoginQueue::default()));
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let per_frame = Arc::new(per_frame);
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
        let mainland = options.mainland;
        let (slot_input, slot_pixels) = per_slot(&username);
        let slot_frame = Arc::clone(&per_frame);

        handles.push(thread::spawn(move || {
            let mut client = prepare_client(config, uid, slot_cache, ifaces_template);

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

            let mut mainland_sent = false;
            Host::run_client(&mut client, &username, slot_input, slot_pixels, |c, name, run_sends| {
                slot_frame(c, name);
                if !mainland_sent && mainland && c.ingame && c.scene_state == 2 {
                    api::interact::mainland_hop(c);
                    mainland_sent = true;
                    if debug_enabled() {
                        eprintln!(
                            "[host-play] slot {name}: queued mainland tele+setvar (scene 2)"
                        );
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
                            s.tile_x = lp.x;
                            s.tile_z = lp.z;
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
            });
        }));
    }

    Play { statuses, handles }
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
}
