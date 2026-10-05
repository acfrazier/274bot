//! Selected-NPC pickpocketing toward an existing inventory goal.
//!
//! `target.npc` accepts a selected NPC config alias or display name. `until`
//! uses the shared `obj`/`qty` shape; an optional anchor bounds target search
//! to its fixed radius (default 12, maximum 32). `settle_ms` is one overall
//! positive deadline (default 60000); this step never banks or trains.

use super::super::compile::{
    CompileContext, CompileError, StepContext, StepOutcome, StepPlan, StepRun,
};
use super::item_arg::ItemArg;
use crate::native::thieve::{Target, Thieve, ThieveActionArgs};
use crate::native::{ActionError, ActionHandle, NativeActions};
use api::game_data::SelectedGameData;
use api::snapshot::WorldTile;
use serde::Deserialize;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

const PICKPOCKET: &str = "Pickpocket";
const STEAL_FROM: &str = "Steal-from";

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct TargetArg {
    npc: String,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
struct Until {
    obj: ItemArg,
    qty: super::s2::QuantityDocument,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    target: TargetArg,
    until: Until,
    #[serde(default)]
    anchor: Option<super::AnchorArg>,
    #[serde(default = "default_radius")]
    radius: u16,
    #[serde(default = "default_settle_ms")]
    settle_ms: u64,
}

const fn default_radius() -> u16 {
    12
}

const fn default_settle_ms() -> u64 {
    60_000
}

pub(super) fn compile(
    args: Args,
    cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    if args.target.npc.trim().is_empty()
        || !(1..=32).contains(&args.radius)
        || args.settle_ms == 0
        || matches!(
            &args.until.qty,
            super::s2::QuantityDocument::Fixed(qty) if *qty < 1
        )
    {
        return Err(CompileError::code("invalid-args"));
    }

    let targets = selected_targets(cx.selected, args.target.npc.trim())?;
    let item_id = args.until.obj.id(cx.selected)?;
    let qty = super::s2::compile_quantity(args.until.qty, cx)?;
    let anchor = super::anchor_tile(args.anchor.as_ref())?;
    let radius = u8::try_from(args.radius).map_err(|_| CompileError::code("invalid-args"))?;
    let duration = Duration::from_millis(args.settle_ms);

    Ok(Arc::new(Plan {
        targets,
        item_id,
        qty,
        anchor,
        radius,
        duration,
    }))
}

fn selected_targets(
    selected: &SelectedGameData,
    selector: &str,
) -> Result<Arc<[Target]>, CompileError> {
    let names = selected
        .npc_names()
        .ok_or_else(|| CompileError::code("thieving-npc-facts-unavailable"))?;
    let alias_match = names
        .rows
        .iter()
        .any(|row| row.config.eq_ignore_ascii_case(selector));
    let mut targets = Vec::new();
    for row in names.rows.iter().filter(|row| {
        if alias_match {
            row.config.eq_ignore_ascii_case(selector)
        } else {
            row.display
                .as_deref()
                .is_some_and(|display| display.eq_ignore_ascii_case(selector))
        }
    }) {
        let actions = [PICKPOCKET, STEAL_FROM]
            .into_iter()
            .filter(|action| row.ops.iter().any(|op| op.eq_ignore_ascii_case(action)));
        let has_action = row
            .ops
            .iter()
            .any(|op| op.eq_ignore_ascii_case(PICKPOCKET) || op.eq_ignore_ascii_case(STEAL_FROM));
        if !has_action {
            continue;
        }
        let required_level = selected
            .required_thieving_npc(row.id)
            .filter(|level| *level >= 1)
            .ok_or_else(|| CompileError::code("unknown-thieving-level"))?;
        let display = row
            .display
            .as_deref()
            .filter(|display| !display.is_empty())
            .ok_or_else(|| CompileError::code("thieving-npc-name-unavailable"))?;
        for action in actions {
            targets.push(Target {
                id: row.id,
                display: Arc::from(display),
                action: Arc::from(action),
                required_level,
            });
        }
    }
    if targets.is_empty() {
        return Err(CompileError::code("unresolved-thieving-npc"));
    }
    Ok(Arc::from(targets))
}

struct Plan {
    targets: Arc<[Target]>,
    item_id: i32,
    qty: super::s2::QuantityPlan,
    anchor: Option<WorldTile>,
    radius: u8,
    duration: Duration,
}

impl StepPlan for Plan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let qty = self.qty.evaluate_step(cx).ok_or_else(|| {
            ActionError::Unavailable(Arc::from("thieving quantity evidence unavailable"))
        })?;
        let handle = cx.tick.actions.begin::<Thieve>(
            ThieveActionArgs {
                targets: Arc::clone(&self.targets),
                item_id: self.item_id,
                goal_qty: qty,
                anchor: self.anchor,
                radius: self.radius,
                deadline: self.duration,
            },
            &mut cx.tick.cx,
        )?;
        Ok(Box::new(Run {
            handle: Some(handle),
            qty,
        }))
    }

    fn settle_timeout(&self) -> Duration {
        self.duration
    }
}

struct Run {
    handle: Option<ActionHandle<Thieve>>,
    qty: i32,
}

