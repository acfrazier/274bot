//! The Fleet window's card drop-down, marks row and plan flow, driven through
//! real Dear ImGui frames: every click is a pointer press on an item the
//! window laid out, and every decision read back is the core's.

use dear_imgui_rs::{Condition, FramePrepareOptions, MouseButton};
use frontend_core::scripts::{CompiledCardId, ScriptChoice, ScriptRunState};
use frontend_core::test_support::scripts::Assignment;
use frontend_core::test_support::session::install_vault;
use frontend_core::test_support::{FixtureProfile, IsolatedStores, VaultFixture};
use frontend_core::{
    FleetAction, FleetCard, MarkPredicate, OperatorCommand, ShownMarks, Targets,
    FOCUSED_CARD_LABEL, OWN_CARD_LABEL,
};
use std::time::{Duration, Instant};

use super::{ActionRecord, FleetLayout};
use crate::session::Session;
use crate::test_support::TestDir;

const SIZE: [f32; 2] = [760.0, 460.0];
/// Where the pointer rests when no item should be hovered.
const AWAY: [f32; 2] = [1200.0, 700.0];
const BOT_TS: &str = "export default class T extends LoopingBot { override loop() {} }\n";

fn centre([min_x, min_y, max_x, max_y]: [f32; 4]) -> [f32; 2] {
    [(min_x + max_x) / 2.0, (min_y + max_y) / 2.0]
}

/// A Fleet window over a real session, one headless frame at a time.
struct Rig {
    ctx: dear_imgui_rs::Context,
    session: Session,
    layout: FleetLayout,
    dir: TestDir,
    // Last: the context singleton drops before the guard.
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Rig {
    /// The profiles `names` in a vault; none is loaded.
    fn new(label: &str, names: &[&str]) -> Self {
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
        session
            .scripts
            .use_library_at(dir.join("js-scripts.json"), dir.join("js-cache"));
        session.fleet_open = true;
        let mut ctx = crate::test_support::imgui_context();
        ctx.io_mut()
            .set_backend_flags(dear_imgui_rs::BackendFlags::RENDERER_HAS_TEXTURES);
        let mut rig = Self {
            ctx,
            session,
            layout: FleetLayout::default(),
            dir,
            _guard: guard,
        };
        rig.frame(AWAY, false);
        rig
    }

    /// The profiles `names`, of which `loaded` are loaded slots.
    fn with_play(label: &str, names: &[&str], loaded: &[&str]) -> Self {
        let mut rig = Self::new(label, names);
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
            rig.session.core.fleet_mut().add(name);
        }
        rig.session.core.set_play(Some(play));
        if let Some(first) = loaded.first() {
            rig.session.set_focus_for_test(first);
        }
        rig.frame(AWAY, false);
        rig
    }

    /// One frame with the pointer at `point`, the left button held when
    /// `pressed`; `self.layout` is what the window laid out.
    fn frame(&mut self, point: [f32; 2], pressed: bool) {
        let io = self.ctx.io_mut();
        io.add_mouse_pos_event(point);
        io.add_mouse_button_event(MouseButton::Left, pressed);
        self.session.pump_status();
        self.ctx.prepare_frame(
            FramePrepareOptions::new([1280.0, 720.0], 1.0 / 60.0).renderer_has_textures(),
        );
        let layout = {
            let ui = self.ctx.frame();
            // Fix the window's place and size: its own size is a first-use
            // default only.
            ui.window(crate::session::PanelWindow::Fleet.title())
                .position([0.0, 0.0], Condition::Always)
                .size(SIZE, Condition::Always)
                .build(|| {});
            super::draw_test_window(ui, &mut self.session)
        };
        self.ctx.render();
        self.layout = layout;
    }

    /// One frame that draws the Fleet window and the Scripts window's Apply
    /// to same-card members section; how many plan rows it drew.
    fn frame_with_scripts_section(&mut self) -> usize {
        self.session.pump_status();
        self.ctx.prepare_frame(
            FramePrepareOptions::new([1280.0, 720.0], 1.0 / 60.0).renderer_has_textures(),
        );
        super::take_plan_rows();
        {
            let ui = self.ctx.frame();
            ui.window(crate::session::PanelWindow::Fleet.title())
                .position([0.0, 0.0], Condition::Always)
                .size(SIZE, Condition::Always)
                .build(|| {});
            let session = &mut self.session;
            ui.window("scripts-section-test")
                .position([800.0, 0.0], Condition::Always)
                .size([400.0, 300.0], Condition::Always)
                .build(|| crate::app::draw_test_apply_section(ui, session));
            super::window(ui, session);
        }
        self.ctx.render();
        super::take_plan_rows()
    }

