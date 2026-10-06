//! Queue-row eligibility from quest-tab, skill, varp, world and bank evidence.
use super::compile::{CompiledEligibility, CompiledItemKind, CompiledPath, CompiledRequirement};
use api::bank_memory::Origin;
use api::quest_facts::QuestCatalog;
use api::selected::{QuestGate, RequirementKind, SkillMinimum};
use api::snapshot::{QuestListStatus, SnapshotView, StatView};
use api::stock::Stock;
use std::sync::Arc;

const QUEST_POINTS_VARP: i32 = 101;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockReason {
    pub id: Arc<str>,
    pub detail: Arc<str>,
    pub source: Option<Arc<str>>,
}

impl std::fmt::Display for BlockReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.id, self.detail)?;
        if let Some(source) = self.source.as_deref().filter(|source| !source.is_empty()) {
            write!(f, " (source: {source})")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Eligibility {
    Ready,
    Done,
    Blocked(Vec<BlockReason>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillGate {
    pub id: Arc<str>,
    pub skill: u8,
    pub name: Arc<str>,
    pub required: u16,
    pub live: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EligibilityResult {
    pub state: Eligibility,
    pub skill_gates: Vec<SkillGate>,
}

/// Evaluate one compiled queue row. A completed quest is terminal before any
/// requirements; item requirements apply only before the quest starts and
/// read the frame's `Stock` (pack, worn and the account's bank memory).
pub fn evaluate(
    path: &CompiledPath,
    view: &SnapshotView<'_>,
    quests: &QuestCatalog,
) -> EligibilityResult {
    evaluate_path(path, &path.eligibility, view, quests)
}

fn evaluate_path(
    path: &CompiledPath,
    header: &CompiledEligibility,
    view: &SnapshotView<'_>,
    quests: &QuestCatalog,
) -> EligibilityResult {
    let mut skills = Vec::new();
    let status = if path.kind == super::path::PathKind::Miniquest {
        // Completion is proved by the owned card/guard reader, not a fabricated tab row.
        QuestListStatus::NotStarted
    } else {
        let Some(status) = tab_status(view, &path.display_name) else {
            return blocked(
                vec![reason(
                    "quest-status",
                    "quest-tab status is not observed",
                    None,
                )],
                skills,
            );
        };
        status
    };
    if status == QuestListStatus::Complete {
        return EligibilityResult {
            state: Eligibility::Done,
            skill_gates: skills,
        };
    }
    if status == QuestListStatus::Unknown {
        return blocked(
            vec![reason(
                "quest-status",
                "quest-tab row has an unknown colour",
                None,
            )],
            skills,
        );
    }

    let started = status == QuestListStatus::InProgress;
    let mut reasons = Vec::new();
    if header.members {
        check_membership(
            view,
            "members",
            "quest requires a members world",
            None,
            &mut reasons,
        );
    }

    let stats = view.stats().map(|observed| observed.value);
    for requirement in header.requirements.iter() {
        if !requirement.at_start {
            continue;
        }
        match &requirement.kind {
            RequirementKind::Skill(minimum) => {
                check_skill(requirement, *minimum, stats, &mut skills, &mut reasons);
            }
            RequirementKind::Item(item) if !started => {
                check_item(
                    requirement,
                    item.item,
                    item.count,
                    &view.stock(),
                    &mut reasons,
                );
            }
            RequirementKind::Item(_) => {}
            RequirementKind::Quest(gate) => {
                let id = match gate {
                    QuestGate::Complete(id) => id.0.as_ref(),
                    QuestGate::Window(window) => window.quest.0.as_ref(),
                };
                check_quest(requirement, id, view, quests, &mut reasons);
            }
            RequirementKind::QuestPoints(required) => {
                check_quest_points(requirement, *required, view, &mut reasons);
            }
            RequirementKind::MembersWorld => check_membership(
                view,
                requirement.id.0.as_ref(),
                "a members world is required",
                Some(requirement.source.as_ref()),
                &mut reasons,
            ),
        }
    }

    if !started {
        let stock = view.stock();
        for item in header
            .items
            .iter()
            .filter(|item| item.kind == CompiledItemKind::MustHave)
        {
            check_item(
                &CompiledRequirement {
                    id: api::selected::FactKey::new(&item.name),
                    at_start: true,
                    source: Arc::from(""),
                    kind: RequirementKind::MembersWorld,
                },
                item.id,
                item.qty,
                &stock,
                &mut reasons,
            );
        }
    }

    if reasons.is_empty() {
        EligibilityResult {
            state: Eligibility::Ready,
            skill_gates: skills,
        }
    } else {
        blocked(reasons, skills)
    }
}

fn tab_status(view: &SnapshotView<'_>, display: &str) -> Option<QuestListStatus> {
    view.quest_statuses()?
        .value
        .iter()
        .find(|row| row.name.trim().eq_ignore_ascii_case(display.trim()))
        .map(|row| row.status())
}

fn check_skill(
    requirement: &CompiledRequirement,
    minimum: SkillMinimum,
    stats: Option<&[StatView]>,
    skill_gates: &mut Vec<SkillGate>,
    reasons: &mut Vec<BlockReason>,
) {
    let live = stats.and_then(|stats| {
        stats
            .iter()
            .find(|stat| stat.index == i32::from(minimum.skill) && stat.used)
    });
    let name = live
        .map(|stat| Arc::from(stat.name.as_str()))
        .unwrap_or_else(|| Arc::from(format!("skill {}", minimum.skill)));
    skill_gates.push(SkillGate {
        id: Arc::clone(&requirement.id.0),
        skill: minimum.skill,
        name,
        required: minimum.level,
        live: live.map(|stat| stat.base),
    });
    match live {
        Some(stat) if stat.base >= i32::from(minimum.level) => {}
        Some(stat) => reasons.push(requirement_reason(
            requirement,
            &format!(
                "{} requires level {}, live base level is {}",
                stat.name, minimum.level, stat.base
            ),
        )),
        None => reasons.push(requirement_reason(
            requirement,
            &format!(
                "skill {} level is not observed (requires {})",
                minimum.skill, minimum.level
            ),
        )),
    }
}

/// A `mustHave` row blocks only on a shortage the account's bank memory
/// observed this session (design-bank-snapshot §2.4): an `Unknown` or
/// `Hint` bank fails open and the provisioner's verifying scan decides.
/// Worn items count (a quest item you wear is carried).
fn check_item(
    requirement: &CompiledRequirement,
    item_id: i32,
    required: u32,
    stock: &Stock<'_>,
    reasons: &mut Vec<BlockReason>,
) {
    if stock.banked_origin() != Origin::Session {
        return;
    }
    let banked = u64::try_from(stock.banked(item_id).unwrap_or(0).max(0)).unwrap_or(0);
    let required = u64::from(required);
    if banked >= required {
        return;
    }
    let Some(held) = stock.held(item_id) else {
        reasons.push(requirement_reason(
            requirement,
            "inventory is not observed while checking this required item",
        ));
        return;
    };
    let worn = u64::try_from(stock.worn(item_id).unwrap_or(0).max(0)).unwrap_or(0);
    let total = u64::try_from(held.max(0))
        .unwrap_or(0)
        .saturating_add(worn)
        .saturating_add(banked);
    if total < required {
        reasons.push(requirement_reason(
            requirement,
            &format!(
                "item {} requires {required}, only {total} is carried or in the bank seen this session",
                requirement.id.0
            ),
        ));
    }
}

fn check_quest(
    requirement: &CompiledRequirement,
    quest_id: &str,
    view: &SnapshotView<'_>,
    quests: &QuestCatalog,
    reasons: &mut Vec<BlockReason>,
) {
    let Some(facts) = quests.quest(quest_id).ok() else {
        reasons.push(requirement_reason(
            requirement,
            &format!("required quest {quest_id} is not in the selected quest catalog"),
        ));
        return;
    };
    match tab_status(view, &facts.display) {
        Some(QuestListStatus::Complete) => {}
        Some(status) => reasons.push(requirement_reason(
            requirement,
            &format!("quest {quest_id} is {}, not complete", status.as_str()),
        )),
        None => reasons.push(requirement_reason(
            requirement,
            &format!("quest {quest_id} status is not observed"),
        )),
    }
}

fn check_quest_points(
    requirement: &CompiledRequirement,
    required: u16,
    view: &SnapshotView<'_>,
    reasons: &mut Vec<BlockReason>,
) {
    let value = view.varps().and_then(|observed| {
        observed
            .value
            .iter()
            .find(|varp| varp.index == QUEST_POINTS_VARP)
            .map(|varp| varp.value)
    });
    match value {
        Some(live) if live >= i32::from(required) => {}
        Some(live) => reasons.push(requirement_reason(
            requirement,
            &format!("requires {required} quest points, live total is {live}"),
        )),
        None => reasons.push(requirement_reason(
            requirement,
            "quest-point varp 101 is not observed",
        )),
    }
}

fn check_membership(
    view: &SnapshotView<'_>,
    id: &str,
    detail: &str,
    source: Option<&str>,
    reasons: &mut Vec<BlockReason>,
) {
    match view.world().map(|observed| observed.value.members) {
        Some(true) => {}
        Some(false) => reasons.push(reason(id, detail, source)),
        None => reasons.push(reason(id, "world membership is not observed", source)),
    }
}

fn requirement_reason(requirement: &CompiledRequirement, detail: &str) -> BlockReason {
    reason(
        requirement.id.0.as_ref(),
        detail,
        Some(requirement.source.as_ref()),
    )
}

fn reason(id: &str, detail: &str, source: Option<&str>) -> BlockReason {
    BlockReason {
        id: Arc::from(id),
        detail: Arc::from(detail),
        source: source.map(Arc::from),
    }
}

fn blocked(reasons: Vec<BlockReason>, skill_gates: Vec<SkillGate>) -> EligibilityResult {
    EligibilityResult {
        state: Eligibility::Blocked(reasons),
        skill_gates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quester::compile::{compile_uncached_for_test, decode_cook};
    use crate::quester::path::{
        QuestItemDocument, QuestItemKindDocument, QuestRequirementDocument,
        QuestRequirementKindDocument,
    };
    use api::bank_memory::BankMemory;
    use api::quest_progress::EvidenceStamp;
    use api::selected::{ClientRevision, FactKey, RunKey};
    use api::snapshot::{
        GameSnapshot, ItemActionFamily, ItemContainer, ItemView, QuestStatusView, StatView,
        VarpView, WorldStateView,
    };

    fn fixture() -> (
        Arc<api::game_data::SelectedGameData>,
        Arc<QuestCatalog>,
        GameSnapshot,
    ) {
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(selected.quest_identity()).unwrap());
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 0,
                colour: 0xf80000,
            }],
            true,
        );
        snapshot.seed_inventory(Vec::new(), 28);
        (selected, quests, snapshot)
    }

    fn req(id: &str, kind: QuestRequirementKindDocument) -> QuestRequirementDocument {
        QuestRequirementDocument {
            id: FactKey::new(id),
            kind,
            at: "Start".into(),
            source: "fixture requirements".into(),
        }
    }

    fn evaluate_snapshot(
        path: &CompiledPath,
        quests: &QuestCatalog,
        snapshot: &GameSnapshot,
        bank: &BankMemory,
    ) -> EligibilityResult {
        let view = SnapshotView::new(
            Some(snapshot),
            EvidenceStamp {
                run: RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                tick: 1,
                sequence: 1,
            },
        )
        .with_bank_memory(Some(bank));
        evaluate(path, &view, quests)
    }

    fn must_have_egg_path(
        selected: &api::game_data::SelectedGameData,
        quests: &QuestCatalog,
    ) -> Arc<CompiledPath> {
        let mut document = decode_cook().unwrap();
        document.quest.as_mut().unwrap().items = vec![QuestItemDocument {
            obj: "egg".into(),
            qty: 1,
            kind: QuestItemKindDocument::MustHave,
            acquire: None,
            from_stage: None,
        }];
        compile_uncached_for_test(&document, selected, quests).unwrap()
    }

    #[test]
    fn failed_skill_requirement_is_blocked_with_source_and_required_vs_live() {
        let (selected, quests, mut snapshot) = fixture();
        let mut document = decode_cook().unwrap();
        document.quest.as_mut().unwrap().requirements.push(req(
            "attack:20",
            QuestRequirementKindDocument::Skill {
                skill: "attack".into(),
                level: 20,
            },
        ));
        snapshot.seed_stats(vec![StatView {
            index: 0,
            name: "attack".into(),
            effective: 10,
            base: 10,
            xp: 0,
            used: true,
        }]);
        let path = compile_uncached_for_test(&document, &selected, &quests).unwrap();
        let result = evaluate_snapshot(&path, &quests, &snapshot, &BankMemory::default());
        let Eligibility::Blocked(reasons) = result.state else {
            panic!("a below-minimum skill must block the queue row");
        };
        assert!(reasons.iter().any(|reason| {
            reason.id.as_ref() == "attack:20"
                && reason.detail.contains("requires level 20")
                && reason.to_string().contains("fixture requirements")
        }));
        assert_eq!(
            result.skill_gates,
            [SkillGate {
                id: Arc::from("attack:20"),
                skill: 0,
                name: Arc::from("attack"),
                required: 20,
                live: Some(10),
            }]
        );
    }

    #[test]
    fn qp_and_quest_requirements_block_until_live_gates_pass() {
        let (selected, quests, mut snapshot) = fixture();
        let mut document = decode_cook().unwrap();
        document.quest.as_mut().unwrap().requirements.extend([
            req(
                "item:egg",
                QuestRequirementKindDocument::Item {
                    obj: "egg".into(),
                    qty: 1,
                },
            ),
            req("qp:20", QuestRequirementKindDocument::QuestPoints(20)),
            req(
                "quest:runemysteries",
                QuestRequirementKindDocument::Quest("runemysteries".into()),
            ),
        ]);
        snapshot.seed_varps(vec![VarpView {
            index: QUEST_POINTS_VARP,
            value: 10,
        }]);
        snapshot.seed_quest_statuses(
            vec![
                QuestStatusView {
                    name: "Cook's Assistant".into(),
                    component_id: 0,
                    colour: 0xf80000,
                },
                QuestStatusView {
                    name: "Rune Mysteries Quest".into(),
                    component_id: 1,
                    colour: 0xf80000,
                },
            ],
            true,
        );
        let path = compile_uncached_for_test(&document, &selected, &quests).unwrap();
        let egg_id = selected.item_by_alias("egg").expect("egg alias").id;
        let compiled_egg = path
            .eligibility
            .requirements
            .iter()
            .find(|requirement| requirement.id.0.as_ref() == "item:egg")
            .expect("compiled egg requirement");
        assert!(matches!(
            &compiled_egg.kind,
            RequirementKind::Item(item) if item.item == egg_id && item.count == 1
        ));
        let result = evaluate_snapshot(&path, &quests, &snapshot, &BankMemory::default());
        let Eligibility::Blocked(reasons) = result.state else {
            panic!("unmet quest-point and quest-completion gates must block");
        };
        assert!(reasons.iter().any(|reason| reason.id.as_ref() == "qp:20"));
        assert!(reasons
            .iter()
            .any(|reason| reason.id.as_ref() == "quest:runemysteries"));
    }

    #[test]
    fn unread_bank_does_not_block_must_have_but_known_empty_bank_does() {
        let (selected, quests, snapshot) = fixture();
        let path = must_have_egg_path(&selected, &quests);
        assert!(!BankMemory::default().known());
        assert_eq!(
            evaluate_snapshot(&path, &quests, &snapshot, &BankMemory::default()).state,
            Eligibility::Ready,
            "an unread bank is unknown, not an empty bank"
        );

        let known_empty = BankMemory::seeded(&[], Origin::Session);
        let result = evaluate_snapshot(&path, &quests, &snapshot, &known_empty);
        let Eligibility::Blocked(reasons) = result.state else {
            panic!("known absence from both pack and bank must block mustHave");
        };
        let expected_id = path.eligibility.items[0].name.as_ref();
        assert!(reasons
            .iter()
            .any(|reason| reason.id.as_ref() == expected_id));
    }

    /// design-bank-snapshot §2.4 / §6 bug 7: only a `Session` shortage blocks;
    /// a `Hint` absence is advisory (the provisioner's scan verifies it) and a
    /// worn `mustHave` item is carried.
    #[test]
    fn hint_absence_and_worn_items_never_block_a_must_have() {
        let (selected, quests, mut snapshot) = fixture();
        let path = must_have_egg_path(&selected, &quests);
        let egg = selected.item_by_alias("egg").expect("egg alias").id;

        let hint_empty = BankMemory::seeded(&[], Origin::Hint);
        assert_eq!(
            evaluate_snapshot(&path, &quests, &snapshot, &hint_empty).state,
            Eligibility::Ready,
            "a hint that lacks the item is advisory, never a block"
        );
        let hint_stocked = BankMemory::seeded(&[(egg, 1)], Origin::Hint);
        assert_eq!(
            evaluate_snapshot(&path, &quests, &snapshot, &hint_stocked).state,
            Eligibility::Ready
        );

        let session_empty = BankMemory::seeded(&[], Origin::Session);
        snapshot.seed_equipment(vec![ItemView {
            def: api::obj_names::ItemDefView {
                id: egg,
                name: Some("Egg".into()),
                stackable: false,
                members: false,
                base_value: 1,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: ItemContainer::Equipment,
            action_family: ItemActionFamily::Component,
            slot: 0,
            count: 1,
            actions: Vec::new(),
            component_id: 0,
        }]);
        assert_eq!(
            evaluate_snapshot(&path, &quests, &snapshot, &session_empty).state,
            Eligibility::Ready,
            "a worn mustHave item is carried; the empty session bank does not block it"
        );
        snapshot.seed_equipment(Vec::new());
        assert!(
            matches!(
                evaluate_snapshot(&path, &quests, &snapshot, &session_empty).state,
                Eligibility::Blocked(_)
            ),
            "with nothing worn the session-known absence blocks again"
        );
    }

    #[test]
    fn item_requirements_are_ignored_after_the_quest_starts_and_complete_is_done() {
        let (selected, quests, mut snapshot) = fixture();
        let path = must_have_egg_path(&selected, &quests);
        let known_empty = BankMemory::seeded(&[], Origin::Session);
        snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 0,
                colour: 0xf8f800,
            }],
            true,
        );
        assert_eq!(
            evaluate_snapshot(&path, &quests, &snapshot, &known_empty).state,
            Eligibility::Ready
        );
        snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 0,
                colour: 0x00f800,
            }],
            true,
        );
        assert_eq!(
            evaluate_snapshot(&path, &quests, &snapshot, &known_empty).state,
            Eligibility::Done
        );
    }
    #[test]
    fn members_world_gate_uses_the_live_profile_world() {
        let (selected, quests, mut snapshot) = fixture();
        let mut document = decode_cook().unwrap();
        document.quest.as_mut().unwrap().members = true;
        let path = compile_uncached_for_test(&document, &selected, &quests).unwrap();
        snapshot.seed_world(WorldStateView {
            members: false,
            ..WorldStateView::default()
        });
        let result = evaluate_snapshot(&path, &quests, &snapshot, &BankMemory::default());
        let Eligibility::Blocked(reasons) = result.state else {
            panic!("a free-to-play world must block a members quest");
        };
        assert!(reasons.iter().any(|reason| reason.id.as_ref() == "members"));

        snapshot.seed_world(WorldStateView {
            members: true,
            ..WorldStateView::default()
        });
        assert_eq!(
            evaluate_snapshot(&path, &quests, &snapshot, &BankMemory::default()).state,
            Eligibility::Ready
        );
    }
}
