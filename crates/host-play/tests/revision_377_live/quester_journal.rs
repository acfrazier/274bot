//! Native Quester ownership checks for the 377 quest journal paths.
use super::*;
use host_play::evidence_writer::{
    EvidenceJob, EvidenceRequest, EvidenceSidecar, EvidenceWriter, PngColor,
};
use scenario::quester::{FixtureLoadout, QuesterFixture, QuesterPathFixture};
use script::native::NativePhase;
use script::{ScriptLifecycleReceipt, ScriptTerminalState};
use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::sync::Mutex;
use vault::{Profile, ProfileSettings};

#[derive(Clone, Debug, serde::Serialize)]
struct Sample {
    tick: u32,
    tile: Option<(i32, i32, i32)>,
    xp: i32,
    level: i32,
    mining_level: i32,
    cooking_level: i32,
    qp: i32,
    quest_colour: Option<i32>,
    inventory: Vec<(i32, i32)>,
    main_text: Vec<String>,
    main_root: i32,
    dialogue: Vec<String>,
    chat: Vec<(i32, String)>,
    combat_level: i32,
    combat_stats: [i32; 4],
    hitpoints: i32,
    in_combat: bool,
    local_index: Option<usize>,
    nearby_npcs: Vec<NpcSample>,
}

#[derive(Clone, Debug, serde::Serialize)]
struct NpcSample {
    index: usize,
    npc_type: Option<usize>,
    name: Option<String>,
    tile: WorldTile,
    level: i32,
    in_combat: bool,
    target: Option<api::snapshot::ActorTargetView>,
    overhead: Option<String>,
}

// Forty-three in these stats gives combat 49 (canon Player.ts:1429-1434).
// That is the first level exempt from the palace's level-24 zombie hunts
// (canon all.npc:2289; Npc.ts:1017). Keep the production danger policy at default.
const COMBAT_STATS_FIXTURE: &[&str] = &[
    "setstat attack 43",
    "setstat strength 43",
    "setstat defence 43",
    "setstat hitpoints 43",
];

#[derive(Clone, Copy)]
struct CombatFoodSeed {
    command: &'static str,
    item_id: i32,
    count: i32,
    receipt_label: &'static str,
    phase_label: &'static str,
}

const COMBAT_FOOD_SEED: CombatFoodSeed = CombatFoodSeed {
    command: "give trout 5",
    item_id: 333,
    count: 5,
    receipt_label: "labelled combat-only area fixture: Attack/Strength/Defence/Hitpoints 43, combat at least 49, five trout; default danger policy",
    phase_label: "SEED own-account combat-only area fixture",
};

// Squire uses the 377 Path's `ice` loadout for its armour, weapon and food.
// The labelled bank seeds satisfy its two-bar fast path and the steel pickaxe
// input selected by `acquire:best-pickaxe` at the seeded Mining 10 minimum.
const SQUIRE_TEST_FOOD_ID: i32 = 379;
const SQUIRE_TEST_FOOD_COUNT: i32 = 6;
const SQUIRE_MINING_LEVEL: i32 = 10;
const SQUIRE_SMITHING_LEVEL: i32 = 1;
const SQUIRE_COOKING_LEVEL: i32 = 10;
const SQUIRE_REQUIREMENT_FIXTURE: &[&str] = &["setstat mining 10", "setstat cooking 10"];
const SQUIRE_BANK_SEED_LABEL: &str = "SEED own-account bank: Squire two-bar fast-path input";
const SQUIRE_BANK_SEED_COMMAND: &str = "givebank iron_bar 2";
const SQUIRE_PICKAXE_BANK_SEED_LABEL: &str =
    "SEED own-account bank: Squire Path acquire:best-pickaxe input";
const SQUIRE_PICKAXE_BANK_SEED_COMMAND: &str = "givebank steel_pickaxe 1";
const SQUIRE_COMBAT_STATS_LABEL: &str = "SEED own-account combat stats: Attack/Strength/Defence/Hitpoints 43, combat at least 49; default danger policy";

const SQUIRE_PATH_JSON: &str = include_str!("../../../script/paths/377/squire.json");
const SQUIRE_PATH_LOADOUT: &str = "ice";

#[derive(Clone, Debug)]
struct PathItemSeed {
    command: String,
    label: String,
}

#[derive(Clone, Debug)]
struct SquirePathLoadoutSeed {
    items: Vec<PathItemSeed>,
    inventory: Vec<(i32, i32)>,
}

fn build_squire_path_fixture(
    selected: &api::game_data::SelectedGameData,
    loadout: Option<FixtureLoadout<'_>>,
    extra_items: &[(&str, i32)],
) -> Result<QuesterFixture, String> {
    let path: script::quester::path::PathDocument =
        serde_json::from_str(SQUIRE_PATH_JSON).map_err(|error| error.to_string())?;
    scenario::quester::build_quester_path_fixture(QuesterPathFixture {
        name: "revision-377-squire",
        quest_display: "The Knight's Sword",
        path: &path,
        selected,
        stage: None,
        loadout,
        extra_items,
        seed_vars: &[],
        stand: WorldTile {
            x: 3222,
            z: 3218,
            level: 0,
        },
        before_relog: Vec::new(),
    })
}

fn resolve_squire_fixture_item(
    selected: &api::game_data::SelectedGameData,
    name: &str,
) -> Result<(i32, String), String> {
    let item = selected
        .item_by_alias(name)
        .or_else(|| {
            selected.items().iter().find(|item| {
                item.name
                    .as_deref()
                    .is_some_and(|known| known.eq_ignore_ascii_case(name))
            })
        })
        .ok_or_else(|| format!("Squire fixture item {name:?} is not in selected 377 content"))?;
    let alias = item
        .alias
        .as_deref()
        .ok_or_else(|| format!("Squire fixture item {name:?} has no give alias"))?;
    Ok((item.id, alias.to_owned()))
}

fn squire_path_loadout_seed(
    selected: &api::game_data::SelectedGameData,
) -> Result<SquirePathLoadoutSeed, String> {
    let fixture = build_squire_path_fixture(
        selected,
        Some(FixtureLoadout::Path(SQUIRE_PATH_LOADOUT)),
        &[],
    )?;
    let loadout = fixture
        .seed
        .loadout
        .as_ref()
        .ok_or("Squire Path fixture did not retain its selected loadout")?;
    let mut items = Vec::with_capacity(loadout.worn.len() + loadout.carry.len());
    let mut inventory = BTreeMap::<i32, i32>::new();
    for (slot, name) in &loadout.worn {
        let (item_id, alias) = resolve_squire_fixture_item(selected, name)?;
        let command = format!("give {alias} 1");
        if !fixture.seed_commands.iter().any(|seed| seed == &command) {
            return Err(format!(
                "Quester fixture omitted Path loadout seed {command:?}"
            ));
        }
        items.push(PathItemSeed {
            command,
            label: format!(
                "SEED own-account Squire Path loadout {SQUIRE_PATH_LOADOUT}: worn {slot} {name} x1"
            ),
        });
        *inventory.entry(item_id).or_default() += 1;
    }
    for row in &loadout.carry {
        let count = i32::try_from(row.qty).map_err(|_| "Squire loadout quantity overflow")?;
        let (item_id, alias) = resolve_squire_fixture_item(selected, &row.item)?;
        let command = format!("give {alias} {count}");
        if !fixture.seed_commands.iter().any(|seed| seed == &command) {
            return Err(format!(
                "Quester fixture omitted Path loadout seed {command:?}"
            ));
        }
        items.push(PathItemSeed {
            command,
            label: format!(
                "SEED own-account Squire Path loadout {SQUIRE_PATH_LOADOUT}: carry {} x{count}",
                row.item
            ),
        });
        *inventory.entry(item_id).or_default() += count;
    }
    items.sort_by(|left, right| left.label.cmp(&right.label));
    if items.is_empty() {
        return Err("Squire Path loadout has no fixture seeds".into());
    }
    Ok(SquirePathLoadoutSeed {
        items,
        inventory: inventory.into_iter().collect(),
    })
}

fn squire_item_count(items: &[(i32, i32)], item_id: i32) -> i32 {
    items
        .iter()
        .filter(|(id, _)| *id == item_id)
        .map(|(_, count)| count)
        .sum()
}

fn merge_squire_item_counts(target: &mut BTreeMap<i32, i32>, source: BTreeMap<i32, i32>) {
    for (item_id, count) in source {
        let held = target.entry(item_id).or_default();
        *held = (*held).max(count);
    }
}

