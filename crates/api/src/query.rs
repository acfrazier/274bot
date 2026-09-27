//! Borrowing family queries: look up views from the last rebuild without
//! allocating a new world copy.
//!
//! `Query<'a, T>` is the chainable predicate builder. The typed filters
//! are extension traits (`EntityQueryExt` … `ChatQueryExt`), one per
//! entity family, each implemented for `Query<T>` over the matching view
//! so a fluent chain reads `npcs().withName("Goblin").withinDistance(10)
//! .nearest()`. `SceneQuery` reads the built scene's collision grid,
//! `widget_search`/`loc_approach` hold the free helper fns.

use crate::snapshot::{
    ActorTargetView, ChatLineView, GroundItemView, ItemView, LocLayer, LocView, LocalTile, NpcView,
    PlayerView, SceneView, SideTabView, StatView, VarpView, WidgetRoot, WidgetVarpBindingView,
    WidgetView, WorldTile,
};
use client::dash3d::CollisionFlag;
use std::sync::Arc;

/// Where the candidate values live: a borrowed snapshot slice, or an
/// owned copy for derived sub-queries (`WidgetQueryExt::items`,
/// `SideTabQueryExt::widgets`).
enum Values<'a, T> {
    Borrowed(&'a [T]),
    Owned(Vec<T>),
}

impl<'a, T> Values<'a, T> {
    fn as_slice(&self) -> &[T] {
        match self {
            Values::Borrowed(v) => v,
            Values::Owned(v) => v,
        }
    }
}

/// Chainable predicate builder over a borrowed slice. Each `where_` narrows
/// the candidate set; terminal methods evaluate the combined predicates.
pub struct Query<'a, T> {
    values: Values<'a, T>,
    #[allow(clippy::type_complexity)]
    predicates: Vec<Box<dyn Fn(&T) -> bool + 'a>>,
}

impl<'a, T> Query<'a, T> {
    pub fn new(values: &'a [T]) -> Self {
        Query {
            values: Values::Borrowed(values),
            predicates: Vec::new(),
        }
    }

    /// A query over an owned candidate set (the derived sub-queries).
    pub fn from_owned(values: Vec<T>) -> Self {
        Query {
            values: Values::Owned(values),
            predicates: Vec::new(),
        }
    }

    pub fn where_(&mut self, p: impl Fn(&T) -> bool + 'a) -> &mut Self {
        self.predicates.push(Box::new(p));
        self
    }

    pub fn results(&self) -> Vec<&T> {
        self.values
            .as_slice()
            .iter()
            .filter(|v| self.predicates.iter().all(|p| p(v)))
            .collect()
    }

    pub fn first(&self) -> Option<&T> {
        self.values
            .as_slice()
            .iter()
            .find(|v| self.predicates.iter().all(|p| p(v)))
    }

    pub fn last(&self) -> Option<&T> {
        self.values
            .as_slice()
            .iter()
            .rev()
            .find(|v| self.predicates.iter().all(|p| p(v)))
    }

    pub fn exists(&self) -> bool {
        self.first().is_some()
    }

    pub fn empty(&self) -> bool {
        self.first().is_none()
    }

    pub fn count(&self) -> usize {
        self.results().len()
    }
}

// --- shared helpers -------------------------------------------------------

