//! Path-backed Quester qualification fixtures, never product gameplay.
//!
//! New Paths use [`quester_stage`] here. The older numeric
//! [`crate::quester_stage`] remains the low-level seed used by the shipped
//! S2 and combat/WalkGuard cells; their bespoke stat tuples are unchanged.
use crate::{start_compiled_step, Proof, Scenario, ScenarioSettings, Seed, Step, StepKind, Wait};
use api::game_data::{QuestIdentityRow, SelectedGameData};
use api::selected::SkillMinimum;
use api::selected::{ClientRevision, FamilyPreparation};
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::skill::Skill;
use script::quester::path::{
    LoadoutCarryDocument, PathDocument, QuestLoadoutDocument, QuestRequirementKindDocument,
};
use script::quester::registry::{self, FolderSource};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Runtime controls for the live `quester_path` cell only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuesterFastSettings {
    pub tick_ms: Option<u32>,
    pub sustain_run: bool,
}

impl QuesterFastSettings {
    pub fn from_env() -> Result<Self, String> {
        let read = |name: &str| {
            std::env::var_os(name)
                .map(|value| {
                    value
                        .into_string()
                        .map_err(|_| format!("{name} must be UTF-8"))
                })
                .transpose()
        };
        Self::parse(
            read("QUESTER_TICK_MS")?.as_deref(),
            read("QUESTER_SUSTAIN_RUN")?.as_deref(),
        )
    }

    pub fn parse(tick_ms: Option<&str>, sustain_run: Option<&str>) -> Result<Self, String> {
        let tick_ms = tick_ms
            .map(|raw| -> Result<u32, String> {
                let value = raw
                    .trim()
                    .parse::<u32>()
                    .map_err(|error| format!("invalid QUESTER_TICK_MS {raw:?}: {error}"))?;
                if value < 300 {
                    return Err("QUESTER_TICK_MS must be at least 300".into());
                }
                Ok(value)
            })
            .transpose()?;
        let sustain_run = match sustain_run.map(str::trim) {
            None => tick_ms.is_some(),
            Some("0") => false,
            Some("1") => true,
            Some(value) => {
                return Err(format!("QUESTER_SUSTAIN_RUN must be 0 or 1, got {value:?}"))
            }
        };
        Ok(Self {
            tick_ms,
            sustain_run,
        })
    }
}

/// Apply the two opt-in live harness levers to one `quester_path` scenario.
pub fn apply_quester_fast_settings(scenario: &mut Scenario, settings: QuesterFastSettings) {
    if let Some(ms) = settings.tick_ms {
        scenario.settings.nav.engine_speed_ms = Some(ms);
        scenario.settings.teardown_world_speed_ms = Some(600);
        scenario.steps.insert(
            0,
            Step {
                name: "confirm quest world tick speed",
                kind: StepKind::Perform {
                    send: Box::new(|_, _| true),
                },
                wait: Wait {
                    arm: Proof::WorldSpeedChanged { ms },
                    budget_ticks: 80,
                },
            },
        );
    }
    if settings.sustain_run {
        scenario
            .settings
            .sustains
            .extend(crate::nav_energy_sustains());
    }
}

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
/// `display` is the selected quest-tab display; fixture builds check it
/// against selected content, so drift fails closed instead of seeding the
/// wrong quest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestFixtureProfile {
    pub quest: &'static str,
    pub display: &'static str,
    pub profile: TestProfile,
}

pub const FIXTURE_PROFILES: &[QuestFixtureProfile] = &[
    QuestFixtureProfile {
        quest: "cook",
        display: "Cook's Assistant",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "sheep",
        display: "Sheep Shearer",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "runemysteries",
        display: "Rune Mysteries Quest",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "romeojuliet",
        display: "Romeo & Juliet",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "imp",
        display: "Imp Catcher",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "prince",
        display: "Prince Ali Rescue",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "hunt",
        display: "Pirate's Treasure",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "priest",
        display: "The Restless Ghost",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "vampire",
        display: "Vampire Slayer",
        profile: TestProfile::Base40,
    },
    // PATHS-OVERNIGHT folder Paths. Base40 until the operator names Base60
    // or a recorded live run establishes the need.
    QuestFixtureProfile {
        quest: "cog",
        display: "Clock Tower",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "death",
        display: "Death Plateau",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "demon",
        display: "Demon Slayer",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "desertrescue",
        display: "The Tourist Trap",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "doric",
        display: "Doric's Quest",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "gobdip",
        display: "Goblin Diplomacy",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "hetty",
        display: "Witch's Potion",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "priestperil",
        display: "Priest in Peril",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "squire",
        display: "The Knight's Sword",
        profile: TestProfile::Base40,
    },
    // Operator-mandated Base60 quests (README, Q-RESET qualification floor).
    QuestFixtureProfile {
        quest: "dragon",
        display: "Dragon Slayer",
        profile: TestProfile::Base60,
    },
    QuestFixtureProfile {
        quest: "ikov",
        display: "Temple of Ikov",
        profile: TestProfile::Base60,
    },
    QuestFixtureProfile {
        quest: "upass",
        display: "Underground Pass",
        profile: TestProfile::Base60,
    },
    QuestFixtureProfile {
        quest: "legends",
        display: "Legends Quest",
        profile: TestProfile::Base60,
    },
    QuestFixtureProfile {
        quest: "elemental_workshop",
        display: "Elemental Workshop",
        profile: TestProfile::Base60,
    },
    QuestFixtureProfile {
        quest: "horror",
        display: "Horror from the Deep",
        profile: TestProfile::Base60,
    },
    QuestFixtureProfile {
        quest: "blackarmgang",
        display: "Shield of Arrav",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "hero",
        display: "Hero's Quest",
        profile: TestProfile::Base40,
    },
    QuestFixtureProfile {
        quest: "barcrawl",
        display: "Alfred Grimhand's Barcrawl",
        profile: TestProfile::Base40,
    },
];