fn squire_settled_items(
    selected: &api::game_data::SelectedGameData,
    predicate: &script::quester::path::PredicateDocument,
) -> Result<BTreeMap<i32, i32>, String> {
    use script::quester::path::PredicateDocument;

    match predicate {
        PredicateDocument::All(children) => {
            let mut result = BTreeMap::new();
            for child in children {
                merge_squire_item_counts(&mut result, squire_settled_items(selected, child)?);
            }
            Ok(result)
        }
        PredicateDocument::Any(children) => {
            let Some((first, rest)) = children.split_first() else {
                return Ok(BTreeMap::new());
            };
            let mut common = squire_settled_items(selected, first)?;
            for child in rest {
                let other = squire_settled_items(selected, child)?;
                common.retain(|item_id, count| {
                    if let Some(other_count) = other.get(item_id) {
                        *count = (*count).min(*other_count);
                        true
                    } else {
                        false
                    }
                });
            }
            Ok(common)
        }
        PredicateDocument::Not(_) => Ok(BTreeMap::new()),
        PredicateDocument::Fact { kind, args, .. }
            if matches!(kind.as_str(), "has_item" | "item_count_at_least" | "worn") =>
        {
            let alias = args
                .get("obj")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| format!("Squire item settle fact has no object: {kind}"))?;
            let (item_id, _) = resolve_squire_fixture_item(selected, alias)?;
            let count = if kind == "item_count_at_least" {
                args.get("qty")
                    .and_then(serde_json::Value::as_u64)
                    .map(i32::try_from)
                    .transpose()
                    .map_err(|_| format!("Squire item settle count overflows for {alias:?}"))?
                    .unwrap_or(1)
            } else {
                1
            };
            Ok(BTreeMap::from([(item_id, count)]))
        }
        PredicateDocument::Fact { .. } => Ok(BTreeMap::new()),
    }
}

fn squire_recipe_withdraws_seeded_output(
    selected: &api::game_data::SelectedGameData,
    steps: &[script::quester::path::StepDocument],
    fixture_bank: &[(i32, i32)],
    item_id: i32,
    required: i32,
) -> Result<bool, String> {
    let mut withdrawal_quantity = 0_i32;
    for step in steps {
        if matches!(step.kind.as_str(), "kill" | "combat") {
            break;
        }
        if step.kind != "bank"
            || step.args.get("op").and_then(serde_json::Value::as_str) != Some("withdraw")
        {
            continue;
        }
        let Some(items) = step.args.get("items").and_then(serde_json::Value::as_array) else {
            continue;
        };
        for item in items {
            let Some(alias) = item.get("obj").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if resolve_squire_fixture_item(selected, alias)?.0 != item_id {
                continue;
            }
            let Some(quantity) = item.get("qty").and_then(serde_json::Value::as_u64) else {
                continue;
            };
            let quantity = i32::try_from(quantity)
                .map_err(|_| format!("Squire bank withdrawal quantity overflows for {alias:?}"))?;
            withdrawal_quantity = withdrawal_quantity.saturating_add(quantity);
        }
    }
    Ok(squire_item_count(fixture_bank, item_id) >= required && withdrawal_quantity >= required)
}

struct SquirePathAudit<'a> {
    selected: &'a api::game_data::SelectedGameData,
    header: &'a script::quester::path::QuestHeaderDocument,
    compiled: &'a script::quester::compile::CompiledPath,
    fixture_inventory: &'a [(i32, i32)],
    fixture_bank: &'a [(i32, i32)],
    visited_recipes: HashSet<String>,
    required_recipes: HashSet<String>,
    reachable_recipes: HashSet<String>,
    loadouts: HashSet<String>,
    gather_items: HashSet<i32>,
    deterministic_items: BTreeMap<i32, i32>,
}

impl<'a> SquirePathAudit<'a> {
    fn new(
        selected: &'a api::game_data::SelectedGameData,
        header: &'a script::quester::path::QuestHeaderDocument,
        compiled: &'a script::quester::compile::CompiledPath,
        fixture_inventory: &'a [(i32, i32)],
        fixture_bank: &'a [(i32, i32)],
    ) -> Self {
        Self {
            selected,
            header,
            compiled,
            fixture_inventory,
            fixture_bank,
            visited_recipes: HashSet::new(),
            required_recipes: HashSet::new(),
            reachable_recipes: HashSet::new(),
            loadouts: HashSet::new(),
            gather_items: HashSet::new(),
            deterministic_items: BTreeMap::new(),
        }
    }

    fn acquire_output_seeded(
        &self,
        recipe: &str,
        settle: &script::quester::path::PredicateDocument,
    ) -> Result<bool, String> {
        let required = squire_settled_items(self.selected, settle)?;
        if required.is_empty() {
            return Ok(false);
        }
        let authored = self
            .header
            .acquire
            .get(recipe)
            .ok_or_else(|| format!("Squire Path omits acquire recipe {recipe:?}"))?;
        for (item_id, count) in required {
            let carried = squire_item_count(self.fixture_inventory, item_id).saturating_add(
                self.deterministic_items
                    .get(&item_id)
                    .copied()
                    .unwrap_or_default(),
            );
            if carried >= count {
                continue;
            }
            if !squire_recipe_withdraws_seeded_output(
                self.selected,
                authored,
                self.fixture_bank,
                item_id,
                count,
            )? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn walk_steps(
        &mut self,
        steps: &[script::quester::path::StepDocument],
        reachable: &mut bool,
    ) -> Result<(), String> {
        for step in steps {
            if matches!(step.kind.as_str(), "kill" | "combat") {
                // Combat loot is a chance outcome. Seeded bank outputs can bypass
                // their containing acquire recipe before this point is reached.
                *reachable = false;
                continue;
            }
            if step.kind == "acquire" {
                let recipe = step
                    .args
                    .get("recipe")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| format!("Squire acquire step {} has no recipe", step.id.0))?;
                if self.acquire_output_seeded(recipe, &step.settle)? {
                    continue;
                }
                self.required_recipes.insert(recipe.to_owned());
                if self.visited_recipes.insert(recipe.to_owned()) {
                    let authored =
                        self.header.acquire.get(recipe).ok_or_else(|| {
                            format!("Squire Path omits acquire recipe {recipe:?}")
                        })?;
                    let planned = self
                        .compiled
                        .provisioning
                        .recipes
                        .get(recipe)
                        .ok_or_else(|| format!("Quester plan omits acquire recipe {recipe:?}"))?;
                    if authored.len() != planned.steps.len()
                        || authored
                            .iter()
                            .zip(planned.steps.iter())
                            .any(|(source, compiled)| source.id != compiled.id)
                    {
                        return Err(format!("Quester plan for {recipe:?} differs from the Path"));
                    }
                    let was_reachable = *reachable;
                    let mut recipe_reachable = was_reachable;
                    self.walk_steps(authored, &mut recipe_reachable)?;
                    if recipe_reachable {
                        self.reachable_recipes.insert(recipe.to_owned());
                    }
                    if was_reachable {
                        *reachable = recipe_reachable;
                    }
                }
                if *reachable {
                    let settled = squire_settled_items(self.selected, &step.settle)?;
                    merge_squire_item_counts(&mut self.deterministic_items, settled);
                }
                continue;
            }
            if step.kind == "loadout" {
                let name = step
                    .args
                    .get("loadout")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| format!("Squire loadout step {} has no loadout", step.id.0))?;
                if !self.header.loadouts.contains_key(name) {
                    return Err(format!("Squire Path omits loadout {name:?}"));
                }
                self.loadouts.insert(name.to_owned());
                continue;
            }
            if step.kind == "gather" {
                let alias = step
                    .args
                    .get("until")
                    .and_then(|until| until.get("obj"))
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        format!("Squire gather step {} has no output item", step.id.0)
                    })?;
                self.gather_items
                    .insert(resolve_squire_fixture_item(self.selected, alias)?.0);
            }
            if step.kind == "bank" {
                // Bank withdrawals are available only from the seeded bank;
                // their settle predicate alone cannot prove the item exists.
                continue;
            }

            if *reachable {
                let settled = squire_settled_items(self.selected, &step.settle)?;
                merge_squire_item_counts(&mut self.deterministic_items, settled);
            }
        }
        Ok(())
    }
}

struct SquirePostSeedAudit {
    missing: Vec<String>,
    loadouts: HashSet<String>,
    required_recipes: HashSet<String>,
    reachable_recipes: HashSet<String>,
}

