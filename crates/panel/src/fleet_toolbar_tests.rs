//! The fleet toolbar at its three sites (the rail, the strip's fleet section
//! and the Fleet window), driven through real Dear ImGui frames with the
//! panel font at 1x, 1.5x and 2x: every item fits its site, every click is
//! a pointer press on an item the toolbar laid out, and every decision read
//! back is the core's.

use dear_imgui_rs::{BackendFlags, Condition, FramePrepareOptions, MouseButton};
use frontend_core::scripts::{CardSource, ScriptChoice};
use frontend_core::test_support::session::install_vault;
use frontend_core::test_support::{FixtureProfile, IsolatedStores, VaultFixture};
use frontend_core::{FleetAction, FleetCard, OperatorCommand, Targets, OWN_CARD_LABEL};

use super::{Site, ToolbarAction, ACTIONS};
use crate::fleet::{take_layout, FleetLayout};
use crate::session::Session;
use crate::test_support::TestDir;

const DISPLAY: [f32; 2] = [3600.0, 3200.0];
/// Where the pointer rests when no item should be hovered.
const AWAY: [f32; 2] = [3500.0, 3100.0];
/// A catalog card name longer than any site is wide.
const LONG_CARD: &str =
    "Lumbridge cow hide tanner, Al Kharid bank runner and Varrock leather crafter deluxe";

/// Where one frame draws the toolbar, in logical pixels.
#[derive(Debug, Clone, Copy)]
enum Place {
    /// The rail's top, this wide.
    Rail(f32),
    /// The right-hand strip, this wide (the rail is hidden: MultiBox off).
    Strip(f32),
    /// The Fleet window at this size.
    Window([f32; 2]),
}

impl Place {
    fn site(self) -> Site {
        match self {
            Place::Rail(_) => Site::Rail,
            Place::Strip(_) => Site::Strip,
            Place::Window(_) => Site::Window,
        }
    }
}

fn centre([min_x, min_y, max_x, max_y]: [f32; 4]) -> [f32; 2] {
    [(min_x + max_x) / 2.0, (min_y + max_y) / 2.0]
}

