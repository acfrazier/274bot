//! Thin fixture adapter that asks the product's followed-route food guard for its real decision.
use std::sync::Arc;

use api::game_data::NpcNameRow;
use api::quest_progress::EvidenceStamp;
use api::selected::RunKey;
use api::snapshot::{
    ActorKind, ActorTargetView, ActorView, GameSnapshot, HitmarkView, HitmarksView,
    ItemActionFamily, ItemContainer, ItemView, LocalPlayerView, NpcView, PlayerView, StatView,
    WorldStateView, WorldTile,
};
use api::{game_data::SelectedGameData, obj_names::ItemDefView};
use script::combat::guard::{GuardOp, WalkGuard};
use script::combat::tables::CombatTables;
use script::native::{WalkAllow, WalkOptions, WalkRequest};
use script::survival::cure::CurePoison;

#[derive(Debug)]
pub struct FoodRule {
    pub heal: u8,
    pub eat_delay: u8,
    pub skill_delay: u8,
    pub choices: Vec<Vec<bool>>,
}

impl FoodRule {
    pub fn should_eat(&self, hp: u8, cooldown: u8) -> bool {
        self.choices
            .get(usize::from(hp))
            .and_then(|row| row.get(usize::from(cooldown)))
            .copied()
            .unwrap_or(false)
    }
}

pub fn build(
    tables: &CombatTables,
    npc: &NpcNameRow,
    food: &api::game_data::GameItem,
    hp_max: u8,
) -> Result<FoodRule, String> {
    let selected_food = tables.food(food.id).ok_or_else(|| {
        format!(
            "{} is not recognized as product food",
            food.alias.as_deref().unwrap_or("food")
        )
    })?;
    let fact = selected_food;
    if fact.message_delay.is_some() {
        return Err(format!(
            "{} does not use the ordinary single-food timing model",
            food.alias.as_deref().unwrap_or("food")
        ));
    }
    let eat_delay_arg = fact.eat_delay_arg.ok_or_else(|| {
        format!(
            "{} has no ordinary food delay",
            food.alias.as_deref().unwrap_or("food")
        )
    })?;
    let skill_delay_arg = fact.skill_delay_arg.ok_or_else(|| {
        format!(
            "{} has no ordinary skill delay",
            food.alias.as_deref().unwrap_or("food")
        )
    })?;
    let eat_delay = u8::try_from(eat_delay_arg.saturating_add(1))
        .map_err(|_| "food eat delay is outside the supported range".to_string())?;
    let skill_delay = u8::try_from(skill_delay_arg)
        .map_err(|_| "food action delay is outside the supported range".to_string())?;
    let heal = u8::try_from(fact.heal)
        .map_err(|_| "food heal is outside the supported range".to_string())?;
    let name = food
        .name
        .clone()
        .ok_or_else(|| "food has no selected display name".to_string())?;
    let product_tables = script::combat::guard::shared_tables()
        .map_err(|error| format!("product food guard unavailable: {error:?}"))?;
    let mut choices = vec![vec![false; usize::from(eat_delay) + 1]; usize::from(hp_max) + 1];
    for hp in 1..=hp_max {
        for cooldown in 0..=eat_delay {
            let tick = 20 + u16::from(cooldown);
            let current = snapshot(tables.selected(), npc, food, hp, hp_max, 1, tick);
            let current_view = view(&current, tick);
            let request = request(tick);
            let mut guard = WalkGuard::begin_food_only_with(
                &request,
                &current_view,
                Arc::clone(&product_tables),
            )
            .map_err(|error| format!("product food guard refused its fixture: {error:?}"))?;
            if cooldown != 0 {
                let age = eat_delay - cooldown;
                let previous_tick = tick.wrapping_sub(u16::from(age));
                let previous = snapshot(tables.selected(), npc, food, hp, hp_max, 2, previous_tick);
                let previous_view = view(&previous, previous_tick);
                let admitted = GuardOp::Eat {
                    name: Arc::from(name.as_str()),
                };
                guard.admitted(&admitted, &previous_view);
            }
            let choice = guard.tick(&current_view, None, CurePoison::Never);
            choices[usize::from(hp)][usize::from(cooldown)] =
                matches!(choice, Some(GuardOp::Eat { .. }));
        }
    }
    Ok(FoodRule {
        heal,
        eat_delay,
        skill_delay,
        choices,
    })
}

