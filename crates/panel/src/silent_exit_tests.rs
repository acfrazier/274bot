//! PANEL-STATE-GAP-1023: panel click handlers that could exit early without
//! submitting a command and without the shared refusal writer
//! (`OperatorSession::record_refusal`). A control drawn disabled cannot be
//! clicked and records nothing, so each such handler either has its missing
//! precondition read where its control is drawn, from the one function the
//! handler takes its target from, or cannot be reached by a click at all.
//!
//! The controls are drawn in real Dear ImGui frames and pressed through
//! ImGui's own item activation (`ActivateItemByID`), which presses an enabled
//! item as a click does and never a disabled one. ImGui makes a pressed button
//! its active item for that frame; a disabled button never becomes it. Every
//! "disabled" assertion sits beside an "enabled" one for the same control
//! (or for a control that is always enabled), so a wrong item id cannot pass
//! a test for the wrong reason.

use std::ffi::CString;
use std::sync::{Arc, MutexGuard};

use api::loadout::Loadout;
use dear_imgui_rs::{sys, Condition, Context, FramePrepareOptions, Ui};
use frontend_core::launch::ProfileTemplate;
use frontend_core::scripts::{LoadoutForm, Loadouts, ScriptRunState, WornSlot};
use frontend_core::test_support::session::{install_vault, with_tutorial_skipped};
use frontend_core::test_support::{FixtureProfile, VaultFixture};

use super::tests::map_fixture::MapFixture;
use super::{script_pause_enabled, script_stop_enabled, PanelWindow, Session};
use crate::test_support::{save_store_patiently, scratch_service, TestDir};

const HARNESS: &str = "Silent exits harness";
const DISPLAY: [f32; 2] = [1440.0, 900.0];

fn empty_play() -> host_play::Play {
    frontend_core::test_support::empty_play(&host_play::PlayOptions {
        host: "127.0.0.1".into(),
        transport: client::Transport::Tcp,
        port: 43594,
        cache_dir: "/tmp".into(),
        lowmem: true,
        mainland: false,
    })
}

/// Bind a Local server profile and nothing else: the panel at start-up, before
/// the vault is unlocked (no focused bot, no play session). Debug draws only
/// on a Local profile.
fn bind_local_profile(session: &mut Session) -> MapFixture {
    let world = nav_planner::world::NavWorld::from_parts(
        nav_planner::collision::WorldCollision {
            origin: api::snapshot::WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 3,
            height: 3,
            walk: vec![0u8; 9],
            blocked: vec![0u64; 1],
            flags: None,
        },
        nav_planner::transport::TransportGraph::default(),
        Vec::new(),
    );
    let fixture = MapFixture::new(&world, "local-289");
    session
        .launcher
        .install_template(ProfileTemplate::from_host(Arc::clone(&fixture.template)));
    fixture
}

/// A vault holding `alice`, whose TutSkip reads "still in the tutorial".
fn alice_vault(dir: &TestDir) -> VaultFixture {
    let mut vault = VaultFixture::create(&dir.join("vault"), "test-passphrase-01");
    vault.put(with_tutorial_skipped(
        FixtureProfile::new("alice", "pw").uid(42),
        Some(false),
    ));
    vault
}

/// The id ImGui gives the top-level item `label` of the window `window`. Call
/// with the frame's context bound, after the window drew.
fn item_id(window: &str, label: &str) -> sys::ImGuiID {
    let window = CString::new(window).unwrap();
    let label = CString::new(label).unwrap();
    // SAFETY: the caller binds the frame's context; both names are
    // NUL-terminated copies that outlive the calls, and the window is read
    // only after the lookup found it.
    unsafe {
        let found = sys::igFindWindowByName(window.as_ptr());
        assert!(!found.is_null(), "{window:?} was drawn");
        sys::igGetIDWithSeed_Str(label.as_ptr(), std::ptr::null(), (*found).ID)
    }
}

/// Queue ImGui's own activation of the item `id`: it is pressed on the next
/// frame as a click presses it, unless the item is disabled. Call with the
/// frame's context bound.
fn queue_press(id: sys::ImGuiID) {
    // SAFETY: the caller binds the frame's context.
    unsafe { sys::igActivateItemByID(id) };
}

