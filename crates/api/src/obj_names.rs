//! Thin obj-id → name table for compiled scripts. The full `ObjType`
//! decode carries model/sprite/op data a script never reads; this keeps
//! only `id` + `name`, indexed by id, built once per `Play` and shared by
//! every slot (lean channels never load their own cache).

use client::config::ObjType;

/// Obj-id → name, `names[id]` for `0 <= id < names.len()`; ids outside
/// that range (negative or not-yet-loaded) read as `None`.
#[derive(Default)]
pub struct ObjNames {
    names: Vec<Option<String>>,
}

impl ObjNames {
    /// Keep only each type's id+name. Obj ids are small non-negative ints
    /// (`ObjType::unpack` assigns `0..count` in table order), so the table
    /// is sized to `max(id)+1` and indexed directly; a missing id reads
    /// `None`.
    pub fn from_objs(objs: &[ObjType]) -> Self {
        let max_id = objs.iter().map(|o| o.id).max().unwrap_or(-1);
        let mut names = vec![None; (max_id + 1).max(0) as usize];
        for o in objs {
            if o.id >= 0 && (o.id as usize) < names.len() {
                names[o.id as usize] = Some(o.name.clone());
            }
        }
        Self { names }
    }

    /// The name for obj `id`, `None` when the id is unknown or out of
    /// range.
    pub fn name(&self, id: i32) -> Option<&str> {
        if id < 0 {
            return None;
        }
        self.names.get(id as usize).and_then(|n| n.as_deref())
    }
}