fn squire_post_seed_missing_items(
    selected: &api::game_data::SelectedGameData,
    path: &script::quester::path::PathDocument,
    compiled: &script::quester::compile::CompiledPath,
    seeded_stage: &str,
    fixture_inventory: &[(i32, i32)],
    fixture_bank: &[(i32, i32)],
) -> Result<SquirePostSeedAudit, String> {
    let header = path
        .quest
        .as_ref()
        .ok_or("Squire Path has no quest header")?;
    let start_index = compiled
        .provisioning
        .stages
        .iter()
        .position(|stage| stage.0.as_ref() == seeded_stage)
        .ok_or_else(|| format!("Quester plan omits seeded stage {seeded_stage:?}"))?;
    let role = path
        .roles
        .iter()
        .find(|role| role.role == compiled.role)
        .ok_or("Squire Path role is absent from the compiled plan")?;

    let mut required_items = Vec::<(i32, i32, String)>::new();
    for item in compiled.provisioning.items.iter().filter(|item| {
        item.from_stage_index
            .is_none_or(|stage_index| stage_index >= start_index)
    }) {
        let count = i32::try_from(item.qty)
            .map_err(|_| format!("Squire item quantity overflows for {}", item.name))?;
        required_items.push((item.id, count, format!("need {} x{count}", item.name)));
    }

    let mut audit =
        SquirePathAudit::new(selected, header, compiled, fixture_inventory, fixture_bank);
    let mut path_reachable = true;
    for sequence in compiled.sequences.iter().skip(start_index) {
        let authored = role
            .sequences
            .iter()
            .find(|candidate| candidate.stage == sequence.stage)
            .ok_or_else(|| format!("Squire Path omits stage {}", sequence.stage.0))?;
        if authored.steps.len() != sequence.steps.len()
            || authored
                .steps
                .iter()
                .zip(sequence.steps.iter())
                .any(|(source, compiled)| {
                    source.id != compiled.id || source.kind.as_str() != compiled.kind.as_ref()
                })
        {
            return Err(format!(
                "Quester plan for stage {} does not match the Path steps",
                sequence.stage.0
            ));
        }
        audit.walk_steps(&authored.steps, &mut path_reachable)?;
    }

    for name in audit.loadouts.iter() {
        let authored = header
            .loadouts
            .get(name)
            .ok_or_else(|| format!("Squire Path omits loadout {name:?}"))?;
        for (slot, alias) in &authored.worn {
            let (item_id, _) = resolve_squire_fixture_item(selected, alias)?;
            required_items.push((item_id, 1, format!("loadout {name} worn {slot} {alias}")));
        }
        let qualified = format!("{}/{}", path.id.0, name);
        let carry = compiled
            .provisioning
            .loadout_carry
            .get(qualified.as_str())
            .ok_or_else(|| format!("Quester plan omits loadout carry {qualified:?}"))?;
        for item in carry.iter() {
            let count = item.qty;
            required_items.push((
                item.item.id,
                count,
                format!("loadout {name} carry {} x{count}", item.item.name),
            ));
        }
    }

    let gather_tool_ids = compiled
        .provisioning
        .gather_tool_needs
        .iter()
        .filter(|need| audit.gather_items.contains(&need.item_id))
        .flat_map(|need| need.tools.iter().map(|tool| tool.id))
        .collect::<HashSet<_>>();
    for recipe in audit.required_recipes.iter() {
        let plan = compiled
            .provisioning
            .recipes
            .get(recipe.as_str())
            .ok_or_else(|| format!("Quester plan omits acquire recipe {recipe:?}"))?;
        for item in plan
            .peak_items
            .iter()
            .filter(|item| !gather_tool_ids.contains(&item.item.id))
        {
            required_items.push((
                item.item.id,
                item.qty,
                format!("acquire input {recipe}: {} x{}", item.item.name, item.qty),
            ));
        }
    }

    let available_count = |item_id| {
        squire_item_count(fixture_inventory, item_id)
            .saturating_add(squire_item_count(fixture_bank, item_id))
            .saturating_add(
                audit
                    .deterministic_items
                    .get(&item_id)
                    .copied()
                    .unwrap_or_default(),
            )
    };
    let mut missing = Vec::new();
    for (item_id, count, description) in required_items {
        let available = available_count(item_id);
        if available < count {
            let name = selected
                .items()
                .iter()
                .find(|item| item.id == item_id)
                .and_then(|item| item.name.as_deref())
                .unwrap_or("unknown item");
            missing.push(format!(
                "{description} (item {name}, held/seeded {available})"
            ));
        }
    }
    for item_id in audit.gather_items.iter() {
        let need = compiled
            .provisioning
            .gather_tool_needs
            .iter()
            .find(|need| need.item_id == *item_id)
            .ok_or_else(|| format!("Quester plan omits gather tool need for item {item_id}"))?;
        if !need.tools.is_empty() && !need.tools.iter().any(|tool| available_count(tool.id) > 0) {
            let resource = selected
                .item_by_id(*item_id)
                .and_then(|item| item.name.as_deref())
                .unwrap_or("unknown resource");
            missing.push(format!(
                "gather tool input for {resource}: no planned tool is seeded or acquired"
            ));
        }
    }

    missing.sort();
    missing.dedup();
    Ok(SquirePostSeedAudit {
        missing,
        loadouts: audit.loadouts,
        required_recipes: audit.required_recipes,
        reachable_recipes: audit.reachable_recipes,
    })
}

#[derive(Clone, Copy)]
struct Quest {
    key: &'static str,
    label: &'static str,
    var: &'static str,
    row_label: &'static str,
    terminal: &'static str,
    skill: i32,
    xp_delta: i32,
    qp_delta: i32,
    qp_reward: &'static str,
}

// quest_sheep.rs2:5 gives 150 XP. The fresh account also spins 20 balls:
// spinning.rs2:60-64 and spinning.struct:8-12 give 2.5 XP per ball.
const SHEEP: Quest = Quest {
    key: "sheep",
    label: "Sheep Shearer",
    row_label: "Sheep Shearer",
    var: "sheep",
    terminal: "22",
    skill: 12,
    xp_delta: 200,
    qp_delta: 1,
    qp_reward: "1 Quest Point",
};
const RUNE: Quest = Quest {
    key: "runemysteries",
    label: "Rune Mysteries",
    row_label: "Rune Mysteries Quest",
    var: "runemysteries",
    terminal: "6",
    skill: 20,
    xp_delta: 0,
    qp_delta: 1,
    qp_reward: "1 Quest Point",
};
const ROMEO: Quest = Quest {
    key: "romeojuliet",
    label: "Romeo & Juliet",
    row_label: "Romeo & Juliet",
    var: "rjquest",
    terminal: "100",
    skill: 0,
    xp_delta: 0,
    qp_delta: 5,
    qp_reward: "5 Quest Points",
};
// Magic is stat index 6 (quester_imp.rs). Crafting is 12 (sheep). Attack is 0.
// Vampire's reward skill is Attack, which the combat fixture also raises, so
// its witness compares the delta against the post-fixture baseline.
const HETTY: Quest = Quest {
    key: "hetty",
    label: "Witch's Potion",
    row_label: "Witch's Potion",
    var: "hetty",
    terminal: "3",
    skill: 6,
    xp_delta: 325,
    qp_delta: 1,
    qp_reward: "1 Quest Point",
};
const GOBDIP: Quest = Quest {
    key: "gobdip",
    label: "Goblin Diplomacy",
    row_label: "Goblin Diplomacy",
    var: "goblinquest",
    terminal: "6",
    skill: 12,
    xp_delta: 200,
    qp_delta: 5,
    qp_reward: "5 Quest Points",
};
const VAMPIRE: Quest = Quest {
    key: "vampire",
    label: "Vampire Slayer",
    row_label: "Vampire Slayer",
    var: "vampire",
    terminal: "3",
    skill: 0,
    xp_delta: 4825,
    qp_delta: 3,
    qp_reward: "3 Quest Points",
};
const DEMON: Quest = Quest {
    key: "demon",
    label: "Demon Slayer",
    row_label: "Demon Slayer",
    var: "demonstart",
    terminal: "30",
    skill: 6,
    xp_delta: 0,
    qp_delta: 3,
    qp_reward: "3 Quest Points",
};
// The 377 journal row is "The Knight's Sword" (content/scripts/interfaces/questlist.if:231-244).
// The completion reward and terminal var come from content/scripts/quests/quest_squire/scripts/quest_squire.rs2:219-223.
// Smithing is stat index 13, the reward skill.
const SQUIRE: Quest = Quest {
    key: "squire",
    label: "Knight's Sword",
    row_label: "The Knight's Sword",
    var: "squire",
    terminal: "7",
    skill: 13,
    xp_delta: 12_725,
    qp_delta: 1,
    qp_reward: "1 Quest Point",
};

