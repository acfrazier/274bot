//! Path-backed Quester qualification fixtures, never product gameplay.
//!
//! New Paths use [`quester_stage`] here. The older numeric
//! [`crate::quester_stage`] remains the low-level seed used by the shipped
//! S2 and combat/WalkGuard cells; their bespoke stat tuples are unchanged.
use crate::{start_compiled_step, Proof, Scenario, ScenarioSettings, Seed, Step, StepKind, Wait};
use api::game_data::{QuestIdentityRow, SelectedGameData};
use api::selected::SkillMinimum;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::skill::Skill;
use script::quester::path::{LoadoutCarryDocument, PathDocument, QuestLoadoutDocument};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::time::Duration;

/// Qualification floor, not an eligibility gate or a claim of live PASS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TestProfile {
    Base40,
    Base60,
}

impl TestProfile {
    pub const fn combat_level(self) -> u16 {
        match self {
            Self::Base40 => 40,
            Self::Base60 => 60,
        }
    }
}

/// Add one row per authored quest, in release order. Move to Base60 only
/// when the operator names it or a recorded live run establishes the need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestFixtureProfile {
    pub quest: &'static str,
    pub profile: TestProfile,
}

pub const FIXTURE_PROFILES: &[QuestFixtureProfile] = &[
    QuestFixtureProfile {
        quest: "cook",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "sheep",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "runemysteries",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "romeojuliet",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "imp",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "prince",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "hunt",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "priest",
        profile: TestProfile::Base40,
    },
    // Operator-mandated Base60 quests (README, Q-RESET qualification floor).
    QuestFixtureProfile {
        quest: "dragon",
        profile: TestProfile::Base60,
    },
    QuestFixtureProfile {
        quest: "ikov",
        profile: TestProfile::Base60,
    },
    QuestFixtureProfile {
        quest: "upass",
        profile: TestProfile::Base60,
    },
    QuestFixtureProfile {
        quest: "legends",
        profile: TestProfile::Base60,
    },
    QuestFixtureProfile {
        quest: "elemental_workshop",
        profile: TestProfile::Base60,
    },
    QuestFixtureProfile {
        quest: "horror",
        profile: TestProfile::Base60,
    },
];

pub fn fixture_profile(quest: &str) -> Result<TestProfile, String> {
    FIXTURE_PROFILES
        .iter()
        .find(|row| row.quest == quest)
        .map(|row| row.profile)
        .ok_or_else(|| format!("no qualification profile for quest {quest}"))
}

/// The operator's three starting kits. These are item seeds, not permission
/// to raise stats to an item's wearing requirement. A Base40 ranged fixture
/// still has Ranged40; any refused black-d'hide/yew piece must be reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardKit {
    Melee,
    Magic,
    Ranged,
}

pub fn standard_kit(style: StandardKit) -> QuestLoadoutDocument {
    let (worn, carry) = match style {
        StandardKit::Melee => (
            &[
                ("hat", "rune_full_helm"),
                ("torso", "rune_chainbody"),
                ("legs", "rune_platelegs"),
                ("lefthand", "rune_kiteshield"),
                ("righthand", "rune_scimitar"),
            ][..],
            &[][..],
        ),
        StandardKit::Magic => (
            &[("righthand", "staff_of_air")][..],
            &[("airrune", 600), ("chaosrune", 300)][..],
        ),
        StandardKit::Ranged => (
            &[
                ("torso", "black_dragonhide_body"),
                ("legs", "black_dragonhide_chaps"),
                ("hands", "black_dragon_vambraces"),
                ("righthand", "yew_shortbow"),
                ("quiver", "rune_arrow"),
            ][..],
            &[("rune_arrow", 300)][..],
        ),
    };
    QuestLoadoutDocument {
        worn: worn
            .iter()
            .map(|(slot, obj)| ((*slot).into(), (*obj).into()))
            .collect(),
        carry: carry
            .iter()
            .map(|(item, qty)| LoadoutCarryDocument {
                item: (*item).into(),
                qty: *qty,
            })
            .collect(),
    }
}

