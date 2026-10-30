//! Navigation qualification inputs, not facts authored by a quest guide.
//!
//! Main's 2026-10-30 ruling selects QUEST-TERRA seed state for each sequence,
//! then the preceding spatial step's authored destination. Only a guide with
//! no seed file uses the W4 Lumbridge start and a fresh-account fixture.

use super::{env_path, sha256, tile, tile_json};
use api::game_data::SelectedGameData;
use nav_planner::WorldState;
use serde_json::{json, Value};
use std::fs;
use std::io::ErrorKind;

pub(super) struct GuideQualificationFixture {
    pub(super) start: api::WorldTile,
    pub(super) first_start_rule: &'static str,
    pub(super) state: WorldState,
    pub(super) state_json: Value,
    pub(super) source: Value,
}

impl GuideQualificationFixture {
    pub(super) fn load(
        guide: &str,
        role: &str,
        selected: &SelectedGameData,
        identities: &api::game_data::QuestIdentityFacts,
    ) -> Result<Self, String> {
        let root = env_path(
            "SURVIVABLE_SEED_ROOT",
            "/Volumes/dev-scratch/274bot-evidence/QUEST-TERRA",
        );
        let path = root.join(guide).join("seed.json");
        let (mut seed, source, first_start_rule) = match fs::read(&path) {
            Ok(bytes) => {
                let seed: Value = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("parse seed {}: {error}", path.display()))?;
                (
                    seed,
                    json!({"path":path.display().to_string(),"sha256":sha256(&bytes)}),
                    "quest_seed_start",
                )
            }
            Err(error) if error.kind() == ErrorKind::NotFound => (
                json!({
                    "start_tile":{"x":3222,"z":3218,"level":0},
                    "stats":{},"items":[],"coins":0,"completed_quests":[],
                }),
                Value::Null,
                "lumbridge_fallback",
            ),
            Err(error) => return Err(format!("read seed {}: {error}", path.display())),
        };
        let overrides = seed.pointer(&format!("/role_overrides/{role}")).cloned();
        if let Some(overrides) = overrides.and_then(|value| value.as_object().cloned()) {
            let object = seed.as_object_mut().ok_or("seed must be an object")?;
            for (key, value) in overrides {
                object.insert(key, value);
            }
        }
        let coordinate = |name: &str| -> Result<i32, String> {
            seed["start_tile"][name]
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .ok_or_else(|| format!("seed {} has no usable start_tile.{name}", path.display()))
        };
        let start = tile(coordinate("x")?, coordinate("z")?, coordinate("level")?);
        let mut state = WorldState::empty().with_map_members(true);
        state.inventory_ready = true;
        state.equipment_ready = true;
        // Engine PlayerLoading.ts:39-49: fresh-account levels are 1, HP is 10.
        // PlayerStat.ts:1-22 supplies these revision-289 skill indexes.
        const SKILLS: [&str; 21] = [
            "attack",
            "defence",
            "strength",
            "hitpoints",
            "ranged",
            "prayer",
            "magic",
            "cooking",
            "woodcutting",
            "fletching",
            "fishing",
            "firemaking",
            "crafting",
            "smithing",
            "mining",
            "herblore",
            "agility",
            "thieving",
            "stat18",
            "stat19",
            "runecraft",
        ];
        state.stats = (0..21)
            .map(|index| (index, if index == 3 { 10 } else { 1 }))
            .collect();
        for (name, level) in seed["stats"]
            .as_object()
            .ok_or("seed stats must be an object")?
        {
            let index = SKILLS
                .iter()
                .position(|skill| skill == name)
                .ok_or_else(|| format!("unknown seed skill {name} in {}", path.display()))?;
            let level = level
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .filter(|value| *value > 0)
                .ok_or_else(|| format!("invalid seed level for {name} in {}", path.display()))?;
            state.stats.insert(index as i32, level);
        }
        let [attack, defence, strength, hp, ranged, prayer, magic] =
            std::array::from_fn(|index| i64::from(state.stats[&(index as i32)]));
        let style = (attack + strength)
            .max(ranged / 2 + ranged)
            .max(magic / 2 + magic);
        state.combat_level = Some(((10 * (defence + hp + prayer / 2) + 13 * style) / 40) as i32);
        let count = |value: &Value| -> Result<i32, String> {
            value
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .filter(|value| *value >= 0)
                .ok_or_else(|| format!("invalid seed item count in {}", path.display()))
        };
        let coins = count(&seed["coins"])?;
        if coins > 0 {
            let item = selected
                .item_by_alias("coins")
                .ok_or("selected data has no coins")?;
            state.inv.insert(item.id, coins);
        }
        for item in seed["items"]
            .as_array()
            .ok_or("seed items must be an array")?
        {
            let alias = item["alias"].as_str().ok_or("seed item has no alias")?;
            let id = selected
                .item_by_alias(alias)
                .ok_or_else(|| format!("unknown seed item {alias} in {}", path.display()))?
                .id;
            let quantity = count(&item["count"])?;
            if quantity == 0 {
                continue;
            }
            match item["where"].as_str().unwrap_or("inventory") {
                "inventory" => {
                    let held = state.inv.entry(id).or_insert(0i32);
                    *held = held
                        .checked_add(quantity)
                        .ok_or("seed inventory count overflow")?;
                }
                "worn" => {
                    state.worn.insert(id);
                }
                // Banked items are not held; strict qualification cannot fetch them.
                "bank" => {}
                location => return Err(format!("unknown seed item location {location}")),
            }
        }
        for quest in seed["completed_quests"]
            .as_array()
            .ok_or("seed completed_quests must be an array")?
        {
            let id = quest
                .as_str()
                .ok_or("seed completed quest is not a string")?;
            let row = identities
                .rows
                .iter()
                .find(|row| row.id == id || row.display == id)
                .ok_or_else(|| {
                    format!("unknown completed seed quest {id} in {}", path.display())
                })?;
            state.quests.insert(row.display.clone());
            state.varps.insert(row.varp_id, row.complete);
        }
        let mut worn: Vec<_> = state.worn.iter().copied().collect();
        worn.sort_unstable();
        let mut quests: Vec<_> = state.quests.iter().cloned().collect();
        quests.sort_unstable();
        let state_json = json!({
            "fixture":"guide_seed_qualification_v1",
            "guide":guide,"role":role,"seed_source":source,
            "first_start_rule":first_start_rule,"initial_start":tile_json(start),
            "map_members":state.map_members,"inventory_ready":true,"equipment_ready":true,
            "inventory":state.inv,"worn":worn,"stats":state.stats,
            "combat_level":state.combat_level,"quests":quests,"varps":state.varps,
            "quest_evidence":null,
            "qualification_limits":"Seed state is held constant between steps; no quest-stage progress, acquired items or bank fetch is invented. The three admission loadouts are separate from this navigation seed state.",
            "seed_quest_points_needed":seed.get("quest_points_needed"),
        });
        Ok(Self {
            start,
            first_start_rule,
            state,
            state_json,
            source,
        })
    }
}
