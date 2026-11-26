//! Lock-order checker. With the `lock-order` feature off, [`Mutex`] is
//! [`std::sync::Mutex`] and this module's tracker is not on the product
//! lock path. With the feature on, [`Mutex`] is [`TrackingMutex`]: the
//! first time one thread acquires two of those locks in both orders, it
//! panics, even when the acquisitions never overlap in time.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{
    LazyLock, Mutex as StdMutex, MutexGuard as StdMutexGuard, PoisonError, TryLockError,
};

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

fn next_id() -> usize {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

struct Graph {
    /// `held -> acquired while held`.
    edges: HashMap<usize, HashSet<usize>>,
    names: HashMap<usize, &'static str>,
    pending: Vec<usize>,
    seen: HashSet<usize>,
}

impl Graph {
    fn reaches(&mut self, from: usize, to: usize) -> bool {
        if from == to || !self.edges.contains_key(&from) {
            return false;
        }
        self.pending.clear();
        self.seen.clear();
        self.pending.push(from);
        while let Some(node) = self.pending.pop() {
            if !self.seen.insert(node) {
                continue;
            }
            let Some(next) = self.edges.get(&node) else {
                continue;
            };
            if next.contains(&to) {
                return true;
            }
            self.pending.extend(next.iter().copied());
        }
        false
    }

    fn label(&self, id: usize) -> String {
        match self.names.get(&id) {
            Some(name) => format!("{name}#{id}"),
            None => format!("lock#{id}"),
        }
    }
}

fn graph() -> &'static StdMutex<Graph> {
    static GRAPH: LazyLock<StdMutex<Graph>> = LazyLock::new(|| {
        StdMutex::new(Graph {
            edges: HashMap::new(),
            names: HashMap::new(),
            pending: Vec::new(),
            seen: HashSet::new(),
        })
    });
    &GRAPH
}

fn with_graph<R>(body: impl FnOnce(&mut Graph) -> R) -> R {
    let mut guard = graph().lock().unwrap_or_else(PoisonError::into_inner);
    body(&mut guard)
}

thread_local! {
    static HELD: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

fn unhold(id: usize) {
    HELD.with(|held| {
        let mut held = held.borrow_mut();
        if let Some(at) = held.iter().rposition(|item| *item == id) {
            held.remove(at);
        }
    });
}

fn forget(id: usize) {
    with_graph(|graph| {
        graph.edges.remove(&id);
        for next in graph.edges.values_mut() {
            next.remove(&id);
        }
        graph.names.remove(&id);
    });
}

/// Record `id` as acquired while the locks on this thread's held stack are
/// still held. Panics, after releasing the graph lock, when that edge
/// closes a cycle.
fn track(id: usize, name: &'static str) {
    let inversion = HELD.with(|held| {
        let mut held = held.borrow_mut();
        let inversion = with_graph(|graph| {
            graph.names.insert(id, name);
            for &previous in held.iter() {
                // An edge already in the acyclic graph needs no second search.
                if previous == id
                    || graph
                        .edges
                        .get(&previous)
                        .is_some_and(|next| next.contains(&id))
                {
                    continue;
                }
                if graph.reaches(id, previous) {
                    return Some(format!(
                        "lock order inversion: {} then {} closes a cycle ({} was already ordered before {})",
                        graph.label(previous),
                        graph.label(id),
                        graph.label(id),
                        graph.label(previous),
                    ));
                }
            }
            for &previous in held.iter().filter(|&&previous| previous != id) {
                graph.edges.entry(previous).or_default().insert(id);
            }
            None
        });
        if inversion.is_none() {
            held.push(id);
        }
        inversion
    });
    if let Some(message) = inversion {
        panic!("{message}");
    }
}

/// Order-tracking mutex. Same lock/poison API as [`std::sync::Mutex`].
pub struct TrackingMutex<T> {
    id: usize,
    name: &'static str,
    inner: StdMutex<T>,
}

impl<T> TrackingMutex<T> {
    pub fn new(value: T) -> Self {
        Self {
            id: next_id(),
            name: std::any::type_name::<T>(),
            inner: StdMutex::new(value),
        }
    }

    pub fn lock(&self) -> Result<TrackingGuard<'_, T>, PoisonError<TrackingGuard<'_, T>>> {
        // Record before waiting so an inverted order panics instead of hanging.
        track(self.id, self.name);
        match self.inner.lock() {
            Ok(inner) => Ok(TrackingGuard { inner, id: self.id }),
            Err(poison) => Err(PoisonError::new(TrackingGuard {
                inner: poison.into_inner(),
                id: self.id,
            })),
        }
    }

    pub fn try_lock(&self) -> Result<TrackingGuard<'_, T>, TryLockError<TrackingGuard<'_, T>>> {
        match self.inner.try_lock() {
            Ok(inner) => {
                track(self.id, self.name);
                Ok(TrackingGuard { inner, id: self.id })
            }
            Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
            Err(TryLockError::Poisoned(poison)) => {
                track(self.id, self.name);
                Err(TryLockError::Poisoned(PoisonError::new(TrackingGuard {
                    inner: poison.into_inner(),
                    id: self.id,
                })))
            }
        }
    }

    pub fn get_mut(&mut self) -> Result<&mut T, PoisonError<&mut T>> {
        self.inner.get_mut()
    }

    pub fn is_poisoned(&self) -> bool {
        self.inner.is_poisoned()
    }
}

impl<T: Default> Default for TrackingMutex<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> Drop for TrackingMutex<T> {
    fn drop(&mut self) {
        forget(self.id);
    }
}

pub struct TrackingGuard<'a, T> {
    inner: StdMutexGuard<'a, T>,
    id: usize,
}