/// The m8aq `normalized`: trimmed, case-folded.
fn normalized(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

/// Anchored `*`-wildcard match (the m8aq `matchesWildcard` glob: every
/// non-`*` char is literal; the caller lowercases for the `i` flag).
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;
    while ti < t.len() {
        if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Chebyshev distance; a level mismatch is infinitely far (the m8aq
/// `chebyshevDistance`). This is public so adapters can reuse the host's
/// distance policy rather than reimplementing coordinate arithmetic.
pub fn chebyshev_to(a: WorldTile, b: WorldTile) -> i32 {
    if a.level != b.level {
        i32::MAX
    } else {
        (a.x - b.x).abs().max((a.z - b.z).abs())
    }
}

// --- view accessors (the trait bounds the extension traits impl over) ----

trait EntityQueryView {
    fn view_name(&self) -> Option<&str>;
    fn view_actions(&self) -> &[Option<String>];
    fn view_id(&self) -> Option<i32>;
    fn view_index(&self) -> Option<usize>;
}

trait WorldQueryView: EntityQueryView {
    fn view_tile(&self) -> WorldTile;
    fn view_distance(&self) -> i32;
}

trait ActorQueryView: WorldQueryView {
    fn view_animation(&self) -> i32;
    fn view_pose_animation(&self) -> i32;
    fn view_in_combat(&self) -> bool;
    fn view_health(&self) -> i32;
    fn view_total_health(&self) -> i32;
    fn view_target(&self) -> Option<ActorTargetView>;
    fn view_moving(&self) -> bool;
    fn view_running(&self) -> bool;
}

trait NpcQueryView: ActorQueryView {
    fn view_level(&self) -> i32;
    fn view_size(&self) -> i32;
}

trait PlayerQueryView: ActorQueryView {
    fn view_combat_level(&self) -> i32;
    fn view_skill_level(&self) -> i32;
}

trait LocalQueryView: WorldQueryView {
    fn view_layer(&self) -> LocLayer;
    fn view_shape(&self) -> i32;
    fn view_angle(&self) -> i32;
    fn view_footprint_width(&self) -> i32;
    fn view_footprint_length(&self) -> i32;
    fn view_block_walk(&self) -> bool;
    fn view_block_range(&self) -> bool;
    fn view_active(&self) -> bool;
    fn view_animation(&self) -> i32;
}

trait StackQueryView {
    fn view_count(&self) -> i32;
    fn view_stackable(&self) -> bool;
    fn view_noted(&self) -> bool;
    fn view_members(&self) -> bool;
    fn view_base_value(&self) -> i32;
}

trait ItemQueryView: EntityQueryView + StackQueryView {
    fn view_slot(&self) -> i32;
}

trait StatQueryView {
    fn view_index(&self) -> i32;
    fn view_name(&self) -> &str;
    fn view_effective(&self) -> i32;
    fn view_base(&self) -> i32;
    fn view_xp(&self) -> i32;
    fn view_used(&self) -> bool;
}

trait VarpQueryView {
    fn view_index(&self) -> i32;
    fn view_value(&self) -> i32;
}

trait WidgetQueryView {
    fn view_component_id(&self) -> i32;
    fn view_layer_id(&self) -> i32;
    fn view_parent_id(&self) -> i32;
    fn view_root_component_id(&self) -> i32;
    fn view_root(&self) -> WidgetRoot;
    fn view_if_type(&self) -> i32;
    fn view_button_type(&self) -> i32;
    fn view_client_code(&self) -> i32;
    fn view_button_text(&self) -> Option<&str>;
    fn view_target_base(&self) -> Option<&str>;
    fn view_model_type(&self) -> i32;
    fn view_model_id(&self) -> i32;
    fn view_varp_bindings(&self) -> &[WidgetVarpBindingView];
    fn view_hidden(&self) -> bool;
    fn view_text(&self) -> Option<&str>;
    fn view_alternate_text(&self) -> Option<&str>;
    fn view_actions(&self) -> &[Option<String>];
    fn view_items(&self) -> &[ItemView];
}

trait SideTabQueryView {
    fn view_index(&self) -> i32;
    fn view_root_component_id(&self) -> i32;
    fn view_available(&self) -> bool;
    fn view_active(&self) -> bool;
    fn view_visible(&self) -> bool;
    fn view_widgets(&self) -> &[WidgetView];
}

trait ChatQueryView {
    fn view_if_type(&self) -> i32;
    fn view_username(&self) -> Option<&str>;
    fn view_text(&self) -> &str;
    fn view_sequence(&self) -> i32;
}

impl EntityQueryView for NpcView {
    fn view_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    fn view_actions(&self) -> &[Option<String>] {
        &self.actions
    }
    fn view_id(&self) -> Option<i32> {
        self.r#type.map(|t| t as i32)
    }
    fn view_index(&self) -> Option<usize> {
        Some(self.index)
    }
}

impl WorldQueryView for NpcView {
    fn view_tile(&self) -> WorldTile {
        self.tile
    }
    fn view_distance(&self) -> i32 {
        self.distance
    }
}

impl ActorQueryView for NpcView {
    fn view_animation(&self) -> i32 {
        self.animation
    }
    fn view_pose_animation(&self) -> i32 {
        self.pose_animation
    }
    fn view_in_combat(&self) -> bool {
        self.in_combat
    }
    fn view_health(&self) -> i32 {
        self.health
    }
    fn view_total_health(&self) -> i32 {
        self.total_health
    }
    fn view_target(&self) -> Option<ActorTargetView> {
        self.target
    }
    fn view_moving(&self) -> bool {
        self.moving
    }
    fn view_running(&self) -> bool {
        self.running
    }
}

impl NpcQueryView for NpcView {
    fn view_level(&self) -> i32 {
        self.level
    }
    fn view_size(&self) -> i32 {
        self.size
    }
}

impl EntityQueryView for PlayerView {
    fn view_name(&self) -> Option<&str> {
        self.actor.name.as_deref()
    }
    fn view_actions(&self) -> &[Option<String>] {
        &self.actor.actions
    }
    fn view_id(&self) -> Option<i32> {
        None
    }
    fn view_index(&self) -> Option<usize> {
        Some(self.index)
    }
}

impl WorldQueryView for PlayerView {
    fn view_tile(&self) -> WorldTile {
        self.actor.tile
    }
    fn view_distance(&self) -> i32 {
        self.actor.distance
    }
}

impl ActorQueryView for PlayerView {
    fn view_animation(&self) -> i32 {
        self.actor.animation
    }
    fn view_pose_animation(&self) -> i32 {
        self.actor.pose_animation
    }
    fn view_in_combat(&self) -> bool {
        self.actor.in_combat
    }
    fn view_health(&self) -> i32 {
        self.actor.health
    }
    fn view_total_health(&self) -> i32 {
        self.actor.total_health
    }
    fn view_target(&self) -> Option<ActorTargetView> {
        self.actor.target
    }
    fn view_moving(&self) -> bool {
        self.actor.moving
    }
    fn view_running(&self) -> bool {
        self.actor.running
    }
}

impl PlayerQueryView for PlayerView {
    fn view_combat_level(&self) -> i32 {
        self.combat_level
    }
    fn view_skill_level(&self) -> i32 {
        self.skill_level
    }
}

impl EntityQueryView for LocView {
    fn view_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    fn view_actions(&self) -> &[Option<String>] {
        &self.actions
    }
    fn view_id(&self) -> Option<i32> {
        Some(self.id)
    }
    fn view_index(&self) -> Option<usize> {
        None
    }
}

impl WorldQueryView for LocView {
    fn view_tile(&self) -> WorldTile {
        self.tile
    }
    fn view_distance(&self) -> i32 {
        self.distance
    }
}

impl LocalQueryView for LocView {
    fn view_layer(&self) -> LocLayer {
        self.layer
    }
    fn view_shape(&self) -> i32 {
        self.shape
    }
    fn view_angle(&self) -> i32 {
        self.angle
    }
    fn view_footprint_width(&self) -> i32 {
        self.footprint_width
    }
    fn view_footprint_length(&self) -> i32 {
        self.footprint_length
    }
    fn view_block_walk(&self) -> bool {
        self.block_walk
    }
    fn view_block_range(&self) -> bool {
        self.block_range
    }
    fn view_active(&self) -> bool {
        self.active
    }
    fn view_animation(&self) -> i32 {
        self.animation
    }
}

impl EntityQueryView for GroundItemView {
    fn view_name(&self) -> Option<&str> {
        self.def.name.as_deref()
    }
    fn view_actions(&self) -> &[Option<String>] {
        &self.actions
    }
    fn view_id(&self) -> Option<i32> {
        Some(self.def.id)
    }
    fn view_index(&self) -> Option<usize> {
        None
    }
}

impl WorldQueryView for GroundItemView {
    fn view_tile(&self) -> WorldTile {
        self.tile
    }
    fn view_distance(&self) -> i32 {
        self.distance
    }
}

impl StackQueryView for GroundItemView {
    fn view_count(&self) -> i32 {
        self.count
    }
    fn view_stackable(&self) -> bool {
        self.def.stackable
    }
    fn view_noted(&self) -> bool {
        self.def.noted
    }
    fn view_members(&self) -> bool {
        self.def.members
    }
    fn view_base_value(&self) -> i32 {
        self.def.base_value
    }
}

impl EntityQueryView for ItemView {
    fn view_name(&self) -> Option<&str> {
        self.def.name.as_deref()
    }
    fn view_actions(&self) -> &[Option<String>] {
        &self.actions
    }
    fn view_id(&self) -> Option<i32> {
        Some(self.def.id)
    }
    fn view_index(&self) -> Option<usize> {
        None
    }
}

impl StackQueryView for ItemView {
    fn view_count(&self) -> i32 {
        self.count
    }
    fn view_stackable(&self) -> bool {
        self.def.stackable
    }
    fn view_noted(&self) -> bool {
        self.def.noted
    }
    fn view_members(&self) -> bool {
        self.def.members
    }
    fn view_base_value(&self) -> i32 {
        self.def.base_value
    }
}

impl ItemQueryView for ItemView {
    fn view_slot(&self) -> i32 {
        self.slot
    }
}

impl StatQueryView for StatView {
    fn view_index(&self) -> i32 {
        self.index
    }
    fn view_name(&self) -> &str {
        &self.name
    }
    fn view_effective(&self) -> i32 {
        self.effective
    }
    fn view_base(&self) -> i32 {
        self.base
    }
    fn view_xp(&self) -> i32 {
        self.xp
    }
    fn view_used(&self) -> bool {
        self.used
    }
}

impl VarpQueryView for VarpView {
    fn view_index(&self) -> i32 {
        self.index
    }
    fn view_value(&self) -> i32 {
        self.value
    }
}

impl WidgetQueryView for WidgetView {
    fn view_component_id(&self) -> i32 {
        self.component_id
    }
    fn view_layer_id(&self) -> i32 {
        self.layer_id
    }
    fn view_parent_id(&self) -> i32 {
        self.parent_id
    }
    fn view_root_component_id(&self) -> i32 {
        self.root_component_id
    }
    fn view_root(&self) -> WidgetRoot {
        self.root
    }
    fn view_if_type(&self) -> i32 {
        self.type_
    }
    fn view_button_type(&self) -> i32 {
        self.button_type
    }
    fn view_client_code(&self) -> i32 {
        self.client_code
    }
    fn view_button_text(&self) -> Option<&str> {
        self.button_text.as_deref()
    }
    fn view_target_base(&self) -> Option<&str> {
        self.target_base.as_deref()
    }
    fn view_model_type(&self) -> i32 {
        self.model_type
    }
    fn view_model_id(&self) -> i32 {
        self.model_id
    }
    fn view_varp_bindings(&self) -> &[WidgetVarpBindingView] {
        &self.varp_bindings
    }
    fn view_hidden(&self) -> bool {
        self.hidden
    }
    fn view_text(&self) -> Option<&str> {
        self.text.as_deref()
    }
    fn view_alternate_text(&self) -> Option<&str> {
        self.alternate_text.as_deref()
    }
    fn view_actions(&self) -> &[Option<String>] {
        &self.actions
    }
    fn view_items(&self) -> &[ItemView] {
        &self.items
    }
}

impl SideTabQueryView for SideTabView {
    fn view_index(&self) -> i32 {
        self.index
    }
    fn view_root_component_id(&self) -> i32 {
        self.root_component_id
    }
    fn view_available(&self) -> bool {
        self.available
    }
    fn view_active(&self) -> bool {
        self.active
    }
    fn view_visible(&self) -> bool {
        self.visible
    }
    fn view_widgets(&self) -> &[WidgetView] {
        &self.widgets
    }
}

impl ChatQueryView for ChatLineView {
    fn view_if_type(&self) -> i32 {
        self.type_
    }
    fn view_username(&self) -> Option<&str> {
        self.username.as_deref()
    }
    fn view_text(&self) -> &str {
        &self.text
    }
    fn view_sequence(&self) -> i32 {
        self.sequence
    }
}

// --- typed extension traits -----------------------------------------------

/// The m8aq `(string | number)[]` of `withNameOrId`: each value is either
/// a name (compared case-insensitively) or a def id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameOrId {
    Name(String),
    Id(i32),
}

/// A rectangle of world tiles (the m8aq `WorldArea`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldArea {
    pub min_x: i32,
    pub max_x: i32,
    pub min_z: i32,
    pub max_z: i32,
    pub level: i32,
}

impl WorldArea {
    pub fn contains_tile(&self, tile: WorldTile) -> bool {
        tile.level == self.level
            && tile.x >= self.min_x
            && tile.x <= self.max_x
            && tile.z >= self.min_z
            && tile.z <= self.max_z
    }
}

/// Entity filters: name/id/index/action matching (the m8aq `EntityQuery`).
pub trait EntityQueryExt<'a, T> {
    fn with_name(&mut self, names: &[&str]) -> &mut Self;
    fn with_id(&mut self, ids: &[i32]) -> &mut Self;
    fn with_index(&mut self, indexes: &[usize]) -> &mut Self;
    fn with_action(&mut self, actions: &[&str]) -> &mut Self;
    fn name_contains(&mut self, terms: &[&str]) -> &mut Self;
    fn matches_wildcard(&mut self, patterns: &[&str]) -> &mut Self;
    fn matches_regex<F>(&mut self, matcher: F) -> &mut Self
    where
        F: Fn(&str) -> bool + 'a;
    fn with_name_or_id(&mut self, values: &[NameOrId]) -> &mut Self;
}