fn needs_combat_fixture(key: &str) -> bool {
    matches!(
        key,
        "runemysteries" | "hetty" | "gobdip" | "vampire" | "demon" | "squire"
    )
}

fn split_completion(key: &str) -> Option<&'static str> {
    match key {
        "squire" => Some("Knight's Sword Quest!"),
        "romeojuliet" => Some("Romeo & Juliet Quest!"),
        "hetty" => Some("Witches Potion Quest!"),
        "gobdip" => Some("Goblin Diplomacy Quest!"),
        "vampire" => Some("Vampire Slayer Quest!"),
        "demon" => Some("Demon Slayer Quest!"),
        _ => None,
    }
}

impl Sample {
    fn read(client: &Client, snapshot: &GameSnapshot, quest: Quest) -> Self {
        let stat = snapshot.stats().iter().find(|row| row.index == quest.skill);
        Self {
            tick: snapshot.tick(),
            tile: snapshot.tile(),
            xp: stat.map_or(0, |row| row.xp),
            level: stat.map_or(1, |row| row.base),
            mining_level: snapshot
                .stats()
                .iter()
                .find(|row| row.name == "mining")
                .map_or(1, |row| row.base),
            cooking_level: snapshot
                .stats()
                .iter()
                .find(|row| row.name == "cooking")
                .map_or(1, |row| row.base),
            qp: client.var.get(101).copied().unwrap_or(-1),
            quest_colour: snapshot
                .quest_statuses()
                .iter()
                .find(|row| row.name == quest.row_label)
                .map(|row| row.colour),
            inventory: snapshot
                .inventory()
                .iter()
                .map(|row| (row.def.id, row.count))
                .collect(),
            main_root: snapshot.modals().main,
            main_text: snapshot.main_modal_texts().to_vec(),
            dialogue: snapshot.chat_modal_texts().to_vec(),
            chat: snapshot
                .chat_lines()
                .iter()
                .map(|row| (row.sequence, row.text.clone()))
                .collect(),
            combat_level: snapshot.local_player().map_or(0, |p| p.player.combat_level),
            combat_stats: ["attack", "strength", "defence", "hitpoints"].map(|name| {
                snapshot
                    .stats()
                    .iter()
                    .find(|s| s.name == name)
                    .map_or(0, |s| s.base)
            }),
            hitpoints: snapshot
                .stats()
                .iter()
                .find(|s| s.name == "hitpoints")
                .map_or(0, |s| s.effective),
            in_combat: snapshot
                .local_player()
                .is_some_and(|p| p.player.actor.in_combat),
            local_index: snapshot.local_player().map(|p| p.player.index),
            nearby_npcs: snapshot
                .npcs()
                .iter()
                .filter(|n| n.distance <= 18)
                .map(|n| NpcSample {
                    index: n.index,
                    npc_type: n.r#type,
                    name: n.name.clone(),
                    tile: n.tile,
                    level: n.level,
                    in_combat: n.in_combat,
                    target: n.target,
                    overhead: n.overhead_text.clone(),
                })
                .collect(),
        }
    }
    fn fresh(&self) -> bool {
        self.xp == 0 && self.qp == 0 && self.quest_colour == Some(0xF80000)
    }
    fn safe_to_relog(&self) -> bool {
        !self.in_combat
            && !self.nearby_npcs.iter().any(|npc| {
                npc.in_combat
                    && npc.target.is_some_and(|target| {
                        target.kind == api::snapshot::ActorKind::Player
                            && Some(target.index) == self.local_index
                    })
            })
    }
    fn combat_can_relog(&self) -> bool {
        self.combat_level >= 49
            && self.combat_stats.iter().all(|level| *level == 43)
            && self.hitpoints == 43
            && self.safe_to_relog()
    }
    fn combat_fixture_ready(
        &self,
        quest: Quest,
        squire_loadout: Option<&SquirePathLoadoutSeed>,
    ) -> bool {
        if !self.combat_can_relog() {
            return false;
        }
        if quest.key == "squire" {
            let default_food = [(SQUIRE_TEST_FOOD_ID, SQUIRE_TEST_FOOD_COUNT)];
            let required =
                squire_loadout.map_or(&default_food[..], |seed| seed.inventory.as_slice());
            required.iter().all(|(id, count)| {
                self.inventory
                    .iter()
                    .filter(|(inventory_id, _)| inventory_id == id)
                    .map(|(_, held)| held)
                    .sum::<i32>()
                    >= *count
            })
        } else {
            self.inventory
                .iter()
                .filter(|(id, _)| *id == COMBAT_FOOD_SEED.item_id)
                .map(|(_, count)| count)
                .sum::<i32>()
                == COMBAT_FOOD_SEED.count
        }
    }
    fn quest_fixture_ready_with_loadout(
        &self,
        quest: Quest,
        squire_loadout: Option<&SquirePathLoadoutSeed>,
    ) -> bool {
        match quest.key {
            "squire" => {
                self.combat_fixture_ready(quest, squire_loadout)
                    && self.mining_level == SQUIRE_MINING_LEVEL
                    && self.level == SQUIRE_SMITHING_LEVEL
                    && self.cooking_level == SQUIRE_COOKING_LEVEL
            }
            key if needs_combat_fixture(key) => self.combat_fixture_ready(quest, None),
            _ => true,
        }
    }
    fn quest_pre_relog_ready(&self, quest: Quest) -> bool {
        if quest.key == "squire" {
            // setstat skill rows are verified after the clean relog, when the
            // server publishes them. Before it, wait only for the visible combat fixture.
            self.combat_level >= 49 && self.safe_to_relog()
        } else {
            self.combat_can_relog()
        }
    }
    fn complete(&self, q: Quest, base_xp: i32) -> bool {
        let completion = if let Some(second) = split_completion(q.key) {
            self.main_text
                .iter()
                .any(|s| s.contains("You have completed the"))
                && self.main_text.iter().any(|s| s.contains(second))
        } else {
            self.main_text
                .iter()
                .any(|s| s.contains(&format!("You have completed the {} Quest!", q.label)))
        };
        let reward = match q.key {
            "sheep" => {
                self.main_text.iter().any(|s| s.contains("150 Crafting XP"))
                    && self.main_text.iter().any(|s| s == q.qp_reward)
            }
            "runemysteries" => {
                self.main_text.iter().any(|s| s == "Runecrafting skill")
                    && self.main_text.iter().any(|s| s == "Air talisman")
                    && self.main_text.iter().any(|s| s == q.qp_reward)
            }
            "hetty" => {
                self.main_text.iter().any(|s| s == "325 Magic XP")
                    && self.main_text.iter().any(|s| s == q.qp_reward)
            }
            "gobdip" => {
                self.main_text.iter().any(|s| s == "200 Crafting XP")
                    && self.main_text.iter().any(|s| s == "A Gold Bar")
                    && self.main_text.iter().any(|s| s == q.qp_reward)
            }
            "vampire" => {
                self.main_text.iter().any(|s| s == "4,825 Attack XP")
                    && self.main_text.iter().any(|s| s == q.qp_reward)
            }
            "demon" => {
                self.main_text.iter().any(|s| s == "Silverlight")
                    && self.main_text.iter().any(|s| s == q.qp_reward)
            }
            "squire" => {
                self.main_text.iter().any(|s| s == "12,725 Smithing XP")
                    && self.main_text.iter().any(|s| s == q.qp_reward)
            }
            "romeojuliet" => self.main_text.iter().any(|s| s == q.qp_reward),
            _ => false,
        };
        // Draynor adds variable Attack XP before Vampire's reward. Squire's
        // dwarf drops add no Smithing XP, so its reward delta remains exact.
        let xp_delta = self.xp.checked_sub(base_xp);
        let xp_ok = if q.key == "vampire" {
            xp_delta.is_some_and(|gained| gained >= q.xp_delta)
        } else {
            xp_delta == Some(q.xp_delta)
        };
        xp_ok
            && self.qp == q.qp_delta
            && self.quest_colour == Some(0x00F800)
            && completion
            && reward
    }
}

fn native_run_completed(receipt: Option<&ScriptLifecycleReceipt>, generation: Option<u64>) -> bool {
    receipt.is_some_and(|r| {
        Some(r.runtime_generation) == generation && r.state == ScriptTerminalState::Completed
    })
}

fn witness_predicate(sample: &Sample, quest: Quest, base_xp: i32, baseline: bool) -> bool {
    witness_predicate_with_loadout(sample, quest, base_xp, baseline, None)
}

