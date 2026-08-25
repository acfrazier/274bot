//! 274 bot host: one OS thread per client slot.

mod auto_run;
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
use client::render::backend::FrameOutput;
use client::render::Renderer;
use vault::Profile;

pub use slot::{dirty_families, should_emit_tick, DirtyFamilies, DrainResult, Pump};
pub use slot_io::{map_image_to_applet, wake_channel, FrameBuf, InputEv, SlotInput, SlotPark, SlotWake};

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

/// Park bound for an idle slot: the 274 server's game-tick cadence
/// (`PLAYER_INFO` every ~600 ms), so a missed/absent packet never leaves a
/// parked slot asleep past one tick.
const IDLE_PARK_MS: Duration = Duration::from_millis(600);

/// Park bound while the client socket is stalled (a wake consumed no bytes:
/// EOF, or a packet still mid-flight). Shorter than the tick bound so the
/// game-loop watchdog still notices a dead server, but long enough that a
/// permanently-readable socket cannot busy-spin the slot.
const STALL_PARK_MS: Duration = Duration::from_millis(200);

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

            Self::run_client(
                &mut client,
                &profile.username,
                None,
                None,
                None,
                |_, _, _| false,
                |_| false,
            );
        })
    }

    /// Drive one client's `mainloop` at the slot cadence until `probe`
    /// returns true (checked before every tick, so the slot thread can stop
    /// a rail ✕ or return to its control loop within one frame). Each
    /// tick [`Host::client_tick`] runs `observe` **before** [`Host::client_frame`]
    /// so the panel can latch slot state (draw/focus) before the paint
    /// decision for **this** tick, then drains input, latches the click,
    /// runs `mainloop`, and renders (via the slot's optional `Renderer`)
    /// only while `client.draw` is on. Dirty snapshot families
    /// rebuild from [`DrainResult::dirty`] (not `Pump::dirty()` after
    /// drain); settle runs when a family gen moved; think (auto-run) reads
    /// energy from the snapshot stat view when it has been rebuilt. The
    /// third observe arg is the count of accepted auto-run `set_run(true)`
    /// sends (from the previous tick); observe's return is whether the slot
    /// has script/cheat/nav work, which keeps a busy slot on the frame loop.
    ///
    /// The scheduler is event-driven: a slot that draws or captures input
    /// keeps the fixed 20 ms [`FRAME_MS`] loop — that loop *is* the render
    /// cadence. Everything else parks on `poll(2)` over the client socket's
    /// readability, the `ctl` control wake (focus/draw/stop/spawn), and the
    /// game-tick timeout ([`IDLE_PARK_MS`]), waking once per park to drain
    /// the socket and re-evaluate. Packets are never dropped: a readable
    /// socket wakes the park, and `mainloop` is the first thing that drains
    /// it. A wake that consumed no bytes (EOF, partial packet) skips the
    /// socket on the next park so it cannot busy-spin.
    pub fn run_client<F, P>(
        client: &mut Client,
        username: &str,
        input: Option<Arc<SlotInput>>,
        mailbox: Option<Arc<FrameBuf>>,
        ctl: Option<Arc<SlotPark>>,
        mut observe: F,
        mut probe: P,
    ) where
        F: FnMut(&mut Client, &str, u32) -> bool,
        P: FnMut(&mut Client) -> bool,
    {
        let mut slot = SlotLoop::new();
        let mut run_sends = 0u32;
        // Prime busy so the first pass runs one tick: the cadence decision
        // must see the observe hook's draw/capture/busy mirror, which only
        // exists after a tick has run.
        let mut busy = true;
        // A socket wake that consumed no bytes leaves the socket readable,
        // so re-polling it would busy-spin; skip the socket until a tick
        // consumes bytes again.
        let mut socket_stalled = false;
        loop {
            if probe(client) {
                return;
            }
            if frame_cadence(client, input.as_deref()) || busy {
                socket_stalled = false;
                let start = std::time::Instant::now();
                busy = Self::client_tick(
                    client,
                    &mut slot,
                    username,
                    input.as_deref(),
                    mailbox.as_deref(),
                    &mut run_sends,
                    &mut observe,
                );
                // Java GameShell sleeps the leftover of 20 ms *after* the work.
                // A fixed sleep *before* the tick made the period 20 ms + Pix3D
                // (slow picture, extra idle). If the tick overruns, skip sleep.
                if let Some(rest) = FRAME_MS.checked_sub(start.elapsed()) {
                    thread::sleep(rest);
                }
                continue;
            }
            // Idle: park until a packet, a control kick, or the game-tick
            // bound, then run one tick (drain the socket / apply the panel's
            // `set_draw`) and re-evaluate.
            let before = stream_bytes(client);
            let timeout = if socket_stalled {
                STALL_PARK_MS
            } else {
                IDLE_PARK_MS
            };
            let reason = park(client, ctl.as_deref(), !socket_stalled, timeout);
            busy = Self::client_tick(
                client,
                &mut slot,
                username,
                input.as_deref(),
                mailbox.as_deref(),
                &mut run_sends,
                &mut observe,
            );
            if stream_bytes(client) != before {
                socket_stalled = false;
            } else if reason == ParkWake::Socket {
                socket_stalled = true;
            }
        }
    }

    /// One host tick: `observe` first (the panel latches slot state), then
    /// one [`Host::client_frame`]. Unfocused / renderer-off slots skip the
    /// paint on this tick, not the next. Returns observe's busy flag (the
    /// slot has script/cheat/nav work and must not be parked).
    #[allow(private_interfaces)]
    pub fn client_tick<F>(
        client: &mut Client,
        slot: &mut SlotLoop,
        username: &str,
        input: Option<&SlotInput>,
        mailbox: Option<&FrameBuf>,
        run_sends: &mut u32,
        observe: &mut F,
    ) -> bool
    where
        F: FnMut(&mut Client, &str, u32) -> bool,
    {
        let busy = observe(client, username, *run_sends);
        Self::client_frame(client, slot, username, input, mailbox, run_sends);
        busy
    }

    /// One 20 ms frame: drain optional input into the shell, latch the
    /// click, run one `mainloop` pass, render the frame (the slot's
    /// optional `Renderer` — `client.draw` gates the paint; a drawing
    /// slot stores the rendered `FrameOutput` into the optional mailbox,
    /// mirroring `Client::run`), then drain gens. The panel samples the
    /// mailbox: `FrameBuf::snapshot` packs the `PixMap` CPU path, and
    /// Task 4c binds `FrameOutput::Texture` directly. `run_sends` is
    /// overwritten with the slot's running count of accepted auto-run
    /// sends.
    /// `SlotLoop` stays module-private (tests live in this module); the
    /// pub surface exists so `run_client` and the tests share the frame.
    #[allow(private_interfaces)]
    pub fn client_frame(
        client: &mut Client,
        slot: &mut SlotLoop,
        username: &str,
        input: Option<&SlotInput>,
        mailbox: Option<&FrameBuf>,
        run_sends: &mut u32,
    ) {
        if let Some(inp) = input {
            inp.drain(&mut client.shell);
        }
        client.shell.latch_click();
        let t_loop = std::time::Instant::now();
        client.mainloop();
        slot.loop_ns = slot
            .loop_ns
            .wrapping_add(t_loop.elapsed().as_nanos() as u64);
        let capture = input.map(|i| i.enabled()).unwrap_or(false);
        // Channel-tune / first rebuild: TV static must re-roll every 20 ms,
        // not the 1 fps watch cadence (otherwise the zap is one snow frame
        // a second and looks like a frozen splash).
        let zap = client.ingame && client.scene_state != 2;
        // `client.draw` is the renderer switch the panel latches via
        // `Client::set_draw`; `full_rate` is slot-local for now.
        // TODO(Task 2): wire `full_rate` from the panel/TV control path.
        let paint = raster_this_tick(
            client.draw,
            capture || zap || slot.full_rate,
            &mut slot.raster_n,
            &mut slot.raster_was_on,
        );
        // A drawing slot lazily builds its `Renderer` on the first paint
        // tick; a headless (draw off) slot constructs none and never
        // enters a draw.
        let mut frame: Option<FrameOutput> = None;
        if paint {
            let t_r = std::time::Instant::now();
            let renderer = slot
                .renderer
                .get_or_insert_with(|| Renderer::new(client.config.lowmem));
            // `mainredraw` is the fidelity seam: it runs the `check_minimap`
            // render half (loading splash + minimap image) and
            // `follow_camera` before dispatching game/title draw.
            frame = Some(renderer.mainredraw(client));
            slot.raster_ns = slot.raster_ns.wrapping_add(t_r.elapsed().as_nanos() as u64);
            slot.paint_n = slot.paint_n.wrapping_add(1);
        } else {
            slot.skip_n = slot.skip_n.wrapping_add(1);
        }
        slot.log_n = slot.log_n.wrapping_add(1);
        if debug_enabled() && slot.log_n.is_multiple_of(50) {
            eprintln!(
                "[host] slot {username}: loop_us={} raster_us={} paints={} skips={}",
                slot.loop_ns / 1000,
                slot.raster_ns / 1000,
                slot.paint_n,
                slot.skip_n
            );
        }
        let result = slot.after_drain(client);
        if should_emit_tick(result.player_info) && debug_enabled() {
            eprintln!("[host] slot {username}: tick");
        }
        // The whole `FrameOutput` lands in the mailbox, not just the
        // packed pixels: Task 4c stores `FrameOutput::Texture` here and
        // the panel binds it directly. The GPU backend still composites
        // its scene to CPU, so today the mailbox only ever carries a
        // `PixMap`; `FrameBuf::snapshot` keeps the panel's CPU upload
        // path unchanged.
        if let Some(frame) = frame {
            if let Some(mailbox) = mailbox {
                mailbox.store(frame);
            }
        }
        *run_sends = slot.run_sends;
    }
}

