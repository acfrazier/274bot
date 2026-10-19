//! Per-slot frame work the core does on each slot's client thread, and the
//! hook a front end supplies for its own per-frame client work.
//!
//! The core owns the host-side frame duties both front ends share: manual
//! input cancels an armed walk, each frame's snapshot is published (a session
//! boundary clears the slot's walk and step latch), and after the front end's
//! hook the armed walk follows one leg per player tick. The front end's
//! [`FrameHook`] receives the slot's client as `&mut dyn Any` and downcasts it
//! once to the client type it links (one `TypeId` compare, no allocation).

use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use api::snapshot::{GameSnapshot, WorldTile};
use host_play::{SlotFrameInput, WalkArms};
use nav::router::Route;
use nav::world::NavWorld;
use nav::WorldState;

/// A front end's per-frame client work for one slot.
pub trait FrameHook: Send + Sync + 'static {
    /// Called on `slot`'s client thread every frame, after the core
    /// published the frame's snapshot and before the armed walk follows.
    /// `client` is the slot's client; downcast it to the client type.
    fn frame(&self, client: &mut dyn Any, slot: &str, frame: &FrameContext<'_>);
}

/// What the core observed this frame, for the front end's hook.
pub struct FrameContext<'a> {
    slot: &'a str,
    hold: bool,
    session_boundary: bool,
    walk_cancelled: bool,
    shared: &'a FrameShared,
}

impl FrameContext<'_> {
    /// The operator holds the slot (scenario follow freezes).
    pub fn hold(&self) -> bool {
        self.hold
    }

    /// The client crossed a session boundary this frame (login, logout or
    /// reconnect); the core already ended the slot's armed walk.
    pub fn session_boundary(&self) -> bool {
        self.session_boundary
    }

    /// Manual input cancelled the slot's armed walk this frame.
    pub fn walk_cancelled(&self) -> bool {
        self.walk_cancelled
    }

    /// Flag the front end's walk destination for clearing (read back with
    /// [`crate::OperatorSession::take_walk_clear`]).
    pub fn mark_walk_clear(&self) {
        self.shared.mark_walk_clear();
    }

    /// Read the slot's running script route (if a script armed one) for
    /// paint. The script route owner stays locked while `read` runs.
    pub fn with_script_route<R>(
        &self,
        read: impl FnOnce(Option<crate::walk_map::RouteProjectionView<'_>>) -> R,
    ) -> R {
        let handle = self.shared.script_nav_paint.lock().unwrap().clone();
        match handle {
            Some(handle) => handle.with_map_route(self.slot, |route| {
                read(route.map(crate::walk_map::RouteProjectionView::from_host))
            }),
            None => read(None),
        }
    }

    /// The slot's armed walk route and the tile it currently aims at.
    pub fn walk_route(&self) -> (Option<Route>, Option<WorldTile>) {
        let arm = self
            .shared
            .walk_arms
            .lock()
            .unwrap()
            .get(self.slot)
            .cloned();
        match arm {
            Some(arm) => {
                let arm = arm.lock().unwrap();
                (arm.route.as_deref().cloned(), arm.traveller.current_aim())
            }
            None => (None, None),
        }
    }
}

/// One slot's latest published frame facts.
struct SlotSnapshot {
    snapshot: GameSnapshot,
    world_state: Option<WorldState>,
}

type StepLatch = (u64, (i32, i32, i32));

/// Frame state shared between the core and the slot threads.
pub(crate) struct FrameShared {
    walk_arms: WalkArms,
    snapshots: Mutex<HashMap<String, SlotSnapshot>>,
    /// The last player tick and tile each slot's walk stepped on.
    step_latch: Mutex<HashMap<String, StepLatch>>,
    /// Slots whose next frame must forget the client generations it
    /// published against (a fresh slot under a reused name).
    lifetime_resets: Mutex<HashSet<String>>,
    /// Set when a walk ended, was refused or was cancelled.
    walk_clear: AtomicBool,
    /// Also keep a [`WorldState`] per published snapshot.
    keep_world_state: AtomicBool,
    nav_world: Mutex<Option<Arc<NavWorld>>>,
    map_members: AtomicBool,
    script_nav_paint: Mutex<Option<host_play::ScriptNavPaint>>,
}

