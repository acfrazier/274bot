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
/// `chebyshevDistance`).
fn chebyshev_to(a: WorldTile, b: WorldTile) -> i32 {
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

/// Options for `SceneQuery::can_reach` (the m8aq `SceneReachOptions`).
#[derive(Debug, Clone, Copy, Default)]
pub struct SceneReachOptions {
    /// Max BFS expansions; the m8aq default is 400 when `None`.
    pub max_steps: Option<u32>,
    /// Allow stopping one tile away from a blocked destination.
    pub adjacent_ok: bool,
}

/// The eight walk directions (N/E/S/W then the diagonals), the m8aq
/// `DIRS`.
const DIRS: [(i32, i32); 8] = [
    (-1, 0),
    (1, 0),
    (0, -1),
    (0, 1),
    (-1, -1),
    (1, -1),
    (-1, 1),
    (1, 1),
];

fn local_in_bounds(scene: &SceneView, tile: LocalTile) -> bool {
    tile.lx >= 0 && tile.lz >= 0 && tile.lx < scene.width && tile.lz < scene.height
}

/// `f & mask == 0` over a missing flag (out of bounds reads as closed).
fn flags_open(flags: Option<i32>, mask: i32) -> bool {
    flags.is_some_and(|f| f & mask == 0)
}

/// Whether one step from `(lx, lz)` by `(dx, dz)` is clear: orthogonal
/// steps check the destination's player-walk mask, diagonals additionally
/// require both orthogonal legs (the m8aq `canStepLocal`).
fn can_step_local(
    flags: &dyn Fn(i32, i32) -> Option<i32>,
    lx: i32,
    lz: i32,
    dx: i32,
    dz: i32,
) -> bool {
    let nx = lx + dx;
    let nz = lz + dz;
    if dx == 0 && dz == 0 {
        return false;
    }
    if dx == 0 || dz == 0 {
        if dx == -1 {
            return flags_open(flags(nx, nz), CollisionFlag::PL_WALK_E);
        }
        if dx == 1 {
            return flags_open(flags(nx, nz), CollisionFlag::PL_WALK_W);
        }
        if dz == -1 {
            return flags_open(flags(nx, nz), CollisionFlag::PL_WALK_N);
        }
        return flags_open(flags(nx, nz), CollisionFlag::PL_WALK_S);
    }
    if dx == -1 && dz == -1 {
        return flags_open(flags(nx, nz), CollisionFlag::PL_WALK_NE)
            && can_step_local(flags, lx, lz, -1, 0)
            && can_step_local(flags, lx, lz, 0, -1);
    }
    if dx == 1 && dz == -1 {
        return flags_open(flags(nx, nz), CollisionFlag::PL_WALK_NW)
            && can_step_local(flags, lx, lz, 1, 0)
            && can_step_local(flags, lx, lz, 0, -1);
    }
    if dx == -1 && dz == 1 {
        return flags_open(flags(nx, nz), CollisionFlag::PL_WALK_SE)
            && can_step_local(flags, lx, lz, -1, 0)
            && can_step_local(flags, lx, lz, 0, 1);
    }
    flags_open(flags(nx, nz), CollisionFlag::PL_WALK_SW)
        && can_step_local(flags, lx, lz, 1, 0)
        && can_step_local(flags, lx, lz, 0, 1)
}

/// Whether the destination tile's wall mask toward `(dx, dz)` is clear
/// (the `adjacentOk` stop check).
fn can_reach_adjacent_tile(
    flags: &dyn Fn(i32, i32) -> Option<i32>,
    nx: i32,
    nz: i32,
    dx: i32,
    dz: i32,
) -> bool {
    let Some(f) = flags(nx, nz) else {
        return false;
    };
    let wall_mask = match (dx, dz) {
        (-1, 0) => CollisionFlag::W_E,
        (1, 0) => CollisionFlag::W_W,
        (0, -1) => CollisionFlag::W_N,
        (0, 1) => CollisionFlag::W_S,
        (-1, -1) => CollisionFlag::W_NE | CollisionFlag::W_N | CollisionFlag::W_E,
        (1, -1) => CollisionFlag::W_NW | CollisionFlag::W_N | CollisionFlag::W_W,
        (-1, 1) => CollisionFlag::W_SE | CollisionFlag::W_S | CollisionFlag::W_E,
        (1, 1) => CollisionFlag::W_SW | CollisionFlag::W_S | CollisionFlag::W_W,
        _ => return false,
    };
    f & wall_mask == 0
}

/// BFS over `can_step_local` (the m8aq `canReachLocal`).
fn can_reach_local(
    flags: &dyn Fn(i32, i32) -> Option<i32>,
    from: (i32, i32),
    to: (i32, i32),
    max_steps: u32,
    adjacent_ok: bool,
) -> bool {
    if flags(from.0, from.1).is_none() {
        return false;
    }
    let key = |lx: i32, lz: i32| lx * 256 + lz;
    let mut seen: Vec<i32> = vec![key(from.0, from.1)];
    let mut queue: Vec<(i32, i32)> = vec![from];
    let mut head = 0;
    let mut expansions = 0u32;
    while head < queue.len() {
        let cur = queue[head];
        head += 1;
        if cur == to {
            return true;
        }
        if adjacent_ok
            && (cur.0 - to.0).abs() + (cur.1 - to.1).abs() == 1
            && can_reach_adjacent_tile(flags, to.0, to.1, to.0 - cur.0, to.1 - cur.1)
        {
            return true;
        }
        expansions += 1;
        if expansions > max_steps {
            return false;
        }
        for (dx, dz) in DIRS {
            let k = key(cur.0 + dx, cur.1 + dz);
            if !seen.contains(&k) && can_step_local(flags, cur.0, cur.1, dx, dz) {
                seen.push(k);
                queue.push((cur.0 + dx, cur.1 + dz));
            }
        }
    }
    false
}

/// The built scene's collision surface plus the local player's tile
/// (the m8aq `SceneQuery`): tile mapping, flag reads, walkability and
/// reach.
pub struct SceneQuery<'a>(&'a SceneView, Option<WorldTile>);

