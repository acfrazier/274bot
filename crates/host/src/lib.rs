//! 274 bot host: one OS thread per client slot.

mod auto_run;
pub mod lean;
pub mod login_queue;
mod slot;
mod slot_io;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use api::interact::set_run;
use api::settle::{modal_delta, Settle};
use api::snapshot::{Family, GameSnapshot};
use auto_run::auto_run_tick;
use client::client::{Client, ClientConfig};
use client::config::{Cache, IfType};
use vault::Profile;

pub use slot::{dirty_families, should_emit_tick, DirtyFamilies, DrainResult, Pump};
pub use slot_io::{map_image_to_applet, InputEv, PixelBuf, SlotInput};

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
        || std::env::var("BOT_DEBUG")
            .map(|v| v == "1")
            .unwrap_or(false)
}

/// The 274 client's frame time: one `mainloop` pass every 20 ms.
const FRAME_MS: Duration = Duration::from_millis(20);

/// Host: spawns and owns per-client slot threads.
pub struct Host;

/// Build a slot `Client` from a process-wide cache (no second unpack / CRC
/// probe). `error_loading` is false after a successful `from_shared`.
pub fn prepare_client(
    config: ClientConfig,
    uid: i32,
    cache: Arc<Cache>,
    ifaces: Vec<Option<IfType>>,
) -> Client {
    let mut client = Client::from_shared(config, cache, ifaces);
    client.login_uid = uid;
    client
}

