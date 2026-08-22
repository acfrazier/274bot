//! Panel session: owns the unlocked vault, the running slot map, the shared
//! `Focus`, and per-slot pixel/input channels. The dear-app frame reads
//! `Session`; slot threads stay in `host_play` (spawned via `run_with_io`
//! with per-profile `PixelBuf`/`SlotInput`, keeping the login FIFO and the
//! mainland hop).

use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use host::{InputEv, PixelBuf, SlotInput, map_image_to_applet};
use host_play::{open_vault, run_with_io, Play, PlayOptions, SlotStatus};
use vault::{Profile, Vault};

use crate::focus::should_draw;

const DEFAULT_PORT: u16 = 43594;

/// Vault path used by panel-play (`~/.274bot/vault`, the same file host-play
/// uses).
pub fn default_vault_path() -> PathBuf {
    match env::var("HOME") {
        Ok(home) => PathBuf::from(format!("{home}/.274bot/vault")),
        Err(_) => PathBuf::from(".274bot/vault"),
    }
}

fn default_cache_dir() -> String {
    match env::var("HOME") {
        Ok(home) => format!("{home}/experiments/Server/engine/data/pack/client"),
        Err(_) => "experiments/Server/engine/data/pack/client".into(),
    }
}

/// Panel-side per-slot IO: the pixel buffer the slot paints into while its
/// renderer is on, and the input channel it drains only while capture is on.
pub struct SlotIo {
    pub input: Arc<SlotInput>,
    pub pixels: Arc<PixelBuf>,
}

/// Click-through helper: maps a click inside the Game Image (local coords,
/// Image widget size) to applet coords and enqueues `InputEv::Down`. No-op
/// when the capture channel has been dropped (capture off) or the point is
/// outside the Image. This is the click path; the real app also sends
/// Move/Up from the ImGui mouse.
pub fn maybe_send_click(tx: &Option<Sender<InputEv>>, lx: f32, ly: f32, w: f32, h: f32) {
    let Some(tx) = tx else { return; };
    let Some((x, y)) = map_image_to_applet(lx, ly, w, h) else { return; };
    let _ = tx.send(InputEv::Down { button: 1, x, y });
}

pub struct Session {
    /// Shared focus policy; slot threads read it every frame (observe) to
    /// apply `client.set_draw(should_draw(&focus))`.
    pub focus: Arc<Mutex<crate::focus::Focus>>,
    pub vault: Option<Vault>,
    /// Last vault/connection error shown in the banner.
    pub error: Option<String>,
    /// Running slot threads and their shared statuses (created at unlock).
    pub play: Option<Play>,
    /// Per-username slot IO.
    pub slots: HashMap<String, SlotIo>,
    /// The focused slot's live capture sender; `None` while capture is off,
    /// so UI send paths no-op.
    pub capture_tx: Option<Sender<InputEv>>,
    /// Mainland checkbox; the per-frame hook queues the hop at scene 2.
    pub mainland: Arc<AtomicBool>,
    /// Panel log lines (status transitions), capped at [`LOG_CAP`].
    pub log: Arc<Mutex<Vec<String>>>,
    /// Vault passphrase scratch buffer for the in-panel unlock prompt.
    pub pass_scratch: String,
    /// Last status poll (delta source for the log).
    pub statuses: Vec<SlotStatus>,
    mainland_sent: Arc<Mutex<HashSet<String>>>,
    options: PlayOptions,
}