impl<'a, T: EntityQueryView> EntityQueryExt<'a, T> for Query<'a, T> {
    fn with_name(&mut self, names: &[&str]) -> &mut Self {
        let wanted: Vec<String> = names.iter().map(|n| normalized(n)).collect();
        self.where_(move |v: &T| {
            v.view_name()
                .is_some_and(|name| wanted.contains(&normalized(name)))
        })
    }

    fn with_id(&mut self, ids: &[i32]) -> &mut Self {
        let ids: Vec<i32> = ids.to_vec();
        self.where_(move |v: &T| v.view_id().is_some_and(|id| ids.contains(&id)))
    }

    fn with_index(&mut self, indexes: &[usize]) -> &mut Self {
        let indexes: Vec<usize> = indexes.to_vec();
        self.where_(move |v: &T| v.view_index().is_some_and(|i| indexes.contains(&i)))
    }

    fn with_action(&mut self, actions: &[&str]) -> &mut Self {
        let wanted: Vec<String> = actions.iter().map(|a| normalized(a)).collect();
        self.where_(move |v: &T| {
            let actions = v.view_actions();
            !actions.is_empty()
                && actions.iter().any(|a| {
                    a.as_deref()
                        .is_some_and(|a| wanted.contains(&normalized(a)))
                })
        })
    }

    fn name_contains(&mut self, terms: &[&str]) -> &mut Self {
        let wanted: Vec<String> = terms.iter().map(|t| normalized(t)).collect();
        self.where_(move |v: &T| {
            v.view_name().is_some_and(|name| {
                let name = normalized(name);
                !wanted.is_empty() && wanted.iter().any(|term| name.contains(term.as_str()))
            })
        })
    }

    fn matches_wildcard(&mut self, patterns: &[&str]) -> &mut Self {
        let patterns: Vec<String> = patterns.iter().map(|p| p.to_ascii_lowercase()).collect();
        self.where_(move |v: &T| {
            v.view_name().is_some_and(|name| {
                let name = name.to_ascii_lowercase();
                !patterns.is_empty() && patterns.iter().any(|p| wildcard_match(p, &name))
            })
        })
    }

    fn matches_regex<F>(&mut self, matcher: F) -> &mut Self
    where
        F: Fn(&str) -> bool + 'a,
    {
        self.where_(move |v: &T| v.view_name().is_some_and(&matcher))
    }

    fn with_name_or_id(&mut self, values: &[NameOrId]) -> &mut Self {
        let names: Vec<String> = values
            .iter()
            .filter_map(|v| match v {
                NameOrId::Name(n) => Some(normalized(n)),
                NameOrId::Id(_) => None,
            })
            .collect();
        let ids: Vec<i32> = values
            .iter()
            .filter_map(|v| match v {
                NameOrId::Id(i) => Some(*i),
                NameOrId::Name(_) => None,
            })
            .collect();
        self.where_(move |v: &T| {
            let name_match = v
                .view_name()
                .is_some_and(|name| names.contains(&normalized(name)));
            let id_match = v.view_id().is_some_and(|id| ids.contains(&id));
            name_match || id_match
        })
    }
}