fn witness_predicate_with_loadout(
    sample: &Sample,
    quest: Quest,
    base_xp: i32,
    baseline: bool,
    squire_loadout: Option<&SquirePathLoadoutSeed>,
) -> bool {
    if baseline {
        if quest.key == "vampire" {
            // The labelled combat fixture raises Attack, the reward skill.
            sample.qp == 0
                && sample.quest_colour == Some(0xF80000)
                && sample.combat_fixture_ready(quest, None)
        } else {
            sample.fresh()
                && base_xp == sample.xp
                && sample.quest_fixture_ready_with_loadout(quest, squire_loadout)
        }
    } else {
        sample.complete(quest, base_xp)
    }
}

#[test]
fn journal_witness_rejects_stale_or_partial_product_evidence() {
    let fresh = Sample {
        tick: 1,
        tile: Some((3200, 3200, 0)),
        xp: 0,
        level: 1,
        mining_level: 1,
        cooking_level: 1,
        qp: 0,
        quest_colour: Some(0xF80000),
        inventory: vec![],
        main_root: 0,
        main_text: vec![],
        dialogue: vec![],
        chat: vec![],
        combat_level: 3,
        combat_stats: [1, 1, 1, 10],
        hitpoints: 10,
        in_combat: false,
        local_index: Some(1),
        nearby_npcs: vec![],
    };
    assert!(witness_predicate(&fresh, SHEEP, 0, true));
    assert!(
        !witness_predicate(&fresh, RUNE, 0, true),
        "Rune Mysteries must reject the original unprepared combat fixture"
    );
    let mut prepared = fresh.clone();
    prepared.combat_level = 49;
    prepared.combat_stats = [43; 4];
    prepared.hitpoints = 43;
    prepared.inventory = vec![(333, 5)];
    assert!(witness_predicate(&prepared, RUNE, 0, true));
    let mut old_area_fixture = prepared.clone();
    old_area_fixture.combat_level = 46;
    old_area_fixture.combat_stats = [40; 4];
    old_area_fixture.hitpoints = 40;
    assert!(
        !old_area_fixture.combat_can_relog(),
        "combat 46 does not clear the default level-24 sewer-zombie policy"
    );
    for invalid in 0..4 {
        let mut sample = prepared.clone();
        match invalid {
            0 => sample.combat_level = 48,
            1 => sample.inventory.clear(),
            2 => sample.hitpoints = 10,
            _ => sample.in_combat = true,
        }
        assert!(!witness_predicate(&sample, RUNE, 0, true));
        if invalid == 1 {
            assert!(
                sample.combat_can_relog(),
                "mainland inventory is published only after the clean relog"
            );
        }
    }
    for quest in [SHEEP, RUNE, ROMEO] {
        let mut completed = Sample {
            tick: 2,
            tile: fresh.tile,
            xp: quest.xp_delta,
            level: 1,
            mining_level: 1,
            cooking_level: 1,
            qp: quest.qp_delta,
            quest_colour: Some(0x00F800),
            inventory: vec![],
            main_root: 0,
            main_text: vec![],
            dialogue: vec![],
            chat: vec![],
            combat_level: 3,
            combat_stats: [1, 1, 1, 10],
            hitpoints: 10,
            in_combat: false,
            local_index: Some(1),
            nearby_npcs: vec![],
        };
        completed.main_text = match quest.key {
            "sheep" => vec![
                "You have completed the Sheep Shearer Quest!".into(),
                "150 Crafting XP".into(),
                "1 Quest Point".into(),
            ],
            "runemysteries" => vec![
                "You have completed the Rune Mysteries Quest!".into(),
                "Runecrafting skill".into(),
                "Air talisman".into(),
                "1 Quest Point".into(),
            ],
            _ => vec![
                "You have completed the".into(),
                "Romeo & Juliet Quest!".into(),
                "5 Quest Points".into(),
            ],
        };
        assert!(
            witness_predicate(&completed, quest, 0, false),
            "{} terminal witness",
            quest.key
        );
        completed.quest_colour = Some(0xF80000);
        assert!(
            !witness_predicate(&completed, quest, 0, false),
            "{} pending quest row",
            quest.key
        );
        completed.quest_colour = Some(0x00F800);
        assert!(
            !witness_predicate(&completed, quest, quest.xp_delta + 1, false),
            "{} incorrect XP delta",
            quest.key
        );
    }
    assert!(!native_run_completed(None, Some(1)));
    for state in [
        ScriptTerminalState::Stopped,
        ScriptTerminalState::Failed,
        ScriptTerminalState::Cancelled,
    ] {
        let receipt = ScriptLifecycleReceipt {
            runtime_generation: 4,
            state,
            tick: 1,
            reason: "fixture".into(),
        };
        assert!(!native_run_completed(Some(&receipt), Some(4)));
    }
    let completed = ScriptLifecycleReceipt {
        runtime_generation: 4,
        state: ScriptTerminalState::Completed,
        tick: 2,
        reason: "complete".into(),
    };
    assert!(native_run_completed(Some(&completed), Some(4)));
    assert!(!native_run_completed(Some(&completed), Some(5)));
}