pub fn fixture_profile(quest: &str) -> Result<TestProfile, String> {
    fixture_row(quest).map(|row| row.profile)
}

/// The full harness row for a quest: static id, display and profile, so
/// env-driven cells need no allocation to satisfy fixture lifetimes.
pub fn fixture_row(quest: &str) -> Result<QuestFixtureProfile, String> {
    FIXTURE_PROFILES
        .iter()
        .find(|row| row.quest == quest)
        .copied()
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
/// the same selected revision. Stage values come from the Path's varp hints;
/// zero is never substituted for an absent/synthetic hint — a hint-less stage
/// (released Cook/Sheep/Rune/Romeo/Imp) needs an explicit operator seed via
/// [`quester_stage_with_seeds`].
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

/// A real card-backed miniquest: explicit content variable seeds, no quest identity.
pub struct MiniquestStage<'a> {
    pub name: &'static str,
    pub path: &'a PathDocument,
    pub selected: &'a SelectedGameData,
    pub stage: &'a str,
    pub loadout: Option<FixtureLoadout<'a>>,
    pub extra_items: &'a [(&'a str, i32)],
    pub stand: WorldTile,
    pub seed_vars: &'a [(&'a str, i32)],
    pub proof: Proof,
}

struct FixtureSeed<'a> {
    name: &'static str,
    path: &'a PathDocument,
    selected: &'a SelectedGameData,
    loadout: Option<FixtureLoadout<'a>>,
    extra_items: &'a [(&'a str, i32)],
    stand: WorldTile,
    seed_vars: &'a [(&'a str, i32)],
    proof: Proof,
}

/// Optional JSON input shared by the headless and headed Path harnesses.
#[derive(Debug, Deserialize)]
pub struct QuesterPathSeeds {
    pub stage: String,
    pub stand: [i32; 3],
    #[serde(default)]
    pub loadout: Option<QuesterPathLoadout>,
    #[serde(default)]
    pub extra_items: Vec<(String, i32)>,
    #[serde(default)]
    pub seed_vars: Vec<(String, i32)>,
    #[serde(default)]
    pub before_relog: Vec<String>,
    #[serde(default)]
    pub expect: Vec<String>,
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize)]
pub enum QuesterPathLoadout {
    #[serde(rename = "path")]
    Path(String),
    #[serde(rename = "standard")]
    Standard(String),
}

impl QuesterPathSeeds {
    pub fn fixture_loadout(&self) -> Result<Option<FixtureLoadout<'_>>, String> {
        self.loadout
            .as_ref()
            .map(|loadout| match loadout {
                QuesterPathLoadout::Path(key) => Ok(FixtureLoadout::Path(key)),
                QuesterPathLoadout::Standard(style) => match style.as_str() {
                    "melee" => Ok(FixtureLoadout::Standard(StandardKit::Melee)),
                    "magic" => Ok(FixtureLoadout::Standard(StandardKit::Magic)),
                    "ranged" => Ok(FixtureLoadout::Standard(StandardKit::Ranged)),
                    other => Err(format!("unknown standard kit {other:?}")),
                },
            })
            .transpose()
    }

    pub fn extra_item_refs(&self) -> Vec<(&str, i32)> {
        self.extra_items
            .iter()
            .map(|(item, quantity)| (item.as_str(), *quantity))
            .collect()
    }

    pub fn seed_var_refs(&self) -> Vec<(&str, i32)> {
        self.seed_vars
            .iter()
            .map(|(varp, value)| (varp.as_str(), *value))
            .collect()
    }

    pub fn before_relog_steps(&self) -> Vec<Step> {
        self.before_relog
            .iter()
            .cloned()
            .map(|command| {
                command_step(
                    "seed account state before the final relog",
                    command,
                    Proof::SideTabAvailable { index: 3 },
                )
            })
            .collect()
    }
}

