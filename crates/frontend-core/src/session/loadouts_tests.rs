//! The loadout rows of the command seam over a real session and a lent
//! [`LoadoutForm`]: what each row does to the form, and the terminal its
//! member settles with. The rows write through an in-memory store that can
//! refuse writes or change under the form, as the operator store does.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use api::identity::Settlement;
use api::loadout::{CarryEntry, Loadout};
use api::local_state::{LoadoutOp, StoreVersion};
use api::snapshot::{GameSnapshot, ItemActionFamily, ItemContainer, ItemView};
use host_play::{InstancePermit, SlotStatus};
use script::{LoadoutsBacking, LoadoutsBaseline, LoadoutsSaveError};

use super::*;
use crate::command::{CommandFailure, Completion, SubjectKind};
use crate::scripts::{Loadouts, LoadoutsErrorKind};
use crate::Targets;

fn version(n: u64) -> StoreVersion {
    (0..n).fold(StoreVersion::INITIAL, |v, _| v.checked_next().unwrap())
}

/// The operator store, in memory.
#[derive(Default)]
struct Memory {
    state: Mutex<(Vec<Loadout>, u64)>,
    /// Refuse every write, as a full disk does.
    refuse_writes: AtomicBool,
}

impl Memory {
    fn seeded(rows: &[Loadout]) -> Arc<Self> {
        let memory = Self::default();
        memory.state.lock().unwrap().0 = rows.to_vec();
        Arc::new(memory)
    }

    fn baseline(&self) -> LoadoutsBaseline {
        let state = self.state.lock().unwrap();
        LoadoutsBaseline {
            rows: state.0.clone(),
            version: version(state.1),
        }
    }

    fn names(&self) -> Vec<String> {
        self.baseline()
            .rows
            .into_iter()
            .map(|row| row.name)
            .collect()
    }

    /// Another writer's edit, landing after a form loaded the rows.
    fn outside(&self, edit: impl FnOnce(&mut Vec<Loadout>)) {
        let mut state = self.state.lock().unwrap();
        edit(&mut state.0);
        state.1 += 1;
    }
}

impl LoadoutsBacking for Memory {
    fn load(&self) -> Result<LoadoutsBaseline, String> {
        Ok(self.baseline())
    }

    fn commit(
        &self,
        ops: Vec<LoadoutOp>,
        expected: StoreVersion,
    ) -> Result<LoadoutsBaseline, LoadoutsSaveError> {
        if self.refuse_writes.load(Ordering::SeqCst) {
            return Err(LoadoutsSaveError::Failed("the disk is full".into()));
        }
        let mut state = self.state.lock().unwrap();
        if version(state.1) != expected {
            drop(state);
            return Err(LoadoutsSaveError::Conflict {
                latest: self.baseline(),
                reason: "version conflict".into(),
            });
        }
        for op in ops {
            match op {
                LoadoutOp::Upsert(row) => match state.0.iter_mut().find(|old| old.name == row.name)
                {
                    Some(old) => *old = row,
                    None => state.0.push(row),
                },
                LoadoutOp::Remove { name } => state.0.retain(|old| old.name != name),
                LoadoutOp::Rename { from, loadout } => {
                    if let Some(at) = state.0.iter().position(|old| old.name == from) {
                        state.0[at] = loadout;
                    }
                }
            }
            state.1 += 1;
        }
        drop(state);
        Ok(self.baseline())
    }
}

/// A form over `memory`, loaded now.
fn form_over(memory: &Arc<Memory>) -> LoadoutForm {
    LoadoutForm::over(Loadouts::over_backing_for_test(
        Arc::clone(memory) as Arc<dyn LoadoutsBacking>
    ))
}

/// A surface with no slot devices that lends the loadout form, when it has
/// one.
struct FormSurface<'a> {
    form: Option<&'a mut LoadoutForm>,
}

