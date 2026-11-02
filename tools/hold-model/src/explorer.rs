//! Capped breadth-first exploration with parent links and trace rebuild.
//!
//! The search keeps each visited state once. A state cap, a frontier cap, a
//! byte cap and a depth horizon bound every run. A failed transition returns
//! the shortest trace that reaches it. The seat model's explorer has the same
//! shape but is welded to the seat state (its coverage counters and profile
//! prefixes), so this crate keeps its own small generic copy.

use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::mem::size_of;
use std::time::{Duration, Instant};

pub const DEFAULT_STATE_CAP: usize = 20_000_000;
pub const DEFAULT_FRONTIER_CAP: usize = 10_000_000;
pub const DEFAULT_BYTE_CAP: usize = 8 * 1024 * 1024 * 1024;
/// Most hit edges one goal keeps; a goal with more hits at its shortest depth
/// reports the cap, not the count.
pub const GOAL_EDGE_CAP: usize = 4096;

/// A transition that proves a property false.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Violation {
    pub name: &'static str,
    pub detail: String,
}

/// A model the explorer can search.
pub trait Machine {
    type State: Clone + Eq + Hash;
    type Action: Clone + Debug + Eq + Hash;

    fn initial(&self) -> Self::State;
    /// Every action enabled in `state`, appended to `out`.
    fn actions(&self, state: &Self::State, out: &mut Vec<Self::Action>);
    /// The state after `action`, or the violation that the transition proves.
    fn step(&self, state: &Self::State, action: &Self::Action) -> Result<Self::State, Violation>;
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CapKind {
    States,
    Frontier,
    Bytes,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Limits {
    pub state_cap: usize,
    pub frontier_cap: usize,
    pub byte_cap: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            state_cap: DEFAULT_STATE_CAP,
            frontier_cap: DEFAULT_FRONTIER_CAP,
            byte_cap: DEFAULT_BYTE_CAP,
        }
    }
}

/// The shortest trace set of one goal: every hit edge at the goal's first depth.
#[derive(Clone, Debug)]
pub struct GoalHits<A> {
    /// Number of actions of the shortest trace.
    pub depth: usize,
    /// `(parent node, action)` of each hit edge, up to [`GOAL_EDGE_CAP`].
    pub edges: Vec<(usize, A)>,
}

/// The result of one bounded run.
pub struct Exploration<S, A> {
    pub horizon: usize,
    pub states: usize,
    pub transitions: usize,
    pub max_frontier: usize,
    pub deepest: usize,
    pub estimated_peak_bytes: usize,
    pub elapsed: Duration,
    /// Every state within the horizon was visited and no cap was reached.
    pub complete: bool,
    pub cap: Option<(CapKind, String)>,
    /// The first violation, with the shortest trace that reaches it.
    pub failure: Option<(Violation, Vec<A>)>,
    /// Goal hits by goal id, in the order that goals were first reached.
    pub goals: Vec<(u32, GoalHits<A>)>,
    nodes: Vec<Node<S, A>>,
    preds: HashMap<usize, Vec<(usize, A)>>,
}

struct Node<S, A> {
    state: S,
    parent: u32,
    via: Option<A>,
    depth: u32,
}

const NONE: u32 = u32::MAX;

impl<S: Clone, A: Clone> Exploration<S, A> {
    /// The shortest trace to `node`, from the initial state.
    pub fn trace_to(&self, node: usize) -> Vec<A> {
        let mut out = Vec::new();
        let mut at = node;
        while let Some(via) = &self.nodes[at].via {
            out.push(via.clone());
            at = self.nodes[at].parent as usize;
        }
        out.reverse();
        out
    }

    /// The state of `node`.
    pub fn state_of(&self, node: usize) -> &S {
        &self.nodes[node].state
    }

    /// Every shortest trace of `goal`, up to `cap` traces. Needs a run with
    /// `retain_dag` set; without it only one prefix per hit edge is known.
    pub fn goal_traces(&self, goal: u32, cap: usize) -> Vec<Vec<A>> {
        let Some((_, hits)) = self.goals.iter().find(|(id, _)| *id == goal) else {
            return Vec::new();
        };
        let mut traces = Vec::new();
        for (parent, action) in &hits.edges {
            let mut prefixes = Vec::new();
            self.paths_to(*parent, &mut Vec::new(), &mut prefixes, cap);
            for mut prefix in prefixes {
                prefix.push(action.clone());
                traces.push(prefix);
                if traces.len() >= cap {
                    return traces;
                }
            }
        }
        traces
    }