/// A fixture's loadout is either an actual named Path kit or one of the
/// operator's standard kits. None seeds no gear; it does not invent a kit.
#[derive(Debug, Clone, Copy)]
pub enum FixtureLoadout<'a> {
    Path(&'a str),
    Standard(StandardKit),
}

/// Inputs for one sequence fixture. `identity` and `selected` must come from
/// the same selected revision. Stage values come only from the Path's varp
/// hints; zero is never substituted for an absent/synthetic hint.
pub struct QuesterStage<'a> {
    pub name: &'static str,
    /// Quest-tab display from selected content, not a journal title.
    pub quest_display: &'static str,
    pub path: &'a PathDocument,
    pub identity: &'a QuestIdentityRow,
    pub selected: &'a SelectedGameData,
    pub stage: &'a str,
    pub loadout: Option<FixtureLoadout<'a>>,
    pub extra_items: &'a [(&'a str, i32)],
    pub stand: WorldTile,
}

/// Scenario ownership is independent of the cloneable Start observation.
pub struct QuesterFixture {
    pub scenario: Scenario,
    pub start_settings: Map<String, Value>,
    pub seed: QuesterSeed,
    pub seed_commands: Vec<String>,
}

/// Keep this receipt with the live observations. `stats` is the planned
/// profile; [`Self::observe_start`] proves what actually landed.
#[derive(Debug, Clone)]
pub struct QuesterSeed {
    pub profile: TestProfile,
    pub stats: Vec<SkillMinimum>,
    pub loadout: Option<QuestLoadoutDocument>,
    pub stand: WorldTile,
    expected_inventory: Vec<(i32, i32)>,
    expected_equipment: Vec<i32>,
}

impl QuesterSeed {
    /// Call immediately before the harness emits Start. A stale, higher
    /// stat or a missing/refused kit cannot turn into invented TestedStats.
    pub fn observe_start(&self, snapshot: &GameSnapshot) -> Result<Value, String> {
        validate_start_seed(
            snapshot,
            &self.stats,
            &self.expected_inventory,
            &self.expected_equipment,
        )?;
        validate_start_stand(snapshot.tile(), self.stand)?;
        let (x, z, level) = snapshot.tile().expect("validated fixture stand");
        let observed: Vec<_> = self
            .stats
            .iter()
            .map(|minimum| {
                let stat = snapshot
                    .stats()
                    .iter()
                    .find(|stat| stat.index == i32::from(minimum.skill))
                    .expect("validated fixture stat");
                json!({"skill": minimum.skill, "base": stat.base, "effective": stat.effective})
            })
            .collect();
        Ok(json!({"profile": self.profile, "stats": observed,
            "loadout": self.loadout, "inventory": self.expected_inventory,
            "equipment": self.expected_equipment, "tile": {"x": x, "z": z, "level": level}}))
    }
}

fn validate_start_stand(tile: Option<(i32, i32, i32)>, stand: WorldTile) -> Result<(), String> {
    if tile != Some((stand.x, stand.z, stand.level)) {
        return Err(format!(
            "fixture stand expected ({},{},{}), observed {tile:?}",
            stand.x, stand.z, stand.level
        ));
    }
    Ok(())
}

