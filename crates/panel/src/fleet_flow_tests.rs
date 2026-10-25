//! The Fleet window's card drop-down, marks row and plan flow, driven through
//! real Dear ImGui frames: every click is a pointer press on an item the
//! window laid out, and every decision read back is the core's.

use dear_imgui_rs::{Condition, FramePrepareOptions, MouseButton};
use frontend_core::command::Subject;
use frontend_core::scripts::{CompiledCardId, ScriptChoice};
use frontend_core::scripts::ScriptRunState;
use frontend_core::test_support::scripts::Assignment;
use frontend_core::test_support::session::install_vault;
use frontend_core::test_support::{FixtureProfile, IsolatedStores, VaultFixture};
use frontend_core::{
    FleetAction, FleetCard, MarkPredicate, OperatorCommand, PlanScope, ShownMarks, Targets,
    FOCUSED_CARD_LABEL, OWN_CARD_LABEL,
};
use std::time::{Duration, Instant};

use super::{ActionRecord, FleetLayout};
use crate::fleet_toolbar::ToolbarAction;
use crate::session::Session;
use crate::test_support::TestDir;

const SIZE: [f32; 2] = [760.0, 460.0];
/// Where the pointer rests when no item should be hovered.
const AWAY: [f32; 2] = [1200.0, 700.0];
#[cfg(feature = "js")]
const BOT_TS: &str = "export default class T extends LoopingBot { override loop() {} }\n";

fn centre([min_x, min_y, max_x, max_y]: [f32; 4]) -> [f32; 2] {
    [(min_x + max_x) / 2.0, (min_y + max_y) / 2.0]
}

