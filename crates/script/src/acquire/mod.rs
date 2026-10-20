//! Inventory-first acquisition shared by native consumers.
//!
//! Quantities are final held targets. A usable minimum can be below the
//! target. Bank hints require a visit before they can refuse a request.

pub mod plan;
pub mod run;

use crate::bank::ops::MAX_MEMO;
use crate::native::{ActionError, WalkOptions};
use crate::native_bank::{BankAction, BankReceipt, Withdrawal};
use api::bank_memory::Origin;
use api::game_data::SelectedGameData;
use api::named_banks::{BankPreferences, NamedBank};
use api::quest_progress::EvidenceStamp;
use api::snapshot::{ItemView, WorldTile};
use std::sync::Arc;

/// Items refused by the engine's `bank_check_allowed` content:
/// `content/scripts/interface_bank/scripts/bank.rs2:129-149` names the box,
/// anthems and monkey. Its `unbankable` parameter is true on the lamp in
/// `content/scripts/general/configs/xplamp.obj:13` and the toad in
/// `content/scripts/quests/quest_chompybird/configs/quest_chompybird.obj:74`.
/// The box refusal is silent and calls `try_add_macro_cube` before returning
/// false, so a missing transfer cannot distinguish it from an ordinary no-op.
pub(crate) const UNBANKABLE_ALIASES: &[&str] = &[
    "macro_cube",
    "misc_awful_anthem",
    "misc_good_anthem",
    "mm_monkey_in_backpack",
    "macro_genilamp",
    "bloated_toad",
];
const _: () = assert!(UNBANKABLE_ALIASES.len() <= u8::BITS as usize);