/// World filters: distance/tile matching plus the nearest terminals
/// (the m8aq `WorldQuery`).
pub trait WorldQueryExt<'a, T>: EntityQueryExt<'a, T> {
    fn within_distance(&mut self, distance: i32) -> &mut Self;
    fn within_distance_to(&mut self, point: WorldTile, distance: i32) -> &mut Self;
    fn inside(&mut self, area: WorldArea) -> &mut Self;
    fn on_level(&mut self, level: i32) -> &mut Self;
    fn on_tile(&mut self, tile: WorldTile) -> &mut Self;
    fn nearest(&self) -> Option<&T>;
    fn nearest_to(&self, point: WorldTile) -> Option<&T>;
}

impl<'a, T: WorldQueryView> WorldQueryExt<'a, T> for Query<'a, T> {
    fn within_distance(&mut self, distance: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_distance() <= distance)
    }

    fn within_distance_to(&mut self, point: WorldTile, distance: i32) -> &mut Self {
        self.where_(move |v: &T| chebyshev_to(v.view_tile(), point) <= distance)
    }

    fn inside(&mut self, area: WorldArea) -> &mut Self {
        self.where_(move |v: &T| area.contains_tile(v.view_tile()))
    }

    fn on_level(&mut self, level: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_tile().level == level)
    }

    fn on_tile(&mut self, tile: WorldTile) -> &mut Self {
        self.where_(move |v: &T| v.view_tile() == tile)
    }

    fn nearest(&self) -> Option<&T> {
        self.results().into_iter().reduce(|best, v| {
            if v.view_distance() < best.view_distance() {
                v
            } else {
                best
            }
        })
    }

    fn nearest_to(&self, point: WorldTile) -> Option<&T> {
        self.results().into_iter().reduce(|best, v| {
            if chebyshev_to(v.view_tile(), point) < chebyshev_to(best.view_tile(), point) {
                v
            } else {
                best
            }
        })
    }
}

/// Actor filters: animation/combat/interaction/movement/health
/// (the m8aq `ActorQuery`).
pub trait ActorQueryExt<'a, T>: WorldQueryExt<'a, T> {
    fn with_animation(&mut self, animations: &[i32]) -> &mut Self;
    fn with_pose_animation(&mut self, animations: &[i32]) -> &mut Self;
    fn in_combat(&mut self) -> &mut Self;
    fn not_in_combat(&mut self) -> &mut Self;
    fn interacting(&mut self) -> &mut Self;
    fn not_interacting(&mut self) -> &mut Self;
    fn targeting_npc(&mut self, indexes: &[usize]) -> &mut Self;
    fn targeting_player(&mut self, indexes: &[usize]) -> &mut Self;
    fn moving(&mut self) -> &mut Self;
    fn stationary(&mut self) -> &mut Self;
    fn running(&mut self) -> &mut Self;
    fn walking(&mut self) -> &mut Self;
    fn alive(&mut self) -> &mut Self;
    fn dead(&mut self) -> &mut Self;
    fn health_at_least(&mut self, percent: i32) -> &mut Self;
    fn health_at_most(&mut self, percent: i32) -> &mut Self;
}

impl<'a, T: ActorQueryView> ActorQueryExt<'a, T> for Query<'a, T> {
    fn with_animation(&mut self, animations: &[i32]) -> &mut Self {
        let animations: Vec<i32> = animations.to_vec();
        self.where_(move |v: &T| animations.contains(&v.view_animation()))
    }

    fn with_pose_animation(&mut self, animations: &[i32]) -> &mut Self {
        let animations: Vec<i32> = animations.to_vec();
        self.where_(move |v: &T| animations.contains(&v.view_pose_animation()))
    }

    fn in_combat(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_in_combat())
    }

    fn not_in_combat(&mut self) -> &mut Self {
        self.where_(|v: &T| !v.view_in_combat())
    }

    fn interacting(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_target().is_some())
    }

    fn not_interacting(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_target().is_none())
    }

    fn targeting_npc(&mut self, indexes: &[usize]) -> &mut Self {
        let indexes: Vec<usize> = indexes.to_vec();
        self.where_(move |v: &T| {
            v.view_target().is_some_and(|t| {
                t.kind == crate::snapshot::ActorKind::Npc && indexes.contains(&t.index)
            })
        })
    }

    fn targeting_player(&mut self, indexes: &[usize]) -> &mut Self {
        let indexes: Vec<usize> = indexes.to_vec();
        self.where_(move |v: &T| {
            v.view_target().is_some_and(|t| {
                t.kind == crate::snapshot::ActorKind::Player && indexes.contains(&t.index)
            })
        })
    }

    fn moving(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_moving())
    }

    fn stationary(&mut self) -> &mut Self {
        self.where_(|v: &T| !v.view_moving())
    }

    fn running(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_moving() && v.view_running())
    }

    fn walking(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_moving() && !v.view_running())
    }

    fn alive(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_total_health() == 0 || v.view_health() > 0)
    }

    fn dead(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_total_health() > 0 && v.view_health() == 0)
    }

    fn health_at_least(&mut self, percent: i32) -> &mut Self {
        self.where_(move |v: &T| {
            v.view_total_health() > 0
                && (v.view_health() as i64) * 100 >= (v.view_total_health() as i64) * percent as i64
        })
    }

    fn health_at_most(&mut self, percent: i32) -> &mut Self {
        self.where_(move |v: &T| {
            v.view_total_health() > 0
                && (v.view_health() as i64) * 100 <= (v.view_total_health() as i64) * percent as i64
        })
    }
}

