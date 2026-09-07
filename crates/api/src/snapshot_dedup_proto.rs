//! Bounded A1 prototype: rebuild-edge exact content dedup for Widgets,
//! SideTabs, and Loc family bodies.
//!
//! Not production wiring. Cursors keep real `GameSnapshot` gates and run the
//! existing rebuild edge, then intern completed bodies by total equality into
//! a slot-local weak registry. Quiet gate hits do no equality work. No
//! generation-only share stamp and no read-path deep compare.

use crate::snapshot::{Family, GameSnapshot, LocView, SideTabView, WidgetView};
use client::client::Client;
use std::sync::{Arc, Weak};

/// Maximum concurrent owner cursors per slot (host, host-play, panel).
pub const MAX_CURSORS: usize = 3;

/// Per-family counters for the discriminator report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FamilyCounters {
    pub walks: u64,
    pub quiet_skips: u64,
    pub equality_comparisons: u64,
    pub equality_hits: u64,
    pub equality_misses: u64,
    pub publishes: u64,
}

/// Aggregate counters across the three prototype families.
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
}

/// Allocation-style accounting in bytes (not RSS). Counts unique live Arc
/// bodies, retained scratch peaks, and weak-registry metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllocationAccount {
    /// Sum of per-owner body payload estimates as if each kept a private Vec.
    pub old_per_owner_payload_bytes: usize,
    /// Sum of unique live shared body payload estimates.
    pub unique_body_payload_bytes: usize,
    /// High-water mark of scratch candidate bodies during rebuild/intern.
    pub scratch_peak_bytes: usize,
    /// Registry slot metadata (weak handles × families × cursors).
    pub registry_metadata_bytes: usize,
}

/// Slot-local weak registry of currently published family bodies.
///
/// One weak slot per cursor index per family. Overwrite on replacement;
/// drop clears the slot. Dead weaks are ignored on lookup.
#[derive(Default)]
pub struct SlotFamilyRegistry {
    widgets: [Option<Weak<Vec<WidgetView>>>; MAX_CURSORS],
    side_tabs: [Option<Weak<Vec<SideTabView>>>; MAX_CURSORS],
    loc: [Option<Weak<Vec<LocView>>>; MAX_CURSORS],
}

