//! Real ImGui frames for the window openers and the MultiBox unlock
//! sequence: an opener brings its already-open window forward (once), and
//! none of these paths starts a script.

use std::ffi::CString;
use std::sync::{Arc, Mutex};

use dear_imgui_rs::{sys, Condition, Context, FramePrepareOptions, MouseButton, Ui};

use super::PanelState;
use crate::session::{PanelWindow, Session};
use crate::test_support::TestDir;

use host_play as map_host;

// The shared host-play fixture (also loaded by `session_tests`), for a
// Local server profile: Debug draws only on one.
#[path = "../../host-play/tests/support/map_fixture.rs"]
#[allow(dead_code, clippy::duplicate_mod)]
mod map_fixture;

const OPENER_HOST: &str = "Opener harness";
const DISPLAY: [f32; 2] = [1440.0, 900.0];

fn panel_state(session: Session) -> PanelState {
    PanelState::with_session(
        session,
        Arc::new(Mutex::new(crate::window::ShotState::default())),
    )
}

fn context() -> Context {
    let mut ctx = Context::create();
    ctx.set_ini_filename::<std::path::PathBuf>(None).unwrap();
    ctx
}

/// Of the windows drawn this frame, the one ImGui focused last (the front
/// of its focus order, which every focus change goes through). Call with
/// the frame's context bound, after the windows drew.
fn focused_root() -> Option<String> {
    const HARNESS: [&str; 3] = [OPENER_HOST, "Rail harness", "Game harness"];
    let names = PanelWindow::ALL.map(PanelWindow::title);
    names
        .into_iter()
        .chain(HARNESS)
        .filter_map(|name| {
            let c_name = CString::new(name).unwrap();
            // SAFETY: the caller binds the frame's context; the name is a
            // NUL-terminated copy that outlives the call, and a window is
            // read only after the lookup found it.
            unsafe {
                let window = sys::igFindWindowByName(c_name.as_ptr());
                (!window.is_null() && (*window).Active).then(|| ((*window).FocusOrder, name))
            }
        })
        .max_by_key(|&(order, _)| order)
        .map(|(_, name)| name.to_string())
}

/// Queue ImGui's own activation of `label` in `window` (the id ImGui gives
/// an item at the window's top level): it is pressed on the next frame as a
/// click presses it. Call with the frame's context bound.
fn activate(window: &str, label: &str) {
    // SAFETY: the caller binds the frame's context; both names are
    // NUL-terminated copies that outlive the calls, and the window is read
    // only after the lookup found it.
    unsafe {
        let name = CString::new(window).unwrap();
        let found = sys::igFindWindowByName(name.as_ptr());
        assert!(!found.is_null(), "{window:?} was drawn");
        let label = CString::new(label).unwrap();
        sys::igActivateItemByID(sys::igGetIDWithSeed_Str(
            label.as_ptr(),
            std::ptr::null(),
            (*found).ID,
        ));
    }
}

/// [`activate`] for a row of the Profiles list (its own child window).
fn activate_profiles_row(name: &str) {
    // SAFETY: as in `activate`.
    unsafe {
        let profiles = sys::igFindWindowByName(c"Profiles".as_ptr());
        assert!(!profiles.is_null(), "Profiles was drawn");
        let list_id = sys::igGetIDWithSeed_Str(
            c"##profiles-list".as_ptr(),
            std::ptr::null(),
            (*profiles).ID,
        );
        activate(&format!("Profiles/##profiles-list_{list_id:08X}"), name);
    }
}

/// Every opener target, in the app's per-frame order; each returns early
/// while closed. WalkTo draws inside a Game pane as in production.
fn target_windows(ui: &Ui, state: &mut PanelState) {
    if state.session.walkto_open {
        ui.window("Game harness")
            .position([700.0, 300.0], Condition::Always)
            .size([600.0, 500.0], Condition::Always)
            .build(|| {
                crate::picker::draw_picker(ui, None, &mut state.session, &mut state.walk_map)
            });
    }
    super::chooser_window(ui, &mut state.session, None);
    super::settings_window(ui, &mut state.session, None);
    super::debug_panel_window(ui, &mut state.session, None);
    super::browse_window(ui, &mut state.session);
    super::nav_settings_window(ui, &mut state.session, None);
    super::script_prefs_window(ui, &mut state.session, None);
    crate::loadouts::window(ui, &mut state.session);
    super::log_window_with_body(ui, &mut state.session, None, |ui, session| {
        crate::log_pane::log_body(ui, session, true)
    });
    crate::fleet::window(ui, &mut state.session);
}