/// A session with `names` in the vault and `loaded` of them on the wall,
/// and an ImGui context with the panel font at `scale`.
struct Rig {
    ctx: dear_imgui_rs::Context,
    session: Session,
    scale: f32,
    _dir: TestDir,
    // Last: the context singleton drops before the guard.
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Rig {
    fn new(label: &str, names: &[String], loaded: &[String], scale: f32) -> Self {
        let guard = crate::test_support::imgui_context_guard();
        IsolatedStores::ensure_thread();
        let dir = TestDir::new(label);
        let mut vault = VaultFixture::create(&dir.join("vault"), "test-passphrase-01");
        for (i, name) in names.iter().enumerate() {
            vault.put(FixtureProfile::new(name, "pw").uid(1 + i as i32));
        }
        let mut session = Session::new();
        install_vault(&mut session.core, vault);
        session.persist_ui = false;
        #[cfg(feature = "js")]
        session
            .scripts
            .use_library_at(dir.join("js-scripts.json"), dir.join("js-cache"));
        let mut play = frontend_core::test_support::empty_play(&host_play::PlayOptions {
            host: "127.0.0.1".into(),
            transport: client::Transport::Tcp,
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        });
        for name in loaded {
            play.attach_arm(name, host_play::SlotArm::new(42, false));
            session.core.fleet_mut().add(name);
        }
        session.core.set_play(Some(play));
        if let Some(first) = loaded.first() {
            session.set_focus_for_test(first);
        }
        let mut ctx = crate::test_support::imgui_context();
        ctx.io_mut()
            .set_backend_flags(BackendFlags::RENDERER_HAS_TEXTURES);
        assert!(crate::window::add_panel_font(&mut ctx));
        let _ = ctx.font_atlas_mut().build();
        crate::app::apply_ui_scale(ctx.style_mut(), scale);
        let mut rig = Self {
            ctx,
            session,
            scale,
            _dir: dir,
            _guard: guard,
        };
        rig.settle(Place::Window([760.0, 460.0]));
        rig
    }

    /// One frame of `place` with the pointer at `at`, the left button held
    /// when `down`: what the toolbar laid out.
    fn frame(&mut self, place: Place, at: [f32; 2], down: bool) -> FleetLayout {
        let io = self.ctx.io_mut();
        io.add_mouse_pos_event(at);
        io.add_mouse_button_event(MouseButton::Left, down);
        self.session.pump_status();
        self.ctx
            .prepare_frame(FramePrepareOptions::new(DISPLAY, 1.0 / 60.0).renderer_has_textures());
        take_layout();
        let s = self.scale;
        let mut rail_scroll = 0.0;
        {
            let ui = self.ctx.frame();
            let session = &mut self.session;
            match place {
                Place::Rail(width) => {
                    session.fleet_open = false;
                    ui.window("Rail harness")
                        .position([0.0, 0.0], Condition::Always)
                        .size([width * s, 1500.0 * s], Condition::Always)
                        .build(|| {
                            crate::app::draw_test_rail_toolbar(ui, session);
                            rail_scroll = ui.scroll_max_x();
                        });
                }
                Place::Strip(width) => {
                    session.fleet_open = false;
                    session.multibox = false;
                    crate::app::draw_test_panel_window(
                        ui,
                        session,
                        [0.0, 0.0],
                        [width * s, 3000.0],
                    );
                }
                Place::Window(size) => {
                    session.fleet_open = true;
                    ui.window(crate::session::PanelWindow::Fleet.title())
                        .position([0.0, 0.0], Condition::Always)
                        .size([size[0] * s, size[1] * s], Condition::Always)
                        .build(|| {});
                    crate::fleet::window(ui, session);
                }
            }
            crate::world_picker::popup(ui, session);
        }
        self.ctx.render();
        let mut layout = take_layout();
        if let Place::Rail(_) = place {
            layout.scroll_max_x = rail_scroll;
        }
        layout
    }

    /// Frames with the pointer away; the last one's layout.
    fn settle(&mut self, place: Place) -> FleetLayout {
        let mut layout = FleetLayout::default();
        for _ in 0..4 {
            layout = self.frame(place, AWAY, false);
        }
        layout
    }

    /// A left click on `rect`'s centre (hover, press, release), then frames
    /// that show the result: the last layout.
    fn click(&mut self, place: Place, rect: [f32; 4]) -> FleetLayout {
        let at = centre(rect);
        self.frame(place, at, false);
        self.frame(place, at, true);
        self.frame(place, at, false);
        self.frame(place, at, false);
        self.settle(place)
    }

    fn button(layout: &FleetLayout, action: ToolbarAction) -> [f32; 4] {
        layout
            .actions
            .iter()
            .find(|record| record.action == action)
            .unwrap_or_else(|| panic!("{action:?} is not drawn: {:?}", layout.actions))
            .rect
    }

    fn commands(&self, command: OperatorCommand) -> usize {
        self.session
            .core
            .commands()
            .reports()
            .filter(|report| report.command() == command)
            .count()
    }
}

fn names(count: usize) -> Vec<String> {
    (0..count).map(|i| format!("bot-{i:03}")).collect()
}

/// Every toolbar item sits inside the site's content edges, no button is
/// narrower than its text, and the site never scrolls sideways.
fn assert_fits(layout: &FleetLayout, case: &str) {
    let [left, right] = layout.content;
    assert!(right > left, "{case}: the toolbar was drawn");
    assert_eq!(layout.actions.len(), ACTIONS.len(), "{case}: nine buttons");
    for item in &layout.items {
        assert!(
            item[0] >= left - 0.5 && item[2] <= right + 0.5,
            "{case}: an item spans {}..{}, outside {left}..{right}",
            item[0],
            item[2]
        );
    }
    for record in &layout.actions {
        let width = record.rect[2] - record.rect[0];
        assert!(
            width + 0.5 >= record.text_width,
            "{case}: {:?} is {width} wide for {} of text",
            record.label,
            record.text_width
        );
        assert!(
            !record.label.contains('\u{2026}') && !record.label.contains("..."),
            "{case}: {:?}",
            record.label
        );
    }
    assert!(
        layout.scroll_max_x < 0.5,
        "{case}: scrolls sideways by {}",
        layout.scroll_max_x
    );
}

/// The toolbar fits the rail at 264 and squeezed to 200 logical pixels, the
/// strip at 330 and the Fleet window at 760×460 and its 360×240 minimum, at
/// 1x, 1.5x and 2x, with a long catalog card, 100 bots in scope (marked or
/// not), a waiting plan naming all 100 and a result line. Folded, its
/// summary fits too.
#[test]
fn the_toolbar_fits_every_site_at_every_scale() {
    let all = names(100);
    for scale in [1.0, 1.5, 2.0] {
        let mut rig = Rig::new("toolbar-fit", &all, &all, scale);
        // A result (Log in all 100, confirmed), then a waiting plan (Assign
        // Gatherer to all 100), then the long card as the fleet card.
        rig.session.plan_fleet_action(FleetAction::Login);
        rig.session.confirm_fleet_plan();
        assert!(rig.session.core.fleet_result().is_some(), "Log in ran");
        let pin = |rig: &mut Rig, choice: ScriptChoice| {
            rig.session
                .fleet_model
                .stage_card(FleetCard::pinned(choice));
            rig.session
                .submit(OperatorCommand::SetFleetCard, Targets::Application);
        };
        let gatherer = frontend_core::scripts::CompiledCardId::lookup("Gatherer").unwrap();
        pin(&mut rig, ScriptChoice::compiled(gatherer));
        rig.session.plan_fleet_action(FleetAction::Assign);
        assert!(rig.session.core.fleet_plan().is_some(), "Assign all asks");
        pin(
            &mut rig,
            ScriptChoice::loaded(CardSource::Catalog, LONG_CARD),
        );
        for marked in [false, true] {
            if marked {
                for name in &all {
                    let identity = rig.session.core.profile_identity(name).unwrap();
                    rig.session.fleet_model.set_mark_for_test(identity, true);
                }
            }
            for place in [
                Place::Rail(crate::rail::RAIL_W),
                Place::Rail(200.0),
                Place::Strip(330.0),
                Place::Window([760.0, 460.0]),
                Place::Window([crate::fleet::MIN_WIDTH, 240.0]),
            ] {
                let case = format!("{place:?} at {scale}x, marked {marked}");
                rig.session.ui.fleet_toolbar_open = true;
                let layout = rig.settle(place);
                assert_eq!(layout.site, Some(place.site()), "{case}");
                assert!(layout.plan.is_some(), "{case}: the plan row");
                assert!(layout.result.is_some(), "{case}: the result line");
                assert!(
                    layout.actions[0].label.contains("100"),
                    "{case}: {:?}",
                    layout.actions[0].label
                );
                assert_fits(&layout, &case);
                if let Place::Window(_) = place {
                    continue;
                }
                rig.session.ui.fleet_toolbar_open = false;
                let folded = rig.settle(place);
                assert!(folded.actions.is_empty(), "{case}: folded");
                assert!(folded.folded.is_some(), "{case}: the folded summary");
                assert!(folded.plan.is_some(), "{case}: folded keeps the plan");
                assert!(folded.result.is_some(), "{case}: folded keeps the result");
                let [left, right] = folded.content;
                for item in &folded.items {
                    assert!(
                        item[0] >= left - 0.5 && item[2] <= right + 0.5,
                        "{case}: a folded item spans {}..{}, outside {left}..{right}",
                        item[0],
                        item[2]
                    );
                }
            }
        }
    }
}

/// The same toolbar, clicked at each site: the scope label, the prompt, the
/// Cancel and the Confirm, and the result are the same, and each Confirm
/// runs one command. A click on the rail's and the strip's fold header
/// folds the toolbar (saved, apart from the tile folds) and keeps the plan.
#[test]
fn each_site_plans_confirms_and_folds_through_real_clicks() {
    let all = names(3);
    let mut seen: Vec<(String, String, String)> = Vec::new();
    for place in [
        Place::Rail(crate::rail::RAIL_W),
        Place::Strip(330.0),
        Place::Window([760.0, 460.0]),
    ] {
        let mut rig = Rig::new("toolbar-sites", &all, &all, 1.0);
        let layout = rig.settle(place);
        let login = ToolbarAction::Plan(FleetAction::Login);
        let label = layout
            .actions
            .iter()
            .find(|record| record.action == login)
            .map(|record| record.label.clone())
            .unwrap();
        assert_eq!(label, "Log in all 3###fleet-action-4", "{place:?}");

        // Plan, then Cancel: nothing runs.
        let layout = rig.click(place, Rig::button(&layout, login));
        let plan = layout.plan.clone().expect("Log in all asks first");
        let layout = rig.click(place, plan.cancel);
        assert!(layout.plan.is_none() && rig.session.core.fleet_plan().is_none());
        assert_eq!(rig.commands(OperatorCommand::Login), 0, "{place:?}");

        // Plan again. The rail and the strip fold now: the folded toolbar
        // keeps the plan row, and its Confirm runs it.
        let mut layout = rig.click(place, Rig::button(&layout, login));
        let plan = layout.plan.clone().expect("Log in all asks first");
        let folds = !matches!(place, Place::Window(_));
        if folds {
            let previews = rig.session.ui.rail_preview.clone();
            layout = rig.click(place, layout.fold_header);
            assert!(!rig.session.ui.fleet_toolbar_open, "{place:?}: folded");
            assert!(layout.actions.is_empty(), "{place:?}: no buttons folded");
            assert_eq!(
                layout.folded.as_deref(),
                Some("0 marked \u{b7} Each bot's own card"),
                "{place:?}"
            );
            assert_eq!(rig.session.ui.rail_preview, previews, "tile folds kept");
        }
        let confirm = layout.plan.clone().expect("the plan row stays").confirm;
        let layout = rig.click(place, confirm);
        assert_eq!(rig.commands(OperatorCommand::Login), 1, "{place:?}");
        let (result, _) = layout.result.clone().expect("the result line");
        seen.push((label, plan.prompt, result));
        if folds {
            let layout = rig.click(place, layout.fold_header);
            assert!(rig.session.ui.fleet_toolbar_open, "{place:?}: open again");
            assert_eq!(layout.actions.len(), ACTIONS.len());
        }
    }
    assert!(
        seen.windows(2).all(|pair| pair[0] == pair[1]),
        "every site shows the same label, prompt and result: {seen:?}"
    );
}

/// With the rail and the Fleet window on screen together, both toolbars
/// show the one waiting plan; a Confirm in one runs it once, and the other's
/// Confirm afterwards finds nothing to run.
#[test]
fn two_toolbars_on_screen_run_one_plan_once() {
    let all = names(2);
    let mut rig = Rig::new("toolbar-two", &all, &all, 1.0);
    rig.session.plan_fleet_action(FleetAction::Login);
    let frame = |rig: &mut Rig, at: [f32; 2], down: bool| -> (FleetLayout, FleetLayout) {
        let io = rig.ctx.io_mut();
        io.add_mouse_pos_event(at);
        io.add_mouse_button_event(MouseButton::Left, down);
        rig.session.pump_status();
        rig.ctx
            .prepare_frame(FramePrepareOptions::new(DISPLAY, 1.0 / 60.0).renderer_has_textures());
        crate::fleet::take_plan_rows();
        take_layout();
        let (rail, window);
        {
            let ui = rig.ctx.frame();
            let session = &mut rig.session;
            session.fleet_open = true;
            ui.window("Rail harness")
                .position([0.0, 0.0], Condition::Always)
                .size([264.0, 900.0], Condition::Always)
                .build(|| crate::app::draw_test_rail_toolbar(ui, session));
            rail = take_layout();
            ui.window(crate::session::PanelWindow::Fleet.title())
                .position([400.0, 0.0], Condition::Always)
                .size([760.0, 460.0], Condition::Always)
                .build(|| {});
            crate::fleet::window(ui, session);
            window = take_layout();
        }
        rig.ctx.render();
        (rail, window)
    };
    for _ in 0..4 {
        frame(&mut rig, AWAY, false);
    }
    let (rail, window) = frame(&mut rig, AWAY, false);
    assert_eq!(
        crate::fleet::take_plan_rows(),
        2,
        "one plan row per toolbar"
    );
    let (rail_plan, window_plan) = (rail.plan.unwrap(), window.plan.unwrap());
    assert_eq!(rail_plan.prompt, window_plan.prompt, "one prompt");
    let at = centre(rail_plan.confirm);
    frame(&mut rig, at, false);
    frame(&mut rig, at, true);
    frame(&mut rig, at, false);
    assert_eq!(rig.commands(OperatorCommand::Login), 1);
    assert!(rig.session.core.fleet_plan().is_none());
    let at = centre(window_plan.confirm);
    for down in [false, true, false, false] {
        frame(&mut rig, at, down);
    }
    assert_eq!(rig.commands(OperatorCommand::Login), 1, "nothing ran twice");
    let (rail, window) = frame(&mut rig, AWAY, false);
    assert_eq!(rail.result.unwrap().0, window.result.unwrap().0);
}

/// With the rail and the Fleet window on screen together, a card picked
/// through real clicks in either one's drop-down is pinned on the first
/// try: the other toolbar's closed drop-down leaves the open search alone.
/// (Headed F5 proof: a pick in either drop-down only highlighted the entry
/// while both were drawn.)
#[test]
fn a_card_pick_lands_with_two_toolbars_on_screen() {
    let all = names(2);
    let mut rig = Rig::new("toolbar-two-card", &all, &all, 1.0);
    let frame = |rig: &mut Rig, at: [f32; 2], down: bool| -> (FleetLayout, FleetLayout) {
        let io = rig.ctx.io_mut();
        io.add_mouse_pos_event(at);
        io.add_mouse_button_event(MouseButton::Left, down);
        rig.session.pump_status();
        rig.ctx
            .prepare_frame(FramePrepareOptions::new(DISPLAY, 1.0 / 60.0).renderer_has_textures());
        take_layout();
        let (rail, window);
        {
            let ui = rig.ctx.frame();
            let session = &mut rig.session;
            session.fleet_open = true;
            ui.window("Rail harness")
                .position([0.0, 0.0], Condition::Always)
                .size([264.0, 900.0], Condition::Always)
                .build(|| crate::app::draw_test_rail_toolbar(ui, session));
            rail = take_layout();
            ui.window(crate::session::PanelWindow::Fleet.title())
                .position([400.0, 0.0], Condition::Always)
                .size([760.0, 460.0], Condition::Always)
                .build(|| {});
            crate::fleet::window(ui, session);
            window = take_layout();
        }
        rig.ctx.render();
        (rail, window)
    };
    let click = |rig: &mut Rig, at: [f32; 2]| {
        for down in [false, true, false] {
            frame(rig, at, down);
        }
    };
    for _ in 0..4 {
        frame(&mut rig, AWAY, false);
    }
    // First the rail's drop-down picks Gatherer, then the Fleet window's
    // picks each bot's own card again.
    for (site_is_rail, wanted, label) in [
        (true, "Gatherer", "Gatherer"),
        (false, OWN_CARD_LABEL, OWN_CARD_LABEL),
    ] {
        let (rail, window) = frame(&mut rig, AWAY, false);
        let combo = if site_is_rail {
            rail.card_combo
        } else {
            window.card_combo
        };
        click(&mut rig, centre(combo));
        let (rail, window) = frame(&mut rig, AWAY, false);
        let entries = if site_is_rail {
            rail.card_entries
        } else {
            window.card_entries
        };
        let (_, entry) = entries
            .iter()
            .find(|(text, _)| text.contains(wanted))
            .unwrap_or_else(|| panic!("{wanted} listed: {entries:?}"));
        click(&mut rig, centre(*entry));
        frame(&mut rig, AWAY, false);
        assert_eq!(
            rig.session.fleet_model.card_label(),
            label,
            "the pick in the {} drop-down landed on the first click",
            if site_is_rail { "rail" } else { "Fleet window" }
        );
    }
}

/// Grid and MultiBox off draw the whole toolbar in the strip's fleet
/// section; with the rail shown the strip draws none (the rail does).
#[test]
fn grid_and_multibox_off_carry_the_toolbar_in_the_strip() {
    let all = names(2);
    let mut rig = Rig::new("toolbar-strip", &all, &all, 1.0);
    for (multibox, grid, drawn) in [
        (false, false, true),
        (true, true, true),
        (true, false, false),
    ] {
        let strip = |rig: &mut Rig| {
            rig.session.pump_status();
            rig.ctx.prepare_frame(
                FramePrepareOptions::new(DISPLAY, 1.0 / 60.0).renderer_has_textures(),
            );
            take_layout();
            {
                let ui = rig.ctx.frame();
                rig.session.multibox = multibox;
                rig.session.wall.grid = grid;
                crate::app::draw_test_panel_window(
                    ui,
                    &mut rig.session,
                    [0.0, 0.0],
                    [330.0, 3000.0],
                );
            }
            rig.ctx.render();
            take_layout()
        };
        let mut layout = FleetLayout::default();
        for _ in 0..3 {
            layout = strip(&mut rig);
        }
        let case = format!("multibox {multibox}, grid {grid}");
        assert_eq!(rig.session.rail_shown(), !drawn, "{case}");
        if drawn {
            assert_eq!(layout.site, Some(Site::Strip), "{case}");
            assert_eq!(layout.actions.len(), ACTIONS.len(), "{case}");
        } else {
            assert_eq!(layout.site, None, "{case}: the rail has it");
        }
    }
}

/// The toolbar's Walk names its scope and opens WalkTo in Group mode over
/// the marked bots, adding no mark.
#[test]
fn walk_opens_walkto_over_the_scope() {
    let all = names(2);
    let mut rig = Rig::new("toolbar-walk", &all, &all, 1.0);
    let place = Place::Rail(crate::rail::RAIL_W);
    let first = rig.session.core.profile_identity("bot-000").unwrap();
    rig.session.fleet_model.set_mark_for_test(first, true);
    let layout = rig.settle(place);
    let walk = Rig::button(&layout, ToolbarAction::Walk);
    assert_eq!(
        layout.actions[6].label, "Walk 1###fleet-action-6",
        "Walk names its scope"
    );
    rig.click(place, walk);
    assert!(rig.session.walkto_open);
    assert_eq!(
        rig.session.walk_send.mode,
        crate::session::WalkSendMode::Group
    );
    assert_eq!(rig.session.fleet_model.marks().len(), 1, "no mark added");
}

/// With nothing marked, the toolbar's Walk covers the bots its label
/// counts: the loaded, shown ones. Two bots can walk and the filter shows
/// one; a click on `Walk all 1` at the rail, the strip and the Fleet
/// window marks that bot only, and the hidden one is not added.
#[test]
fn walk_without_marks_keeps_the_shown_scope() {
    let all = names(2);
    for place in [
        Place::Rail(crate::rail::RAIL_W),
        Place::Strip(330.0),
        Place::Window([760.0, 460.0]),
    ] {
        let mut rig = Rig::new("toolbar-walk-shown", &all, &all, 1.0);
        {
            let play = rig.session.core.play_for_test().unwrap();
            let mut statuses = play.statuses.lock().unwrap();
            for name in &all {
                statuses.push(host_play::SlotStatus {
                    username: name.clone(),
                    connected: true,
                    ingame: true,
                    tile_x: 3200,
                    tile_z: 3200,
                    ..Default::default()
                });
            }
        }
        rig.session.pump_status();
        rig.session
            .fleet_model
            .staged_filter_mut()
            .push_str(&all[0]);
        rig.session
            .submit(OperatorCommand::SetFleetFilter, Targets::Application);
        let layout = rig.settle(place);
        assert!(rig.session.fleet_model.marks().is_empty(), "{place:?}");
        assert_eq!(
            layout.actions[6].label, "Walk all 1###fleet-action-6",
            "{place:?}"
        );
        rig.click(place, Rig::button(&layout, ToolbarAction::Walk));
        assert!(rig.session.walkto_open, "{place:?}");
        let rows = rig.session.walk_send.rows();
        assert!(
            rows.len() == 2 && rows.iter().all(|row| row.status.is_eligible()),
            "{place:?}: both bots are in the Send list and can walk"
        );
        let selected: Vec<&str> = rows
            .iter()
            .filter(|row| row.checked(rig.session.fleet_model.marks()))
            .map(|row| row.name())
            .collect();
        assert_eq!(
            selected,
            [all[0].as_str()],
            "{place:?}: the hidden bot is not added"
        );
        assert_eq!(rig.session.fleet_model.marks().len(), 1, "{place:?}");
    }
}

/// One frame that draws the toolbar at each of `sites` (open or folded),
/// each in its own harness window; no site draws only the empty frame. No
/// session sync: the frame draws only.
fn toolbar_frame(rig: &mut Rig, sites: &[Site], open: bool) {
    rig.session.ui.fleet_toolbar_open = open;
    rig.ctx
        .prepare_frame(FramePrepareOptions::new(DISPLAY, 1.0 / 60.0).renderer_has_textures());
    {
        let ui = rig.ctx.frame();
        let session = &mut rig.session;
        for (index, site) in sites.iter().enumerate() {
            let name = match site {
                Site::Rail => "Rail toolbar harness",
                Site::Strip => "Strip toolbar harness",
                Site::Window => "Window toolbar harness",
            };
            ui.window(name)
                .position([400.0 * index as f32, 0.0], Condition::Always)
                .size([330.0, 1500.0], Condition::Always)
                .build(|| match site {
                    Site::Rail => crate::app::draw_test_rail_toolbar(ui, session),
                    Site::Strip => {
                        let open = super::header(ui, session, "fleet");
                        super::body(ui, session, Site::Strip, open);
                    }
                    Site::Window => super::body(ui, session, Site::Window, true),
                });
        }
    }
    rig.ctx.render();
}

/// Allocation calls over ten warm frames.
fn toolbar_frame_calls(rig: &mut Rig, sites: &[Site], open: bool) -> usize {
    for _ in 0..5 {
        toolbar_frame(rig, sites, open);
    }
    let before = crate::test_alloc::alloc_calls();
    for _ in 0..10 {
        toolbar_frame(rig, sites, open);
    }
    crate::test_alloc::alloc_calls() - before
}

/// A session with 100 bots in scope, a result and a waiting 100-bot plan.
fn busy_rig(label: &str) -> Rig {
    let all = names(100);
    let mut rig = Rig::new(label, &all, &all, 1.0);
    rig.session.plan_fleet_action(FleetAction::Login);
    rig.session.confirm_fleet_plan();
    let gatherer = frontend_core::scripts::CompiledCardId::lookup("Gatherer").unwrap();
    rig.session
        .fleet_model
        .stage_card(FleetCard::pinned(ScriptChoice::compiled(gatherer)));
    rig.session
        .submit(OperatorCommand::SetFleetCard, Targets::Application);
    rig.session.plan_fleet_action(FleetAction::Assign);
    rig.session.pump_status();
    assert!(rig.session.core.fleet_plan().is_some());
    assert!(rig.session.core.fleet_result().is_some());
    rig
}

/// A state a warm toolbar frame can show. Every state also shows the
/// waiting plan's prompt and the result line ([`busy_rig`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shown {
    /// Nothing open, the pointer away.
    Closed,
    /// The filter holds `bot-01` (ten of the hundred bots shown); folded,
    /// the summary shows it.
    Filtered,
    /// The card drop-down open, its search empty.
    CardPicker,
    /// The card drop-down open, `ga` typed in its search: some cards match.
    CardSearch,
    /// The card drop-down open, `zqx` typed in its search: no card matches.
    CardSearchNoMatch,
    /// The sort drop-down open.
    SortPicker,
    /// The Marks menu open.
    MarksMenu,
    /// The Marks menu's Mark by status submenu open.
    StatusMenu,
    /// The tooltip of the Marks menu's Move marked to top.
    MoveToTopTooltip,
    /// The Stop button's tooltip.
    ButtonTooltip,
    /// The result line's tooltip.
    ResultTooltip,
}