/// The id of the item ImGui holds active at this point of the frame: the
/// button a press just took (ImGui holds it for the whole press frame), else
/// none. A disabled item is never made active. Read through ImGui's own
/// accessor: reading the generated context struct's fields directly gave
/// values ImGui never stores. Call with the frame's context bound.
fn active_id() -> sys::ImGuiID {
    // SAFETY: the caller binds the frame's context.
    unsafe { sys::igGetActiveID() }
}

/// One section of the strip drawn through real Dear ImGui frames, alone in a
/// window of its own.
struct Rig {
    ctx: Context,
    session: Session,
    draw: fn(&Ui, &mut Session),
    _dir: TestDir,
    _fixture: MapFixture,
    // Last: the context singleton drops before the guard.
    _guard: MutexGuard<'static, ()>,
}

impl Rig {
    fn new(
        session: Session,
        dir: TestDir,
        fixture: MapFixture,
        draw: fn(&Ui, &mut Session),
    ) -> Self {
        let guard = crate::test_support::imgui_context_guard();
        let mut rig = Self {
            ctx: crate::test_support::imgui_context(),
            session,
            draw,
            _dir: dir,
            _fixture: fixture,
            _guard: guard,
        };
        rig.session.persist_ui = false;
        rig.session.core.set_spawn_workers(false);
        for _ in 0..3 {
            rig.frame(None);
        }
        rig
    }

    /// One real frame with the section in the window. `queue` names an item
    /// whose press is queued for the next frame. Returns that item's id and
    /// the id ImGui holds active at the end of this frame.
    fn frame(&mut self, queue: Option<&str>) -> (Option<sys::ImGuiID>, sys::ImGuiID) {
        self.ctx
            .prepare_frame(FramePrepareOptions::new(DISPLAY, 1.0 / 60.0).renderer_has_textures());
        let seen = {
            let ui = self.ctx.frame();
            let draw = self.draw;
            let session = &mut self.session;
            let mut queued = None;
            ui.window(HARNESS)
                .position([20.0, 20.0], Condition::Always)
                .size([600.0, 860.0], Condition::Always)
                .build(|| {
                    queued = queue.map(|label| ui.get_id(label).raw());
                    draw(ui, session);
                });
            if let Some(id) = queued {
                ui.with_bound_context(|| queue_press(id));
            }
            (queued, ui.with_bound_context(active_id))
        };
        self.ctx.render();
        seen
    }

    /// Whether ImGui takes a press on the item `label`: the press is queued in
    /// one frame and happens in the next.
    fn accepts(&mut self, label: &str) -> bool {
        let (id, _) = self.frame(Some(label));
        let id = id.expect("the item was laid out in the window");
        let (_, active) = self.frame(None);
        active == id
    }
}

fn debug_buttons(ui: &Ui, session: &mut Session) {
    crate::app::draw_test_debug_section(ui, session);
}

/// P-S2, P-S8: with a Local profile bound and no bot focused (start-up, or an
/// unlocked vault with nothing picked), the Lumbridge and maxme buttons are
/// disabled, as the DebugPanel button beside them is not. Before the fix they
/// were enabled, and a click returned without submitting a command, logging a
/// refusal or telling the operator anything.
#[test]
fn the_rail_cheats_are_disabled_with_no_focused_bot() {
    let dir = TestDir::new("silent-exits-no-focus");
    let vault = alice_vault(&dir);
    let mut session = Session::new();
    let fixture = bind_local_profile(&mut session);
    let mut rig = Rig::new(session, dir, fixture, debug_buttons);

    // Start-up, before the vault is unlocked.
    assert!(rig.accepts("Panel"), "DebugPanel is always enabled");
    assert!(!rig.accepts("Lumb"), "Lumbridge needs a focused bot");
    assert!(!rig.accepts("maxme"), "maxme needs a bot to send to");

    // Unlocked, with a play session, and still nothing is focused.
    install_vault(&mut rig.session.core, vault);
    rig.session.core.set_play(Some(empty_play()));
    assert!(rig.accepts("Panel"), "DebugPanel is always enabled");
    assert!(!rig.accepts("Lumb"), "Lumbridge needs a focused bot");
    assert!(!rig.accepts("maxme"), "maxme needs a bot to send to");
}

