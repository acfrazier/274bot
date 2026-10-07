//! 274 bot host: one OS thread per client slot.

mod auto_run;
pub mod login_queue;
#[cfg(feature = "performance-profile")]
pub mod performance_profile;
mod random;
mod slot;
mod slot_io;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use api::host_log;
use api::hostlog::{Category, Level};
use api::interact::set_run;
use api::run_policy::RunPolicyOverride;
use api::snapshot::GameSnapshot;
use auto_run::{auto_run_ready, auto_run_tick, resolve_run_policy, RunPolicy};
use client::client::{Client, ClientConfig};
use client::config::{Cache, IfType, IfTypeMut};
use client::render::backend::{BackendKind, FrameOutput};
use client::render::Renderer;
use vault::{Profile, ProfileSettings};

pub use random::{
    detect, CooldownMap, DetectedRandom, Guardian, RandomClaim, RandomKind, RandomStatus,
};
pub use slot::{dirty_families, should_emit_tick, DirtyFamilies, DrainResult, Pump};
pub use slot_io::{
    map_image_to_applet, wake_channel, FrameBuf, InputEv, ManualMoveIntent, MouseOwner, SlotInput,
    SlotPark, SlotWake,
};
#[cfg(feature = "journal-paint-proof")]
pub use slot_io::{JournalPaintStamp, JournalProofFrame};

/// Enable host debug logging (host-play maps `--debug` to this).
pub fn set_debug(enabled: bool) {
    api::hostlog::set_debug(enabled);
}

/// Host debug logging is on when `BOT_DEBUG=1` or [`set_debug`] ran.
pub fn debug_enabled() -> bool {
    api::hostlog::debug_enabled()
}

/// The 274 client's frame time: one `mainloop` pass every 20 ms.
const FRAME_MS: Duration = Duration::from_millis(20);
/// Java 289's steady-state title flame frame time.
const TITLE_FLAME_FRAME_TIME: Duration = Duration::from_millis(37);

/// `BOT_DEBUG=1` summaries: every N `client_frame`s, **not** per game tick.
const DEBUG_SUMMARY_EVERY: u32 = 50;
/// One `client_frame` over this (µs) is a hitch vs the 20 ms cadence.
/// Logged as `[host] hitch` only — default `BOT_DEBUG=1` stays a window summary.
const HITCH_US: u64 = 50_000;
/// Observe sits in front of present. One missed ~60 fps frame is enough
/// to feel, and the 50 ms `client_frame` hitch never sees it.
const OBSERVE_HITCH_US: u64 = 16_000;

#[derive(Clone, Copy, Default)]
struct DebugSnap {
    loop_ns: u64,
    raster_ns: u64,
    observe_ns: u64,
    paint_n: u64,
    skip_n: u64,
    tick_n: u64,
}

/// Deltas since the last summary, in µs / counts.
fn debug_window_delta(prev: DebugSnap, now: DebugSnap) -> (u64, u64, u64, u64, u64, u64) {
    (
        now.loop_ns.wrapping_sub(prev.loop_ns) / 1000,
        now.raster_ns.wrapping_sub(prev.raster_ns) / 1000,
        now.observe_ns.wrapping_sub(prev.observe_ns) / 1000,
        now.paint_n.wrapping_sub(prev.paint_n),
        now.skip_n.wrapping_sub(prev.skip_n),
        now.tick_n.wrapping_sub(prev.tick_n),
    )
}

fn debug_hitch_us(frame_us: u64) -> Option<u64> {
    (frame_us >= HITCH_US).then_some(frame_us)
}

fn debug_observe_hitch_us(observe_us: u64) -> Option<u64> {
    (observe_us >= OBSERVE_HITCH_US).then_some(observe_us)
}

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

/// Host-owned script auto-run overlay. Host-play identifies the active script
/// run and applies only generation-matched FlatBuffer updates through this
/// sink; script Stop/replacement clears the value, while relogs preserve it.
#[derive(Debug, Default)]
pub struct ScriptRunPolicy {
    runtime_generation: Option<u64>,
    policy_override: Option<RunPolicyOverride>,
}

impl ScriptRunPolicy {
    /// Synchronize the script runtime observed on this host frame. `None` is
    /// Stop/Idle/Error. Every identity transition clears the old overlay.
    pub fn sync_runtime(&mut self, runtime_generation: Option<u64>) {
        if self.runtime_generation != runtime_generation {
            self.runtime_generation = runtime_generation;
            self.policy_override = None;
        }
    }

    /// Apply one decoded isolate update only to the runtime that produced it.
    pub fn apply_override(&mut self, runtime_generation: u64, policy: Option<RunPolicyOverride>) {
        if self.runtime_generation == Some(runtime_generation) {
            self.policy_override = policy;
        }
    }

    fn resolved(&self) -> RunPolicy {
        resolve_run_policy(self.policy_override, RunPolicy::default())
    }
}

/// Build a slot `Client` from a process-wide cache and the shared iface
/// decode `Arc` (no second unpack / CRC probe). `error_loading` is false
/// after a successful `from_shared`.
pub fn prepare_client(
    config: ClientConfig,
    uid: i32,
    cache: Arc<Cache>,
    ifaces: Arc<Vec<Option<Box<IfType>>>>,
    ifaces_mut: impl Into<Arc<Vec<Option<Arc<IfTypeMut>>>>>,
) -> Client {
    let mut client = Client::from_shared(config, cache, ifaces, ifaces_mut);
    client.login_uid = uid;
    client
}

