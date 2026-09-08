//! Opt-in A1 rebuild-edge exact content dedup for Widgets, SideTabs, and Loc.
//!
//! Feature `snapshot-dedup` enables Arc family bodies and slot-local weak
//! registry interning after the original rebuild edge. Feature-off keeps the
//! original Vec storage path with no equality or registry work.
//!
//! Per-slot lifetime only: owners register cursors, weak entries clean on
//! drop/restart, no global cross-client body cache, no strong history.

use crate::snapshot::{LocView, SideTabView, WidgetView};
use std::sync::{Arc, Mutex, Weak};

/// Maximum simultaneous owner cursors expected per slot (host, host-play, UI).
/// Dynamic registration may exceed this; it is only a sizing hint.
pub const MAX_CURSORS_HINT: usize = 8;

/// Opaque cursor identity within one [`SlotFamilyRegistry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CursorId(u32);

impl CursorId {
    pub fn raw(self) -> u32 {
        self.0
    }
}

/// Per-family counters (optional diagnostics; not required on quiet feature-on).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FamilyCounters {
    pub walks: u64,
    pub quiet_skips: u64,
    pub equality_comparisons: u64,
    pub equality_hits: u64,
    pub equality_misses: u64,
    pub publishes: u64,
}

/// Aggregate counters across the three candidate families.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DedupCounters {
    pub widgets: FamilyCounters,
    pub side_tabs: FamilyCounters,
    pub loc: FamilyCounters,
}

impl DedupCounters {
    pub fn equality_comparisons(&self) -> u64 {
        self.widgets.equality_comparisons
            + self.side_tabs.equality_comparisons
            + self.loc.equality_comparisons
    }

    pub fn walks(&self) -> u64 {
        self.widgets.walks + self.side_tabs.walks + self.loc.walks
    }

    pub fn equality_hits(&self) -> u64 {
        self.widgets.equality_hits + self.side_tabs.equality_hits + self.loc.equality_hits
    }

    pub fn equality_misses(&self) -> u64 {
        self.widgets.equality_misses + self.side_tabs.equality_misses + self.loc.equality_misses
    }
}

/// Allocation-style accounting in bytes (not RSS).
///
/// Exclusive capacity includes spare `Vec` capacity on unique bodies.
/// Arc header uses the real `ArcInner` layout estimate (strong + weak + data).
/// Weak slots are counted once per live cursor registration, not double-counted
/// against payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct AllocationAccount {
    pub old_per_owner_payload_bytes: usize,
    pub unique_body_payload_bytes: usize,
    /// Sum of unique payloads replicated once per holder that would own a private copy.
    pub duplicate_nested_payload_bytes: usize,
    pub scratch_peak_bytes: usize,
    pub registry_metadata_bytes: usize,
    pub live_owner_count: usize,
    pub unique_body_count: usize,
    pub arc_header_bytes: usize,
    pub weak_slot_bytes: usize,
    pub equality_comparisons: u64,
    pub equality_hits: u64,
    pub equality_misses: u64,
    pub walks: u64,
    pub quiet_skips: u64,
    pub publishes: u64,
}

/// Real Arc allocation header estimate: strong count, weak count, and data.
#[inline]
pub fn arc_inner_header_bytes() -> usize {
    // Layout matches std's ArcInner { strong, weak, data } on the heap.
    std::mem::size_of::<usize>() * 2 + std::mem::align_of::<usize>()
}

struct CursorSlot {
    id: CursorId,
    widgets: Option<Weak<Vec<WidgetView>>>,
    side_tabs: Option<Weak<Vec<SideTabView>>>,
    loc: Option<Weak<Vec<LocView>>>,
}

/// Slot-local weak registry of currently published family bodies.
///
/// Bound by live registered cursors for this slot instance. Overwrite on
/// replacement; unregister on cursor drop/restart; dead weaks ignored.
#[derive(Default)]
pub struct SlotFamilyRegistry {
    next_id: u32,
    cursors: Vec<CursorSlot>,
    /// Aggregate equality/walk counters for this instance (memory-profile).
    /// Updated on intern and optional quiet-skip bumps; not a strong history.
    counters: DedupCounters,
}

