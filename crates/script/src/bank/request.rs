//! Typed banking arguments carried by native seat starts.
//!
//! The operation identifies both the accepted fields and the meaning of
//! `quantity`. Callback handles stay in the isolate; the wire carries only
//! their fixed operation-specific presence bits.

use crate::native::request::NativeKind;
use api::snapshot::WorldTile;

/// Native operation represented by one typed bank seat start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankSeatOperation {
    None,
    OpenBooth,
    OpenNearest,
    OpenNearestWorld,
    OpenObject,
    OpenNpc,
    OpenAccess,
    Open,
    Select,
    DepositLabel,
    WithdrawLabel,
    WithdrawDelta,
    WithdrawLoad,
    DepositAll,
    DepositKeep,
    DepositMatch,
    SetNoteMode,
    Close,
    WaitReady,
    WaitSnapshotAfter,
    BankNearest,
}
/// A failure while validating one native bank request.
#[derive(Debug, thiserror::Error)]
pub enum BankRequestError {
    #[error("bank seat start has no operation")]
    NoOperation,
    #[error("bank operation {operation:?} requires native kind {expected_kind:?}")]
    NativeKindMismatch {
        operation: BankSeatOperation,
        expected_kind: NativeKind,
    },
    #[error("object open-first name and op must be supplied together")]
    ObjectOpenFirstPair,
    #[error("bank operation {operation:?} does not accept an open-first object access")]
    OpenFirstNotAccepted { operation: BankSeatOperation },
    #[error("bank snapshot baseline must be finite and nonnegative")]
    InvalidSnapshotBaseline,
    #[error("bank quantity is outside the safe integer range")]
    UnsafeQuantity,
    #[error("bank operation {operation:?} carries unused fields (mask {mask:#x})")]
    UnusedFields {
        operation: BankSeatOperation,
        mask: u64,
    },
    #[error("bank operation {operation:?} has unknown callback bits {mask:#x}")]
    UnknownCallbackBits {
        operation: BankSeatOperation,
        mask: u32,
    },
    #[error("OpenObject requires object_access")]
    OpenObjectMissingAccess,
    #[error("OpenNpc requires npc_access")]
    OpenNpcMissingAccess,
    #[error("DepositLabel requires item_name")]
    DepositLabelMissingItemName,
    #[error("DepositLabel requires menu_op")]
    DepositLabelMissingMenuOp,
    #[error("WithdrawLabel requires exactly one of item_id and item_name")]
    WithdrawLabelNeedsOneSelector,
    #[error("id-based WithdrawLabel requires menu_op and no quantity")]
    IdWithdrawLabelNeedsMenuOp,
    #[error("name-based WithdrawLabel cannot combine menu_op and quantity")]
    NameWithdrawLabelConflicts,
    #[error("WithdrawDelta requires exactly one of item_id and item_name")]
    WithdrawDeltaNeedsOneSelector,
    #[error("WithdrawDelta requires quantity")]
    WithdrawDeltaMissingQuantity,
    #[error("WithdrawDelta quantity must be a positive i32")]
    WithdrawDeltaInvalidQuantity,
    #[error("name-based WithdrawDelta does not accept lands_as_id")]
    NameWithdrawDeltaHasLandsAsId,
    #[error("WithdrawLoad requires an item selector")]
    WithdrawLoadMissingSelector,
    #[error("WaitSnapshotAfter requires baseline_generation")]
    WaitSnapshotAfterMissingBaseline,
    #[error("Select origin is outside the supported bank-selection bounds")]
    SelectOriginOutOfBounds,
    #[error("bank start string count exceeds the start charge bound")]
    TooManyStrings,
    #[error("bank start text exceeds {max_bytes} bytes")]
    TextTooLong { max_bytes: usize },
}

/// Optional menu access for a named world object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankSeatObjectAccess {
    pub(crate) name: String,
    pub(crate) op: String,
    pub(crate) open_first_name: Option<String>,
    pub(crate) open_first_op: Option<String>,
}
impl BankSeatObjectAccess {
    /// Create one object access with its optional open-first pair.
    pub fn new(
        name: String,
        op: String,
        open_first_name: Option<String>,
        open_first_op: Option<String>,
    ) -> Self {
        Self {
            name,
            op,
            open_first_name,
            open_first_op,
        }
    }

