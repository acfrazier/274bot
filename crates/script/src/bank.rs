//! Shared native bank selector and transfer machines.
//!
//! The Quester and other compiled cards consume the same snapshot-driven
//! transfer core; selection is delegated to the host's bounded bank-pick worker.
pub use crate::native_bank::{
    AccessKind, BankPickReceipt, BankPickRequest, BankStandAccess, Close, Deposit, DepositArgs,
    Open, OpenArgs, PickKind, Select, SelectArgs, SelectedBank, Withdraw, WithdrawArgs, Withdrawal,
};