impl Shown {
    /// Every state of an open toolbar.
    const OPEN: [Shown; 11] = [
        Shown::Closed,
        Shown::Filtered,
        Shown::CardPicker,
        Shown::CardSearch,
        Shown::CardSearchNoMatch,
        Shown::SortPicker,
        Shown::MarksMenu,
        Shown::StatusMenu,
        Shown::MoveToTopTooltip,
        Shown::ButtonTooltip,
        Shown::ResultTooltip,
    ];
    /// Every state of a folded toolbar (the rail's and the strip's).
    const FOLDED: [Shown; 3] = [Shown::Closed, Shown::Filtered, Shown::ResultTooltip];
}

/// Whether the last frame of `_rig` (whose context is the current one)
/// drew a tooltip.
fn tooltip_shown(_rig: &Rig) -> bool {
    // SAFETY: the rig's context is the current ImGui context (its guard is
    // held) and no frame is open; the window pointer, if any, belongs to
    // that context and is read at once, not kept.
    unsafe {
        let window = dear_imgui_rs::sys::igFindWindowByName(c"##Tooltip_00".as_ptr());
        !window.is_null() && (*window).Active
    }
}

/// The size of the tooltip window the last frame of `_rig` drew.
fn tooltip_size(_rig: &Rig) -> [f32; 2] {
    // SAFETY: as in `tooltip_shown`: the rig's context is current, no frame
    // is open, and the window pointer is read at once, not kept.
    unsafe {
        let window = dear_imgui_rs::sys::igFindWindowByName(c"##Tooltip_00".as_ptr());
        assert!(!window.is_null(), "a tooltip window exists");
        [(*window).Size.x, (*window).Size.y]
    }
}