/// What one harness frame does before drawing.
#[derive(Clone, Copy, Default)]
struct FrameInput<'a> {
    /// Activate `(window, label)` (pressed next frame).
    press: Option<(&'a str, &'a str)>,
    /// Activate this Profiles list row.
    row: Option<&'a str>,
    /// Give this window the focus before it draws, as the operator's
    /// click into it does.
    focus: Option<&'a str>,
}

/// One real frame: the target windows, then the opener harness window
/// with `opener` drawn in it. Returns the focused root window afterwards.
fn frame(
    ctx: &mut Context,
    state: &mut PanelState,
    opener: fn(&Ui, &mut PanelState),
    input: FrameInput,
) -> Option<String> {
    ctx.prepare_frame(FramePrepareOptions::new(DISPLAY, 1.0 / 60.0).renderer_has_textures());
    let focused = {
        let ui = ctx.frame();
        if let Some((window, label)) = input.press {
            ui.with_bound_context(|| activate(window, label));
        }
        if let Some(row) = input.row {
            ui.with_bound_context(|| activate_profiles_row(row));
        }
        if let Some(window) = input.focus {
            ui.set_window_focus(Some(window));
        }
        target_windows(ui, state);
        ui.window(OPENER_HOST)
            .position([20.0, 20.0], Condition::Always)
            .size([380.0, 860.0], Condition::Always)
            .build(|| opener(ui, state));
        ui.with_bound_context(focused_root)
    };
    ctx.render();
    focused
}

fn frames(
    ctx: &mut Context,
    state: &mut PanelState,
    opener: fn(&Ui, &mut PanelState),
    n: usize,
) -> Option<String> {
    let mut focused = None;
    for _ in 0..n {
        focused = frame(ctx, state, opener, FrameInput::default());
    }
    focused
}

fn assert_no_start(session: &Session, step: &str) {
    if let Some(op) = session.core.last_operation() {
        assert_ne!(
            op.action,
            frontend_core::ActionKind::ScriptStart,
            "{step}: nothing may start a script"
        );
    }
    if let Some(play) = session.core.play() {
        for name in session.core.slots().keys() {
            assert_eq!(
                play.script_state(name),
                script::RunState::Idle,
                "{step}: {name}'s script stays idle"
            );
        }
    }
}

/// How a case presses its opener once its window is already open.
#[derive(Clone, Copy)]
enum Press {
    /// The real button `label` in window `host` (activated by ImGui).
    Button {
        host: &'static str,
        label: &'static str,
    },
    /// The opener's own operation: its button needs a fixture this unit
    /// test cannot build (a script schema, an rs2b0t defer file).
    Op(fn(&mut Session)),
}

struct OpenerCase {
    name: &'static str,
    target: PanelWindow,
    /// Returns a fixture the case keeps alive (the Local server profile).
    setup: fn(&mut PanelState) -> Option<map_fixture::MapFixture>,
    opener: fn(&Ui, &mut PanelState),
    press: Press,
}

fn no_setup(_: &mut PanelState) -> Option<map_fixture::MapFixture> {
    None
}
fn no_opener(_: &Ui, _: &mut PanelState) {}

fn add_vault(state: &mut PanelState) {
    let dir = TestDir::new("opener-focus-vault");
    let mut vault = vault::Vault::create(&dir.join("vault"), "test-passphrase-01").unwrap();
    for (i, name) in ["alice", "bob"].into_iter().enumerate() {
        vault
            .upsert(vault::Profile {
                username: name.into(),
                password: "pw".into(),
                uid: 1 + i as i32,
                settings: vault::ProfileSettings::default(),
            })
            .unwrap();
    }
    state.session.core.set_vault(Some(vault));
}