pub(crate) fn unbankable_index(selected: &SelectedGameData, item: &ItemView) -> Option<usize> {
    let alias = selected.item_by_id(item.def.id)?.alias.as_deref()?;
    UNBANKABLE_ALIASES.iter().position(|known| *known == alias)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Held,
    Carried,
    /// A carried item covers the need. A held item also requires one wear.
    Wielded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeedKind {
    Required,
    Optional,
}

/// Sources allowed for one need, independently of other needs in the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSet(u8);

impl SourceSet {
    pub const PACK: Self = Self(1);
    pub const WORN: Self = Self(2);
    pub const BANK: Self = Self(4);
    pub const SHOP: Self = Self(8);
    pub const CHILD: Self = Self(16);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for SourceSet {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

#[derive(Debug, Clone)]
pub struct Need {
    pub item: i32,
    pub name: Arc<str>,
    /// Final desired count, never an amount to add.
    pub target: i32,
    /// Smallest count that permits the consumer to continue.
    pub minimum: i32,
    pub presence: Presence,
    pub kind: NeedKind,
    pub sources: SourceSet,
    /// Acceptable ids in preference order. Empty means the exact item only.
    pub alternatives: Arc<[i32]>,
}

pub enum DepositSpec {
    Ids {
        products: Arc<[i32]>,
        keep: Arc<[i32]>,
    },
    Capacity {
        keep: Arc<[i32]>,
        slots: u8,
    },
}

pub struct BankChoice {
    pub bank: Option<NamedBank>,
    pub explicit: Option<Arc<str>>,
    pub required: bool,
    pub preferences: BankPreferences,
}

#[non_exhaustive]
pub enum BankStage {
    Planned,
    Authored(Arc<[BankAction]>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseBank {
    Close,
    LeaveOpen,
}

pub enum VendorChoice {
    None,
    Nearest,
    Explicit {
        keeper_config: Arc<str>,
        anchor: Option<WorldTile>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoinPolicy {
    None,
    ForPurchase { sources: SourceSet },
}

pub struct AcquireRequest {
    pub needs: Arc<[Need]>,
    pub deposit: Option<DepositSpec>,
    pub bank: BankChoice,
    pub stage: BankStage,
    pub partial_ok: bool,
    pub close: CloseBank,
    pub vendor: VendorChoice,
    pub coins: CoinPolicy,
    pub walk: WalkOptions,
    pub required_after: EvidenceStamp,
}

impl AcquireRequest {
    /// An authored bank sequence keeps the bank open for the next step.
    pub fn bank_actions(
        bank: Option<NamedBank>,
        required: bool,
        actions: Arc<[BankAction]>,
        partial_ok: bool,
        required_after: EvidenceStamp,
    ) -> Self {
        Self {
            needs: Arc::from([]),
            deposit: None,
            bank: BankChoice {
                bank,
                explicit: None,
                required,
                preferences: BankPreferences::default(),
            },
            stage: BankStage::Authored(actions),
            partial_ok,
            close: CloseBank::LeaveOpen,
            vendor: VendorChoice::None,
            coins: CoinPolicy::None,
            walk: WalkOptions::default(),
            required_after,
        }
    }
}

/// Immutable content and the current world's membership gate.
#[derive(Clone, Copy, Default)]
pub struct WorldFacts<'a> {
    /// Bank-only acquisition does not need selected shop facts.
    pub data: Option<&'a SelectedGameData>,
    pub members: bool,
}

#[expect(
    clippy::large_enum_variant,
    reason = "the bounded plan stays inline so planning does not allocate a box"
)]
pub enum AcquireVerdict {
    Satisfied,
    Plan(AcquirePlan),
    Blocked(AcquireBlock),
}

/// Fixed-size work produced by the incremental planner.
pub struct AcquirePlan {
    pub(crate) withdrawals: [Option<Withdrawal>; MAX_MEMO],
    pub(crate) withdrawal_count: usize,
    pub(crate) wears: [Option<i32>; MAX_MEMO],
    pub(crate) wear_count: usize,
    pub(crate) scan: bool,
    pub(crate) child: Option<u8>,
    pub(crate) authored: bool,
    pub looked: Looked,
}

impl AcquirePlan {
    pub(crate) fn empty() -> Self {
        Self {
            withdrawals: std::array::from_fn(|_| None),
            withdrawal_count: 0,
            wears: [None; MAX_MEMO],
            wear_count: 0,
            scan: false,
            child: None,
            authored: false,
            looked: Looked::default(),
        }
    }

    pub fn authored() -> Self {
        Self {
            authored: true,
            ..Self::empty()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockReason {
    AuthoredShortage,
    SupplyMissing,
    TooFewCoins {
        needed: i64,
        held: i64,
        banked: i64,
    },
    VendorUnreachable,
    VendorPickUndecided,
    NoVendor,
    NoStock {
        restock_ticks: i32,
        refused_at_tick: u64,
        aligned_to: Option<u64>,
        revisit_due: u64,
    },
    NoSpace {
        slots_needed: i32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankSeen {
    pub origin: Origin,
    pub observed_at_unix: u64,
    pub rows: u16,
    pub count: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VendorOutcome {
    Routed { cost: u32 },
    NoPath,
    Undecided,
    OutOfStock,
    MembersOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VendorTried {
    pub keeper_id: i32,
    pub outcome: VendorOutcome,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Looked {
    pub pack: i32,
    pub worn: i32,
    pub bank: Option<BankSeen>,
    pub vendors: [Option<VendorTried>; 3],
    pub more_vendors: u8,
    pub short_of_target: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquireBlock {
    pub item: Arc<str>,
    pub need: u8,
    pub reason: BlockReason,
    pub looked: Looked,
    pub message: Arc<str>,
}

impl AcquireBlock {
    pub fn code(&self) -> &'static str {
        match self.reason {
            BlockReason::AuthoredShortage | BlockReason::SupplyMissing => "supply-missing",
            BlockReason::TooFewCoins { .. } => "too-few-coins",
            BlockReason::VendorUnreachable => "vendor-unreachable",
            BlockReason::VendorPickUndecided => "vendor-pick-undecided",
            BlockReason::NoVendor => "no-vendor",
            BlockReason::NoStock { .. } => "no-stock",
            BlockReason::NoSpace { .. } => "no-space",
        }
    }

    pub fn action_error(&self) -> ActionError {
        ActionError::Blocked(Arc::clone(&self.message))
    }
}

pub enum AcquireOutcome {
    Done(AcquireReceipt),
    ChildDue { need: u8 },
}

pub struct AcquireReceipt {
    pub complete: bool,
    pub withdrew_any: bool,
    pub deposited: u32,
    pub kept_unbankable: bool,
    pub bank_open: bool,
    pub bought: u8,
    pub last_bank: Option<BankReceipt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenGate {
    Open,
    Hold,
}