/// P-S3, P-S8: a focused bot is not enough: with no play session the TutSkip,
/// Lumbridge and maxme buttons are disabled too, and all three work once a
/// play session exists. TutSkip is drawn only for a focused bot, so this is
/// the state in which it is drawn at all.
#[test]
fn the_rail_cheats_are_disabled_without_a_play_session_and_work_with_one() {
    let dir = TestDir::new("silent-exits-no-play");
    let vault = alice_vault(&dir);
    let mut session = Session::new();
    let fixture = bind_local_profile(&mut session);
    install_vault(&mut session.core, vault);
    session.set_focus_for_test("alice");
    assert_eq!(session.focused_name().as_deref(), Some("alice"));
    assert_eq!(session.focused_tutorial_skipped(), Some(false));
    let mut rig = Rig::new(session, dir, fixture, debug_buttons);

    assert!(rig.accepts("Panel"), "DebugPanel is always enabled");
    for label in ["TutSkip", "Lumb", "maxme"] {
        assert!(!rig.accepts(label), "{label} needs a play session");
    }

    rig.session.core.set_play(Some(empty_play()));
    assert!(rig.accepts("Panel"), "DebugPanel is always enabled");
    // TutSkip persists its flag and so leaves the row: it goes before maxme.
    for label in ["Lumb", "TutSkip", "maxme"] {
        assert!(rig.accepts(label), "{label} works with a play session");
    }
}

/// P-S9a-c: the one function that gives a Debug command its bot names what is
/// missing, in the order the old early returns checked it, and the focused
/// send and the Debug panel's own predicate agree with it. The panel's Send
/// and the maxme button draw disabled while it has a reason; the handlers
/// take their bot from it.
#[test]
fn a_debug_command_names_what_its_target_is_missing() {
    // Not a Local profile.
    {
        let remote = Session::new();
        let not_local = "Debug commands require a Local profile";
        assert_eq!(remote.debug_send_target(), Err(not_local));
        assert_eq!(crate::debug_panel::send_block(&remote), Some(not_local));
    }

    // A Local profile with nothing focused, then a focused bot with no play
    // session, then both.
    let dir = TestDir::new("silent-exits-target");
    let vault = alice_vault(&dir);
    let mut session = Session::new();
    let _fixture = bind_local_profile(&mut session);
    assert_eq!(session.debug_send_target(), Err("Pick a focused bot"));
    assert_eq!(
        session.send_debug_command("getcoord"),
        Err("Pick a focused bot".to_string())
    );
    assert_eq!(
        crate::debug_panel::send_block(&session),
        Some("Pick a focused bot")
    );

    install_vault(&mut session.core, vault);
    session.set_focus_for_test("alice");
    assert_eq!(session.debug_send_target(), Err("No active play session"));
    assert_eq!(
        session.send_debug_command("getcoord"),
        Err("No active play session".to_string())
    );
    assert_eq!(
        crate::debug_panel::send_block(&session),
        Some("No active play session")
    );

    session.core.set_play(Some(empty_play()));
    assert_eq!(session.debug_send_target(), Ok("alice"));
    assert_eq!(crate::debug_panel::send_block(&session), None);
}

/// One frame of the Loadouts window. `queue` names an item of that window
/// whose press is queued for the next frame. Returns that item's id and the
/// id ImGui holds active at the end of this frame.
fn loadouts_frame(
    ctx: &mut Context,
    session: &mut Session,
    queue: Option<&str>,
) -> (Option<sys::ImGuiID>, sys::ImGuiID) {
    ctx.prepare_frame(FramePrepareOptions::new(DISPLAY, 1.0 / 60.0).renderer_has_textures());
    let seen = {
        let ui = ctx.frame();
        let id = queue.map(|label| {
            let window = PanelWindow::Loadouts.title();
            ui.with_bound_context(|| item_id(window, label))
        });
        if let Some(id) = id {
            ui.with_bound_context(|| queue_press(id));
        }
        crate::loadouts::window(ui, session);
        (id, ui.with_bound_context(active_id))
    };
    ctx.render();
    seen
}