fn validate_start_seed(
    snapshot: &GameSnapshot,
    stats: &[SkillMinimum],
    inventory: &[(i32, i32)],
    equipment: &[i32],
) -> Result<(), String> {
    if !snapshot.ingame() || snapshot.scene_state() != 2 {
        return Err("fixture Start needs ingame && scene_state == 2".into());
    }
    for minimum in stats {
        let stat = snapshot
            .stats()
            .iter()
            .find(|stat| stat.index == i32::from(minimum.skill))
            .ok_or_else(|| format!("fixture stat {} is unobserved", minimum.skill))?;
        if stat.base != i32::from(minimum.level) || stat.effective != i32::from(minimum.level) {
            return Err(format!(
                "fixture stat {} expected {}, observed base {} effective {}",
                minimum.skill, minimum.level, stat.base, stat.effective
            ));
        }
    }
    for (id, count) in inventory {
        let held: i32 = snapshot
            .inventory()
            .iter()
            .filter(|item| item.def.id == *id)
            .map(|item| item.count)
            .sum();
        if held < *count {
            return Err(format!(
                "fixture item {id} expected {count}, observed {held}"
            ));
        }
    }
    for id in equipment {
        if !snapshot
            .equipment()
            .iter()
            .any(|item| item.def.id == *id && item.count > 0)
        {
            return Err(format!("fixture equipment {id} was not worn"));
        }
    }
    Ok(())
}

fn stats_for(path: &PathDocument, profile: TestProfile) -> Result<Vec<SkillMinimum>, String> {
    let mut levels = BTreeMap::from([
        (0, profile.combat_level()),
        (1, profile.combat_level()),
        (2, profile.combat_level()),
        (3, profile.combat_level()),
        (4, profile.combat_level()),
        (5, 43),
        (6, profile.combat_level()),
    ]);
    let header = path
        .quest
        .as_ref()
        .ok_or("qualification Path has no quest header")?;
    for requirement in &header.requirements {
        // Serializing the typed document kind also works across schema 3's
        // enum cutover; no duplicate Path decoder or schema is introduced.
        let kind = serde_json::to_value(&requirement.kind).map_err(|error| error.to_string())?;
        let Some(skill) = kind.get("Skill") else {
            continue;
        };
        let index = if let Some(name) = skill.get("skill").and_then(Value::as_str) {
            Skill::names
                .iter()
                .position(|candidate| candidate.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("unknown fixture skill {name}"))? as u8
        } else {
            skill
                .get("skill")
                .and_then(Value::as_u64)
                .filter(|index| *index < Skill::names.len() as u64)
                .ok_or("malformed fixture Skill requirement")? as u8
        };
        let level = skill
            .get("level")
            .and_then(Value::as_u64)
            .filter(|level| (1..=99).contains(level))
            .ok_or("malformed fixture Skill level")? as u16;
        levels
            .entry(index)
            .and_modify(|floor| *floor = (*floor).max(level))
            .or_insert(level);
    }
    Ok(levels
        .into_iter()
        .map(|(skill, level)| SkillMinimum { skill, level })
        .collect())
}

fn command_step(name: &'static str, command: String, proof: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |client, _| api::interact::cheat(client, &command).is_sent()),
        },
        wait: Wait {
            arm: proof,
            budget_ticks: 100,
        },
    }
}

