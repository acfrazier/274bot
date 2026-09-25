//! Frozen Banking.open precedence, shared by periodic banking and world opens.
//! Local access remains local; only the no-scene/no-preset fallback selects by
//! walk cost. Selection and access are separate awaited Rust continuations.
use crate::bank_access::{AccessArgs, BankAccess, NpcAccess, NpcAccessArgs, Opener};
use crate::bank_open::BankOpen;
use crate::bank_op::BankView;
use crate::bank_select::{self, SelectArgs, SelectBank};
use crate::machine::{Begin, Cx, Family, Step, Thrown};
use crate::observed;
use crate::walk::Resilient;
use api::{named_banks::NamedBank, snapshot::WorldTile};
use serde::Deserialize;
use serde_json::Value;

pub(crate) enum Access { Object(AccessArgs), Npc(NpcAccessArgs) }
pub(crate) struct Dest { tile: WorldTile, access: Option<Access> }
impl Dest {
    fn named(bank: NamedBank) -> Self {
        let access = bank.definition.and_then(|definition| {
            if let Some(npc) = definition.npc {
                Some(Access::Npc(NpcAccessArgs { name: npc.name.into(), op: npc.op.into(), choose: definition.choose.unwrap_or_default().into() }))
            } else {
                definition.object.map(|object| Access::Object(AccessArgs {
                    name: object.name.into(), op: object.op.into(),
                    open_first: definition.open_first.map(|open| Opener { name: open.name.into(), op: open.op.into() }),
                }))
            }
        });
        Self { tile: bank.tile, access }
    }
}