/// A short result's tooltip takes one line: it is not a column one letter
/// wide and as tall as the text is long (the wrap position the tooltip
/// sets must hold).
#[test]
fn the_result_tooltip_holds_a_short_result_on_one_line() {
    let all = names(3);
    for scale in [1.0, 2.0] {
        let mut rig = Rig::new("toolbar-result-tooltip", &all, &all, scale);
        rig.session.plan_fleet_action(FleetAction::Login);
        rig.session.confirm_fleet_plan();
        let place = Place::Window([760.0, 460.0]);
        let (text, rect) = rig.settle(place).result.expect("a result line");
        let line = rect[3] - rect[1];
        let at = centre(rect);
        for _ in 0..40 {
            rig.frame(place, at, false);
        }
        assert!(
            tooltip_shown(&rig),
            "{scale}x: the result line has a tooltip"
        );
        let [width, height] = tooltip_size(&rig);
        assert!(
            height < 3.0 * line && width > height,
            "{scale}x: tooltip for {text:?} is {width}x{height}, one line is {line}"
        );
    }
}

/// A press and release of the left button at `at` over toolbar frames of
/// `sites`, the pointer left there.
fn press_at(rig: &mut Rig, sites: &[Site], open: bool, at: [f32; 2]) {
    for down in [false, true, false] {
        rig.ctx.io_mut().add_mouse_pos_event(at);
        rig.ctx
            .io_mut()
            .add_mouse_button_event(MouseButton::Left, down);
        toolbar_frame(rig, sites, open);
    }
}