/// Jump to one authored stage, stage the exact qualification profile and
/// loadout, then Start. All cheats precede Start; the post-Start steps only
/// observe completion. Always relog after the seed, including permanent
/// non-transmitted varps, so quest-tab colours cannot be stale.
pub fn quester_stage(request: QuesterStage<'_>) -> Result<QuesterFixture, String> {
    let QuesterStage {
        name,
        quest_display,
        path,
        identity,
        selected,
        stage,
        loadout,
        extra_items,
        stand,
    } = request;
    if path.id.0.as_ref() != identity.id || quest_display != identity.display {
        return Err("fixture Path/selected quest identity mismatch".into());
    }
    let selected_identity = selected
        .quest_identity()
        .and_then(|family| family.rows.iter().find(|row| row.id == identity.id))
        .ok_or("fixture quest identity is not in selected content")?;
    if selected_identity.varp != identity.varp
        || selected_identity.varp_id != identity.varp_id
        || selected_identity.display != identity.display
    {
        return Err("fixture quest identity differs from selected content".into());
    }
    if !(0..=16383).contains(&stand.x)
        || !(0..=16383).contains(&stand.z)
        || !(0..=3).contains(&stand.level)
    {
        return Err("fixture stand is outside world coordinates".into());
    }
    let role = path
        .roles
        .first()
        .filter(|_| path.roles.len() == 1)
        .ok_or("single-account fixture needs exactly one Path role")?;
    if !role
        .sequences
        .iter()
        .any(|sequence| sequence.stage.0.as_ref() == stage)
    {
        return Err(format!("fixture stage {stage} has no sequence"));
    }
    let progress = role
        .progress
        .as_ref()
        .ok_or("fixture Path has no progress program")?;
    let mut values = progress
        .rules
        .iter()
        .filter(|rule| rule.stage.0.as_ref() == stage)
        .filter_map(|rule| rule.varp);
    let value = values
        .next()
        .ok_or_else(|| format!("fixture stage {stage} has no varp hint"))?;
    if values.any(|other| other != value) {
        return Err(format!(
            "fixture stage {stage} has contradictory varp hints"
        ));
    }
    let profile = fixture_profile(&identity.id)?;
    let stats = stats_for(path, profile)?;
    let header = path
        .quest
        .as_ref()
        .ok_or("fixture Path has no quest header")?;
    let loadout = match loadout {
        None => None,
        Some(FixtureLoadout::Standard(style)) => Some(standard_kit(style)),
        Some(FixtureLoadout::Path(key)) => Some(
            header
                .loadouts
                .get(key)
                .ok_or_else(|| format!("fixture Path has no loadout {key}"))?
                .clone(),
        ),
    };
    let mut steps = crate::script_live_seed_steps();
    let mut seed_commands = Vec::new();
    {
        let mut push = |name, command: String, proof| {
            seed_commands.push(command.clone());
            steps.push(command_step(name, command, proof));
        };
        push(
            "reset qualification stats before seeding",
            "minme".into(),
            Proof::StatAtMost { id: 1, max: 1 },
        );
        push(
            "clear fixture equipment",
            "~clearinv worn".into(),
            Proof::SideTabAvailable { index: 3 },
        );
        push(
            "clear fixture inventory",
            "~clearinv".into(),
            Proof::SideTabAvailable { index: 3 },
        );
        for stat in &stats {
            // Proof::Stat slot 16 means run energy, not Agility. The full
            // direct-snapshot Await below checks every actual skill slot.
            push(
                "stage qualification skill",
                format!(
                    "setstat {} {}",
                    Skill::names[usize::from(stat.skill)],
                    stat.level
                ),
                Proof::SideTabAvailable { index: 3 },
            );
        }
        push(
            "reset authored quest stage",
            format!("setvar {} {value}", identity.varp),
            Proof::SideTabAvailable { index: 3 },
        );
    }
    let resolve = |name: &str| {
        let item = selected
            .item_by_alias(name)
            .or_else(|| {
                selected.items().iter().find(|item| {
                    item.name
                        .as_deref()
                        .is_some_and(|known| known.eq_ignore_ascii_case(name))
                })
            })
            .ok_or_else(|| format!("fixture item {name} is not in selected content"))?;
        let alias = item
            .alias
            .as_deref()
            .ok_or_else(|| format!("fixture item {name} has no config alias for give"))?;
        Ok::<_, String>((item, alias))
    };
    let mut expected_equipment = Vec::new();
    let mut inventory = BTreeMap::<i32, (&str, i32)>::new();
    if let Some(loadout) = &loadout {
        for name in loadout.worn.values() {
            let (item, alias) = resolve(name)?;
            let id = item.id;
            let command = format!("give {alias} 1");
            seed_commands.push(command.clone());
            steps.push(command_step(
                "seed qualification worn item",
                command,
                Proof::ItemId { id, count: 1 },
            ));
            steps.push(crate::scenarios::combat::wear_combat_item_step(
                "wear qualification kit item",
                id,
            ));
            expected_equipment.push(id);
        }
        for row in &loadout.carry {
            let qty = i32::try_from(row.qty).map_err(|_| "fixture carry quantity overflow")?;
            if qty < 1 {
                return Err("fixture carry quantity must be positive".into());
            }
            let (item, alias) = resolve(&row.item)?;
            let (_, count) = inventory.entry(item.id).or_insert((alias, 0));
            *count = count
                .checked_add(qty)
                .ok_or("fixture carry quantity overflow")?;
        }
    }
    for (name, qty) in extra_items {
        if *qty < 1 {
            return Err("fixture extra item quantity must be positive".into());
        }
        let (item, alias) = resolve(name)?;
        let (_, count) = inventory.entry(item.id).or_insert((alias, 0));
        *count = count
            .checked_add(*qty)
            .ok_or("fixture extra quantity overflow")?;
    }
    let mut expected_inventory = Vec::new();
    for (id, (alias, qty)) in inventory {
        let command = format!("give {alias} {qty}");
        seed_commands.push(command.clone());
        steps.push(command_step(
            "seed qualification carry item",
            command,
            Proof::ItemId { id, count: qty },
        ));
        expected_inventory.push((id, qty));
    }
    steps.push(Step {
        name: "relog after permanent quest stage and loadout seed",
        kind: StepKind::Relog,
        wait: Wait {
            arm: Proof::SideTabAvailable { index: 3 },
            budget_ticks: 600,
        },
    });
    let teleport = api::interact::tele_args(stand.level, stand.x, stand.z);
    seed_commands.push(teleport.clone());
    steps.push(command_step(
        "stand at the authored sequence",
        teleport,
        Proof::Arrived {
            x: stand.x,
            z: stand.z,
            level: stand.level,
        },
    ));
    let gate_stats = stats.clone();
    let gate_inventory = expected_inventory.clone();
    let gate_equipment = expected_equipment.clone();
    steps.push(Step {
        name: "prove the whole fixture profile and loadout before Start",
        kind: StepKind::Await {
            evidence: "exact qualification stats, carried items, worn kit and stand",
            ready: Box::new(move |snapshot| {
                validate_start_seed(snapshot, &gate_stats, &gate_inventory, &gate_equipment).is_ok()
                    && validate_start_stand(snapshot.tile(), stand).is_ok()
            }),
        },
        wait: Wait {
            arm: Proof::SideTabAvailable { index: 3 },
            budget_ticks: 200,
        },
    });
    steps.push(start_compiled_step());
    let proof = Proof::QuestDone {
        name: quest_display,
    };
    steps.push(Step {
        name: "observe native quest completion",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: proof,
            budget_ticks: 12_000,
        },
    });
    let scenario = Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: Duration::from_secs(3600),
            start_script: Some("Quester"),
            terminal_shot: Some(name),
            nav: crate::gold_script_nav(),
            ..Default::default()
        },
    };
    Ok(QuesterFixture {
        scenario,
        start_settings: Map::from_iter([("quests".into(), json!([identity.id]))]),
        seed: QuesterSeed {
            profile,
            stats,
            loadout,
            stand,
            expected_inventory,
            expected_equipment,
        },
        seed_commands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::selected::{ClientRevision, FactKey};
    use script::quester::path::QuestRequirementDocument;

    fn document() -> PathDocument {
        let mut path: PathDocument =
            serde_json::from_str(include_str!("../../script/paths/289/cook.json")).unwrap();
        let progress = path.roles[0].progress.as_mut().unwrap();
        progress
            .rules
            .push(script::quester::path::ProgressRuleDocument {
                stage: FactKey::new("cook:0"),
                all: vec!["fixture start".into()],
                any: vec![],
                not: vec![],
                varp: Some(0),
            });
        path
    }

    fn fixture(path: &PathDocument) -> Result<QuesterFixture, String> {
        fixture_with_loadout(
            path,
            FixtureLoadout::Standard(StandardKit::Melee),
            &[("coins", 1000)],
        )
    }

    fn fixture_with_loadout(
        path: &PathDocument,
        loadout: FixtureLoadout<'_>,
        extra_items: &[(&str, i32)],
    ) -> Result<QuesterFixture, String> {
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let identity = selected
            .quest_identity()
            .unwrap()
            .rows
            .iter()
            .find(|row| row.id == "cook")
            .unwrap();
        quester_stage(QuesterStage {
            name: "quester_profile_test",
            quest_display: "Cook's Assistant",
            path,
            identity,
            selected: &selected,
            stage: "cook:0",
            loadout: Some(loadout),
            extra_items,
            stand: WorldTile {
                x: 3209,
                z: 3215,
                level: 0,
            },
        })
    }

    #[test]
    fn authored_stage_profile_and_kit_are_seeded_only_before_start() {
        let fixture = fixture(&document()).unwrap();
        assert!(fixture
            .seed_commands
            .iter()
            .any(|command| command == "setvar cookquest 0"));
        assert_eq!(
            fixture
                .seed
                .stats
                .iter()
                .map(|stat| (stat.skill, stat.level))
                .collect::<Vec<_>>(),
            [
                (0, 40),
                (1, 40),
                (2, 40),
                (3, 40),
                (4, 40),
                (5, 43),
                (6, 40)
            ]
        );
        let start = fixture
            .scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert!(fixture.scenario.steps[..start]
            .iter()
            .any(|step| matches!(step.kind, StepKind::Relog)));
        assert!(fixture.scenario.steps[..start]
            .iter()
            .any(|step| step.wait.arm == Proof::EquipmentId { id: 1333 }));
        assert!(matches!(
            fixture.scenario.steps[start - 1].kind,
            StepKind::Await { .. }
        ));
        assert_eq!(fixture.scenario.steps[start + 1..].len(), 1);
        assert_eq!(
            fixture.scenario.steps[start + 1].wait.arm,
            Proof::QuestDone {
                name: "Cook's Assistant"
            }
        );
        assert_eq!(
            fixture.start_settings,
            Map::from_iter([("quests".into(), json!(["cook"]))])
        );
    }

    #[test]
    fn relog_precedes_the_proved_authored_stand() {
        let fixture = fixture(&document()).unwrap();
        let relog = fixture
            .scenario
            .steps
            .iter()
            .rposition(|step| matches!(step.kind, StepKind::Relog))
            .unwrap();
        assert_eq!(
            fixture.scenario.steps[relog + 1].wait.arm,
            Proof::Arrived {
                x: 3209,
                z: 3215,
                level: 0
            }
        );
        let stand = fixture.seed.stand;
        assert!(validate_start_stand(None, stand).is_err());
        assert!(validate_start_stand(Some((3220, 3220, 0)), stand).is_err());
        assert!(validate_start_stand(Some((3209, 3215, 0)), stand).is_ok());
    }

    #[test]
    fn display_named_path_kits_seed_aliases_and_merge_alias_extras() {
        let mut path = document();
        path.quest.as_mut().unwrap().loadouts.insert(
            "display".into(),
            QuestLoadoutDocument {
                worn: BTreeMap::from([("righthand".into(), "Rune scimitar".into())]),
                carry: vec![
                    LoadoutCarryDocument {
                        item: "Prayer potion(4)".into(),
                        qty: 1,
                    },
                    LoadoutCarryDocument {
                        item: "Lobster".into(),
                        qty: 6,
                    },
                ],
            },
        );
        let fixture = fixture_with_loadout(
            &path,
            FixtureLoadout::Path("display"),
            &[("4doseprayerrestore", 1), ("coins", 1000)],
        )
        .unwrap();
        for command in [
            "give rune_scimitar 1",
            "give 4doseprayerrestore 2",
            "give lobster 6",
        ] {
            assert!(fixture.seed_commands.iter().any(|seed| seed == command));
        }
        assert_eq!(
            fixture
                .seed_commands
                .iter()
                .filter(|command| command.starts_with("give 4doseprayerrestore "))
                .count(),
            1
        );
        assert!(fixture.seed.expected_inventory.contains(&(2434, 2)));
    }

    #[test]
    fn a_missing_stage_hint_never_seeds_zero() {
        let mut path = document();
        path.roles[0].progress.as_mut().unwrap().rules.clear();
        assert!(fixture(&path).err().unwrap().contains("no varp hint"));
        path.roles[0].sequences[0].stage = FactKey::new("unknown");
        assert!(fixture(&path).err().unwrap().contains("has no sequence"));
    }

    #[test]
    fn requirements_add_only_their_exact_noncombat_levels() {
        let mut path = document();
        path.quest.as_mut().unwrap().requirements = vec![QuestRequirementDocument {
            id: FactKey::new("crafting"),
            kind: json!({"Skill": {"skill": "crafting", "level": 31}}),
            at: "Start".into(),
            source: "fixture regression".into(),
        }];
        let stats = stats_for(&path, TestProfile::Base40).unwrap();
        assert_eq!(
            stats.iter().find(|stat| stat.skill == 12).unwrap().level,
            31
        );
        assert!(!stats.iter().any(|stat| stat.skill == 8));
        assert_eq!(stats_for(&path, TestProfile::Base60).unwrap()[0].level, 60);
    }

    #[test]
    fn unknown_profile_and_loadout_fail_closed() {
        assert!(fixture_profile("not_a_quest").is_err());
        let mut path = document();
        path.quest.as_mut().unwrap().loadouts.insert(
            "broken".into(),
            QuestLoadoutDocument {
                worn: BTreeMap::from([("righthand".into(), "missing_obj".into())]),
                carry: vec![],
            },
        );
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let identity = selected
            .quest_identity()
            .unwrap()
            .rows
            .iter()
            .find(|row| row.id == "cook")
            .unwrap();
        let result = quester_stage(QuesterStage {
            name: "invalid_kit",
            quest_display: "Cook's Assistant",
            path: &path,
            identity,
            selected: &selected,
            stage: "cook:0",
            loadout: Some(FixtureLoadout::Path("broken")),
            extra_items: &[],
            stand: WorldTile {
                x: 3209,
                z: 3215,
                level: 0,
            },
        });
        assert!(result.err().unwrap().contains("not in selected content"));
    }

    #[test]
    fn pre_start_gate_rejects_stale_stats_missing_kit_and_energy_as_agility() {
        use api::snapshot::StatView;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        let expected = [
            SkillMinimum {
                skill: 0,
                level: 40,
            },
            SkillMinimum {
                skill: 16,
                level: 25,
            },
        ];
        let stat = |index, level| StatView {
            index,
            name: Skill::names[index as usize].into(),
            effective: level,
            base: level,
            xp: 0,
            used: true,
        };
        snapshot.seed_stats(vec![stat(0, 99), stat(16, 1)]);
        assert!(validate_start_seed(&snapshot, &expected, &[], &[]).is_err());
        snapshot.seed_stats(vec![stat(0, 40), stat(16, 1)]);
        assert!(validate_start_seed(&snapshot, &expected, &[], &[]).is_err());
        snapshot.seed_stats(vec![stat(0, 40), stat(16, 25)]);
        assert!(validate_start_seed(&snapshot, &expected, &[], &[]).is_ok());
        assert!(validate_start_seed(&snapshot, &expected, &[], &[1333]).is_err());
        assert!(validate_start_seed(&snapshot, &expected, &[(995, 1000)], &[]).is_err());
    }

    #[test]
    fn profiles_have_unique_quests_and_operator_hard_floor() {
        let mut ids = std::collections::HashSet::new();
        assert!(FIXTURE_PROFILES.iter().all(|row| ids.insert(row.quest)));
        for quest in [
            "dragon",
            "ikov",
            "upass",
            "legends",
            "elemental_workshop",
            "horror",
        ] {
            assert_eq!(fixture_profile(quest).unwrap(), TestProfile::Base60);
        }
    }
}
