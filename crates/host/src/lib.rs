//! 274 bot host: one OS thread per client slot.

mod auto_run;
pub mod login_queue;
mod slot;
mod slot_io;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use api::interact::set_run;
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
/// wall-clock game-loop watchdog still notices a dead server promptly, but
/// long enough that a permanently-readable socket cannot busy-spin the slot.
const STALL_PARK_MS: Duration = Duration::from_millis(200);

/// Park bound for a watch-only 1 fps sidecar: the 1 s wall-clock repaint
/// cadence. The slot wakes once a second to drain the socket and decide
/// the paint; a packet arriving mid-park still wakes the poll immediately,
/// so nothing is ever dropped.
const WATCH_PARK_MS: Duration = Duration::from_secs(1);

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
    /// drain); think (auto-run) reads
    /// energy from the snapshot stat view when it has been rebuilt. The
    /// third observe arg is the count of accepted auto-run `set_run(true)`
    /// sends (from the previous tick); observe's return is whether the slot
    /// has script/cheat/nav work, which keeps a busy slot on the frame loop.
    ///
    /// The scheduler is event-driven: a slot that captures input, is still
    /// loading (TV static), or runs full-rate TV keeps the fixed 20 ms
    /// [`FRAME_MS`] loop — that loop *is* the render cadence. A watch-only
    /// 1 fps sidecar (draw on, nothing on the 20 ms loop) parks on the 1 s
    /// wall-clock repaint bound; everything else parks on `poll(2)` over
    /// the client socket's readability, the `ctl` control wake
    /// (focus/draw/stop/spawn), and the game-tick timeout ([`IDLE_PARK_MS`]),
    /// waking once per park to drain the socket and re-evaluate. Packets
    /// are never dropped: a readable socket wakes the park, and `mainloop`
    /// is the first thing that drains it. A wake that consumed no bytes
    /// (EOF, partial packet) skips the socket on the next park so it cannot
    /// busy-spin.
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
            // Idle: park until a packet, a control kick, or the bound, then
            // run one tick (drain the socket / apply the panel's
            // `set_draw`) and re-evaluate. A watch-only sidecar wakes on
            // the 1 s repaint bound; everything else on the game-tick bound.
            let before = stream_bytes(client);
            let timeout = if socket_stalled {
                STALL_PARK_MS
            } else if watch_only(client, input.as_deref()) {
                WATCH_PARK_MS
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
    /// mirroring `Client::run`), then drain gens. The panel takes the
    /// mailbox: `FrameBuf::take` hands the whole `FrameOutput` off (the
    /// `Texture` binds / reads back at the panel, the `PixMap` packs via
    /// [`FrameBuf::snapshot`]). `run_sends` is
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
        // The 50 fps cadence latch: `client.draw` is the renderer switch
        // the panel latches via `Client::set_draw`; `full_rate` is the
        // per-slot knob the panel's sidecar-50 pref drives through the
        // shared `SlotInput`.
        let full_rate = input.map(|i| i.full_rate()).unwrap_or(false);
        let paint = raster_this_tick(
            client.draw,
            capture || zap || full_rate,
            t_loop,
            &mut slot.raster_last,
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
                .get_or_insert_with(|| {
                    // GPU-first (the host default): the slot's renderer
                    // prefers the wgpu backend, with `CpuBackend` as the
                    // fallback on wgpu init failure (`Renderer::new`
                    // selects, `Renderer::backend_kind` reports). The
                    // preference is process-wide and idempotent, so the
                    // first paint of any slot opts the process in;
                    // `BOT_CPU=1` forces the CPU fidelity path.
                    let cpu = std::env::var("BOT_CPU").map(|v| v == "1").unwrap_or(false);
                    Renderer::set_prefer_gpu(!cpu);
                    Renderer::new(client.config.lowmem)
                });
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
        let result = slot.after_drain(client);
        if should_emit_tick(result.player_info) {
            slot.tick_n = slot.tick_n.wrapping_add(1);
        }
        if debug_enabled() && slot.log_n.is_multiple_of(50) {
            eprintln!(
                "[host] slot {username}: loop_us={} raster_us={} paints={} skips={} ticks={}",
                slot.loop_ns / 1000,
                slot.raster_ns / 1000,
                slot.paint_n,
                slot.skip_n,
                slot.tick_n
            );
        }
        // The whole `FrameOutput` lands in the mailbox, not just the
        // packed pixels: the panel takes the `FrameOutput::Texture` (GPU
        // backend) or packs the `PixMap` (CPU backend) at its consume
        // site, and `FrameBuf::snapshot` keeps the CPU packing path for
        // the tests.
        if let Some(frame) = frame {
            if let Some(mailbox) = mailbox {
                mailbox.store(frame);
            }
        }
        *run_sends = slot.run_sends;
    }
}