/// The pointer to `at`, over enough frames for a hover tooltip to show.
fn hover_at(rig: &mut Rig, sites: &[Site], open: bool, at: [f32; 2]) {
    rig.ctx.io_mut().add_mouse_pos_event(at);
    for _ in 0..40 {
        toolbar_frame(rig, sites, open);
    }
}

/// One recorded toolbar frame of `sites`: what the last site laid out.
fn recorded_frame(rig: &mut Rig, sites: &[Site], open: bool) -> FleetLayout {
    take_layout();
    toolbar_frame(rig, sites, open);
    take_layout()
}

/// Close whatever is open (a click away from every toolbar), clear the
/// filter and rest the pointer away.
fn reset_shown(rig: &mut Rig, sites: &[Site], open: bool) {
    press_at(rig, sites, open, AWAY);
    rig.session.fleet_model.staged_filter_mut().clear();
    rig.session
        .submit(OperatorCommand::SetFleetFilter, Targets::Application);
    for _ in 0..3 {
        toolbar_frame(rig, sites, open);
    }
    assert!(
        rig.session.fleet_card_filter.is_none(),
        "every drop-down closed"
    );
}

/// Bring the last of `sites` into `shown` through pointer and key input,
/// as an operator would.
fn enter_shown(rig: &mut Rig, sites: &[Site], open: bool, shown: Shown) {
    let layout = recorded_frame(rig, sites, open);
    let type_text = |rig: &mut Rig, text: &str| {
        for c in text.chars() {
            rig.ctx.io_mut().add_input_character(c);
            toolbar_frame(rig, sites, open);
        }
    };
    match shown {
        Shown::Closed => {}
        Shown::Filtered => {
            rig.session
                .fleet_model
                .staged_filter_mut()
                .push_str("bot-01");
            rig.session
                .submit(OperatorCommand::SetFleetFilter, Targets::Application);
        }
        Shown::CardPicker | Shown::CardSearch | Shown::CardSearchNoMatch => {
            press_at(rig, sites, open, centre(layout.card_combo));
            rig.ctx.io_mut().add_mouse_pos_event(AWAY);
            match shown {
                Shown::CardSearch => type_text(rig, "ga"),
                Shown::CardSearchNoMatch => type_text(rig, "zqx"),
                _ => {}
            }
        }
        Shown::SortPicker => {
            press_at(rig, sites, open, centre(layout.sort_combo));
            rig.ctx.io_mut().add_mouse_pos_event(AWAY);
        }
        Shown::MarksMenu | Shown::StatusMenu | Shown::MoveToTopTooltip => {
            press_at(rig, sites, open, centre(layout.marks_button));
            rig.ctx.io_mut().add_mouse_pos_event(AWAY);
            let menu = recorded_frame(rig, sites, open);
            match shown {
                Shown::StatusMenu => {
                    let (_, rect) = menu
                        .marks_menu
                        .iter()
                        .find(|(label, _)| label == "Mark by status")
                        .expect("the Marks menu lists Mark by status");
                    hover_at(rig, sites, open, centre(*rect));
                }
                Shown::MoveToTopTooltip => hover_at(rig, sites, open, centre(menu.move_to_top)),
                _ => {}
            }
        }
        Shown::ButtonTooltip => {
            let stop = Rig::button(&layout, ToolbarAction::Plan(FleetAction::Stop));
            hover_at(rig, sites, open, centre(stop));
        }
        Shown::ResultTooltip => {
            let (_, rect) = layout.result.expect("a result line");
            hover_at(rig, sites, open, centre(rect));
        }
    }
}