impl SlotFamilyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new owner cursor. Returns a stable id for this registry.
    pub fn register(&mut self) -> CursorId {
        let id = CursorId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.cursors.push(CursorSlot {
            id,
            widgets: None,
            side_tabs: None,
            loc: None,
        });
        id
    }

    /// Clear one cursor's weak entries and remove its registration.
    pub fn unregister(&mut self, cursor_id: CursorId) {
        self.cursors.retain(|c| c.id != cursor_id);
    }

    pub fn live_owner_count(&self) -> usize {
        self.cursors.len()
    }

    pub fn counters(&self) -> &DedupCounters {
        &self.counters
    }

    pub fn counters_mut(&mut self) -> &mut DedupCounters {
        &mut self.counters
    }

    /// Quiet gate miss (no equality work). Feature-on only; memory-profile
    /// consumers read the aggregate. Does not walk bodies.
    pub fn bump_quiet_skip_widgets(&mut self) {
        self.counters.widgets.quiet_skips += 1;
    }

    pub fn bump_quiet_skip_side_tabs(&mut self) {
        self.counters.side_tabs.quiet_skips += 1;
    }

    pub fn bump_quiet_skip_loc(&mut self) {
        self.counters.loc.quiet_skips += 1;
    }

    fn slot_mut(&mut self, cursor_id: CursorId) -> Option<&mut CursorSlot> {
        self.cursors.iter_mut().find(|c| c.id == cursor_id)
    }

    fn clean_widgets(&mut self) {
        for c in &mut self.cursors {
            if let Some(w) = &c.widgets {
                if w.strong_count() == 0 {
                    c.widgets = None;
                }
            }
        }
    }

    fn clean_side_tabs(&mut self) {
        for c in &mut self.cursors {
            if let Some(w) = &c.side_tabs {
                if w.strong_count() == 0 {
                    c.side_tabs = None;
                }
            }
        }
    }

    fn clean_loc(&mut self) {
        for c in &mut self.cursors {
            if let Some(w) = &c.loc {
                if w.strong_count() == 0 {
                    c.loc = None;
                }
            }
        }
    }

    /// Intern a completed widgets body. Exact equality only.
    pub fn intern_widgets(
        &mut self,
        cursor_id: CursorId,
        candidate: Vec<WidgetView>,
        counters: &mut FamilyCounters,
    ) -> Arc<Vec<WidgetView>> {
        self.clean_widgets();
        for c in &self.cursors {
            if c.id == cursor_id {
                continue;
            }
            if let Some(weak) = &c.widgets {
                if let Some(existing) = weak.upgrade() {
                    counters.equality_comparisons += 1;
                    self.counters.widgets.equality_comparisons += 1;
                    if widgets_eq(existing.as_slice(), candidate.as_slice()) {
                        counters.equality_hits += 1;
                        self.counters.widgets.equality_hits += 1;
                        if let Some(slot) = self.slot_mut(cursor_id) {
                            slot.widgets = Some(Arc::downgrade(&existing));
                        }
                        return existing;
                    }
                    counters.equality_misses += 1;
                    self.counters.widgets.equality_misses += 1;
                }
            }
        }
        counters.publishes += 1;
        self.counters.widgets.publishes += 1;
        let arc = Arc::new(candidate);
        if let Some(slot) = self.slot_mut(cursor_id) {
            slot.widgets = Some(Arc::downgrade(&arc));
        }
        arc
    }

    pub fn intern_side_tabs(
        &mut self,
        cursor_id: CursorId,
        candidate: Vec<SideTabView>,
        counters: &mut FamilyCounters,
    ) -> Arc<Vec<SideTabView>> {
        self.clean_side_tabs();
        for c in &self.cursors {
            if c.id == cursor_id {
                continue;
            }
            if let Some(weak) = &c.side_tabs {
                if let Some(existing) = weak.upgrade() {
                    counters.equality_comparisons += 1;
                    self.counters.side_tabs.equality_comparisons += 1;
                    if side_tabs_eq(existing.as_slice(), candidate.as_slice()) {
                        counters.equality_hits += 1;
                        self.counters.side_tabs.equality_hits += 1;
                        if let Some(slot) = self.slot_mut(cursor_id) {
                            slot.side_tabs = Some(Arc::downgrade(&existing));
                        }
                        return existing;
                    }
                    counters.equality_misses += 1;
                    self.counters.side_tabs.equality_misses += 1;
                }
            }
        }
        counters.publishes += 1;
        self.counters.side_tabs.publishes += 1;
        let arc = Arc::new(candidate);
        if let Some(slot) = self.slot_mut(cursor_id) {
            slot.side_tabs = Some(Arc::downgrade(&arc));
        }
        arc
    }

    pub fn intern_loc(
        &mut self,
        cursor_id: CursorId,
        candidate: Vec<LocView>,
        counters: &mut FamilyCounters,
    ) -> Arc<Vec<LocView>> {
        self.clean_loc();
        for c in &self.cursors {
            if c.id == cursor_id {
                continue;
            }
            if let Some(weak) = &c.loc {
                if let Some(existing) = weak.upgrade() {
                    counters.equality_comparisons += 1;
                    self.counters.loc.equality_comparisons += 1;
                    if locs_eq(existing.as_slice(), candidate.as_slice()) {
                        counters.equality_hits += 1;
                        self.counters.loc.equality_hits += 1;
                        if let Some(slot) = self.slot_mut(cursor_id) {
                            slot.loc = Some(Arc::downgrade(&existing));
                        }
                        return existing;
                    }
                    counters.equality_misses += 1;
                    self.counters.loc.equality_misses += 1;
                }
            }
        }
        counters.publishes += 1;
        self.counters.loc.publishes += 1;
        let arc = Arc::new(candidate);
        if let Some(slot) = self.slot_mut(cursor_id) {
            slot.loc = Some(Arc::downgrade(&arc));
        }
        arc
    }

    pub fn metadata_bytes(&self) -> usize {
        self.cursors.len() * std::mem::size_of::<CursorSlot>()
    }

    pub fn unique_widget_bodies(&self) -> Vec<Arc<Vec<WidgetView>>> {
        unique_arcs(self.cursors.iter().filter_map(|c| c.widgets.as_ref()))
    }

    pub fn unique_side_tab_bodies(&self) -> Vec<Arc<Vec<SideTabView>>> {
        unique_arcs(self.cursors.iter().filter_map(|c| c.side_tabs.as_ref()))
    }

    pub fn unique_loc_bodies(&self) -> Vec<Arc<Vec<LocView>>> {
        unique_arcs(self.cursors.iter().filter_map(|c| c.loc.as_ref()))
    }
}