impl SlotSurface for FormSurface<'_> {
    type Io = ();

    fn attach(
        &mut self,
        _name: &str,
        profile: &mut crate::SpawnProfile<'_>,
        _retained: Option<()>,
    ) -> crate::SlotAttach<()> {
        profile.set_raster(crate::command::RasterMode::Off);
        crate::SlotAttach::new((), None, None)
    }

    fn lifetime_reset(&mut self, _name: &str, _walk_was_armed: bool) {}

    fn released(&mut self, _name: &str) {}

    fn models(&mut self) -> crate::Models<'_> {
        match self.form.as_deref_mut() {
            Some(form) => crate::Models::none().with_loadouts(form),
            None => crate::Models::none(),
        }
    }

    fn draw_changed(&mut self, _switch: crate::command::DrawSwitch, _on: bool) {}
}

fn core() -> OperatorSession<()> {
    OperatorSession::new(InstancePermit::SkipLock)
}

/// How a command's one member ended, and the detail the owner gave.
#[derive(Debug, PartialEq)]
enum Ended {
    Done,
    Skipped(SkipReason),
    Rejected(Rejection),
    Failed(FailureKind),
}

type Outcome = (Ended, Option<String>);

fn outcome_of(core: &OperatorSession<()>, request: RequestId) -> Outcome {
    let report = core
        .commands()
        .report(request)
        .expect("the command's report");
    assert!(report.is_settled(), "a loadout row settles at dispatch");
    let member = &report.members()[0];
    let ended = match member.terminal().expect("settled") {
        Settlement::Completed(Completion::Done) => Ended::Done,
        Settlement::Completed(Completion::Skipped(reason)) => Ended::Skipped(*reason),
        Settlement::Failed(CommandFailure::Rejected(rejection)) => Ended::Rejected(*rejection),
        Settlement::Failed(CommandFailure::Failed(kind)) => Ended::Failed(*kind),
        _ => panic!("unexpected terminal {:?}", member.terminal()),
    };
    (ended, member.detail().map(str::to_owned))
}

/// Submit `command` for `targets` with `form` lent.
fn submit_for(
    core: &mut OperatorSession<()>,
    form: &mut LoadoutForm,
    command: OperatorCommand,
    targets: Targets<'_>,
) -> Outcome {
    let submitted = core
        .submit(command, targets, &mut FormSurface { form: Some(form) })
        .expect("the command opens");
    outcome_of(core, submitted.request())
}

/// Submit the application row `command` with `form` lent.
fn submit(
    core: &mut OperatorSession<()>,
    form: &mut LoadoutForm,
    command: OperatorCommand,
) -> Outcome {
    submit_for(core, form, command, Targets::Application)
}

fn melee() -> Loadout {
    Loadout::new("melee").with_carry("Lobster", 8)
}

fn names(form: &LoadoutForm) -> Vec<String> {
    form.names()
}

#[test]
fn the_rows_are_application_rows_except_the_copy() {
    for row in [
        LoadoutRow::Select,
        LoadoutRow::Edit(LoadoutField::Name),
        LoadoutRow::Create,
        LoadoutRow::Duplicate,
        LoadoutRow::Save,
        LoadoutRow::Delete,
    ] {
        assert_eq!(row.command().subject(), SubjectKind::Application, "{row:?}");
    }
    assert_eq!(
        LoadoutRow::CopyEquipment.command().subject(),
        SubjectKind::Profile
    );
}

#[test]
fn select_makes_the_selected_row_the_draft_and_drops_unsaved_edits() {
    let memory = Memory::seeded(&[melee(), Loadout::new("range")]);
    let mut form = form_over(&memory);
    let mut core = core();
    assert!(
        form.draft().is_none(),
        "nothing is selected before a Select"
    );

    form.stage_selection(1);
    assert_eq!(
        submit(&mut core, &mut form, OperatorCommand::SelectLoadout),
        (Ended::Done, None)
    );
    assert_eq!(form.draft().unwrap().name, "range");
    let generation = form.generation();

    // An unsaved edit of this draft is dropped by the next Select.
    form.draft_mut().unwrap().name = "unsaved".into();
    form.stage_selection(0);
    assert_eq!(
        submit(&mut core, &mut form, OperatorCommand::SelectLoadout).0,
        Ended::Done
    );
    assert_eq!(form.draft().unwrap(), &melee());
    assert_ne!(form.generation(), generation, "the draft was replaced");
}