fn with_vault(state: &mut PanelState) -> Option<map_fixture::MapFixture> {
    add_vault(state);
    None
}

fn with_marked_bob(state: &mut PanelState) -> Option<map_fixture::MapFixture> {
    add_vault(state);
    let bob = state.session.core.profile_identity("bob").unwrap();
    state.session.fleet_selection.set(bob, true);
    state.session.open_window(PanelWindow::Fleet);
    None
}

fn with_script_section(state: &mut PanelState) -> Option<map_fixture::MapFixture> {
    // A filled catalog: Browse/Load never raise the first-use import prompt.
    state.session.scripts.mark_catalog_filled();
    state
        .session
        .ui
        .collapsed
        .entry("_".into())
        .or_default()
        .insert("script".into(), false);
    None
}

fn with_local_profile(state: &mut PanelState) -> Option<map_fixture::MapFixture> {
    let world = nav::world::NavWorld::from_parts(
        nav::collision::WorldCollision {
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
        nav::transport::TransportGraph::default(),
        Vec::new(),
    );
    let fixture = map_fixture::MapFixture::new(&world, "local-289");
    state.session.server_profile = Some(Arc::clone(fixture.template.profile()));
    Some(fixture)
}

fn profile_section(ui: &Ui, state: &mut PanelState) {
    super::profile_section(ui, &mut state.session);
}
fn rail_profiles(ui: &Ui, state: &mut PanelState) {
    super::add_bot_button(ui, state);
}
fn config_row(ui: &Ui, state: &mut PanelState) {
    super::slot_config_row(ui, &mut state.session);
}
fn title_row(ui: &Ui, state: &mut PanelState) {
    super::title_row(ui, &mut state.session);
}
fn action_row(ui: &Ui, state: &mut PanelState) {
    super::walkto_button(ui, &mut state.session);
}
fn debug_section(ui: &Ui, state: &mut PanelState) {
    super::debug_section(ui, &mut state.session);
}
fn script_section(ui: &Ui, state: &mut PanelState) {
    super::script_section(ui, &mut state.session);
}

fn opener_cases() -> Vec<OpenerCase> {
    let fleet = PanelWindow::Fleet.title();
    let button = |label| Press::Button {
        host: OPENER_HOST,
        label,
    };
    vec![
        OpenerCase {
            name: "Profiles",
            target: PanelWindow::Profiles,
            setup: with_vault,
            opener: profile_section,
            press: button("Profiles"),
        },
        OpenerCase {
            name: "rail Profiles…",
            target: PanelWindow::Profiles,
            setup: with_vault,
            opener: rail_profiles,
            press: button("Profiles…"),
        },
        OpenerCase {
            name: "General config",
            target: PanelWindow::GeneralConfig,
            setup: no_setup,
            opener: config_row,
            press: button("General config"),
        },
        OpenerCase {
            name: "Nav config",
            target: PanelWindow::NavConfig,
            setup: no_setup,
            opener: config_row,
            press: button("Nav config"),
        },
        OpenerCase {
            name: "Loadouts",
            target: PanelWindow::Loadouts,
            setup: no_setup,
            opener: config_row,
            press: button("Loadouts"),
        },
        OpenerCase {
            name: "Script prefs",
            target: PanelWindow::ScriptPrefs,
            setup: no_setup,
            opener: no_opener,
            press: Press::Op(|s| {
                s.open_window(PanelWindow::ScriptPrefs);
            }),
        },
        OpenerCase {
            name: "Log",
            target: PanelWindow::Log,
            setup: no_setup,
            opener: title_row,
            press: button("Log"),
        },
        OpenerCase {
            name: "Fleet",
            target: PanelWindow::Fleet,
            setup: no_setup,
            opener: action_row,
            press: button("Fleet"),
        },
        OpenerCase {
            name: "Debug Panel",
            target: PanelWindow::Debug,
            setup: with_local_profile,
            opener: debug_section,
            press: button(super::debug_caption("DebugPanel")),
        },
        OpenerCase {
            name: "maxme",
            target: PanelWindow::Debug,
            setup: with_local_profile,
            opener: debug_section,
            press: button("maxme"),
        },
        OpenerCase {
            name: "Browse…",
            target: PanelWindow::Browse,
            setup: with_script_section,
            opener: script_section,
            press: button("Browse…"),
        },
        OpenerCase {
            name: "Load",
            target: PanelWindow::LoadScript,
            setup: with_script_section,
            opener: script_section,
            press: button("Load"),
        },
        OpenerCase {
            name: "Import catalog…",
            target: PanelWindow::ImportCatalog,
            setup: no_setup,
            opener: no_opener,
            press: Press::Op(|s| s.open_rs2b0t_catalog_picker(false)),
        },
        OpenerCase {
            name: "Fleet Walk 1 to...",
            target: PanelWindow::WalkTo,
            setup: with_marked_bob,
            opener: no_opener,
            press: Press::Button {
                host: fleet,
                label: "Walk 1 to...",
            },
        },
    ]
}

/// The window an operator clicks to press the case's opener.
fn opener_host(case: &OpenerCase) -> &'static str {
    match case.press {
        Press::Button { host, .. } => host,
        Press::Op(_) => OPENER_HOST,
    }
}