fn unique_arcs<'a, T: 'a>(
    weaks: impl Iterator<Item = &'a Weak<Vec<T>>>,
) -> Vec<Arc<Vec<T>>> {
    let mut out = Vec::new();
    for w in weaks {
        if let Some(a) = w.upgrade() {
            if !out.iter().any(|e| Arc::ptr_eq(e, &a)) {
                out.push(a);
            }
        }
    }
    out
}

/// Shared handle: one registry Arc per slot lifetime, plus this owner's cursor.
#[derive(Clone)]
pub struct DedupHandle {
    pub registry: Arc<Mutex<SlotFamilyRegistry>>,
    pub cursor: CursorId,
}

impl DedupHandle {
    pub fn new(registry: Arc<Mutex<SlotFamilyRegistry>>, cursor: CursorId) -> Self {
        Self { registry, cursor }
    }

    /// Create a fresh registry and register the first cursor.
    pub fn new_slot_owner() -> Self {
        SlotDedupInstance::new().attach()
    }

    /// Register an additional owner on an existing slot registry.
    pub fn additional_owner(registry: &Arc<Mutex<SlotFamilyRegistry>>) -> Self {
        let cursor = registry.lock().unwrap().register();
        Self {
            registry: Arc::clone(registry),
            cursor,
        }
    }

    pub fn unregister(&self) {
        if let Ok(mut g) = self.registry.lock() {
            g.unregister(self.cursor);
        }
    }
}

/// Slot-instance token: one weak family registry for the lifetime of a single
/// slot run. Clone among that instance's real owners (host SlotLoop, host-play
/// nav_snapshot, panel nav_states). Not keyed by username — two concurrent
/// instances with the same display name hold independent tokens.
///
/// Dropping every clone releases the last strong refs to the registry once
/// owner cursors unregister; there is no process-wide name table.
#[derive(Clone)]
pub struct SlotDedupInstance {
    registry: Arc<Mutex<SlotFamilyRegistry>>,
}