/// Frame-loop cadence: a slot that captures input, is still loading (TV
/// static re-rolls every 20 ms), or runs full-rate (the panel's sidecar-50
/// pref, or the TV full-rate latch) keeps the fixed 20 ms loop — that loop
/// *is* the render cadence (50 fps) and the input drain cadence. Draw-on
/// but not capture/full-rate is **watch-only 1 fps**: the picture only
/// refreshes once a second, so the slot parks on the 1 s wall-clock bound
/// instead of holding the 20 ms sim loop for a 1 fps sidecar. `busy`
/// (script/cheat/nav work from the observe hook) also keeps a slot on the
/// frame loop.
fn frame_cadence(client: &Client, input: Option<&SlotInput>) -> bool {
    input.map(|i| i.enabled()).unwrap_or(false)
        || (client.draw
            && (input.map(|i| i.full_rate()).unwrap_or(false)
                || (client.ingame && client.scene_state != 2)))
}

/// A watch-only 1 fps sidecar: the renderer is on but nothing needs the
/// 20 ms loop (no capture, no full-rate, not still loading) — it parks on
/// the 1 s wall-clock repaint bound.
fn watch_only(client: &Client, input: Option<&SlotInput>) -> bool {
    client.draw && !frame_cadence(client, input)
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

/// Watch-only repaint bound: the rail/sidecar picture refreshes once a
/// wall-clock second. Elapsed time, not a tick count — the slot is parked
/// on [`WATCH_PARK_MS`], so ticks (one per wake) are not a clock.
const WATCH_PAINT_MS: Duration = Duration::from_secs(1);

/// rs2b0t rail: watch-only paints on a 1 s **wall-clock** cadence — the
/// slot is parked, so ticks are not a clock; elapsed time since the last
/// paint decides, not a tick count. The first tick after draw rises paints
/// immediately so checking the box is not a cold hitch. Capture (input /
/// TV static / full-rate) paints every tick. Draw off never paints.
fn raster_this_tick(
    draw: bool,
    capture: bool,
    now: Instant,
    last_paint: &mut Option<Instant>,
    was_on: &mut bool,
) -> bool {
    if !draw {
        *was_on = false;
        return false;
    }
    if capture {
        *last_paint = Some(now);
        *was_on = true;
        return true;
    }
    let rising = !*was_on;
    *was_on = true;
    let due = match *last_paint {
        Some(last) => rising || now.duration_since(last) >= WATCH_PAINT_MS,
        None => true, // first watch paint: nothing painted yet
    };
    if due {
        *last_paint = Some(now);
    }
    due
}

/// Per-slot post-drain state: snapshot, auto-run, the full-rate
/// switch, and the optional `Renderer` a drawing slot owns.
struct SlotLoop {
    pump: Pump,
    snapshot: GameSnapshot,
    run_on: bool,
    run_sends: u32,
    renderer: Option<Renderer>,
    /// `Instant` of the last paint of any kind; the watch-only 1 fps
    /// decision repaints when this is ≥1 s old.
    raster_last: Option<Instant>,
    raster_was_on: bool,
    loop_ns: u64,
    raster_ns: u64,
    paint_n: u64,
    skip_n: u64,
    /// Game-tick edges (`PLAYER_INFO` this drain). Counted internally;
    /// printed in the 50-frame summary, not per tick.
    tick_n: u64,
    log_n: u32,
}

impl SlotLoop {
    fn new() -> Self {
        Self {
            pump: Pump::new(),
            snapshot: GameSnapshot::new(),
            run_on: false,
            run_sends: 0,
            renderer: None,
            raster_last: None,
            raster_was_on: false,
            loop_ns: 0,
            raster_ns: 0,
            paint_n: 0,
            skip_n: 0,
            tick_n: 0,
            log_n: 0,
        }
    }

    fn after_drain(&mut self, client: &mut Client) -> DrainResult {
        let result = self.pump.drain(client.gens);
        rebuild_dirty(&mut self.snapshot, client, result.dirty);

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
        // Loc and ground-item changes bump the scene gen, so the same
        // drain flag rebuilds their views.
        snapshot.rebuild_family(client, Family::Loc);
        snapshot.rebuild_family(client, Family::GroundItem);
    }
    if dirty.iface {
        snapshot.rebuild_family(client, Family::Iface);
    }
    if dirty.camera {
        snapshot.rebuild_family(client, Family::Camera);
    }
    if dirty.map_flag {
        snapshot.rebuild_family(client, Family::MapFlag);
    }
    if dirty.world {
        snapshot.rebuild_family(client, Family::World);
    }
    // The iface-derived families re-read the materialized `client.ifaces`
    // (and the inv slot data), so their gens are the iface and inv flags;
    // each family's own gate no-ops the ones that did not move.
    if dirty.iface || dirty.inv {
        snapshot.rebuild_family(client, Family::Inventory);
        snapshot.rebuild_family(client, Family::Equipment);
        snapshot.rebuild_family(client, Family::Bank);
        snapshot.rebuild_family(client, Family::BankSide);
        snapshot.rebuild_family(client, Family::Trade);
        snapshot.rebuild_family(client, Family::Widgets);
        snapshot.rebuild_family(client, Family::SideTabs);
        snapshot.rebuild_family(client, Family::ChatOptions);
        snapshot.rebuild_family(client, Family::MakeProducts);
        snapshot.rebuild_family(client, Family::QuestStatuses);
        snapshot.rebuild_family(client, Family::Modals);
        snapshot.rebuild_family(client, Family::Controls);
        snapshot.rebuild_family(client, Family::Menu);
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
    use std::sync::OnceLock;
    use std::time::Instant;

    /// Force the wgpu backend's process-wide init to fail so every
    /// renderer this test process constructs lands on `CpuBackend` (the
    /// client's documented `R274_TEST_FORCE_NO_GPU` test hook): the host's
    /// GPU-first default must not open a real device inside
    /// `cargo test -p host`. Once set, the client caches the failure, so
    /// the first renderer construction wins — call before any test
    /// constructs one.
    static FORCE_CPU_BACKEND: OnceLock<()> = OnceLock::new();
    fn force_cpu_backend() {
        FORCE_CPU_BACKEND.get_or_init(|| {
            std::env::set_var("R274_TEST_FORCE_NO_GPU", "1");
            Renderer::set_prefer_gpu(true);
        });
    }

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

    /// The drain's dirty flags must feed `rebuild_dirty` for the four new
    /// families too, or `snapshot.gens().{iface,camera,map_flag,world}`
    /// stay permanently 0 (the API-v2 views will rely on this path).
    #[test]
    fn drain_rebuilds_the_four_new_families() {
        let mut client = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let mut slot = SlotLoop::new();

        client.gens.iface = 1;
        let result = slot.after_drain(&mut client);
        assert!(result.dirty.iface);
        assert_eq!(
            slot.snapshot.gens().iface,
            1,
            "drain must rebuild the iface family"
        );

        client.gens.camera = 1;
        let result = slot.after_drain(&mut client);
        assert!(result.dirty.camera);
        assert_eq!(
            slot.snapshot.gens().camera,
            1,
            "drain must rebuild the camera family"
        );

        client.gens.map_flag = 1;
        let result = slot.after_drain(&mut client);
        assert!(result.dirty.map_flag);
        assert_eq!(
            slot.snapshot.gens().map_flag,
            1,
            "drain must rebuild the map_flag family"
        );

        client.gens.world = 1;
        let result = slot.after_drain(&mut client);
        assert!(result.dirty.world);
        assert_eq!(
            slot.snapshot.gens().world,
            1,
            "drain must rebuild the world family"
        );
    }

    /// The drain's iface/inv flags must rebuild the iface-derived v2
    /// families too, or the host path (as opposed to the scenario
    /// runner's `GameSnapshot::rebuild`) would keep their views
    /// permanently empty.
    #[test]
    fn drain_rebuilds_the_iface_derived_families() {
        use client::config::if_type::ComponentType;

        let mut ifaces = vec![None; 1000];
        ifaces[500] = Some(IfType {
            id: 500,
            r#type: ComponentType::TYPE_INV,
            link_obj_type: Some(vec![4, 5, 0]),
            link_obj_number: Some(vec![1, 100, 0]),
            obj_ops: true,
            ..IfType::default()
        });
        let mut client = prepare_client(cfg(), 1, Arc::new(Cache::default()), ifaces);
        client.side_icon[3] = 500;
        let mut slot = SlotLoop::new();

        client.gens.iface = 1;
        client.gens.inv = 1;
        let result = slot.after_drain(&mut client);
        assert!(result.dirty.iface);
        assert!(result.dirty.inv);
        assert_eq!(
            slot.snapshot.inventory().len(),
            2,
            "an iface/inv drain must rebuild the inventory family"
        );
        assert_eq!(slot.snapshot.inventory_size(), 3);

        // A quiet drain leaves the v2 gates alone: unchanged gens do not
        // re-mark anything dirty.
        let result = slot.after_drain(&mut client);
        assert!(!result.dirty.any());
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
        force_cpu_backend();
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
        force_cpu_backend();
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
        force_cpu_backend();
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
        force_cpu_backend();
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
    fn raster_this_tick_watch_is_wall_clock_one_fps_capture_is_every_tick() {
        let t0 = Instant::now();
        let mut last = None;
        let mut on = false;
        assert!(!raster_this_tick(false, false, t0, &mut last, &mut on));
        assert!(
            raster_this_tick(true, false, t0, &mut last, &mut on),
            "rising edge paints now"
        );
        // Sub-second wakes (the parked slot's cadence) stay quiet…
        assert!(!raster_this_tick(
            true,
            false,
            t0 + Duration::from_millis(500),
            &mut last,
            &mut on
        ));
        // …and the paint lands once a wall-clock second elapses.
        assert!(raster_this_tick(
            true,
            false,
            t0 + Duration::from_secs(1),
            &mut last,
            &mut on
        ));
        assert!(!raster_this_tick(
            true,
            false,
            t0 + Duration::from_secs(1) + Duration::from_millis(100),
            &mut last,
            &mut on
        ));
        // Capture paints every tick (minimenu / TV static / full-rate).
        assert!(raster_this_tick(
            true,
            true,
            t0 + Duration::from_secs(1),
            &mut last,
            &mut on
        ));
        assert!(raster_this_tick(
            true,
            true,
            t0 + Duration::from_secs(1),
            &mut last,
            &mut on
        ));
        on = false;
        assert!(
            raster_this_tick(true, false, t0 + Duration::from_secs(1), &mut last, &mut on),
            "draw rising after off paints immediately"
        );
    }

    #[test]
    fn watch_only_paints_first_tick_then_once_per_second() {
        force_cpu_backend();
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let buf = FrameBuf::new();
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        c.set_draw(true);
        c.ingame = true;
        c.scene_state = 2;
        Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends);
        assert_eq!(buf.generation(), 1);
        // A fast second tick (the parked slot draining a burst) must not
        // repaint before a wall-clock second elapses.
        Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends);
        assert_eq!(buf.generation(), 1);
        thread::sleep(Duration::from_secs(1) + Duration::from_millis(50));
        Host::client_frame(&mut c, &mut slot, "t", None, Some(&buf), &mut sends);
        assert_eq!(buf.generation(), 2, "watch-only repaints after 1 s");
    }

    #[test]
    fn full_rate_paints_every_tick_after_scene_ready() {
        force_cpu_backend();
        let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
        let buf = FrameBuf::new();
        let mut slot = SlotLoop::new();
        let mut sends = 0u32;
        c.set_draw(true);
        // The sidecar-50 pref drives the frame cadence through the shared
        // SlotInput, not a slot-local field.
        let inp = SlotInput::new();
        inp.set_full_rate(true);
        c.ingame = true;
        c.scene_state = 2;
        Host::client_frame(&mut c, &mut slot, "t", Some(&inp), Some(&buf), &mut sends);
        assert_eq!(buf.generation(), 1);
        Host::client_frame(&mut c, &mut slot, "t", Some(&inp), Some(&buf), &mut sends);
        assert_eq!(
            buf.generation(),
            2,
            "TV full_rate must redraw 2D+3D every 20 ms, not 1 fps watch"
        );
        // Clearing the latch drops back to the 1 fps watch cadence.
        inp.set_full_rate(false);
        Host::client_frame(&mut c, &mut slot, "t", Some(&inp), Some(&buf), &mut sends);
        assert_eq!(
            buf.generation(),
            2,
            "full_rate off must not paint sub-second"
        );
    }

    #[test]
    fn loading_scene_paints_every_tick_for_tv_static() {
        force_cpu_backend();
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
        force_cpu_backend();
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
        force_cpu_backend();
        let (mirror, seen) = seen();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let inp = SlotInput::new();
        inp.set_enabled(true);
        let handle = thread::spawn(move || {
            let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
            c.set_draw(true);
            Host::run_client(
                &mut c,
                "focused",
                Some(inp),
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
    fn watch_only_sidecar_parks_wakes_once_per_second_and_paints() {
        force_cpu_backend();
        let (wake, park) = crate::slot_io::wake_channel();
        let (mirror, seen) = seen();
        let buf = FrameBuf::new();
        let buf2 = Arc::clone(&buf);
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
            c.set_draw(true);
            c.ingame = true;
            c.scene_state = 2;
            Host::run_client(
                &mut c,
                "sidecar",
                None,
                Some(buf2),
                Some(Arc::new(park)),
                |c, _, _| {
                    mirror.lock().unwrap().0 = c.loop_cycle;
                    false
                },
                |_| stop2.load(Ordering::Relaxed),
            );
        });
        // The rising edge paints the first frame, then the slot parks.
        // Poll for the first paint — no fixed sleep, because under parallel
        // `--workspace` load the slot thread can be delayed arbitrarily.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if buf.generation() >= 1 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "watch-only sidecar never painted its first frame"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let g0 = buf.generation();
        assert!(g0 >= 1, "watch-only rising edge must paint the first frame");
        // The second paint is gated on a wall-clock second since the first
        // (the elapsed-time paint decision), so it must still arrive on the
        // 1 s park — poll for it with a generous deadline.
        let second = Instant::now();
        let deadline = second + Duration::from_secs(10);
        loop {
            if buf.generation() >= g0 + 2 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "watch-only sidecar never repainted (gen stuck at {})",
                buf.generation()
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            second.elapsed() >= Duration::from_millis(900),
            "watch-only repaint must wait a wall-clock second, not every tick"
        );
        // Parking proof: a 20 ms loop would tick ~100× in 2.2 s; the 1 s
        // park stays near 2–3 wakes.
        let t1 = seen.lock().unwrap().0;
        thread::sleep(Duration::from_secs(2) + Duration::from_millis(200));
        let t2 = seen.lock().unwrap().0;
        let ticks = t2 - t1;
        assert!(
            (1..=5).contains(&ticks),
            "watch-only sidecar must wake ~1×/s, not 50×/s: ticks {t1} -> {t2} ({ticks} in 2.2 s)"
        );
        stop.store(true, Ordering::Relaxed);
        wake.wake();
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
        force_cpu_backend();
        let (wake, park) = crate::slot_io::wake_channel();
        let (mirror, seen) = seen();
        let inp = SlotInput::new();
        // The panel mirrors focus into the slot thread via the observe
        // hook (per_frame → `set_draw`, capture → `input.set_enabled`).
        // The kick flips the slot from draw-off to focused+capture, which
        // must wake the park into the 20 ms frame loop.
        let want = Arc::new(AtomicBool::new(false));
        let want2 = Arc::clone(&want);
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            let mut c = prepare_client(cfg(), 1, Arc::new(Cache::default()), vec![]);
            Host::run_client(
                &mut c,
                "kicked",
                Some(Arc::clone(&inp)),
                None,
                Some(Arc::new(park)),
                |c, _, _| {
                    let on = want2.load(Ordering::Relaxed);
                    c.set_draw(on);
                    inp.set_enabled(on);
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
        want.store(true, Ordering::Relaxed);
        wake.wake();
        // Gate on the tick count, not a fixed deadline: under parallel load
        // the kicked slot can take a while to run 8 ticks, so give it a
        // generous window (a still-parked slot would be far slower).
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let t = seen.lock().unwrap().0;
            if t >= before + 8 {
                break; // the kicked slot is ticking again
            }
            assert!(
                Instant::now() < deadline,
                "kicked slot never resumed ticking, {t} ticks in 10 s"
            );
            thread::sleep(Duration::from_millis(10));
        }
        // Then the rate: ≥3 ticks per 300 ms is the ~20 ms cadence (a
        // parked slot yields ≤1), so the kick really re-entered the frame
        // loop instead of just ticking slowly.
        let t1 = seen.lock().unwrap().0;
        thread::sleep(Duration::from_millis(300));
        let t2 = seen.lock().unwrap().0;
        assert!(
            t2 >= t1 + 3,
            "kicked slot must tick at ~20 ms, {t1} -> {t2} over 300 ms"
        );
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