/// Check that the last of `sites` still shows `shown` after the measured
/// frames.
fn assert_shown(rig: &mut Rig, sites: &[Site], open: bool, shown: Shown) {
    let site = *sites.last().unwrap();
    let case = format!("{sites:?} (open {open}) {shown:?}");
    let layout = recorded_frame(rig, sites, open);
    assert!(layout.plan.is_some(), "{case}: the prompt is drawn");
    assert!(layout.result.is_some(), "{case}: the result is drawn");
    // The open drop-down's search shows in the entries it narrows (below).
    let picker_open = rig
        .session
        .fleet_card_filter
        .as_ref()
        .is_some_and(|(owner, _)| *owner == site);
    let card_picker = matches!(
        shown,
        Shown::CardPicker | Shown::CardSearch | Shown::CardSearchNoMatch
    );
    assert_eq!(picker_open, card_picker, "{case}: card drop-down");
    let tooltip = matches!(
        shown,
        Shown::MoveToTopTooltip | Shown::ButtonTooltip | Shown::ResultTooltip
    );
    assert_eq!(tooltip_shown(rig), tooltip, "{case}: tooltip");
    let menu = matches!(
        shown,
        Shown::MarksMenu | Shown::StatusMenu | Shown::MoveToTopTooltip
    );
    assert_eq!(!layout.marks_menu.is_empty(), menu, "{case}: Marks menu");
    let by_status = frontend_core::MarkPredicate::ALL[0].label();
    assert_eq!(
        layout
            .marks_menu
            .iter()
            .any(|(label, _)| label == by_status),
        shown == Shown::StatusMenu,
        "{case}: Mark by status submenu"
    );
    assert_eq!(
        layout.sort_choices.len(),
        if shown == Shown::SortPicker { 3 } else { 0 },
        "{case}: sort drop-down"
    );
    match shown {
        Shown::Filtered => {
            assert_eq!(rig.session.fleet_model.filter().text(), "bot-01", "{case}");
            assert_eq!(rig.session.fleet_model.counts().shown(), 10, "{case}");
        }
        Shown::CardPicker => {
            assert!(layout.card_entries.len() > 2, "{case}: Browse cards listed");
        }
        Shown::CardSearch => {
            assert!(
                !layout.card_entries.is_empty()
                    && layout
                        .card_entries
                        .iter()
                        .all(|(label, _)| frontend_core::contains_ci(label, "ga")),
                "{case}: the search `ga` narrows the entries: {:?}",
                layout.card_entries
            );
        }
        Shown::CardSearchNoMatch => {
            assert!(
                layout.card_entries.is_empty(),
                "{case}: the search `zqx` matches no card"
            );
        }
        _ => assert!(layout.card_entries.is_empty(), "{case}"),
    }
}