fn request(tick: u16) -> WalkRequest {
    let allow = WalkAllow {
        food: true,
        ..WalkAllow::default()
    };
    WalkRequest {
        target: WorldTile {
            x: 2600,
            z: 3200,
            level: 0,
        },
        candidate_goals: None,
        loc_id: None,
        radius: 1,
        arrival: nav_planner::contract::ArrivalKind::Reach,
        options: WalkOptions::default(),
        required_after: stamp(tick),
        evidence: None,
        cross: Box::default(),
        avoid: Box::default(),
        protect: false,
        food_guard: true,
        allow,
    }
}

fn snapshot(
    selected: &SelectedGameData,
    npc: &NpcNameRow,
    food: &api::game_data::GameItem,
    hp: u8,
    hp_max: u8,
    food_count: i32,
    tick: u16,
) -> GameSnapshot {
    let tile = WorldTile {
        x: 2600,
        z: 3200,
        level: 0,
    };
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_world(WorldStateView {
        map_base_x: 2560,
        map_base_z: 3160,
        members: true,
        ..WorldStateView::default()
    });
    snapshot.seed_stats(
        (0..25)
            .map(|index| StatView {
                index,
                name: api::snapshot::stat_name(index as usize).to_owned(),
                effective: if index == 3 {
                    i32::from(hp)
                } else if index == 5 {
                    1
                } else {
                    40
                },
                base: if index == 3 {
                    i32::from(hp_max)
                } else if index == 5 {
                    1
                } else {
                    40
                },
                xp: 0,
                used: api::snapshot::stat_used(index as usize),
            })
            .collect(),
    );
    let inventory = selected
        .items()
        .iter()
        .find(|item| item.id == food.id)
        .map(|item| ItemView {
            def: ItemDefView {
                id: item.id,
                name: item.name.clone(),
                stackable: item.stackable,
                members: item.members,
                base_value: item.cost,
                noted: item.certificate_link != -1,
                certificate_link: item.certificate_link,
                certificate_template: item.certificate_template,
            },
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: food_count,
            actions: Vec::new(),
            component_id: 0,
        })
        .into_iter()
        .collect();
    snapshot.seed_inventory(inventory, 28);
    snapshot.seed_equipment(Vec::new());
    snapshot.seed_npcs(vec![NpcView {
        index: 7,
        r#type: Some(npc.id as usize),
        name: npc.display.clone(),
        actions: vec![Some("Attack".into())],
        tile: WorldTile {
            x: tile.x + 1,
            z: tile.z,
            level: 0,
        },
        distance: 1,
        animation: -1,
        animation_frame: -1,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: -1,
        health: npc.hitpoints,
        total_health: npc.hitpoints,
        face_entity: -1,
        target: Some(ActorTargetView {
            kind: ActorKind::Player,
            index: 1,
        }),
        moving: false,
        running: false,
        in_combat: true,
        level: 1,
        size: npc.size.max(1),
        network: WorldTile {
            x: tile.x + 1,
            z: tile.z,
            level: 0,
        },
        x: 0,
        z: 0,
        yaw: 0,
    }]);
    snapshot.seed_players(Vec::new());
    snapshot.seed_projectiles(Vec::new());
    snapshot.seed_local_player(LocalPlayerView {
        player: PlayerView {
            index: 1,
            network: tile,
            actor: ActorView {
                name: None,
                actions: Vec::new(),
                tile,
                distance: 0,
                animation: -1,
                animation_frame: -1,
                pose_animation: -1,
                orientation: 0,
                target_orientation: 0,
                overhead_text: None,
                spot_animation: -1,
                spot_animation_stamp: -1,
                health: i32::from(hp),
                total_health: i32::from(hp_max),
                face_entity: -1,
                target: None,
                moving: false,
                running: false,
                in_combat: true,
            },
            combat_level: 46,
            skill_level: 0,
            headicons: 0,
            weapon: None,
        },
        energy: 100,
        weight: 0,
        animation_update: None,
    });
    snapshot.seed_hitmarks(HitmarksView {
        marks: [HitmarkView {
            value: 0,
            kind: 0,
            cycle: 0,
        }; 4],
        loop_cycle: i32::from(tick),
    });
    snapshot
}

fn view(snapshot: &GameSnapshot, tick: u16) -> api::snapshot::SnapshotView<'_> {
    api::snapshot::SnapshotView::new(Some(snapshot), stamp(tick))
}

fn stamp(tick: u16) -> EvidenceStamp {
    EvidenceStamp {
        run: RunKey {
            slot: 0,
            run: 0,
            session: 0,
        },
        tick: u64::from(tick),
        sequence: 0,
    }
}