#[test]
fn each_opener_focuses_its_window_when_it_is_already_open() {
    // Test lock order: ImGui, then nav statics (WalkTo reads the pack).
    let _guard = crate::test_support::imgui_context_guard();
    let _nav = crate::picker::lock_nav_statics();
    let covered: Vec<_> = opener_cases().iter().map(|case| case.target).collect();
    for window in PanelWindow::ALL {
        assert!(covered.contains(&window), "{window:?} has an opener case");
    }
    for case in opener_cases() {
        let name = case.name;
        let mut ctx = context();
        let mut state = panel_state(Session::new());
        state.session.persist_ui = false;
        let _fixture = (case.setup)(&mut state);
        frames(&mut ctx, &mut state, case.opener, 2);

        // Open the target the ordinary way; it appears and has the focus.
        match case.target {
            PanelWindow::WalkTo => state.session.open_walkto_for_marked(),
            PanelWindow::LoadScript => state.session.open_script_load_browser(),
            PanelWindow::ImportCatalog => state.session.open_rs2b0t_catalog_picker(false),
            target => {
                state.session.open_window(target);
            }
        }
        let title = case.target.title();
        let focused = frames(&mut ctx, &mut state, case.opener, 3);
        assert_eq!(focused.as_deref(), Some(title), "{name}: opened");

        // The operator works elsewhere: the target stays open behind.
        let host = opener_host(&case);
        let input = FrameInput {
            focus: Some(host),
            ..FrameInput::default()
        };
        frame(&mut ctx, &mut state, case.opener, input);
        let focused = frames(&mut ctx, &mut state, case.opener, 2);
        assert_eq!(
            focused.as_deref(),
            Some(host),
            "{name}: the opener has the focus"
        );
        assert!(
            state.session.window_focus.is_none(),
            "{name}: no request left"
        );
        // Dialog and draft state an already-open window must keep.
        state.session.loadouts_search = "keep".into();
        state.session.script_dialog_search = "keep".into();

        match case.press {
            Press::Button { host, label } => {
                let input = FrameInput {
                    press: Some((host, label)),
                    ..FrameInput::default()
                };
                frame(&mut ctx, &mut state, case.opener, input);
            }
            Press::Op(op) => op(&mut state.session),
        }
        let focused = frames(&mut ctx, &mut state, case.opener, 3);
        assert_eq!(
            focused.as_deref(),
            Some(title),
            "{name}: the already-open window comes forward"
        );
        assert!(
            state.session.window_focus.is_none(),
            "{name}: focus applied once"
        );
        assert_eq!(
            state.session.loadouts_search, "keep",
            "{name}: Loadouts draft kept"
        );
        assert_eq!(
            state.session.script_dialog_search, "keep",
            "{name}: dialog search kept"
        );
        assert_no_start(&state.session, name);

        // One-shot: once the operator moves on, the window does not take
        // the focus back.
        frame(&mut ctx, &mut state, case.opener, input);
        let focused = frames(&mut ctx, &mut state, case.opener, 3);
        assert_eq!(
            focused.as_deref(),
            Some(host),
            "{name}: focus is not re-requested"
        );
    }
}