#[test]
fn select_with_no_row_skips_and_leaves_no_draft() {
    let memory = Memory::seeded(&[melee()]);
    let mut form = form_over(&memory);
    let mut core = core();
    form.stage_selection(0);
    submit(&mut core, &mut form, OperatorCommand::SelectLoadout);
    assert!(form.draft().is_some());

    form.stage_selection(5);
    assert_eq!(
        submit(&mut core, &mut form, OperatorCommand::SelectLoadout).0,
        Ended::Skipped(SkipReason::NothingSelected)
    );
    assert!(form.draft().is_none(), "the stale draft is dropped");
}

#[test]
fn an_edit_commits_and_a_status_about_the_last_save_no_longer_describes_it() {
    let memory = Memory::seeded(&[melee()]);
    let mut form = form_over(&memory);
    let mut core = core();
    submit(&mut core, &mut form, OperatorCommand::SelectLoadout);
    assert_eq!(
        submit(&mut core, &mut form, OperatorCommand::SaveLoadout).0,
        Ended::Done
    );
    assert_eq!(form.status(), "Saved.");

    form.draft_mut().unwrap().carry[0].qty = 9;
    assert_eq!(
        submit(
            &mut core,
            &mut form,
            OperatorCommand::EditLoadoutDraft(LoadoutField::Supplies)
        ),
        (Ended::Done, None)
    );
    assert_eq!(form.status(), "", "the saved notice no longer applies");
    assert_eq!(form.draft().unwrap().carry[0].qty, 9);
    assert_eq!(
        form.loadouts()[0].carry[0].qty,
        8,
        "an edit writes nothing; Save does"
    );
    assert_eq!(memory.baseline().rows[0].carry[0].qty, 8);
}

#[test]
fn an_edit_or_a_save_with_no_draft_is_not_staged() {
    let memory = Memory::seeded(&[]);
    let mut form = form_over(&memory);
    let mut core = core();
    for command in [
        OperatorCommand::EditLoadoutDraft(LoadoutField::Name),
        OperatorCommand::SaveLoadout,
    ] {
        assert_eq!(
            submit(&mut core, &mut form, command),
            (
                Ended::Rejected(Rejection::NotStaged),
                Some("No loadout selected.".to_string())
            ),
            "{command:?}"
        );
        assert_eq!(form.status(), "No loadout selected.");
    }
}

#[test]
fn create_writes_a_new_row_at_once_and_selects_it() {
    let memory = Memory::seeded(&[melee()]);
    let mut form = form_over(&memory);
    let mut core = core();
    assert_eq!(
        submit(&mut core, &mut form, OperatorCommand::CreateLoadout),
        (Ended::Done, None)
    );
    assert_eq!(names(&form), ["melee", "loadout"]);
    assert_eq!(memory.names(), ["melee", "loadout"], "written at once");
    assert_eq!(form.selected(), 1);
    assert_eq!(form.draft().unwrap().name, "loadout");
    assert_eq!(form.status(), "Saved.");

    submit(&mut core, &mut form, OperatorCommand::CreateLoadout);
    assert_eq!(names(&form), ["melee", "loadout", "loadout 2"]);
}

#[test]
fn a_refused_write_settles_failed_and_rolls_the_rows_back() {
    let memory = Memory::seeded(&[melee()]);
    let mut form = form_over(&memory);
    let mut core = core();
    submit(&mut core, &mut form, OperatorCommand::SelectLoadout);
    memory.refuse_writes.store(true, Ordering::SeqCst);

    let (ended, detail) = submit(&mut core, &mut form, OperatorCommand::CreateLoadout);
    assert_eq!(ended, Ended::Failed(FailureKind::WriteFailed));
    assert_eq!(detail.as_deref(), Some("the disk is full"));
    assert_eq!(names(&form), ["melee"], "the added row is rolled back");
    assert!(!form.store().is_dirty());
    assert_eq!(form.status(), "Save failed: the disk is full");
    assert_eq!(form.selected(), 0);
    assert_eq!(memory.names(), ["melee"]);

    // The same for a draft that cannot be written: the draft is kept for
    // another try.
    form.draft_mut().unwrap().name = "melee2".into();
    assert_eq!(
        submit(&mut core, &mut form, OperatorCommand::SaveLoadout).0,
        Ended::Failed(FailureKind::WriteFailed)
    );
    assert_eq!(names(&form), ["melee"]);
    assert_eq!(form.draft().unwrap().name, "melee2");
    assert_eq!(form.status(), "Save failed: the disk is full");
}