impl FrameShared {
    pub(crate) fn new() -> Self {
        Self {
            walk_arms: WalkArms::default(),
            snapshots: Mutex::new(HashMap::new()),
            step_latch: Mutex::new(HashMap::new()),
            lifetime_resets: Mutex::new(HashSet::new()),
            walk_clear: AtomicBool::new(false),
            keep_world_state: AtomicBool::new(false),
            nav_world: Mutex::new(None),
            map_members: AtomicBool::new(false),
            script_nav_paint: Mutex::new(None),
        }
    }

    pub(crate) fn walk_arms(&self) -> &WalkArms {
        &self.walk_arms
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn replace_walk_arms(&mut self, arms: WalkArms) {
        self.walk_arms = arms;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn publish_test_snapshot(
        &self,
        name: &str,
        snapshot: GameSnapshot,
        world_state: WorldState,
    ) {
        self.snapshots.lock().unwrap().insert(
            name.to_owned(),
            SlotSnapshot {
                snapshot,
                world_state: Some(world_state),
            },
        );
    }

    pub(crate) fn set_keep_world_state(&self, keep: bool) {
        self.keep_world_state.store(keep, Ordering::Relaxed);
    }

    pub(crate) fn bind_world(&self, world: Option<Arc<NavWorld>>, map_members: bool) {
        *self.nav_world.lock().unwrap() = world;
        self.map_members.store(map_members, Ordering::Relaxed);
    }

    pub(crate) fn bind_script_nav_paint(&self, paint: host_play::ScriptNavPaint) {
        *self.script_nav_paint.lock().unwrap() = Some(paint);
    }

    pub(crate) fn take_walk_clear(&self) -> bool {
        self.walk_clear.swap(false, Ordering::Relaxed)
    }

    pub(crate) fn mark_walk_clear(&self) {
        self.walk_clear.store(true, Ordering::Relaxed);
    }

    /// Forget `name`'s step latch (a new walk steps on the next tick).
    pub(crate) fn clear_step_latch(&self, name: &str) {
        self.step_latch.lock().unwrap().remove(name);
    }

    /// Read `name`'s latest published snapshot.
    pub(crate) fn with_snapshot<R>(
        &self,
        name: &str,
        f: impl FnOnce(&GameSnapshot) -> R,
    ) -> Option<R> {
        self.snapshots
            .lock()
            .unwrap()
            .get(name)
            .map(|slot| f(&slot.snapshot))
    }

    /// Read `name`'s latest published snapshot with its world state (only
    /// kept when the front end asked for it).
    pub(crate) fn with_world_state<R>(
        &self,
        name: &str,
        f: impl FnOnce(&GameSnapshot, &WorldState) -> R,
    ) -> Option<R> {
        let snapshots = self.snapshots.lock().unwrap();
        let slot = snapshots.get(name)?;
        slot.world_state
            .as_ref()
            .map(|world| f(&slot.snapshot, world))
    }

    pub(crate) fn cancel_walk_on_manual_input(
        &self,
        name: &str,
        here: Option<(i32, i32, i32)>,
        frame: SlotFrameInput,
    ) -> bool {
        let cancelled = host_play::cancel_walk_arm_on_manual_input(
            name,
            &self.walk_arms,
            here.map(|(x, z, level)| WorldTile { x, z, level }),
            frame,
        );
        if cancelled {
            self.mark_walk_clear();
        }
        cancelled
    }

    pub(crate) fn take_lifetime_reset(&self, name: &str) -> bool {
        self.lifetime_resets.lock().unwrap().remove(name)
    }

    /// Publish `name`'s frame through `publish` into its retained snapshot;
    /// a session boundary ends the slot's walk. Returns whether the frame
    /// crossed a session boundary.
    pub(crate) fn publish(
        &self,
        name: &str,
        publish: impl FnOnce(&mut GameSnapshot) -> host::FrontendPublication,
    ) -> bool {
        let session_boundary = {
            let mut snapshots = self.snapshots.lock().unwrap();
            let slot = snapshots
                .entry(name.to_string())
                .or_insert_with(|| SlotSnapshot {
                    snapshot: GameSnapshot::new(),
                    world_state: None,
                });
            let publication = publish(&mut slot.snapshot);
            if self.keep_world_state.load(Ordering::Relaxed)
                && (publication.changed || slot.world_state.is_none())
            {
                slot.world_state = Some(WorldState::from_snapshot(&slot.snapshot));
            }
            publication.session_boundary
        };
        if session_boundary {
            self.end_session(name);
        }
        session_boundary
    }

    pub(crate) fn context<'a>(
        &'a self,
        slot: &'a str,
        hold: bool,
        session_boundary: bool,
        walk_cancelled: bool,
    ) -> FrameContext<'a> {
        FrameContext {
            slot,
            hold,
            session_boundary,
            walk_cancelled,
            shared: self,
        }
    }