/// The rail's Profiles… button, clicked with the mouse: a closed Profiles
/// opens, an already-open one comes forward over the rail, and a closed one
/// reopens.
#[test]
fn rail_profiles_button_opens_focuses_and_reopens_profiles() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut ctx = context();
    let mut state = panel_state(Session::new());
    state.session.persist_ui = false;
    let rail_frame = |ctx: &mut Context, state: &mut PanelState| -> ([f32; 2], Option<String>) {
        ctx.prepare_frame(FramePrepareOptions::new(DISPLAY, 1.0 / 60.0).renderer_has_textures());
        let result = {
            let ui = ctx.frame();
            let button = ui
                .window("Rail harness")
                .position([20.0, 20.0], Condition::Always)
                .size([264.0, 200.0], Condition::Always)
                .build(|| {
                    super::add_bot_button(ui, state);
                    let (min, max) = (ui.item_rect_min(), ui.item_rect_max());
                    [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5]
                })
                .unwrap();
            ui.set_window_pos_by_name("Profiles", [400.0, 20.0]);
            super::chooser_window(ui, &mut state.session, None);
            (button, ui.with_bound_context(focused_root))
        };
        ctx.render();
        result
    };
    let click = |ctx: &mut Context, state: &mut PanelState| -> Option<String> {
        let (button, _) = rail_frame(ctx, state);
        ctx.io_mut().add_mouse_pos_event(button);
        rail_frame(ctx, state);
        ctx.io_mut().add_mouse_button_event(MouseButton::Left, true);
        rail_frame(ctx, state);
        ctx.io_mut()
            .add_mouse_button_event(MouseButton::Left, false);
        rail_frame(ctx, state);
        rail_frame(ctx, state).1
    };
    rail_frame(&mut ctx, &mut state);
    rail_frame(&mut ctx, &mut state);
    assert!(!state.session.wall.chooser_open);

    assert_eq!(click(&mut ctx, &mut state).as_deref(), Some("Profiles"));
    assert!(state.session.wall.chooser_open, "a closed Profiles opens");

    // Clicking into the rail takes the focus off Profiles first.
    let (button, _) = rail_frame(&mut ctx, &mut state);
    ctx.io_mut()
        .add_mouse_pos_event([button[0], button[1] + 60.0]);
    rail_frame(&mut ctx, &mut state);
    ctx.io_mut().add_mouse_button_event(MouseButton::Left, true);
    rail_frame(&mut ctx, &mut state);
    ctx.io_mut()
        .add_mouse_button_event(MouseButton::Left, false);
    let (_, focused) = rail_frame(&mut ctx, &mut state);
    assert_eq!(focused.as_deref(), Some("Rail harness"));

    assert_eq!(
        click(&mut ctx, &mut state).as_deref(),
        Some("Profiles"),
        "an already-open Profiles comes forward over the rail"
    );
    assert!(state.session.wall.chooser_open, "the button never toggles");

    state.session.wall.chooser_open = false;
    rail_frame(&mut ctx, &mut state);
    click(&mut ctx, &mut state);
    assert!(state.session.wall.chooser_open, "close then click reopens");
}