impl SlotDedupInstance {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(Mutex::new(SlotFamilyRegistry::new())),
        }
    }

    /// Register a new owner cursor on this instance's registry.
    pub fn attach(&self) -> DedupHandle {
        DedupHandle::additional_owner(&self.registry)
    }

    pub fn registry(&self) -> &Arc<Mutex<SlotFamilyRegistry>> {
        &self.registry
    }

    pub fn live_owner_count(&self) -> usize {
        self.registry.lock().unwrap().live_owner_count()
    }

    /// Same underlying registry (same slot instance).
    pub fn same_instance(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.registry, &other.registry)
    }
}

impl Default for SlotDedupInstance {
    fn default() -> Self {
        Self::new()
    }
}

/// Play-local (or test-local) name → instance map. Not process-global: each
/// [`Play`] / harness owns its own directory so concurrent same-name
/// instances across plays never collide. Preferred production path still
/// passes [`SlotDedupInstance`] tokens through owner boundaries; this map
/// only bridges owners that cannot hold the token directly (panel per-frame).
#[derive(Default)]
pub struct SlotDedupDirectory {
    inner: Mutex<std::collections::HashMap<String, SlotDedupInstance>>,
}

impl SlotDedupDirectory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn install(&self, name: &str, instance: SlotDedupInstance) {
        self.inner
            .lock()
            .unwrap()
            .insert(name.to_string(), instance);
    }

    pub fn get(&self, name: &str) -> Option<SlotDedupInstance> {
        self.inner.lock().unwrap().get(name).cloned()
    }

    pub fn remove(&self, name: &str) -> Option<SlotDedupInstance> {
        self.inner.lock().unwrap().remove(name)
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Snapshot of live instances for memory-profile publication.
    pub fn instances(&self) -> Vec<(String, SlotDedupInstance)> {
        self.inner
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

/// Total field equality for WidgetView slices (PartialEq already derived).
pub fn widgets_eq(a: &[WidgetView], b: &[WidgetView]) -> bool {
    a == b
}

pub fn side_tabs_eq(a: &[SideTabView], b: &[SideTabView]) -> bool {
    a == b
}

/// Explicit total field comparator for LocView (no derived PartialEq).
pub fn loc_eq(a: &LocView, b: &LocView) -> bool {
    a.typecode == b.typecode
        && a.info == b.info
        && a.id == b.id
        && a.name == b.name
        && a.description == b.description
        && a.actions == b.actions
        && a.tile == b.tile
        && a.distance == b.distance
        && a.layer == b.layer
        && a.shape == b.shape
        && a.angle == b.angle
        && a.width == b.width
        && a.length == b.length
        && a.footprint_width == b.footprint_width
        && a.footprint_length == b.footprint_length
        && a.block_walk == b.block_walk
        && a.block_range == b.block_range
        && a.active == b.active
        && a.animation == b.animation
        && a.map_function == b.map_function
        && a.map_scene == b.map_scene
        && a.force_approach == b.force_approach
}

pub fn locs_eq(a: &[LocView], b: &[LocView]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| loc_eq(x, y))
}

fn opt_str_bytes(s: &Option<String>) -> usize {
    s.as_ref().map(|t| t.capacity()).unwrap_or(0)
}

fn actions_bytes(actions: &[Option<String>]) -> usize {
    actions.iter().map(opt_str_bytes).sum::<usize>()
        + actions.len() * std::mem::size_of::<Option<String>>()
}

/// Nested payload estimate for one widgets body (bytes, not RSS).
pub fn widgets_payload_bytes(body: &[WidgetView]) -> usize {
    let mut n = body.len() * std::mem::size_of::<WidgetView>();
    for w in body {
        n += opt_str_bytes(&w.text);
        n += opt_str_bytes(&w.alternate_text);
        n += opt_str_bytes(&w.button_text);
        n += opt_str_bytes(&w.target_verb);
        n += opt_str_bytes(&w.target_base);
        if let Some(scripts) = &w.scripts {
            n += scripts.capacity() * std::mem::size_of::<Option<Vec<i32>>>();
            for s in scripts {
                if let Some(v) = s {
                    n += v.capacity() * std::mem::size_of::<i32>();
                }
            }
        }
        if let Some(v) = &w.script_comparators {
            n += v.capacity() * std::mem::size_of::<i32>();
        }
        if let Some(v) = &w.script_operands {
            n += v.capacity() * std::mem::size_of::<i32>();
        }
        n += w.varp_bindings.capacity()
            * std::mem::size_of::<crate::snapshot::WidgetVarpBindingView>();
        n += actions_bytes(&w.actions);
        n += w.items.capacity() * std::mem::size_of::<crate::snapshot::ItemView>();
        for item in &w.items {
            n += opt_str_bytes(&item.def.name);
            n += actions_bytes(&item.actions);
        }
    }
    n
}

pub fn widgets_payload_bytes_vec(body: &Vec<WidgetView>) -> usize {
    body.capacity() * std::mem::size_of::<WidgetView>()
        + widgets_payload_bytes(body).saturating_sub(body.len() * std::mem::size_of::<WidgetView>())
}

pub fn side_tabs_payload_bytes(body: &[SideTabView]) -> usize {
    let mut n = body.len() * std::mem::size_of::<SideTabView>();
    for tab in body {
        n += widgets_payload_bytes(&tab.widgets);
        n += tab.widgets.capacity().saturating_sub(tab.widgets.len())
            * std::mem::size_of::<WidgetView>();
    }
    n
}

pub fn side_tabs_payload_bytes_vec(body: &Vec<SideTabView>) -> usize {
    body.capacity() * std::mem::size_of::<SideTabView>()
        + side_tabs_payload_bytes(body)
            .saturating_sub(body.len() * std::mem::size_of::<SideTabView>())
}

pub fn locs_payload_bytes(body: &[LocView]) -> usize {
    let mut n = body.len() * std::mem::size_of::<LocView>();
    for loc in body {
        n += opt_str_bytes(&loc.name);
        n += opt_str_bytes(&loc.description);
        n += actions_bytes(&loc.actions);
    }
    n
}

pub fn locs_payload_bytes_vec(body: &Vec<LocView>) -> usize {
    body.capacity() * std::mem::size_of::<LocView>()
        + locs_payload_bytes(body).saturating_sub(body.len() * std::mem::size_of::<LocView>())
}

/// Compare old per-owner private capacities vs unique shared bodies + overhead.
/// `holders` is a list of (widgets_arc, side_tabs_arc, loc_arc) per live owner.
pub fn account_allocations_arcs(
    holders: &[(
        Arc<Vec<WidgetView>>,
        Arc<Vec<SideTabView>>,
        Arc<Vec<LocView>>,
    )],
    reg: &SlotFamilyRegistry,
    scratch_peak_bytes: usize,
) -> AllocationAccount {
    account_allocations_arcs_with_counters(holders, reg, scratch_peak_bytes, None)
}

/// Like [`account_allocations_arcs`] with optional equality/walk counters.
pub fn account_allocations_arcs_with_counters(
    holders: &[(
        Arc<Vec<WidgetView>>,
        Arc<Vec<SideTabView>>,
        Arc<Vec<LocView>>,
    )],
    reg: &SlotFamilyRegistry,
    scratch_peak_bytes: usize,
    counters: Option<&DedupCounters>,
) -> AllocationAccount {
    let mut old_private = 0usize;
    let mut unique = 0usize;

    let widget_bodies = reg.unique_widget_bodies();
    for body in &widget_bodies {
        let payload = widgets_payload_bytes_vec(body.as_ref());
        unique += payload;
        let n = holders
            .iter()
            .filter(|(w, _, _)| Arc::ptr_eq(w, body))
            .count()
            .max(1);
        old_private += payload * n;
    }
    let side_bodies = reg.unique_side_tab_bodies();
    for body in &side_bodies {
        let payload = side_tabs_payload_bytes_vec(body.as_ref());
        unique += payload;
        let n = holders
            .iter()
            .filter(|(_, s, _)| Arc::ptr_eq(s, body))
            .count()
            .max(1);
        old_private += payload * n;
    }
    let loc_bodies = reg.unique_loc_bodies();
    for body in &loc_bodies {
        let payload = locs_payload_bytes_vec(body.as_ref());
        unique += payload;
        let n = holders
            .iter()
            .filter(|(_, _, l)| Arc::ptr_eq(l, body))
            .count()
            .max(1);
        old_private += payload * n;
    }

    let live = reg.live_owner_count();
    let unique_arc_count = widget_bodies.len() + side_bodies.len() + loc_bodies.len();
    // ArcInner { strong: AtomicUsize, weak: AtomicUsize, data: T } header only
    // (data payload is counted separately in unique_body_payload_bytes).
    let arc_header = unique_arc_count * arc_inner_header_bytes();
    // One Option<Weak<Vec<_>>> per family per live cursor — not per unique body.
    let weak_slot = live * 3 * std::mem::size_of::<Option<Weak<Vec<()>>>>();

    let mut acct = AllocationAccount {
        old_per_owner_payload_bytes: old_private,
        unique_body_payload_bytes: unique,
        duplicate_nested_payload_bytes: old_private.saturating_sub(unique),
        scratch_peak_bytes,
        registry_metadata_bytes: reg.metadata_bytes(),
        live_owner_count: live,
        unique_body_count: unique_arc_count,
        arc_header_bytes: arc_header,
        weak_slot_bytes: weak_slot,
        ..AllocationAccount::default()
    };
    if let Some(c) = counters {
        acct.equality_comparisons = c.equality_comparisons();
        acct.equality_hits = c.equality_hits();
        acct.equality_misses = c.equality_misses();
        acct.walks = c.walks();
        acct.quiet_skips = c.widgets.quiet_skips + c.side_tabs.quiet_skips + c.loc.quiet_skips;
        acct.publishes =
            c.widgets.publishes + c.side_tabs.publishes + c.loc.publishes;
    }
    acct
}

/// Account live unique bodies from a registry without holder arcs (uses
/// strong_count on each unique body as the would-be private replication factor).
pub fn account_registry_live(
    reg: &SlotFamilyRegistry,
    scratch_peak_bytes: usize,
    counters: Option<&DedupCounters>,
) -> AllocationAccount {
    let mut holders: Vec<(
        Arc<Vec<WidgetView>>,
        Arc<Vec<SideTabView>>,
        Arc<Vec<LocView>>,
    )> = Vec::new();
    let empty_w = Arc::new(Vec::new());
    let empty_s = Arc::new(Vec::new());
    let empty_l = Arc::new(Vec::new());
    // Synthesize holder rows from unique bodies × strong_count so old_private
    // reflects live Arc clones without requiring external owner lists.
    let w_bodies = reg.unique_widget_bodies();
    let s_bodies = reg.unique_side_tab_bodies();
    let l_bodies = reg.unique_loc_bodies();
    let max_n = reg.live_owner_count().max(1);
    for i in 0..max_n {
        let w = w_bodies
            .iter()
            .find(|b| Arc::strong_count(b) > i)
            .cloned()
            .unwrap_or_else(|| Arc::clone(&empty_w));
        let s = s_bodies
            .iter()
            .find(|b| Arc::strong_count(b) > i)
            .cloned()
            .unwrap_or_else(|| Arc::clone(&empty_s));
        let l = l_bodies
            .iter()
            .find(|b| Arc::strong_count(b) > i)
            .cloned()
            .unwrap_or_else(|| Arc::clone(&empty_l));
        holders.push((w, s, l));
    }
    if holders.is_empty() {
        holders.push((empty_w, empty_s, empty_l));
    }
    account_allocations_arcs_with_counters(&holders, reg, scratch_peak_bytes, counters)
}

/// Serde-friendly family counter bag for memory-profile JSON.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct FamilyCountersJson {
    pub walks: u64,
    pub quiet_skips: u64,
    pub equality_comparisons: u64,
    pub equality_hits: u64,
    pub equality_misses: u64,
    pub publishes: u64,
}