/// Frame-loop cadence: a slot that draws or captures input keeps the fixed
/// 20 ms loop — that loop *is* the render cadence (50 fps) and the input
/// drain cadence. Everything else (draw off, no capture) may park on
/// socket-read unless the observe hook reported script/cheat/nav work.
fn frame_cadence(client: &Client, input: Option<&SlotInput>) -> bool {
    client.draw || input.map(|i| i.enabled()).unwrap_or(false)
}

/// Payload bytes the client's stream has consumed; 0 when no stream.
fn stream_bytes(client: &Client) -> u64 {
    client
        .stream
        .as_ref()
        .map(|s| s.bytes_in())
        .unwrap_or(0)
}

/// Why a park returned.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ParkWake {
    /// The client socket became readable (data, EOF, or a socket error).
    Socket,
    /// A control kick landed (focus/draw/stop/spawn).
    Control,
    /// The park timeout (game-tick bound, or the stall bound) elapsed.
    Timeout,
}

/// Block until the client socket is readable, a control wake lands, or
/// `timeout` elapses. The socket is polled only when `poll_socket` — a
/// previous no-consumption wake (EOF or a packet still mid-flight) leaves
/// the socket permanently readable, so re-polling it would busy-spin.
/// No socket and no control channel falls back to sleeping the timeout.
#[allow(unsafe_code)]
fn park(client: &Client, ctl: Option<&SlotPark>, poll_socket: bool, timeout: Duration) -> ParkWake {
    let mut fds = [libc::pollfd {
        fd: -1,
        events: 0,
        revents: 0,
    }; 2];
    let mut n = 0usize;
    if let Some(ctl) = ctl {
        fds[n] = libc::pollfd {
            fd: ctl.fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        n += 1;
    }
    let socket = if poll_socket {
        client.stream.as_ref().map(|stream| {
            fds[n] = libc::pollfd {
                fd: stream.fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let idx = n;
            n += 1;
            idx
        })
    } else {
        None
    };
    if n == 0 {
        thread::sleep(timeout);
        return ParkWake::Timeout;
    }
    let ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    let rc = unsafe { libc::poll(fds.as_mut_ptr(), n as libc::nfds_t, ms) };
    if rc > 0 {
        let fired = |i: usize| {
            fds[i].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0
        };
        let control_fired = ctl.is_some() && fired(0);
        if control_fired {
            // Consume the kick bytes: an undrained control fd stays
            // readable and would re-fire every park (busy loop).
            ctl.unwrap().drain();
        }
        if let Some(i) = socket {
            if fired(i) {
                return ParkWake::Socket;
            }
        }
        if control_fired {
            return ParkWake::Control;
        }
    }
    ParkWake::Timeout
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
    (*n).is_multiple_of(WATCH_RASTER_TICKS)
}

/// Per-slot post-drain state: snapshot, settle, auto-run, the full-rate
/// switch, and the optional `Renderer` a drawing slot owns.
struct SlotLoop {
    pump: Pump,
    snapshot: GameSnapshot,
    settle: Settle,
    run_on: bool,
    run_sends: u32,
    last_modal: Option<i32>,
    /// TV full-rate latch: focused+input-on slots paint every tick even
    /// after the scene is ready.
    /// TODO(Task 2): wire this from the panel/TV control path.
    full_rate: bool,
    renderer: Option<Renderer>,
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
            full_rate: false,
            renderer: None,
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
    use std::time::Instant;

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
        tx.send(InputEv::Down {
            button: 1,
            x: 20,
            y: 20,
        })
        .unwrap();
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
    fn client_frame_skips_frame_store_when_draw_off() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let buf = FrameBuf::new();
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        // Renderer off: no frame is rendered and nothing is stored.
        c.set_draw(false);
        Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends);
        assert!(buf.snapshot().is_empty(), "draw off must not store pixels");
        // Renderer on: the first (rising-edge) tick paints a full applet
        // into the buffer (with no title assets in this test the paint is
        // empty, but the frame still packs a full applet; the non-zero
        // paint is proven live by the panel_view e2e).
        c.set_draw(true);
        Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends);
        assert_eq!(
            buf.snapshot().len(),
            (client::client::APPLET_W * client::client::APPLET_H) as usize
        );
    }

    #[test]
    fn headless_slot_constructs_no_renderer_and_never_draws() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        // `client.draw` defaults false: a headless slot never paints. The
        // check is slot-local (not the global `Renderer::constructed()`
        // counter, which other tests' renderers bump concurrently).
        for _ in 0..3 {
            Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends);
        }
        assert!(
            slot.renderer.is_none(),
            "draw off must not construct a Renderer"
        );
        assert_eq!(slot.skip_n, 3);
        assert_eq!(slot.paint_n, 0);
        assert!(slot.loop_ns > 0, "mainloop still ran");
    }

    #[test]
    fn client_frame_keeps_loop_counters_slot_local() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        c.set_draw(false);
        Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends);
        assert_eq!(slot.skip_n, 1);
        assert_eq!(slot.paint_n, 0);
        assert!(slot.loop_ns > 0);
    }

    #[test]
    fn client_frame_draw_on_paints_this_tick() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        c.set_draw(true);
        Host::client_frame(&mut c, &mut slot, "t", None, None, &mut sends);
        assert_eq!(slot.paint_n, 1);
        assert_eq!(slot.skip_n, 0);
    }

    #[test]
    fn mainredraw_runs_check_minimap_on_a_paint_tick() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let buf = FrameBuf::new();
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        c.set_draw(true);
        c.ingame = true;
        c.scene_state = 2;
        // A fresh client starts with `minimap_level = -1` (reset by login)
        // while `minusedlevel` is 0. Only `Renderer::mainredraw`'s
        // `check_minimap` render half brings `minimap_level` up to
        // `minusedlevel`; the raw `game_draw` stage does not, so this
        // pins the render dispatch to `mainredraw`.
        assert_eq!(c.minimap_level, -1);
        assert_ne!(c.minimap_level, c.minusedlevel);
        Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends);
        assert_eq!(
            c.minimap_level,
            c.minusedlevel,
            "mainredraw must run the check_minimap render half"
        );
        assert_eq!(slot.paint_n, 1);
        assert_eq!(
            buf.snapshot().len(),
            (client::client::APPLET_W * client::client::APPLET_H) as usize
        );
    }

    #[test]
    fn client_tick_observe_runs_before_the_frame_paint() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let buf = FrameBuf::new();
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        assert!(!c.draw, "slots start with the renderer off");
        // A drawing slot paints this tick; an observe-after-frame would
        // skip it, so observe-before must run first.
        c.set_draw(true);
        let observed = AtomicBool::new(false);
        Host::client_tick(
            &mut c,
            &mut slot,
            "t",
            None,
            Some(&buf),
            &mut sends,
            &mut |_, _, _| {
                observed.store(true, Ordering::Relaxed);
                false
            },
        );
        assert!(observed.load(Ordering::Relaxed));
        assert_eq!(
            buf.snapshot().len(),
            (client::client::APPLET_W * client::client::APPLET_H) as usize
        );
        let gen = buf.generation();
        c.set_draw(false);
        Host::client_tick(
            &mut c,
            &mut slot,
            "t",
            None,
            Some(&buf),
            &mut sends,
            &mut |_, _, _| false,
        );
        assert!(!c.draw);
        assert_eq!(
            buf.generation(),
            gen,
            "renderer off must not store a frame this tick"
        );
    }

    #[test]
    fn raster_this_tick_watch_is_one_fps_capture_is_every_tick() {
        let mut n = 0;
        let mut on = false;
        assert!(!raster_this_tick(false, false, &mut n, &mut on));
        assert!(
            raster_this_tick(true, false, &mut n, &mut on),
            "rising edge paints now"
        );
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
        let buf = FrameBuf::new();
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        c.set_draw(true);
        c.ingame = true;
        c.scene_state = 2;
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
    fn full_rate_paints_every_tick_after_scene_ready() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let buf = FrameBuf::new();
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        c.set_draw(true);
        slot.full_rate = true;
        c.ingame = true;
        c.scene_state = 2;
        Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends);
        assert_eq!(buf.generation(), 1);
        Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends);
        assert_eq!(
            buf.generation(),
            2,
            "TV full_rate must redraw 2D+3D every 20 ms, not 1 fps watch"
        );
    }

    #[test]
    fn loading_scene_paints_every_tick_for_tv_static() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let buf = FrameBuf::new();
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        c.set_draw(true);
        c.ingame = true;
        c.scene_state = 1;
        Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends);
        assert_eq!(buf.generation(), 1);
        Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends);
        assert_eq!(
            buf.generation(),
            2,
            "scene_state != 2 must re-roll static every 20 ms"
        );
    }

    #[test]
    fn capture_draw_copies_every_tick() {
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let buf = FrameBuf::new();
        let inp = SlotInput::new();
        inp.set_enabled(true);
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        c.set_draw(true);
        Host::client_frame(&mut c, &mut slot, "t", Some(&inp), Some(&buf), &mut sends);
        Host::client_frame(&mut c, &mut slot, "t", Some(&inp), Some(&buf), &mut sends);
        assert_eq!(buf.generation(), 2);
    }

    /// Observe mirror: the shared handle the slot thread's observe hook
    /// updates, so a `run_client` thread is observable without owning the
    /// client. `(loop_cycle, reboot_timer)` for packet tests.
    type Seen = std::sync::Arc<std::sync::Mutex<(i32, i32)>>;

    fn seen() -> (Seen, Seen) {
        let s = std::sync::Arc::new(std::sync::Mutex::new((0, 0)));
        (std::sync::Arc::clone(&s), s)
    }

    // `Client` is !Send (its `present` target is a `Box<dyn PresentTarget>`),
    // so like host-play's slot threads the tests build the client *inside*
    // the spawned closure and only move Send handles across the boundary.

    #[test]
    fn idle_slot_parks_between_packets_and_wakes_on_one() {
        use std::io::Write;
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (wake, park) = crate::slot_io::wake_channel();
        let (mirror, seen) = seen();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let handle = thread::spawn(move || {
            let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
            let stream =
                client::io::ClientStream::connect(&addr.ip().to_string(), addr.port()).unwrap();
            c.stream = Some(stream);
            c.ingame = true;
            // The login handshake leaves the packet decoder mid-frame
            // (`ptype == -1` → read the next header byte); without it a
            // fresh client misreads the first socket byte as a header.
            c.ptype = -1;
            Host::run_client(
                &mut c,
                "idle",
                None,
                None,
                Some(Arc::new(park)),
                |c, _, _| {
                    let mut v = mirror.lock().unwrap();
                    v.0 = c.loop_cycle;
                    v.1 = c.reboot_timer;
                    false
                },
                |_| stop2.load(Ordering::Relaxed),
            );
            done_tx.send(()).unwrap();
        });
        let (mut server, _) = listener.accept().unwrap();

        // First park happens after the opening tick; a quiet window must
        // not advance mainloop at all (no packet, no control, no timer yet).
        thread::sleep(Duration::from_millis(120));
        let before = *seen.lock().unwrap();
        thread::sleep(Duration::from_millis(120));
        let after = *seen.lock().unwrap();
        assert_eq!(
            after, before,
            "idle slot must not call mainloop between packet arrivals"
        );

        // A server packet (UPDATE_REBOOT_TIMER, two payload bytes) must
        // wake the park and apply within a frame.
        server.write_all(&[89, 0, 10]).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let (_, reboot) = *seen.lock().unwrap();
            if reboot > 0 {
                break;
            }
            assert!(Instant::now() < deadline, "packet never applied");
            thread::sleep(Duration::from_millis(5));
        }

        stop.store(true, Ordering::Relaxed);
        wake.wake();
        done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("stop control must return a parked slot");
        handle.join().unwrap();
    }

    #[test]
    fn focused_slot_keeps_the_twenty_ms_cadence() {
        let (mirror, seen) = seen();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
            c.set_draw(true);
            Host::run_client(
                &mut c,
                "focused",
                None,
                None,
                None,
                |c, _, _| {
                    mirror.lock().unwrap().0 = c.loop_cycle;
                    false
                },
                |_| stop2.load(Ordering::Relaxed),
            );
        });
        // Gate on the count, not a fixed sleep: a parked slot (600 ms
        // timeout, no socket/control) manages ≤8 ticks in 5 s, so reaching
        // 12 proves the frame loop is running no matter how contended the
        // test machine is.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let cycles = seen.lock().unwrap().0;
            if cycles >= 12 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "focused slot never entered the frame loop, {cycles} ticks in 5 s"
            );
            thread::sleep(Duration::from_millis(10));
        }
        // Then the rate: the 20 ms cadence yields dozens per 300 ms; a
        // parked slot would yield ≤1.
        let t1 = seen.lock().unwrap().0;
        thread::sleep(Duration::from_millis(300));
        let t2 = seen.lock().unwrap().0;
        assert!(
            t2 >= t1 + 3,
            "focused slot must tick at ~20 ms, {t1} -> {t2} over 300 ms"
        );
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
    }

    #[test]
    fn busy_observe_keeps_the_slot_on_the_frame_loop() {
        let (mirror, seen) = seen();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
            Host::run_client(
                &mut c,
                "scripted",
                None,
                None,
                None,
                |c, _, _| {
                    mirror.lock().unwrap().0 = c.loop_cycle;
                    true // script/cheat/nav work due: never park
                },
                |_| stop2.load(Ordering::Relaxed),
            );
        });
        // Same gate as the focused test: ≥12 ticks in 5 s is unreachable
        // for a parked slot (≤8), so this proves a busy slot never parks.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let cycles = seen.lock().unwrap().0;
            if cycles >= 12 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "a busy (scripted) slot must keep ticking, {cycles} ticks in 5 s"
            );
            thread::sleep(Duration::from_millis(10));
        }
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
    }

    #[test]
    fn stop_control_wakes_a_parked_slot_and_returns() {
        let (wake, park) = crate::slot_io::wake_channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let handle = thread::spawn(move || {
            let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
            Host::run_client(
                &mut c,
                "parked",
                None,
                None,
                Some(Arc::new(park)),
                |_, _, _| false,
                |_| stop2.load(Ordering::Relaxed),
            );
            done_tx.send(()).unwrap();
        });
        // Let the slot park, then stop + kick must return it promptly.
        thread::sleep(Duration::from_millis(100));
        stop.store(true, Ordering::Relaxed);
        wake.wake();
        done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("a stop control must wake a parked slot");
        handle.join().unwrap();
    }

    #[test]
    fn draw_kick_wakes_a_parked_slot_into_the_frame_loop() {
        let (wake, park) = crate::slot_io::wake_channel();
        let (mirror, seen) = seen();
        let want_draw = Arc::new(AtomicBool::new(false));
        let want_draw2 = Arc::clone(&want_draw);
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
            Host::run_client(
                &mut c,
                "kicked",
                None,
                None,
                Some(Arc::new(park)),
                // The panel mirrors focus into the slot thread via the
                // observe hook (per_frame → set_draw), reading the shared
                // intent like `run_with_io`'s per_frame reads the Focus.
                |c, _, _| {
                    c.set_draw(want_draw2.load(Ordering::Relaxed));
                    mirror.lock().unwrap().0 = c.loop_cycle;
                    false
                },
                |_| stop2.load(Ordering::Relaxed),
            );
        });
        // Opening tick, then parked (draw off): a quiet window must not
        // advance the tick count.
        thread::sleep(Duration::from_millis(80));
        let before = seen.lock().unwrap().0;
        assert!(before <= 1, "draw-off slot must park, got {before} ticks");
        // The panel flips its draw intent and kicks the parked thread.
        want_draw.store(true, Ordering::Relaxed);
        wake.wake();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let t = seen.lock().unwrap().0;
            if t >= before + 8 {
                break; // the ~20 ms cadence resumed after the kick
            }
            assert!(
                Instant::now() < deadline,
                "kicked slot never resumed the frame loop"
            );
            thread::sleep(Duration::from_millis(5));
        }
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
    }

    #[test]
    fn spurious_kick_does_not_busy_loop_a_parked_slot() {
        let (wake, park) = crate::slot_io::wake_channel();
        let (mirror, seen) = seen();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let handle = thread::spawn(move || {
            let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
            Host::run_client(
                &mut c,
                "kicked-idle",
                None,
                None,
                Some(Arc::new(park)),
                |c, _, _| {
                    mirror.lock().unwrap().0 = c.loop_cycle;
                    false // stays idle after the kick
                },
                |_| stop2.load(Ordering::Relaxed),
            );
            done_tx.send(()).unwrap();
        });
        thread::sleep(Duration::from_millis(80));
        let before = seen.lock().unwrap().0;
        // Two kicks while the slot stays idle: each must wake it once and
        // re-park — an undrained control fd would re-fire every park and
        // spin the thread.
        wake.wake();
        wake.wake();
        thread::sleep(Duration::from_millis(120));
        let after = seen.lock().unwrap().0;
        assert!(
            after <= before + 2,
            "a spurious kick must not busy-loop a parked slot, ticks {before} -> {after}"
        );
        stop.store(true, Ordering::Relaxed);
        wake.wake();
        done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("stop must still return the slot");
        handle.join().unwrap();
    }
}