impl<T> Drop for TrackingGuard<'_, T> {
    fn drop(&mut self) {
        unhold(self.id);
    }
}

impl<T> std::ops::Deref for TrackingGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> std::ops::DerefMut for TrackingGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

/// Product mutex. Plain [`std::sync::Mutex`] unless `lock-order` is on.
#[cfg(feature = "lock-order")]
pub use TrackingMutex as Mutex;

#[cfg(not(feature = "lock-order"))]
pub use std::sync::Mutex;

/// Guard for the product [`Mutex`], plain unless `lock-order` is on.
#[cfg(feature = "lock-order")]
pub use TrackingGuard as MutexGuard;

#[cfg(not(feature = "lock-order"))]
pub use std::sync::MutexGuard;

#[cfg(test)]
mod tests {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    #[cfg(not(feature = "lock-order"))]
    use std::sync::Mutex as StdMutex;

    use super::TrackingMutex;

    fn panic_text(payload: Box<dyn std::any::Any + Send>) -> String {
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| {
                payload
                    .downcast_ref::<&str>()
                    .map(|text| (*text).to_owned())
            })
            .unwrap_or_default()
    }

    #[test]
    fn opposite_order_panics_without_contention() {
        let first = TrackingMutex::new(1u8);
        let second = TrackingMutex::new(2u8);
        {
            let _first = first.lock().expect("first lock");
            let _second = second.lock().expect("second lock");
        }
        let panicked = catch_unwind(AssertUnwindSafe(|| {
            let _second = second.lock().expect("second lock");
            let _first = first.lock().expect("inverted first lock");
        }));
        let message = panic_text(panicked.expect_err("opposite order must panic"));
        assert!(
            message.contains("lock order inversion"),
            "panic text: {message}"
        );
    }

    #[test]
    fn repeated_same_order_does_not_panic() {
        let first = TrackingMutex::new(1u8);
        let second = TrackingMutex::new(2u8);
        for _ in 0..3 {
            let _first = first.lock().expect("first lock");
            let _second = second.lock().expect("second lock");
        }
    }

    #[test]
    fn unlocked_between_acquisitions_is_not_an_order() {
        let first = TrackingMutex::new(1u8);
        let second = TrackingMutex::new(2u8);
        {
            let _first = first.lock().expect("first lock");
        }
        {
            let _second = second.lock().expect("second lock");
        }
        {
            let _second = second.lock().expect("second lock");
        }
        {
            let _first = first.lock().expect("first lock");
        }
    }

    #[cfg(not(feature = "lock-order"))]
    #[test]
    fn feature_off_mutex_is_the_plain_mutex() {
        let mutex: StdMutex<u8> = super::Mutex::new(7);
        assert_eq!(*mutex.lock().expect("plain lock"), 7);
        assert_eq!(
            std::mem::size_of::<super::Mutex<u8>>(),
            std::mem::size_of::<StdMutex<u8>>()
        );
    }

    #[cfg(feature = "lock-order")]
    #[test]
    fn feature_on_alias_is_the_tracking_mutex() {
        let first = super::Mutex::new(1u8);
        let second = super::Mutex::new(2u8);
        {
            let _first = first.lock().expect("first lock");
            let _second = second.lock().expect("second lock");
        }
        let panicked = catch_unwind(AssertUnwindSafe(|| {
            let _second = second.lock().expect("second lock");
            let _first = first.lock().expect("inverted first lock");
        }));
        assert!(panicked.is_err(), "feature alias must panic on inversion");
    }
}