/// Keep the panel log bounded.
const LOG_CAP: usize = 200;

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// Empty session: no vault, no slots, default `PlayOptions` (same engine
    /// defaults as the host-play CLI). Unlock via [`Session::unlock`].
    pub fn new() -> Self {
        Self {
            focus: Arc::new(Mutex::new(crate::focus::Focus {
                focused: None,
                renderer: true,
                game_pane_open: true,
                capture: false,
            })),
            vault: None,
            error: None,
            play: None,
            slots: HashMap::new(),
            capture_tx: None,
            mainland: Arc::new(AtomicBool::new(
                env::var("BOT_MAINLAND").as_deref() == Ok("1"),
            )),
            log: Arc::new(Mutex::new(Vec::new())),
            pass_scratch: String::new(),
            statuses: Vec::new(),
            mainland_sent: Arc::new(Mutex::new(HashSet::new())),
            options: PlayOptions {
                host: "127.0.0.1".into(),
                port: DEFAULT_PORT,
                cache_dir: default_cache_dir(),
                lowmem: true,
                // The panel queues the mainland hop itself (live checkbox),
                // so the spawn-time `PlayOptions.mainland` stays false.
                mainland: false,
            },
        }
    }

    /// Unlock (or first-run create) the vault and spawn every profile as a
    /// host slot with per-profile pixel + input channels. Returns whether
    /// the unlock succeeded; failures land in [`Session::error`].
    pub fn unlock(&mut self, pass: &str) -> bool {
        let path = default_vault_path();
        match open_vault(&path, pass) {
            Ok(vault) => {
                let profiles: Vec<Profile> = vault.profiles().cloned().collect();
                self.error = None;
                self.spawn_all(vault, profiles);
                true
            }
            Err(e) => {
                self.error = Some(e.to_string());
                false
            }
        }
    }

    /// Spawn one slot thread per profile via `host_play::run_with_io`. Each
    /// slot gets its own `PixelBuf` + `SlotInput` (never shared across
    /// slots). The per-frame hook applies the focus `set_draw` switch and
    /// the live mainland hop.
    fn spawn_all(&mut self, vault: Vault, profiles: Vec<Profile>) {
        let mut io: HashMap<String, (Arc<SlotInput>, Arc<PixelBuf>)> = HashMap::new();
        for p in &profiles {
            io.insert(p.username.clone(), (SlotInput::new(), PixelBuf::new()));
        }
        let focus = Arc::clone(&self.focus);
        let log = Arc::clone(&self.log);
        let mainland = Arc::clone(&self.mainland);
        let mainland_sent = Arc::clone(&self.mainland_sent);
        let options = self.options.clone();
        let play = run_with_io(
            &options,
            profiles,
            |name| match io.get(name) {
                Some((input, pixels)) => (Some(Arc::clone(input)), Some(Arc::clone(pixels))),
                None => (None, None),
            },
            move |c, name| {
                c.set_draw(should_draw(&focus.lock().unwrap()));
                if mainland.load(Ordering::Relaxed)
                    && c.ingame
                    && c.scene_state == 2
                    && mainland_sent.lock().unwrap().insert(name.to_string())
                {
                    api::interact::mainland_hop(c);
                    log.lock().unwrap().push(format!("{name}: mainland hop queued"));
                }
            },
        );
        self.slots = io
            .into_iter()
            .map(|(name, (input, pixels))| (name, SlotIo { input, pixels }))
            .collect();
        self.play = Some(play);
        self.statuses = self
            .play
            .as_ref()
            .map(|p| p.statuses())
            .unwrap_or_default();
        self.vault = Some(vault);
    }

    /// Poll slot statuses and append log lines for transitions (slot up,
    /// login errors, ingame, scene changes). Call once per UI frame.
    pub fn pump_status(&mut self) {
        let Some(play) = &self.play else { return; };
        let current = play.statuses();
        {
            let mut log = self.log.lock().unwrap();
            for s in &current {
                let prev = self.statuses.iter().find(|p| p.username == s.username);
                match prev {
                    None => {
                        log.push(format!("{}: slot up", s.username));
                        if let Some(e) = &s.error {
                            log.push(format!("{}: login {}", s.username, e));
                        }
                    }
                    Some(p) => {
                        if p.error.is_none() && s.error.is_some() {
                            log.push(format!(
                                "{}: login {}",
                                s.username,
                                s.error.as_deref().unwrap_or_default()
                            ));
                        }
                        if !p.ingame && s.ingame {
                            log.push(format!("{}: ingame", s.username));
                        }
                        if p.scene_state != s.scene_state {
                            log.push(format!("{}: scene {}", s.username, s.scene_state));
                        }
                    }
                }
            }
            while log.len() > LOG_CAP {
                log.remove(0);
            }
        }
        self.statuses = current;
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

    /// The focused slot's pixel buffer (None when nothing is focused).
    pub fn focused_pixels(&self) -> Option<Arc<PixelBuf>> {
        self.focused_slot().map(|s| Arc::clone(&s.pixels))
    }

    fn focused_slot(&self) -> Option<&SlotIo> {
        let name = self.focused_name()?;
        self.slots.get(&name)
    }

    /// Switch the focused profile. Capture follows the new focus when the
    /// single capture toggle is on (never two keyboards).
    pub fn select(&mut self, name: &str) {
        let mut focus = self.focus.lock().unwrap();
        if focus.focused.as_deref() == Some(name) {
            return;
        }
        let old = focus.focused.clone();
        focus.focused = Some(name.to_string());
        let capture = focus.capture;
        drop(focus);
        if capture {
            if let Some(old) = old {
                if let Some(slot) = self.slots.get(&old) {
                    slot.input.set_enabled(false);
                }
            }
            self.capture_on(name);
        } else {
            self.capture_tx = None;
        }
    }

    /// Renderer checkbox. The slot threads apply `set_draw` from the focus
    /// in their per-frame observe hook, so no other wiring is needed.
    pub fn set_renderer(&mut self, on: bool) {
        self.focus.lock().unwrap().renderer = on;
    }

    /// Capture checkbox. On: attach a fresh channel and enable the focused
    /// slot's drain. Off: disable the drain and drop the sender so the UI
    /// cannot enqueue (the slot thread does no `try_recv` while disabled).
    pub fn set_capture(&mut self, on: bool) {
        self.focus.lock().unwrap().capture = on;
        if on {
            let name = self.focused_name();
            match name {
                Some(name) => self.capture_on(&name),
                None => self.capture_tx = None,
            }
        } else {
            self.capture_off();
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
}

#[cfg(test)]
mod tests {
    use super::maybe_send_click;
    use host::InputEv;

    #[test]
    fn maybe_send_click_is_noop_without_tx() {
        maybe_send_click(&None, 1.0, 1.0, 765.0, 503.0);
    }

    #[test]
    fn maybe_send_click_sends_when_tx_present() {
        let (tx, rx) = std::sync::mpsc::channel();
        maybe_send_click(&Some(tx), 0.0, 0.0, 765.0, 503.0);
        match rx.try_recv() {
            Ok(InputEv::Down { x, y, .. }) => assert_eq!((x, y), (0, 0)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn maybe_send_click_outside_image_sends_nothing() {
        let (tx, rx) = std::sync::mpsc::channel();
        maybe_send_click(&Some(tx), -5.0, 10.0, 765.0, 503.0);
        assert!(rx.try_recv().is_err());
    }
}