pub(crate) fn read_tile(value: &Value) -> Option<WorldTile> {
    let value = value.get("tile").unwrap_or(value);
    Some(WorldTile {
        x: i32::try_from(value.get("x")?.as_i64()?).ok()?,
        z: i32::try_from(value.get("z")?.as_i64()?).ok()?,
        level: value.get("level").and_then(Value::as_i64).and_then(|v| i32::try_from(v).ok()).unwrap_or(0),
    })
}
pub(crate) fn read_dest(value: &Value) -> Option<Dest> {
    let tile = read_tile(value)?;
    let access = match (value.get("npcAccess"), value.get("access")) {
        (Some(npc), _) if !npc.is_null() => Some(Access::Npc(serde_json::from_value(npc.clone()).ok()?)),
        (_, Some(object)) if !object.is_null() => Some(Access::Object(serde_json::from_value(object.clone()).ok()?)),
        _ => None,
    };
    Some(Dest { tile, access })
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BankingOpenArgs {
    #[serde(default)] stand: Value,
    #[serde(default)] destination: Value,
    #[serde(default)] pub(crate) booth_name: Option<String>,
    #[serde(default)] pub(crate) booth_op: Option<String>,
    #[serde(default)] prefer_nearby: Option<bool>,
    #[serde(default)] nearby_radius: Option<f64>,
    #[serde(default)] obstacles: Vec<String>,
}

enum Target { Access(Option<Access>), Preset(WorldTile) }
enum Phase {
    Choose,
    Select(SelectBank),
    Walk(Resilient, Target),
    Open(Target),
    Object(Box<BankAccess>),
    Npc(Box<NpcAccess>),
    Booth(Box<BankOpen>),
    Ready,
}
pub(crate) struct BankingOpen {
    phase: Phase,
    stand: Option<WorldTile>,
    destination: Option<Dest>,
    booth_name: String,
    booth_op: String,
    prefer_nearby: bool,
    nearby_radius: f64,
    unsupported_obstacles: bool,
    log_hook: Option<usize>,
}
impl BankingOpen {
    pub(crate) fn for_destination(destination: Option<Dest>, log_hook: Option<usize>) -> Self {
        let mut open = Self::new(BankingOpenArgs::default(), log_hook);
        open.destination = destination;
        open
    }
    pub(crate) fn new(args: BankingOpenArgs, log_hook: Option<usize>) -> Self {
        Self {
            phase: Phase::Choose, stand: read_tile(&args.stand), destination: read_dest(&args.destination),
            booth_name: args.booth_name.unwrap_or_else(|| "Bank booth".into()),
            booth_op: args.booth_op.unwrap_or_else(|| "Use-quickly".into()),
            prefer_nearby: args.prefer_nearby != Some(false), nearby_radius: args.nearby_radius.unwrap_or(14.),
            unsupported_obstacles: args.obstacles.iter().any(|name| !name.trim().is_empty()), log_hook,
        }
    }
    pub(crate) fn booth(&mut self, name: Option<String>, op: Option<String>) {
        if let Some(name) = name { self.booth_name = name; }
        if let Some(op) = op { self.booth_op = op; }
    }
    fn walk(&mut self, tile: WorldTile, radius: i32, target: Target, cx: &mut Cx<'_>) {
        self.phase = match Resilient::new(tile, radius, 120_000, None, false).start(cx) {
            Ok(walk) => Phase::Walk(walk, target),
            // Frozen Banking.open awaits the walk but does not use its bool.
            Err(_) => Phase::Open(target),
        };
    }
    fn walk_dest(&mut self, dest: Dest, cx: &mut Cx<'_>) {
        self.walk(dest.tile, 4, Target::Access(dest.access), cx);
    }
    fn choose(&mut self, cx: &mut Cx<'_>) -> Option<Step<bool>> {
        let bank = BankView::now();
        if bank.open {
            if bank.loaded { return Some(Step::Done(true)); }
            cx.clock().arm(crate::bank_open::BANK_READY_MS);
            self.phase = Phase::Ready;
            return Some(Step::Wait);
        }
        let (here, scene_booth) = observed::with(|scene| {
            let view = scene.since_login();
            let here = view.here().map(|t| WorldTile { x: t.x, z: t.z, level: t.level });
            let distance = view.locs().into_iter().flatten()
                .filter(|row| row.name.as_deref().is_some_and(|name| name.eq_ignore_ascii_case(&self.booth_name)))
                .filter(|row| row.actions.iter().any(|op| !op.is_empty() && op.as_ref() != "hidden"))
                .map(|row| row.distance).min();
            let nearest_booth = view.nearest_booth().filter(|booth| {
                booth.name.as_deref().is_some_and(|name| name.eq_ignore_ascii_case(&self.booth_name))
                    && booth.op.as_deref().is_some_and(|op| op.eq_ignore_ascii_case(&self.booth_op))
            }).and_then(|booth| here.filter(|here| here.level == booth.tile.level).map(|here| {
                here.x.abs_diff(booth.tile.x).max(here.z.abs_diff(booth.tile.z)) as i32
            }));
            let distance = distance.into_iter().chain(nearest_booth).min();
            (here, distance)
        });
        let nearest = here.and_then(bank_select::nearest_bank);
        if self.prefer_nearby && scene_booth.is_some_and(|distance| f64::from(distance) <= self.nearby_radius) {
            self.phase = Phase::Open(Target::Access(None));
        } else if let Some(bank) = nearest.filter(|bank| self.prefer_nearby && here.is_some_and(|here| {
            within(here, bank.tile, self.nearby_radius)
                && self.stand.is_none_or(|stand| !within(here, stand, self.nearby_radius))
        })) {
            self.walk_dest(Dest::named(bank), cx);
        } else if let Some(stand) = self.stand {
            // The previous Banking.open rejected obstacle policy. Keep that
            // capability fail-closed; this cutover does not invent a door loop.
            if self.unsupported_obstacles { return Some(Step::Fail(Thrown::new("not impl: Banking.open.obstacles"))); }
            let known = bank_select::nearest_bank(stand).filter(|bank| within(stand, bank.tile, self.nearby_radius));
            let target = match known.map(Dest::named).and_then(|dest| dest.access) {
                Some(access) => Target::Access(Some(access)),
                None => Target::Preset(stand),
            };
            self.walk(stand, 2, target, cx);
        } else if scene_booth.is_some() {
            self.phase = Phase::Open(Target::Access(None));
        } else if let Some(destination) = self.destination.take() {
            self.walk_dest(destination, cx);
        } else if let Some(select) = SelectBank::start(SelectArgs { allow_wilderness: true, ..Default::default() }, cx) {
            self.phase = Phase::Select(select);
        } else {
            self.phase = Phase::Open(Target::Access(None));
        }
        None
    }
    pub(crate) fn run(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        loop {
            match std::mem::replace(&mut self.phase, Phase::Choose) {
                Phase::Choose => if let Some(step) = self.choose(cx) { return step; },
                Phase::Select(select) => match select.result_bank(cx) {
                    None => { self.phase = Phase::Select(select); return Step::Wait; }
                    Some(Some(bank)) => self.walk_dest(Dest::named(bank), cx),
                    Some(None) => self.phase = Phase::Open(Target::Access(None)),
                },
                Phase::Walk(mut walk, target) => {
                    if walk.step(cx).is_none() { self.phase = Phase::Walk(walk, target); return Step::Wait; }
                    self.phase = Phase::Open(target);
                }
                Phase::Open(target) => match target {
                    Target::Access(access) => self.phase = match access {
                        Some(Access::Npc(args)) => Phase::Npc(Box::new(NpcAccess::new(args, self.log_hook))),
                        Some(Access::Object(args)) => Phase::Object(Box::new(BankAccess::new(args, self.log_hook))),
                        None => Phase::Object(Box::new(BankAccess::new(AccessArgs {
                            name: std::mem::take(&mut self.booth_name), op: std::mem::take(&mut self.booth_op), open_first: None,
                        }, self.log_hook))),
                    },
                    Target::Preset(stand) => match BankOpen::at_stand(stand,
                        std::mem::take(&mut self.booth_name), std::mem::take(&mut self.booth_op), cx) {
                        Begin::Run(open) => self.phase = Phase::Booth(Box::new(open)),
                        Begin::Done(ok) => return Step::Done(ok),
                        Begin::Refuse(_) => return Step::Done(false),
                    },
                },
                Phase::Object(mut open) => { let step = open.run(cx); self.phase = Phase::Object(open); return step; }
                Phase::Npc(mut open) => { let step = open.run(cx); self.phase = Phase::Npc(open); return step; }
                Phase::Booth(mut open) => { let step = open.step(cx); self.phase = Phase::Booth(open); return step; }
                Phase::Ready => {
                    let bank = BankView::now();
                    if bank.open && bank.loaded { return Step::Done(true); }
                    if cx.clock().bound_reached() { return Step::Done(false); }
                    self.phase = Phase::Ready;
                    return Step::Wait;
                }
            }
        }
    }
}
fn within(a: WorldTile, b: WorldTile, radius: f64) -> bool {
    radius >= 0. && (bank_select::air_distance_squared(a, b) as f64) <= radius * radius
}
impl Family for BankingOpen {
    const NAME: &'static str = "banking_open";
    const EXCLUSIVE: bool = true;
    const EXCLUSIVE_GROUP: &'static str = BankOpen::NAME;
    const CALLBACKS: &'static [&'static str] = &["log"];
    const SYNC_HOOKS: &'static [usize] = &[0];
    const KICK_ON_START: bool = true;
    type Args = BankingOpenArgs;
    type Output = bool;
    fn begin(args: Self::Args, _cx: &mut Cx<'_>) -> Begin<Self> { Begin::Run(Self::new(args, Some(0))) }
    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> { self.run(cx) }
}
