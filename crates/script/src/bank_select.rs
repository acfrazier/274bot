//! Select-only banking continuation. Routing belongs to the host worker;
//! this machine owns the waiter, never movement or a Navigator callback.
use crate::machine::{Begin, Cx, Family, Step};
use crate::{observed, shim::InteractReq};
use api::named_banks::{BankPreferences, NamedBank, NamedBankFacts};
use api::snapshot::WorldTile;
use serde::Deserialize;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

thread_local! {
    static BANKS: RefCell<Arc<NamedBankFacts>> = RefCell::new(Arc::new(NamedBankFacts::empty()));
    static PREFERENCES: Cell<BankPreferences> = Cell::default();
}
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn install(facts: Arc<NamedBankFacts>) {
    BANKS.with(|banks| *banks.borrow_mut() = facts);
}

pub(crate) fn settings(bag: &serde_json::Map<String, Value>) {
    PREFERENCES.set(BankPreferences {
        use_mage_bank: bag.get("useMageBank").and_then(Value::as_bool).unwrap_or(false),
        use_zanaris_bank: bag.get("useZanarisBank").and_then(Value::as_bool).unwrap_or(false),
    });
}

pub(crate) fn banks() -> Vec<NamedBank> {
    BANKS.with(|banks| banks.borrow().banks().to_vec())
}

fn eligible(bank: &NamedBank) -> bool {
    observed::with(|scene| {
        let view = scene.since_login();
        bank.eligible(
            |id| (id == 10).then(|| view.stats().and_then(|skills| skills.fishing).map(|skill| skill.base)).flatten(),
            |name| matches!(view.quest_statuses(), Some(observed::QuestTab::Bound(rows))
                if rows.iter().any(|row| row.name.as_ref() == name && row.status.as_ref() == "complete")),
            PREFERENCES.get(),
        )
    })
}

pub(crate) fn unlocked(name: &str, tile: WorldTile) -> bool {
    BANKS.with(|banks| banks.borrow().banks().iter()
        .any(|bank| bank.name == name && bank.tile == tile && eligible(bank)))
}

pub(crate) fn bank_value(bank: &NamedBank) -> Value {
    let mut value = json!({"name": bank.name, "tile": {"x": bank.tile.x, "z": bank.tile.z, "level": bank.tile.level}});
    if let Some(definition) = bank.definition {
        if let Some(approach) = definition.approach {
            value["approach"] = json!({"x": approach.x, "z": approach.z, "level": approach.level});
        }
        if let Some(npc) = definition.npc {
            value["npcAccess"] = json!({"name": npc.name, "op": npc.op});
            if let Some(choose) = definition.choose { value["npcAccess"]["choose"] = json!(choose); }
        } else if let Some(object) = definition.object {
            value["access"] = json!({"name": object.name, "op": object.op});
            if let Some(open) = definition.open_first {
                value["access"]["openFirst"] = json!({"name": open.name, "op": open.op});
            }
        }
        if definition.skill.is_some() || definition.quest.is_some() || definition.setting.is_some() {
            value["requires"] = json!({});
            if let Some((_, level)) = definition.skill { value["requires"]["skill"] = json!({"name": "fishing", "level": level}); }
            if let Some(quest) = definition.quest { value["requires"]["quest"] = json!(quest); }
            if let Some(setting) = definition.setting { value["requires"]["setting"] = json!(setting); }
        }
    }
    value
}

pub(crate) fn selected_bank(index: i32) -> Option<NamedBank> {
    BANKS.with(|banks| usize::try_from(index).ok()
        .and_then(|index| banks.borrow().banks().get(index).copied()))
}

pub(crate) fn selected(index: i32) -> Value {
    selected_bank(index).as_ref().map(bank_value).unwrap_or(Value::Null)
}

pub(crate) fn nearest_bank(from: WorldTile) -> Option<NamedBank> {
    BANKS.with(|banks| banks.borrow().banks().iter()
        .filter(|bank| eligible(bank))
        .min_by_key(|bank| air_distance_squared(from, bank.air_tile())).copied())
}

pub(crate) fn nearest(from: WorldTile) -> Value {
    nearest_bank(from).as_ref().map(bank_value).unwrap_or(Value::Null)
}

pub(crate) fn ranked(from: WorldTile) -> Vec<NamedBank> {
    let mut banks = banks();
    banks.retain(eligible);
    banks.sort_by_key(|bank| air_distance_squared(from, bank.air_tile()));
    banks
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
    #[serde(default)]
    pub use_mage_bank: Option<bool>,
    #[serde(default)]
    pub use_zanaris_bank: Option<bool>,
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
        let preferences = PREFERENCES.get();
        cx.emit(InteractReq::SelectBank {
            x: from.x, z: from.z, level: from.level,
            allow_wilderness: args.allow_wilderness, request_id,
            use_mage_bank: args.use_mage_bank.unwrap_or(preferences.use_mage_bank),
            use_zanaris_bank: args.use_zanaris_bank.unwrap_or(preferences.use_zanaris_bank),
        });
        Some(Self { request_id })
    }

    pub(crate) fn result_bank(&self) -> Option<Option<NamedBank>> {
        observed::with(|scene| {
            let result = scene.since_login().bank_selection()?;
            (result.request_id == self.request_id && result.kind != 0)
                .then(|| selected_bank(result.bank_index))
        })
    }

    pub(crate) fn result(&self) -> Option<Value> {
        self.result_bank().map(|bank| bank.as_ref().map(bank_value).unwrap_or(Value::Null))
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