    /// Return the object name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the object menu operation.
    pub fn op(&self) -> &str {
        &self.op
    }

    /// Return the optional name used to open the object first.
    pub fn open_first_name(&self) -> Option<&str> {
        self.open_first_name.as_deref()
    }

    /// Return the optional operation used to open the object first.
    pub fn open_first_op(&self) -> Option<&str> {
        self.open_first_op.as_deref()
    }
}

/// Optional banker conversation access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankSeatNpcAccess {
    pub(crate) name: String,
    pub(crate) op: String,
    pub(crate) choose: Option<String>,
}
impl BankSeatNpcAccess {
    /// Create one NPC access with its optional conversation choice.
    pub fn new(name: String, op: String, choose: Option<String>) -> Self {
        Self { name, op, choose }
    }

    /// Return the NPC name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return the NPC menu operation.
    pub fn op(&self) -> &str {
        &self.op
    }

    /// Return the optional conversation choice.
    pub fn choose(&self) -> Option<&str> {
        self.choose.as_deref()
    }
}

/// Caller-selected bank location and its optional access method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankSeatDestination {
    pub(crate) tile: WorldTile,
    pub(crate) object_access: Option<BankSeatObjectAccess>,
    pub(crate) npc_access: Option<BankSeatNpcAccess>,
}
impl BankSeatDestination {
    /// Create one bank destination with its optional access methods.
    pub fn new(
        tile: WorldTile,
        object_access: Option<BankSeatObjectAccess>,
        npc_access: Option<BankSeatNpcAccess>,
    ) -> Self {
        Self {
            tile,
            object_access,
            npc_access,
        }
    }

    /// Return the destination tile.
    pub fn tile(&self) -> WorldTile {
        self.tile
    }

    /// Return the optional object access.
    pub fn object_access(&self) -> Option<&BankSeatObjectAccess> {
        self.object_access.as_ref()
    }

    /// Return the optional NPC access.
    pub fn npc_access(&self) -> Option<&BankSeatNpcAccess> {
        self.npc_access.as_ref()
    }
}

/// Select one bank item for `withdrawLoad`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WithdrawLoadSelector {
    /// Select the row by its object ID.
    Id(i32),
    /// Select the row by its display name.
    Name(String),
}

/// Owned arguments for one bank operation.
#[derive(Debug, Clone, PartialEq)]
pub struct BankSeatArgs {
    pub(crate) operation: BankSeatOperation,
    pub(crate) item_id: Option<i32>,
    pub(crate) item_name: Option<String>,
    pub(crate) withdraw_load_selector: Option<WithdrawLoadSelector>,
    pub(crate) menu_op: Option<String>,
    pub(crate) quantity: Option<i64>,
    pub(crate) lands_as_id: Option<i32>,
    pub(crate) stand: Option<WorldTile>,
    pub(crate) destination: Option<BankSeatDestination>,
    pub(crate) object_access: Option<BankSeatObjectAccess>,
    pub(crate) npc_access: Option<BankSeatNpcAccess>,
    pub(crate) fallback_access: Option<BankSeatObjectAccess>,
    pub(crate) return_to: Option<WorldTile>,
    pub(crate) keep_names: Vec<String>,
    pub(crate) timeout_ms: Option<f64>,
    pub(crate) baseline_generation: Option<f64>,
    pub(crate) prefer_nearby: Option<bool>,
    pub(crate) nearby_radius: Option<f64>,
    pub(crate) obstacles: Vec<String>,
    pub(crate) allow_wilderness: bool,
    pub(crate) use_mage_bank: Option<bool>,
    pub(crate) use_zanaris_bank: Option<bool>,
    pub(crate) note_mode: bool,
    pub(crate) deposit_all: bool,
    pub(crate) common_junk: bool,
    pub(crate) hooks_present: u32,
}

