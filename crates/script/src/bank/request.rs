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

/// Optional menu access for a named world object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankSeatObjectAccess {
    pub(crate) name: String,
    pub(crate) op: String,
    pub(crate) open_first_name: Option<String>,
    pub(crate) open_first_op: Option<String>,
}

/// Optional banker conversation access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankSeatNpcAccess {
    pub(crate) name: String,
    pub(crate) op: String,
    pub(crate) choose: Option<String>,
}

/// Caller-selected bank location and its optional access method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankSeatDestination {
    pub(crate) tile: WorldTile,
    pub(crate) object_access: Option<BankSeatObjectAccess>,
    pub(crate) npc_access: Option<BankSeatNpcAccess>,
}

/// Owned arguments for one bank operation.
#[derive(Debug, Clone, PartialEq)]
pub struct BankSeatArgs {
    pub(crate) operation: BankSeatOperation,
    pub(crate) item_id: Option<i32>,
    pub(crate) item_name: Option<String>,
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
pub(crate) const HOOK_LOG: u32 = 1 << 0;
/// `DepositMatch` callback bits, in the fixed `match`, `log` order.
pub(crate) const DEPOSIT_MATCH_HOOK_MATCH: u32 = 1 << 0;
pub(crate) const DEPOSIT_MATCH_HOOK_LOG: u32 = 1 << 1;
/// `BankNearest` callback bits, in the fixed `deposit`, `afterDeposit`, `log` order.
pub(crate) const BANK_NEAREST_HOOK_DEPOSIT: u32 = 1 << 0;
pub(crate) const BANK_NEAREST_HOOK_AFTER_DEPOSIT: u32 = 1 << 1;
pub(crate) const BANK_NEAREST_HOOK_LOG: u32 = 1 << 2;

impl BankSeatArgs {
    /// Create one operation with its wire defaults.
    pub(crate) fn new(operation: BankSeatOperation) -> Self {
        Self {
            operation,
            item_id: None,
            item_name: None,
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

    /// Validate the operation, native family, callback mask, and every field
    /// before the request enters or leaves the FlatBuffer codec.
    pub(crate) fn validate(&self, kind: NativeKind) -> Result<(), String> {
        if self.operation == BankSeatOperation::None {
            return Err("bank seat start has no operation".into());
        }
        let expected_kind = if self.operation == BankSeatOperation::BankNearest {
            NativeKind::BankNearest
        } else {
            NativeKind::BankPlan
        };
        if kind != expected_kind {
            return Err(format!(
                "bank operation {:?} requires native kind {expected_kind:?}",
                self.operation
            ));
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
            return Err(format!(
                "bank operation {:?} does not accept an open-first object access",
                self.operation
            ));
        }
        if let Some(generation) = self.baseline_generation {
            if !generation.is_finite() || generation < 0.0 {
                return Err("bank snapshot baseline must be finite and nonnegative".into());
            }
        }
        if let Some(quantity) = self.quantity {
            if quantity.unsigned_abs() > MAX_SAFE_INTEGER as u64 {
                return Err("bank quantity is outside the safe integer range".into());
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
            BankSeatOperation::WithdrawLoad => F_ITEM_NAME,
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
            return Err(format!(
                "bank operation {:?} carries unused fields (mask {:#x})",
                self.operation,
                populated_fields & !allowed_fields
            ));
        }
        let allowed_hooks = hook_mask(self.operation);
        if self.hooks_present & !allowed_hooks != 0 {
            return Err(format!(
                "bank operation {:?} has unknown callback bits {:#x}",
                self.operation,
                self.hooks_present & !allowed_hooks
            ));
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
                return Err("OpenObject requires object_access".into());
            }
            BankSeatOperation::OpenNpc if self.npc_access.is_none() => {
                return Err("OpenNpc requires npc_access".into());
            }
            BankSeatOperation::OpenObject | BankSeatOperation::OpenNpc => {}
            BankSeatOperation::DepositLabel => {
                require(self.item_name.is_some(), "DepositLabel requires item_name")?;
                require(self.menu_op.is_some(), "DepositLabel requires menu_op")?;
            }
            BankSeatOperation::WithdrawLabel => {
                require(
                    self.item_id.is_some() ^ self.item_name.is_some(),
                    "WithdrawLabel requires exactly one of item_id and item_name",
                )?;
                if self.item_id.is_some() {
                    require(
                        self.menu_op.is_some() && self.quantity.is_none(),
                        "id-based WithdrawLabel requires menu_op and no quantity",
                    )?;
                } else {
                    require(
                        !(self.menu_op.is_some() && self.quantity.is_some()),
                        "name-based WithdrawLabel cannot combine menu_op and quantity",
                    )?;
                }
            }
            BankSeatOperation::WithdrawDelta => {
                require(
                    self.item_id.is_some() ^ self.item_name.is_some(),
                    "WithdrawDelta requires exactly one of item_id and item_name",
                )?;
                let quantity = self
                    .quantity
                    .ok_or_else(|| "WithdrawDelta requires quantity".to_string())?;
                require(
                    (1..=i64::from(i32::MAX)).contains(&quantity),
                    "WithdrawDelta quantity must be a positive i32",
                )?;
                require(
                    self.item_id.is_some() || self.lands_as_id.is_none(),
                    "name-based WithdrawDelta does not accept lands_as_id",
                )?;
            }
            BankSeatOperation::WithdrawLoad => {
                require(self.item_name.is_some(), "WithdrawLoad requires item_name")?;
            }
            BankSeatOperation::WaitSnapshotAfter => {
                require(
                    self.baseline_generation.is_some(),
                    "WaitSnapshotAfter requires baseline_generation",
                )?;
            }
        }
        if self.operation == BankSeatOperation::Select {
            if let Some(from) = self.stand {
                if !(0..4).contains(&from.level)
                    || !(0..=16_383).contains(&from.x)
                    || !(0..=16_383).contains(&from.z)
                {
                    return Err(
                        "Select origin is outside the supported bank-selection bounds".into(),
                    );
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

    fn validate_text_bounds(&self) -> Result<(), String> {
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
            return Err("bank start string count exceeds the start charge bound".into());
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

fn validate_access(access: &Option<BankSeatObjectAccess>) -> Result<(), String> {
    if let Some(access) = access {
        require(
            access.open_first_name.is_some() == access.open_first_op.is_some(),
            "object open-first name and op must be supplied together",
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
) -> Result<(), String> {
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
) -> Result<(), String> {
    add_text_bound(bytes, strings, &access.name)?;
    add_text_bound(bytes, strings, &access.op)?;
    if let Some(choose) = &access.choose {
        add_text_bound(bytes, strings, choose)?;
    }
    Ok(())
}

fn add_text_bound(bytes: &mut usize, strings: &mut usize, value: &str) -> Result<(), String> {
    *bytes = bytes
        .checked_add(value.len())
        .filter(|total| *total <= MAX_START_CHARGE)
        .ok_or_else(|| format!("bank start text exceeds {MAX_START_CHARGE} bytes"))?;
    *strings += 1;
    Ok(())
}

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

const MAX_START_CHARGE: usize = api::script_contract::MAX_START_CHARGE;