impl<'a> SceneQuery<'a> {
    pub fn new(scene: &'a SceneView, player_tile: Option<WorldTile>) -> Self {
        SceneQuery(scene, player_tile)
    }

    /// Whether `tile` lies inside the built scene on its level.
    pub fn contains(&self, tile: WorldTile) -> bool {
        let scene = self.0;
        if !scene.available || tile.level != scene.level {
            return false;
        }
        let lx = tile.x - scene.base_x;
        let lz = tile.z - scene.base_z;
        lx >= 0 && lz >= 0 && lx < scene.width && lz < scene.height
    }

    /// The scene's build origin (m8aq `base()`).
    pub fn base(&self) -> WorldTile {
        WorldTile {
            x: self.0.base_x,
            z: self.0.base_z,
            level: self.0.level,
        }
    }

    /// The scene-local tile, `None` when the world tile is outside the
    /// scene (or on another level).
    pub fn to_local(&self, tile: WorldTile) -> Option<LocalTile> {
        if !self.contains(tile) {
            return None;
        }
        Some(LocalTile {
            lx: tile.x - self.0.base_x,
            lz: tile.z - self.0.base_z,
        })
    }

    /// The world tile of a scene-local tile, `None` when out of bounds
    /// (or the scene is unavailable).
    pub fn to_world(&self, tile: LocalTile) -> Option<WorldTile> {
        let scene = self.0;
        if !scene.available || !local_in_bounds(scene, tile) {
            return None;
        }
        Some(WorldTile {
            x: scene.base_x + tile.lx,
            z: scene.base_z + tile.lz,
            level: scene.level,
        })
    }

    /// The packed collision flags of a world tile, `None` when it is
    /// outside the scene.
    pub fn collision_at(&self, tile: WorldTile) -> Option<i32> {
        let local = self.to_local(tile)?;
        self.collision_at_local(local)
    }

    /// The packed collision flags of a scene-local tile, `None` when it
    /// is outside the scene.
    pub fn collision_at_local(&self, tile: LocalTile) -> Option<i32> {
        let scene = self.0;
        if !scene.available || !local_in_bounds(scene, tile) {
            return None;
        }
        scene
            .collision_flags
            .get((tile.lx * scene.height + tile.lz) as usize)
            .copied()
    }

    /// Whether the tile's flags are known (in-scene).
    pub fn probeable(&self, tile: WorldTile) -> bool {
        self.collision_at(tile).is_some()
    }

    /// Whether no walk-blocking flag bit is set on the tile. The client
    /// has no single `WALK_BLOCKED` const; the m8aq reference reads the
    /// packed `SQ_BLOCKED` mask (walk scenery + blocked ground/npcs).
    pub fn walkable(&self, tile: WorldTile) -> bool {
        self.collision_at(tile)
            .is_some_and(|flags| flags & CollisionFlag::SQ_BLOCKED == 0)
    }

