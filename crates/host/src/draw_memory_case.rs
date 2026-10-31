//! Memory-only paint input and owner observations shared by both comparison arms.
//!
//! This module does not implement a renderer or admission policy. The selected
//! head calls its existing paint path, with an optional paint-only loading input.
//! The input is restored before simulation resumes, including on unwind. The
//! fixed-layout observation owns no frame pixels, textures, strings, or vectors.
//! Its census enumerates only the owners documented by the client's common
//! `memory_case` module; it is not a complete heap or native-allocation census.

use std::cell::RefCell;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use client::client::Client;
use client::render::backend::FrameOutput;
use client::render::memory_case::Census;

use crate::render::HeadIdentity;

thread_local! {
    static ACTIVE: RefCell<Option<Arc<Control>>> = const { RefCell::new(None) };
}

/// One diagnostic allocation per observed slot, independent of drawing state.
/// Both comparison arms use this exact type and install it on their actual
/// slot thread. No allocation occurs while recording a paint observation.
#[derive(Default)]
pub struct Control {
    loading: AtomicBool,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    paints: u64,
    loading_paints: u64,
    latest: Option<Observation>,
}

/// A completed call to the arm's existing paint path. `scene_cycle` in the
/// census is the renderer's preparation counter, not a GPU completion fence.
#[derive(Clone, Copy, Debug)]
pub struct Observation {
    pub slot: u64,
    pub head_generation: u64,
    pub paints: u64,
    pub loading_paints: u64,
    pub simulation_scene_state: i32,
    pub paint_scene_state: i32,
    pub census: Census,
}

impl Control {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set_loading(&self, loading: bool) {
        self.loading.store(loading, Ordering::Release);
    }

    pub fn latest(&self) -> Option<Observation> {
        self.state.lock().unwrap().latest
    }

    /// Release only the diagnostic observation. The renderer and mailbox own
    /// their resources; this operation never detaches or drains either one.
    pub fn clear_observation(&self) {
        self.state.lock().unwrap().latest = None;
    }

    fn record(
        &self,
        identity: HeadIdentity,
        simulation_scene_state: i32,
        paint_scene_state: i32,
        loading: bool,
        census: Census,
    ) {
        let mut state = self.state.lock().unwrap();
        state.paints = state.paints.checked_add(1).expect("diagnostic paint counter exhausted");
        if loading {
            state.loading_paints = state
                .loading_paints
                .checked_add(1)
                .expect("diagnostic loading counter exhausted");
        }
        state.latest = Some(Observation {
            slot: identity.slot().incarnation().get(),
            head_generation: identity.generation().get(),
            paints: state.paints,
            loading_paints: state.loading_paints,
            simulation_scene_state,
            paint_scene_state,
            census,
        });
    }
}

/// Bind the diagnostic to the current slot thread. Repeated frame hooks do
/// no allocation or Arc reference-count update once the same binding exists.
pub fn install(control: &Arc<Control>) {
    ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        if active
            .as_ref()
            .is_none_or(|previous| !Arc::ptr_eq(previous, control))
        {
            *active = Some(Arc::clone(control));
        }
    });
}

/// Release the current thread's diagnostic reference, without touching render
/// state. The isolated allocation receipt calls this before its final sample.
pub fn clear() {
    ACTIVE.with(|active| *active.borrow_mut() = None);
}

/// Call exactly the existing paint implementation, then retain only the
/// common fixed-layout census. No frame output is cloned or retained here.
pub(crate) fn paint(
    client: &mut Client,
    identity: HeadIdentity,
    redraw: impl FnOnce(&mut Client) -> (FrameOutput, Census),
) -> FrameOutput {
    let control = ACTIVE.with(|active| active.borrow().clone());
    let Some(control) = control else {
        return redraw(client).0;
    };
    let simulation_scene_state = client.scene_state;
    let loading = client.ingame && control.loading.load(Ordering::Acquire);
    if loading {
        client.scene_state = 1;
    }
    let paint_scene_state = client.scene_state;
    let result = catch_unwind(AssertUnwindSafe(|| redraw(client)));
    client.scene_state = simulation_scene_state;
    let (output, census) = match result {
        Ok(value) => value,
        Err(payload) => resume_unwind(payload),
    };
    control.record(
        identity,
        simulation_scene_state,
        paint_scene_state,
        loading,
        census,
    );
    output
}