impl Host {
    /// Spawn one slot thread. Builds a `Client` from the shared cache/iface
    /// template (see [`prepare_client`]), then drives `mainloop` via
    /// [`Host::run_client`] at 20 ms. Login + `maininit` live in host-play
    /// so the FIFO sits in front of the handshake.
    pub fn spawn_slot(
        config: ClientConfig,
        profile: Profile,
        cache: Arc<Cache>,
        ifaces_template: Vec<Option<IfType>>,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let mut client = prepare_client(config, profile.uid, cache, ifaces_template);

            if debug_enabled() {
                eprintln!("[host] slot {}: thread up", profile.username);
            }

            Self::run_client(&mut client, &profile.username, None, None, |_, _, _| {}, |_| false);
        })
    }

    /// Drive one client's `mainloop` at 20 ms until `probe` returns true
    /// (checked before every tick, so the slot thread can stop a rail ✕
    /// or return to its control loop within one frame). Each
    /// tick [`Host::client_tick`] runs `observe` **before** [`Host::client_frame`]
    /// so panel `set_draw` (focused + renderer checkbox) gates **this**
    /// `mainredraw`, then drains input, latches the click, runs `mainloop`,
    /// and copies pixels only while `client.draw`. Dirty snapshot families
    /// rebuild from [`DrainResult::dirty`] (not `Pump::dirty()` after
    /// drain); settle runs when a family gen moved; think (auto-run) reads
    /// energy from the snapshot stat view when it has been rebuilt. The
    /// third observe arg is the count of accepted auto-run `set_run(true)`
    /// sends (from the previous tick).
    pub fn run_client<F, P>(
        client: &mut Client,
        username: &str,
        input: Option<Arc<SlotInput>>,
        pixels: Option<Arc<PixelBuf>>,
        mut observe: F,
        mut probe: P,
    ) where
        F: FnMut(&mut Client, &str, u32),
        P: FnMut(&mut Client) -> bool,
    {
        let mut slot = SlotLoop::new();
        let mut run_sends = 0u32;
        loop {
            if probe(client) {
                return;
            }
            let start = std::time::Instant::now();
            Self::client_tick(
                client,
                &mut slot,
                username,
                input.as_deref(),
                pixels.as_deref(),
                &mut run_sends,
                &mut observe,
            );
            // Java GameShell sleeps the leftover of 20 ms *after* the work.
            // A fixed sleep *before* the tick made the period 20 ms + Pix3D
            // (slow picture, extra idle). If the tick overruns, skip sleep.
            if let Some(rest) = FRAME_MS.checked_sub(start.elapsed()) {
                thread::sleep(rest);
            }
        }
    }

    /// One host tick: `observe` first (panel latches `set_draw`), then one
    /// [`Host::client_frame`]. Unfocused / renderer-off slots skip Pix3D
    /// on this tick, not the next.
    #[allow(private_interfaces)]
    pub fn client_tick<F>(
        client: &mut Client,
        slot: &mut SlotLoop,
        username: &str,
        input: Option<&SlotInput>,
        pixels: Option<&PixelBuf>,
        run_sends: &mut u32,
        observe: &mut F,
    ) where
        F: FnMut(&mut Client, &str, u32),
    {
        observe(client, username, *run_sends);
        Self::client_frame(client, slot, username, input, pixels, run_sends);
    }

    /// One 20 ms frame: drain optional input into the shell, latch the
    /// click, run one `mainloop` pass, render the frame (`mainredraw` —
    /// the draw switch gates the paint; this is what fills `draw_area` and
    /// runs the `game_draw` minimenu build, mirroring `Client::run`),
    /// drain gens, then copy the redraw into optional pixels. `run_sends`
    /// is overwritten with the slot's running count of accepted auto-run
    /// sends.
    /// `SlotLoop` stays module-private (tests live in this module); the
    /// pub surface exists so `run_client` and the tests share the frame.
    #[allow(private_interfaces)]
    pub fn client_frame(
        client: &mut Client,
        slot: &mut SlotLoop,
        username: &str,
        input: Option<&SlotInput>,
        pixels: Option<&PixelBuf>,
        run_sends: &mut u32,
    ) {
        if let Some(inp) = input {
            inp.drain(&mut client.shell);
        }
        client.shell.latch_click();
        let t_loop = std::time::Instant::now();
        client.mainloop();
        slot.loop_ns = slot.loop_ns.wrapping_add(t_loop.elapsed().as_nanos() as u64);
        let capture = input.map(|i| i.enabled()).unwrap_or(false);
        let paint = raster_this_tick(
            client.draw,
            capture,
            &mut slot.raster_n,
            &mut slot.raster_was_on,
        );
        if paint {
            let t_r = std::time::Instant::now();
            client.mainredraw();
            slot.raster_ns = slot.raster_ns.wrapping_add(t_r.elapsed().as_nanos() as u64);
            slot.paint_n = slot.paint_n.wrapping_add(1);
        } else {
            slot.skip_n = slot.skip_n.wrapping_add(1);
        }
        client.loop_ns = slot.loop_ns;
        client.raster_ns = slot.raster_ns;
        client.paint_n = slot.paint_n;
        client.skip_n = slot.skip_n;
        slot.log_n = slot.log_n.wrapping_add(1);
        if debug_enabled() && slot.log_n % 50 == 0 {
            eprintln!(
                "[host] slot {username}: loop_us={} raster_us={} paints={} skips={} game_draw={} title={}",
                slot.loop_ns / 1000,
                slot.raster_ns / 1000,
                slot.paint_n,
                slot.skip_n,
                client.game_draw_enters,
                client.title_screen_draw_enters
            );
        }
        let result = slot.after_drain(client);
        if should_emit_tick(result.player_info) && debug_enabled() {
            eprintln!("[host] slot {username}: tick");
        }
        if paint {
            if let Some(px) = pixels {
                px.copy_from(
                    &client.draw_area.pixels,
                    client.draw_area.width,
                    client.draw_area.height,
                );
            }
        }
        *run_sends = slot.run_sends;
    }
}

/// rs2b0t rail: watch-only is **1 fps** (every 50 ticks of 20 ms). The first
/// tick after draw rises paints immediately so checking the box is not a
/// cold hitch. Capture paints every tick (minimenu). Draw off never paints.
const WATCH_RASTER_TICKS: u32 = 50;

fn raster_this_tick(draw: bool, capture: bool, n: &mut u32, was_on: &mut bool) -> bool {
    if !draw {
        *was_on = false;
        return false;
    }
    if capture {
        *was_on = true;
        return true;
    }
    let rising = !*was_on;
    *was_on = true;
    if rising {
        *n = 0;
        return true;
    }
    *n = n.wrapping_add(1);
    *n % WATCH_RASTER_TICKS == 0
}