#[test]
fn a_form_with_nowhere_to_save_settles_failed_and_keeps_its_rows() {
    let mut form = LoadoutForm::over(Loadouts::detached(vec![melee()]));
    let mut core = core();
    let (ended, detail) = submit(&mut core, &mut form, OperatorCommand::CreateLoadout);
    assert_eq!(ended, Ended::Failed(FailureKind::WriteFailed));
    assert_eq!(
        detail.as_deref(),
        Some("loadouts are not bound to the local-state service")
    );
    assert_eq!(names(&form), ["melee"]);
}

#[test]
fn duplicate_copies_the_saved_row_not_the_draft_and_selects_the_copy() {
    let memory = Memory::seeded(&[melee(), Loadout::new("range")]);
    let mut form = form_over(&memory);
    let mut core = core();
    form.stage_selection(0);
    submit(&mut core, &mut form, OperatorCommand::SelectLoadout);
    form.draft_mut().unwrap().carry[0].qty = 99;

    assert_eq!(
        submit(&mut core, &mut form, OperatorCommand::DuplicateLoadout),
        (Ended::Done, None)
    );
    assert_eq!(names(&form), ["melee", "range", "melee copy"]);
    assert_eq!(memory.names(), ["melee", "range", "melee copy"]);
    assert_eq!(form.selected(), 2);
    assert_eq!(
        form.draft().unwrap().carry[0],
        CarryEntry::new("Lobster", 8),
        "the copy holds what was saved"
    );
    assert_eq!(form.status(), "Saved.");
}

#[test]
fn duplicate_and_delete_with_nothing_selected_skip() {
    let memory = Memory::seeded(&[]);
    let mut form = form_over(&memory);
    let mut core = core();
    for command in [
        OperatorCommand::DuplicateLoadout,
        OperatorCommand::DeleteLoadout,
    ] {
        assert_eq!(
            submit(&mut core, &mut form, command),
            (Ended::Skipped(SkipReason::NothingSelected), None),
            "{command:?}"
        );
    }
    assert!(names(&form).is_empty());
}

#[test]
fn save_replaces_the_selected_row_and_a_rename_keeps_its_place() {
    let memory = Memory::seeded(&[melee(), Loadout::new("range"), Loadout::new("tank")]);
    let mut form = form_over(&memory);
    let mut core = core();
    form.stage_selection(0);
    submit(&mut core, &mut form, OperatorCommand::SelectLoadout);
    let draft = form.draft_mut().unwrap();
    draft.name = "melee2".into();
    draft.carry[0].qty = 9;

    assert_eq!(
        submit(&mut core, &mut form, OperatorCommand::SaveLoadout),
        (Ended::Done, None)
    );
    assert_eq!(names(&form), ["melee2", "range", "tank"]);
    assert_eq!(memory.names(), ["melee2", "range", "tank"]);
    assert_eq!(memory.baseline().rows[0].carry[0].qty, 9);
    assert_eq!(form.selected(), 0);
    assert_eq!(form.draft().unwrap().name, "melee2");
    assert_eq!(form.status(), "Saved.");
}

#[test]
fn a_draft_with_no_row_to_replace_is_added() {
    let memory = Memory::seeded(&[]);
    let mut form = form_over(&memory);
    let mut core = core();
    form.stage_draft(Loadout::new("fish").with_carry("net", 1));
    assert_eq!(
        submit(&mut core, &mut form, OperatorCommand::SaveLoadout),
        (Ended::Done, None)
    );
    assert_eq!(memory.names(), ["fish"]);
    assert_eq!(form.selected(), 0);
}