impl From<&FamilyCounters> for FamilyCountersJson {
    fn from(c: &FamilyCounters) -> Self {
        Self {
            walks: c.walks,
            quiet_skips: c.quiet_skips,
            equality_comparisons: c.equality_comparisons,
            equality_hits: c.equality_hits,
            equality_misses: c.equality_misses,
            publishes: c.publishes,
        }
    }
}

/// One slot-instance row for memory-profile (allocation bytes, not RSS).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct SlotDedupDiagnostic {
    pub slot_name: String,
    pub live_owner_count: usize,
    pub unique_body_count: usize,
    pub unique_body_payload_bytes: usize,
    pub duplicate_nested_payload_bytes: usize,
    pub old_per_owner_payload_bytes: usize,
    pub scratch_peak_bytes: usize,
    pub registry_metadata_bytes: usize,
    pub arc_header_bytes: usize,
    pub weak_slot_bytes: usize,
    pub equality_comparisons: u64,
    pub equality_hits: u64,
    pub equality_misses: u64,
    pub walks: u64,
    pub quiet_skips: u64,
    pub publishes: u64,
    pub widgets: FamilyCountersJson,
    pub side_tabs: FamilyCountersJson,
    pub loc: FamilyCountersJson,
}

impl SlotDedupInstance {
    /// Bounded diagnostic sample for this instance (unique-body accounting only).
    pub fn diagnostic(&self, slot_name: &str, scratch_peak_bytes: usize) -> SlotDedupDiagnostic {
        let reg = self.registry.lock().unwrap();
        let acct = account_registry_live(&reg, scratch_peak_bytes, Some(reg.counters()));
        SlotDedupDiagnostic {
            slot_name: slot_name.to_string(),
            live_owner_count: acct.live_owner_count,
            unique_body_count: acct.unique_body_count,
            unique_body_payload_bytes: acct.unique_body_payload_bytes,
            duplicate_nested_payload_bytes: acct.duplicate_nested_payload_bytes,
            old_per_owner_payload_bytes: acct.old_per_owner_payload_bytes,
            scratch_peak_bytes: acct.scratch_peak_bytes,
            registry_metadata_bytes: acct.registry_metadata_bytes,
            arc_header_bytes: acct.arc_header_bytes,
            weak_slot_bytes: acct.weak_slot_bytes,
            equality_comparisons: acct.equality_comparisons,
            equality_hits: acct.equality_hits,
            equality_misses: acct.equality_misses,
            walks: acct.walks,
            quiet_skips: acct.quiet_skips,
            publishes: acct.publishes,
            widgets: FamilyCountersJson::from(&reg.counters().widgets),
            side_tabs: FamilyCountersJson::from(&reg.counters().side_tabs),
            loc: FamilyCountersJson::from(&reg.counters().loc),
        }
    }
}