/// Hook presence bit for methods with one synchronous `log` callback.
pub const HOOK_LOG: u32 = 1 << 0;
/// `DepositMatch` callback bits, in the fixed `match`, `log` order.
pub const DEPOSIT_MATCH_HOOK_MATCH: u32 = 1 << 0;
pub const DEPOSIT_MATCH_HOOK_LOG: u32 = 1 << 1;
/// `BankNearest` callback bits, in the fixed `deposit`, `afterDeposit`, `log` order.
pub const BANK_NEAREST_HOOK_DEPOSIT: u32 = 1 << 0;
pub const BANK_NEAREST_HOOK_AFTER_DEPOSIT: u32 = 1 << 1;
pub const BANK_NEAREST_HOOK_LOG: u32 = 1 << 2;

impl BankSeatArgs {
    /// Create one operation with its wire defaults.
    pub fn new(operation: BankSeatOperation) -> Self {
        Self {
            operation,
            item_id: None,
            item_name: None,
            withdraw_load_selector: None,
            menu_op: None,
            quantity: None,
            lands_as_id: None,
            stand: None,
            destination: None,
            object_access: None,
            npc_access: None,
            fallback_access: None,
            return_to: None,
            keep_names: Vec::new(),
            timeout_ms: None,
            baseline_generation: None,
            prefer_nearby: None,
            nearby_radius: None,
            obstacles: Vec::new(),
            allow_wilderness: false,
            use_mage_bank: None,
            use_zanaris_bank: None,
            note_mode: false,
            deposit_all: false,
            common_junk: true,
            hooks_present: 0,
        }
    }
    /// Return the operation represented by this request.
    pub fn operation(&self) -> BankSeatOperation {
        self.operation
    }

    /// Return the optional item ID.
    pub fn item_id(&self) -> Option<i32> {
        self.item_id
    }

    /// Set the optional item ID.
    pub fn set_item_id(&mut self, item_id: Option<i32>) {
        self.item_id = item_id;
    }

    /// Return the optional item name.
    pub fn item_name(&self) -> Option<&str> {
        self.item_name.as_deref()
    }

    /// Set the optional item name.
    pub fn set_item_name(&mut self, item_name: Option<String>) {
        self.item_name = item_name;
    }

    /// Return the optional `withdrawLoad` selector.
    pub fn withdraw_load_selector(&self) -> Option<&WithdrawLoadSelector> {
        self.withdraw_load_selector.as_ref()
    }

    /// Set the optional `withdrawLoad` selector.
    pub fn set_withdraw_load_selector(&mut self, selector: Option<WithdrawLoadSelector>) {
        self.withdraw_load_selector = selector;
    }

    /// Return the optional menu operation.
    pub fn menu_op(&self) -> Option<&str> {
        self.menu_op.as_deref()
    }

    /// Set the optional menu operation.
    pub fn set_menu_op(&mut self, menu_op: Option<String>) {
        self.menu_op = menu_op;
    }

    /// Return the optional withdrawal quantity.
    pub fn quantity(&self) -> Option<i64> {
        self.quantity
    }

    /// Set the optional withdrawal quantity.
    pub fn set_quantity(&mut self, quantity: Option<i64>) {
        self.quantity = quantity;
    }

    /// Return the optional ID that receives a withdrawal.
    pub fn lands_as_id(&self) -> Option<i32> {
        self.lands_as_id
    }

    /// Set the optional ID that receives a withdrawal.
    pub fn set_lands_as_id(&mut self, lands_as_id: Option<i32>) {
        self.lands_as_id = lands_as_id;
    }

    /// Return the optional origin tile.
    pub fn stand(&self) -> Option<WorldTile> {
        self.stand
    }

    /// Set the optional origin tile.
    pub fn set_stand(&mut self, stand: Option<WorldTile>) {
        self.stand = stand;
    }

    /// Return the optional destination.
    pub fn destination(&self) -> Option<&BankSeatDestination> {
        self.destination.as_ref()
    }

    /// Set the optional destination.
    pub fn set_destination(&mut self, destination: Option<BankSeatDestination>) {
        self.destination = destination;
    }

    /// Return the optional object access.
    pub fn object_access(&self) -> Option<&BankSeatObjectAccess> {
        self.object_access.as_ref()
    }

    /// Set the optional object access.
    pub fn set_object_access(&mut self, object_access: Option<BankSeatObjectAccess>) {
        self.object_access = object_access;
    }