struct Witness {
    snapshot: GameSnapshot,
    pump: Pump,
    log: fs::File,
    account: String,
    directory: PathBuf,
    writer: Arc<EvidenceWriter>,
    jobs: Vec<EvidenceJob>,
    quest: Quest,
    squire_loadout: Option<SquirePathLoadoutSeed>,
    relog_requested: bool,
    fixture_seeded: bool,
    fixture_item_cursor: usize,
    relog_detached: bool,
    relogged: bool,
    baseline: Option<Sample>,
    latest: Option<Sample>,
    base_xp: Option<i32>,
    reads: usize,
    pending: Option<i32>,
    read_tick: Option<u32>,
    last_tick: Option<u32>,
    product_captured: bool,
    native_complete: bool,
    error: Option<String>,
}
impl Witness {
    fn record(&mut self, value: Value) -> Result<(), String> {
        writeln!(self.log, "{value}").map_err(|e| e.to_string())?;
        self.log.flush().map_err(|e| e.to_string())
    }
    fn capture(&mut self, client: &mut Client, phase: &str, sample: &Sample) -> Result<(), String> {
        let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
        let draw = client.draw;
        client.set_draw(true);
        let out = renderer.mainredraw(client);
        client.set_draw(draw);
        let client::render::backend::FrameOutput::PixMap(pixels) = out else {
            return Err("377 Quester capture did not return CPU framebuffer".into());
        };
        let mut seeds =
            vec!["own-account tutorial bypass and mainland position, then clean relog".to_owned()];
        if self.quest.key == "squire" {
            seeds.push(SQUIRE_COMBAT_STATS_LABEL.to_owned());
            let loadout = self
                .squire_loadout
                .as_ref()
                .ok_or("Squire receipt has no Path loadout fixture")?;
            seeds.extend(loadout.items.iter().map(|seed| seed.label.clone()));
            seeds.push("labelled own-account quest minima: Mining 10 (content/scripts/quests/quest_squire/scripts/squire_journal.rs2:7-12) and Cooking 10 (content/scripts/skill_cooking/configs/cooking_inv/configs/pies/pies.struct:13-17); Smithing remains fresh at level 1 / 0 XP".to_owned());
            seeds.push(SQUIRE_BANK_SEED_LABEL.to_owned());
            seeds.push(SQUIRE_PICKAXE_BANK_SEED_LABEL.to_owned());
        } else if needs_combat_fixture(self.quest.key) {
            seeds.push(COMBAT_FOOD_SEED.receipt_label.to_owned());
        }
        let bank_seeds = if self.quest.key == "squire" {
            json!([
                {"label":SQUIRE_BANK_SEED_LABEL,"item_alias":"iron_bar","count":2,"command":SQUIRE_BANK_SEED_COMMAND,"scope":"own-account bank"},
                {"label":SQUIRE_PICKAXE_BANK_SEED_LABEL,"item_alias":"steel_pickaxe","count":1,"command":SQUIRE_PICKAXE_BANK_SEED_COMMAND,"scope":"own-account bank"}
            ])
        } else {
            json!([])
        };
        let quest_item_seeds = if self.quest.key == "squire" {
            self.squire_loadout
                .as_ref()
                .map_or(0, |seed| seed.inventory.len() + 2)
        } else {
            0
        };
        let receipt = json!({"account":self.account,"phase":phase,"revision":377,"quest":self.quest.key,"sample":sample,"baseline":self.baseline,"driver":"production Play.script_start(Quester), bundled 377 quest path","renderer":"real Client CpuPix3D framebuffer","seeds":seeds,"bank_seeds":bank_seeds,"quest_state_seeds":0,"quest_item_seeds":quest_item_seeds,"reward_xp_seeds":0,"quest_point_seeds":0});
        self.record(receipt.clone())?;
        self.jobs.push(self.writer.submit(EvidenceRequest {
            png_path: self.directory.join(format!("{phase}.png")),
            width: pixels.width as u32,
            height: pixels.height as u32,
            pixels: pixels.pixels,
            color: PngColor::Rgba,
            sidecar: Some(EvidenceSidecar {
                path: self.directory.join(format!("{phase}.json")),
                receipt,
                patch_error: None,
            }),
        }));
        Ok(())
    }
    fn frame(&mut self, client: &mut Client, hold: bool) -> Result<(), String> {
        host::publish_snapshot(&mut self.snapshot, client, self.pump.drain_client(client));
        if self.relog_requested && !self.snapshot.ingame() {
            self.relog_detached = true
        }
        if hold
            || !self.snapshot.ingame()
            || !self.snapshot.attached()
            || self.snapshot.scene_state() != 2
        {
            return Ok(());
        }
        let s = Sample::read(client, &self.snapshot, self.quest);
        self.latest = Some(s.clone());
        if self.last_tick != Some(s.tick) {
            self.record(json!({"phase":"observation","sample":s}))?;
            self.last_tick = Some(s.tick);
        }
        if !self.relogged {
            if self.relog_detached
                && self
                    .snapshot
                    .side_tabs()
                    .iter()
                    .any(|t| t.index == 3 && t.available)
            {
                self.relogged = true;
                self.record(json!({"phase":"fixture-relogged","sample":s}))?;
            } else {
                if !self.relog_requested
                    && s.tick >= 3
                    && s.tile
                        .is_some_and(|(x, z, l)| lumbridge(WorldTile { x, z, level: l }))
                {
                    if needs_combat_fixture(self.quest.key) {
                        if !self.fixture_seeded {
                            let requirement_fixture = if self.quest.key == "squire" {
                                SQUIRE_REQUIREMENT_FIXTURE
                            } else {
                                &[]
                            };
                            let bank_seed: &[&str] = if self.quest.key == "squire" {
                                &[SQUIRE_BANK_SEED_COMMAND, SQUIRE_PICKAXE_BANK_SEED_COMMAND]
                            } else {
                                &[]
                            };
                            let mut commands = COMBAT_STATS_FIXTURE.to_vec();
                            if self.quest.key != "squire" {
                                commands.push(COMBAT_FOOD_SEED.command);
                            }
                            commands.extend_from_slice(requirement_fixture);
                            commands.extend_from_slice(bank_seed);
                            for command in commands {
                                if !interact::cheat(client, command).is_sent() {
                                    return Err(format!(
                                        "combat fixture command refused: {command}"
                                    ));
                                }
                                let phase = if command == COMBAT_FOOD_SEED.command
                                    && self.quest.key != "squire"
                                {
                                    COMBAT_FOOD_SEED.phase_label
                                } else {
                                    match command {
                                        "setstat mining 10" => {
                                            "SEED quest Mining 10 minimum (content/scripts/quests/quest_squire/scripts/squire_journal.rs2:7-12)"
                                        }
                                        "setstat cooking 10" => {
                                            "SEED redberry-pie Cooking 10 minimum (content/scripts/skill_cooking/configs/cooking_inv/configs/pies/pies.struct:13-17; content/scripts/skill_cooking/configs/cooking_source/cooking_generic.dbrow:201-212)"
                                        }
                                        SQUIRE_BANK_SEED_COMMAND => SQUIRE_BANK_SEED_LABEL,
                                        SQUIRE_PICKAXE_BANK_SEED_COMMAND => {
                                            SQUIRE_PICKAXE_BANK_SEED_LABEL
                                        }
                                        _ if self.quest.key == "squire" => SQUIRE_COMBAT_STATS_LABEL,
                                        _ => "SEED own-account combat-only area fixture",
                                    }
                                };
                                if matches!(
                                    command,
                                    SQUIRE_BANK_SEED_COMMAND | SQUIRE_PICKAXE_BANK_SEED_COMMAND
                                ) {
                                    self.record(json!({"phase":phase,"command":command,"scope":"own-account bank"}))?;
                                } else {
                                    self.record(json!({"phase":phase,"command":command}))?;
                                }
                            }
                            self.fixture_seeded = true;
                            return Ok(());
                        }
                        if self.quest.key == "squire" {
                            let seed = self
                                .squire_loadout
                                .as_ref()
                                .ok_or("Squire run has no Path loadout fixture")?
                                .items
                                .get(self.fixture_item_cursor)
                                .cloned();
                            if let Some(seed) = seed {
                                if !interact::cheat(client, &seed.command).is_sent() {
                                    return Err(format!(
                                        "Squire Path loadout seed refused: {}",
                                        seed.command
                                    ));
                                }
                                self.record(json!({"phase":seed.label,"command":seed.command,"scope":"own-account inventory"}))?;
                                self.fixture_item_cursor += 1;
                                return Ok(());
                            }
                        }
                        // Tutorial login has no inventory side tab yet. Squire's
                        // Path loadout and skill minima are verified after clean relog.
                        if !s.quest_pre_relog_ready(self.quest) {
                            return Ok(());
                        }
                        self.record(json!({"phase":"combat fixture visible; skill minima will be verified after clean relog","sample":s}))?;
                    }
                    let ifaces = Arc::clone(&client.ifaces);
                    if !interact::logout(client, &ifaces) {
                        return Err("could not cleanly logout mainland fixture".into());
                    }
                    self.relog_requested = true;
                    self.record(json!({"phase":"SEED own-account tutorial/mainland preparation; clean relog","sample":s}))?;
                }
                return Ok(());
            }
        }
        if self.baseline.is_none() {
            // The first post-relog frame can publish combat level and food
            // before named stat rows. Do not fail that partial frame.
            if !s.quest_fixture_ready_with_loadout(self.quest, self.squire_loadout.as_ref()) {
                return Ok(());
            }
            if !witness_predicate_with_loadout(
                &s,
                self.quest,
                s.xp,
                true,
                self.squire_loadout.as_ref(),
            ) {
                return Err(format!("fresh-account quest conditions failed: {s:?}"));
            }
            self.base_xp = Some(s.xp);
            self.capture(client, "fresh", &s)?;
            self.baseline = Some(s.clone());
            return Ok(());
        }
        if s.complete(self.quest, self.base_xp.unwrap_or(0)) && !self.product_captured {
            self.capture(client, "product-completion", &s)?;
            self.product_captured = true;
        }
        let expected = [
            (
                format!("getvar {}", self.quest.var),
                format!("get {}: 0", self.quest.var),
            ),
            ("getvar qp".into(), "get qp: 0".into()),
            (
                format!("getvar {}", self.quest.var),
                format!("get {}: {}", self.quest.var, self.quest.terminal),
            ),
            (
                "getvar qp".into(),
                format!("get qp: {}", self.quest.qp_delta),
            ),
        ];
        if self.reads < 4 && (self.reads < 2 || (self.product_captured && self.native_complete)) {
            if let Some(seq) = self.pending {
                if s.chat
                    .iter()
                    .any(|(n, text)| *n > seq && text == &expected[self.reads].1)
                {
                    self.record(json!({"phase":"SERVER READBACK observed","command":expected[self.reads].0,"reply":expected[self.reads].1,"sample":s}))?;
                    self.reads += 1;
                    self.pending = None;
                }
            } else if self.read_tick != Some(s.tick) {
                let before = s.chat.iter().map(|(n, _)| *n).max().unwrap_or(-1);
                if !interact::cheat(client, &expected[self.reads].0).is_sent() {
                    return Err("own-account getvar read refused".into());
                }
                self.record(json!({"phase":"SERVER READBACK request, own-account read-only","command":expected[self.reads].0,"after_sequence":before}))?;
                self.pending = Some(before);
                self.read_tick = Some(s.tick);
            }
        }
        Ok(())
    }
}