#[test]
fn save_refuses_a_blank_name_and_a_name_another_row_has() {
    let memory = Memory::seeded(&[melee(), Loadout::new("range")]);
    let mut form = form_over(&memory);
    let mut core = core();
    form.stage_selection(1);
    submit(&mut core, &mut form, OperatorCommand::SelectLoadout);

    form.draft_mut().unwrap().name = "  ".into();
    assert_eq!(
        submit(&mut core, &mut form, OperatorCommand::SaveLoadout),
        (
            Ended::Rejected(Rejection::NotApplicable),
            Some("Name is required.".to_string())
        )
    );
    assert_eq!(form.status(), "Name is required.");

    form.draft_mut().unwrap().name = "melee".into();
    assert_eq!(
        submit(&mut core, &mut form, OperatorCommand::SaveLoadout),
        (
            Ended::Rejected(Rejection::NotApplicable),
            Some("A loadout named \"melee\" already exists.".to_string())
        )
    );
    assert_eq!(memory.names(), ["melee", "range"], "nothing was written");
    assert_eq!(names(&form), ["melee", "range"]);
    assert_eq!(form.selected(), 1);
    assert_eq!(form.draft().unwrap().name, "melee", "the draft is kept");
}

/// Another writer changed the rows after this form loaded them: the save is
/// refused with the conflict, nothing of the other writer is overwritten,
/// the form shows what the store holds now and the selection follows its
/// row by name.
#[test]
fn a_conflicting_save_settles_failed_and_the_selection_follows_its_row() {
    let memory = Memory::seeded(&[Loadout::new("first"), melee()]);
    let mut form = form_over(&memory);
    let mut core = core();
    form.stage_selection(1);
    submit(&mut core, &mut form, OperatorCommand::SelectLoadout);
    memory.outside(|rows| {
        rows.retain(|row| row.name != "first");
        rows.push(Loadout::new("tank"));
    });

    form.draft_mut().unwrap().name = "melee2".into();
    let (ended, detail) = submit(&mut core, &mut form, OperatorCommand::SaveLoadout);
    assert_eq!(ended, Ended::Failed(FailureKind::WriteFailed));
    assert!(
        detail
            .as_deref()
            .is_some_and(|text| text.contains("changed since this list was loaded")),
        "{detail:?}"
    );
    assert!(
        form.status().starts_with("Save failed:"),
        "{}",
        form.status()
    );
    assert_eq!(names(&form), ["melee", "tank"]);
    assert_eq!(memory.names(), ["melee", "tank"]);
    assert_eq!(form.selected(), 0, "melee is now the first row");
    assert_eq!(form.draft().unwrap().name, "melee2");
}

#[test]
fn delete_removes_the_selected_row_at_once_and_keeps_the_selection_in_range() {
    let memory = Memory::seeded(&[Loadout::new("a"), Loadout::new("b")]);
    let mut form = form_over(&memory);
    let mut core = core();
    form.stage_selection(1);
    submit(&mut core, &mut form, OperatorCommand::SelectLoadout);

    assert_eq!(
        submit(&mut core, &mut form, OperatorCommand::DeleteLoadout),
        (Ended::Done, None)
    );
    assert_eq!(memory.names(), ["a"], "removed at once");
    assert_eq!(form.selected(), 0, "the selection moves onto the last row");
    assert_eq!(form.draft().unwrap().name, "a");
    assert_eq!(form.status(), "Saved.");

    submit(&mut core, &mut form, OperatorCommand::DeleteLoadout);
    assert!(memory.names().is_empty());
    assert!(form.draft().is_none(), "no row is left to be the draft");
    assert_eq!(
        submit(&mut core, &mut form, OperatorCommand::DeleteLoadout).0,
        Ended::Skipped(SkipReason::NothingSelected)
    );
}

#[test]
fn a_refused_delete_is_rolled_back() {
    let memory = Memory::seeded(&[Loadout::new("a"), Loadout::new("b")]);
    let mut form = form_over(&memory);
    let mut core = core();
    form.stage_selection(1);
    submit(&mut core, &mut form, OperatorCommand::SelectLoadout);
    memory.refuse_writes.store(true, Ordering::SeqCst);

    assert_eq!(
        submit(&mut core, &mut form, OperatorCommand::DeleteLoadout).0,
        Ended::Failed(FailureKind::WriteFailed)
    );
    assert_eq!(names(&form), ["a", "b"]);
    assert_eq!(form.selected(), 1);
    assert_eq!(memory.names(), ["a", "b"]);
}