/// NPC filters (the m8aq `NpcQuery`).
pub trait NpcQueryExt<'a, T>: ActorQueryExt<'a, T> {
    fn with_level(&mut self, levels: &[i32]) -> &mut Self;
    fn level_at_least(&mut self, level: i32) -> &mut Self;
    fn level_at_most(&mut self, level: i32) -> &mut Self;
    fn with_size(&mut self, sizes: &[i32]) -> &mut Self;
    fn interacting_with_local(&mut self, self_slot: usize) -> &mut Self;
}

impl<'a, T: NpcQueryView> NpcQueryExt<'a, T> for Query<'a, T> {
    fn with_level(&mut self, levels: &[i32]) -> &mut Self {
        let levels: Vec<i32> = levels.to_vec();
        self.where_(move |v: &T| levels.contains(&v.view_level()))
    }

    fn level_at_least(&mut self, level: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_level() >= level)
    }

    fn level_at_most(&mut self, level: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_level() <= level)
    }

    fn with_size(&mut self, sizes: &[i32]) -> &mut Self {
        let sizes: Vec<i32> = sizes.to_vec();
        self.where_(move |v: &T| sizes.contains(&v.view_size()))
    }

    fn interacting_with_local(&mut self, self_slot: usize) -> &mut Self {
        self.targeting_player(&[self_slot])
    }
}

/// Player filters (the m8aq `PlayerQuery`).
pub trait PlayerQueryExt<'a, T>: ActorQueryExt<'a, T> {
    fn with_combat_level(&mut self, levels: &[i32]) -> &mut Self;
    fn combat_level_at_least(&mut self, level: i32) -> &mut Self;
    fn combat_level_at_most(&mut self, level: i32) -> &mut Self;
    fn with_skill_level(&mut self, levels: &[i32]) -> &mut Self;
}

impl<'a, T: PlayerQueryView> PlayerQueryExt<'a, T> for Query<'a, T> {
    fn with_combat_level(&mut self, levels: &[i32]) -> &mut Self {
        let levels: Vec<i32> = levels.to_vec();
        self.where_(move |v: &T| levels.contains(&v.view_combat_level()))
    }

    fn combat_level_at_least(&mut self, level: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_combat_level() >= level)
    }

    fn combat_level_at_most(&mut self, level: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_combat_level() <= level)
    }

    fn with_skill_level(&mut self, levels: &[i32]) -> &mut Self {
        let levels: Vec<i32> = levels.to_vec();
        self.where_(move |v: &T| levels.contains(&v.view_skill_level()))
    }
}

/// Ground-item filters (the m8aq `GroundItemQuery`).
pub trait GroundItemQueryExt<'a, T>: WorldQueryExt<'a, T> {
    fn with_count(&mut self, count: i32) -> &mut Self;
    fn count_at_least(&mut self, count: i32) -> &mut Self;
    fn count_at_most(&mut self, count: i32) -> &mut Self;
    fn stackable(&mut self) -> &mut Self;
    fn unstackable(&mut self) -> &mut Self;
    fn noted(&mut self) -> &mut Self;
    fn unnoted(&mut self) -> &mut Self;
    fn members(&mut self) -> &mut Self;
    fn free_to_play(&mut self) -> &mut Self;
    fn value_at_least(&mut self, value: i32) -> &mut Self;
    fn value_at_most(&mut self, value: i32) -> &mut Self;
}

impl<'a, T: WorldQueryView + StackQueryView> GroundItemQueryExt<'a, T> for Query<'a, T> {
    fn with_count(&mut self, count: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_count() == count)
    }

    fn count_at_least(&mut self, count: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_count() >= count)
    }

    fn count_at_most(&mut self, count: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_count() <= count)
    }

    fn stackable(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_stackable())
    }

    fn unstackable(&mut self) -> &mut Self {
        self.where_(|v: &T| !v.view_stackable())
    }

    fn noted(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_noted())
    }

    fn unnoted(&mut self) -> &mut Self {
        self.where_(|v: &T| !v.view_noted())
    }

    fn members(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_members())
    }

    fn free_to_play(&mut self) -> &mut Self {
        self.where_(|v: &T| !v.view_members())
    }

    fn value_at_least(&mut self, value: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_base_value() >= value)
    }

    fn value_at_most(&mut self, value: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_base_value() <= value)
    }
}

/// Item filters plus the count-summing `total` terminal (the m8aq
/// `ItemQuery`).
pub trait ItemQueryExt<'a, T>: EntityQueryExt<'a, T> {
    fn with_slot(&mut self, slots: &[i32]) -> &mut Self;
    fn with_count(&mut self, count: i32) -> &mut Self;
    fn count_at_least(&mut self, count: i32) -> &mut Self;
    fn count_at_most(&mut self, count: i32) -> &mut Self;
    fn stackable(&mut self) -> &mut Self;
    fn unstackable(&mut self) -> &mut Self;
    fn noted(&mut self) -> &mut Self;
    fn unnoted(&mut self) -> &mut Self;
    fn members(&mut self) -> &mut Self;
    fn free_to_play(&mut self) -> &mut Self;
    fn value_at_least(&mut self, value: i32) -> &mut Self;
    fn value_at_most(&mut self, value: i32) -> &mut Self;
    fn total(&self) -> i32;
}

impl<'a, T: ItemQueryView> ItemQueryExt<'a, T> for Query<'a, T> {
    fn with_slot(&mut self, slots: &[i32]) -> &mut Self {
        let slots: Vec<i32> = slots.to_vec();
        self.where_(move |v: &T| slots.contains(&v.view_slot()))
    }

    fn with_count(&mut self, count: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_count() == count)
    }

    fn count_at_least(&mut self, count: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_count() >= count)
    }

    fn count_at_most(&mut self, count: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_count() <= count)
    }

    fn stackable(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_stackable())
    }

    fn unstackable(&mut self) -> &mut Self {
        self.where_(|v: &T| !v.view_stackable())
    }

    fn noted(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_noted())
    }

    fn unnoted(&mut self) -> &mut Self {
        self.where_(|v: &T| !v.view_noted())
    }

    fn members(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_members())
    }

    fn free_to_play(&mut self) -> &mut Self {
        self.where_(|v: &T| !v.view_members())
    }

    fn value_at_least(&mut self, value: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_base_value() >= value)
    }

    fn value_at_most(&mut self, value: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_base_value() <= value)
    }

    fn total(&self) -> i32 {
        self.results().into_iter().map(|v| v.view_count()).sum()
    }
}