fn run_quest(q: Quest) -> Result<(), String> {
    let bound = load_bound_377()?;
    resource_record(&bound);
    let squire_loadout = if q.key == "squire" {
        let selected = bound
            .profile
            .game_data()
            .ok_or("377 selected game data unavailable for Squire Path fixture")?;
        Some(squire_path_loadout_seed(&selected)?)
    } else {
        None
    };
    let account = host_play::mint_live_names(1).remove(0);
    let directory = required_path("QUESTER_377_EVIDENCE_DIR")?.join(&account);
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let log = fs::File::create(directory.join("observations.jsonl")).map_err(|e| e.to_string())?;
    let writer = Arc::new(EvidenceWriter::new(4));
    let state = Arc::new(Mutex::new(Witness {
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        log,
        account: account.clone(),
        directory,
        writer: writer.clone(),
        jobs: Vec::new(),
        quest: q,
        squire_loadout,
        relog_requested: false,
        relog_detached: false,
        relogged: false,
        fixture_seeded: false,
        fixture_item_cursor: 0,
        baseline: None,
        latest: None,
        base_xp: None,
        reads: 0,
        pending: None,
        read_tick: None,
        last_tick: None,
        product_captured: false,
        native_complete: false,
        error: None,
    }));
    let frame = state.clone();
    let name = account.clone();
    let mut play = host_play::run_with_template(
        Arc::clone(&bound.template),
        true,
        vec![],
        |_| (None, None),
        move |c, u, f| {
            if u == name {
                let mut s = frame.lock().unwrap();
                if s.error.is_none() {
                    if let Err(e) = s.frame(c, f.hold) {
                        s.error = Some(e)
                    }
                }
            }
        },
    )?;
    play.try_spawn_slot(
        Profile {
            username: account.clone(),
            password: host_play::profile_password(&account).into(),
            uid: fresh_uid(300),
            settings: ProfileSettings::default(),
        },
        None,
        None,
        None,
    )?;
    let budget = if matches!(q.key, "hetty" | "gobdip" | "vampire" | "demon" | "squire") {
        3600
    } else {
        1500
    };
    let deadline = Instant::now() + Duration::from_secs(budget);
    let prep = Instant::now() + Duration::from_secs(180);
    let mut started = false;
    let mut relogin_requested = false;
    let mut complete = false;
    let mut generation = None;
    let mut lifecycle = None;
    let mut last_status = String::new();
    let result = (|| loop {
        if Path::new("/Volumes/dev-scratch/274bot-evidence/MAC-QUIET.lock").exists() {
            return Err("MAC-QUIET.lock appeared".into());
        }
        if let Some(error) = play.script_last_error(&account) {
            return Err(format!("native Quester lifecycle: {error}"));
        }
        let status = play.script_native_status(&account);
        let receipt = play.script_lifecycle_receipt(&account);
        let mut s = state.lock().map_err(|_| "witness poisoned")?;
        if let Some(error) = &s.error {
            return Err(error.clone());
        }
        if !relogin_requested && s.relog_detached {
            drop(s);
            if let Some(arm) = play.arm(&account) {
                arm.arm_explicit_login();
                relogin_requested = true;
            }
            continue;
        }
        // Initial readbacks must precede Start, not race the first quest action.
        if !started && s.baseline.is_some() && s.reads == 2 {
            s.record(json!({"phase":"native Quester start", "quest":q.key}))?;
            drop(s);
            play.script_start(
                &account,
                script::CompiledId("Quester"),
                serde_json::Map::from_iter([("quests".into(), json!([q.key]))]),
            )?;
            started = true;
            continue;
        }
        if let Some(status) = status {
            let detail = format!("{status:?}");
            if detail != last_status {
                let event = json!({"phase":"native status", "status":detail, "latest":s.latest});
                s.record(event)?;
                last_status = detail;
            }
            if status.phase == NativePhase::Blocked || status.failure.is_some() {
                return Err(format!("native Quester failed: {status:?}"));
            }
            generation.get_or_insert(status.run.run);
        }
        if !complete && native_run_completed(receipt.as_ref(), generation) {
            s.record(json!({"phase":"native lifecycle", "receipt":receipt}))?;
            lifecycle = receipt;
            complete = true;
            s.native_complete = true;
        }
        if complete && s.product_captured && s.reads == 4 {
            let event = json!({"outcome":"PASS", "account":account, "native_lifecycle":lifecycle, "baseline":s.baseline, "latest":s.latest});
            s.record(event)?;
            return Ok(());
        }
        if Instant::now() >= deadline || (!started && Instant::now() >= prep) {
            return Err(format!(
                "quest witness deadline started={started} complete={complete} reads={} latest={:?}",
                s.reads, s.latest
            ));
        }
        drop(s);
        std::thread::sleep(Duration::from_millis(20));
    })();
    play.script_stop(&account);
    play.stop_slot(&account);
    drop(play);
    let mut s = state.lock().map_err(|_| "witness poisoned")?;
    let event = json!({"phase":"final","outcome":if result.is_ok(){"PASS"}else{"FAIL"},"error":result.as_ref().err(),"quest":q.key,"latest":s.latest});
    s.record(event)?;
    let jobs = s.jobs.clone();
    drop(s);
    for job in jobs {
        let outcome = writer
            .wait(&job, Duration::from_secs(30))
            .ok_or("screenshot write timed out")?;
        if let Some(e) = outcome.error {
            return Err(e);
        }
    }
    result
}

#[test]
#[ignore = "requires LIVE=1, owned qj3 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_sheep_native() -> Result<(), String> {
    run_quest(SHEEP)
}
#[test]
#[ignore = "requires LIVE=1, owned qm3 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_runemysteries_native() -> Result<(), String> {
    run_quest(RUNE)
}
#[test]
#[ignore = "requires LIVE=1, owned qj3 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_romeojuliet_native() -> Result<(), String> {
    run_quest(ROMEO)
}
fn batch2_product(quest: Quest, extra: &[&str]) -> Sample {
    let mut sample = Sample {
        tick: 2,
        tile: Some((3200, 3200, 0)),
        xp: quest.xp_delta,
        level: 1,
        mining_level: 1,
        cooking_level: 1,
        qp: quest.qp_delta,
        quest_colour: Some(0x00F800),
        inventory: vec![],
        main_root: 0,
        main_text: vec!["You have completed the".into()],
        dialogue: vec![],
        chat: vec![],
        combat_level: 49,
        combat_stats: [43; 4],
        hitpoints: 43,
        in_combat: false,
        local_index: Some(1),
        nearby_npcs: vec![],
    };
    if let Some(second) = split_completion(quest.key) {
        sample.main_text.push(second.into());
    }
    sample.main_text.push(quest.qp_reward.into());
    sample
        .main_text
        .extend(extra.iter().map(|line| (*line).into()));
    sample
}

#[test]
fn batch2_witness_accepts_the_377_scroll_and_rejects_a_red_row() {
    for (quest, extra) in [
        (HETTY, &["325 Magic XP"][..]),
        (GOBDIP, &["200 Crafting XP", "A Gold Bar"][..]),
        (VAMPIRE, &["4,825 Attack XP"][..]),
        (DEMON, &["Silverlight"][..]),
        (SQUIRE, &["12,725 Smithing XP"][..]),
    ] {
        let base = if quest.key == "vampire" { 1154 } else { 0 };
        let mut sample = batch2_product(quest, extra);
        sample.xp = base + quest.xp_delta;
        if quest.key == "vampire" {
            sample.xp += 40;
        }
        assert!(
            witness_predicate(&sample, quest, base, false),
            "{} product",
            quest.key
        );
        sample.quest_colour = Some(0xF80000);
        assert!(
            !witness_predicate(&sample, quest, base, false),
            "{}",
            quest.key
        );
    }
}