    /// Step `name`'s armed walk one leg per player-info tick (a hop is sent
    /// once per server tick, not re-sent every frame). `step` sends the leg.
    pub(crate) fn follow_walk(
        &self,
        name: &str,
        facts: FollowFacts,
        step: impl FnOnce(&GameSnapshot, &mut host_play::WalkArm, Option<&NavWorld>, bool) -> bool,
    ) {
        let FollowFacts {
            here,
            tick,
            hold,
            host_move_owned,
        } = facts;
        let Some(arm) = self.walk_arms.lock().unwrap().get(name).cloned() else {
            return;
        };
        let map_members = self.map_members.load(Ordering::Relaxed);
        let world = self.nav_world.lock().unwrap().clone();
        let finished = {
            let snapshots = self.snapshots.lock().unwrap();
            let Some(slot) = snapshots.get(name) else {
                return;
            };
            let mut arm = arm.lock().unwrap();
            let refused = world.as_deref().is_some_and(|world| {
                host_play::observe_walk_arm_admission(
                    &slot.snapshot,
                    &mut arm,
                    world,
                    map_members,
                    Some(name),
                )
            });
            if refused || !crate::walk_permissions::manual_walk_may_follow(hold, host_move_owned) {
                if refused {
                    self.mark_walk_clear();
                }
                return;
            }
            {
                let mut latch = self.step_latch.lock().unwrap();
                if latch.get(name) == Some(&(tick, here)) {
                    return;
                }
                latch.insert(name.to_string(), (tick, here));
            }
            step(&slot.snapshot, &mut arm, world.as_deref(), map_members)
        };
        if finished {
            self.mark_walk_clear();
        }
    }

    /// End `name`'s session: its armed walk is cancelled and its step latch
    /// cleared. Returns whether a walk was armed.
    fn end_session(&self, name: &str) -> bool {
        self.step_latch.lock().unwrap().remove(name);
        let removed = self.walk_arms.lock().unwrap().remove(name);
        let Some(arm) = removed else {
            return false;
        };
        host_play::cancel_walk_arm(Some(name), &mut arm.lock().unwrap(), None, "SessionEnded");
        true
    }

    /// A slot spawn or removal under `name`: forget everything published for
    /// the previous client of that name and end its session. Returns whether
    /// a walk was armed.
    pub(crate) fn end_lifetime(&self, name: &str) -> bool {
        self.snapshots.lock().unwrap().remove(name);
        self.lifetime_resets
            .lock()
            .unwrap()
            .insert(name.to_string());
        self.end_session(name)
    }
}

/// One frame's facts the armed-walk follow needs.
pub(crate) struct FollowFacts {
    pub(crate) here: (i32, i32, i32),
    pub(crate) tick: u64,
    pub(crate) hold: bool,
    pub(crate) host_move_owned: bool,
}

/// The host-play operations the slot frame applies to the client type `C`.
/// This crate cannot name the client type, so [`slot_frame`] takes them as
/// values and the call site passes host-play's own functions, whose types
/// fix `C` (and the client-generation type `G`).
pub(crate) struct ClientOps<H, P, T, S> {
    /// The player's world tile, once a local player decodes.
    pub(crate) here: H,
    /// Publish the frame into the slot's snapshot against its generations.
    pub(crate) publish: P,
    /// The client's player-info tick.
    pub(crate) tick: T,
    /// Send the armed walk's next leg.
    pub(crate) step: S,
}

