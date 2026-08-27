//! Borrowing family queries: look up views from the last rebuild without
//! allocating a new world copy.

use crate::snapshot::NpcView;

/// Chainable predicate builder over a borrowed slice. Each `where_` narrows
/// the candidate set; terminal methods evaluate the combined predicates.
pub struct Query<'a, T> {
    values: &'a [T],
    predicates: Vec<Box<dyn Fn(&T) -> bool + 'a>>,
}

impl<'a, T> Query<'a, T> {
    pub fn new(values: &'a [T]) -> Self {
        Query {
            values,
            predicates: Vec::new(),
        }
    }

    pub fn where_(&mut self, p: impl Fn(&T) -> bool + 'a) -> &mut Self {
        self.predicates.push(Box::new(p));
        self
    }

    pub fn results(&self) -> Vec<&T> {
        self.values
            .iter()
            .filter(|v| self.predicates.iter().all(|p| p(v)))
            .collect()
    }

    pub fn first(&self) -> Option<&T> {
        self.values
            .iter()
            .find(|v| self.predicates.iter().all(|p| p(v)))
    }

    pub fn last(&self) -> Option<&T> {
        self.values
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

/// The view for a slot index, if that slot was live in the last rebuild.
pub fn npc_by_index(npcs: &[NpcView], index: usize) -> Option<&NpcView> {
    npcs.iter().find(|view| view.index == index)
}

/// Live views standing on a tile.
pub fn npcs_at(npcs: &[NpcView], x: i32, z: i32) -> impl Iterator<Item = &NpcView> {
    npcs.iter().filter(move |view| view.x == x && view.z == z)
}