    /// Return the optional NPC access.
    pub fn npc_access(&self) -> Option<&BankSeatNpcAccess> {
        self.npc_access.as_ref()
    }

    /// Set the optional NPC access.
    pub fn set_npc_access(&mut self, npc_access: Option<BankSeatNpcAccess>) {
        self.npc_access = npc_access;
    }

    /// Return the optional fallback object access.
    pub fn fallback_access(&self) -> Option<&BankSeatObjectAccess> {
        self.fallback_access.as_ref()
    }

    /// Set the optional fallback object access.
    pub fn set_fallback_access(&mut self, fallback_access: Option<BankSeatObjectAccess>) {
        self.fallback_access = fallback_access;
    }

    /// Return the optional return tile.
    pub fn return_to(&self) -> Option<WorldTile> {
        self.return_to
    }

    /// Set the optional return tile.
    pub fn set_return_to(&mut self, return_to: Option<WorldTile>) {
        self.return_to = return_to;
    }

    /// Return the item names to keep.
    pub fn keep_names(&self) -> &[String] {
        &self.keep_names
    }

    /// Set the item names to keep.
    pub fn set_keep_names(&mut self, keep_names: Vec<String>) {
        self.keep_names = keep_names;
    }

    /// Return the optional timeout.
    pub fn timeout_ms(&self) -> Option<f64> {
        self.timeout_ms
    }

    /// Set the optional timeout.
    pub fn set_timeout_ms(&mut self, timeout_ms: Option<f64>) {
        self.timeout_ms = timeout_ms;
    }

    /// Return the optional snapshot baseline.
    pub fn baseline_generation(&self) -> Option<f64> {
        self.baseline_generation
    }

    /// Set the optional snapshot baseline.
    pub fn set_baseline_generation(&mut self, baseline_generation: Option<f64>) {
        self.baseline_generation = baseline_generation;
    }

    /// Return the optional nearby-bank preference.
    pub fn prefer_nearby(&self) -> Option<bool> {
        self.prefer_nearby
    }

    /// Set the optional nearby-bank preference.
    pub fn set_prefer_nearby(&mut self, prefer_nearby: Option<bool>) {
        self.prefer_nearby = prefer_nearby;
    }

    /// Return the optional nearby-bank radius.
    pub fn nearby_radius(&self) -> Option<f64> {
        self.nearby_radius
    }

    /// Set the optional nearby-bank radius.
    pub fn set_nearby_radius(&mut self, nearby_radius: Option<f64>) {
        self.nearby_radius = nearby_radius;
    }

    /// Return the obstacle names.
    pub fn obstacles(&self) -> &[String] {
        &self.obstacles
    }

    /// Set the obstacle names.
    pub fn set_obstacles(&mut self, obstacles: Vec<String>) {
        self.obstacles = obstacles;
    }

    /// Return whether wilderness banks are allowed.
    pub fn allow_wilderness(&self) -> bool {
        self.allow_wilderness
    }

    /// Set whether wilderness banks are allowed.
    pub fn set_allow_wilderness(&mut self, allow_wilderness: bool) {
        self.allow_wilderness = allow_wilderness;
    }

    /// Return the optional mage-bank preference.
    pub fn use_mage_bank(&self) -> Option<bool> {
        self.use_mage_bank
    }

    /// Set the optional mage-bank preference.
    pub fn set_use_mage_bank(&mut self, use_mage_bank: Option<bool>) {
        self.use_mage_bank = use_mage_bank;
    }

    /// Return the optional Zanaris-bank preference.
    pub fn use_zanaris_bank(&self) -> Option<bool> {
        self.use_zanaris_bank
    }

    /// Set the optional Zanaris-bank preference.
    pub fn set_use_zanaris_bank(&mut self, use_zanaris_bank: Option<bool>) {
        self.use_zanaris_bank = use_zanaris_bank;
    }

    /// Return whether withdrawals use note mode.
    pub fn note_mode(&self) -> bool {
        self.note_mode
    }

    /// Set whether withdrawals use note mode.
    pub fn set_note_mode(&mut self, note_mode: bool) {
        self.note_mode = note_mode;
    }

