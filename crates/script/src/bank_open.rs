//! Rust-owned one-attempt bank approach/open sequencing, a
//! [`crate::machine`] family. The facts are read from the isolate scene at
//! call time; JavaScript starts one open with the caller's mode, stand and
//! booth names, and awaits the frozen boolean. Rust owns the verbs, the
//! observed identity and the clocks.

use crate::machine::{Begin, Cx, Family, Step};
use crate::observed::{self, Scene, SceneRow};
use crate::shim::InteractReq;
use serde::Deserialize;
use serde_json::Value;

pub const WALK_BOUND_MS: u64 = 60_000;
pub const BANK_READY_MS: u64 = 5_000;
const ACCESS_RADIUS: i32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tile {
    x: i32,
    z: i32,
    level: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Booth {
    tile: Tile,
    id: i32,
    distance: i32,
    name: Option<String>,
    action: Option<String>,
    actions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApproachFact {
    loc_id: i32,
    tile: Tile,
    can_operate: bool,
    dest: Option<Tile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Booth,
    Nearest,
    NearestWorld,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    WaitStand,
    WaitSelected,
    WaitNearest,
    WaitReady,
    WaitFresh,
}

/// What one decision left to do: keep the row, or settle with the frozen
/// boolean.
enum Decision {
    Running,
    Done(bool),
}

struct Observation {
    ingame: bool,
    here: Option<Tile>,
    bank_open: bool,
    bank_loaded: bool,
    bank_generation: u64,
    nearest_booth: Option<Booth>,
    locs: Vec<Booth>,
    locs_populated: bool,
    has_booth_stands: bool,
    approaches: Vec<ApproachFact>,
}

impl Observation {
    /// A logout forgets the session: only pages posted since login count.
    fn from_scene(scene: &Scene) -> Self {
        let session = scene.since_login();
        let tile = |t: observed::Tile| Tile {
            x: t.x,
            z: t.z,
            level: t.level,
        };
        Self {
            ingame: session.ingame().unwrap_or(false),
            here: session.here().map(tile),
            bank_open: session.bank_open().unwrap_or(false),
            bank_loaded: session.bank_loaded().unwrap_or(false),
            bank_generation: session.bank_generation().unwrap_or(0),
            nearest_booth: session.nearest_booth().map(|booth| Booth {
                tile: tile(booth.tile),
                id: booth.id,
                distance: i32::MAX,
                name: None,
                action: None,
                actions: Vec::new(),
            }),
            locs: session
                .locs()
                .map(|rows| rows.iter().filter_map(bank_candidate).collect())
                .unwrap_or_default(),
            locs_populated: session.locs().is_some(),
            has_booth_stands: session.has_booth_stands().unwrap_or(false),
            approaches: session
                .bank_approaches()
                .map(|rows| {
                    rows.iter()
                        .map(|row| ApproachFact {
                            loc_id: row.loc_id,
                            tile: tile(row.tile),
                            can_operate: row.can_operate,
                            dest: row.dest.map(tile),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

fn bank_candidate(row: &SceneRow) -> Option<Booth> {
    let name = row.name.as_deref()?;
    if !name
        .as_bytes()
        .windows(4)
        .any(|word| word.eq_ignore_ascii_case(b"bank"))
    {
        return None;
    }
    let actions = row
        .actions
        .iter()
        .filter(|action| !action.is_empty() && &***action != "hidden")
        .map(|action| action.to_string())
        .collect();
    Some(Booth {
        tile: Tile {
            x: row.x,
            z: row.z,
            level: row.level,
        },
        id: row.id,
        distance: row.distance,
        name: Some(name.to_string()),
        action: None,
        actions,
    })
}

/// The caller's start: the mode, an optional stand tile and the booth's
/// name/action. The typed tile is decoded here so an out-of-range one
/// fails closed as `invalid-stand` instead of a decode error.
#[derive(Deserialize)]
pub(crate) struct BankOpenArgs {
    #[serde(default)]
    mode: String,
    #[serde(default)]
    stand: Option<Value>,
    #[serde(default)]
    booth_name: Option<String>,
    #[serde(default)]
    booth_action: Option<String>,
}

/// One open: the row is the whole attempt's state.
pub(crate) struct BankOpen {
    mode: Mode,
    phase: Phase,
    stand: Option<Tile>,
    selected: Option<Booth>,
    wanted_name: Option<String>,
    wanted_action: Option<String>,
    open_generation: u64,
    last_approach_dest: Option<Tile>,
}

impl BankOpen {
    fn new(mode: Mode, wanted_name: Option<String>, wanted_action: Option<String>) -> Self {
        Self {
            mode,
            phase: Phase::WaitReady,
            stand: None,
            selected: None,
            wanted_name,
            wanted_action,
            open_generation: 0,
            last_approach_dest: None,
        }
    }

    fn wait_ready(&mut self, cx: &mut Cx<'_>) {
        self.phase = Phase::WaitReady;
        cx.clock().arm(BANK_READY_MS);
    }

    fn walk_near(&mut self, phase: Phase, tile: Tile, radius: i32, cx: &mut Cx<'_>) {
        self.phase = phase;
        cx.clock().arm(WALK_BOUND_MS);
        cx.emit(walk_near_req(tile, radius));
    }

    fn walk_approach(&mut self, dest: Tile, start_bound: bool, cx: &mut Cx<'_>) {
        self.phase = Phase::WaitSelected;
        // A re-walk inside the same approach keeps its bound: only the
        // first leg (or the first walk of a new row) arms it.
        if start_bound || cx.clock().deadline.is_none() {
            cx.clock().arm(WALK_BOUND_MS);
        }
        self.last_approach_dest = Some(dest);
        cx.emit(walk_near_req(dest, 0));
    }

    fn open(&mut self, booth: Booth, obs: &Observation, cx: &mut Cx<'_>) {
        self.open_generation = obs.bank_generation;
        self.phase = Phase::WaitFresh;
        cx.clock().arm(BANK_READY_MS);
        cx.emit(InteractReq::OpenBooth {
            x: booth.tile.x,
            z: booth.tile.z,
            level: booth.tile.level,
            id: booth.id,
            name: booth.name,
            action: booth.action,
        });
    }

    fn begin_sequence(
        &mut self,
        stand: Option<Tile>,
        stand_invalid: bool,
        obs: &Observation,
        cx: &mut Cx<'_>,
    ) -> Decision {
        self.stand = stand;
        if !obs.ingame {
            return Decision::Done(false);
        }
        if obs.bank_open {
            if obs.bank_loaded {
                return Decision::Done(true);
            }
            self.wait_ready(cx);
            return Decision::Running;
        }
        if stand_invalid {
            return Decision::Done(false);
        }
        match self.mode {
            Mode::Booth => {
                if let Some(stand) = stand {
                    if !near(obs.here, stand) {
                        self.walk_near(Phase::WaitStand, stand, ACCESS_RADIUS, cx);
                        return Decision::Running;
                    }
                }
                self.select_and_open(obs, cx)
            }
            Mode::Nearest => {
                let Some(selected) = self.select_named(obs) else {
                    return Decision::Done(false);
                };
                self.selected = Some(selected);
                self.approach_or_open(obs, cx)
            }
            Mode::NearestWorld => {
                if let Some(booth) = obs.nearest_booth.clone() {
                    if near(obs.here, booth.tile) {
                        self.selected = Some(unnamed(booth));
                        return self.approach_or_open(obs, cx);
                    }
                }
                if obs.here.is_none() || !obs.has_booth_stands {
                    return Decision::Done(false);
                }
                self.phase = Phase::WaitNearest;
                cx.clock().arm(WALK_BOUND_MS);
                cx.emit(InteractReq::WalkNearestBank);
                Decision::Running
            }
        }
    }

    fn select_named(&self, obs: &Observation) -> Option<Booth> {
        let name = self.wanted_name.as_deref()?;
        let action = self.wanted_action.as_deref()?;
        let mut selected = obs
            .locs
            .iter()
            .filter(|row| {
                row.id >= 0
                    && row
                        .name
                        .as_deref()
                        .is_some_and(|got| got.eq_ignore_ascii_case(name))
                    && row
                        .actions
                        .iter()
                        .any(|got| got.eq_ignore_ascii_case(action))
                    && obs.here.is_none_or(|here| here.level == row.tile.level)
            })
            .min_by_key(|row| row.distance)
            .cloned()?;
        selected.name = Some(name.to_string());
        selected.action = Some(action.to_string());
        Some(selected)
    }

    fn select_and_open(&mut self, obs: &Observation, cx: &mut Cx<'_>) -> Decision {
        let selected = if self.wanted_name.is_some() || self.wanted_action.is_some() {
            self.select_named(obs)
        } else {
            obs.nearest_booth.clone().map(unnamed)
        };
        let Some(selected) = selected.filter(|row| row.id >= 0) else {
            return Decision::Done(false);
        };
        self.selected = Some(selected);
        self.approach_or_open(obs, cx)
    }

    fn live_approach<'a>(&self, obs: &'a Observation) -> Option<&'a ApproachFact> {
        let selected = self.selected.as_ref()?;
        obs.approaches
            .iter()
            .find(|row| row.loc_id == selected.id && row.tile == selected.tile)
    }

    fn approach_or_open(&mut self, obs: &Observation, cx: &mut Cx<'_>) -> Decision {
        let Some(selected) = self.selected.clone() else {
            return Decision::Done(false);
        };
        match self.live_approach(obs).cloned() {
            None => Decision::Done(false),
            Some(row) if row.can_operate => {
                self.open(selected, obs, cx);
                Decision::Running
            }
            Some(row) => match row.dest {
                Some(dest) => {
                    self.walk_approach(dest, true, cx);
                    Decision::Running
                }
                None => Decision::Done(false),
            },
        }
    }

    fn selected_still_present(&self, obs: &Observation) -> bool {
        let Some(selected) = self.selected.as_ref() else {
            return false;
        };
        if selected.name.is_none() && selected.action.is_none() {
            return if obs.locs_populated {
                obs.locs
                    .iter()
                    .any(|row| row.id == selected.id && row.tile == selected.tile)
            } else {
                obs.nearest_booth
                    .as_ref()
                    .is_some_and(|row| row.id == selected.id && row.tile == selected.tile)
            };
        }
        obs.locs.iter().any(|row| {
            row.id == selected.id
                && row.tile == selected.tile
                && same_optional_text(row.name.as_deref(), selected.name.as_deref())
                && selected.action.as_deref().is_some_and(|action| {
                    row.actions
                        .iter()
                        .any(|got| got.eq_ignore_ascii_case(action))
                })
                && obs.here.is_none_or(|here| here.level == row.tile.level)
        })
    }

    fn step_phase(&mut self, obs: &Observation, cx: &mut Cx<'_>) -> Decision {
        if !obs.ingame {
            return Decision::Done(false);
        }
        match self.phase {
            Phase::WaitReady if obs.bank_open && obs.bank_loaded => Decision::Done(true),
            Phase::WaitFresh
                if obs.bank_open
                    && obs.bank_loaded
                    && obs.bank_generation > self.open_generation =>
            {
                Decision::Done(true)
            }
            Phase::WaitStand if obs.bank_open => {
                if obs.bank_loaded {
                    Decision::Done(true)
                } else {
                    self.wait_ready(cx);
                    Decision::Running
                }
            }
            Phase::WaitStand if self.stand.is_some_and(|stand| near(obs.here, stand)) => {
                self.select_and_open(obs, cx)
            }
            Phase::WaitSelected if obs.bank_open => {
                if obs.bank_loaded {
                    Decision::Done(true)
                } else {
                    self.wait_ready(cx);
                    Decision::Running
                }
            }
            Phase::WaitSelected => {
                if !self.selected_still_present(obs) {
                    return Decision::Done(false);
                }
                match self.live_approach(obs).cloned() {
                    None => Decision::Done(false),
                    Some(row) if row.can_operate => {
                        let selected = self.selected.take().expect("selected booth");
                        self.open(selected, obs, cx);
                        Decision::Running
                    }
                    Some(_) if cx.clock().bound_reached() => Decision::Done(false),
                    Some(row) => match row.dest {
                        None => Decision::Done(false),
                        Some(dest) if self.last_approach_dest == Some(dest) => Decision::Running,
                        Some(dest) => {
                            self.walk_approach(dest, false, cx);
                            Decision::Running
                        }
                    },
                }
            }
            Phase::WaitNearest if obs.bank_open => {
                if obs.bank_loaded {
                    Decision::Done(true)
                } else {
                    self.wait_ready(cx);
                    Decision::Running
                }
            }
            Phase::WaitNearest => {
                if let Some(booth) = obs.nearest_booth.clone() {
                    if near(obs.here, booth.tile) {
                        self.selected = Some(unnamed(booth));
                        return self.approach_or_open(obs, cx);
                    }
                }
                if cx.clock().bound_reached() {
                    Decision::Done(false)
                } else {
                    Decision::Running
                }
            }
            Phase::WaitReady | Phase::WaitFresh | Phase::WaitStand => {
                if cx.clock().bound_reached() {
                    Decision::Done(false)
                } else {
                    Decision::Running
                }
            }
        }
    }
}

impl Family for BankOpen {
    const NAME: &'static str = "bank_open";
    /// One open at a time: a newer start ends the older row `superseded`
    /// and its await settles false. The frozen surface has no guard.
    const EXCLUSIVE: bool = true;
    type Args = BankOpenArgs;
    type Output = bool;

    fn begin(args: BankOpenArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let obs = observed::with(Observation::from_scene);
        let stand = read_tile(args.stand.as_ref());
        let stand_invalid =
            args.stand.as_ref().is_some_and(|value| !value.is_null()) && stand.is_none();
        let mut open = Self::new(parse_mode(&args.mode), args.booth_name, args.booth_action);
        match open.begin_sequence(stand, stand_invalid, &obs, cx) {
            Decision::Running => Begin::Run(open),
            Decision::Done(ok) => Begin::Done(ok),
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        let obs = observed::with(Observation::from_scene);
        match self.step_phase(&obs, cx) {
            Decision::Running => Step::Wait,
            Decision::Done(ok) => Step::Done(ok),
        }
    }
}

fn walk_near_req(tile: Tile, radius: i32) -> InteractReq {
    InteractReq::WalkNear {
        x: tile.x,
        z: tile.z,
        level: tile.level,
        radius,
        allow_teleports: false,
        allow_wilderness: true,
        allow_bank_fetch: true,
        request_id: 0,
    }
}

fn unnamed(mut booth: Booth) -> Booth {
    booth.name = None;
    booth.action = None;
    booth
}

fn near(here: Option<Tile>, target: Tile) -> bool {
    here.is_some_and(|here| distance(Some(here), target) <= ACCESS_RADIUS)
}

fn distance(here: Option<Tile>, target: Tile) -> i32 {
    let Some(here) = here else {
        return i32::MAX;
    };
    if here.level != target.level {
        return i32::MAX;
    }
    i32::try_from(here.x.abs_diff(target.x).max(here.z.abs_diff(target.z))).unwrap_or(i32::MAX)
}

fn same_optional_text(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        (None, None) => true,
        _ => false,
    }
}

fn parse_mode(mode: &str) -> Mode {
    match mode {
        "open-nearest" => Mode::Nearest,
        "open-nearest-world" => Mode::NearestWorld,
        _ => Mode::Booth,
    }
}

fn read_tile(value: Option<&Value>) -> Option<Tile> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    Some(Tile {
        x: i32::try_from(value.get("x")?.as_i64()?).ok()?,
        z: i32::try_from(value.get("z")?.as_i64()?).ok()?,
        level: i32::try_from(value.get("level")?.as_i64()?).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::{self, Called, Js, Outcome, Pending, Reply, Started, Take};
    use serde_json::json;
    use std::rc::Rc;

    /// The bank-open family never calls a script callback.
    struct NoJs;

    impl Js for NoJs {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(
            &mut self,
            _hook: Option<&crate::load::callback_v8::HeldCallback>,
            _args: &[Value],
        ) -> Called {
            panic!("the bank-open family calls no script callback");
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            panic!("the bank-open family calls no script callback");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    fn booth_row(id: i32, x: i32, z: i32, distance: i32, name: &str, actions: &[&str]) -> SceneRow {
        SceneRow {
            id,
            name: Some(Rc::from(name)),
            x,
            z,
            level: 0,
            distance,
            actions: actions.iter().map(|action| Rc::from(*action)).collect(),
        }
    }

    fn approach(
        loc_id: i32,
        x: i32,
        z: i32,
        can_operate: bool,
        dest: Option<(i32, i32)>,
    ) -> observed::BankApproach {
        observed::BankApproach {
            loc_id,
            tile: observed::Tile { x, z, level: 0 },
            can_operate,
            dest: dest.map(|(x, z)| observed::Tile { x, z, level: 0 }),
        }
    }

    /// The posted scene one attempt decides from: a booth at 3011,3354 and
    /// the player standing at 3010,3352.
    fn post_scene(locs: Vec<SceneRow>, approaches: Vec<observed::BankApproach>) {
        observed::post(0, |post| {
            post.session(true)
                .here(observed::Tile {
                    x: 3010,
                    z: 3352,
                    level: 0,
                })
                .bank_generation(7)
                .locs(locs)
                .bank_approaches(approaches);
        });
    }

    fn named_booth() -> Vec<SceneRow> {
        vec![booth_row(
            2213,
            3011,
            3354,
            2,
            "Bank booth",
            &["Use-quickly"],
        )]
    }

    fn named_approach(dest: Option<(i32, i32)>) -> Vec<observed::BankApproach> {
        vec![approach(2213, 3011, 3354, false, dest)]
    }

    fn reset() {
        machine::on_reset();
        observed::on_reset();
    }

    fn tick() {
        machine::step(&mut NoJs);
    }

    fn drain() -> Vec<InteractReq> {
        machine::merge_ops(Vec::new())
    }

    fn start(args: Value) -> Started {
        machine::start("bank_open", args, Vec::new(), 0)
    }

    fn named_nearest() -> Value {
        json!({
            "mode": "open-nearest",
            "stand": null,
            "booth_name": "Bank booth",
            "booth_action": "Use-quickly",
        })
    }

    fn running(started: Started) -> machine::Handle {
        match started {
            Started::Running(handle) => handle,
            other => panic!("expected a running row, got {other:?}"),
        }
    }

    fn done(handle: machine::Handle) -> Value {
        match machine::take(handle) {
            Take::Settled(Outcome::Done(value)) => value,
            other => panic!("expected a done outcome, got {other:?}"),
        }
    }

    fn walk_near_dest() -> InteractReq {
        InteractReq::WalkNear {
            x: 3011,
            z: 3353,
            level: 0,
            radius: 0,
            allow_teleports: false,
            allow_wilderness: true,
            allow_bank_fetch: true,
            request_id: 0,
        }
    }

    fn named_open_booth() -> InteractReq {
        InteractReq::OpenBooth {
            x: 3011,
            z: 3354,
            level: 0,
            id: 2213,
            name: Some("Bank booth".into()),
            action: Some("Use-quickly".into()),
        }
    }

    #[test]
    fn walk_and_ready_bounds_are_native() {
        assert_eq!(WALK_BOUND_MS, 60_000);
        assert_eq!(BANK_READY_MS, 5_000);
    }

    #[test]
    fn out_of_range_stand_fails_closed() {
        reset();
        observed::post(0, |post| {
            post.session(true);
        });
        assert_eq!(
            start(json!({
                "mode": "open-booth",
                "stand": {"x": 2_147_483_648_i64, "z": 3355, "level": 0},
            })),
            Started::Settled(Outcome::Done(json!(false))),
            "an unreadable stand is not a walk"
        );
        assert!(drain().is_empty());
    }

    #[test]
    fn named_approach_walks_radius_zero_dest_not_loc_tile() {
        reset();
        post_scene(named_booth(), named_approach(Some((3011, 3353))));
        let handle = running(start(named_nearest()));
        assert_eq!(drain(), vec![walk_near_dest()]);
        assert_eq!(machine::take(handle), Take::Pending);
    }

    #[test]
    fn ready_projected_can_operate_opens_without_walk() {
        reset();
        observed::post(0, |post| {
            post.session(true)
                .here(observed::Tile {
                    x: 3011,
                    z: 3353,
                    level: 0,
                })
                .bank_generation(7)
                .locs(named_booth())
                .bank_approaches(vec![approach(2213, 3011, 3354, true, Some((3011, 3353)))]);
        });
        assert!(
            matches!(start(named_nearest()), Started::Running(_)),
            "a projected can_operate opens without walking"
        );
        assert_eq!(drain(), vec![named_open_booth()]);
    }

    #[test]
    fn a_ready_bank_settles_true_without_a_verb() {
        reset();
        observed::post(0, |post| {
            post.session(true)
                .here(observed::Tile {
                    x: 3010,
                    z: 3352,
                    level: 0,
                })
                .bank_open(true)
                .bank_loaded(true)
                .bank_generation(7);
        });
        assert_eq!(
            start(named_nearest()),
            Started::Settled(Outcome::Done(json!(true)))
        );
        assert!(drain().is_empty());
    }

    #[test]
    fn missing_model_and_empty_dest_fail_closed() {
        reset();
        post_scene(named_booth(), Vec::new());
        assert_eq!(
            start(named_nearest()),
            Started::Settled(Outcome::Done(json!(false))),
            "no projected approach is unreachable"
        );
        assert!(drain().is_empty());

        post_scene(named_booth(), named_approach(None));
        assert_eq!(
            start(named_nearest()),
            Started::Settled(Outcome::Done(json!(false))),
            "a projected approach without a dest is unreachable"
        );
        assert!(drain().is_empty());
    }

    #[test]
    fn walk_timeout_and_reset_fail_closed_without_another_command() {
        reset();
        post_scene(named_booth(), named_approach(Some((3011, 3353))));
        let handle = running(start(named_nearest()));
        assert_eq!(drain(), vec![walk_near_dest()]);
        machine::age(handle, WALK_BOUND_MS + 1);
        tick();
        assert_eq!(done(handle), json!(false));
        tick();
        assert!(drain().is_empty(), "a timed-out walk sends nothing more");

        let handle = running(start(named_nearest()));
        assert_eq!(drain(), vec![walk_near_dest()]);
        machine::on_reset();
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Aborted(machine::AbortReason::Reset))
        );
        tick();
        assert!(drain().is_empty(), "an aborted row never walks again");
    }

    #[test]
    fn dest_change_rewalks_without_renewing_the_bound() {
        reset();
        post_scene(named_booth(), named_approach(Some((3011, 3353))));
        let handle = running(start(named_nearest()));
        assert_eq!(drain(), vec![walk_near_dest()]);
        // The approach's bound is 60s from the begin; spend half of it.
        machine::age(handle, WALK_BOUND_MS / 2);

        post_scene(named_booth(), named_approach(Some((3013, 3355))));
        tick();
        assert_eq!(
            drain(),
            vec![InteractReq::WalkNear {
                x: 3013,
                z: 3355,
                level: 0,
                radius: 0,
                allow_teleports: false,
                allow_wilderness: true,
                allow_bank_fetch: true,
                request_id: 0,
            }],
            "a new dest re-walks inside the same bound"
        );
        assert_eq!(
            machine::take(handle),
            Take::Pending,
            "the re-walk keeps the row"
        );

        // Spend the rest of the original bound: a re-walk that renewed it
        // would still have half a window left here.
        machine::age(handle, WALK_BOUND_MS / 2 + 1);
        tick();
        assert_eq!(
            done(handle),
            json!(false),
            "the re-walk must not renew the approach bound"
        );
    }

    #[test]
    fn a_dest_change_after_the_bound_times_out_without_another_walk() {
        reset();
        post_scene(named_booth(), named_approach(Some((3011, 3353))));
        let handle = running(start(named_nearest()));
        assert_eq!(drain(), vec![walk_near_dest()]);
        machine::age(handle, WALK_BOUND_MS + 1);

        post_scene(named_booth(), named_approach(Some((3013, 3355))));
        tick();
        assert_eq!(done(handle), json!(false));
        assert!(drain().is_empty(), "an expired bound never walks again");
    }

    #[test]
    fn identity_lost_after_the_approach_sends_no_click() {
        reset();
        post_scene(named_booth(), named_approach(Some((3011, 3353))));
        let handle = running(start(named_nearest()));
        assert_eq!(drain(), vec![walk_near_dest()]);

        observed::post(0, |post| {
            post.here(observed::Tile {
                x: 3011,
                z: 3353,
                level: 0,
            })
            .locs(vec![booth_row(
                999,
                3011,
                3354,
                1,
                "Bank booth",
                &["Use-quickly"],
            )])
            .bank_approaches(named_approach(Some((3011, 3353))));
        });
        tick();
        assert_eq!(done(handle), json!(false));
        assert!(drain().is_empty(), "a replaced loc never gets a click");
    }

    #[test]
    fn pause_and_hold_freeze_mid_approach() {
        reset();
        post_scene(named_booth(), named_approach(Some((3011, 3353))));
        let handle = running(start(named_nearest()));
        assert_eq!(drain(), vec![walk_near_dest()]);
        // Both freezes read the frozen clock, so a row that stepped while
        // paused or held would find its (already due) bound reached and
        // settle instead of waiting.
        machine::age(handle, WALK_BOUND_MS + 1);
        machine::on_pause();
        tick();
        assert_eq!(
            machine::take(handle),
            Take::Pending,
            "paused rows do not step"
        );
        machine::on_resume();
        machine::on_hold(true);
        tick();
        assert_eq!(
            machine::take(handle),
            Take::Pending,
            "held rows do not step"
        );
        machine::on_hold(false);
        tick();
        assert_eq!(
            done(handle),
            json!(false),
            "the row walks its bound out once it runs again"
        );
        assert!(drain().is_empty(), "no second walk after the bound");
    }

    #[test]
    fn world_mode_walks_through_the_native_verb_then_opens() {
        reset();
        observed::post(0, |post| {
            post.session(true)
                .here(observed::Tile {
                    x: 3000,
                    z: 3000,
                    level: 0,
                })
                .bank_generation(7)
                .has_booth_stands(true);
        });
        let handle = running(start(json!({ "mode": "open-nearest-world" })));
        assert_eq!(drain(), vec![InteractReq::WalkNearestBank]);

        observed::post(0, |post| {
            post.here(observed::Tile {
                x: 3011,
                z: 3353,
                level: 0,
            })
            .nearest_booth(observed::NearestBooth {
                tile: observed::Tile {
                    x: 3011,
                    z: 3354,
                    level: 0,
                },
                id: 2213,
            })
            .bank_approaches(vec![approach(
                2213,
                3011,
                3354,
                true,
                Some((3011, 3353)),
            )]);
        });
        tick();
        assert_eq!(
            drain(),
            vec![InteractReq::OpenBooth {
                x: 3011,
                z: 3354,
                level: 0,
                id: 2213,
                name: None,
                action: None,
            }],
            "the world walk opens the unnamed nearest booth"
        );
        assert_eq!(machine::take(handle), Take::Pending);
    }
}
