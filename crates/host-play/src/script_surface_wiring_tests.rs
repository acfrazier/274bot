//! A new slot has no Luau runtime before Start. It rejects a foreign Luau
//! card, whether this build has a Luau factory or not.

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
    let expected = if cfg!(feature = "luau") {
        SurfaceError::ForeignCard(SurfaceKind::Luau)
    } else {
        SurfaceError::NoFactory(SurfaceKind::Luau)
    };
    assert_eq!(
        slot.start_surface(start),
        Err(StartLoadError::Surface(expected))
    );
    assert!(!slot.surface_active());
    assert_eq!(slot.state(), script::RunState::Idle);
}