/// Construct a slot with all connection/resource inputs bound before the
/// client's OnDemand setup. The caller shares one profile across its slots.
pub fn prepare_client_with_profile(
    profile: Arc<client::session::ClientSessionProfile>,
    uid: i32,
    members: bool,
    lowmem: bool,
    cache: Arc<Cache>,
    ifaces: Arc<Vec<Option<Box<IfType>>>>,
    ifaces_mut: impl Into<Arc<Vec<Option<Arc<IfTypeMut>>>>>,
) -> Result<Client, String> {
    let config = profile.client_config(members, lowmem);
    let mut client = Client::from_shared_with_profile(config, cache, ifaces, ifaces_mut, profile)?;
    client.login_uid = uid;
    Ok(client)
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
        ifaces_template: Arc<Vec<Option<Box<IfType>>>>,
        ifaces_mut_template: Arc<Vec<Option<Arc<IfTypeMut>>>>,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            api::hostlog::bind_slot(&profile.username);
            let mut client = prepare_client(
                config,
                profile.uid,
                cache,
                ifaces_template,
                ifaces_mut_template,
            );

            host_log!(Category::Lifecycle, Level::Info, "thread up");

            let settings = profile.settings;
            let lamp_auto = settings.lamp_auto;
            let lamp_skill = settings.lamp_skill.clone();
            Self::run_client(
                &mut client,
                &profile.username,
                settings,
                Arc::new(AtomicBool::new(true)),
                Arc::new(AtomicBool::new(lamp_auto)),
                Arc::new(Mutex::new(lamp_skill)),
                None,
                None,
                None,
                ScriptRunPolicy::default(),
                |_, _, _, _, _| false,
                |_| false,
                |_| RandomClaim::Host,
            );
        })
    }

    /// Drive one client's `mainloop` at the slot cadence until `probe`
    /// returns true (checked before every tick, so the slot thread can stop
    /// a rail ✕ or return to its control loop within one frame). Each
    /// tick [`Host::client_tick`] drains input, latches the click, applies
    /// packets in `mainloop`, then runs `observe` before rendering. The panel
    /// can latch draw/focus for this frame while scripts see fresh packets.
    /// Rendering uses the slot's optional `Renderer` only while `client.draw`
    /// is on. Dirty snapshot families
    /// rebuild from [`DrainResult::dirty`] (not `Pump::dirty()` after
    /// drain); think (auto-run) reads energy from the live client field.
    /// The third observe arg is the count of accepted auto-run
    /// `set_run(true)` sends (from the previous tick); the fourth is the
    /// [`RandomStatus`] this frame's guardian published — the observe
    /// copies it onto the slot status row and gates script tick / follow on
    /// its hold. Observe's return is whether the slot has script/cheat/nav
    /// work, which keeps a busy slot on the frame loop. `settings` is the
    /// slot's vault profile settings (the guardian's toggle); `knock` is the
    /// script's rising-edge `on_random` arm (`Host` when the slot has no
    /// scripts).
    ///
    /// The scheduler is event-driven: captured input, a loading scene (TV
    /// static), or full-rate TV keeps the fixed 20 ms [`FRAME_MS`] mainloop.
    /// Captured/full-rate title raster is bounded by the 37 ms flame deadline
    /// or the in-game → title edge. A visible uncaptured title uses the
    /// same 1 s watch park as a ready in-game sidecar; its flame simulation
    /// catches up from wall time on the next paint. Everything else parks on
    /// `poll(2)` over the client socket's readability, the `ctl` control wake
    /// (focus/draw/stop/spawn), and the game-tick timeout ([`IDLE_PARK_MS`]),
    /// waking once per park to drain the socket and re-evaluate. Packets are
    /// never dropped: a readable socket wakes the park, and `mainloop` is the
    /// first thing that drains it. A wake that consumed no bytes (EOF, partial
    /// packet) skips the socket on the next park so it cannot busy-spin.
    ///
    /// The host owns the script run-policy overlay; `observe` synchronizes the
    /// active runtime and applies decoded FlatBuffer updates through its final
    /// [`ScriptRunPolicy`] argument. The caller passes the returned state into
    /// the next login loop so a live script run preserves its policy on relog.
    #[allow(clippy::too_many_arguments)]
    pub fn run_client<F, P, K>(
        client: &mut Client,
        username: &str,
        settings: ProfileSettings,
        random_events: Arc<AtomicBool>,
        lamp_auto: Arc<AtomicBool>,
        lamp_skill: Arc<Mutex<String>>,
        input: Option<Arc<SlotInput>>,
        mailbox: Option<Arc<FrameBuf>>,
        ctl: Option<Arc<SlotPark>>,
        run_policy: ScriptRunPolicy,
        mut observe: F,
        mut probe: P,
        mut knock: K,
    ) -> ScriptRunPolicy
    where
        F: FnMut(&mut Client, &str, u32, &RandomStatus, &mut ScriptRunPolicy) -> bool,
        P: FnMut(&mut Client) -> bool,
        K: FnMut(&DetectedRandom) -> RandomClaim,
    {
        let mut slot =
            SlotLoop::with_settings(settings, random_events, lamp_auto, lamp_skill, run_policy);
        #[cfg(feature = "performance-profile")]
        {
            slot.profile = Some(performance_profile::register(username));
        }
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
                return slot.run_policy;
            }
            if frame_cadence(client, input.as_deref()) || busy {
                socket_stalled = false;
                let start = std::time::Instant::now();
                let (new_busy, _) = Self::client_tick(
                    client,
                    &mut slot,
                    username,
                    input.as_deref(),
                    mailbox.as_deref(),
                    &mut run_sends,
                    &mut observe,
                    Some(&mut knock as &mut dyn FnMut(&DetectedRandom) -> RandomClaim),
                );
                busy = new_busy;
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
            let (new_busy, _) = Self::client_tick(
                client,
                &mut slot,
                username,
                input.as_deref(),
                mailbox.as_deref(),
                &mut run_sends,
                &mut observe,
                Some(&mut knock as &mut dyn FnMut(&DetectedRandom) -> RandomClaim),
            );
            busy = new_busy;
            if stream_bytes(client) != before {
                socket_stalled = false;
            } else if reason == ParkWake::Socket {
                socket_stalled = true;
            }
        }
    }

    /// Apply packets and refresh the guardian before observing this frame.
    /// Observation still precedes paint, so draw/focus changes affect this
    /// frame. Returns the observer's busy flag and the fresh guardian status.
    #[allow(private_interfaces)]
    #[allow(clippy::too_many_arguments)]
    pub fn client_tick<F>(
        client: &mut Client,
        slot: &mut SlotLoop,
        username: &str,
        input: Option<&SlotInput>,
        mailbox: Option<&FrameBuf>,
        run_sends: &mut u32,
        observe: &mut F,
        knock: Option<&mut dyn FnMut(&DetectedRandom) -> RandomClaim>,
    ) -> (bool, RandomStatus)
    where
        F: FnMut(&mut Client, &str, u32, &RandomStatus, &mut ScriptRunPolicy) -> bool,
    {
        let _profile_tick = client::profiling::CLIENT_TICK.start();
        Self::client_frame_observed(
            client, slot, username, input, mailbox, run_sends, knock, observe,
        )
    }

    /// One 20 ms frame: drain optional input into the shell, latch the
    /// click, run one `mainloop` pass, drain gens, refresh the guardian,
    /// then render the frame (the slot's optional `Renderer` —
    /// `client.draw` gates paint; a drawing slot stores `FrameOutput`
    /// into the optional mailbox, mirroring `Client::run`). A GPU↔CPU switch
    /// or a memory-mode change at the next login reattaches the renderer
    /// on the next paint — the slot's `Client` is retained. The panel takes the
    /// mailbox: `FrameBuf::take` hands the whole `FrameOutput` off (the
    /// `Texture` binds / reads back at the panel, the `PixMap` packs via
    /// [`FrameBuf::snapshot`]). `run_sends` is
    /// overwritten with the slot's running count of accepted auto-run
    /// sends. Returns the [`RandomStatus`] the guardian published this
    /// frame. `knock` is
    /// the script's rising-edge `on_random` arm; `None` (no scripts on
    /// the slot) keeps the claim Host.
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
        knock: Option<&mut dyn FnMut(&DetectedRandom) -> RandomClaim>,
    ) -> RandomStatus {
        Self::client_frame_observed(
            client,
            slot,
            username,
            input,
            mailbox,
            run_sends,
            knock,
            &mut |_, _, _, _, _| false,
        )
        .1
    }

    #[allow(clippy::too_many_arguments)]
    fn client_frame_observed<F>(
        client: &mut Client,
        slot: &mut SlotLoop,
        username: &str,
        input: Option<&SlotInput>,
        mailbox: Option<&FrameBuf>,
        run_sends: &mut u32,
        knock: Option<&mut dyn FnMut(&DetectedRandom) -> RandomClaim>,
        observe: &mut F,
    ) -> (bool, RandomStatus)
    where
        F: FnMut(&mut Client, &str, u32, &RandomStatus, &mut ScriptRunPolicy) -> bool,
    {
        client.set_external_reconnect_owner(true);
        client.set_hosted_title_label(Some(username));
        if let Some(inp) = input {
            let owner = inp.consume_native_frame(&mut client.shell);
            inp.classify_manual_move_intent(client, owner);
        } else {
            client.shell.latch_click();
        }
        let was_ingame = client.ingame;
        let random_events = slot.random_events.load(Ordering::Relaxed);
        slot.settings.random_events = random_events;
        let t_loop = std::time::Instant::now();
        client.mainloop();
        // The hosted title's `mainloop` changes nothing it draws, so the only
        // title paint owed ahead of its cadence is the in-game → title edge:
        // it replaces the last game frame now and starts a fresh title
        // cadence without storing another per-slot scheduler field.
        let title_dirty = was_ingame && !client.ingame;
        if title_dirty {
            slot.raster_last = None;
            slot.raster_was_on = false;
        }
        slot.loop_ns = slot
            .loop_ns
            .wrapping_add(t_loop.elapsed().as_nanos() as u64);
        let result = slot.after_drain(client);
        // Random-event guardian (spec pump placement): the snapshot is
        // fresh after the drain. Sync the live toggle from the shared
        // atomic (panel/TUI may flip it mid-session) onto the cloned
        // `ProfileSettings` the guardian reads; the returned
        // `RandomStatus` is passed to this frame's observe.
        slot.settings.random_events = slot.random_events.load(Ordering::Relaxed);
        slot.settings.lamp_auto = slot.lamp_auto.load(Ordering::Relaxed);
        if let Ok(skill) = slot.lamp_skill.lock() {
            slot.settings.lamp_skill = skill.clone();
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let status = slot
            .guardian
            .tick(client, &slot.snapshot, &slot.settings, now_ms, knock);
        slot.guardian_status = status.clone();
        if should_emit_tick(result.player_info) {
            slot.tick_n = slot.tick_n.wrapping_add(1);
        }
        // Publish actual render demand before the frontend observe hook.
        // A GPU-requested head may have fallen back to CpuPix3D, so the
        // renderer kind — not the request — controls nav materialization.
        let want_cpu = slot_want_cpu(input);
        client.set_nav_debug_drawable(
            !want_cpu
                && slot
                    .renderer
                    .as_ref()
                    .is_some_and(|renderer| renderer.backend_kind() == BackendKind::Gpu),
        );
        let t_obs = Instant::now();
        let busy = observe(client, username, *run_sends, &status, &mut slot.run_policy);
        let observe_ns = t_obs.elapsed().as_nanos() as u64;
        slot.observe_ns = slot.observe_ns.wrapping_add(observe_ns);
        if observe_ns > slot.observe_max_ns {
            slot.observe_max_ns = observe_ns;
        }
        if debug_enabled() {
            if let Some(us) = debug_observe_hitch_us(observe_ns / 1000) {
                host_log!(
                    Category::FrameStats,
                    Level::Debug,
                    slot = username,
                    "hitch: observe_us={us} paints={} skips={} ticks={}",
                    slot.paint_n,
                    slot.skip_n,
                    slot.tick_n
                );
            }
        }
        slot.apply_auto_run(client);
        // Observe and guardian actions enqueue after mainloop's socket send.
        // Keep frame cadence until those bytes drain; an otherwise idle slot
        // must not park for a server tick with a client command pending.
        let busy = busy || (client.stream.is_some() && client.out.pos > 0);
        // Channel-tune / first rebuild: TV static must re-roll every 20 ms,
        // not the 1 fps watch cadence (otherwise the zap is one snow frame
        // a second and looks like a frozen splash). A captured or full-rate
        // title follows its 37 ms flame deadline plus visible-state dirtiness.
        // An uncaptured title uses the same wall-clock 1 fps watch policy as a
        // ready in-game view; TitleFlames catches up from elapsed wall time at
        // the next paint instead of making the slot spin for animation alone.
        let zap = client.ingame && client.scene_state != 2;
        let full_rate = input.map(|i| i.full_rate()).unwrap_or(false);
        let title_fast = input.map(|i| i.enabled()).unwrap_or(false) || full_rate;
        // The per-slot CpuPix3D latch and BOT_CPU environment were sampled
        // before observe so the frontend sees current nav-draw demand.
        // A drawing slot lazily builds its `Renderer` on the first paint
        // tick; a headless (draw off) slot constructs none and never
        // enters a draw.
        // Draw off detaches: an unheaded slot must not keep headed data
        // (GPU textures, chrome) around. Cadence skips (1 fps watch, draw
        // still on) keep the renderer. A head that no longer matches the
        // slot also detaches: GPU↔CPU switches and next-login memory changes
        // reattach on the same Client. Compare the head's build request,
        // not its fallback kind, so a failed GPU init never rebuilds every
        // frame. A changed request repaints this tick rather than waiting
        // for the next 1 fps watch bound.
        let backend_flip = slot.renderer.is_some()
            && client.draw
            && (slot.renderer_prefer_cpu != Some(want_cpu)
                || slot.renderer_lowmem != Some(client.config.lowmem));
        if !client.draw || backend_flip {
            // Drop any GPU `FrameOutput::Texture` *before* the backend:
            // the handle's view aliases `GpuBackend.frame_texture`. 49
            // heads detaching at once (only-render-selected) would
            // otherwise leave mailboxes / ImGui bound to destroyed
            // textures.
            if let Some(mailbox) = mailbox {
                let _ = mailbox.take();
            }
            slot.renderer = None;
            slot.renderer_prefer_cpu = None;
            slot.renderer_lowmem = None;
            if backend_flip {
                slot.raster_last = None;
                slot.raster_was_on = false;
            }
            // A draw-off detach must not leave headed data on the sim:
            // drop the overlay meshes a headed build/attach wrote (keep
            // the compact tile stamps) and re-arm the attach materialize,
            // so the next head's first paint restores them without a
            // rebuild (rule 6 of the head design). Fires on the falling
            // edge only. A GPU↔CPU / mem flip drops the head while `draw`
            // stays on: overlay meshes on the sim survive, loc models do
            // not — re-arm share-light and dirty the minimap pixmap latch.
            rearm_after_head_drop(client, !client.draw && slot.draw_was_on, backend_flip);
        }
        slot.draw_was_on = client.draw;
        let paint = if client.ingame {
            raster_this_tick(
                client.draw,
                zap || full_rate,
                t_loop,
                &mut slot.raster_last,
                &mut slot.raster_was_on,
            )
        } else if title_fast {
            title_raster_this_tick(
                client.draw,
                title_dirty,
                t_loop,
                &mut slot.raster_last,
                &mut slot.raster_was_on,
            )
        } else {
            // Passing dirtiness through the draw gate is essential: on a
            // draw-off logout edge it must not construct a renderer.
            raster_this_tick(
                client.draw,
                title_dirty,
                t_loop,
                &mut slot.raster_last,
                &mut slot.raster_was_on,
            )
        };
        let mut frame: Option<FrameOutput> = None;
        if paint {
            let t_r = std::time::Instant::now();
            // The head's build request is latched beside it so the detach
            // check above sees a GPU↔CPU switch or next-login memory change.
            slot.renderer_prefer_cpu = Some(want_cpu);
            slot.renderer_lowmem = Some(client.config.lowmem);
            let renderer = slot.renderer.get_or_insert_with(|| {
                // A new head owns a blank 512×512 minimap. Dirty the
                // Client latch (a backend/next-login mode change keeps draw
                // on, so `set_draw` does not) or `check_minimap` will skip.
                client.minimap_level = -1;
                // GPU-first (the host default): the slot's renderer
                // prefers the wgpu backend, with `CpuBackend` as the
                // fallback on wgpu init failure (`Renderer::new`
                // selects, `Renderer::backend_kind` reports). The
                // preference is process-wide and idempotent, so the
                // first paint of any slot opts the process in;
                // `BOT_CPU=1` forces the CPU fidelity path.
                Renderer::new_prefer(client.config.lowmem, !want_cpu)
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
        #[cfg(feature = "performance-profile")]
        slot.publish_profile();
        if debug_enabled() {
            let frame_us = t_loop.elapsed().as_micros() as u64;
            if let Some(us) = debug_hitch_us(frame_us) {
                host_log!(
                    Category::FrameStats,
                    Level::Debug,
                    slot = username,
                    "hitch: frame_us={us} paints={} skips={} ticks={}",
                    slot.paint_n,
                    slot.skip_n,
                    slot.tick_n
                );
            }
            if slot.log_n.is_multiple_of(DEBUG_SUMMARY_EVERY) {
                let now = DebugSnap {
                    loop_ns: slot.loop_ns,
                    raster_ns: slot.raster_ns,
                    observe_ns: slot.observe_ns,
                    paint_n: slot.paint_n,
                    skip_n: slot.skip_n,
                    tick_n: slot.tick_n,
                };
                let (d_loop, d_raster, d_observe, d_paint, d_skip, d_tick) =
                    debug_window_delta(slot.dbg, now);
                let window_ms = slot.dbg_at.map(|t| t.elapsed().as_millis()).unwrap_or(0);
                let max_observe_us = slot.observe_max_ns / 1000;
                host_log!(
                    Category::FrameStats,
                    Level::Debug,
                    slot = username,
                    "d_loop_us={d_loop} d_raster_us={d_raster} d_observe_us={d_observe} max_observe_us={max_observe_us} d_paints={d_paint} d_skips={d_skip} d_ticks={d_tick} window_ms={window_ms}"
                );
                slot.dbg = now;
                slot.dbg_at = Some(Instant::now());
                slot.observe_max_ns = 0;
            }
        }
        // The whole `FrameOutput` lands in the mailbox, not just the
        // packed pixels: the panel takes the `FrameOutput::Texture` (GPU
        // backend) or packs the `PixMap` (CPU backend) at its consume
        // site, and `FrameBuf::snapshot` keeps the CPU packing path for
        // the tests.
        if let Some(frame) = frame {
            if let Some(mailbox) = mailbox {
                #[cfg(feature = "journal-paint-proof")]
                mailbox.store_journal_paint(
                    frame,
                    JournalPaintStamp {
                        modal_root: client.main_modal_id,
                        journal_paint_hidden: client.journal_paint_hidden(),
                    },
                );
                #[cfg(not(feature = "journal-paint-proof"))]
                mailbox.store(frame);
            }
        }
        *run_sends = slot.run_sends;
        (busy, status)
    }
}

/// Title frames for a logged-out slot whose owner waits outside
/// [`Host::run_client`]: the login FIFO, a retry backoff (the 60 s code-5
/// cooldown included) or a transfer countdown. Each frame is the pump's own
/// frame without the observe hook, on the pump's title cadence: a captured
/// or full-rate title every 20 ms (flames paint on their 37 ms deadline), a
/// visible watch-only title once a second, and a draw-off title once per
/// [`IDLE_PARK_MS`] so a draw toggle still lands, with no renderer. User
/// input is applied as it arrives, and a Log In made while the owner already
/// waits to log in is dropped: it asks for nothing new. The owner builds one
/// when a wait starts and drops it before the pump, with its own renderer,
/// takes over.
pub struct TitleWait {
    slot: SlotLoop,
    input: Option<Arc<SlotInput>>,
    mailbox: Option<Arc<FrameBuf>>,
    last_frame: Option<Instant>,
}

impl TitleWait {
    pub fn new(input: Option<Arc<SlotInput>>, mailbox: Option<Arc<FrameBuf>>) -> Self {
        Self {
            slot: SlotLoop::new(),
            input,
            mailbox,
            last_frame: None,
        }
    }

    /// Run the logged-out title frame that is due, if any. `per_frame` runs
    /// after `mainloop`, where the pump runs its per-frame hook (the panel
    /// mirrors draw and focus there). Returns how long the owner may block
    /// before the next frame is due.
    pub fn frame(
        &mut self,
        client: &mut Client,
        username: &str,
        mut per_frame: impl FnMut(&mut Client),
    ) -> Duration {
        debug_assert!(!client.ingame, "a login wait runs on the title");
        let input = self.input.as_deref();
        let interval = if frame_cadence(client, input) {
            FRAME_MS
        } else if client.draw {
            WATCH_PARK_MS
        } else {
            IDLE_PARK_MS
        };
        let now = Instant::now();
        if let Some(since) = self.last_frame.map(|last| now.duration_since(last)) {
            if since < interval {
                return interval - since;
            }
        }
        self.last_frame = Some(now);
        let mut run_sends = 0;
        Host::client_frame_observed(
            client,
            &mut self.slot,
            username,
            input,
            self.mailbox.as_deref(),
            &mut run_sends,
            None,
            &mut |c, _, _, _, _| {
                per_frame(c);
                false
            },
        );
        let _ = client.take_title_login_request();
        interval.saturating_sub(now.elapsed())
    }
}

/// Frame-loop cadence: a slot that captures input, is still loading (TV
/// static re-rolls every 20 ms), or runs full-rate (the panel's sidecar-50
/// pref, or the TV full-rate latch) keeps the fixed 20 ms loop — that loop
fn slot_want_cpu(input: Option<&SlotInput>) -> bool {
    let from_slot = input.map(|i| i.prefer_cpu()).unwrap_or(false);
    let from_env = std::env::var("BOT_CPU").map(|v| v == "1").unwrap_or(false);
    from_slot || from_env
}

/// Whether this slot's 20 ms loop is the **input** cadence (`SlotInput`
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
    client.stream.as_ref().map(|s| s.bytes_in()).unwrap_or(0)
}

/// Re-arm sim flags after dropping a `Renderer`.
///
/// Draw-off: dematerialize overlay meshes (stamps stay) and re-arm
/// overlay + share-light. Backend flip (`draw` stays on): overlay meshes
/// stay on the sim, but loc models died with the head — re-arm share-light
/// and dirty `minimap_level` so the new pixmap is composed.
fn rearm_after_head_drop(client: &mut Client, draw_off_edge: bool, backend_flip: bool) {
    if draw_off_edge {
        client.world.dematerialize_overlay();
    } else if backend_flip {
        client.world.share_light_pending = true;
        client.minimap_level = -1;
    }
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

/// Readiness includes WS leftovers and TLS/tungstenite buffers, not just
/// kernel bytes. `available` is nonblocking; an error wakes the ordinary
/// client transport-loss path. A stalled partial packet remains suppressed.
fn buffered_socket_ready(client: &mut Client, poll_socket: bool) -> bool {
    poll_socket
        && client
            .stream
            .as_mut()
            .is_some_and(|stream| stream.available().map_or(true, |bytes| bytes != 0))
}

/// Block until the client socket is readable, a control wake lands, or
/// `timeout` elapses. The socket is polled only when `poll_socket` — a
/// previous no-consumption wake (EOF or a packet still mid-flight) leaves
/// the socket permanently readable, so re-polling it would busy-spin.
/// No socket and no control channel falls back to sleeping the timeout.
///
/// Unix production body is stack `[pollfd; 2]` + inline `libc::poll` (no
/// heap). Windows counterpart uses fixed `[WSAPOLLFD; 2]` + `WSAPoll`.
#[cfg(unix)]
#[allow(unsafe_code)]
fn park(
    client: &mut Client,
    ctl: Option<&SlotPark>,
    poll_socket: bool,
    timeout: Duration,
) -> ParkWake {
    if buffered_socket_ready(client, poll_socket) {
        if let Some(ctl) = ctl {
            ctl.drain();
        }
        return ParkWake::Socket;
    }
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

/// Windows park: fixed-size stack `WSAPoll` arrays (max 2 handles), same
/// control-drain / socket-priority / timeout semantics as the Unix body.
#[cfg(windows)]
#[allow(unsafe_code)]
fn park(
    client: &mut Client,
    ctl: Option<&SlotPark>,
    poll_socket: bool,
    timeout: Duration,
) -> ParkWake {
    if buffered_socket_ready(client, poll_socket) {
        if let Some(ctl) = ctl {
            ctl.drain();
        }
        return ParkWake::Socket;
    }
    use windows_sys::Win32::Networking::WinSock::{
        WSAPoll, POLLERR, POLLHUP, POLLIN, POLLNVAL, SOCKET, WSAPOLLFD,
    };
    let mut fds = [WSAPOLLFD {
        fd: SOCKET::MAX,
        events: 0,
        revents: 0,
    }; 2];
    let mut n = 0usize;
    if let Some(ctl) = ctl {
        fds[n] = WSAPOLLFD {
            fd: ctl.raw_socket() as SOCKET,
            events: POLLIN,
            revents: 0,
        };
        n += 1;
    }
    let socket = if poll_socket {
        client.stream.as_ref().map(|stream| {
            fds[n] = WSAPOLLFD {
                fd: stream.raw_socket() as SOCKET,
                events: POLLIN,
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
    let rc = unsafe { WSAPoll(fds.as_mut_ptr(), n as u32, ms) };
    if rc > 0 {
        let fired = |i: usize| fds[i].revents & (POLLIN | POLLHUP | POLLERR | POLLNVAL) != 0;
        let control_fired = ctl.is_some() && fired(0);
        if control_fired {
            // Consume the kick bytes: an undrained control handle stays
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

#[cfg(test)]
fn stream_wait_handle(stream: &client::io::ClientStream) -> slot_io::WaitHandle {
    #[cfg(unix)]
    {
        stream.fd()
    }
    #[cfg(windows)]
    {
        stream.raw_socket()
    }
}

/// Fast logged-out paint cadence for a captured/full-rate title. The first
/// visible title paints immediately; thereafter a paint is due only when the
/// flame clock reaches 37 ms or on the in-game → title edge (`dirty`). An
/// edge paint does not postpone the next flame deadline.
fn title_raster_this_tick(
    draw: bool,
    dirty: bool,
    now: Instant,
    last_flame: &mut Option<Instant>,
    was_on: &mut bool,
) -> bool {
    if !draw {
        *was_on = false;
        return false;
    }
    let rising = !*was_on;
    *was_on = true;
    let flame_due =
        rising || last_flame.is_none_or(|last| now.duration_since(last) >= TITLE_FLAME_FRAME_TIME);
    if flame_due {
        *last_flame = Some(now);
    }
    flame_due || dirty
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

/// Per-login post-drain state: snapshot, auto-run echo, the full-rate switch,
/// the random-event guardian, and the optional `Renderer` a drawing slot
/// owns. The script run policy is supplied by the outer slot thread.
struct SlotLoop {
    pump: Pump,
    snapshot: GameSnapshot,
    run_on: bool,
    run_sends: u32,
    /// Host-owned run-scoped auto-run overlay and runtime fence.
    run_policy: ScriptRunPolicy,
    /// The slot's real `ProfileSettings` (wired once by `run_client` from
    /// the vault profile; the guardian's toggle reads it). `random_events`,
    /// `lamp_auto`, and `lamp_skill` are refreshed each frame from the
    /// live atomics below.
    settings: ProfileSettings,
    /// Live guardian toggle (shared with `SlotArm::random_events`): a
    /// panel/TUI flip reaches the running slot without a respawn.
    random_events: Arc<AtomicBool>,
    /// Live lamp auto-use toggle (shared with `SlotArm::lamp_auto`).
    lamp_auto: Arc<AtomicBool>,
    /// Live lamp skill choice (shared with `SlotArm::lamp_skill`).
    lamp_skill: Arc<Mutex<String>>,
    /// Random-event guardian (act/hold; the trapped-kind hold and the
    /// `on_random` knock live here).
    guardian: Guardian,
    /// Last status published by the guardian. The 289 idle-timer
    /// interruption consumes this before the next `mainloop` pass.
    guardian_status: RandomStatus,
    renderer: Option<Renderer>,
    /// The `prefer_cpu` request the attached head was built with, so a
    /// GPU↔CPU flip on a live slot is a drop+reattach, not a restart.
    /// `None` while no head is attached (kept in sync with
    /// [`SlotLoop::renderer`]).
    renderer_prefer_cpu: Option<bool>,
    /// The memory mode this head was built with. A different next-login
    /// mode rebuilds it before the first paint of that login.
    renderer_lowmem: Option<bool>,
    /// `Instant` of the last paint: the watch-only 1 fps decision repaints
    /// when it is ≥1 s old, and a fast title keeps its flame deadline here.
    raster_last: Option<Instant>,
    raster_was_on: bool,
    /// `client.draw` from the previous frame. The draw-off detach
    /// dematerializes the headed overlay meshes on the falling edge, so a
    /// parked draw-off slot does not re-walk the tile grid every frame.
    draw_was_on: bool,
    loop_ns: u64,
    raster_ns: u64,
    /// Wall time of the observe hook (`script_observe` + snapshot encode).
    observe_ns: u64,
    /// Max single observe in the current `BOT_DEBUG` window.
    observe_max_ns: u64,
    paint_n: u64,
    skip_n: u64,
    /// Game-tick edges (`PLAYER_INFO` this drain). Counted internally;
    /// printed in the 50-frame summary, not per tick.
    tick_n: u64,
    log_n: u32,
    /// Totals at the last `BOT_DEBUG` window line (deltas, not cumulatives).
    dbg: DebugSnap,
    dbg_at: Option<Instant>,
    #[cfg(feature = "performance-profile")]
    profile: Option<Arc<performance_profile::SlotTiming>>,
    #[cfg(feature = "performance-profile")]
    profile_published_at: Instant,
}

impl SlotLoop {
    /// A loop with default settings: host tests and a [`TitleWait`], whose
    /// frames never reach the in-game guardian or auto-run.
    fn new() -> Self {
        Self::with_settings(
            ProfileSettings::default(),
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new("strength".to_string())),
            ScriptRunPolicy::default(),
        )
    }

    fn with_settings(
        settings: ProfileSettings,
        random_events: Arc<AtomicBool>,
        lamp_auto: Arc<AtomicBool>,
        lamp_skill: Arc<Mutex<String>>,
        run_policy: ScriptRunPolicy,
    ) -> Self {
        Self {
            pump: Pump::new(),
            snapshot: GameSnapshot::new(),
            run_on: false,
            run_sends: 0,
            run_policy,
            settings,
            random_events,
            lamp_auto,
            lamp_skill,
            guardian: Guardian::new(),
            guardian_status: RandomStatus::default(),
            renderer: None,
            renderer_prefer_cpu: None,
            renderer_lowmem: None,
            raster_last: None,
            raster_was_on: false,
            draw_was_on: false,
            loop_ns: 0,
            raster_ns: 0,
            observe_ns: 0,
            observe_max_ns: 0,
            paint_n: 0,
            skip_n: 0,
            tick_n: 0,
            log_n: 0,
            dbg: DebugSnap::default(),
            dbg_at: None,
            #[cfg(feature = "performance-profile")]
            profile: None,
            #[cfg(feature = "performance-profile")]
            profile_published_at: Instant::now(),
        }
    }

    #[cfg(feature = "performance-profile")]
    fn publish_profile(&mut self) {
        if self.profile_published_at.elapsed() < Duration::from_secs(1) {
            return;
        }
        if let Some(profile) = &self.profile {
            profile.publish(self.loop_ns, self.observe_ns, self.raster_ns);
        }
        self.profile_published_at = Instant::now();
    }

    fn after_drain(&mut self, client: &mut Client) -> DrainResult {
        let result = self.pump.drain_client(client);
        if result.session_changed || (!client.ingame && self.snapshot.ingame()) {
            self.guardian = Guardian::new();
            self.guardian_status = RandomStatus::default();
            self.run_on = false;
        }
        publish_snapshot(&mut self.snapshot, client, result);
        result
    }

    fn apply_auto_run(&mut self, client: &mut Client) {
        // Live client field: UPDATE_RUNENERGY writes here. Snapshot energy
        // can stay 0 if a stat-family rebuild ran before the energy packet.
        let energy = client.runenergy;
        if let Some(echo) = run_echo(client) {
            self.run_on = echo;
        }
        if energy == 0 {
            // Cannot be running; wins over a stale run-on echo.
            self.run_on = false;
        }
        let policy = self.run_policy.resolved();
        if self.snapshot.local_player().is_some()
            && auto_run_ready(client.ingame, client.scene_state)
            && auto_run_tick(energy, self.run_on, policy)
            && set_run(client, true)
        {
            self.run_on = true;
            self.run_sends += 1;
        }
    }
}

/// Publish one client drain into a host snapshot. Logout and every successful
/// login/reconnect establish a new empty session view at the current generation
/// watermark; ordinary drains rebuild only dirty families and advance the host
/// tick only from the client's actual PLAYER_INFO observation.
///
/// This is the shared production seam for slot loops and narrow harnesses:
/// callers own the [`Pump`] and pass its exact [`DrainResult`].
pub fn publish_snapshot(snapshot: &mut GameSnapshot, client: &Client, result: DrainResult) {
    if !client.ingame {
        snapshot.reset_session(result.gens);
        return;
    }
    let player_info = if result.session_changed {
        // A drain can contain packets on both sides of a reconnect. The
        // client records the exact grant watermark; only later generations
        // may republish a family, including packets later in this same drain.
        let start = client.session_start_gens();
        snapshot.reset_session(start);
        result.gens.player_info != start.player_info
    } else {
        result.player_info
    };
    snapshot.rebuild_from_drain(client, player_info);
}

/// Result of publishing a frontend-owned snapshot from the client's real
/// generation/session watermarks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrontendPublication {
    /// Logout or a successful login/reconnect replaced the observed session.
    pub session_boundary: bool,
    /// The snapshot was reset or at least one packet family moved.
    pub changed: bool,
}

impl Host {
    /// Publish a snapshot owned outside the slot loop while preserving the
    /// production [`Pump`] and [`publish_snapshot`] semantics. Frontends call
    /// this from the post-drain frame hook; `last` is its incremental pump
    /// cursor (separate from snapshot gens because PLAYER_INFO is a tick edge),
    /// so no second world copy or pointer/socket identity heuristic is needed.
    pub fn publish_frontend_snapshot(
        last: &mut client::client::ClientGens,
        snapshot: &mut GameSnapshot,
        client: &Client,
    ) -> FrontendPublication {
        let mut pump = Pump::from_gens(*last);
        let result = pump.drain_client(client);
        *last = pump.last();
        let session_boundary = result.session_changed || !client.ingame;
        let changed = session_boundary || result.dirty.any();
        publish_snapshot(snapshot, client, result);
        FrontendPublication {
            session_boundary,
            changed,
        }
    }
}

/// Run on/off from the orb pair. 152 visible and 153 hidden → running;
/// the inverse → walking. Both the same (unpacked defaults) is unknown —
/// a packed jag starts with `hide = false` on every component.
fn run_echo(client: &Client) -> Option<bool> {
    let off = client.if_(152)?;
    let on = client.if_(153)?;
    if off.hide == on.hide {
        return None;
    }
    Some(!off.hide)
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