#[test]
fn squire_witness_rejects_invalid_baseline_and_partial_product_evidence() {
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R377)
        .expect("selected 377 data");
    let loadout = squire_path_loadout_seed(&selected).expect("Squire Path ice loadout");
    let lobster_id = selected.item_by_alias("lobster").expect("377 lobster").id;
    let rune_full_helm_id = selected
        .item_by_alias("rune_full_helm")
        .expect("377 Rune full helm")
        .id;
    let baseline = Sample {
        tick: 1,
        tile: Some((3000, 3150, 0)),
        xp: 0,
        level: SQUIRE_SMITHING_LEVEL,
        mining_level: SQUIRE_MINING_LEVEL,
        cooking_level: SQUIRE_COOKING_LEVEL,
        qp: 0,
        quest_colour: Some(0xF80000),
        inventory: loadout.inventory.clone(),
        main_root: 0,
        main_text: vec![],
        dialogue: vec![],
        chat: vec![],
        combat_level: 49,
        combat_stats: [43; 4],
        hitpoints: 43,
        in_combat: false,
        local_index: Some(1),
        nearby_npcs: vec![],
    };
    let ready = |sample: &Sample| {
        witness_predicate_with_loadout(sample, SQUIRE, baseline.xp, true, Some(&loadout))
    };
    assert!(ready(&baseline));
    let mut five_trout = baseline.clone();
    five_trout.inventory = vec![(333, 5)];
    assert!(
        !ready(&five_trout),
        "Squire requires its Path ice loadout, not the other combat fixture's trout"
    );
    let mut short_lobster_seed = baseline.clone();
    *short_lobster_seed
        .inventory
        .iter_mut()
        .find(|(id, _)| *id == lobster_id)
        .expect("Path loadout includes lobsters") = (lobster_id, 5);
    assert!(
        !ready(&short_lobster_seed),
        "Squire's blurite step requires six lobsters"
    );
    let mut missing_helm = baseline.clone();
    missing_helm
        .inventory
        .retain(|(item_id, _)| *item_id != rune_full_helm_id);
    assert!(
        !ready(&missing_helm),
        "Squire's Path ice loadout requires its Rune full helm"
    );
    let mut pre_relog = baseline.clone();
    pre_relog.level = 1;
    pre_relog.mining_level = 1;
    pre_relog.cooking_level = 1;
    assert!(pre_relog.quest_pre_relog_ready(SQUIRE));
    pre_relog.in_combat = true;
    assert!(!pre_relog.quest_pre_relog_ready(SQUIRE));

    let mut invalid_baseline = baseline.clone();
    invalid_baseline.mining_level = SQUIRE_MINING_LEVEL - 1;
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));
    invalid_baseline.mining_level = SQUIRE_MINING_LEVEL + 1;
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));
    invalid_baseline.mining_level = SQUIRE_MINING_LEVEL;
    invalid_baseline.level = SQUIRE_SMITHING_LEVEL - 1;
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));
    invalid_baseline.level = SQUIRE_SMITHING_LEVEL + 1;
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));
    invalid_baseline.level = SQUIRE_SMITHING_LEVEL;
    invalid_baseline.cooking_level = SQUIRE_COOKING_LEVEL - 1;
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));
    invalid_baseline.cooking_level = SQUIRE_COOKING_LEVEL + 1;
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));
    invalid_baseline.cooking_level = SQUIRE_COOKING_LEVEL;
    invalid_baseline.xp = 1;
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));
    invalid_baseline.xp = 0;
    assert!(!witness_predicate(&baseline, SQUIRE, baseline.xp + 1, true));
    invalid_baseline.qp = 1;
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));
    invalid_baseline.qp = 0;
    invalid_baseline.quest_colour = Some(0x00F800);
    assert!(!witness_predicate(
        &invalid_baseline,
        SQUIRE,
        baseline.xp,
        true
    ));

    let mut product = batch2_product(SQUIRE, &["12,725 Smithing XP"]);
    product.xp = baseline.xp + SQUIRE.xp_delta;
    assert!(witness_predicate(&product, SQUIRE, baseline.xp, false));
    for unexpected_xp in [-1, 1, 12, 13, 25, 26] {
        let mut invalid_product = product.clone();
        invalid_product.xp += unexpected_xp;
        assert!(
            !witness_predicate(&invalid_product, SQUIRE, baseline.xp, false),
            "unexpected Smithing XP delta: {unexpected_xp}"
        );
    }
    product.qp += 1;
    assert!(!witness_predicate(&product, SQUIRE, baseline.xp, false));
    product.qp = SQUIRE.qp_delta;
    product.quest_colour = Some(0xF80000);
    assert!(!witness_predicate(&product, SQUIRE, baseline.xp, false));
    product.quest_colour = Some(0x00F800);
    product
        .main_text
        .retain(|line| line != "Knight's Sword Quest!");
    assert!(!witness_predicate(&product, SQUIRE, baseline.xp, false));

    let mut wrong_reward_text = batch2_product(SQUIRE, &["12,750 Smithing XP"]);
    wrong_reward_text.xp = baseline.xp + SQUIRE.xp_delta;
    assert!(!witness_predicate(
        &wrong_reward_text,
        SQUIRE,
        baseline.xp,
        false
    ));

    let mut product_without_xp = batch2_product(SQUIRE, &[]);
    product_without_xp.xp = baseline.xp + SQUIRE.xp_delta;
    assert!(!witness_predicate(
        &product_without_xp,
        SQUIRE,
        baseline.xp,
        false
    ));
}

#[test]
fn squire_post_seed_path_audit_rejects_the_old_lobster_fixture() {
    let Some(nav_pack) = api::test_support::local_asset_path_377(
        "NAV_377_PACK",
        "",
        "revision-377 navigation pack for the Squire Path audit",
    ) else {
        return;
    };
    let world = nav_planner::world::NavWorld::load_pack(&nav_pack)
        .expect("decode real revision-377 navigation pack");
    let zones = nav_planner::producer::zones_view(&world)
        .expect("read 377 nav identity")
        .expect("377 Squire Path needs its zone catalog");
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R377)
        .expect("selected 377 data");
    let selected_for_gathering = Arc::clone(&selected);
    script::quester::compile::prepare_for_test(move |worker| {
        selected_for_gathering.prepare_gathering(worker)
    })
    .expect("prepare 377 gather catalog");
    let path: script::quester::path::PathDocument =
        serde_json::from_str(SQUIRE_PATH_JSON).expect("377 Squire Path");
    let quests = api::quest_facts::QuestCatalog::from_identity(selected.quest_identity())
        .expect("377 quest catalog");
    let compiled = script::quester::compile::compile_uncached_for_test_with_zones(
        &path, &selected, &quests, zones,
    )
    .unwrap_or_else(|error| panic!("compile 377 Squire Path: {}", error.code));

    let lobster_id = selected.item_by_alias("lobster").expect("377 lobster").id;
    let iron_bar_id = selected.item_by_alias("iron_bar").expect("377 iron bar").id;
    let steel_pickaxe_id = selected
        .item_by_alias("steel_pickaxe")
        .expect("377 steel pickaxe")
        .id;
    let old_fixture_inventory = [(lobster_id, SQUIRE_TEST_FOOD_COUNT)];
    let old_bank_seed = [(iron_bar_id, 2)];
    let path_bank_seed = [(iron_bar_id, 2), (steel_pickaxe_id, 1)];
    let old_audit = squire_post_seed_missing_items(
        &selected,
        &path,
        &compiled,
        "squire:6",
        &old_fixture_inventory,
        &old_bank_seed,
    )
    .expect("audit old Squire fixture");
    assert!(
        old_audit
            .missing
            .iter()
            .any(|item| item.contains("Rune full helm")),
        "old lobster-only fixture should miss the stage-six Path helm: {:?}",
        old_audit.missing
    );
    assert!(
        old_audit
            .missing
            .iter()
            .any(|item| item.contains("gather tool input")),
        "old fixture should miss the unseeded Path gather tool: {:?}",
        old_audit.missing
    );
    assert!(
        old_audit.loadouts.contains(SQUIRE_PATH_LOADOUT),
        "audit should inspect the Path loadout: {:?}",
        old_audit.loadouts
    );
    assert!(
        old_audit.reachable_recipes.contains("acquire:blurite"),
        "audit should walk the deterministic blurite recipe after the seeded bar route: {:?}",
        old_audit.reachable_recipes
    );
    assert!(
        !old_audit.required_recipes.contains("acquire:iron_bars"),
        "the two-bar bank seed should bypass the chance-based dwarf-drop recipe"
    );

    let path_fixture = squire_path_loadout_seed(&selected).expect("FixtureLoadout::Path(\"ice\")");
    let new_audit = squire_post_seed_missing_items(
        &selected,
        &path,
        &compiled,
        "squire:6",
        &path_fixture.inventory,
        &path_bank_seed,
    )
    .expect("audit Path-backed Squire fixture");
    assert!(
        new_audit.missing.is_empty(),
        "Path-backed Squire fixture leaves unsatisfied stage-six requirements: {:?}",
        new_audit.missing
    );
    assert!(
        new_audit.loadouts.contains(SQUIRE_PATH_LOADOUT),
        "audit should inspect the Path loadout: {:?}",
        new_audit.loadouts
    );
    assert!(
        new_audit.reachable_recipes.contains("acquire:blurite"),
        "blurite must remain reachable without a chance outcome: {:?}",
        new_audit.reachable_recipes
    );
}

#[test]
#[ignore = "requires LIVE=1, owned qc4 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_hetty_native() -> Result<(), String> {
    run_quest(HETTY)
}
#[test]
#[ignore = "requires LIVE=1, owned qc4 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_gobdip_native() -> Result<(), String> {
    run_quest(GOBDIP)
}
#[test]
#[ignore = "requires LIVE=1, owned qc4 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_vampire_native() -> Result<(), String> {
    run_quest(VAMPIRE)
}
#[test]
#[ignore = "requires LIVE=1, owned qc4 prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_demon_native() -> Result<(), String> {
    run_quest(DEMON)
}

#[test]
#[ignore = "requires LIVE=1, owned frsq prefix, canonical 377 inputs and QUESTER_377_EVIDENCE_DIR"]
fn quester_377_squire_native() -> Result<(), String> {
    run_quest(SQUIRE)
}