    /// Return whether the operation deposits all items.
    pub fn deposit_all(&self) -> bool {
        self.deposit_all
    }

    /// Set whether the operation deposits all items.
    pub fn set_deposit_all(&mut self, deposit_all: bool) {
        self.deposit_all = deposit_all;
    }

    /// Return whether common junk is kept.
    pub fn common_junk(&self) -> bool {
        self.common_junk
    }

    /// Set whether common junk is kept.
    pub fn set_common_junk(&mut self, common_junk: bool) {
        self.common_junk = common_junk;
    }

    /// Return the callback-presence mask.
    pub fn hooks_present(&self) -> u32 {
        self.hooks_present
    }

    /// Set the callback-presence mask.
    pub fn set_hooks_present(&mut self, hooks_present: u32) {
        self.hooks_present = hooks_present;
    }

    /// Validate the operation, native family, callback mask, and every field
    /// before the request enters or leaves the FlatBuffer codec.
    pub fn validate(&self, kind: NativeKind) -> Result<(), BankRequestError> {
        if self.operation == BankSeatOperation::None {
            return Err(BankRequestError::NoOperation);
        }
        let expected_kind = if self.operation == BankSeatOperation::BankNearest {
            NativeKind::BankNearest
        } else {
            NativeKind::BankPlan
        };
        if kind != expected_kind {
            return Err(BankRequestError::NativeKindMismatch {
                operation: self.operation,
                expected_kind,
            });
        }
        self.validate_text_bounds()?;
        validate_access(&self.object_access)?;
        validate_access(&self.fallback_access)?;
        if let Some(destination) = &self.destination {
            validate_access(&destination.object_access)?;
        }
        if self.object_access.as_ref().is_some_and(has_open_first)
            && matches!(
                self.operation,
                BankSeatOperation::OpenBooth
                    | BankSeatOperation::OpenNearest
                    | BankSeatOperation::Open
                    | BankSeatOperation::BankNearest
            )
        {
            return Err(BankRequestError::OpenFirstNotAccepted {
                operation: self.operation,
            });
        }
        if let Some(generation) = self.baseline_generation {
            if !generation.is_finite() || generation < 0.0 {
                return Err(BankRequestError::InvalidSnapshotBaseline);
            }
        }
        if let Some(quantity) = self.quantity {
            if quantity.unsigned_abs() > MAX_SAFE_INTEGER as u64 {
                return Err(BankRequestError::UnsafeQuantity);
            }
        }

        let allowed_fields = match self.operation {
            BankSeatOperation::None => 0,
            BankSeatOperation::OpenBooth => F_STAND | F_OBJECT_ACCESS,
            BankSeatOperation::OpenNearest => F_OBJECT_ACCESS,
            BankSeatOperation::OpenNearestWorld => 0,
            BankSeatOperation::OpenObject => F_OBJECT_ACCESS | F_HOOKS,
            BankSeatOperation::OpenNpc => F_NPC_ACCESS | F_HOOKS,
            BankSeatOperation::OpenAccess => {
                F_OBJECT_ACCESS | F_NPC_ACCESS | F_FALLBACK_ACCESS | F_HOOKS
            }
            BankSeatOperation::Open => {
                F_STAND
                    | F_DESTINATION
                    | F_OBJECT_ACCESS
                    | F_PREFER_NEARBY
                    | F_NEARBY_RADIUS
                    | F_OBSTACLES
                    | F_HOOKS
            }
            BankSeatOperation::Select => {
                F_STAND | F_ALLOW_WILDERNESS | F_USE_MAGE_BANK | F_USE_ZANARIS_BANK
            }
            BankSeatOperation::DepositLabel => F_ITEM_NAME | F_MENU_OP,
            BankSeatOperation::WithdrawLabel => F_ITEM_ID | F_ITEM_NAME | F_MENU_OP | F_QUANTITY,
            BankSeatOperation::WithdrawDelta => {
                F_ITEM_ID | F_ITEM_NAME | F_QUANTITY | F_LANDS_AS_ID
            }
            BankSeatOperation::WithdrawLoad => F_WITHDRAW_LOAD_SELECTOR,
            BankSeatOperation::DepositAll => 0,
            BankSeatOperation::DepositKeep => F_KEEP_NAMES,
            BankSeatOperation::DepositMatch => F_HOOKS,
            BankSeatOperation::SetNoteMode => F_NOTE_MODE,
            BankSeatOperation::Close => F_TIMEOUT_MS,
            BankSeatOperation::WaitReady => F_TIMEOUT_MS | F_HOOKS,
            BankSeatOperation::WaitSnapshotAfter => F_TIMEOUT_MS | F_BASELINE_GENERATION,
            BankSeatOperation::BankNearest => {
                F_DESTINATION
                    | F_OBJECT_ACCESS
                    | F_RETURN_TO
                    | F_DEPOSIT_ALL
                    | F_COMMON_JUNK
                    | F_HOOKS
            }
        };
        let populated_fields = self.populated_fields();
        if populated_fields & !allowed_fields != 0 {
            return Err(BankRequestError::UnusedFields {
                operation: self.operation,
                mask: populated_fields & !allowed_fields,
            });
        }
        let allowed_hooks = hook_mask(self.operation);
        if self.hooks_present & !allowed_hooks != 0 {
            return Err(BankRequestError::UnknownCallbackBits {
                operation: self.operation,
                mask: self.hooks_present & !allowed_hooks,
            });
        }
        match self.operation {
            BankSeatOperation::None => unreachable!("checked above"),
            BankSeatOperation::OpenBooth
            | BankSeatOperation::OpenNearest
            | BankSeatOperation::OpenNearestWorld
            | BankSeatOperation::OpenAccess
            | BankSeatOperation::Open
            | BankSeatOperation::Select
            | BankSeatOperation::DepositAll
            | BankSeatOperation::DepositKeep
            | BankSeatOperation::DepositMatch
            | BankSeatOperation::SetNoteMode
            | BankSeatOperation::Close
            | BankSeatOperation::WaitReady
            | BankSeatOperation::BankNearest => {}
            BankSeatOperation::OpenObject if self.object_access.is_none() => {
                return Err(BankRequestError::OpenObjectMissingAccess);
            }
            BankSeatOperation::OpenNpc if self.npc_access.is_none() => {
                return Err(BankRequestError::OpenNpcMissingAccess);
            }
            BankSeatOperation::OpenObject | BankSeatOperation::OpenNpc => {}
            BankSeatOperation::DepositLabel => {
                require(
                    self.item_name.is_some(),
                    BankRequestError::DepositLabelMissingItemName,
                )?;
                require(
                    self.menu_op.is_some(),
                    BankRequestError::DepositLabelMissingMenuOp,
                )?;
            }
            BankSeatOperation::WithdrawLabel => {
                require(
                    self.item_id.is_some() ^ self.item_name.is_some(),
                    BankRequestError::WithdrawLabelNeedsOneSelector,
                )?;
                if self.item_id.is_some() {
                    require(
                        self.menu_op.is_some() && self.quantity.is_none(),
                        BankRequestError::IdWithdrawLabelNeedsMenuOp,
                    )?;
                } else {
                    require(
                        !(self.menu_op.is_some() && self.quantity.is_some()),
                        BankRequestError::NameWithdrawLabelConflicts,
                    )?;
                }
            }
            BankSeatOperation::WithdrawDelta => {
                require(
                    self.item_id.is_some() ^ self.item_name.is_some(),
                    BankRequestError::WithdrawDeltaNeedsOneSelector,
                )?;
                let quantity = self
                    .quantity
                    .ok_or(BankRequestError::WithdrawDeltaMissingQuantity)?;
                require(
                    (1..=i64::from(i32::MAX)).contains(&quantity),
                    BankRequestError::WithdrawDeltaInvalidQuantity,
                )?;
                require(
                    self.item_id.is_some() || self.lands_as_id.is_none(),
                    BankRequestError::NameWithdrawDeltaHasLandsAsId,
                )?;
            }
            BankSeatOperation::WithdrawLoad => {
                require(
                    self.withdraw_load_selector.is_some(),
                    BankRequestError::WithdrawLoadMissingSelector,
                )?;
            }
            BankSeatOperation::WaitSnapshotAfter => {
                require(
                    self.baseline_generation.is_some(),
                    BankRequestError::WaitSnapshotAfterMissingBaseline,
                )?;
            }
        }
        if self.operation == BankSeatOperation::Select {
            if let Some(from) = self.stand {
                if !(0..4).contains(&from.level)
                    || !(0..=16_383).contains(&from.x)
                    || !(0..=16_383).contains(&from.z)
                {
                    return Err(BankRequestError::SelectOriginOutOfBounds);
                }
            }
        }
        Ok(())
    }

