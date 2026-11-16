//! The slot keeps one factory for each surface kind and starts a card with the
//! factory of its kind, so a second surface cannot start through the first one.
#![cfg(feature = "test-hooks")]

use api::identity::ExecutionId;
use script::{
    native::recovery_hints::RecoveryHintsCell,
    surface::{
        SurfaceCard, SurfaceError, SurfaceFactory, SurfaceFamily, SurfaceKind, SurfaceRun,
        SurfaceStart,
    },
    RunState, SlotScript, StartLoadError,
};
use std::{
    any::Any,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

/// Every kind. The array length is `COUNT`, and the match in the test below is
/// exhaustive: a new kind fails to compile until it is listed here.
const ALL_KINDS: [SurfaceKind; SurfaceKind::COUNT] = [SurfaceKind::Js, SurfaceKind::Luau];

#[derive(Debug)]
struct Card(SurfaceKind);

impl SurfaceCard for Card {
    fn kind(&self) -> SurfaceKind {
        self.0
    }

    fn family(&self) -> Option<SurfaceFamily> {
        None
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A factory that counts its spawns and refuses each one, so no run is needed.
struct RefusingFactory {
    kind: SurfaceKind,
    spawns: AtomicUsize,
}

impl RefusingFactory {
    fn new(kind: SurfaceKind) -> Arc<Self> {
        Arc::new(Self {
            kind,
            spawns: AtomicUsize::new(0),
        })
    }

    fn spawns(&self) -> usize {
        self.spawns.load(Ordering::SeqCst)
    }
}

impl SurfaceFactory for RefusingFactory {
    fn kind(&self) -> SurfaceKind {
        self.kind
    }

    fn spawn(
        &self,
        _start: &SurfaceStart,
        _execution: ExecutionId,
        _recovery_hints: Arc<RecoveryHintsCell>,
    ) -> Result<Box<dyn SurfaceRun>, SurfaceError> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        Err(SurfaceError::Spawn(format!("{:?} factory", self.kind)))
    }
}

fn start(kind: SurfaceKind) -> SurfaceStart {
    SurfaceStart {
        card: Arc::new(Card(kind)),
        settings_bag: None,
        game_data: None,
        named_banks: Arc::default(),
        loadouts: Arc::default(),
        wilderness_rules: script::surface::no_wilderness(),
    }
}

fn spawn_refusal(kind: SurfaceKind) -> Result<(), StartLoadError> {
    Err(StartLoadError::Surface(SurfaceError::Spawn(format!(
        "{kind:?} factory"
    ))))
}

#[test]
fn every_kind_has_its_own_entry_in_the_factory_array() {
    let mut taken = [false; SurfaceKind::COUNT];
    for kind in ALL_KINDS {
        match kind {
            SurfaceKind::Js | SurfaceKind::Luau => {}
        }
        let index = kind.index();
        assert!(index < SurfaceKind::COUNT, "{kind:?} is outside the array");
        assert!(
            !std::mem::replace(&mut taken[index], true),
            "{kind:?} shares an entry"
        );
    }
}

#[test]
fn a_card_starts_through_the_factory_of_its_kind_only() {
    let js = RefusingFactory::new(SurfaceKind::Js);
    let luau = RefusingFactory::new(SurfaceKind::Luau);
    let mut slot = SlotScript::new();

    // With only the JS factory bound, a Luau card has no factory and the JS
    // factory is not asked in its place.
    slot.bind_surface_factory(js.clone());
    assert_eq!(
        slot.start_surface(start(SurfaceKind::Luau)),
        Err(StartLoadError::Surface(SurfaceError::NoFactory(
            SurfaceKind::Luau
        )))
    );
    assert_eq!((js.spawns(), luau.spawns()), (0, 0));
    assert_eq!(slot.state(), RunState::Idle);
    assert!(!slot.surface_active());

    assert_eq!(
        slot.start_surface(start(SurfaceKind::Js)),
        spawn_refusal(SurfaceKind::Js)
    );
    assert_eq!((js.spawns(), luau.spawns()), (1, 0));

    // Binding the Luau factory serves Luau cards and leaves the JS entry alone.
    slot.bind_surface_factory(luau.clone());
    assert_eq!(
        slot.start_surface(start(SurfaceKind::Luau)),
        spawn_refusal(SurfaceKind::Luau)
    );
    assert_eq!((js.spawns(), luau.spawns()), (1, 1));
    assert_eq!(
        slot.start_surface(start(SurfaceKind::Js)),
        spawn_refusal(SurfaceKind::Js)
    );
    assert_eq!((js.spawns(), luau.spawns()), (2, 1));
}