/// `name` in game with `worn` on it.
fn in_game_wearing(core: &mut OperatorSession<()>, name: &str, worn: &[(&str, i32)]) {
    core.statuses.push(SlotStatus {
        username: name.into(),
        connected: true,
        ingame: true,
        ..SlotStatus::default()
    });
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_equipment(
        worn.iter()
            .enumerate()
            .map(|(slot, (item, id))| ItemView {
                def: api::ItemDefView {
                    id: *id,
                    name: (!item.is_empty()).then(|| (*item).to_string()),
                    stackable: false,
                    members: false,
                    base_value: 0,
                    noted: false,
                    certificate_link: -1,
                    certificate_template: -1,
                },
                container: ItemContainer::Equipment,
                action_family: ItemActionFamily::Held,
                slot: slot as i32,
                count: 1,
                actions: Vec::new(),
                component_id: -1,
            })
            .collect(),
    );
    core.publish_test_snapshot(name, snapshot, nav::WorldState::default());
}

#[test]
fn copy_equipment_takes_the_members_worn_gear_into_the_draft_and_keeps_supplies() {
    let memory = Memory::seeded(&[melee()]);
    let mut form = form_over(&memory);
    let mut core = core();
    in_game_wearing(
        &mut core,
        "alice",
        &[("Rune scimitar", 1333), ("  ", 1), ("", 2)],
    );
    submit(&mut core, &mut form, OperatorCommand::SelectLoadout);

    assert_eq!(
        submit_for(
            &mut core,
            &mut form,
            OperatorCommand::CopyEquipmentToLoadout,
            Targets::Profile("alice")
        ),
        (Ended::Done, None)
    );
    let draft = form.draft().unwrap();
    assert_eq!(
        draft.carry,
        vec![CarryEntry::new("Lobster", 8)],
        "supplies are kept"
    );
    // Without game data no slot is known, so the gear is kept unassigned:
    // the blank names are left out.
    assert_eq!(draft.unassigned, ["Rune scimitar"]);
    assert_eq!(
        form.status(),
        "Copied current equipment into the draft. Save to keep it."
    );
    assert_eq!(
        memory.baseline().rows,
        [melee()],
        "a copy is a draft edit: nothing is written"
    );
}

#[test]
fn copy_equipment_skips_without_an_ingame_member_or_without_a_draft() {
    let memory = Memory::seeded(&[melee()]);
    let mut form = form_over(&memory);
    let mut core = core();
    in_game_wearing(&mut core, "alice", &[("Rune scimitar", 1333)]);
    let copy = |core: &mut OperatorSession<()>, form: &mut LoadoutForm, name: &str| {
        submit_for(
            core,
            form,
            OperatorCommand::CopyEquipmentToLoadout,
            Targets::Profile(name),
        )
    };
    submit(&mut core, &mut form, OperatorCommand::SelectLoadout);

    // A member that is not in game (no status row, no snapshot).
    assert_eq!(
        copy(&mut core, &mut form, "bob"),
        (Ended::Skipped(SkipReason::NotLoggedIn), None)
    );
    assert_eq!(
        form.status(),
        "Copy current equipment needs a focused ingame character."
    );
    assert!(form.draft().unwrap().unassigned.is_empty());

    // In game, but no loadout is selected to copy into.
    form.stage_selection(7);
    submit(&mut core, &mut form, OperatorCommand::SelectLoadout);
    assert_eq!(
        copy(&mut core, &mut form, "alice"),
        (Ended::Skipped(SkipReason::NothingSelected), None)
    );
    assert_eq!(form.status(), "No loadout selected.");
}

#[test]
fn a_member_is_observable_only_in_game_with_a_snapshot() {
    let mut core = core();
    assert!(!core.can_observe_equipment("alice"));
    assert_eq!(core.observed_equipment("alice"), None);
    in_game_wearing(&mut core, "alice", &[("Rune scimitar", 1333)]);
    assert!(core.can_observe_equipment("alice"));
    assert_eq!(
        core.observed_equipment("alice"),
        Some(vec![("Rune scimitar".to_string(), 1333)])
    );
    core.statuses[0].ingame = false;
    assert!(!core.can_observe_equipment("alice"));
    assert_eq!(core.observed_equipment("alice"), None);
}

