//! Shared native bank selector and transfer machines.
//!
//! The Quester and other compiled cards consume the same snapshot-driven
//! transfer core; selection is delegated to the host's bounded bank-pick worker.

pub(crate) mod npc;
pub mod ops;
pub mod request;

pub mod compat;
#[cfg(test)]
mod prompt_args_tests;
pub(crate) mod seat_open;
pub(crate) mod seat_transfer;

pub use crate::native_bank::{
    AccessKind, BankPickReceipt, BankPickRequest, BankStandAccess, Close, Deposit, DepositArgs,
    Open, OpenArgs, PickKind, Select, SelectArgs, SelectedBank, Withdraw, WithdrawArgs, Withdrawal,
};

/// One Rust-owned settings fact, shared by the V8 materializer and static
/// catalog extraction. Field order preserves the public settings order.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodicSettings {
    bank_strategy: StrategySetting,
    bank_every_items: NumberSetting,
    bank_every_minutes: NumberSetting,
    bank_common_junk: BooleanSetting,
}

#[derive(serde::Serialize)]
struct StrategySetting {
    #[serde(rename = "type")]
    kind: &'static str,
    default: &'static str,
    options: [&'static str; 4],
    label: &'static str,
    help: &'static str,
}

#[derive(serde::Serialize)]
struct NumberSetting {
    #[serde(rename = "type")]
    kind: &'static str,
    default: u32,
    min: u32,
    max: u32,
    label: &'static str,
}

#[derive(serde::Serialize)]
struct BooleanSetting {
    #[serde(rename = "type")]
    kind: &'static str,
    default: bool,
    label: &'static str,
}

pub const PERIODIC_SETTINGS: PeriodicSettings = PeriodicSettings {
    bank_strategy: StrategySetting {
        kind: "string",
        default: "Off",
        options: ["Off", "Loot count", "Time", "Either"],
        label: "Periodic bank",
        help: "save accumulated loot so a death does not lose it all",
    },
    bank_every_items: NumberSetting {
        kind: "number",
        default: 15,
        min: 1,
        max: 27,
        label: "Bank at N loot items",
    },
    bank_every_minutes: NumberSetting {
        kind: "number",
        default: 10,
        min: 1,
        max: 120,
        label: "Bank every N minutes",
    },
    bank_common_junk: BooleanSetting {
        kind: "boolean",
        default: true,
        label: "Also bank gems/fruit/beer/kebabs/caskets",
    },
};
