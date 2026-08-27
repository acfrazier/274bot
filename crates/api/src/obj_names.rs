//! Item/loc definition views the snapshot reads, plus the thin obj-id →
//! name table compiled scripts resolve inventory ids against. The full
//! `ObjType`/`LocType` decode carries model/sprite/op data a script never
//! reads; these keep only the definition surface, indexed by id, built
//! once per `Play` and shared by every slot.

use client::config::{LocType, ObjType};
use serde::Serialize;

/// Owned view of one obj definition, mapped to the surface the snapshot
/// and queries read.
///
/// `noted` is a best-effort 2004 approximation: the 2004 cache has no
/// explicit `noted` flag (OSRS does), so an obj is treated as a bank
/// note/certificate of another iff `certlink != -1`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemDefView {
    pub id: i32,
    pub name: Option<String>,
    pub stackable: bool,
    pub members: bool,
    pub base_value: i32,
    pub noted: bool,
    pub certificate_link: i32,
    pub certificate_template: i32,
}

impl ItemDefView {
    fn from_obj(o: &ObjType) -> Self {
        ItemDefView {
            id: o.id,
            name: (!o.name.is_empty()).then(|| o.name.clone()),
            stackable: o.stackable,
            members: o.members,
            base_value: o.cost,
            noted: o.certlink != -1,
            certificate_link: o.certlink,
            certificate_template: o.certtemplate,
        }
    }
}

/// Owned view of one loc definition, mapped to the surface the snapshot
/// and queries read.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LocDefView {
    pub name: Option<String>,
    pub ops: Vec<String>,
    pub width: i32,
    pub length: i32,
    pub block_walk: bool,
    pub block_range: bool,
    pub active: bool,
    pub force_approach: i32,
}

impl LocDefView {
    fn from_loc(l: &LocType) -> Self {
        LocDefView {
            name: (!l.name.is_empty()).then(|| l.name.clone()),
            ops: l.op.iter().flatten().cloned().collect(),
            width: l.width,
            length: l.length,
            block_walk: l.blockwalk,
            block_range: l.blockrange,
            active: l.active,
            force_approach: l.forceapproach,
        }
    }
}

/// Obj-id → item definition view, `items[id]` for `0 <= id < items.len()`;
/// ids outside that range (negative or not-yet-loaded) read as `None`.
#[derive(Default)]
pub struct ObjNames {
    items: Vec<Option<ItemDefView>>,
}

impl ObjNames {
    /// Map each type's id onto the definition view. Obj ids are small
    /// non-negative ints (`ObjType::unpack` assigns `0..count` in table
    /// order), so the table is sized to `max(id)+1` and indexed directly;
    /// a missing id reads `None`.
    pub fn from_objs(objs: &[ObjType]) -> Self {
        let max_id = objs.iter().map(|o| o.id).max().unwrap_or(-1);
        let mut items = vec![None; (max_id + 1).max(0) as usize];
        for o in objs {
            if o.id >= 0 && (o.id as usize) < items.len() {
                items[o.id as usize] = Some(ItemDefView::from_obj(o));
            }
        }
        Self { items }
    }

    /// The item definition view for obj `id`, `None` when the id is
    /// unknown or out of range.
    pub fn item(&self, id: i32) -> Option<&ItemDefView> {
        if id < 0 {
            return None;
        }
        self.items.get(id as usize).and_then(|i| i.as_ref())
    }

    /// The name for obj `id`, `None` when the id is unknown or the type's
    /// name is empty (the compat shim the script crate resolves
    /// `has_item` against).
    pub fn name(&self, id: i32) -> Option<&str> {
        self.item(id).and_then(|i| i.name.as_deref())
    }

    /// The obj id whose name is exactly `name`, `None` when no loaded obj
    /// matches (the inverse of [`ObjNames::name`], for `has_item("Bones")`
    /// style lookups). Scans the id table on each call; callers resolve
    /// once and keep the id.
    pub fn by_name(&self, name: &str) -> Option<i32> {
        self.items
            .iter()
            .position(|i| i.as_ref().and_then(|v| v.name.as_deref()) == Some(name))
            .map(|id| id as i32)
    }
}

/// Loc-id → loc definition view, `locs[id]` for `0 <= id < locs.len()`;
/// ids outside that range (negative or not-yet-loaded) read as `None`.
#[derive(Default)]
pub struct LocDefs {
    locs: Vec<Option<LocDefView>>,
}

impl LocDefs {
    /// Map each loc's id onto the definition view, sized to `max(id)+1`
    /// and indexed directly like [`ObjNames::from_objs`].
    pub fn from_locs(locs: &[LocType]) -> Self {
        let max_id = locs.iter().map(|l| l.id).max().unwrap_or(-1);
        let mut views = vec![None; (max_id + 1).max(0) as usize];
        for l in locs {
            if l.id >= 0 && (l.id as usize) < views.len() {
                views[l.id as usize] = Some(LocDefView::from_loc(l));
            }
        }
        Self { locs: views }
    }

    /// The loc definition view for loc `id`, `None` when the id is
    /// unknown or out of range.
    pub fn loc(&self, id: i32) -> Option<&LocDefView> {
        if id < 0 {
            return None;
        }
        self.locs.get(id as usize).and_then(|l| l.as_ref())
    }
}