/// Copying observed equipment keeps the draft's supplies and puts each item
/// in the slot the game data names (the helper the row calls).
#[test]
fn copy_equipment_resolves_slots_from_game_data() {
    let data = api::game_data::for_revision(api::selected::ClientRevision::R274).unwrap();
    let mut form = LoadoutForm::over(Loadouts::detached(vec![melee()]));
    form.stage_selection(0);
    let mut core = core();
    submit(&mut core, &mut form, OperatorCommand::SelectLoadout);
    form.copy_equipment(&[("Rune scimitar".into(), 1333)], Some(&data))
        .unwrap();
    let draft = form.draft().unwrap();
    assert_eq!(
        draft.worn.get("righthand").map(String::as_str),
        Some("Rune scimitar")
    );
    assert_eq!(draft.carry[0], CarryEntry::new("Lobster", 8));
}

#[test]
fn a_front_end_that_lends_no_form_has_every_loadout_row_rejected() {
    let mut core = core();
    for command in [
        OperatorCommand::SelectLoadout,
        OperatorCommand::EditLoadoutDraft(LoadoutField::Equipment),
        OperatorCommand::CreateLoadout,
        OperatorCommand::DuplicateLoadout,
        OperatorCommand::SaveLoadout,
        OperatorCommand::DeleteLoadout,
    ] {
        let submitted = core
            .submit(
                command,
                Targets::Application,
                &mut FormSurface { form: None },
            )
            .expect("the command opens");
        assert_eq!(
            outcome_of(&core, submitted.request()).0,
            Ended::Rejected(Rejection::NoModel),
            "{command:?}"
        );
    }
    let submitted = core
        .submit(
            OperatorCommand::CopyEquipmentToLoadout,
            Targets::Profile("alice"),
            &mut FormSurface { form: None },
        )
        .expect("the command opens");
    assert_eq!(
        outcome_of(&core, submitted.request()).0,
        Ended::Rejected(Rejection::NoModel)
    );
}

/// A store that cannot be read.
struct Unreadable;

impl LoadoutsBacking for Unreadable {
    fn load(&self) -> Result<LoadoutsBaseline, String> {
        Err("the store is unreadable".into())
    }

    fn commit(
        &self,
        _ops: Vec<LoadoutOp>,
        _expected: StoreVersion,
    ) -> Result<LoadoutsBaseline, LoadoutsSaveError> {
        Err(LoadoutsSaveError::Failed("nothing was readable".into()))
    }
}

/// `script::LoadoutsStore` says why it refused as text, and the form reads
/// the kind of refusal off it: each of its sentences is pinned here against
/// the real store, so a change of its wording cannot silently reclassify.
#[test]
fn the_refusals_of_the_store_are_sorted_by_kind() {
    // Nowhere to load from or save to.
    let mut detached = Loadouts::detached(vec![melee()]);
    detached.upsert_for_test(Loadout::new("more"));
    assert_eq!(
        detached.save_for_test().map_err(|error| error.kind()),
        Err(LoadoutsErrorKind::NotBound)
    );
    assert_eq!(
        detached.reload_for_test().map_err(|error| error.kind()),
        Err(LoadoutsErrorKind::NotBound)
    );

    // A load that failed and still stands.
    let mut unreadable = Loadouts::over_backing_for_test(Arc::new(Unreadable));
    unreadable.upsert_for_test(Loadout::new("more"));
    assert_eq!(
        unreadable.save_for_test().map_err(|error| error.kind()),
        Err(LoadoutsErrorKind::NotLoaded)
    );
    assert_eq!(
        unreadable
            .reload_for_test()
            .map_err(|error| (error.kind(), error.to_string())),
        Err((LoadoutsErrorKind::Load, "the store is unreadable".into()))
    );

    // Another writer changed the store after the rows loaded.
    let memory = Memory::seeded(&[melee()]);
    let mut loadouts =
        Loadouts::over_backing_for_test(Arc::clone(&memory) as Arc<dyn LoadoutsBacking>);
    memory.outside(|rows| rows.push(Loadout::new("tank")));
    loadouts.upsert_for_test(Loadout::new("mine"));
    assert_eq!(
        loadouts.save_for_test().map_err(|error| error.kind()),
        Err(LoadoutsErrorKind::Conflict)
    );

    // The store refused the write.
    memory.refuse_writes.store(true, Ordering::SeqCst);
    assert_eq!(
        loadouts
            .save_for_test()
            .map_err(|error| (error.kind(), error.to_string())),
        Err((LoadoutsErrorKind::Write, "the disk is full".into()))
    );
}