    /// Frames with the pointer away, to let a popup or a plan row lay out.
    fn settle(&mut self) {
        for _ in 0..3 {
            self.frame(AWAY, false);
        }
    }

    /// A left click on `rect`'s centre: hover, press, release, then frames
    /// that show the result.
    fn click(&mut self, rect: [f32; 4]) {
        let at = centre(rect);
        self.frame(at, false);
        self.frame(at, true);
        self.frame(at, false);
        self.frame(at, false);
        self.frame(AWAY, false);
    }

    fn action(&self, action: FleetAction) -> ActionRecord {
        self.layout
            .actions
            .iter()
            .find(|record| record.action == action)
            .unwrap_or_else(|| panic!("{action:?} was not drawn"))
            .clone()
    }

    /// Click the action button `action`, which must be enabled.
    fn click_action(&mut self, action: FleetAction) {
        let record = self.action(action);
        assert!(!record.disabled, "{action:?} is disabled: {record:?}");
        self.click(record.rect);
    }

    fn menu_item(&self, label: &str) -> [f32; 4] {
        self.layout
            .marks_menu
            .iter()
            .find(|(text, _)| text == label)
            .unwrap_or_else(|| panic!("{label:?} is not in the open marks menu"))
            .1
    }

    /// Open the marks menu and click `label`.
    fn click_marks_menu(&mut self, label: &str) {
        self.click(self.layout.marks_button);
        self.settle();
        self.click(self.menu_item(label));
    }

    /// Open the marks menu, then Mark by status, and click `label`.
    fn click_mark_by_status(&mut self, label: &str) {
        self.click(self.layout.marks_button);
        self.settle();
        let header = self.menu_item("Mark by status");
        let at = centre(header);
        // A menu opens when the pointer rests on it or it is clicked.
        for pressed in [false, true, false] {
            self.frame(at, pressed);
        }
        for _ in 0..3 {
            self.frame(at, false);
        }
        let item = self.menu_item(label);
        self.click(item);
    }

    fn marked_names(&self) -> Vec<String> {
        let model = &self.session.fleet_model;
        model
            .rows()
            .iter()
            .enumerate()
            .filter(|(index, _)| model.is_marked(*index))
            .map(|(_, row)| row.name().to_owned())
            .collect()
    }

    /// Stage and submit `card` as the fleet card, as the drop-down does.
    fn pin(&mut self, card: &str) {
        let id = CompiledCardId::lookup(card).expect("a compiled card");
        self.session
            .fleet_model
            .stage_card(FleetCard::pinned(ScriptChoice::compiled(id)));
        self.session
            .submit(OperatorCommand::SetFleetCard, Targets::Application);
        self.settle();
    }

    fn mark(&mut self, names: &[&str]) {
        for name in names {
            let identity = self.session.core.profile_identity(name).unwrap();
            self.session.fleet_model.set_mark_for_test(identity, true);
        }
        self.settle();
    }

