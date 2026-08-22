//! 274 bot host: one OS thread per client slot.

mod auto_run;
mod login_queue;
mod slot;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use api::interact::set_run;
use auto_run::auto_run_tick;
use client::client::{Client, ClientConfig};
use client::config::{Cache, IfType};
use vault::Profile;

pub use slot::{DirtyFamilies, DrainResult, Pump, dirty_families, should_emit_tick};

/// Host debug toggle, set by host-play's `--debug`; `BOT_DEBUG=1` enables it
/// via [`debug_enabled`] regardless.
static DEBUG: AtomicBool = AtomicBool::new(false);

/// Enable host debug logging (host-play maps `--debug` to this).
pub fn set_debug(enabled: bool) {
    DEBUG.store(enabled, Ordering::Relaxed);
}

/// Host debug logging is on when `BOT_DEBUG=1` or [`set_debug`] ran.
pub fn debug_enabled() -> bool {
    DEBUG.load(Ordering::Relaxed)
        || std::env::var("BOT_DEBUG").map(|v| v == "1").unwrap_or(false)
}

/// The 274 client's frame time: one `mainloop` pass every 20 ms.
const FRAME_MS: Duration = Duration::from_millis(20);

/// Host: spawns and owns per-client slot threads.
pub struct Host;

impl Host {
    /// Spawn one slot thread. Builds a `Client` from `config` on the thread,
    /// stamps it with the profile uid and the shared cache/iface template,
    /// then drives `mainloop` at 20 ms. After each pass the drain pump diffs
    /// `Client.gens`; a `PLAYER_INFO` this drain synthesizes `on_server_tick`.
    /// (The snapshot-family rebuild hooks in at Task 8.)
    pub fn spawn_slot(
        config: ClientConfig,
        profile: Profile,
        cache: Arc<Cache>,
        ifaces_template: Vec<Option<IfType>>,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let mut client = Client::new(config);
            client.login_uid = profile.uid;
            client.cache = cache;
            client.ifaces = ifaces_template;

            if debug_enabled() {
                eprintln!("[host] slot {}: thread up", profile.username);
            }

            let mut pump = Pump::new();
            // Auto-run state: true after the host sent `set_run(true)` and
            // until the player stops running. Start unknown → off, so the
            // first threshold crossing triggers.
            let mut run_on = false;
            loop {
                thread::sleep(FRAME_MS);
                client.mainloop();
                let result = pump.drain(client.gens);
                if should_emit_tick(result.player_info) {
                    on_server_tick(&mut client, &profile.username, &mut run_on);
                }
            }
        })
    }
}

/// Synthesized `on_server_tick`: fired once per drain that applied a
/// `PLAYER_INFO`. Host think hooks here — auto-run is the only behaviour so
/// far; `run_on` is the slot's tracked run state.
fn on_server_tick(client: &mut Client, username: &str, run_on: &mut bool) {
    if debug_enabled() {
        eprintln!("[host] slot {username}: tick");
    }
    if auto_run_tick(client.runenergy, *run_on) {
        if set_run(client, true) {
            *run_on = true;
        }
    }
}

#[test]
fn workspace_compiles() {}