/// Loc filters (the m8aq `LocalQuery`).
pub trait LocalQueryExt<'a, T>: WorldQueryExt<'a, T> {
    fn with_layer(&mut self, layers: &[LocLayer]) -> &mut Self;
    fn with_shape(&mut self, shapes: &[i32]) -> &mut Self;
    fn with_angle(&mut self, angles: &[i32]) -> &mut Self;
    fn with_footprint(&mut self, width: i32, length: i32) -> &mut Self;
    fn blocking_walk(&mut self) -> &mut Self;
    fn not_blocking_walk(&mut self) -> &mut Self;
    fn blocking_range(&mut self) -> &mut Self;
    fn not_blocking_range(&mut self) -> &mut Self;
    fn active(&mut self) -> &mut Self;
    fn inactive(&mut self) -> &mut Self;
    fn animated(&mut self) -> &mut Self;
    fn static_(&mut self) -> &mut Self;
    fn with_animation(&mut self, animations: &[i32]) -> &mut Self;
}

impl<'a, T: LocalQueryView> LocalQueryExt<'a, T> for Query<'a, T> {
    fn with_layer(&mut self, layers: &[LocLayer]) -> &mut Self {
        let layers: Vec<LocLayer> = layers.to_vec();
        self.where_(move |v: &T| layers.contains(&v.view_layer()))
    }

    fn with_shape(&mut self, shapes: &[i32]) -> &mut Self {
        let shapes: Vec<i32> = shapes.to_vec();
        self.where_(move |v: &T| shapes.contains(&v.view_shape()))
    }

    fn with_angle(&mut self, angles: &[i32]) -> &mut Self {
        let angles: Vec<i32> = angles.to_vec();
        self.where_(move |v: &T| angles.contains(&v.view_angle()))
    }

    fn with_footprint(&mut self, width: i32, length: i32) -> &mut Self {
        self.where_(move |v: &T| {
            v.view_footprint_width() == width && v.view_footprint_length() == length
        })
    }

    fn blocking_walk(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_block_walk())
    }

    fn not_blocking_walk(&mut self) -> &mut Self {
        self.where_(|v: &T| !v.view_block_walk())
    }

    fn blocking_range(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_block_range())
    }

    fn not_blocking_range(&mut self) -> &mut Self {
        self.where_(|v: &T| !v.view_block_range())
    }

    fn active(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_active())
    }

    fn inactive(&mut self) -> &mut Self {
        self.where_(|v: &T| !v.view_active())
    }

    fn animated(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_animation() != -1)
    }

    fn static_(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_animation() == -1)
    }

    fn with_animation(&mut self, animations: &[i32]) -> &mut Self {
        let animations: Vec<i32> = animations.to_vec();
        self.where_(move |v: &T| animations.contains(&v.view_animation()))
    }
}

/// Stat filters (the m8aq `StatQuery`).
pub trait StatQueryExt<'a, T> {
    fn with_name(&mut self, names: &[&str]) -> &mut Self;
    fn with_index(&mut self, indexes: &[i32]) -> &mut Self;
    fn with_effective(&mut self, levels: &[i32]) -> &mut Self;
    fn effective_at_least(&mut self, level: i32) -> &mut Self;
    fn effective_at_most(&mut self, level: i32) -> &mut Self;
    fn with_base(&mut self, levels: &[i32]) -> &mut Self;
    fn base_at_least(&mut self, level: i32) -> &mut Self;
    fn base_at_most(&mut self, level: i32) -> &mut Self;
    fn with_experience(&mut self, experience: &[i32]) -> &mut Self;
    fn experience_at_least(&mut self, experience: i32) -> &mut Self;
    fn experience_at_most(&mut self, experience: i32) -> &mut Self;
    fn boosted(&mut self) -> &mut Self;
    fn drained(&mut self) -> &mut Self;
    fn unchanged(&mut self) -> &mut Self;
    fn used(&mut self) -> &mut Self;
}

impl<'a, T: StatQueryView> StatQueryExt<'a, T> for Query<'a, T> {
    fn with_name(&mut self, names: &[&str]) -> &mut Self {
        let wanted: Vec<String> = names.iter().map(|n| normalized(n)).collect();
        self.where_(move |v: &T| !wanted.is_empty() && wanted.contains(&normalized(v.view_name())))
    }

    fn with_index(&mut self, indexes: &[i32]) -> &mut Self {
        let indexes: Vec<i32> = indexes.to_vec();
        self.where_(move |v: &T| indexes.contains(&v.view_index()))
    }

    fn with_effective(&mut self, levels: &[i32]) -> &mut Self {
        let levels: Vec<i32> = levels.to_vec();
        self.where_(move |v: &T| levels.contains(&v.view_effective()))
    }

    fn effective_at_least(&mut self, level: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_effective() >= level)
    }

    fn effective_at_most(&mut self, level: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_effective() <= level)
    }

    fn with_base(&mut self, levels: &[i32]) -> &mut Self {
        let levels: Vec<i32> = levels.to_vec();
        self.where_(move |v: &T| levels.contains(&v.view_base()))
    }

    fn base_at_least(&mut self, level: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_base() >= level)
    }

    fn base_at_most(&mut self, level: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_base() <= level)
    }

    fn with_experience(&mut self, experience: &[i32]) -> &mut Self {
        let experience: Vec<i32> = experience.to_vec();
        self.where_(move |v: &T| experience.contains(&v.view_xp()))
    }

    fn experience_at_least(&mut self, experience: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_xp() >= experience)
    }

    fn experience_at_most(&mut self, experience: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_xp() <= experience)
    }

    fn boosted(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_effective() > v.view_base())
    }

    fn drained(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_effective() < v.view_base())
    }

    fn unchanged(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_effective() == v.view_base())
    }

    fn used(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_used())
    }
}

/// Varp filters (the m8aq `VarpQuery`).
pub trait VarpQueryExt<'a, T> {
    fn with_index(&mut self, indexes: &[i32]) -> &mut Self;
    fn with_value(&mut self, values: &[i32]) -> &mut Self;
    fn zero(&mut self) -> &mut Self;
    fn non_zero(&mut self) -> &mut Self;
    fn value_at_least(&mut self, value: i32) -> &mut Self;
    fn value_at_most(&mut self, value: i32) -> &mut Self;
}

impl<'a, T: VarpQueryView> VarpQueryExt<'a, T> for Query<'a, T> {
    fn with_index(&mut self, indexes: &[i32]) -> &mut Self {
        let indexes: Vec<i32> = indexes.to_vec();
        self.where_(move |v: &T| indexes.contains(&v.view_index()))
    }

    fn with_value(&mut self, values: &[i32]) -> &mut Self {
        let values: Vec<i32> = values.to_vec();
        self.where_(move |v: &T| values.contains(&v.view_value()))
    }

    fn zero(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_value() == 0)
    }

    fn non_zero(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_value() != 0)
    }

    fn value_at_least(&mut self, value: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_value() >= value)
    }

    fn value_at_most(&mut self, value: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_value() <= value)
    }
}

