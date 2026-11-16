//! The host binds the surfaces that its build contains. No surface holds a run
//! before a Start, so a slot that the host builds has no Luau runtime, and it
//! refuses a Luau card as it refuses a card of a surface that its build lacks.

use super::*;
use script::surface::{SurfaceCard, SurfaceError, SurfaceFamily, SurfaceKind, SurfaceStart};
use script::StartLoadError;

#[derive(Debug)]
struct LuauCard;

impl SurfaceCard for LuauCard {
    fn kind(&self) -> SurfaceKind {
        SurfaceKind::Luau
    }

    fn family(&self) -> Option<SurfaceFamily> {
        Some(SurfaceFamily::LuauV2)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[test]
fn a_new_slot_holds_no_luau_run_and_refuses_a_luau_card() {
    let wall: ScriptWall = Arc::default();
    let slot = script_slot_or_insert(&wall, "alice");
    let mut slot = slot.lock().expect("the slot lock is not poisoned");
    assert!(!slot.surface_active());
    assert_eq!(slot.state(), script::RunState::Idle);

    let start = SurfaceStart {
        card: Arc::new(LuauCard),
        settings_bag: None,
        game_data: None,
        named_banks: Arc::default(),
        loadouts: Arc::default(),
        wilderness_rules: script::surface::no_wilderness(),
    };
    assert_eq!(
        slot.start_surface(start),
        Err(StartLoadError::Surface(SurfaceError::NoFactory(
            SurfaceKind::Luau
        )))
    );
    assert!(!slot.surface_active());
    assert_eq!(slot.state(), script::RunState::Idle);
}