    /// Give each of `names` the saved bot script and start it on its own.
    fn start_scripts(&mut self, names: &[&str]) {
        let path = self.dir.join("bot.ts");
        std::fs::write(&path, BOT_TS).unwrap();
        let card = self.session.scripts.load_card(&path).unwrap();
        for name in names {
            self.session
                .persist_successful_assignment(name, card.assignment());
        }
        self.session.core.flush_writes();
        for name in names {
            self.session.script_start_profile(name).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(60);
        while self.session.scripts.starts_pending() && Instant::now() < deadline {
            self.session.core.poll_host();
            self.session.poll_scripts();
            std::thread::sleep(Duration::from_millis(5));
        }
        for name in names {
            self.wait_state(name, ScriptRunState::Running);
        }
    }

    fn wait_state(&mut self, name: &str, want: ScriptRunState) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while self.session.core.script_state(name) != want && Instant::now() < deadline {
            self.session
                .core
                .play_for_test()
                .unwrap()
                .pump_script_lifecycle(name);
            self.session.pump_status();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(self.session.core.script_state(name), want, "{name}");
    }
}

/// Everything a card pick must leave alone, for the focused bot `alice`.
fn untouched(session: &Session) -> impl PartialEq + std::fmt::Debug {
    (
        session
            .scripts
            .browse_selection()
            .map(ScriptChoice::duplicate),
        session
            .scripts
            .pending_browse("alice")
            .map(ScriptChoice::duplicate),
        session.scripts.heading(&session.core, "alice"),
        session.script_sel.as_ref().map(ScriptChoice::duplicate),
        session.profile_assignment("alice"),
        session.profile_assignment("bob"),
        session.core.script_state("alice"),
        session.core.script_state("bob"),
        session.profile_editor.generation(),
        session.scripts.starts_pending(),
        session.core.fleet_plan().is_none(),
        session.core.fleet_result().is_none(),
    )
}

fn compiled_names(session: &Session, count: usize) -> Vec<String> {
    let names: Vec<String> = session
        .scripts
        .browse_cards()
        .filter(|card| card.compiled_id().is_some())
        .map(|card| card.name().to_owned())
        .take(count)
        .collect();
    assert_eq!(
        names.len(),
        count,
        "the registry has {count} compiled cards"
    );
    names
}

/// Picking a pinned card in the Fleet window's drop-down (searched, then
/// clicked) changes the fleet card and nothing else: the focused bot's
/// Browse pick, pending heading and assignment, the profile form and every
/// script stay as they were, and nothing starts. The old window had no
/// card of its own and staged the Browse pick for every card action.
#[test]
fn picking_a_pinned_card_changes_only_the_fleet_card() {
    let mut rig = Rig::with_play("fleet-pick", &["alice", "bob"], &["alice", "bob"]);
    let [assigned, browsed, fleet] =
        <[String; 3]>::try_from(compiled_names(&rig.session, 3)).expect("three compiled cards");
    rig.session.persist_successful_assignment(
        "alice",
        Assignment::compiled(CompiledCardId::lookup(&assigned).unwrap()),
    );
    rig.session.core.flush_writes();
    rig.session.select_script_selection(ScriptChoice::compiled(
        CompiledCardId::lookup(&browsed).unwrap(),
    ));
    rig.settle();
    assert!(
        rig.session.heading_is_pending(),
        "the focused bot has a pending pick"
    );
    assert_eq!(rig.session.fleet_model.card(), &FleetCard::OWN);
    let before = untouched(&rig.session);

    rig.click(rig.layout.card_combo);
    rig.settle();
    assert!(
        rig.layout.card_entries.len() > 3,
        "the open drop-down lists the cards"
    );
    for c in fleet.chars() {
        rig.ctx.io_mut().add_input_character(c);
    }
    rig.settle();
    let entry = rig
        .layout
        .card_entries
        .iter()
        .find(|(text, _)| text == &fleet)
        .unwrap_or_else(|| panic!("{fleet} is listed: {:?}", rig.layout.card_entries))
        .1;
    rig.click(entry);

    let pinned = ScriptChoice::compiled(CompiledCardId::lookup(&fleet).unwrap());
    assert_eq!(rig.session.fleet_model.card(), &FleetCard::pinned(pinned));
    assert_eq!(rig.session.fleet_model.card_label(), fleet);
    assert_eq!(untouched(&rig.session), before);
}

/// The drop-down lists each bot's own card, the focused bot's card, then
/// every Browse card grouped by category in the Browse window's category
/// order (the saved order first).
#[test]
fn the_card_drop_down_lists_own_focused_then_cards_by_category_order() {
    use crate::script_picker::{display_category, resolve_category_order};
    let mut rig = Rig::with_play("fleet-order", &["alice"], &["alice"]);
    let mut present: Vec<String> = Vec::new();
    for card in rig.session.scripts.browse_cards() {
        let category = display_category(card.category()).to_owned();
        if !present.contains(&category) {
            present.push(category);
        }
    }
    assert!(present.len() > 1, "the registry has several categories");
    // The operator's saved Browse order puts the last category first.
    rig.session.ui.script_category_order = vec![present.last().unwrap().clone()];
    let order = resolve_category_order(&rig.session.ui.script_category_order, &present);
    let expected: Vec<String> = [OWN_CARD_LABEL.to_owned(), FOCUSED_CARD_LABEL.to_owned()]
        .into_iter()
        .chain(order.iter().flat_map(|category| {
            rig.session
                .scripts
                .browse_cards()
                .filter(|card| display_category(card.category()) == category)
                .map(|card| card.name().to_owned())
                .collect::<Vec<_>>()
        }))
        .collect();

    rig.click(rig.layout.card_combo);
    rig.settle();
    let listed: Vec<String> = rig
        .layout
        .card_entries
        .iter()
        .map(|(text, _)| text.clone())
        .collect();
    assert_eq!(listed, expected);
}

/// With nothing marked and two loaded bots shown, Stop reads "Stop all 2",
/// shows the plan's prompt first and runs nothing until Confirm; Cancel
/// writes nothing; Confirm settles one result line. The old Stop acted on
/// the marks only, at once.
#[test]
fn stop_all_asks_first_and_runs_only_after_confirm() {
    let mut rig = Rig::with_play("fleet-stop-all", &["alice", "bob"], &["alice", "bob"]);
    rig.start_scripts(&["alice", "bob"]);
    rig.settle();
    assert!(rig.session.fleet_model.marks().is_empty());
    let stop = rig.action(FleetAction::Stop);
    assert_eq!(stop.label, "Stop all 2###fleet-action-1");
    assert!(!stop.disabled);

    // Click: the prompt shows, nothing runs.
    rig.click_action(FleetAction::Stop);
    let plan = rig.layout.plan.clone().expect("the plan row is drawn");
    assert!(plan.prompt.starts_with("Stop"), "{}", plan.prompt);
    assert!(
        plan.prompt.contains("alice") && plan.prompt.contains("bob"),
        "{}",
        plan.prompt
    );
    assert_eq!(
        rig.session.core.fleet_plan().map(|plan| plan.prompt()),
        Some(plan.prompt.as_str()),
        "the row draws the core's prompt verbatim"
    );
    for name in ["alice", "bob"] {
        assert_eq!(rig.session.core.script_state(name), ScriptRunState::Running);
    }
    assert!(rig.session.core.fleet_result().is_none());
    assert!(rig.layout.result.is_none());

    // Cancel: the plan goes, nothing was written or stopped.
    rig.click(plan.cancel);
    assert!(rig.session.core.fleet_plan().is_none());
    assert!(rig.layout.plan.is_none());
    assert!(rig.session.core.fleet_result().is_none());
    for name in ["alice", "bob"] {
        assert_eq!(rig.session.core.script_state(name), ScriptRunState::Running);
    }

    // Again, then Confirm: both stop and one result line settles.
    rig.click_action(FleetAction::Stop);
    let plan = rig
        .layout
        .plan
        .clone()
        .expect("the plan row is drawn again");
    rig.click(plan.confirm);
    assert!(rig.session.core.fleet_plan().is_none());
    for name in ["alice", "bob"] {
        rig.wait_state(name, ScriptRunState::Idle);
    }
    rig.settle();
    let result = rig.session.core.fleet_result().expect("one result");
    assert!(result.settled(), "{}", result.text());
    assert_eq!(result.counts().done, 2, "{}", result.text());
    let (line, _) = rig.layout.result.clone().expect("the result line is drawn");
    assert_eq!(line, result.text());
    assert_eq!(
        rig.session.error, None,
        "the bulk result is not copied to the banner"
    );
}

/// With marks, one hidden by the filter, a marked Assign with a pinned card
/// runs at once (one click, no prompt) on both marked profiles and the one
/// result line reads done 2. The old Assign staged the Browse pick first and
/// kept its report in a mirror on the session.
#[test]
fn a_marked_assign_runs_at_once_on_hidden_marks_too() {
    let mut rig = Rig::new("fleet-assign-marked", &["alice", "bob", "carol"]);
    rig.mark(&["alice", "carol"]);
    rig.session.fleet_model.set_filter_for_test("alice");
    rig.pin("Gatherer");
    assert_eq!(rig.layout.marks_text, "2 marked, 1 not shown");
    let assign = rig.action(FleetAction::Assign);
    assert_eq!(assign.label, "Assign 2###fleet-action-2");

    rig.session.set_focus_for_test("alice");
    assert!(rig.session.script_sel.is_none(), "alice has no card yet");
    rig.click_action(FleetAction::Assign);
    assert_eq!(
        rig.session.script_sel.as_ref().map(|sel| sel.label()),
        Some("Gatherer"),
        "the focused bot's Script heading follows its new assignment"
    );
    assert!(
        rig.session.core.fleet_plan().is_none(),
        "one click, no prompt"
    );
    assert!(rig.layout.plan.is_none());
    rig.session.core.flush_writes();
    rig.settle();
    let gatherer = || {
        Some(Assignment::compiled(
            CompiledCardId::lookup("Gatherer").unwrap(),
        ))
    };
    assert_eq!(rig.session.profile_assignment("alice"), gatherer());
    assert_eq!(rig.session.profile_assignment("carol"), gatherer());
    assert_eq!(rig.session.profile_assignment("bob"), None, "unmarked");
    let result = rig.session.core.fleet_result().expect("the result");
    assert_eq!(result.counts().done, 2, "{}", result.text());
    assert!(result.settled(), "{}", result.text());
    let (line, _) = rig.layout.result.clone().expect("the result line is drawn");
    assert!(line.starts_with("Assign: done 2: alice, carol"), "{line}");
}

/// Assign, Assign & restart and Copy settings are disabled while the fleet
/// card is each bot's own, with the tooltip "pick a card above", and a click
/// on one does nothing; Start, Stop, Log in and Log out stay enabled. A
/// pinned card enables them all.
#[test]
fn card_actions_are_disabled_on_each_bots_own_card() {
    let mut rig = Rig::new("fleet-own-card", &["alice", "bob"]);
    rig.mark(&["alice"]);
    for action in FleetAction::ALL {
        let record = rig.action(action);
        assert_eq!(
            record.disabled,
            action.needs_card(),
            "{action:?} on each bot's own card"
        );
        if action.needs_card() {
            assert_eq!(record.tooltip, "pick a card above", "{action:?}");
        }
    }
    let assign = rig.action(FleetAction::Assign);
    rig.click(assign.rect);
    assert!(rig.session.core.fleet_plan().is_none());
    assert!(rig.session.core.fleet_result().is_none());
    assert_eq!(rig.session.error, None);
    rig.session.core.flush_writes();
    assert_eq!(rig.session.profile_assignment("alice"), None);

    rig.pin("Gatherer");
    for action in FleetAction::ALL {
        assert!(!rig.action(action).disabled, "{action:?} on a pinned card");
    }
}

/// The tri-state box over the shown rows has three states and three clicks:
/// none marked marks the shown rows, some marked marks the rest, all marked
/// unmarks the shown rows (a mark the filter hides stays). The Mark column
/// header is the same box. The old header was a plain label.
#[test]
fn the_tri_state_box_marks_the_rest_and_unmarks_the_shown_rows() {
    let mut rig = Rig::new("fleet-tri-state", &["alice", "bob", "carol"]);
    assert_eq!(rig.session.fleet_model.shown_marks(), ShownMarks::None);
    rig.click(rig.layout.mark_box);
    assert_eq!(rig.session.fleet_model.shown_marks(), ShownMarks::All);
    assert_eq!(rig.marked_names(), ["alice", "bob", "carol"]);
    rig.click(rig.layout.mark_box);
    assert_eq!(rig.session.fleet_model.shown_marks(), ShownMarks::None);
    assert!(rig.marked_names().is_empty());

    // Some: the click marks the rest.
    rig.mark(&["bob"]);
    assert_eq!(rig.session.fleet_model.shown_marks(), ShownMarks::Some);
    rig.click(rig.layout.mark_box);
    assert_eq!(rig.session.fleet_model.shown_marks(), ShownMarks::All);
    assert_eq!(rig.marked_names(), ["alice", "bob", "carol"]);

    // The column header is the same box.
    rig.click(rig.layout.header_mark_box);
    assert_eq!(rig.session.fleet_model.shown_marks(), ShownMarks::None);
    rig.click(rig.layout.header_mark_box);
    assert_eq!(rig.session.fleet_model.shown_marks(), ShownMarks::All);

    // Only the shown rows: carol is hidden and keeps her mark.
    rig.session.fleet_model.set_filter_for_test("alice");
    rig.settle();
    assert_eq!(rig.session.fleet_model.shown_marks(), ShownMarks::All);
    rig.click(rig.layout.mark_box);
    assert_eq!(rig.marked_names(), ["bob", "carol"]);
    assert_eq!(rig.session.fleet_model.shown_marks(), ShownMarks::None);
    assert_eq!(rig.layout.marks_text, "2 marked, 2 not shown");
}

/// The marks menu's Mark shown, Unmark shown and Invert shown act on the
/// shown rows, and each Mark by status entry marks exactly the shown rows
/// of that status (a hidden match stays unmarked).
#[test]
fn the_marks_menu_marks_the_shown_rows_by_status() {
    let mut rig = Rig::with_play("fleet-by-status", &["xa", "xb", "xc", "yd"], &["xa", "xb"]);
    // Shown: xa, xb (loaded) and xc (not loaded); yd is filtered out.
    rig.session.fleet_model.set_filter_for_test("x");
    rig.settle();
    let shown: Vec<String> = rig
        .session
        .fleet_model
        .shown_rows()
        .map(|(_, row)| row.name().to_owned())
        .collect();
    assert_eq!(shown, ["xa", "xb", "xc"], "filter `x`");

    rig.click_marks_menu("Mark shown");
    assert_eq!(rig.marked_names(), ["xa", "xb", "xc"]);
    rig.click_marks_menu("Invert shown");
    assert!(rig.marked_names().is_empty());
    rig.click_marks_menu("Mark shown");
    rig.click_marks_menu("Unmark shown");
    assert!(rig.marked_names().is_empty());

    let mut non_empty = 0;
    for predicate in MarkPredicate::ALL {
        rig.session.fleet_model.clear_marks_for_test();
        rig.settle();
        rig.click_mark_by_status(predicate.label());
        let expected: Vec<String> = rig
            .session
            .fleet_model
            .shown_rows()
            .filter(|(_, row)| predicate.matches(row))
            .map(|(_, row)| row.name().to_owned())
            .collect();
        assert_eq!(
            rig.marked_names(),
            expected,
            "Mark by status > {}",
            predicate.label()
        );
        if predicate == MarkPredicate::NotLoaded {
            assert_eq!(expected, ["xc"], "yd matches but is hidden");
        }
        non_empty += usize::from(!expected.is_empty());
    }
    assert!(
        non_empty >= 2,
        "the statuses of the loaded rows mark something too"
    );
}

/// A steady frame with a plan waiting rebuilds no label (the cached buffers
/// keep their addresses) and submits nothing: the commands recorded, the
/// waiting plan and the prompt stay as they were. The old window formatted
/// its button labels every frame.
#[test]
fn a_steady_frame_with_a_plan_waiting_keeps_its_labels_and_submits_nothing() {
    let mut rig = Rig::with_play("fleet-steady", &["alice", "bob"], &["alice", "bob"]);
    rig.pin("Gatherer");
    rig.click_action(FleetAction::Assign);
    let prompt = rig
        .session
        .core
        .fleet_plan()
        .expect("an Assign over every shown bot waits")
        .prompt()
        .to_owned();
    rig.settle();
    let buffers = |rig: &Rig| {
        let mut buffers = rig.session.fleet_labels.buffers();
        buffers.push(rig.session.fleet_model.card_label().as_ptr());
        buffers
    };
    let before = buffers(&rig);
    let commands = rig.session.core.commands().reports().count();
    for _ in 0..8 {
        rig.frame(AWAY, false);
    }
    assert_eq!(buffers(&rig), before, "no label was rebuilt");
    assert_eq!(rig.session.core.commands().reports().count(), commands);
    assert_eq!(
        rig.session.core.fleet_plan().map(|plan| plan.prompt()),
        Some(prompt.as_str())
    );
    assert!(rig.session.core.fleet_result().is_none());
    assert_eq!(
        rig.layout.plan.as_ref().map(|plan| plan.prompt.as_str()),
        Some(prompt.as_str())
    );
}

const THIEVER_TS: &str = "export const SETTINGS = { target: { type: 'string', default: 'Man' } };\nexport default class T extends LoopingBot { override loop() {} }\n";

/// The Scripts window's Apply to same-card members entry raises the fleet
/// plan's confirmation, and that confirmation is drawn once: by the Fleet
/// window while it is open, by the Scripts section while it is closed. The
/// old entry drew its own prepared-sync row beside the Fleet window's.
#[test]
fn the_copy_confirmation_is_drawn_once() {
    let mut rig = Rig::with_play("fleet-copy-once", &["alice", "bob"], &["alice", "bob"]);
    let path = rig.dir.join("thiever.ts");
    std::fs::write(&path, THIEVER_TS).unwrap();
    let card = rig.session.scripts.load_card(&path).unwrap();
    for name in ["alice", "bob"] {
        rig.session
            .persist_successful_assignment(name, card.assignment());
    }
    rig.session.core.flush_writes();
    rig.session.set_focus_for_test("alice");
    rig.session.restore_script_heading("alice");
    rig.settle();

    rig.session.copy_focused_settings_to_same_card();
    let plan = rig.session.core.fleet_plan().expect("the copy waits");
    assert_eq!(plan.action(), FleetAction::CopySettings);
    assert!(plan.needs_confirm());
    for _ in 0..3 {
        assert_eq!(rig.frame_with_scripts_section(), 1, "Fleet window open");
    }
    rig.session.fleet_open = false;
    for _ in 0..3 {
        assert_eq!(rig.frame_with_scripts_section(), 1, "Fleet window closed");
    }
}