/// A Fleet window over a real session, one headless frame at a time.
struct Rig {
    ctx: dear_imgui_rs::Context,
    session: Session,
    /// The session's WalkTo arms: a test arms a manual walk through these.
    walks: host_play::WalkArms,
    layout: FleetLayout,
    #[cfg(feature = "js")]
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
        let walks = host_play::WalkArms::default();
        session.core.set_walk_arms(std::sync::Arc::clone(&walks));
        install_vault(&mut session.core, vault);
        session.persist_ui = false;
        #[cfg(feature = "js")]
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
            walks,
            layout: FleetLayout::default(),
            #[cfg(feature = "js")]
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
        self.button(ToolbarAction::Plan(action))
    }

    fn button(&self, action: ToolbarAction) -> ActionRecord {
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
    #[cfg(feature = "js")]
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
    #[cfg(not(feature = "js"))]
    fn start_scripts(&mut self, names: &[&str]) {
        let card = CompiledCardId::lookup("Gatherer").expect("compiled Gatherer card");
        for name in names {
            self.session
                .persist_successful_assignment(name, Assignment::compiled(card));
        }
        self.session.core.flush_writes();
        for name in names {
            self.session
                .script_start_profile(name)
                .expect("compiled script starts");
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

    /// The profiles `names`, of which those of `worlds` are loaded and in
    /// game on the world given, on a server with worlds 1, 2 and 3.
    fn with_worlds(label: &str, names: &[&str], worlds: &[(&str, u16)]) -> Self {
        let loaded: Vec<&str> = worlds.iter().map(|(name, _)| *name).collect();
        let mut rig = Self::with_play(label, names, &loaded);
        rig.session.core.set_public_worlds_for_test(&[1, 2, 3]);
        for (name, world) in worlds {
            rig.publish_in_game(name, *world);
        }
        rig.settle();
        rig
    }

    /// Publish `name` as a bot in game on `world`, as its slot does: into
    /// the host's shared status rows. The next frame's pump projects it.
    fn publish_in_game(&mut self, name: &str, world: u16) {
        let mut rows = self
            .session
            .core
            .play_for_test()
            .unwrap()
            .statuses
            .lock()
            .unwrap();
        rows.retain(|row| row.username != name);
        rows.push(host_play::SlotStatus {
            username: name.into(),
            world: Some(world),
            connected: true,
            ingame: true,
            scene_state: 2,
            ..host_play::SlotStatus::default()
        });
    }

    /// `name`'s World / login cell, which must be clickable.
    fn world_cell(&self, name: &str) -> [f32; 4] {
        self.layout
            .world_cells
            .iter()
            .find(|(cell, _)| cell == name)
            .unwrap_or_else(|| {
                panic!(
                    "{name}'s World / login cell is not clickable: {:?}",
                    self.layout.world_cells
                )
            })
            .1
    }

    /// The open picker's rows: each world and whether it is ticked.
    fn picker(&self) -> Vec<(u16, bool)> {
        self.layout
            .world_picker
            .iter()
            .map(|entry| (entry.world, entry.ticked))
            .collect()
    }

    /// The open picker's row for `world`.
    fn picker_row(&self, world: u16) -> [f32; 4] {
        self.layout
            .world_picker
            .iter()
            .find(|entry| entry.world == world)
            .unwrap_or_else(|| {
                panic!(
                    "world {world} is not listed: {:?}",
                    self.layout.world_picker
                )
            })
            .rect
    }

    /// Open the world picker from `name`'s World / login cell.
    fn open_picker_of(&mut self, name: &str) {
        self.click(self.world_cell(name));
        self.settle();
        assert_eq!(self.session.world_picker.target(), Some(name));
    }

    /// Open the world picker from the toolbar's Switch world button.
    fn open_picker_of_marks(&mut self) {
        let button = self.button(ToolbarAction::SwitchWorld);
        assert!(!button.disabled, "Switch world is disabled: {button:?}");
        self.click(button.rect);
        self.settle();
        assert!(self.session.world_picker.is_open());
        assert_eq!(self.session.world_picker.target(), None);
    }

    /// How many commands the session has recorded.
    fn commands(&self) -> usize {
        self.session.core.commands().reports().count()
    }

    /// The members of every Switch world command to `world`, one list of
    /// subjects per command.
    fn switch_world_members(&self, world: u16) -> Vec<Vec<Subject>> {
        self.session
            .core
            .commands()
            .reports()
            .filter(|report| report.command() == OperatorCommand::SwitchWorld(world))
            .map(|report| report.members().iter().map(|m| m.subject()).collect())
            .collect()
    }

    /// The world `name`'s vault row saves for its next login.
    fn saved_world(&self, name: &str) -> Option<u16> {
        self.session
            .core
            .profile(name)
            .expect("a vault profile")
            .settings_draft()
            .world()
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

/// The nine bots of PANEL-STATE-GAP-1023, in game on world 3.
const NINE: [&str; 9] = [
    "alice", "bob", "carol", "dave", "erin", "frank", "grace", "heidi", "ivan",
];

impl Rig {
    /// Nine bots in game, each running a script, every one marked, and the
    /// last script-walk destination published for the first three: the
    /// operator's session before Stop. The status fixture is what a nav step
    /// published, not a live route.
    fn nine_marked_running_with_three_script_walks(label: &str) -> Self {
        let worlds: Vec<_> = NINE.iter().map(|name| (*name, 3)).collect();
        let mut rig = Self::with_worlds(label, &NINE, &worlds);
        rig.start_scripts(&NINE);
        rig.mark(&NINE);
        rig.publish_script_walks(&NINE[..3], Some((2550, 3348, 0)));
        rig.settle();
        rig
    }

    /// Publish (or with `None` retire) the script-walk destination of `names`.
    fn publish_script_walks(&mut self, names: &[&str], walk: Option<(i32, i32, i32)>) {
        let (x, z, level) = walk.unwrap_or((-1, -1, -1));
        let play = self.session.core.play_for_test().unwrap();
        let mut statuses = play.statuses.lock().unwrap();
        for status in statuses
            .iter_mut()
            .filter(|row| names.contains(&row.username.as_str()))
        {
            (status.walk_x, status.walk_z, status.walk_level) = (x, z, level);
        }
    }

    /// The label of every fleet row, as `name: label`: what the sidecar and
    /// the Fleet window's state column draw.
    fn labels(&self) -> Vec<String> {
        self.session
            .fleet_model
            .rows()
            .iter()
            .map(|row| format!("{}: {}", row.name(), row.brief()))
            .collect()
    }

    /// Click Stop over the marked bots, confirm, and wait until every one of
    /// `names` reports an Idle script.
    fn stop_marked(&mut self, names: &[&str]) {
        self.click_action(FleetAction::Stop);
        let plan = self
            .layout
            .plan
            .clone()
            .expect("marked Stop names its running scripts");
        self.click(plan.confirm);
        for name in names {
            self.wait_state(name, ScriptRunState::Idle);
        }
        self.settle();
        let result = self.session.core.fleet_result().unwrap();
        assert!(result.settled(), "{}", result.text());
        assert_eq!(result.counts().done, names.len(), "{}", result.text());
    }
}

fn labels_of(names: &[&str], label: &str) -> Vec<String> {
    names
        .iter()
        .map(|name| format!("{name}: {label}"))
        .collect()
}

/// PANEL-STATE-GAP-1023 (the diagnosis's reproduction): Stop must retire the
/// published script-walk label too. Nine scripts stop, all nine report Idle,
/// and the three bots whose last script walk was still published must read
/// `idle` like the six others, not `running`. The resource line counts
/// clients in game, not scripts.
#[test]
fn stopped_marked_bots_must_not_keep_running_fleet_labels() {
    let mut rig = Rig::nine_marked_running_with_three_script_walks("panel-state-gap-1023");
    assert_eq!(rig.labels(), labels_of(&NINE, "running"));
    rig.stop_marked(&NINE);
    for name in NINE {
        assert_eq!(rig.session.core.script_state(name), ScriptRunState::Idle);
    }
    assert_eq!(
        rig.labels(),
        labels_of(&NINE, "idle"),
        "Stop retained a stale walking projection"
    );
    assert_eq!(
        frontend_core::resources::format_bots(9, 9),
        "9 bots (9 in game)"
    );

    // Repeating Stop finds nothing to stop; the refusal is the core's.
    rig.session
        .submit(OperatorCommand::ClearMarks, Targets::Application);
    rig.mark(&NINE[..3]);
    rig.click_action(FleetAction::Stop);
    assert!(rig.layout.plan.is_none());
    let refusal = rig
        .session
        .error
        .as_deref()
        .expect("no scripts left to stop");
    assert!(refusal.contains("no bot to act on"), "{refusal}");
    assert!(refusal.contains("alice: no script running"), "{refusal}");
    assert_eq!(
        rig.session.core.refusals(),
        1,
        "the repeated Stop is the one refusal the command boundary logged"
    );

    // The banner clears itself after the shared timeout, with no action.
    let later = Instant::now() + frontend_core::ERROR_MESSAGE_TIMEOUT + Duration::from_secs(1);
    rig.session.expire_error(later);
    assert_eq!(rig.session.error, None, "the refusal message expired");
}

/// PANEL-STATE-GAP-1023, acceptance 1-4 through the panel: Stop retires the
/// three script walks in the host rows, every client stays in game, there
/// are zero active scripts and zero script walks, and the old labels do not
/// return after a window change, a focus change, a mark change, a filter
/// change or a new script choice.
#[test]
fn old_script_walk_labels_do_not_return_after_panel_state_changes() {
    let mut rig = Rig::nine_marked_running_with_three_script_walks("panel-state-gap-views");
    rig.stop_marked(&NINE);
    let idle = labels_of(&NINE, "idle");
    assert_eq!(rig.labels(), idle);
    for status in rig
        .session
        .core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap()
        .iter()
    {
        assert_eq!(
            status.script_walk(),
            None,
            "{} kept a target",
            status.username
        );
    }
    for row in rig.session.fleet_model.rows() {
        assert_eq!(
            (
                row.script().is_live(),
                row.script_walk(),
                row.connected(),
                row.phase()
            ),
            (false, false, true, frontend_core::Phase::Ready),
            "{}",
            row.name()
        );
    }

    // Windows: the Fleet window closed and reopened.
    rig.session.fleet_open = false;
    rig.settle();
    rig.session.fleet_open = true;
    rig.settle();
    assert_eq!(rig.labels(), idle, "window change");

    // Focus: each of the three that walked.
    for name in &NINE[..3] {
        rig.session.set_focus_for_test(name);
        rig.settle();
        assert_eq!(rig.labels(), idle, "focus on {name}");
    }

    // Marks: only the three, then by status Running (marks nothing).
    rig.session
        .submit(OperatorCommand::ClearMarks, Targets::Application);
    rig.mark(&NINE[..3]);
    assert_eq!(rig.labels(), idle, "marks");
    rig.session
        .submit(OperatorCommand::ClearMarks, Targets::Application);
    rig.settle();
    rig.click_mark_by_status("running");
    assert!(rig.marked_names().is_empty(), "nothing is running");
    assert_eq!(rig.labels(), idle, "mark by status");

    // Filters.
    for (text, shown) in [("running", 0), ("idle", 9), ("alice", 1), ("", 9)] {
        rig.session.fleet_model.set_filter_for_test(text);
        rig.settle();
        assert_eq!(rig.session.fleet_model.shown().len(), shown, "{text:?}");
        assert_eq!(rig.labels(), idle, "filter {text:?}");
    }

    // A new script choice for the fleet.
    rig.pin("Gatherer");
    assert_eq!(rig.labels(), idle, "script choice");
}

/// PANEL-STATE-GAP-1023, acceptance 6 and 7 (panel half): ivan has only a
/// WalkTo walk. The eight others, three with script walks, stop. Eight read
/// `idle` and ivan still reads `running`, from his own manual-walk fact. A
/// Stop aimed at him alone is refused for want of a script.
#[test]
fn a_manual_walk_stays_visible_after_a_script_stop_and_stop_does_not_target_it() {
    let mut rig = Rig::nine_marked_running_with_three_script_walks("panel-state-gap-manual");
    let arm = std::sync::Arc::new(std::sync::Mutex::new(host_play::WalkArm::default()));
    rig.walks
        .lock()
        .unwrap()
        .insert("ivan".to_owned(), std::sync::Arc::clone(&arm));
    arm.lock().unwrap().route = Some(std::sync::Arc::new(nav_planner::router::Route {
        legs: Vec::new(),
        dest: api::snapshot::WorldTile {
            x: 3220,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    }));
    // Ivan's own script goes first, so the walk is all he is doing.
    rig.session
        .core
        .play_for_test()
        .unwrap()
        .script_stop("ivan");
    rig.wait_state("ivan", ScriptRunState::Idle);
    rig.session
        .submit(OperatorCommand::ClearMarks, Targets::Application);
    rig.mark(&NINE[..8]);
    rig.stop_marked(&NINE[..8]);

    let mut want = labels_of(&NINE, "idle");
    want[8] = "ivan: running".into();
    assert_eq!(rig.labels(), want);
    let ivan = rig.session.fleet_model.rows().last().unwrap();
    assert_eq!(
        (ivan.script(), ivan.script_walk(), ivan.manual_walk()),
        (ScriptRunState::Idle, false, true)
    );
    assert!(rig.session.core.walk_route_armed("ivan"));

    rig.session
        .submit(OperatorCommand::ClearMarks, Targets::Application);
    rig.mark(&["ivan"]);
    rig.click_action(FleetAction::Stop);
    assert!(rig.layout.plan.is_none());
    let refusal = rig.session.error.as_deref().expect("nothing to stop");
    assert!(refusal.contains("ivan: no script running"), "{refusal}");
    assert!(arm.lock().unwrap().route.is_some(), "Stop left the walk");
    assert_eq!(rig.labels(), want);

    arm.lock().unwrap().route = None;
    rig.settle();
    assert_eq!(rig.labels(), labels_of(&NINE, "idle"));
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

/// The Profile cell of `name` in `layout` (the shown rows' order).
fn name_rect(session: &Session, layout: &FleetLayout, name: &str) -> [f32; 4] {
    let at = session
        .fleet_model
        .shown_rows()
        .position(|(_, row)| row.name() == name)
        .unwrap_or_else(|| panic!("{name} is not shown"));
    layout.names[at]
}

/// A click on a loaded bot's name focuses it, in Load order (the name is
/// also the drag handle) and in a view sort; its Mark box only marks it,
/// and a profile that is not loaded focuses nothing. Before, a name click
/// did nothing.
#[test]
fn a_name_click_focuses_the_bot_and_its_mark_box_only_marks_it() {
    let mut rig = Rig::with_play(
        "fleet-name-focus",
        &["alice", "bob", "carol"],
        &["alice", "bob"],
    );
    let focused = |rig: &Rig| rig.session.focused_name();
    assert_eq!(focused(&rig).as_deref(), Some("alice"));

    let mark = rig
        .layout
        .mark_boxes
        .iter()
        .find(|(name, _)| name == "bob")
        .expect("bob's Mark box")
        .1;
    rig.click(mark);
    assert_eq!(rig.marked_names(), ["bob"]);
    assert_eq!(
        focused(&rig).as_deref(),
        Some("alice"),
        "a mark never focuses"
    );

    rig.click(name_rect(&rig.session, &rig.layout, "bob"));
    assert_eq!(
        focused(&rig).as_deref(),
        Some("bob"),
        "Load order name click"
    );
    assert_eq!(rig.marked_names(), ["bob"], "a focus never marks");

    rig.click(name_rect(&rig.session, &rig.layout, "carol"));
    assert_eq!(focused(&rig).as_deref(), Some("bob"), "carol is not loaded");

    rig.session
        .fleet_model
        .set_order_for_test(frontend_core::FleetOrder::Name);
    rig.settle();
    rig.click(name_rect(&rig.session, &rig.layout, "alice"));
    assert_eq!(focused(&rig).as_deref(), Some("alice"), "Name sort click");
    assert_eq!(rig.marked_names(), ["bob"]);
}

#[cfg(feature = "js")]
const THIEVER_TS: &str = "export const SETTINGS = { target: { type: 'string', default: 'Man' } };\nexport default class T extends LoopingBot { override loop() {} }\n";

/// The Scripts window's Apply to same-card members entry raises the fleet
/// plan's confirmation, and the Scripts section draws it only while no
/// fleet toolbar is on screen: not beside the Fleet window's (or the rail's,
/// or the strip's), and alone when every toolbar is gone. The old entry
/// drew its own prepared-sync row beside the Fleet window's.
#[cfg(feature = "js")]
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
    // The Fleet window closes; the strip's fleet section (the rail is not
    // shown) still carries the toolbar, which this rig does not draw.
    rig.session.fleet_open = false;
    assert!(rig.session.fleet_toolbar_shown());
    for _ in 0..3 {
        assert_eq!(rig.frame_with_scripts_section(), 0, "the strip's toolbar");
    }
    crate::ui_state::set_panel_section_visible(&mut rig.session.ui, "fleet", false);
    for _ in 0..3 {
        assert_eq!(rig.frame_with_scripts_section(), 1, "no toolbar on screen");
    }
}

/// The World / login cell of a bot on a public world is clickable and opens
/// the one world picker, which lists the server's worlds (core's cached
/// list) with the bot's own ticked. Rows that cannot switch (a profile that
/// is not loaded, a bot with no world yet) have no clickable cell, and a
/// click away closes the picker having changed nothing. Before, the cell was
/// plain text.
#[test]
fn the_world_cell_opens_the_world_picker_with_the_bots_own_world_ticked() {
    let mut rig = Rig::with_worlds(
        "fleet-world-cell",
        &["alice", "bob", "carol", "dave"],
        &[("alice", 2), ("bob", 3), ("carol", 1)],
    );
    // dave is not loaded; carol is loaded but publishes no world yet.
    rig.session
        .core
        .play_for_test()
        .unwrap()
        .statuses
        .lock()
        .unwrap()
        .retain(|row| row.username != "carol");
    rig.settle();
    let clickable: Vec<&str> = rig
        .layout
        .world_cells
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(clickable, ["alice", "bob"]);
    assert!(
        !rig.session.world_picker.is_open(),
        "nothing opens by itself"
    );
    let commands = rig.commands();

    rig.open_picker_of("alice");
    assert_eq!(rig.picker(), [(1, false), (2, true), (3, false)]);
    let labels: Vec<&str> = rig
        .layout
        .world_picker
        .iter()
        .map(|entry| entry.label.as_str())
        .collect();
    assert_eq!(labels, ["World 1", "World 2", "World 3"]);
    assert!(
        rig.session.core.fleet_plan().is_none() && rig.commands() == commands,
        "opening the picker plans and submits nothing"
    );

    // A click away closes it, and picks nothing.
    rig.click([1200.0, 700.0, 1202.0, 702.0]);
    assert!(!rig.session.world_picker.is_open());
    assert!(rig.layout.world_picker.is_empty());
    assert_eq!(rig.commands(), commands);

    // bob's cell ticks his own world.
    rig.open_picker_of("bob");
    assert_eq!(rig.picker(), [(1, false), (2, false), (3, true)]);
}

/// Picking the world the bot is already on closes the picker and does
/// nothing: no plan, no command, no write, no banner.
#[test]
fn picking_the_bots_own_world_submits_nothing() {
    let mut rig = Rig::with_worlds("fleet-world-same", &["alice"], &[("alice", 2)]);
    let commands = rig.commands();
    rig.open_picker_of("alice");
    rig.click(rig.picker_row(2));

    assert!(!rig.session.world_picker.is_open());
    assert!(rig.session.core.fleet_plan().is_none());
    assert!(rig.layout.plan.is_none());
    assert_eq!(rig.commands(), commands);
    assert_eq!(rig.session.error, None);
    rig.session.core.flush_writes();
    assert_eq!(rig.saved_world("alice"), None, "nothing was saved");
}

/// Picking another world stages that bot alone and plans a Switch world for
/// it. The Fleet window's plan row shows core's prompt (Confirm and Cancel),
/// and nothing runs or is saved before Confirm; Cancel drops the plan and
/// writes nothing; Confirm runs one command over that bot only, and the one
/// result line follows it. A marked bot is not added to a bot's own pick.
#[test]
fn picking_another_world_plans_that_bot_and_runs_only_on_confirm() {
    let mut rig = Rig::with_worlds(
        "fleet-world-plan",
        &["alice", "bob"],
        &[("alice", 2), ("bob", 2)],
    );
    rig.mark(&["bob"]);
    let alice = rig.session.core.profile_identity("alice").unwrap();

    rig.open_picker_of("alice");
    rig.click(rig.picker_row(3));
    assert!(!rig.session.world_picker.is_open(), "a pick closes it");
    let plan = rig.session.core.fleet_plan().expect("the plan waits");
    assert_eq!(plan.action(), FleetAction::SwitchWorld(3));
    assert!(plan.needs_confirm(), "a Switch world plan always asks");
    assert!(matches!(plan.scope(), PlanScope::One));
    assert_eq!(plan.target_names().collect::<Vec<_>>(), ["alice"]);
    let prompt = plan.prompt().to_owned();
    assert!(
        prompt.starts_with("Move 1 bot to world 3: alice."),
        "{prompt}"
    );
    assert!(!prompt.contains("bob"), "the mark did not join: {prompt}");
    let row = rig.layout.plan.clone().expect("the plan row is drawn");
    assert_eq!(
        row.prompt, prompt,
        "the row draws the core's prompt verbatim"
    );
    assert!(rig.switch_world_members(3).is_empty(), "nothing ran yet");

    // Cancel: the plan goes; nothing ran and nothing was saved.
    rig.click(row.cancel);
    assert!(rig.session.core.fleet_plan().is_none());
    assert!(rig.layout.plan.is_none());
    rig.session.core.flush_writes();
    assert!(rig.switch_world_members(3).is_empty());
    assert_eq!(rig.saved_world("alice"), None);
    assert!(rig.session.core.fleet_result().is_none());

    // Again, then Confirm: one command with one member, alice.
    rig.open_picker_of("alice");
    rig.click(rig.picker_row(3));
    let row = rig
        .layout
        .plan
        .clone()
        .expect("the plan row is drawn again");
    rig.click(row.confirm);
    assert!(rig.session.core.fleet_plan().is_none());
    assert_eq!(rig.switch_world_members(3), [[Subject::Profile(alice)]]);
    rig.session.core.flush_writes();
    assert_eq!(rig.saved_world("alice"), Some(3), "saved before the relog");
    assert_eq!(rig.saved_world("bob"), None);
    rig.settle();
    let result = rig.session.core.fleet_result().expect("one result");
    assert_eq!(result.action(), FleetAction::SwitchWorld(3));
    assert_eq!(result.counts().pending, 1, "{}", result.text());
    let (line, _) = rig.layout.result.clone().expect("the result line is drawn");
    assert_eq!(line, result.text());
}

/// A pick from the rail or the Grid plans; its plan row shows in every
/// fleet toolbar. The Fleet window opens to show it only when no toolbar is
/// on screen (here: the strip's fleet section turned off).
#[test]
fn a_pick_opens_the_fleet_window_only_when_no_toolbar_shows_the_confirmation() {
    let mut rig = Rig::with_worlds("fleet-world-opens", &["alice"], &[("alice", 2)]);
    // The rail's click path: the session opens the picker for the bot.
    rig.session.fleet_open = false;
    assert!(rig.session.open_world_picker(Some("alice")));
    rig.settle();
    assert_eq!(rig.picker(), [(1, false), (2, true), (3, false)]);
    rig.click(rig.picker_row(1));
    assert!(rig.session.core.fleet_plan().is_some());
    assert!(
        !rig.session.fleet_open,
        "the strip's toolbar shows the plan"
    );
    rig.session.cancel_fleet_plan();
    rig.settle();

    crate::ui_state::set_panel_section_visible(&mut rig.session.ui, "fleet", false);
    assert!(!rig.session.fleet_toolbar_shown());
    assert!(rig.session.open_world_picker(Some("alice")));
    rig.settle();
    rig.click(rig.picker_row(1));
    assert!(rig.session.fleet_open, "the Fleet window opened");
    assert!(rig.session.core.fleet_plan().is_some());
    let drawn = rig
        .layout
        .plan
        .clone()
        .expect("the window draws the plan row");
    assert_eq!(
        Some(drawn.prompt.as_str()),
        rig.session.core.fleet_plan().map(|plan| plan.prompt())
    );
}

/// The toolbar's Switch world button opens the same picker for the marked
/// bots (every loaded bot shown when none is marked): no world is ticked,
/// a pick plans one command over the marks, hidden ones too, and names the
/// bots already on that world as skipped.
#[test]
fn the_toolbar_switch_world_plans_over_the_marks() {
    let mut rig = Rig::with_worlds(
        "fleet-world-marked",
        &["alice", "bob", "carol", "dave"],
        &[("alice", 2), ("bob", 2), ("carol", 3), ("dave", 2)],
    );
    rig.mark(&["alice", "bob", "carol"]);
    rig.session.fleet_model.set_filter_for_test("alice");
    rig.settle();
    assert_eq!(rig.layout.marks_text, "3 marked, 2 not shown");
    assert_eq!(
        rig.button(ToolbarAction::SwitchWorld).label,
        "Switch world 3###fleet-action-7"
    );

    rig.open_picker_of_marks();
    assert_eq!(rig.picker(), [(1, false), (2, false), (3, false)]);
    rig.click(rig.picker_row(3));
    let plan = rig.session.core.fleet_plan().expect("the plan waits");
    assert_eq!(plan.action(), FleetAction::SwitchWorld(3));
    assert!(plan.needs_confirm());
    assert!(matches!(plan.scope(), PlanScope::Marked { hidden: 2 }));
    assert_eq!(plan.target_names().collect::<Vec<_>>(), ["alice", "bob"]);
    let prompt = plan.prompt().to_owned();
    assert!(
        prompt.starts_with("Move 2 marked bots to world 3 (2 not shown): alice, bob."),
        "{prompt}"
    );
    assert!(
        prompt.contains("Skipped 1: carol: already on this world."),
        "{prompt}"
    );
    assert!(!prompt.contains("dave"), "dave is not marked: {prompt}");
    assert!(rig.switch_world_members(3).is_empty(), "nothing ran yet");
    let row = rig.layout.plan.clone().expect("the plan row is drawn");
    assert_eq!(row.prompt, prompt);
    rig.click(row.confirm);
    let members = rig.switch_world_members(3);
    assert_eq!(members.len(), 1, "one command");
    let alice = rig.session.core.profile_identity("alice").unwrap();
    let bob = rig.session.core.profile_identity("bob").unwrap();
    assert_eq!(
        members[0],
        [Subject::Profile(alice), Subject::Profile(bob)],
        "the marked bots that can move, in order"
    );
    rig.session.core.flush_writes();
    assert_eq!(rig.saved_world("alice"), Some(3));
    assert_eq!(rig.saved_world("bob"), Some(3));
    assert_eq!(rig.saved_world("carol"), None, "already there: no write");
    assert_eq!(rig.saved_world("dave"), None, "not marked");
}

/// With nothing marked the toolbar's Switch world covers the loaded bots
/// the filter shows, and the plan asks first.
#[test]
fn with_nothing_marked_switch_world_covers_the_loaded_shown_bots() {
    let mut rig = Rig::with_worlds(
        "fleet-world-shown",
        &["alice", "bob", "carol"],
        &[("alice", 2), ("bob", 2)],
    );
    assert!(rig.session.fleet_model.marks().is_empty());
    rig.open_picker_of_marks();
    rig.click(rig.picker_row(1));
    let plan = rig.session.core.fleet_plan().expect("the plan waits");
    assert!(matches!(plan.scope(), PlanScope::AllShown));
    assert_eq!(plan.target_names().collect::<Vec<_>>(), ["alice", "bob"]);
    assert!(
        plan.prompt()
            .starts_with("Move 2 shown bots to world 1: alice, bob."),
        "{}",
        plan.prompt()
    );
    assert!(rig.switch_world_members(1).is_empty());
}

/// A server with one world offers no switching anywhere in the window: no
/// World / login cell is clickable (the column is plain text), and the
/// toolbar's Switch world button is disabled with the reason, so a click on
/// it opens nothing and plans nothing.
#[test]
fn a_server_with_one_world_offers_no_picker() {
    let mut rig = Rig::with_play("fleet-world-local", &["alice", "bob"], &["alice", "bob"]);
    rig.publish_in_game("alice", 2);
    rig.publish_in_game("bob", 2);
    rig.mark(&["alice"]);
    assert!(rig.session.core.switch_world_choices().is_none());
    assert!(!rig.session.can_switch_world());
    assert!(
        rig.layout.world_cells.is_empty(),
        "{:?}",
        rig.layout.world_cells
    );
    let commands = rig.commands();

    let button = rig.button(ToolbarAction::SwitchWorld);
    assert!(button.disabled);
    assert_eq!(button.tooltip, crate::fleet_toolbar::NO_WORLDS);
    rig.click(button.rect);
    rig.settle();
    assert!(!rig.session.world_picker.is_open());
    assert!(rig.layout.world_picker.is_empty());
    assert!(rig.session.core.fleet_plan().is_none());
    assert_eq!(rig.commands(), commands);
    assert_eq!(rig.session.error, None);
    assert!(!rig.session.open_world_picker(Some("alice")));
    assert!(!rig.session.open_world_picker(None));
}

/// A steady frame with the picker open rebuilds no label (the cached
/// buffers keep their addresses) and submits nothing.
#[test]
fn a_steady_frame_with_the_picker_open_keeps_its_labels_and_submits_nothing() {
    let mut rig = Rig::with_worlds("fleet-world-steady", &["alice"], &[("alice", 2)]);
    rig.open_picker_of("alice");
    let buffers = rig.session.world_picker.buffers();
    assert_eq!(buffers.len(), 4, "three worlds and the heading");
    let commands = rig.commands();
    for _ in 0..8 {
        rig.frame(AWAY, false);
    }
    assert_eq!(rig.session.world_picker.buffers(), buffers);
    assert_eq!(rig.commands(), commands);
    assert!(rig.session.world_picker.is_open());
    assert!(rig.session.core.fleet_plan().is_none());
    assert_eq!(rig.picker(), [(1, false), (2, true), (3, false)]);
}

/// LOGOUT-RETRY-1025: a bot whose login is refused and retried is reached
/// by the Fleet window's Log out all, which parks it so the slot makes no
/// further login attempt, while a bot that never logged in is skipped.
#[test]
fn logout_retry_all_takes_a_retrying_login_and_skips_a_bot_that_never_logged_in() {
    let mut rig = Rig::with_play("fleet-logout-retry", &["alice", "bob"], &["alice", "bob"]);
    let arm = rig
        .session
        .core
        .play_for_test()
        .unwrap()
        .arm("bob")
        .expect("bob's arm");
    arm.arm_explicit_login();
    {
        let play = rig.session.core.play_for_test().unwrap();
        play.statuses.lock().unwrap().push(host_play::SlotStatus {
            username: "bob".into(),
            startup_phase: host_play::StartupPhase::Error,
            startup_phase_started: Instant::now(),
            error: Some("Error connecting to server.".into()),
            ..host_play::SlotStatus::default()
        });
    }
    rig.settle();
    let bob = rig.session.core.fleet_view().row("bob").unwrap().snapshot();
    assert_eq!(
        (bob.phase(), bob.retrying()),
        (frontend_core::Phase::LoginError, true)
    );
    assert!(arm.wants_login(), "the slot would retry");

    let logout = rig.action(FleetAction::Logout);
    assert!(!logout.disabled, "Log out all is enabled: {logout:?}");
    rig.click_action(FleetAction::Logout);
    let plan = rig.layout.plan.clone().expect("the plan row is drawn");
    assert!(
        plan.prompt.starts_with("Log out 1 shown bot: bob."),
        "{}",
        plan.prompt
    );
    assert!(
        plan.prompt.contains("alice: not logged in"),
        "{}",
        plan.prompt
    );
    rig.click(plan.confirm);
    rig.settle();
    assert!(arm.login_latched(), "bob is parked logged out");
    assert!(!arm.wants_login(), "no further login attempt");
    let alice = rig
        .session
        .core
        .play_for_test()
        .unwrap()
        .arm("alice")
        .unwrap();
    assert!(!alice.login_latched(), "alice never logged in");
}