    fn populated_fields(&self) -> u64 {
        let mut fields = 0;
        macro_rules! some {
            ($($field:ident => $bit:ident),+ $(,)?) => {
                $(if self.$field.is_some() { fields |= $bit; })+
            };
        }
        some!(
            item_id => F_ITEM_ID,
            item_name => F_ITEM_NAME,
            withdraw_load_selector => F_WITHDRAW_LOAD_SELECTOR,
            menu_op => F_MENU_OP,
            quantity => F_QUANTITY,
            lands_as_id => F_LANDS_AS_ID,
            stand => F_STAND,
            destination => F_DESTINATION,
            object_access => F_OBJECT_ACCESS,
            npc_access => F_NPC_ACCESS,
            fallback_access => F_FALLBACK_ACCESS,
            return_to => F_RETURN_TO,
            timeout_ms => F_TIMEOUT_MS,
            baseline_generation => F_BASELINE_GENERATION,
            prefer_nearby => F_PREFER_NEARBY,
            nearby_radius => F_NEARBY_RADIUS,
            use_mage_bank => F_USE_MAGE_BANK,
            use_zanaris_bank => F_USE_ZANARIS_BANK,
        );
        if !self.keep_names.is_empty() {
            fields |= F_KEEP_NAMES;
        }
        if !self.obstacles.is_empty() {
            fields |= F_OBSTACLES;
        }
        if self.allow_wilderness {
            fields |= F_ALLOW_WILDERNESS;
        }
        if self.note_mode {
            fields |= F_NOTE_MODE;
        }
        if self.deposit_all {
            fields |= F_DEPOSIT_ALL;
        }
        // `common_junk` defaults to true on the wire and in `new`.
        if !self.common_junk {
            fields |= F_COMMON_JUNK;
        }
        if self.hooks_present != 0 {
            fields |= F_HOOKS;
        }
        fields
    }