/// Warm toolbar frames make no allocation call at any site (the rail, the
/// strip, the Fleet window, and two toolbars on screen together), open or
/// folded, in every state a toolbar shows ([`Shown`]: a drop-down, the
/// Marks menu or its submenu open, a search or a filter typed, a tooltip
/// shown), with 100 bots in scope, a waiting 100-bot plan and a result.
/// The labels are cached, the rows are borrowed from the one model, both
/// text fields are edited in place, the open card drop-down keeps its card
/// list, and nothing is formatted per frame. Each state is entered through
/// pointer and key input (whose frames may allocate) and checked after the
/// ten measured frames.
#[test]
fn a_warm_toolbar_frame_allocates_nothing_at_any_site() {
    let mut rig = busy_rig("toolbar-alloc");
    crate::fleet::set_recording(false);
    let empty = toolbar_frame_calls(&mut rig, &[], true);
    crate::fleet::set_recording(true);
    let mut cases: Vec<(&[Site], bool, Shown)> = Vec::new();
    for sites in [&[Site::Rail][..], &[Site::Strip][..], &[Site::Window][..]] {
        cases.extend(Shown::OPEN.map(|shown| (sites, true, shown)));
    }
    for sites in [&[Site::Rail][..], &[Site::Strip][..]] {
        cases.extend(Shown::FOLDED.map(|shown| (sites, false, shown)));
    }
    // Two toolbars on screen; the state is at the last one.
    for sites in [
        &[Site::Rail, Site::Window][..],
        &[Site::Window, Site::Rail][..],
    ] {
        cases.extend(
            [Shown::Closed, Shown::CardPicker, Shown::CardSearch].map(|shown| (sites, true, shown)),
        );
    }
    let mut calls = Vec::new();
    for (sites, open, shown) in cases {
        reset_shown(&mut rig, sites, open);
        enter_shown(&mut rig, sites, open, shown);
        crate::fleet::set_recording(false);
        let count = toolbar_frame_calls(&mut rig, sites, open);
        crate::fleet::set_recording(true);
        assert_shown(&mut rig, sites, open, shown);
        calls.push((sites, open, shown, count));
    }
    eprintln!("FLEET-TOOLBAR-ALLOC empty={empty} cases={calls:?}");
    assert_eq!(empty, 0, "the empty harness frame allocates nothing");
    let failing: Vec<_> = calls.iter().filter(|(.., count)| *count != 0).collect();
    assert!(
        failing.is_empty(),
        "allocation calls over 10 warm frames: {failing:?}"
    );
}

/// Measured 2026-10-21: the one set of toolbar labels for 100 marked bots
/// (eleven strings) holds 712 bytes, 416 of them on the heap. The ceiling
/// of 2 KiB keeps about 2.9 times that and fails on a second set per site
/// or a per-row copy.
#[test]
fn the_toolbar_labels_are_one_small_set_per_process() {
    const CEILING: usize = 2 * 1024;
    let mut rig = busy_rig("toolbar-label-bytes");
    for name in names(100) {
        let identity = rig.session.core.profile_identity(&name).unwrap();
        rig.session.fleet_model.set_mark_for_test(identity, true);
    }
    rig.settle(Place::Rail(crate::rail::RAIL_W));
    rig.settle(Place::Window([760.0, 460.0]));
    let labels = &rig.session.fleet_labels;
    let bytes = std::mem::size_of_val(labels) + labels.heap_bytes();
    eprintln!(
        "FLEET-TOOLBAR-LABELS bytes={bytes} heap={}",
        labels.heap_bytes()
    );
    assert!(bytes <= CEILING, "{bytes} bytes of toolbar labels");
}