    /// Whether one adjacent step from `from` to `to` is clear (level and
    /// adjacency checked; diagonal steps need both orthogonal legs).
    pub fn can_step(&self, from: WorldTile, to: WorldTile) -> bool {
        if from.level != to.level {
            return false;
        }
        let dx = to.x - from.x;
        let dz = to.z - from.z;
        if dx.abs().max(dz.abs()) != 1 {
            return false;
        }
        let (Some(from_local), Some(_)) = (self.to_local(from), self.to_local(to)) else {
            return false;
        };
        let flags = |lx: i32, lz: i32| self.collision_at_local(LocalTile { lx, lz });
        can_step_local(&flags, from_local.lx, from_local.lz, dx, dz)
    }

    /// Whether the player can interact with `loc` from `from`; `None`
    /// when the loc's shape has no known approach model.
    pub fn can_operate_from(&self, loc: &LocView, from: WorldTile) -> Option<bool> {
        loc_approach::can_operate_from(loc, self.0, from)
    }

    /// The world tiles the player can operate `loc` from; `None` when
    /// the loc's shape has no known approach model.
    pub fn operable_tiles(&self, loc: &LocView) -> Option<Vec<WorldTile>> {
        loc_approach::operable_tiles(loc, self.0)
    }

    /// Whether the local player can walk to `destination` (BFS with a
    /// step budget; `adjacent_ok` stops one tile short).
    pub fn can_reach(&self, destination: WorldTile, options: &SceneReachOptions) -> bool {
        let Some(player) = self.1 else {
            return false;
        };
        if player.level != destination.level {
            return false;
        }
        let (Some(from), Some(to)) = (self.to_local(player), self.to_local(destination)) else {
            return false;
        };
        let flags = |lx: i32, lz: i32| self.collision_at_local(LocalTile { lx, lz });
        can_reach_local(
            &flags,
            (from.lx, from.lz),
            (to.lx, to.lz),
            options.max_steps.unwrap_or(400),
            options.adjacent_ok,
        )
    }
}

// --- widget_search / loc_approach -----------------------------------------

pub mod widget_search {
    //! Snapshot-level button lookups (the m8aq `WidgetSearch.ts`).

    use crate::snapshot::{GameSnapshot, WidgetView};