pub fn parse_quester_path_seeds(text: &str, source: &str) -> Result<QuesterPathSeeds, String> {
    serde_json::from_str(text).map_err(|error| format!("parse {source}: {error}"))
}

/// Inputs shared by the headless Path cell and the headed catalog scenario.
pub struct QuesterPathFixture<'a> {
    pub name: &'static str,
    pub quest_display: &'static str,
    pub path: &'a PathDocument,
    pub selected: &'a SelectedGameData,
    pub stage: Option<&'a str>,
    pub loadout: Option<FixtureLoadout<'a>>,
    pub extra_items: &'a [(&'a str, i32)],
    pub seed_vars: &'a [(&'a str, i32)],
    pub stand: WorldTile,
    pub before_relog: Vec<Step>,
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
        let QuestRequirementKindDocument::Skill { skill, level } = &requirement.kind else {
            continue;
        };
        let index = Skill::names
            .iter()
            .position(|candidate| candidate.eq_ignore_ascii_case(skill))
            .ok_or_else(|| format!("unknown fixture skill {skill}"))? as u8;
        if !(1..=99).contains(level) {
            return Err("malformed fixture Skill level".into());
        }
        levels
            .entry(index)
            .and_modify(|floor| *floor = (*floor).max(*level))
            .or_insert(*level);
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
    quester_stage_with_seeds(request, &[])
}

/// [`quester_stage`] with explicit caller seeds for stages the Path does not
/// hint. The released Cook/Sheep/Rune/Romeo/Imp Paths carry no progress varp
/// hints; for those stages the caller must name the stage varp and value in
/// `seed_vars` (an operator seed recorded in `seed_commands`, never a
/// substituted zero). Stages with hints use the hint for the stage value and
/// `seed_vars` only for auxiliary seeds, as before.
pub fn quester_stage_with_seeds(
    request: QuesterStage<'_>,
    seed_vars: &[(&str, i32)],
) -> Result<QuesterFixture, String> {
    let role = request
        .path
        .roles
        .first()
        .filter(|_| request.path.roles.len() == 1)
        .ok_or("single-account fixture needs exactly one Path role")?;
    let stage_varp = request.identity.varp.as_str();
    build_quest_fixture(request, role, stage_varp, seed_vars)
}

/// Build the same single-account Base40 fixture for headed and headless
/// Path runs. `stage: None` starts from a clean quest account; an explicit
/// stage uses the shared stage-seeding builder.
pub fn build_quester_path_fixture(
    request: QuesterPathFixture<'_>,
) -> Result<QuesterFixture, String> {
    let row = fixture_row(request.path.id.0.as_ref())?;
    if row.profile != TestProfile::Base40 {
        return Err(format!(
            "{} does not have a Base40 qualification profile",
            request.path.id.0
        ));
    }
    if row.display != request.quest_display {
        return Err(format!(
            "{} fixture display {:?} differs from requested {:?}",
            request.path.id.0, row.display, request.quest_display
        ));
    }
    if request.path.kind != script::quester::path::PathKind::Quest
        || request.path.partner.is_some()
        || request.path.roles.len() != 1
    {
        return Err("Path fixture needs one single-account quest role".into());
    }
    let identity = request
        .selected
        .quest_identity()
        .and_then(|table| {
            table
                .rows
                .iter()
                .find(|row| row.id == request.path.id.0.as_ref())
        })
        .ok_or_else(|| format!("no quest identity row for {}", request.path.id.0))?;
    let mut fixture = if let Some(stage) = request.stage {
        quester_stage_with_seeds(
            QuesterStage {
                name: request.name,
                quest_display: request.quest_display,
                path: request.path,
                identity,
                selected: request.selected,
                stage,
                loadout: request.loadout,
                extra_items: request.extra_items,
                stand: request.stand,
            },
            request.seed_vars,
        )?
    } else {
        validate_path_identity(
            request.path,
            identity,
            request.selected,
            request.quest_display,
        )?;
        build_fixture(FixtureSeed {
            name: request.name,
            path: request.path,
            selected: request.selected,
            loadout: request.loadout,
            extra_items: request.extra_items,
            stand: request.stand,
            seed_vars: request.seed_vars,
            proof: Proof::QuestDone {
                name: request.quest_display,
            },
        })?
    };
    if !request.before_relog.is_empty() {
        let relog = fixture
            .scenario
            .steps
            .iter()
            .rposition(|step| matches!(step.kind, StepKind::Relog))
            .ok_or("Path fixture has no final relog")?;
        fixture
            .scenario
            .steps
            .splice(relog..relog, request.before_relog);
    }
    Ok(fixture)
}