    /// Every shortest path to `node`, appended to `out` in root-to-node order.
    fn paths_to(&self, node: usize, tail: &mut Vec<A>, out: &mut Vec<Vec<A>>, cap: usize) {
        if out.len() >= cap {
            return;
        }
        let first = &self.nodes[node];
        if first.via.is_none() {
            let mut path = tail.clone();
            path.reverse();
            out.push(path);
            return;
        }
        let mut edges: Vec<(usize, A)> = Vec::new();
        if let Some(via) = &first.via {
            edges.push((first.parent as usize, via.clone()));
        }
        if let Some(extra) = self.preds.get(&node) {
            edges.extend(extra.iter().cloned());
        }
        for (parent, action) in edges {
            tail.push(action);
            self.paths_to(parent, tail, out, cap);
            tail.pop();
            if out.len() >= cap {
                return;
            }
        }
    }
}

/// What a run records besides the state graph.
pub struct Options<'a, M: Machine> {
    /// Keep every parent edge at the shortest depth, so that
    /// [`Exploration::goal_traces`] can list every shortest trace.
    pub retain_dag: bool,
    /// Called for each transition that did not fail; it appends the goals
    /// that the edge reaches to the vector.
    pub goal: Option<&'a dyn Fn(&M::State, &M::Action, &M::State, &mut Vec<u32>)>,
}

impl<M: Machine> Default for Options<'_, M> {
    fn default() -> Self {
        Self {
            retain_dag: false,
            goal: None,
        }
    }
}

fn fingerprint<S: Hash>(state: &S) -> u64 {
    let mut hasher = DefaultHasher::new();
    state.hash(&mut hasher);
    hasher.finish()
}

fn node_cost<S, A>() -> usize {
    // The node, its hash table entry, its chain link and its frontier slot.
    size_of::<Node<S, A>>() + 40
}

/// Explore every action sequence of at most `horizon` steps.
pub fn explore<M: Machine>(
    machine: &M,
    horizon: usize,
    limits: Limits,
    options: &Options<'_, M>,
) -> Exploration<M::State, M::Action> {
    let started = Instant::now();
    let mut out = Exploration {
        horizon,
        states: 0,
        transitions: 0,
        max_frontier: 0,
        deepest: 0,
        estimated_peak_bytes: 0,
        elapsed: Duration::ZERO,
        complete: false,
        cap: None,
        failure: None,
        goals: Vec::new(),
        nodes: Vec::new(),
        preds: HashMap::new(),
    };
    let cost = node_cost::<M::State, M::Action>();
    let mut table: HashMap<u64, u32> = HashMap::new();
    let mut chain: Vec<u32> = Vec::new();
    let mut frontier: VecDeque<u32> = VecDeque::new();

    let root = machine.initial();
    table.insert(fingerprint(&root), 0);
    chain.push(NONE);
    out.nodes.push(Node {
        state: root,
        parent: NONE,
        via: None,
        depth: 0,
    });
    frontier.push_back(0);
    out.states = 1;
    out.max_frontier = 1;

    let mut actions = Vec::new();
    while let Some(index) = frontier.pop_front() {
        let index = index as usize;
        let depth = out.nodes[index].depth as usize;
        if depth >= horizon {
            continue;
        }
        actions.clear();
        machine.actions(&out.nodes[index].state, &mut actions);
        for action in actions.drain(..) {
            out.transitions += 1;
            let mut hits: Vec<u32> = Vec::new();
            let next = match machine.step(&out.nodes[index].state, &action) {
                Ok(next) => next,
                Err(violation) => {
                    let mut trace = out.trace_to(index);
                    trace.push(action);
                    out.failure = Some((violation, trace));
                    out.elapsed = started.elapsed();
                    return out;
                }
            };
            if let Some(goal) = options.goal {
                hits.clear();
                goal(&out.nodes[index].state, &action, &next, &mut hits);
                for id in hits.iter().copied() {
                    record_goal(&mut out.goals, id, depth + 1, index, &action);
                }
            }
            let hash = fingerprint(&next);
            let mut cursor = table.get(&hash).copied();
            let mut found = None;
            while let Some(at) = cursor {
                if out.nodes[at as usize].state == next {
                    found = Some(at as usize);
                    break;
                }
                cursor = (chain[at as usize] != NONE).then(|| chain[at as usize]);
            }
            if let Some(known) = found {
                if options.retain_dag && out.nodes[known].depth as usize == depth + 1 {
                    out.preds
                        .entry(known)
                        .or_default()
                        .push((index, action.clone()));
                }
                continue;
            }
            let estimate = (out.states + 1) * cost;
            out.estimated_peak_bytes = out.estimated_peak_bytes.max(estimate);
            let capped = if out.states >= limits.state_cap {
                Some((
                    CapKind::States,
                    format!("state cap {} reached", limits.state_cap),
                ))
            } else if frontier.len() >= limits.frontier_cap {
                Some((
                    CapKind::Frontier,
                    format!("frontier cap {} reached", limits.frontier_cap),
                ))
            } else if estimate > limits.byte_cap {
                Some((
                    CapKind::Bytes,
                    format!(
                        "byte estimate {estimate} exceeds byte cap {}",
                        limits.byte_cap
                    ),
                ))
            } else {
                None
            };
            if let Some(cap) = capped {
                out.cap = Some(cap);
                out.elapsed = started.elapsed();
                return out;
            }
            let at = out.nodes.len() as u32;
            let previous = table.insert(hash, at);
            chain.push(previous.unwrap_or(NONE));
            out.nodes.push(Node {
                state: next,
                parent: index as u32,
                via: Some(action),
                depth: (depth + 1) as u32,
            });
            frontier.push_back(at);
            out.states += 1;
            out.deepest = out.deepest.max(depth + 1);
            out.max_frontier = out.max_frontier.max(frontier.len());
        }
    }
    out.complete = true;
    out.elapsed = started.elapsed();
    out
}