/// Per-slot post-drain state: snapshot, settle, auto-run.
struct SlotLoop {
    pump: Pump,
    snapshot: GameSnapshot,
    settle: Settle,
    run_on: bool,
    run_sends: u32,
    last_modal: Option<i32>,
    raster_n: u32,
    raster_was_on: bool,
    loop_ns: u64,
    raster_ns: u64,
    paint_n: u64,
    skip_n: u64,
    log_n: u32,
}

impl SlotLoop {
    fn new() -> Self {
        Self {
            pump: Pump::new(),
            snapshot: GameSnapshot::new(),
            settle: Settle::default(),
            run_on: false,
            run_sends: 0,
            last_modal: None,
            raster_n: 0,
            raster_was_on: false,
            loop_ns: 0,
            raster_ns: 0,
            paint_n: 0,
            skip_n: 0,
            log_n: 0,
        }
    }

    fn after_drain(&mut self, client: &mut Client) -> DrainResult {
        let result = self.pump.drain(client.gens);
        rebuild_dirty(&mut self.snapshot, client, result.dirty);
        if result.dirty.any() {
            let after = if client.main_modal_id >= 0 {
                Some(client.main_modal_id)
            } else {
                None
            };
            let (opened, closed) = modal_delta(self.last_modal, after);
            if opened.is_some() {
                self.settle.modal_opened = opened;
            }
            if closed.is_some() {
                self.settle.modal_closed = closed;
            }
            self.last_modal = after;
            if result.player_info {
                self.settle.ticks = self.settle.ticks.saturating_add(1);
            }
        }

        let energy = if self.snapshot.gens().stat > 0 {
            self.snapshot.runenergy()
        } else {
            client.runenergy
        };
        if let Some(echo) = run_echo(client) {
            self.run_on = echo;
        }
        if energy == 0 {
            // Cannot be running; wins over a stale run-on echo.
            self.run_on = false;
        }
        if auto_run_tick(energy, self.run_on) && set_run(client, true) {
            self.run_on = true;
            self.run_sends += 1;
        }
        result
    }
}

fn rebuild_dirty(snapshot: &mut GameSnapshot, client: &Client, dirty: DirtyFamilies) {
    if dirty.npc {
        snapshot.rebuild_family(client, Family::Npc);
    }
    if dirty.player {
        snapshot.rebuild_family(client, Family::Player);
    }
    if dirty.inv {
        snapshot.rebuild_family(client, Family::Inv);
    }
    if dirty.varp {
        snapshot.rebuild_family(client, Family::Varp);
    }
    if dirty.stat {
        snapshot.rebuild_family(client, Family::Stat);
    }
    if dirty.chat {
        snapshot.rebuild_family(client, Family::Chat);
    }
    if dirty.scene {
        snapshot.rebuild_family(client, Family::Scene);
    }
}