/// Prepare one explicitly selected paired role; the shared pair runner starts both.
/// `stage_varp` names the pinned role's real content variable, not a synthetic identity.
pub fn quester_role_stage(
    request: QuesterStage<'_>,
    gang: script::quester::pair::Gang,
    stage_varp: &str,
    seed_vars: &[(&str, i32)],
) -> Result<QuesterFixture, String> {
    let declaration = request
        .path
        .partner
        .as_ref()
        .ok_or("role fixture needs a partner declaration")?;
    let declared = declaration
        .roles
        .iter()
        .find(|role| role.gang == gang)
        .ok_or("fixture gang is not declared by the Path")?;
    let role = request
        .path
        .roles
        .iter()
        .find(|role| role.role.as_ref() == Some(&declared.id))
        .ok_or("fixture role is not authored by the Path")?;
    let mut fixture = build_quest_fixture(request, role, stage_varp, seed_vars)?;
    fixture.start_settings.insert(
        "gang".into(),
        json!(match gang {
            script::quester::pair::Gang::Phoenix => "phoenix",
            script::quester::pair::Gang::BlackArm => "blackarm",
        }),
    );
    Ok(fixture)
}

fn validate_path_identity(
    path: &PathDocument,
    identity: &QuestIdentityRow,
    selected: &SelectedGameData,
    quest_display: &str,
) -> Result<(), String> {
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
    Ok(())
}

fn validate_identity(request: &QuesterStage<'_>) -> Result<(), String> {
    validate_path_identity(
        request.path,
        request.identity,
        request.selected,
        request.quest_display,
    )
}

fn validate_stage(
    role: &script::quester::path::PathRoleDocument,
    stage: &str,
) -> Result<(), String> {
    if !role
        .sequences
        .iter()
        .any(|sequence| sequence.stage.0.as_ref() == stage)
    {
        return Err(format!("fixture stage {stage} has no sequence"));
    }
    Ok(())
}

/// Operator seed for a stage the Path does not hint. This is not a
/// substituted zero: the caller names the stage varp and value explicitly,
/// and the seed is recorded in `seed_commands` with every other cheat.
fn explicit_stage_seed(
    extra_vars: &[(&str, i32)],
    stage_varp: &str,
    stage: &str,
) -> Result<i32, String> {
    let mut explicit = extra_vars.iter().filter(|(varp, _)| *varp == stage_varp);
    let value = explicit
        .next()
        .ok_or_else(|| {
            format!("fixture stage {stage} has no varp hint; pass an explicit {stage_varp} seed")
        })?
        .1;
    if explicit.next().is_some() {
        return Err(format!(
            "fixture stage {stage} has contradictory explicit seeds"
        ));
    }
    Ok(value)
}

fn build_quest_fixture(
    request: QuesterStage<'_>,
    role: &script::quester::path::PathRoleDocument,
    stage_varp: &str,
    extra_vars: &[(&str, i32)],
) -> Result<QuesterFixture, String> {
    validate_identity(&request)?;
    validate_stage(role, request.stage)?;
    let progress = role
        .progress
        .as_ref()
        .ok_or("fixture Path has no progress program")?;
    let mut values = progress
        .rules
        .iter()
        .filter(|rule| rule.stage.0.as_ref() == request.stage)
        .filter_map(|rule| rule.varp);
    let (value, used_explicit) = match values.next() {
        Some(value) => {
            if values.any(|other| other != value) {
                return Err(format!(
                    "fixture stage {} has contradictory varp hints",
                    request.stage
                ));
            }
            (value, false)
        }
        // No authored hint (released Cook/Sheep/Rune/Romeo/Imp): accept one
        // explicit operator seed naming the stage varp, recorded below like
        // every other cheat. Anything else still fails closed.
        None => (
            explicit_stage_seed(extra_vars, stage_varp, request.stage)?,
            true,
        ),
    };
    let mut seed_vars = Vec::with_capacity(extra_vars.len() + 1);
    seed_vars.push((stage_varp, value));
    if used_explicit {
        // The explicit stage seed is already pushed; re-adding it would trip
        // the unique-alias check in `build_fixture`.
        seed_vars.extend(
            extra_vars
                .iter()
                .filter(|(varp, _)| *varp != stage_varp)
                .copied(),
        );
    } else {
        seed_vars.extend_from_slice(extra_vars);
    }
    build_fixture(FixtureSeed {
        name: request.name,
        path: request.path,
        selected: request.selected,
        loadout: request.loadout,
        extra_items: request.extra_items,
        stand: request.stand,
        seed_vars: &seed_vars,
        proof: Proof::QuestDone {
            name: request.quest_display,
        },
    })
}

/// The same qualification profile/loadout/Start proof as a quest, without a fake tab.
pub fn miniquest_stage(request: MiniquestStage<'_>) -> Result<QuesterFixture, String> {
    if request.path.kind != script::quester::path::PathKind::Miniquest
        || request.path.partner.is_some()
        || request.path.roles.len() != 1
    {
        return Err("miniquest fixture needs one real card-backed Path role".into());
    }
    validate_stage(&request.path.roles[0], request.stage)?;
    if request.seed_vars.is_empty() {
        return Err("miniquest fixture needs explicit content seeds".into());
    }
    build_fixture(FixtureSeed {
        name: request.name,
        path: request.path,
        selected: request.selected,
        loadout: request.loadout,
        extra_items: request.extra_items,
        stand: request.stand,
        seed_vars: request.seed_vars,
        proof: request.proof,
    })
}

