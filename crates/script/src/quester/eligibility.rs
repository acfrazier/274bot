//! Queue-row eligibility from quest-tab, skill, varp, world and bank evidence.
use super::bank_memo::BankMemo;
use super::compile::{CompiledEligibility, CompiledItemKind, CompiledPath};
use super::path::QuestRequirementDocument;
use api::quest_facts::QuestCatalog;
use api::selected::{ItemAmount, QuestGate, SkillMinimum};
use api::snapshot::{QuestListStatus, SnapshotView, StatView};
use serde::Deserialize;
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

#[derive(Deserialize)]
enum RequirementKind {
    Skill(SkillMinimum),
    Item(ItemAmount),
    Quest(QuestGate),
    QuestPoints(u16),
    MembersWorld,
}

/// Evaluate one compiled queue row. A completed quest is terminal before any
/// requirements; item requirements apply only before the quest starts.
pub fn evaluate(
    path: &CompiledPath,
    view: &SnapshotView<'_>,
    quests: &QuestCatalog,
    bank: &BankMemo,
) -> EligibilityResult {
    evaluate_path(path, &path.eligibility, view, quests, bank)
}

fn evaluate_path(
    path: &CompiledPath,
    header: &CompiledEligibility,
    view: &SnapshotView<'_>,
    quests: &QuestCatalog,
    bank: &BankMemo,
) -> EligibilityResult {
    let mut skills = Vec::new();
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
        if !requirement.at.eq_ignore_ascii_case("start") {
            continue;
        }
        let kind = match serde_json::from_value::<RequirementKind>(requirement.kind.clone()) {
            Ok(kind) => kind,
            Err(_) => {
                reasons.push(requirement_reason(
                    requirement,
                    "requirement kind is malformed or unsupported",
                ));
                continue;
            }
        };
        match kind {
            RequirementKind::Skill(minimum) => {
                check_skill(requirement, minimum, stats, &mut skills, &mut reasons);
            }
            RequirementKind::Item(item) if !started => {
                check_item(requirement, item.item, item.count, view, bank, &mut reasons);
            }
            RequirementKind::Item(_) => {}
            RequirementKind::Quest(gate) => {
                let id = match gate {
                    QuestGate::Complete(id) => id.0,
                    QuestGate::Window(window) => window.quest.0,
                };
                check_quest(requirement, &id, view, quests, &mut reasons);
            }
            RequirementKind::QuestPoints(required) => {
                check_quest_points(requirement, required, view, &mut reasons);
            }
            RequirementKind::MembersWorld => check_membership(
                view,
                requirement.id.0.as_ref(),
                "a members world is required",
                Some(requirement.source.as_str()),
                &mut reasons,
            ),
        }
    }

    if !started {
        for item in header
            .items
            .iter()
            .filter(|item| item.kind == CompiledItemKind::MustHave)
        {
            check_item(
                &QuestRequirementDocument {
                    id: api::selected::FactKey::new(&item.name),
                    kind: serde_json::Value::Null,
                    at: "Start".into(),
                    source: String::new(),
                },
                item.id,
                item.qty,
                view,
                bank,
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
    requirement: &QuestRequirementDocument,
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

fn check_item(
    requirement: &QuestRequirementDocument,
    item_id: i32,
    required: u32,
    view: &SnapshotView<'_>,
    bank: &BankMemo,
    reasons: &mut Vec<BlockReason>,
) {
    let bank_count = bank.count(item_id);
    let Some(inventory) = view.inventory().map(|observed| observed.value) else {
        if bank_count
            .is_some_and(|count| u64::try_from(count.max(0)).unwrap_or(0) >= u64::from(required))
        {
            return;
        }
        if bank_count.is_none() {
            // An unread bank is not an observed empty bank. Do not turn a
            // `mustHave` hint into a false blocker before a bank scan exists.
            return;
        }
        reasons.push(requirement_reason(
            requirement,
            "inventory is not observed while checking this required item",
        ));
        return;
    };
    let in_pack: u64 = inventory
        .iter()
        .filter(|row| row.def.id == item_id)
        .map(|row| u64::try_from(row.count.max(0)).unwrap_or(0))
        .sum();
    let Some(bank_count) = bank_count else {
        // An unread bank is unknown, never an empty bank. The pack observation
        // alone cannot prove that a `mustHave` requirement is missing.
        return;
    };
    let total = in_pack.saturating_add(u64::try_from(bank_count.max(0)).unwrap_or(0));
    if total < u64::from(required) {
        reasons.push(requirement_reason(
            requirement,
            &format!(
                "item {} requires {required}, only {total} is in pack and known bank",
                requirement.id.0
            ),
        ));
    }
}

fn check_quest(
    requirement: &QuestRequirementDocument,
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
    requirement: &QuestRequirementDocument,
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

fn requirement_reason(requirement: &QuestRequirementDocument, detail: &str) -> BlockReason {
    reason(
        requirement.id.0.as_ref(),
        detail,
        Some(requirement.source.as_str()),
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
    use crate::quester::path::QuestItemDocument;
    use api::quest_progress::EvidenceStamp;
    use api::selected::{ClientRevision, FactKey, RunKey};
    use api::snapshot::{GameSnapshot, QuestStatusView, StatView, VarpView, WorldStateView};

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

    fn req(id: &str, kind: serde_json::Value) -> QuestRequirementDocument {
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
        bank: &BankMemo,
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
        );
        evaluate(path, &view, quests, bank)
    }

    #[test]
    fn failed_skill_requirement_is_blocked_with_source_and_required_vs_live() {
        let (selected, quests, mut snapshot) = fixture();
        let mut document = decode_cook().unwrap();
        document.quest.as_mut().unwrap().requirements.push(req(
            "attack:20",
            serde_json::json!({"Skill": {"skill": 0, "level": 20}}),
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
        let result = evaluate_snapshot(&path, &quests, &snapshot, &BankMemo::default());
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
            req("qp:20", serde_json::json!({"QuestPoints": 20})),
            req(
                "quest:runemysteries",
                serde_json::json!({"Quest": {"Complete": "runemysteries"}}),
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
        let result = evaluate_snapshot(&path, &quests, &snapshot, &BankMemo::default());
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
        let mut document = decode_cook().unwrap();
        document.quest.as_mut().unwrap().items = vec![QuestItemDocument {
            obj: "egg".into(),
            qty: 1,
            kind: "mustHave".into(),
            acquire: None,
        }];
        let path = compile_uncached_for_test(&document, &selected, &quests).unwrap();
        assert!(!BankMemo::default().known());
        assert_eq!(
            evaluate_snapshot(&path, &quests, &snapshot, &BankMemo::default()).state,
            Eligibility::Ready,
            "an unread bank is unknown, not an empty bank"
        );

        let mut known_empty = BankMemo::default();
        known_empty.update(&crate::native_bank::BankReceipt {
            counts: Vec::new(),
            complete: true,
        });
        let result = evaluate_snapshot(&path, &quests, &snapshot, &known_empty);
        let Eligibility::Blocked(reasons) = result.state else {
            panic!("known absence from both pack and bank must block mustHave");
        };
        let expected_id = path.eligibility.items[0].name.as_ref();
        assert!(reasons
            .iter()
            .any(|reason| reason.id.as_ref() == expected_id));
    }

    #[test]
    fn item_requirements_are_ignored_after_the_quest_starts_and_complete_is_done() {
        let (selected, quests, mut snapshot) = fixture();
        let mut document = decode_cook().unwrap();
        document.quest.as_mut().unwrap().items = vec![QuestItemDocument {
            obj: "egg".into(),
            qty: 1,
            kind: "mustHave".into(),
            acquire: None,
        }];
        let path = compile_uncached_for_test(&document, &selected, &quests).unwrap();
        let mut known_empty = BankMemo::default();
        known_empty.update(&crate::native_bank::BankReceipt {
            counts: Vec::new(),
            complete: true,
        });
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
        let result = evaluate_snapshot(&path, &quests, &snapshot, &BankMemo::default());
        let Eligibility::Blocked(reasons) = result.state else {
            panic!("a free-to-play world must block a members quest");
        };
        assert!(reasons.iter().any(|reason| reason.id.as_ref() == "members"));

        snapshot.seed_world(WorldStateView {
            members: true,
            ..WorldStateView::default()
        });
        assert_eq!(
            evaluate_snapshot(&path, &quests, &snapshot, &BankMemo::default()).state,
            Eligibility::Ready
        );
    }
}