    /// One combat-style button paired with the nearest text label
    /// (the m8aq `combatStyleLabels` row).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CombatStyleLabel {
        pub mode: i32,
        pub label: String,
        pub component_id: i32,
    }

    /// The open roots' and side tabs' widgets under `root_component_id`.
    fn all_widgets(
        snapshot: &GameSnapshot,
        root_component_id: i32,
    ) -> impl Iterator<Item = &WidgetView> + '_ {
        snapshot
            .widgets()
            .iter()
            .chain(
                snapshot
                    .side_tabs()
                    .iter()
                    .flat_map(|tab| tab.widgets.iter()),
            )
            .filter(move |w| w.root_component_id == root_component_id)
    }

    /// The BUTTON_CLOSE component of the root, -1 when none is open.
    pub fn close_button_com_id(snapshot: &GameSnapshot, root_component_id: i32) -> i32 {
        all_widgets(snapshot, root_component_id)
            .find(|w| w.button_type == 3)
            .map(|w| w.component_id)
            .unwrap_or(-1)
    }

    /// The component whose button text equals `label` (trimmed,
    /// case-insensitive), -1 when none matches.
    pub fn button_by_text(snapshot: &GameSnapshot, root_component_id: i32, label: &str) -> i32 {
        let wanted = label.trim().to_ascii_lowercase();
        all_widgets(snapshot, root_component_id)
            .find(|w| {
                w.button_text
                    .as_deref()
                    .is_some_and(|t| t.trim().to_ascii_lowercase() == wanted)
            })
            .map(|w| w.component_id)
            .unwrap_or(-1)
    }

    /// The BUTTON_TARGET component whose target base equals `base`
    /// (trimmed, case-insensitive), -1 when none matches.
    pub fn target_button_by_base(
        snapshot: &GameSnapshot,
        root_component_id: i32,
        base: &str,
    ) -> i32 {
        let wanted = base.trim().to_ascii_lowercase();
        all_widgets(snapshot, root_component_id)
            .find(|w| {
                w.button_type == 2
                    && w.target_base
                        .as_deref()
                        .is_some_and(|t| t.trim().to_ascii_lowercase() == wanted)
            })
            .map(|w| w.component_id)
            .unwrap_or(-1)
    }

    /// The BUTTON_SELECT component bound to `varp` = `value`, -1 when
    /// none matches.
    pub fn select_button_by_varp(
        snapshot: &GameSnapshot,
        root_component_id: i32,
        varp: i32,
        value: i32,
    ) -> i32 {
        all_widgets(snapshot, root_component_id)
            .find(|w| {
                w.button_type == 5
                    && w.varp_bindings
                        .iter()
                        .any(|b| b.varp == varp && b.value == Some(value))
            })
            .map(|w| w.component_id)
            .unwrap_or(-1)
    }

    /// True when `label` is one of the melee style names the isolate matches
    /// (`Accurate` / `Aggressive` / `Controlled` / `Defensive`), ignoring
    /// surrounding whitespace and parentheses. 274 combat IFs put that name
    /// on the same row as Punch/Kick; nearest-y alone would post the action.
    fn is_melee_style_label(label: &str) -> bool {
        let n = label
            .trim()
            .trim_matches(|c: char| c == '(' || c == ')')
            .trim()
            .to_ascii_lowercase();
        matches!(
            n.as_str(),
            "accurate" | "aggressive" | "controlled" | "defensive"
        )
    }

    fn nearest_text<'a>(texts: &[(&'a str, i32)], y: i32) -> Option<&'a str> {
        texts
            .iter()
            .min_by_key(|(_, ty)| (*ty - y).abs())
            .map(|(t, _)| *t)
    }

    /// The varp-select buttons of the root with their nearest text
    /// labels, sorted by mode (the m8aq default varp is 43). Prefers a
    /// melee style name already on the IF over a closer action name.
    pub fn combat_style_labels(
        snapshot: &GameSnapshot,
        root_component_id: i32,
        varp: i32,
    ) -> Vec<CombatStyleLabel> {
        let widgets: Vec<&WidgetView> = all_widgets(snapshot, root_component_id).collect();
        let buttons: Vec<(i32, i32, i32)> = widgets
            .iter()
            .filter(|w| w.button_type == 5 && w.varp_bindings.iter().any(|b| b.varp == varp))
            .map(|w| {
                let mode = w
                    .varp_bindings
                    .iter()
                    .find(|b| b.varp == varp)
                    .and_then(|b| b.value)
                    .unwrap_or(0);
                (w.component_id, mode, w.y)
            })
            .collect();
        let texts: Vec<(&str, i32)> = widgets
            .iter()
            .filter_map(|w| {
                w.text
                    .as_deref()
                    .filter(|t| !t.is_empty())
                    .map(|t| (t, w.y))
            })
            .collect();
        let style_texts: Vec<(&str, i32)> = texts
            .iter()
            .copied()
            .filter(|(t, _)| is_melee_style_label(t))
            .collect();
        let mut out: Vec<CombatStyleLabel> = buttons
            .into_iter()
            .map(|(component_id, mode, y)| {
                let label = nearest_text(&style_texts, y)
                    .or_else(|| nearest_text(&texts, y))
                    .unwrap_or_default()
                    .to_string();
                CombatStyleLabel {
                    mode,
                    label,
                    component_id,
                }
            })
            .collect();
        out.sort_by_key(|l| l.mode);
        out
    }
}

pub mod loc_approach {
    //! Where a footprint loc can be operated from (the m8aq
    //! `LocApproach.ts`): a 4-bit force-approach mask rotated by the
    //! loc's angle, checked against the scene's directional wall flags.

    use crate::snapshot::{LocView, SceneView, WorldTile};
    use client::dash3d::CollisionFlag;

    /// The loc shapes with a real footprint (the m8aq
    /// `FOOTPRINT_SHAPES`); other shapes read as "don't know".
    const FOOTPRINT_SHAPES: [i32; 3] = [10, 11, 22];

    const FORCE_NORTH: i32 = 0x1;
    const FORCE_EAST: i32 = 0x2;
    const FORCE_SOUTH: i32 = 0x4;
    const FORCE_WEST: i32 = 0x8;

    /// Rotate the 4-bit force-approach mask by the loc's angle (the m8aq
    /// `rotateForceApproach`).
    fn rotate_force_approach(force_approach: i32, angle: i32) -> i32 {
        if angle == 0 {
            return force_approach;
        }
        ((force_approach << angle) & 0xf) | (force_approach >> (4 - angle))
    }

    /// The flags of a local tile; out of bounds (or a missing flag) reads
    /// as fully blocked.
    fn collision_at(scene: &SceneView, lx: i32, lz: i32) -> i32 {
        if lx < 0 || lz < 0 || lx >= scene.width || lz >= scene.height {
            return CollisionFlag::SQ_BLOCKED;
        }
        scene
            .collision_flags
            .get((lx * scene.height + lz) as usize)
            .copied()
            .unwrap_or(CollisionFlag::SQ_BLOCKED)
    }