/// The operator's sequence with real ImGui input: MultiBox while the vault
/// is locked, unlock (auto-login on both profiles), then pick the other
/// profile and back. Both bots join the rail, the Game view and the script
/// card follow the pick, and nothing starts a script.
#[test]
fn multibox_before_unlock_then_picks_keep_both_on_the_rail_without_a_start() {
    let _guard = crate::test_support::imgui_context_guard();
    let dir = TestDir::new("multibox-unlock-real-input");
    let path = dir.join("vault");
    let mut vault = vault::Vault::create(&path, "test-passphrase-01").unwrap();
    for (uid, (name, kind, card)) in [
        ("alice", "catalog", "Thiever"),
        ("bob", "compiled", "Gatherer"),
    ]
    .into_iter()
    .enumerate()
    {
        vault
            .upsert(vault::Profile {
                username: name.into(),
                password: "pw".into(),
                uid: uid as i32 + 1,
                settings: vault::ProfileSettings {
                    auto_login: true,
                    script_assignment: Some(vault::ScriptAssignment {
                        source_kind: kind.into(),
                        identity: card.into(),
                        display_name: card.into(),
                        unavailable: None,
                    }),
                    ..vault::ProfileSettings::default()
                },
            })
            .unwrap();
    }
    drop(vault);
    crate::ui_state::save(&crate::ui_state::PanelUiState {
        last_focus: Some("alice".into()),
        ..Default::default()
    });
    let mut session = Session::new();
    session.persist_ui = false;
    session.core.set_spawn_workers(false);
    let mut state = panel_state(session);
    let mut ctx = context();
    fn panel(ui: &Ui, state: &mut PanelState) {
        super::title_row(ui, &mut state.session);
        if state.session.multibox {
            super::add_bot_button(ui, state);
        }
    }
    let step = |ctx: &mut Context, state: &mut PanelState, input: FrameInput| {
        state.session.pump_status();
        frame(ctx, state, panel, input);
        state.session.pump_status();
        frames(ctx, state, panel, 2)
    };
    let gatherer = Some(script::ScriptSel::Compiled(script::CompiledId("Gatherer")));
    frames(&mut ctx, &mut state, panel, 2);

    let press = |label| FrameInput {
        press: Some((OPENER_HOST, label)),
        ..FrameInput::default()
    };
    step(&mut ctx, &mut state, press("MultiBox"));
    let s = &state.session;
    assert!(s.multibox && s.wall.chooser_open, "MultiBox opens Profiles");
    assert!(s.core.vault().is_none() && s.core.slots().is_empty());
    assert_no_start(s, "MultiBox while locked");

    // The unlock prompt in Profiles: type, click, then the startup pump's
    // unlock of that passphrase (here at the test vault).
    state.session.pass_scratch.push_str("test-passphrase-01");
    let unlock = if state.session.default_vault_exists().unwrap() {
        "Unlock vault"
    } else {
        "Create vault"
    };
    step(
        &mut ctx,
        &mut state,
        FrameInput {
            press: Some(("Profiles", unlock)),
            ..FrameInput::default()
        },
    );
    let pass = state
        .session
        .take_requested_unlock()
        .expect("unlock requested");
    assert!(state.session.unlock_at(&path, pass.as_str()));
    step(&mut ctx, &mut state, FrameInput::default());
    let s = &state.session;
    assert_eq!(s.focused_name().as_deref(), Some("alice"));
    assert!(s.core.play().unwrap().arm("alice").unwrap().wants_login());
    assert!(
        s.core.fleet().contains("alice"),
        "the unlocked bot is on the rail"
    );
    assert_no_start(s, "unlock");

    step(&mut ctx, &mut state, press("Profiles…"));
    for (pick, heading) in [
        ("bob", gatherer.clone()),
        ("alice", None),
        ("bob", gatherer.clone()),
    ] {
        step(
            &mut ctx,
            &mut state,
            FrameInput {
                row: Some(pick),
                ..FrameInput::default()
            },
        );
        let s = &state.session;
        assert_eq!(s.focused_name().as_deref(), Some(pick), "pick {pick}");
        assert_eq!(
            s.tv_name().as_deref(),
            Some(pick),
            "the Game view follows {pick}"
        );
        assert!(s.core.fleet().contains("alice") && s.core.fleet().contains("bob"));
        assert_eq!(s.core.slots().len(), 2);
        if let Some(heading) = heading {
            assert_eq!(s.script_sel, Some(heading), "{pick}'s own card");
        } else {
            assert_eq!(
                s.script_sel,
                Some(script::ScriptSel::Loaded(
                    script::ScriptSource::Catalog,
                    "Thiever".into()
                )),
                "alice's own card"
            );
        }
        assert_no_start(s, pick);
    }
}
