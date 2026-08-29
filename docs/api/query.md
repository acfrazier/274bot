# Query: the fluent read DSL

`api::query` is the fluent filter surface over the snapshot views. One
generic base plus per-entity extension traits; every terminal reads from
the last snapshot rebuild without allocating a world copy.

## Base

```rust
let mut q = api::query::Query::new(snap.npcs());   // Query<'_, NpcView>
q.where_(|n| n.actor.health > 0).within_distance(8); // chain filters
q.first() / q.last() / q.results() / q.exists() / q.empty() / q.count()
```

`Query<'a, T> { values: &'a [T], predicates: Vec<Box<dyn Fn(&T)->bool + 'a>> }`.
`where_` pushes an `AND` predicate and returns `&mut Self`; `results()`
returns `Vec<&T>`.

## Typed filters (extension traits over `Query<T>`)

- `EntityQueryExt` — `with_name`, `with_id`, `with_index`, `with_action`,
  `name_contains`, `matches_wildcard`, `matches_regex` (a closure matcher),
  `with_name_or_id`.
- `WorldQueryExt` — `within_distance`, `within_distance_to`, `inside`,
  `on_level`, `on_tile`, `nearest`, `nearest_to`.
- `ActorQueryExt` — `with_animation`, `with_pose_animation`, `in_combat`,
  `not_in_combat`, `interacting`, `not_interacting`, `targeting_npc`,
  `targeting_player`, `moving`, `stationary`, `running`, `walking`, `alive`,
  `dead`, `health_at_least`, `health_at_most`.
- `NpcQueryExt` — `with_level`, `level_at_least`, `level_at_most`,
  `with_size`, `interacting_with_local`.
- `PlayerQueryExt` — `with_combat_level`, `combat_level_at_least`,
  `combat_level_at_most`, `with_skill_level`.
- `GroundItemQueryExt` / `ItemQueryExt` — `with_count`, `count_at_least`,
  `count_at_most`, `stackable`, `unstackable`, `noted`, `unnoted`,
  `members`, `free_to_play`, `value_at_least`, `value_at_most`; `ItemQueryExt`
  adds `with_slot` and `total`.
- `LocalQueryExt` — `with_layer`, `with_shape`, `with_angle`,
  `with_footprint`, `blocking_walk`, `not_blocking_walk`, `blocking_range`,
  `not_blocking_range`, `active`, `inactive`, `animated`, `static_`,
  `with_animation`.
- `StatQueryExt` — `with_name`, `with_index`, `with_effective`,
  `effective_at_least/most`, `with_base`, `base_at_least/most`,
  `with_experience`, `experience_at_least/most`, `boosted`, `drained`,
  `unchanged`, `used`.
- `VarpQueryExt` — `with_index`, `with_value`, `zero`, `non_zero`,
  `value_at_least/most`.
- `WidgetQueryExt` — `with_component_id`, `with_layer_id`, `with_parent_id`,
  `with_root_component_id`, `with_root`, `with_type`, `with_button_type`,
  `with_client_code`, `with_button_text`, `with_target_base`,
  `with_model_object_id`, `bound_to_varp`, `hidden`, `not_hidden`,
  `with_text`, `text_contains`, `text_matches`, `with_action`,
  `with_item_id`, `with_any_item`, `with_item_action`, `items`.
- `SideTabQueryExt` — `with_index`, `with_root_component_id`, `available`,
  `unavailable`, `active`, `inactive`, `visible`, `not_visible`, `widgets`.
- `ChatQueryExt` — `with_type_`, `sent_by`, `with_sender`,
  `without_sender`, `with_text`, `text_contains`, `text_matches`, `since`,
  `latest_sequence`.

## Free helpers

- `SceneQuery` — `contains`, `base`, `to_local`, `to_world`, `collision_at`,
  `collision_at_local`, `probeable`, `walkable`, `can_step`,
  `can_operate_from`, `operable_tiles`, `can_reach`.
- `widget_search` — `close_button_com_id`, `button_by_text`,
  `target_button_by_base`, `select_button_by_varp`, `combat_style_labels`.
- `loc_approach` — `can_operate_from`, `operable_tiles`.
