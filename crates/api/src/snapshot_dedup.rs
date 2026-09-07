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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllocationAccount {
    pub old_per_owner_payload_bytes: usize,
    pub unique_body_payload_bytes: usize,
    pub scratch_peak_bytes: usize,
    pub registry_metadata_bytes: usize,
    pub live_owner_count: usize,
    pub arc_header_bytes: usize,
    pub weak_slot_bytes: usize,
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
                    if widgets_eq(existing.as_slice(), candidate.as_slice()) {
                        counters.equality_hits += 1;
                        if let Some(slot) = self.slot_mut(cursor_id) {
                            slot.widgets = Some(Arc::downgrade(&existing));
                        }
                        return existing;
                    }
                    counters.equality_misses += 1;
                }
            }
        }
        counters.publishes += 1;
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
                    if side_tabs_eq(existing.as_slice(), candidate.as_slice()) {
                        counters.equality_hits += 1;
                        if let Some(slot) = self.slot_mut(cursor_id) {
                            slot.side_tabs = Some(Arc::downgrade(&existing));
                        }
                        return existing;
                    }
                    counters.equality_misses += 1;
                }
            }
        }
        counters.publishes += 1;
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
                    if locs_eq(existing.as_slice(), candidate.as_slice()) {
                        counters.equality_hits += 1;
                        if let Some(slot) = self.slot_mut(cursor_id) {
                            slot.loc = Some(Arc::downgrade(&existing));
                        }
                        return existing;
                    }
                    counters.equality_misses += 1;
                }
            }
        }
        counters.publishes += 1;
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
        let registry = Arc::new(Mutex::new(SlotFamilyRegistry::new()));
        let cursor = registry.lock().unwrap().register();
        Self { registry, cursor }
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

/// Process-wide name→registry map so UI per-frame hooks share the slot thread
/// registry installed at spawn. Entries live only while the slot thread runs.
#[derive(Default)]
pub struct SlotDedupTable {
    inner: Mutex<std::collections::HashMap<String, Arc<Mutex<SlotFamilyRegistry>>>>,
}

impl SlotDedupTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    pub fn install(&self, name: &str, registry: Arc<Mutex<SlotFamilyRegistry>>) {
        self.inner
            .lock()
            .unwrap()
            .insert(name.to_string(), registry);
    }

    pub fn get(&self, name: &str) -> Option<Arc<Mutex<SlotFamilyRegistry>>> {
        self.inner.lock().unwrap().get(name).map(Arc::clone)
    }

    pub fn remove(&self, name: &str) {
        self.inner.lock().unwrap().remove(name);
    }

    /// Get-or-create a registry for `name` (UI path before spawn is rare).
    pub fn get_or_create(&self, name: &str) -> Arc<Mutex<SlotFamilyRegistry>> {
        let mut g = self.inner.lock().unwrap();
        g.entry(name.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(SlotFamilyRegistry::new())))
            .clone()
    }
}

/// Process-wide table shared by host / host-play / panel owners for one slot
/// name. Not a body cache — only registry handles keyed by slot username.
pub fn process_slot_table() -> &'static SlotDedupTable {
    use std::sync::OnceLock;
    static TABLE: OnceLock<SlotDedupTable> = OnceLock::new();
    TABLE.get_or_init(SlotDedupTable::new)
}

/// Register a new owner cursor on the process table for `slot_name`.
pub fn attach_owner_for_slot(slot_name: &str) -> DedupHandle {
    let reg = process_slot_table().get_or_create(slot_name);
    DedupHandle::additional_owner(&reg)
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
    holders: &[(Arc<Vec<WidgetView>>, Arc<Vec<SideTabView>>, Arc<Vec<LocView>>)],
    reg: &SlotFamilyRegistry,
    scratch_peak_bytes: usize,
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
    let unique_arc_count =
        widget_bodies.len() + side_bodies.len() + loc_bodies.len();
    // Approximate Arc heap header overhead (strong/weak counts + data ptr).
    let arc_header = unique_arc_count * (std::mem::size_of::<usize>() * 3);

    AllocationAccount {
        old_per_owner_payload_bytes: old_private,
        unique_body_payload_bytes: unique,
        scratch_peak_bytes,
        registry_metadata_bytes: reg.metadata_bytes(),
        live_owner_count: live,
        arc_header_bytes: arc_header,
        weak_slot_bytes: live * 3 * std::mem::size_of::<Option<Weak<Vec<()>>>>(),
    }
}