    /// Whether `src` can interact with the footprint `dst..dst+size`:
    /// standing inside it, or adjacent on a side whose wall flag and
    /// force-approach bit are clear (the m8aq `testLoc`).
    #[allow(clippy::too_many_arguments)]
    fn test_loc(
        src_x: i32,
        src_z: i32,
        dst_x: i32,
        dst_z: i32,
        size_x: i32,
        size_z: i32,
        force_approach: i32,
        scene: &SceneView,
    ) -> bool {
        let max_x = dst_x + size_x - 1;
        let max_z = dst_z + size_z - 1;

        if src_x >= dst_x && src_x <= max_x && src_z >= dst_z && src_z <= max_z {
            return true;
        }
        if src_x == dst_x - 1
            && src_z >= dst_z
            && src_z <= max_z
            && (collision_at(scene, src_x, src_z) & CollisionFlag::W_E) == 0
            && (force_approach & FORCE_WEST) == 0
        {
            return true;
        }
        if src_x == max_x + 1
            && src_z >= dst_z
            && src_z <= max_z
            && (collision_at(scene, src_x, src_z) & CollisionFlag::W_W) == 0
            && (force_approach & FORCE_EAST) == 0
        {
            return true;
        }
        if src_z == dst_z - 1
            && src_x >= dst_x
            && src_x <= max_x
            && (collision_at(scene, src_x, src_z) & CollisionFlag::W_N) == 0
            && (force_approach & FORCE_SOUTH) == 0
        {
            return true;
        }
        if src_z == max_z + 1
            && src_x >= dst_x
            && src_x <= max_x
            && (collision_at(scene, src_x, src_z) & CollisionFlag::W_S) == 0
            && (force_approach & FORCE_NORTH) == 0
        {
            return true;
        }
        false
    }

    /// Whether `from` can operate `loc` in `scene`; `None` when the
    /// loc's shape has no approach model (or the scene/level don't line
    /// up), `Some(false)` when the tile is out of bounds or blocked.
    pub fn can_operate_from(loc: &LocView, scene: &SceneView, from: WorldTile) -> Option<bool> {
        if !FOOTPRINT_SHAPES.contains(&loc.shape) {
            return None;
        }
        if !scene.available || from.level != scene.level || loc.tile.level != scene.level {
            return None;
        }
        let src_x = from.x - scene.base_x;
        let src_z = from.z - scene.base_z;
        if src_x < 0 || src_z < 0 || src_x >= scene.width || src_z >= scene.height {
            return Some(false);
        }
        let dst_x = loc.tile.x - scene.base_x;
        let dst_z = loc.tile.z - scene.base_z;
        let force_approach = rotate_force_approach(loc.force_approach, loc.angle);
        Some(test_loc(
            src_x,
            src_z,
            dst_x,
            dst_z,
            loc.footprint_width,
            loc.footprint_length,
            force_approach,
            scene,
        ))
    }

    /// Every walkable tile from which `loc` can be operated; `None` when
    /// the loc's shape has no approach model.
    pub fn operable_tiles(loc: &LocView, scene: &SceneView) -> Option<Vec<WorldTile>> {
        if !FOOTPRINT_SHAPES.contains(&loc.shape) {
            return None;
        }
        if !scene.available || loc.tile.level != scene.level {
            return None;
        }
        let dst_x = loc.tile.x - scene.base_x;
        let dst_z = loc.tile.z - scene.base_z;
        let force_approach = rotate_force_approach(loc.force_approach, loc.angle);
        let size_x = loc.footprint_width;
        let size_z = loc.footprint_length;
        let mut tiles = Vec::new();
        for lx in (dst_x - 1).max(0)..=(dst_x + size_x).min(scene.width - 1) {
            for lz in (dst_z - 1).max(0)..=(dst_z + size_z).min(scene.height - 1) {
                if collision_at(scene, lx, lz) & CollisionFlag::SQ_BLOCKED != 0 {
                    continue;
                }
                if test_loc(lx, lz, dst_x, dst_z, size_x, size_z, force_approach, scene) {
                    tiles.push(WorldTile {
                        x: scene.base_x + lx,
                        z: scene.base_z + lz,
                        level: scene.level,
                    });
                }
            }
        }
        Some(tiles)
    }
}

/// The view for a slot index, if that slot was live in the last rebuild.
pub fn npc_by_index(npcs: &[NpcView], index: usize) -> Option<&NpcView> {
    npcs.iter().find(|view| view.index == index)
}

/// Live views standing on a tile.
pub fn npcs_at(npcs: &[NpcView], x: i32, z: i32) -> impl Iterator<Item = &NpcView> {
    npcs.iter().filter(move |view| view.x == x && view.z == z)
}