/// Widget filters plus the item-bearing `items` sub-query (the m8aq
/// `WidgetQuery`).
pub trait WidgetQueryExt<'a, T> {
    fn with_component_id(&mut self, component_ids: &[i32]) -> &mut Self;
    fn with_layer_id(&mut self, layer_ids: &[i32]) -> &mut Self;
    fn with_parent_id(&mut self, parent_ids: &[i32]) -> &mut Self;
    fn with_root_component_id(&mut self, root_component_ids: &[i32]) -> &mut Self;
    fn with_root(&mut self, roots: &[WidgetRoot]) -> &mut Self;
    fn with_type_(&mut self, types: &[i32]) -> &mut Self;
    fn with_button_type(&mut self, button_types: &[i32]) -> &mut Self;
    fn with_client_code(&mut self, client_codes: &[i32]) -> &mut Self;
    fn with_button_text(&mut self, texts: &[&str]) -> &mut Self;
    fn with_target_base(&mut self, targets: &[&str]) -> &mut Self;
    fn with_model_object_id(&mut self, item_ids: &[i32]) -> &mut Self;
    fn bound_to_varp(&mut self, varp: i32, value: Option<i32>) -> &mut Self;
    fn hidden(&mut self) -> &mut Self;
    fn not_hidden(&mut self) -> &mut Self;
    fn with_text(&mut self, texts: &[&str]) -> &mut Self;
    fn text_contains(&mut self, terms: &[&str]) -> &mut Self;
    fn text_matches<F>(&mut self, matcher: F) -> &mut Self
    where
        F: Fn(&str) -> bool + 'a;
    fn with_action(&mut self, actions: &[&str]) -> &mut Self;
    fn with_item_id(&mut self, item_ids: &[i32]) -> &mut Self;
    fn with_any_item(&mut self) -> &mut Self;
    fn with_item_action(&mut self, actions: &[&str]) -> &mut Self;
    fn items(&self) -> Query<'_, ItemView>;
}

impl<'a, T: WidgetQueryView> WidgetQueryExt<'a, T> for Query<'a, T> {
    fn with_component_id(&mut self, component_ids: &[i32]) -> &mut Self {
        let component_ids: Vec<i32> = component_ids.to_vec();
        self.where_(move |v: &T| component_ids.contains(&v.view_component_id()))
    }

    fn with_layer_id(&mut self, layer_ids: &[i32]) -> &mut Self {
        let layer_ids: Vec<i32> = layer_ids.to_vec();
        self.where_(move |v: &T| layer_ids.contains(&v.view_layer_id()))
    }

    fn with_parent_id(&mut self, parent_ids: &[i32]) -> &mut Self {
        let parent_ids: Vec<i32> = parent_ids.to_vec();
        self.where_(move |v: &T| parent_ids.contains(&v.view_parent_id()))
    }

    fn with_root_component_id(&mut self, root_component_ids: &[i32]) -> &mut Self {
        let root_component_ids: Vec<i32> = root_component_ids.to_vec();
        self.where_(move |v: &T| root_component_ids.contains(&v.view_root_component_id()))
    }

    fn with_root(&mut self, roots: &[WidgetRoot]) -> &mut Self {
        let roots: Vec<WidgetRoot> = roots.to_vec();
        self.where_(move |v: &T| roots.contains(&v.view_root()))
    }

    fn with_type_(&mut self, types: &[i32]) -> &mut Self {
        let types: Vec<i32> = types.to_vec();
        self.where_(move |v: &T| types.contains(&v.view_if_type()))
    }

    fn with_button_type(&mut self, button_types: &[i32]) -> &mut Self {
        let button_types: Vec<i32> = button_types.to_vec();
        self.where_(move |v: &T| button_types.contains(&v.view_button_type()))
    }

    fn with_client_code(&mut self, client_codes: &[i32]) -> &mut Self {
        let client_codes: Vec<i32> = client_codes.to_vec();
        self.where_(move |v: &T| client_codes.contains(&v.view_client_code()))
    }

    fn with_button_text(&mut self, texts: &[&str]) -> &mut Self {
        let wanted: Vec<String> = texts.iter().map(|t| normalized(t)).collect();
        self.where_(move |v: &T| {
            v.view_button_text()
                .is_some_and(|t| wanted.contains(&normalized(t)))
        })
    }

    fn with_target_base(&mut self, targets: &[&str]) -> &mut Self {
        let wanted: Vec<String> = targets.iter().map(|t| normalized(t)).collect();
        self.where_(move |v: &T| {
            v.view_target_base()
                .is_some_and(|t| wanted.contains(&normalized(t)))
        })
    }

    fn with_model_object_id(&mut self, item_ids: &[i32]) -> &mut Self {
        let item_ids: Vec<i32> = item_ids.to_vec();
        self.where_(move |v: &T| v.view_model_type() == 4 && item_ids.contains(&v.view_model_id()))
    }

    fn bound_to_varp(&mut self, varp: i32, value: Option<i32>) -> &mut Self {
        self.where_(move |v: &T| {
            v.view_varp_bindings()
                .iter()
                .any(|b| b.varp == varp && value.is_none_or(|want| b.value == Some(want)))
        })
    }

    fn hidden(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_hidden())
    }

    fn not_hidden(&mut self) -> &mut Self {
        self.where_(|v: &T| !v.view_hidden())
    }

    fn with_text(&mut self, texts: &[&str]) -> &mut Self {
        let wanted: Vec<String> = texts.iter().map(|t| normalized(t)).collect();
        self.where_(move |v: &T| {
            v.view_text()
                .into_iter()
                .chain(v.view_alternate_text())
                .any(|text| wanted.contains(&normalized(text)))
        })
    }

    fn text_contains(&mut self, terms: &[&str]) -> &mut Self {
        let wanted: Vec<String> = terms.iter().map(|t| normalized(t)).collect();
        self.where_(move |v: &T| {
            let texts: Vec<&str> = v
                .view_text()
                .into_iter()
                .chain(v.view_alternate_text())
                .collect();
            !wanted.is_empty()
                && texts.iter().any(|t| {
                    wanted
                        .iter()
                        .any(|term| normalized(t).contains(term.as_str()))
                })
        })
    }

    fn text_matches<F>(&mut self, matcher: F) -> &mut Self
    where
        F: Fn(&str) -> bool + 'a,
    {
        self.where_(move |v: &T| {
            v.view_text().is_some_and(&matcher) || v.view_alternate_text().is_some_and(&matcher)
        })
    }

    fn with_action(&mut self, actions: &[&str]) -> &mut Self {
        let wanted: Vec<String> = actions.iter().map(|a| normalized(a)).collect();
        self.where_(move |v: &T| {
            v.view_actions().iter().any(|a| {
                a.as_deref()
                    .is_some_and(|a| wanted.contains(&normalized(a)))
            })
        })
    }

    fn with_item_id(&mut self, item_ids: &[i32]) -> &mut Self {
        let item_ids: Vec<i32> = item_ids.to_vec();
        self.where_(move |v: &T| {
            v.view_items()
                .iter()
                .any(|item| item_ids.contains(&item.def.id))
        })
    }

    fn with_any_item(&mut self) -> &mut Self {
        self.where_(|v: &T| !v.view_items().is_empty())
    }

    fn with_item_action(&mut self, actions: &[&str]) -> &mut Self {
        let wanted: Vec<String> = actions.iter().map(|a| normalized(a)).collect();
        self.where_(move |v: &T| {
            v.view_items().iter().any(|item| {
                item.actions.iter().any(|a| {
                    a.as_deref()
                        .is_some_and(|a| wanted.contains(&normalized(a)))
                })
            })
        })
    }

    fn items(&self) -> Query<'_, ItemView> {
        let items: Vec<ItemView> = self
            .results()
            .into_iter()
            .flat_map(|v| v.view_items().iter().cloned())
            .collect();
        Query::from_owned(items)
    }
}