/// P-S10: the equipment slot buttons are the only way into the item search
/// that edits the worn gear, and `assign_slot` returned without a word when
/// there was no draft (no loadout exists). The slot buttons and Add supply are
/// disabled until there is one. A slot button that is pressed opens its item
/// search, so a pressed slot is seen as the search's slot; Add supply opens
/// nothing, so ImGui's active item shows it.
#[test]
fn a_loadout_slot_and_add_supply_are_disabled_without_a_draft() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ctx = crate::test_support::imgui_context();
    let empty_dir = TestDir::new("silent-exits-loadouts-empty");
    let row_dir = TestDir::new("silent-exits-loadouts-row");
    let mut session = Session::new();
    session.persist_ui = false;
    // A loadout set with no row: nothing to edit, so no draft.
    session.loadouts = LoadoutForm::over(Loadouts::operator(&scratch_service(&empty_dir)));
    session.loadouts_open = true;
    assert!(session.loadouts.draft().is_none());
    for _ in 0..3 {
        loadouts_frame(&mut ctx, &mut session, None);
    }
    let slot = WornSlot::Hat;
    let slot_button = format!("{}\n—##slot-{}", slot.label(), slot.id());

    loadouts_frame(&mut ctx, &mut session, Some(slot_button.as_str()));
    loadouts_frame(&mut ctx, &mut session, None);
    assert!(
        session.loadouts_search_slot.is_none(),
        "a slot button without a draft opens no item search"
    );
    let (id, _) = loadouts_frame(&mut ctx, &mut session, Some("Add supply"));
    let (_, active) = loadouts_frame(&mut ctx, &mut session, None);
    assert_ne!(Some(active), id, "Add supply without a draft is disabled");

    // One saved loadout: the window selects it, and both controls work.
    let mut store = Loadouts::operator(&scratch_service(&row_dir));
    store.upsert_for_test(Loadout::new("melee").with_carry("Lobster", 8));
    save_store_patiently(&mut store);
    session.loadouts = LoadoutForm::over(store);
    crate::loadouts::select(&mut session, 0);
    assert!(session.loadouts.draft().is_some());
    for _ in 0..3 {
        loadouts_frame(&mut ctx, &mut session, None);
    }

    let (id, _) = loadouts_frame(&mut ctx, &mut session, Some("Add supply"));
    let (_, active) = loadouts_frame(&mut ctx, &mut session, None);
    assert_eq!(Some(active), id, "Add supply works with a draft");
    loadouts_frame(&mut ctx, &mut session, Some(slot_button.as_str()));
    loadouts_frame(&mut ctx, &mut session, None);
    assert!(
        matches!(session.loadouts_search_slot, Some(WornSlot::Hat)),
        "a slot button with a draft opens the item search for that slot"
    );
}

/// P-S4 to P-S7 and P-S12: the controls whose handlers return early without a
/// focused bot are not reachable by a click. The script section's Pause and
/// Stop read the focused bot's script state, which is `Idle` with no focus
/// and with no play session, and both buttons draw disabled in `Idle` (Stop is
/// also enabled for a Start queued for the selected bot, which needs a
/// focus). TutSkip is drawn only for a focused profile, and the Relog-now
/// button only for a focused bot with a memory notice.
#[test]
fn pause_stop_tutskip_and_relog_are_not_clickable_without_a_focused_bot_in_play() {
    let mut session = Session::new();
    assert_eq!(session.focused_script_state(), ScriptRunState::Idle);
    assert!(!script_pause_enabled(session.focused_script_state()));
    assert!(!script_stop_enabled(session.focused_script_state()));
    assert_eq!(session.focused_tutorial_skipped(), None);
    assert!(session.focused_memory_notice().is_none());

    // A focused bot with no play session has no script state either.
    let dir = TestDir::new("silent-exits-pins");
    install_vault(&mut session.core, alice_vault(&dir));
    session.set_focus_for_test("alice");
    assert_eq!(session.focused_name().as_deref(), Some("alice"));
    assert_eq!(session.focused_script_state(), ScriptRunState::Idle);
    assert!(!script_pause_enabled(session.focused_script_state()));
    assert!(!script_stop_enabled(session.focused_script_state()));
    assert!(session.focused_memory_notice().is_none());
}