impl StepRun for Run {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        let Some(handle) = self.handle.as_ref() else {
            return Poll::Ready(Err(ActionError::Stale));
        };
        match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(held)) if held >= self.qty => Poll::Ready(Ok(StepOutcome {
                progress: None,
                evidence: cx.tick.cx.evidence(),
                receipt: None,
            })),
            Poll::Ready(Ok(_)) => Poll::Ready(Err(ActionError::Failed(Arc::from(
                "thieving action completed below its inventory goal",
            )))),
        }
    }

    fn cancel(&mut self, actions: &mut NativeActions) {
        if let Some(handle) = self.handle.take() {
            actions.cancel(handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quester::families::tests::{local_player, with_tick};
    use api::selected::ClientRevision;
    use api::snapshot::{GameSnapshot, NpcView, StatView};

    fn plan() -> Plan {
        let selected = selected();
        Plan {
            targets: selected_targets(&selected, "digworkman1").unwrap(),
            item_id: 995,
            qty: super::super::s2::QuantityPlan::Fixed(1),
            anchor: None,
            radius: 12,
            duration: Duration::from_secs(60),
        }
    }

    fn snapshot(effective: i32, base: i32, action: &str) -> GameSnapshot {
        let tile = WorldTile {
            x: 3300,
            z: 3300,
            level: 0,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![], 28);
        snapshot.seed_stats(vec![StatView {
            index: 17,
            name: "thieving".into(),
            effective,
            base,
            xp: 0,
            used: true,
        }]);
        snapshot.seed_local_player(local_player(tile));
        snapshot.seed_npcs(vec![NpcView {
            index: 91,
            r#type: Some(613),
            name: Some("Digsite workman".into()),
            actions: vec![Some(action.into())],
            tile,
            distance: 1,
            animation: -1,
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health: 1,
            total_health: 1,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 1,
            size: 1,
            network: tile,
            x: 0,
            z: 0,
            yaw: 0,
        }]);
        snapshot
    }

    fn with_step<R>(
        tick: &mut crate::native::NativeTick<'_>,
        f: impl FnOnce(&mut StepContext<'_, '_>) -> R,
    ) -> R {
        let quests = api::quest_facts::QuestCatalog::empty();
        let bank = crate::quester::bank_memo::BankMemo::default();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let choices = crate::quester::choices::QuestChoices::default();
        let required_after = tick.cx.evidence();
        f(&mut StepContext {
            tick,
            quests: &quests,
            progress: &[],
            required_after,
            bank: &bank,
            banks: &banks,
            choices: &choices,
        })
    }

    fn has_interaction(ledger: &Option<Box<crate::native::ledger::Ledger>>) -> bool {
        ledger.as_ref().is_some_and(|ledger| {
            ledger.outbox.iter().any(|interaction| {
                matches!(
                    &interaction.effect,
                    crate::native::HostEffect::Interaction(
                        crate::shim::InteractReq::Npc {
                            name,
                            action,
                            index: Some(index),
                        }
                    ) if name == "Digsite workman"
                        && action == STEAL_FROM
                        && *index == 91
                )
            })
        })
    }
    fn has_any_interaction(ledger: &Option<Box<crate::native::ledger::Ledger>>) -> bool {
        ledger.as_ref().is_some_and(|ledger| {
            ledger.outbox.iter().any(|interaction| {
                matches!(
                    &interaction.effect,
                    crate::native::HostEffect::Interaction(_)
                )
            })
        })
    }

    #[test]
    fn family_dispatches_to_the_selected_npc_instance() {
        let plan = plan();
        let snapshot = snapshot(25, 25, STEAL_FROM);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(has_interaction(&ledger));
    }

    #[test]
    fn family_level_gate_uses_effective_level_not_base_level() {
        let plan = plan();
        let snapshot = snapshot(24, 30, STEAL_FROM);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 2, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Err(ActionError::Failed(_)))
        ));
        assert!(!has_any_interaction(&ledger));
    }

    #[test]
    fn family_waits_without_the_selected_thieving_action() {
        let plan = plan();
        let snapshot = snapshot(30, 30, "Talk-to");
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(!has_any_interaction(&ledger));
    }

    fn selected() -> Arc<SelectedGameData> {
        api::game_data::for_revision(ClientRevision::R289).expect("selected game data")
    }

    #[test]
    fn target_alias_and_display_resolve_to_exact_selected_npc_levels() {
        let selected = selected();
        let alias = selected_targets(&selected, "digworkman1").unwrap();
        assert_eq!(alias.len(), 1);
        assert_eq!(alias[0].id, 613);
        assert_eq!(alias[0].display.as_ref(), "Digsite workman");
        assert_eq!(alias[0].action.as_ref(), STEAL_FROM);
        assert_eq!(alias[0].required_level, 25);

        let display = selected_targets(&selected, "Digsite workman").unwrap();
        assert_eq!(
            display.iter().map(|target| target.id).collect::<Vec<_>>(),
            vec![613, 614]
        );
        assert!(display.iter().all(|target| target.required_level == 25));
        for (alias, id) in [("troll_prison_guard1", 1128), ("troll_prison_guard2", 1129)] {
            let target = selected_targets(&selected, alias).unwrap();
            assert!(
                target
                    .iter()
                    .any(|target| target.id == id && target.required_level == 30),
                "Troll prison guard alias {alias} keeps its source-backed level-30 gate"
            );
        }

        for (alias, id) in [("man", 1), ("man2", 2), ("man3", 3)] {
            let target = selected_targets(&selected, alias).unwrap();
            assert!(
                target
                    .iter()
                    .any(|target| target.id == id && target.required_level == 1),
                "Waterfall men use the selected level-1 pickpocket fact for {alias}"
            );
        }
    }

    #[test]
    fn unknown_or_non_thieving_npc_does_not_resolve_as_a_target() {
        let selected = selected();
        assert!(selected_targets(&selected, "not_a_selected_npc").is_err());
        assert!(selected_targets(&selected, "Banker").is_err());
    }
}