fn build_fixture(request: FixtureSeed<'_>) -> Result<QuesterFixture, String> {
    let FixtureSeed {
        name,
        path,
        selected,
        loadout,
        extra_items,
        stand,
        seed_vars,
        proof,
    } = request;
    if !(0..=16383).contains(&stand.x)
        || !(0..=16383).contains(&stand.z)
        || !(0..=3).contains(&stand.level)
    {
        return Err("fixture stand is outside world coordinates".into());
    }
    let profile = fixture_profile(path.id.0.as_ref())?;
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
        for (index, (varp, value)) in seed_vars.iter().enumerate() {
            if varp.is_empty()
                || !varp
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                || seed_vars[..index].iter().any(|(other, _)| other == varp)
            {
                return Err("fixture content variable must be a unique config alias".into());
            }
            push(
                "reset authored content stage",
                format!("setvar {varp} {value}"),
                Proof::SideTabAvailable { index: 3 },
            );
        }
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
        start_settings: Map::from_iter([("quests".into(), json!([path.id.0.as_ref()]))]),
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

/// Load the currently published Path bytes. `reload_quester_path_source`
/// decides whether that snapshot is bundled or supplied by a folder.
pub fn load_quester_path_document(quest: &str) -> Result<(PathDocument, &'static str), String> {
    let paths = registry::snapshot();
    let source = paths.path_source(quest).label();
    let bytes = paths
        .bytes(quest)
        .ok_or_else(|| format!("{quest} is not available from the {source} Path registry"))?;
    let path: PathDocument =
        serde_json::from_slice(bytes.as_ref()).map_err(|error| format!("{quest}: {error}"))?;
    if path.id.0.as_ref() != quest {
        return Err(format!(
            "Path registry returned id {:?} for requested {quest}",
            path.id.0
        ));
    }
    Ok((path, source))
}

/// Publish one source in the existing Path registry and ensure it serves the
/// requested id. An explicitly supplied folder must contain that Path rather
/// than silently falling back to the embedded index.
pub fn reload_quester_path_source(
    quest: &str,
    folder: Option<&Path>,
) -> Result<&'static str, String> {
    let source = if let Some(folder) = folder {
        if !folder.is_absolute() || !folder.is_dir() {
            return Err(format!(
                "QUESTER_PATH_DIR must be an absolute directory: {}",
                folder.display()
            ));
        }
        FolderSource {
            enabled: true,
            folder: folder.to_owned(),
        }
    } else {
        FolderSource::default()
    };
    let previous = registry::source();
    registry::set_source(source.clone());
    let loaded = (|| {
        let selected =
            FamilyPreparation::run(|_| api::game_data::for_revision(ClientRevision::R289))
                .map_err(|error| format!("selected data worker: {error:?}"))?
                .join()
                .map_err(|error| format!("selected data worker: {error:?}"))?
                .map_err(|error| format!("selected R289 data: {error}"))?;
        let data = std::sync::Arc::clone(&selected);
        let loaded = FamilyPreparation::run(move |worker| registry::reload(&data, worker))
            .map_err(|error| format!("Path registry worker: {error:?}"))?
            .join()
            .map_err(|error| format!("Path registry worker: {error:?}"))?
            .map_err(|error| format!("Path registry reload: {error:?}"))?;
        let path_source = loaded.path_source(quest);
        if loaded.bytes(quest).is_none() {
            return Err(format!(
                "{quest} is not served by the {} Path registry",
                if source.enabled { "folder" } else { "embedded" }
            ));
        }
        if source.enabled
            && !matches!(
                path_source,
                script::quester::registry::PathSource::Folder
                    | script::quester::registry::PathSource::Draft
            )
        {
            return Err(format!(
                "{quest} is not present in QUESTER_PATH_DIR {}",
                source.folder.display()
            ));
        }
        Ok(path_source.label())
    })();
    if loaded.is_err() {
        registry::set_source(previous);
    }
    loaded
}

/// The initial sequence is the sequence corresponding to the Path's
/// `not_started` colour. Teleporting to its first authored approach anchor
/// gives the headed Path run the same deterministic starting point as its
/// fixture builder.
pub fn quester_path_start_anchor(path: &PathDocument) -> Result<WorldTile, String> {
    let role = path
        .roles
        .first()
        .filter(|_| path.roles.len() == 1)
        .ok_or("Path start needs exactly one role")?;
    let progress = role
        .progress
        .as_ref()
        .ok_or("Path start role has no progress colours")?;
    let not_started = progress.colour.not_started.0.as_ref();
    let sequence = role
        .sequences
        .iter()
        .find(|sequence| sequence.stage.0.as_ref() == not_started)
        .ok_or_else(|| format!("Path has no not-started sequence {not_started}"))?;
    for step in &sequence.steps {
        if let Some(tile) = find_anchor_tile(&step.args)? {
            return Ok(tile);
        }
    }
    Err(format!(
        "Path not-started sequence {not_started} has no authored anchor.tile"
    ))
}