impl SlotDedupDirectory {
    /// All live instances as diagnostic rows (memory-profile path).
    pub fn diagnostics(&self, scratch_peak_bytes: usize) -> Vec<SlotDedupDiagnostic> {
        self.instances()
            .into_iter()
            .map(|(name, inst)| inst.diagnostic(&name, scratch_peak_bytes))
            .collect()
    }

    /// Aggregate across the play-local directory for one sample line.
    pub fn aggregate_diagnostic(&self, scratch_peak_bytes: usize) -> AllocationAccount {
        let rows = self.diagnostics(scratch_peak_bytes);
        let mut agg = AllocationAccount::default();
        for r in &rows {
            agg.old_per_owner_payload_bytes += r.old_per_owner_payload_bytes;
            agg.unique_body_payload_bytes += r.unique_body_payload_bytes;
            agg.duplicate_nested_payload_bytes += r.duplicate_nested_payload_bytes;
            agg.scratch_peak_bytes = agg.scratch_peak_bytes.max(r.scratch_peak_bytes);
            agg.registry_metadata_bytes += r.registry_metadata_bytes;
            agg.live_owner_count += r.live_owner_count;
            agg.unique_body_count += r.unique_body_count;
            agg.arc_header_bytes += r.arc_header_bytes;
            agg.weak_slot_bytes += r.weak_slot_bytes;
            agg.equality_comparisons += r.equality_comparisons;
            agg.equality_hits += r.equality_hits;
            agg.equality_misses += r.equality_misses;
            agg.walks += r.walks;
            agg.quiet_skips += r.quiet_skips;
            agg.publishes += r.publishes;
        }
        agg
    }
}