    fn validate_text_bounds(&self) -> Result<(), BankRequestError> {
        let mut bytes = 0usize;
        let mut strings = 0usize;
        macro_rules! text {
            ($value:expr) => {
                if let Some(value) = $value {
                    add_text_bound(&mut bytes, &mut strings, value)?;
                }
            };
        }
        text!(self.item_name.as_deref());
        if let Some(WithdrawLoadSelector::Name(name)) = &self.withdraw_load_selector {
            add_text_bound(&mut bytes, &mut strings, name)?;
        }
        text!(self.menu_op.as_deref());
        if let Some(access) = &self.object_access {
            add_object_text(&mut bytes, &mut strings, access)?;
        }
        if let Some(access) = &self.fallback_access {
            add_object_text(&mut bytes, &mut strings, access)?;
        }
        if let Some(access) = &self.npc_access {
            add_npc_text(&mut bytes, &mut strings, access)?;
        }
        if let Some(destination) = &self.destination {
            if let Some(access) = &destination.object_access {
                add_object_text(&mut bytes, &mut strings, access)?;
            }
            if let Some(access) = &destination.npc_access {
                add_npc_text(&mut bytes, &mut strings, access)?;
            }
        }
        for name in &self.keep_names {
            add_text_bound(&mut bytes, &mut strings, name)?;
        }
        for obstacle in &self.obstacles {
            add_text_bound(&mut bytes, &mut strings, obstacle)?;
        }
        if strings > MAX_START_CHARGE / 16 {
            return Err(BankRequestError::TooManyStrings);
        }
        Ok(())
    }
}

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const F_ITEM_ID: u64 = 1 << 0;
const F_ITEM_NAME: u64 = 1 << 1;
const F_MENU_OP: u64 = 1 << 2;
const F_QUANTITY: u64 = 1 << 3;
const F_LANDS_AS_ID: u64 = 1 << 4;
const F_STAND: u64 = 1 << 5;
const F_DESTINATION: u64 = 1 << 6;
const F_OBJECT_ACCESS: u64 = 1 << 7;
const F_NPC_ACCESS: u64 = 1 << 8;
const F_FALLBACK_ACCESS: u64 = 1 << 9;
const F_RETURN_TO: u64 = 1 << 10;
const F_KEEP_NAMES: u64 = 1 << 11;
const F_TIMEOUT_MS: u64 = 1 << 12;
const F_BASELINE_GENERATION: u64 = 1 << 13;
const F_PREFER_NEARBY: u64 = 1 << 14;
const F_NEARBY_RADIUS: u64 = 1 << 15;
const F_OBSTACLES: u64 = 1 << 16;
const F_ALLOW_WILDERNESS: u64 = 1 << 17;
const F_USE_MAGE_BANK: u64 = 1 << 18;
const F_USE_ZANARIS_BANK: u64 = 1 << 19;
const F_NOTE_MODE: u64 = 1 << 20;
const F_DEPOSIT_ALL: u64 = 1 << 21;
const F_COMMON_JUNK: u64 = 1 << 22;
const F_HOOKS: u64 = 1 << 23;
const F_WITHDRAW_LOAD_SELECTOR: u64 = 1 << 24;