/// Run on/off from the orb pair. 152 visible and 153 hidden → running;
/// the inverse → walking. Both the same (unpacked defaults) is unknown —
/// a packed jag starts with `hide = false` on every component.
fn run_echo(client: &Client) -> Option<bool> {
    let off = client.ifaces.get(152).and_then(|s| s.as_ref())?;
    let on = client.ifaces.get(153).and_then(|s| s.as_ref())?;
    if off.hide == on.hide {
        return None;
    }
    Some(!off.hide)
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::interact::RUN_ORB_IFACE;
    use client::io::ClientProt;

    fn cfg() -> ClientConfig {
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: true,
        }
    }

    #[test]
    fn prepare_client_shares_arc_and_clears_error_loading() {
        let cache = Arc::new(Cache::default());
        let a = prepare_client(cfg(), 1, Arc::clone(&cache), vec![]);
        let b = prepare_client(cfg(), 2, Arc::clone(&cache), vec![]);
        assert!(Arc::ptr_eq(&a.cache, &b.cache));
        assert!(Arc::ptr_eq(&a.cache, &cache));
        assert!(!a.error_loading);
        assert!(!b.error_loading);
        assert_eq!(a.login_uid, 1);
        assert_eq!(b.login_uid, 2);
    }

    #[test]
    fn slot_rebuilds_from_drain_dirty_not_post_drain_pump_dirty() {
        let mut client = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let mut slot = SlotLoop::new();
        client.gens.npc = 1;
        let result = slot.after_drain(&mut client);
        assert!(result.dirty.npc);
        assert_eq!(slot.snapshot.gens().npc, 1);
        assert_eq!(slot.pump.dirty(client.gens), DirtyFamilies::default());
    }

    #[test]
    fn auto_run_20_0_20_sends_twice() {
        let mut client = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let mut slot = SlotLoop::new();
        client.runenergy = 20;
        client.gens.stat = 1;
        slot.after_drain(&mut client);
        assert_eq!(slot.run_sends, 1);
        assert_eq!(client.out.data()[0], ClientProt::IF_BUTTON.id as u8);
        let iface = u16::from_be_bytes([client.out.data()[1], client.out.data()[2]]);
        assert_eq!(iface, RUN_ORB_IFACE as u16);

        client.out.pos = 0;
        client.runenergy = 0;
        client.gens.stat = 2;
        slot.after_drain(&mut client);
        assert!(!slot.run_on);
        assert_eq!(slot.run_sends, 1);

        client.runenergy = 20;
        client.gens.stat = 3;
        slot.after_drain(&mut client);
        assert_eq!(slot.run_sends, 2);
        assert!(slot.run_on);
    }

    #[test]
    fn already_running_echo_does_not_send() {
        let mut ifaces = vec![None; 154];
        ifaces[152] = Some(IfType {
            hide: false,
            ..IfType::default()
        });
        ifaces[153] = Some(IfType {
            hide: true,
            ..IfType::default()
        });
        let mut client = prepare_client(cfg(), 1, Arc::new(Cache::default()), ifaces);
        client.runenergy = 20;
        client.gens.stat = 1;
        let mut slot = SlotLoop::new();
        slot.after_drain(&mut client);
        assert_eq!(slot.run_sends, 0, "already on → no extra send");
        assert!(slot.run_on);
    }

    #[test]
    fn unpacked_ifaces_both_visible_still_sends() {
        let mut ifaces = vec![None; 154];
        ifaces[152] = Some(IfType::default());
        ifaces[153] = Some(IfType::default());
        let mut client = prepare_client(cfg(), 1, Arc::new(Cache::default()), ifaces);
        client.runenergy = 20;
        client.gens.stat = 1;
        let mut slot = SlotLoop::new();
        slot.after_drain(&mut client);
        assert_eq!(slot.run_sends, 1);
    }

    #[test]
    fn client_frame_applies_click_only_when_input_enabled() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let inp = SlotInput::new();
        let (tx, rx) = std::sync::mpsc::channel();
        inp.connect_rx(rx);
        tx.send(InputEv::Down { button: 1, x: 20, y: 20 }).unwrap();
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        inp.set_enabled(false);
        Host::client_frame(&mut c, &mut slot, "t", Some(&inp), None, &mut sends);
        assert_eq!(c.shell.mouse_click_button, 0);
        inp.set_enabled(true);
        Host::client_frame(&mut c, &mut slot, "t", Some(&inp), None, &mut sends);
        assert_eq!(c.shell.mouse_click_button, 1);
    }

    #[test]
    fn client_frame_skips_pixel_copy_when_draw_off() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let buf = PixelBuf::new();
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        // Renderer off: the redraw area is never copied into the buffer.
        c.set_draw(false);
        Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends);
        assert!(buf.snapshot().is_empty(), "draw off must not copy pixels");
        // Renderer on: `mainredraw` runs the frame render pass (with no
        // title assets in this test the paint is empty, but the frame still
        // copies a full applet into the buffer; the non-zero paint is
        // proven live by the panel_view e2e).
        c.set_draw(true);
        Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends);
        assert_eq!(
            buf.snapshot().len(),
            (client::client::APPLET_W * client::client::APPLET_H) as usize
        );
    }

    #[test]
    fn client_frame_draw_off_does_not_enter_game_or_title_draw() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        c.set_draw(false);
        for _ in 0..3 {
            Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends);
        }
        assert_eq!(c.game_draw_enters, 0);
        assert_eq!(c.title_screen_draw_enters, 0);
        assert_eq!(slot.skip_n, 3);
        assert_eq!(slot.paint_n, 0);
        assert!(slot.loop_ns > 0, "mainloop still ran");
    }

    #[test]
    fn client_frame_stamps_loop_counters_on_client() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        c.set_draw(false);
        Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends);
        assert_eq!(c.skip_n, 1);
        assert_eq!(c.paint_n, 0);
        assert_eq!(c.loop_ns, slot.loop_ns);
        assert!(c.loop_ns > 0);
    }

    #[test]
    fn client_frame_draw_on_increments_title_and_paint_n() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        c.set_draw(true);
        Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends);
        assert!(c.title_screen_draw_enters >= 1);
        assert_eq!(c.game_draw_enters, 0);
        assert_eq!(slot.paint_n, 1);
    }

    #[test]
    fn client_tick_latches_observe_draw_before_redraw() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let buf = PixelBuf::new();
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        assert!(!c.draw, "slots start with the renderer off");
        // Observe-after-frame would skip this paint (draw still false
        // during mainredraw). Observe-before must copy this tick.
        Host::client_tick(
            &mut c,
            &mut slot,
            "t",
            None,
            Some(&buf),
            &mut sends,
            &mut |c, _, _| c.set_draw(true),
        );
        assert!(c.draw);
        assert_eq!(
            buf.snapshot().len(),
            (client::client::APPLET_W * client::client::APPLET_H) as usize
        );
        let gen = buf.generation();
        Host::client_tick(
            &mut c,
            &mut slot,
            "t",
            None,
            Some(&buf),
            &mut sends,
            &mut |c, _, _| c.set_draw(false),
        );
        assert!(!c.draw);
        assert_eq!(
            buf.generation(),
            gen,
            "renderer off must not copy pixels this tick"
        );
    }

    #[test]
    fn raster_this_tick_watch_is_one_fps_capture_is_every_tick() {
        let mut n = 0;
        let mut on = false;
        assert!(!raster_this_tick(false, false, &mut n, &mut on));
        assert!(raster_this_tick(true, false, &mut n, &mut on), "rising edge paints now");
        for _ in 0..(WATCH_RASTER_TICKS - 1) {
            assert!(!raster_this_tick(true, false, &mut n, &mut on));
        }
        assert!(raster_this_tick(true, false, &mut n, &mut on));
        assert!(raster_this_tick(true, true, &mut n, &mut on));
        assert!(raster_this_tick(true, true, &mut n, &mut on));
        on = false;
        assert!(
            raster_this_tick(true, false, &mut n, &mut on),
            "draw rising after off paints immediately"
        );
    }

    #[test]
    fn watch_only_draw_copies_first_tick_then_one_fps() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        c.set_draw(true);
        let buf = PixelBuf::new();
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends);
        assert_eq!(buf.generation(), 1);
        for _ in 0..(WATCH_RASTER_TICKS - 1) {
            Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends);
            assert_eq!(buf.generation(), 1);
        }
        Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends);
        assert_eq!(buf.generation(), 2);
    }

    #[test]
    fn capture_draw_copies_every_tick() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        c.set_draw(true);
        let buf = PixelBuf::new();
        let inp = SlotInput::new();
        inp.set_enabled(true);
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        Host::client_frame(&mut c, &mut slot, "t", Some(&inp), Some(&buf), &mut sends);
        Host::client_frame(&mut c, &mut slot, "t", Some(&inp), Some(&buf), &mut sends);
        assert_eq!(buf.generation(), 2);
    }
}
