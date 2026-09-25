//! Select-only banking continuation. Routing belongs to the host worker;
//! this machine owns the waiter, never movement or a Navigator callback.
use crate::machine::{Begin, Cx, Family, Step};
use crate::{observed, shim::InteractReq};
use api::named_banks::{NamedBank, NamedBankFacts};
use api::snapshot::WorldTile;
use serde::Deserialize;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

thread_local! {
    static BANKS: RefCell<Arc<NamedBankFacts>> = RefCell::new(Arc::new(NamedBankFacts::empty()));
}
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn install(facts: Arc<NamedBankFacts>) {
    BANKS.with(|banks| *banks.borrow_mut() = facts);
}

pub(crate) fn bank_value(bank: &NamedBank) -> Value {
    json!({"name": bank.name, "tile": {"x": bank.tile.x, "z": bank.tile.z, "level": bank.tile.level}})
}

pub(crate) fn selected(index: i32) -> Value {
    BANKS.with(|banks| {
        usize::try_from(index).ok()
            .and_then(|index| banks.borrow().banks().get(index).map(bank_value))
            .unwrap_or(Value::Null)
    })
}

pub(crate) fn nearest(from: WorldTile) -> Value {
    BANKS.with(|banks| {
        banks.borrow().banks().iter()
            .min_by_key(|bank| air_distance_squared(from, bank.tile))
            .map(bank_value).unwrap_or(Value::Null)
    })
}

pub(crate) fn air_distance_squared(a: WorldTile, b: WorldTile) -> i64 {
    let dx = i64::from(a.x) - i64::from(b.x);
    let dz = i64::from(a.z) - i64::from(b.z);
    dx * dx + dz * dz
}

#[derive(Deserialize)]
pub(crate) struct FromTile {
    pub x: i32,
    pub z: i32,
    #[serde(default)]
    pub level: i32,
}

#[derive(Deserialize, Default)]
pub(crate) struct SelectArgs {
    #[serde(default)]
    pub from: Option<FromTile>,
    #[serde(default)]
    pub allow_wilderness: bool,
}

pub(crate) struct SelectBank {
    request_id: u64,
}

impl SelectBank {
    pub(crate) fn start(args: SelectArgs, cx: &mut Cx<'_>) -> Option<Self> {
        let from = args.from.or_else(|| observed::with(|scene| {
            scene.since_login().here().map(|t| FromTile { x: t.x, z: t.z, level: t.level })
        }))?;
        if !(0..4).contains(&from.level) || !(0..=16383).contains(&from.x) || !(0..=16383).contains(&from.z) {
            return None;
        }
        let request_id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        cx.emit(InteractReq::SelectBank {
            x: from.x, z: from.z, level: from.level,
            allow_wilderness: args.allow_wilderness, request_id,
        });
        Some(Self { request_id })
    }

    pub(crate) fn result(&self) -> Option<Value> {
        observed::with(|scene| {
            let result = scene.since_login().bank_selection()?;
            (result.request_id == self.request_id && result.kind != 0)
                .then(|| selected(result.bank_index))
        })
    }
}

impl Family for SelectBank {
    const NAME: &'static str = "bank_select";
    const EXCLUSIVE: bool = true;
    type Args = SelectArgs;
    type Output = Value;

    fn begin(args: Self::Args, cx: &mut Cx<'_>) -> Begin<Self> {
        match Self::start(args, cx) {
            Some(row) => Begin::Run(row),
            None => Begin::Done(Value::Null),
        }
    }

    fn step(&mut self, _cx: &mut Cx<'_>) -> Step<Value> {
        self.result().map_or(Step::Wait, Step::Done)
    }
}
