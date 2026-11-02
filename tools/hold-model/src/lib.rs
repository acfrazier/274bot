//! Bounded conformance oracle for the auto-run hold. No product crate depends
//! on this crate, except as a dev-dependency of the conformance test.
//!
//! The model (`model.rs`) is a transition system over small counters. The
//! explorer (`explorer.rs`) searches it breadth first inside caps and returns
//! the shortest trace of any violation. See README.md.

mod explorer;
mod model;

pub use explorer::*;
pub use model::*;

/// A named set of bounds for one run.
#[derive(Clone, Copy, Debug)]
pub struct Profile {
    pub name: &'static str,
    pub bounds: Bounds,
}

/// The named profiles. Every run uses one of them, with a horizon.
pub fn profiles() -> Vec<Profile> {
    let all = Bounds::standard();
    vec![
        Profile {
            name: "smoke",
            bounds: Bounds {
                horizon: 8,
                max_tick: 3,
                max_wall: 3,
                max_pages: 3,
                ..all
            },
        },
        Profile {
            name: "modal",
            bounds: Bounds {
                quiet: false,
                compat: false,
                ..all
            },
        },
        Profile {
            name: "quiet",
            bounds: Bounds {
                modal: false,
                compat: false,
                ..all
            },
        },
        Profile {
            name: "compat",
            bounds: Bounds {
                modal: false,
                quiet: false,
                ..all
            },
        },
        Profile {
            name: "all",
            bounds: all,
        },
    ]
}

pub fn profile(name: &str) -> Option<Profile> {
    profiles().into_iter().find(|p| p.name == name)
}

/// Explore one profile with one mutation. With `all_violations` the run does
/// not stop at the first violation: `Exploration::failures` holds the
/// shortest trace of each violation name.
pub fn explore_profile(
    profile: &Profile,
    mutation: Mutation,
    horizon: Option<usize>,
    limits: Limits,
    all_violations: bool,
) -> Exploration<State, Action> {
    let model = Model::new(profile.bounds, mutation);
    explore(
        &model,
        horizon.unwrap_or(profile.bounds.horizon),
        limits,
        &Options {
            all_violations,
            ..Options::default()
        },
    )
}

pub const KINDS: [Kind; 3] = [Kind::Modal, Kind::Quiet, Kind::Compat];
pub const CAUSES: [Cause; 8] = [
    Cause::Page,
    Cause::Bound,
    Cause::Dropped,
    Cause::Stop,
    Cause::Session,
    Cause::Death,
    Cause::Wall,
    Cause::HostTerminal,
];

fn goal_id(kind: Kind, cause: Cause) -> u32 {
    let k = KINDS.iter().position(|x| *x == kind).unwrap_or(0);
    let c = CAUSES.iter().position(|x| *x == cause).unwrap_or(0);
    (k * CAUSES.len() + c) as u32
}

/// A release that a trace reaches: one lease kind ending for one cause.
pub type Release = (Kind, Cause);

/// Every shortest trace to each release of each lease kind, up to `cap`
/// traces per release. The goal of a release is any transition that ends a
/// lease of that kind with that cause. The run records every parent edge.
pub fn release_traces(
    bounds: Bounds,
    mutation: Mutation,
    horizon: usize,
    limits: Limits,
    cap: usize,
) -> (
    Exploration<State, Action>,
    Vec<(Release, Vec<Vec<Action>>)>,
) {
    let model = Model::new(bounds, mutation);
    let goal = |pre: &State, action: &Action, _post: &State, out: &mut Vec<u32>| {
        if let Ok((_, info)) = model.step_detail(pre, action) {
            for (kind, cause) in &info.releases {
                out.push(goal_id(*kind, *cause));
            }
        }
    };
    let run = explore(
        &model,
        horizon,
        limits,
        &Options {
            retain_dag: true,
            goal: Some(&goal),
            ..Options::default()
        },
    );
    let mut found = Vec::new();
    for kind in KINDS {
        for cause in CAUSES {
            let traces = run.goal_traces(goal_id(kind, cause), cap);
            if !traces.is_empty() {
                found.push(((kind, cause), traces));
            }
        }
    }
    (run, found)
}