fn find_anchor_tile(value: &Value) -> Result<Option<WorldTile>, String> {
    if let Some(tile) = value.get("anchor").and_then(|anchor| anchor.get("tile")) {
        let coordinates = tile
            .as_array()
            .filter(|coordinates| coordinates.len() == 3)
            .ok_or("Path anchor.tile must contain [x,z,level]")?;
        let coordinate = |index: usize| {
            coordinates[index]
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .ok_or("Path anchor.tile coordinates must be integers")
        };
        let tile = WorldTile {
            x: coordinate(0)?,
            z: coordinate(1)?,
            level: coordinate(2)?,
        };
        if !(0..=16383).contains(&tile.x)
            || !(0..=16383).contains(&tile.z)
            || !(0..=3).contains(&tile.level)
        {
            return Err("Path anchor.tile is outside world coordinates".into());
        }
        return Ok(Some(tile));
    }
    match value {
        Value::Array(values) => {
            for value in values {
                if let Some(tile) = find_anchor_tile(value)? {
                    return Ok(Some(tile));
                }
            }
        }
        Value::Object(fields) => {
            for value in fields.values() {
                if let Some(tile) = find_anchor_tile(value)? {
                    return Ok(Some(tile));
                }
            }
        }
        _ => {}
    }
    Ok(None)
}

/// Runtime-selected `quester_path` scenario used by `panel-play --live`.
pub fn quester_path_scenario_from_env() -> Result<Scenario, String> {
    let fast_settings = QuesterFastSettings::from_env()?;
    let quest = require_quester_path(std::env::var("QUESTER_PATH").ok())?;
    let deadline = quester_path_deadline()?;
    let folder = std::env::var_os("QUESTER_PATH_DIR").map(PathBuf::from);
    reload_quester_path_source(&quest, folder.as_deref())?;
    let (path, _) = load_quester_path_document(&quest)?;
    let row = fixture_row(&quest)?;
    let start = quester_path_start_anchor(&path)?;
    let selected = api::game_data::for_revision(ClientRevision::R289)?;

    let seeds = std::env::var_os("QUESTER_SEEDS")
        .map(|path| {
            let path = PathBuf::from(path);
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("read {}: {error}", path.display()))?;
            parse_quester_path_seeds(&text, "QUESTER_SEEDS")
        })
        .transpose()?;
    if let Some(mode) = seeds.as_ref().and_then(|seeds| seeds.mode.as_deref()) {
        if !matches!(mode, "stage" | "clean") {
            return Err(format!("unknown smoke mode {mode:?}"));
        }
    }
    let stand = if let Some(seeds) = &seeds {
        let stand = WorldTile {
            x: seeds.stand[0],
            z: seeds.stand[1],
            level: seeds.stand[2],
        };
        if stand != start {
            return Err(format!(
                "QUESTER_SEEDS stand ({},{},{}) must match Path start anchor ({},{},{})",
                stand.x, stand.z, stand.level, start.x, start.z, start.level
            ));
        }
        stand
    } else {
        start
    };
    let loadout = match &seeds {
        Some(seeds) => seeds.fixture_loadout()?,
        None => None,
    };
    let extra_items = seeds
        .as_ref()
        .map(QuesterPathSeeds::extra_item_refs)
        .unwrap_or_default();
    let seed_vars = seeds
        .as_ref()
        .map(QuesterPathSeeds::seed_var_refs)
        .unwrap_or_default();
    let before_relog = seeds
        .as_ref()
        .map(QuesterPathSeeds::before_relog_steps)
        .unwrap_or_default();
    let mut fixture = build_quester_path_fixture(QuesterPathFixture {
        name: "quester_path",
        quest_display: row.display,
        path: &path,
        selected: &selected,
        stage: seeds.as_ref().map(|seeds| seeds.stage.as_str()),
        loadout,
        extra_items: &extra_items,
        seed_vars: &seed_vars,
        stand,
        before_relog,
    })?;
    fixture.scenario.settings.script_settings_overrides = Some(fixture.start_settings);
    fixture.scenario.settings.deadline = deadline;
    fixture.scenario.settings.terminal_shot = Some("quester_path");
    apply_quester_fast_settings(&mut fixture.scenario, fast_settings);
    Ok(fixture.scenario)
}

fn require_quester_path(value: Option<String>) -> Result<String, String> {
    let quest = value
        .ok_or_else(|| "QUESTER_PATH is required; set it to a Path id such as `cook`".to_owned())?;
    if quest.trim().is_empty() {
        return Err("QUESTER_PATH must be a non-empty Path id".into());
    }
    Ok(quest)
}