fn record_goal<A: Clone>(
    goals: &mut Vec<(u32, GoalHits<A>)>,
    id: u32,
    depth: usize,
    parent: usize,
    action: &A,
) {
    match goals.iter_mut().find(|(known, _)| *known == id) {
        None => goals.push((
            id,
            GoalHits {
                depth,
                edges: vec![(parent, action.clone())],
            },
        )),
        Some((_, hits)) => {
            if depth == hits.depth && hits.edges.len() < GOAL_EDGE_CAP {
                hits.edges.push((parent, action.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A counter that fails at 5, to test shortest traces and caps.
    struct Counter {
        fail_at: u8,
    }

    impl Machine for Counter {
        type State = u8;
        type Action = u8;

        fn initial(&self) -> u8 {
            0
        }

        fn actions(&self, _: &u8, out: &mut Vec<u8>) {
            out.extend([1, 2]);
        }

        fn step(&self, state: &u8, action: &u8) -> Result<u8, Violation> {
            let next = state + action;
            if next == self.fail_at {
                return Err(Violation {
                    name: "counter",
                    detail: format!("reached {next}"),
                });
            }
            Ok(next)
        }
    }

    #[test]
    fn failure_trace_is_shortest() {
        let result = explore(
            &Counter { fail_at: 5 },
            10,
            Limits::default(),
            &Options::default(),
        );
        let (violation, trace) = result.failure.expect("fails");
        assert_eq!(violation.name, "counter");
        assert_eq!(trace.len(), 3);
        assert_eq!(trace.iter().map(|a| u32::from(*a)).sum::<u32>(), 5);
    }

    #[test]
    fn caps_stop_the_run_and_a_clean_run_completes() {
        let capped = explore(
            &Counter { fail_at: 250 },
            50,
            Limits {
                state_cap: 3,
                ..Limits::default()
            },
            &Options::default(),
        );
        assert!(!capped.complete);
        assert_eq!(capped.cap.map(|(kind, _)| kind), Some(CapKind::States));
        let clean = explore(
            &Counter { fail_at: 250 },
            4,
            Limits::default(),
            &Options::default(),
        );
        assert!(clean.complete && clean.cap.is_none() && clean.failure.is_none());
        assert_eq!(clean.states, 9);
    }

    #[test]
    fn every_shortest_trace_of_a_goal_is_listed() {
        let goal = |_: &u8, _: &u8, next: &u8, out: &mut Vec<u32>| {
            if *next == 4 {
                out.push(7);
            }
        };
        let result = explore(
            &Counter { fail_at: 250 },
            6,
            Limits::default(),
            &Options {
                retain_dag: true,
                goal: Some(&goal),
            },
        );
        // 4 = 2+2 only at depth 2; 1+1+2, 1+2+1, 2+1+1 are depth 3.
        let traces = result.goal_traces(7, 100);
        assert_eq!(traces, vec![vec![2, 2]]);
        let goal = |_: &u8, _: &u8, next: &u8, out: &mut Vec<u32>| {
            if *next == 5 {
                out.push(8);
            }
        };
        let result = explore(
            &Counter { fail_at: 250 },
            6,
            Limits::default(),
            &Options {
                retain_dag: true,
                goal: Some(&goal),
            },
        );
        let mut traces = result.goal_traces(8, 100);
        traces.sort();
        assert_eq!(traces, vec![vec![1, 2, 2], vec![2, 1, 2], vec![2, 2, 1]]);
    }
}