impl SlotFamilyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn clean_widgets(&mut self) {
        for slot in &mut self.widgets {
            if let Some(w) = slot {
                if w.strong_count() == 0 {
                    *slot = None;
                }
            }
        }
    }

    fn clean_side_tabs(&mut self) {
        for slot in &mut self.side_tabs {
            if let Some(w) = slot {
                if w.strong_count() == 0 {
                    *slot = None;
                }
            }
        }
    }

    fn clean_loc(&mut self) {
        for slot in &mut self.loc {
            if let Some(w) = slot {
                if w.strong_count() == 0 {
                    *slot = None;
                }
            }
        }
    }

    /// Intern a completed widgets body for `cursor_id`. Exact equality only.
    pub fn intern_widgets(
        &mut self,
        cursor_id: usize,
        candidate: Vec<WidgetView>,
        counters: &mut FamilyCounters,
    ) -> Arc<Vec<WidgetView>> {
        assert!(cursor_id < MAX_CURSORS);
        self.clean_widgets();
        for (i, slot) in self.widgets.iter().enumerate() {
            if i == cursor_id {
                continue;
            }
            if let Some(weak) = slot {
                if let Some(existing) = weak.upgrade() {
                    counters.equality_comparisons += 1;
                    if widgets_eq(existing.as_slice(), candidate.as_slice()) {
                        counters.equality_hits += 1;
                        // Publish this cursor's weak onto the shared body.
                        self.widgets[cursor_id] = Some(Arc::downgrade(&existing));
                        return existing;
                    }
                    counters.equality_misses += 1;
                }
            }
        }
        counters.publishes += 1;
        let arc = Arc::new(candidate);
        self.widgets[cursor_id] = Some(Arc::downgrade(&arc));
        arc
    }

    pub fn intern_side_tabs(
        &mut self,
        cursor_id: usize,
        candidate: Vec<SideTabView>,
        counters: &mut FamilyCounters,
    ) -> Arc<Vec<SideTabView>> {
        assert!(cursor_id < MAX_CURSORS);
        self.clean_side_tabs();
        for (i, slot) in self.side_tabs.iter().enumerate() {
            if i == cursor_id {
                continue;
            }
            if let Some(weak) = slot {
                if let Some(existing) = weak.upgrade() {
                    counters.equality_comparisons += 1;
                    if side_tabs_eq(existing.as_slice(), candidate.as_slice()) {
                        counters.equality_hits += 1;
                        self.side_tabs[cursor_id] = Some(Arc::downgrade(&existing));
                        return existing;
                    }
                    counters.equality_misses += 1;
                }
            }
        }
        counters.publishes += 1;
        let arc = Arc::new(candidate);
        self.side_tabs[cursor_id] = Some(Arc::downgrade(&arc));
        arc
    }

    pub fn intern_loc(
        &mut self,
        cursor_id: usize,
        candidate: Vec<LocView>,
        counters: &mut FamilyCounters,
    ) -> Arc<Vec<LocView>> {
        assert!(cursor_id < MAX_CURSORS);
        self.clean_loc();
        for (i, slot) in self.loc.iter().enumerate() {
            if i == cursor_id {
                continue;
            }
            if let Some(weak) = slot {
                if let Some(existing) = weak.upgrade() {
                    counters.equality_comparisons += 1;
                    if locs_eq(existing.as_slice(), candidate.as_slice()) {
                        counters.equality_hits += 1;
                        self.loc[cursor_id] = Some(Arc::downgrade(&existing));
                        return existing;
                    }
                    counters.equality_misses += 1;
                }
            }
        }
        counters.publishes += 1;
        let arc = Arc::new(candidate);
        self.loc[cursor_id] = Some(Arc::downgrade(&arc));
        arc
    }

    /// Clear the cursor's weak entries (cursor drop / restart).
    pub fn unregister(&mut self, cursor_id: usize) {
        assert!(cursor_id < MAX_CURSORS);
        self.widgets[cursor_id] = None;
        self.side_tabs[cursor_id] = None;
        self.loc[cursor_id] = None;
    }

    pub fn metadata_bytes(&self) -> usize {
        // Three Option<Weak<T>> arrays of MAX_CURSORS.
        3 * MAX_CURSORS * std::mem::size_of::<Option<Weak<Vec<()>>>>()
    }

    pub fn unique_widget_bodies(&self) -> Vec<Arc<Vec<WidgetView>>> {
        unique_arcs_from_slots(&self.widgets)
    }

    pub fn unique_side_tab_bodies(&self) -> Vec<Arc<Vec<SideTabView>>> {
        unique_arcs_from_slots(&self.side_tabs)
    }

    pub fn unique_loc_bodies(&self) -> Vec<Arc<Vec<LocView>>> {
        unique_arcs_from_slots(&self.loc)
    }
}

fn unique_arcs_from_slots<T>(slots: &[Option<Weak<Vec<T>>>]) -> Vec<Arc<Vec<T>>> {
    let mut out: Vec<Arc<Vec<T>>> = Vec::new();
    for slot in slots {
        if let Some(w) = slot {
            if let Some(arc) = w.upgrade() {
                if !out.iter().any(|e| Arc::ptr_eq(e, &arc)) {
                    out.push(arc);
                }
            }
        }
    }
    out
}

/// Owner-local cursor: real GameSnapshot gates + Arc family bodies.
pub struct DedupCursor {
    id: usize,
    /// Owns gates/history; family Vecs are cleared after each rebuild take.
    snap: GameSnapshot,
    widgets: Arc<Vec<WidgetView>>,
    side_tabs: Arc<Vec<SideTabView>>,
    loc: Arc<Vec<LocView>>,
    counters: DedupCounters,
    scratch_peak_bytes: usize,
}