fn quester_path_deadline() -> Result<Duration, String> {
    let raw = std::env::var_os("QUESTER_PATH_DEADLINE_S")
        .map(|raw| {
            raw.into_string()
                .map_err(|_| "QUESTER_PATH_DEADLINE_S must be UTF-8 seconds".to_owned())
        })
        .transpose()?;
    quester_path_deadline_from(raw.as_deref())
}

fn quester_path_deadline_from(raw: Option<&str>) -> Result<Duration, String> {
    const DEFAULT_SECONDS: u64 = 45 * 60;
    let Some(raw) = raw else {
        return Ok(Duration::from_secs(DEFAULT_SECONDS));
    };
    let seconds = raw
        .parse::<u64>()
        .map_err(|error| format!("invalid QUESTER_PATH_DEADLINE_S {raw:?}: {error}"))?;
    if seconds == 0 {
        return Err("QUESTER_PATH_DEADLINE_S must be greater than zero".into());
    }
    Ok(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::selected::{ClientRevision, FactKey};
    use script::quester::path::QuestRequirementDocument;

    #[test]
    fn quester_fast_settings_parse_tick_and_sustain_controls() {
        assert_eq!(
            QuesterFastSettings::parse(None, None).unwrap(),
            QuesterFastSettings {
                tick_ms: None,
                sustain_run: false,
            }
        );
        assert_eq!(
            QuesterFastSettings::parse(Some(" 300 "), None).unwrap(),
            QuesterFastSettings {
                tick_ms: Some(300),
                sustain_run: true,
            }
        );
        assert_eq!(
            QuesterFastSettings::parse(Some("600"), Some("0")).unwrap(),
            QuesterFastSettings {
                tick_ms: Some(600),
                sustain_run: false,
            }
        );
        assert_eq!(
            QuesterFastSettings::parse(None, Some("1")).unwrap(),
            QuesterFastSettings {
                tick_ms: None,
                sustain_run: true,
            }
        );
        assert!(QuesterFastSettings::parse(Some("299"), None).is_err());
        assert!(QuesterFastSettings::parse(Some("bad"), None).is_err());
        assert!(QuesterFastSettings::parse(None, Some("true")).is_err());
    }

    #[test]
    fn quester_fast_settings_add_confirmation_energy_and_teardown() {
        let mut scenario = fixture(&document()).unwrap().scenario;
        let original_steps = scenario.steps.len();
        apply_quester_fast_settings(
            &mut scenario,
            QuesterFastSettings {
                tick_ms: Some(300),
                sustain_run: true,
            },
        );
        assert_eq!(scenario.settings.nav.engine_speed_ms, Some(300));
        assert_eq!(scenario.settings.teardown_world_speed_ms, Some(600));
        assert_eq!(scenario.steps.len(), original_steps + 1);
        assert_eq!(
            scenario.steps[0].wait.arm,
            Proof::WorldSpeedChanged { ms: 300 }
        );
        assert_eq!(scenario.steps[0].name, "confirm quest world tick speed");
        assert_eq!(scenario.settings.sustains, crate::nav_energy_sustains());
    }

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
            kind: QuestRequirementKindDocument::Skill {
                skill: "crafting".into(),
                level: 31,
            },
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

    #[test]
    fn card_fixture_reuses_the_profile_without_a_synthetic_quest_identity() {
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let identity = selected
            .quest_identity()
            .unwrap()
            .rows
            .iter()
            .find(|row| row.id == "barcrawl")
            .expect("native zero-QP miniquest identity");
        assert_eq!(identity.quest_points, 0);
        assert_eq!(identity.varp_id, 77);
        // The fixture builder needs a miniquest document, not the private
        // production Path or a made-up quest-tab identity.
        let mut path = document();
        path.id = FactKey::new("barcrawl");
        path.kind = script::quester::path::PathKind::Miniquest;
        path.quest.as_mut().unwrap().quest_points = 0;
        path.roles[0].sequences[0].stage = FactKey::new("barcrawl:tour");
        let card = selected.item_by_alias("barcrawl_card").unwrap().id;
        let fixture = miniquest_stage(MiniquestStage {
            name: "barcrawl_fixture_unit",
            path: &path,
            selected: &selected,
            stage: "barcrawl:tour",
            loadout: None,
            extra_items: &[("barcrawl_card", 1), ("coins", 408)],
            stand: WorldTile {
                x: 3044,
                z: 3258,
                level: 0,
            },
            seed_vars: &[("barcrawl", 2305)],
            proof: Proof::ItemIdAtMost { id: card, count: 0 },
        })
        .unwrap();
        assert!(fixture
            .seed_commands
            .iter()
            .any(|command| command == "setvar barcrawl 2305"));
        assert_eq!(fixture.start_settings["quests"], json!(["barcrawl"]));
        assert_eq!(fixture.seed.profile, TestProfile::Base40);
        assert_eq!(
            fixture.scenario.proof,
            Proof::ItemIdAtMost { id: card, count: 0 }
        );
        let start = fixture
            .scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert!(
            fixture.scenario.steps[start + 1..]
                .iter()
                .all(|step| { step.name == "observe native quest completion" }),
            "no content variable seed may run after Start"
        );
    }

    fn raw_cook() -> PathDocument {
        serde_json::from_str(include_str!("../../script/paths/289/cook.json")).unwrap()
    }

    fn hint_less_cook(
        path: &PathDocument,
        selected: &SelectedGameData,
        identity: &QuestIdentityRow,
        seed_vars: &[(&str, i32)],
    ) -> Result<QuesterFixture, String> {
        quester_stage_with_seeds(
            QuesterStage {
                name: "quester_explicit_seed",
                quest_display: "Cook's Assistant",
                path,
                identity,
                selected,
                stage: "cook:0",
                loadout: None,
                extra_items: &[],
                stand: WorldTile {
                    x: 3209,
                    z: 3215,
                    level: 0,
                },
            },
            seed_vars,
        )
    }

    #[test]
    fn hint_less_stage_builds_with_one_explicit_operator_seed() {
        // The released Cook Path carries no progress varp hints.
        let path = raw_cook();
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let identity = selected
            .quest_identity()
            .unwrap()
            .rows
            .iter()
            .find(|row| row.id == "cook")
            .unwrap();
        let fixture = hint_less_cook(&path, &selected, identity, &[("cookquest", 0)]).unwrap();
        assert!(fixture
            .seed_commands
            .iter()
            .any(|command| command == "setvar cookquest 0"));
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
    }

    #[test]
    fn hint_less_stage_without_an_explicit_seed_still_fails() {
        let path = raw_cook();
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let identity = selected
            .quest_identity()
            .unwrap()
            .rows
            .iter()
            .find(|row| row.id == "cook")
            .unwrap();
        assert!(hint_less_cook(&path, &selected, identity, &[])
            .err()
            .unwrap()
            .contains("no varp hint"));
    }

    #[test]
    fn contradictory_explicit_stage_seeds_fail() {
        let path = raw_cook();
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let identity = selected
            .quest_identity()
            .unwrap()
            .rows
            .iter()
            .find(|row| row.id == "cook")
            .unwrap();
        assert!(hint_less_cook(
            &path,
            &selected,
            identity,
            &[("cookquest", 0), ("cookquest", 1)]
        )
        .err()
        .unwrap()
        .contains("contradictory explicit seeds"));
    }

    #[test]
    fn headed_path_fixture_uses_start_anchor_and_dynamic_quest_settings() {
        let path = raw_cook();
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let start = quester_path_start_anchor(&path).unwrap();
        assert_eq!(
            start,
            WorldTile {
                x: 3209,
                z: 3215,
                level: 0,
            }
        );
        let fixture = build_quester_path_fixture(QuesterPathFixture {
            name: "quester_path",
            quest_display: "Cook's Assistant",
            path: &path,
            selected: &selected,
            stage: None,
            loadout: None,
            extra_items: &[],
            seed_vars: &[],
            stand: start,
            before_relog: vec![command_step(
                "pre-relog fixture seed",
                "givebank egg 1".into(),
                Proof::SideTabAvailable { index: 3 },
            )],
        })
        .unwrap();
        assert_eq!(
            fixture.start_settings,
            Map::from_iter([("quests".into(), json!(["cook"]))])
        );
        assert_eq!(fixture.scenario.settings.start_script, Some("Quester"));
        assert_eq!(
            fixture.scenario.settings.terminal_shot,
            Some("quester_path")
        );
        assert!(!fixture
            .seed_commands
            .iter()
            .any(|command| command == "setvar cookquest 0"));
        let setup = fixture
            .scenario
            .steps
            .iter()
            .position(|step| step.name == "pre-relog fixture seed")
            .unwrap();
        let relog = fixture
            .scenario
            .steps
            .iter()
            .rposition(|step| matches!(step.kind, StepKind::Relog))
            .unwrap();
        assert!(setup < relog);
        assert_eq!(
            fixture.scenario.steps[relog + 1].wait.arm,
            Proof::Arrived {
                x: start.x,
                z: start.z,
                level: start.level,
            }
        );
    }

    #[test]
    fn headed_path_requires_an_id_and_has_a_bounded_deadline() {
        assert!(require_quester_path(None)
            .unwrap_err()
            .contains("QUESTER_PATH is required"));
        assert!(require_quester_path(Some("  ".into())).is_err());
        assert_eq!(
            quester_path_deadline_from(None).unwrap(),
            Duration::from_secs(45 * 60)
        );
        assert_eq!(
            quester_path_deadline_from(Some("600")).unwrap(),
            Duration::from_secs(600)
        );
        assert!(quester_path_deadline_from(Some("0")).is_err());
        assert!(quester_path_deadline_from(Some("soon")).is_err());
    }
}