fn hook_mask(operation: BankSeatOperation) -> u32 {
    match operation {
        BankSeatOperation::OpenObject
        | BankSeatOperation::OpenNpc
        | BankSeatOperation::OpenAccess
        | BankSeatOperation::Open
        | BankSeatOperation::WaitReady => HOOK_LOG,
        BankSeatOperation::DepositMatch => DEPOSIT_MATCH_HOOK_MATCH | DEPOSIT_MATCH_HOOK_LOG,
        BankSeatOperation::BankNearest => {
            BANK_NEAREST_HOOK_DEPOSIT | BANK_NEAREST_HOOK_AFTER_DEPOSIT | BANK_NEAREST_HOOK_LOG
        }
        _ => 0,
    }
}

fn validate_access(access: &Option<BankSeatObjectAccess>) -> Result<(), BankRequestError> {
    if let Some(access) = access {
        require(
            access.open_first_name.is_some() == access.open_first_op.is_some(),
            BankRequestError::ObjectOpenFirstPair,
        )?;
    }
    Ok(())
}

fn has_open_first(access: &BankSeatObjectAccess) -> bool {
    access.open_first_name.is_some() || access.open_first_op.is_some()
}

fn add_object_text(
    bytes: &mut usize,
    strings: &mut usize,
    access: &BankSeatObjectAccess,
) -> Result<(), BankRequestError> {
    add_text_bound(bytes, strings, &access.name)?;
    add_text_bound(bytes, strings, &access.op)?;
    if let Some(name) = &access.open_first_name {
        add_text_bound(bytes, strings, name)?;
    }
    if let Some(op) = &access.open_first_op {
        add_text_bound(bytes, strings, op)?;
    }
    Ok(())
}

fn add_npc_text(
    bytes: &mut usize,
    strings: &mut usize,
    access: &BankSeatNpcAccess,
) -> Result<(), BankRequestError> {
    add_text_bound(bytes, strings, &access.name)?;
    add_text_bound(bytes, strings, &access.op)?;
    if let Some(choose) = &access.choose {
        add_text_bound(bytes, strings, choose)?;
    }
    Ok(())
}

fn add_text_bound(
    bytes: &mut usize,
    strings: &mut usize,
    value: &str,
) -> Result<(), BankRequestError> {
    *bytes = bytes
        .checked_add(value.len())
        .filter(|total| *total <= MAX_START_CHARGE)
        .ok_or(BankRequestError::TextTooLong {
            max_bytes: MAX_START_CHARGE,
        })?;
    *strings += 1;
    Ok(())
}

fn require(condition: bool, error: BankRequestError) -> Result<(), BankRequestError> {
    if condition {
        Ok(())
    } else {
        Err(error)
    }
}

const MAX_START_CHARGE: usize = api::script_contract::MAX_START_CHARGE;