/// Side-tab filters plus the `widgets` sub-query (the m8aq
/// `SideTabQuery`).
pub trait SideTabQueryExt<'a, T> {
    fn with_index(&mut self, indexes: &[i32]) -> &mut Self;
    fn with_root_component_id(&mut self, component_ids: &[i32]) -> &mut Self;
    fn available(&mut self) -> &mut Self;
    fn unavailable(&mut self) -> &mut Self;
    fn active(&mut self) -> &mut Self;
    fn inactive(&mut self) -> &mut Self;
    fn visible(&mut self) -> &mut Self;
    fn not_visible(&mut self) -> &mut Self;
    fn widgets(&self) -> Query<'_, WidgetView>;
}

impl<'a, T: SideTabQueryView> SideTabQueryExt<'a, T> for Query<'a, T> {
    fn with_index(&mut self, indexes: &[i32]) -> &mut Self {
        let indexes: Vec<i32> = indexes.to_vec();
        self.where_(move |v: &T| indexes.contains(&v.view_index()))
    }

    fn with_root_component_id(&mut self, component_ids: &[i32]) -> &mut Self {
        let component_ids: Vec<i32> = component_ids.to_vec();
        self.where_(move |v: &T| component_ids.contains(&v.view_root_component_id()))
    }

    fn available(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_available())
    }

    fn unavailable(&mut self) -> &mut Self {
        self.where_(|v: &T| !v.view_available())
    }

    fn active(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_active())
    }

    fn inactive(&mut self) -> &mut Self {
        self.where_(|v: &T| !v.view_active())
    }

    fn visible(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_visible())
    }

    fn not_visible(&mut self) -> &mut Self {
        self.where_(|v: &T| !v.view_visible())
    }

    fn widgets(&self) -> Query<'_, WidgetView> {
        let widgets: Vec<WidgetView> = self
            .results()
            .into_iter()
            .flat_map(|v| v.view_widgets().iter().cloned())
            .collect();
        Query::from_owned(widgets)
    }
}

/// Chat filters plus the sequence terminals (the m8aq `ChatQuery`).
pub trait ChatQueryExt<'a, T> {
    fn with_type_(&mut self, types: &[i32]) -> &mut Self;
    /// Chat lines whose username is one of `usernames` (case-insensitive).
    fn sent_by(&mut self, usernames: &[&str]) -> &mut Self;
    fn with_sender(&mut self) -> &mut Self;
    fn without_sender(&mut self) -> &mut Self;
    fn with_text(&mut self, texts: &[&str]) -> &mut Self;
    fn text_contains(&mut self, terms: &[&str]) -> &mut Self;
    fn text_matches<F>(&mut self, matcher: F) -> &mut Self
    where
        F: Fn(&str) -> bool + 'a;
    fn since(&mut self, sequence: i32) -> &mut Self;
    fn latest_sequence(&self) -> i32;
}

impl<'a, T: ChatQueryView> ChatQueryExt<'a, T> for Query<'a, T> {
    fn with_type_(&mut self, types: &[i32]) -> &mut Self {
        let types: Vec<i32> = types.to_vec();
        self.where_(move |v: &T| types.contains(&v.view_if_type()))
    }

    fn sent_by(&mut self, usernames: &[&str]) -> &mut Self {
        let wanted: Vec<String> = usernames.iter().map(|u| normalized(u)).collect();
        self.where_(move |v: &T| {
            v.view_username()
                .is_some_and(|u| wanted.contains(&normalized(u)))
        })
    }

    fn with_sender(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_username().is_some_and(|u| !u.trim().is_empty()))
    }

    fn without_sender(&mut self) -> &mut Self {
        self.where_(|v: &T| v.view_username().is_none_or(|u| u.trim().is_empty()))
    }

    fn with_text(&mut self, texts: &[&str]) -> &mut Self {
        let wanted: Vec<String> = texts.iter().map(|t| normalized(t)).collect();
        self.where_(move |v: &T| wanted.contains(&normalized(v.view_text())))
    }

    fn text_contains(&mut self, terms: &[&str]) -> &mut Self {
        let wanted: Vec<String> = terms.iter().map(|t| normalized(t)).collect();
        self.where_(move |v: &T| {
            let text = normalized(v.view_text());
            !wanted.is_empty() && wanted.iter().any(|term| text.contains(term.as_str()))
        })
    }

    fn text_matches<F>(&mut self, matcher: F) -> &mut Self
    where
        F: Fn(&str) -> bool + 'a,
    {
        self.where_(move |v: &T| matcher(v.view_text()))
    }

    fn since(&mut self, sequence: i32) -> &mut Self {
        self.where_(move |v: &T| v.view_sequence() > sequence)
    }

    fn latest_sequence(&self) -> i32 {
        self.results()
            .into_iter()
            .map(|v| v.view_sequence())
            .max()
            .unwrap_or(0)
    }
}

// --- SceneQuery -----------------------------------------------------------

mod reach;
pub use reach::*;

// --- widget_search / loc_approach -----------------------------------------

pub mod widget_search;

pub mod loc_approach;

/// The view for a slot index, if that slot was live in the last rebuild.
pub fn npc_by_index(npcs: &[NpcView], index: usize) -> Option<&NpcView> {
    npcs.iter().find(|view| view.index == index)
}

/// Live views standing on a tile.
pub fn npcs_at(npcs: &[NpcView], x: i32, z: i32) -> impl Iterator<Item = &NpcView> {
    npcs.iter().filter(move |view| view.x == x && view.z == z)
}