impl DedupCursor {
    pub fn new(id: usize) -> Self {
        assert!(id < MAX_CURSORS);
        Self {
            id,
            snap: GameSnapshot::new(),
            widgets: Arc::new(Vec::new()),
            side_tabs: Arc::new(Vec::new()),
            loc: Arc::new(Vec::new()),
            counters: DedupCounters::default(),
            scratch_peak_bytes: 0,
        }
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn counters(&self) -> &DedupCounters {
        &self.counters
    }

    pub fn scratch_peak_bytes(&self) -> usize {
        self.scratch_peak_bytes
    }

    pub fn widgets(&self) -> &[WidgetView] {
        self.widgets.as_slice()
    }

    pub fn side_tabs(&self) -> &[SideTabView] {
        self.side_tabs.as_slice()
    }

    pub fn locs(&self) -> &[LocView] {
        self.loc.as_slice()
    }

    pub fn widgets_arc(&self) -> Arc<Vec<WidgetView>> {
        Arc::clone(&self.widgets)
    }

    pub fn side_tabs_arc(&self) -> Arc<Vec<SideTabView>> {
        Arc::clone(&self.side_tabs)
    }

    pub fn locs_arc(&self) -> Arc<Vec<LocView>> {
        Arc::clone(&self.loc)
    }

    /// Rebuild Widgets at the existing gate edge, then exact-content intern.
    pub fn rebuild_widgets(&mut self, client: &Client, reg: &mut SlotFamilyRegistry) -> bool {
        let moved = self.snap.rebuild_family(client, Family::Widgets);
        if !moved {
            self.counters.widgets.quiet_skips += 1;
            return false;
        }
        self.counters.widgets.walks += 1;
        let candidate = self.snap.proto_take_widgets();
        let scratch = widgets_payload_bytes(&candidate);
        self.scratch_peak_bytes = self.scratch_peak_bytes.max(scratch);
        self.widgets = reg.intern_widgets(self.id, candidate, &mut self.counters.widgets);
        true
    }

    pub fn rebuild_side_tabs(&mut self, client: &Client, reg: &mut SlotFamilyRegistry) -> bool {
        let moved = self.snap.rebuild_family(client, Family::SideTabs);
        if !moved {
            self.counters.side_tabs.quiet_skips += 1;
            return false;
        }
        self.counters.side_tabs.walks += 1;
        let candidate = self.snap.proto_take_side_tabs();
        let scratch = side_tabs_payload_bytes(&candidate);
        self.scratch_peak_bytes = self.scratch_peak_bytes.max(scratch);
        self.side_tabs = reg.intern_side_tabs(self.id, candidate, &mut self.counters.side_tabs);
        true
    }

    pub fn rebuild_loc(&mut self, client: &Client, reg: &mut SlotFamilyRegistry) -> bool {
        let moved = self.snap.rebuild_family(client, Family::Loc);
        if !moved {
            self.counters.loc.quiet_skips += 1;
            return false;
        }
        self.counters.loc.walks += 1;
        let candidate = self.snap.proto_take_locs();
        let scratch = locs_payload_bytes(&candidate);
        self.scratch_peak_bytes = self.scratch_peak_bytes.max(scratch);
        self.loc = reg.intern_loc(self.id, candidate, &mut self.counters.loc);
        true
    }

    /// Rebuild the three prototype families.
    pub fn rebuild_families(&mut self, client: &Client, reg: &mut SlotFamilyRegistry) -> bool {
        let mut dirty = false;
        dirty |= self.rebuild_widgets(client, reg);
        dirty |= self.rebuild_side_tabs(client, reg);
        dirty |= self.rebuild_loc(client, reg);
        dirty
    }

    /// Drop registration; caller should drop the cursor afterward.
    pub fn unregister(self, reg: &mut SlotFamilyRegistry) {
        reg.unregister(self.id);
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
    // Prefer capacity when available via Vec.
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
pub fn account_allocations(
    cursors: &[&DedupCursor],
    reg: &SlotFamilyRegistry,
) -> AllocationAccount {
    let mut scratch_peak = 0usize;
    for c in cursors {
        scratch_peak = scratch_peak.max(c.scratch_peak_bytes());
    }

    let mut old_private = 0usize;
    let mut unique = 0usize;

    // Count holders via cursor ptr_eq — do not use Arc::strong_count while
    // the unique-body collector also holds a temporary strong ref.
    let widget_bodies = reg.unique_widget_bodies();
    for body in &widget_bodies {
        let payload = widgets_payload_bytes_vec(body.as_ref());
        unique += payload;
        let holders = cursors
            .iter()
            .filter(|c| Arc::ptr_eq(&c.widgets_arc(), body))
            .count()
            .max(1);
        old_private += payload * holders;
    }
    let side_bodies = reg.unique_side_tab_bodies();
    for body in &side_bodies {
        let payload = side_tabs_payload_bytes_vec(body.as_ref());
        unique += payload;
        let holders = cursors
            .iter()
            .filter(|c| Arc::ptr_eq(&c.side_tabs_arc(), body))
            .count()
            .max(1);
        old_private += payload * holders;
    }
    let loc_bodies = reg.unique_loc_bodies();
    for body in &loc_bodies {
        let payload = locs_payload_bytes_vec(body.as_ref());
        unique += payload;
        let holders = cursors
            .iter()
            .filter(|c| Arc::ptr_eq(&c.locs_arc(), body))
            .count()
            .max(1);
        old_private += payload * holders;
    }

    AllocationAccount {
        old_per_owner_payload_bytes: old_private,
        unique_body_payload_bytes: unique,
        scratch_peak_bytes: scratch_peak,
        registry_metadata_bytes: reg.metadata_bytes(),
    }
}