impl<H, P, T, S> ClientOps<H, P, T, S> {
    /// Bundle the operations; the bounds give a closure argument (`tick`)
    /// its client type from the host-play functions passed beside it.
    pub(crate) fn new<C, G>(here: H, publish: P, step: S, tick: T) -> Self
    where
        H: Fn(&C) -> Option<(i32, i32, i32)>,
        P: Fn(&mut G, &mut GameSnapshot, &C) -> host::FrontendPublication,
        S: Fn(
            &mut C,
            &GameSnapshot,
            &mut host_play::WalkArm,
            Option<&NavWorld>,
            (i32, i32, i32),
            bool,
            Option<&str>,
        ) -> bool,
        T: Fn(&C) -> u64,
    {
        Self {
            here,
            publish,
            tick,
            step,
        }
    }
}

/// The per-frame closure the core hands host-play for every slot.
pub(crate) fn slot_frame<C, G, H, P, T, S>(
    shared: Arc<FrameShared>,
    hook: Arc<dyn FrameHook>,
    ops: ClientOps<H, P, T, S>,
) -> impl Fn(&mut C, &str, SlotFrameInput) + Send + Sync + 'static
where
    C: Any,
    G: Default + Send + 'static,
    H: Fn(&C) -> Option<(i32, i32, i32)> + Send + Sync + 'static,
    P: Fn(&mut G, &mut GameSnapshot, &C) -> host::FrontendPublication + Send + Sync + 'static,
    T: Fn(&C) -> u64 + Send + Sync + 'static,
    S: Fn(
            &mut C,
            &GameSnapshot,
            &mut host_play::WalkArm,
            Option<&NavWorld>,
            (i32, i32, i32),
            bool,
            Option<&str>,
        ) -> bool
        + Send
        + Sync
        + 'static,
{
    // The client generations each slot last published against. Kept here,
    // on the slot threads, because only they read them.
    let gens: Mutex<HashMap<String, G>> = Mutex::new(HashMap::new());
    move |client, name, frame| {
        let hold = frame.hold;
        let host_move_owned = frame.host_move_owned;
        let walk_cancelled = shared.cancel_walk_on_manual_input(name, (ops.here)(client), frame);
        let session_boundary = {
            let mut gens = gens.lock().unwrap();
            if shared.take_lifetime_reset(name) {
                gens.remove(name);
            }
            let last = gens.entry(name.to_string()).or_default();
            shared.publish(name, |snapshot| (ops.publish)(last, snapshot, client))
        };
        hook.frame(
            client,
            name,
            &shared.context(name, hold, session_boundary, walk_cancelled),
        );
        let Some(here) = (ops.here)(client) else {
            return;
        };
        let facts = FollowFacts {
            here,
            tick: (ops.tick)(client),
            hold,
            host_move_owned,
        };
        shared.follow_walk(name, facts, |snapshot, arm, world, members| {
            (ops.step)(client, snapshot, arm, world, here, members, Some(name))
        });
    }
}

/// What a speaker-gate step did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioChange {
    None,
    Opened,
    Closed,
}

impl AudioChange {
    fn from_host(change: host_play::audio::AudioChange) -> Self {
        match change {
            host_play::audio::AudioChange::None => Self::None,
            host_play::audio::AudioChange::Opened => Self::Opened,
            host_play::audio::AudioChange::Closed => Self::Closed,
        }
    }
}

/// The focused-slot speaker gate: at most one speaker `S`, owned by the
/// focused slot while its applied client mode plays music.
pub struct AudioGate<S>(host_play::audio::AudioGate<S>);

impl<S> Default for AudioGate<S> {
    fn default() -> Self {
        Self(host_play::audio::AudioGate::new())
    }
}

impl<S> AudioGate<S> {
    /// The slot holding the speaker.
    pub fn owner(&self) -> Option<String> {
        self.0.owner()
    }

    /// `name` left: close its speaker if it holds it.
    pub fn release(&self, name: &str) -> AudioChange {
        AudioChange::from_host(self.0.release(name))
    }

    /// One frame of `name`: open a speaker through `open` when it should
    /// own one, close it when it should not.
    pub fn frame(
        &self,
        name: &str,
        focused: Option<&str>,
        music_on: bool,
        open: impl FnOnce() -> Option<S>,
    ) -> AudioChange {
        AudioChange::from_host(self.0.frame(name, focused, music_on, open))
    }
}

#[cfg(test)]
#[path = "frames_tests.rs"]
mod tests;
