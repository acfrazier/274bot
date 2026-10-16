//! F0 request envelope and family policy. No seat dispatch is activated here.
//! Family arguments remain typed native values, not JS machine arguments.

use api::script_contract::{
    PausePolicy, ReconnectPolicy, Refusal, RequestClass, RequestStamp, SessionFamily,
    TakeoverPolicy, GATHER_SETTINGS_MAX_BYTES, HOOK_CHARGE, MAX_HOOKS, MAX_START_CHARGE,
    START_METADATA,
};

/// Semantic families in §25 R5-1.2/4.6/8.5, independent of JS aliases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeKind {
    Gather,
    Combat,
    Hunt,
    Quester,
    DeathRecovery,
    QuestProgress,
    Walk,
    BankPlan,
    BankNearest,
    PeriodicBank,
    Dialogue,
    MakeX,
    Shop,
    Trade,
    PartnerTrade,
    Teleport,
    Reach,
    Prayer,
    Autocast,
    Special,
    Modals,
    Fire,
    Clue,
    Duel,
    CakeStall,
}

/// A family has every required contract-table column by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FamilyPolicy {
    class: RequestClass,
    pause: PausePolicy,
    takeover: TakeoverPolicy,
    reconnect: ReconnectPolicy,
    sub_operation: Option<ReconnectPolicy>,
    commands: Option<ReconnectPolicy>,
    args_max_charge: usize,
    encoded_settings_max: Option<usize>,
    source: &'static str,
}
impl FamilyPolicy {
    pub const fn class(self) -> RequestClass {
        self.class
    }
    pub const fn pause(self) -> PausePolicy {
        self.pause
    }
    pub const fn takeover(self) -> TakeoverPolicy {
        self.takeover
    }
    pub const fn reconnect(self) -> ReconnectPolicy {
        self.reconnect
    }
    pub const fn sub_operation(self) -> Option<ReconnectPolicy> {
        self.sub_operation
    }
    pub const fn commands(self) -> Option<ReconnectPolicy> {
        self.commands
    }
    /// Maximum pending-start custody charge, not a fabricated encoding bound
    /// for a family that has not yet had its I-wire/I-family cutover.
    pub const fn args_max_charge(self) -> usize {
        self.args_max_charge
    }
    pub const fn encoded_settings_max(self) -> Option<usize> {
        self.encoded_settings_max
    }
    pub const fn source(self) -> &'static str {
        self.source
    }
}
impl NativeKind {
    pub const ALL: [Self; 25] = [
        Self::Gather,
        Self::Combat,
        Self::Hunt,
        Self::Quester,
        Self::DeathRecovery,
        Self::QuestProgress,
        Self::Walk,
        Self::BankPlan,
        Self::BankNearest,
        Self::PeriodicBank,
        Self::Dialogue,
        Self::MakeX,
        Self::Shop,
        Self::Trade,
        Self::PartnerTrade,
        Self::Teleport,
        Self::Reach,
        Self::Prayer,
        Self::Autocast,
        Self::Special,
        Self::Modals,
        Self::Fire,
        Self::Clue,
        Self::Duel,
        Self::CakeStall,
    ];
    /// Exhaustive classification keeps bankNearest and PeriodicBank as
    /// Capabilities, and quest progress as a Session (§25 R5-4.6).
    pub const fn policy(self) -> FamilyPolicy {
        let class = match self {
            Self::Gather => RequestClass::Session(SessionFamily::Gather),
            Self::Combat => RequestClass::Session(SessionFamily::Combat),
            Self::Hunt => RequestClass::Session(SessionFamily::Hunt),
            Self::Quester => RequestClass::Session(SessionFamily::Quester),
            Self::DeathRecovery => RequestClass::Session(SessionFamily::DeathRecovery),
            Self::QuestProgress => RequestClass::Session(SessionFamily::Progress),
            Self::Walk
            | Self::BankPlan
            | Self::BankNearest
            | Self::PeriodicBank
            | Self::Dialogue
            | Self::MakeX
            | Self::Shop
            | Self::Trade
            | Self::PartnerTrade
            | Self::Teleport
            | Self::Reach
            | Self::Prayer
            | Self::Autocast
            | Self::Special
            | Self::Modals
            | Self::Fire
            | Self::Clue
            | Self::Duel
            | Self::CakeStall => RequestClass::Capability,
        };
        FamilyPolicy {
            class,
            pause: match self {
                Self::Gather | Self::Combat => PausePolicy::CancelOperation,
                _ => PausePolicy::Freeze,
            },
            takeover: match self {
                Self::Gather => TakeoverPolicy::SignalGather,
                Self::Combat => TakeoverPolicy::CancelCombat,
                Self::Walk => TakeoverPolicy::InterruptWalk,
                _ => TakeoverPolicy::None,
            },
            reconnect: match self {
                Self::QuestProgress => ReconnectPolicy::Carry,
                _ => ReconnectPolicy::Resume,
            },
            sub_operation: match self {
                Self::Gather | Self::Combat | Self::Quester => Some(ReconnectPolicy::Interrupt),
                _ => None,
            },
            commands: match self {
                Self::Gather | Self::Combat | Self::QuestProgress | Self::Walk => {
                    Some(ReconnectPolicy::Carry)
                }
                _ => None,
            },
            args_max_charge: MAX_START_CHARGE,
            encoded_settings_max: match self {
                Self::Gather => Some(GATHER_SETTINGS_MAX_BYTES),
                _ => None,
            },
            source: "§25 R5-1.2, R5-4.6, R5-8.5; §28.4 pending custody",
        }
    }
}

/// Native family payload plus the shared identity/lineage header. `A` is the
/// family's owned Rust argument value. This is not the FlatBuffer or a JS value.
/// Existing typed arguments (WalkRequest, BankPickRequest, combat/gather requests)
/// remain where their real behavior lives; I-family binds them to this envelope.
#[derive(Debug)]
pub struct NativeRequest<A> {
    stamp: RequestStamp,
    kind: NativeKind,
    args: A,
}
impl<A> NativeRequest<A> {
    pub const fn new(stamp: RequestStamp, kind: NativeKind, args: A) -> Self {
        Self { stamp, kind, args }
    }
    pub const fn stamp(&self) -> RequestStamp {
        self.stamp
    }
    pub const fn kind(&self) -> NativeKind {
        self.kind
    }
    pub const fn args(&self) -> &A {
        &self.args
    }
    pub fn into_args(self) -> A {
        self.args
    }
}

/// One canonical serialized start buffer in adapter custody (§28.4).
/// No typed JSON/string clone is retained beside it. Callback handles themselves
/// stay in the adapter; this value records only their bounded charge.
#[derive(Debug)]
pub struct PendingPayload {
    bytes: Vec<u8>,
    handles: usize,
    charge: usize,
}
impl PendingPayload {
    /// Charge capacity rather than length, including metadata and hook roots.
    /// Gather's encoded-settings vector has its separate existing 4-KiB cap.
    pub fn new(
        bytes: Vec<u8>,
        handles: usize,
        gather_settings_bytes: Option<usize>,
    ) -> Result<Self, Refusal> {
        if handles > MAX_HOOKS
            || gather_settings_bytes.is_some_and(|size| size > GATHER_SETTINGS_MAX_BYTES)
        {
            return Err(Refusal::ArgsTooLarge);
        }
        let charge = bytes
            .capacity()
            .checked_add(START_METADATA)
            .and_then(|charge| charge.checked_add(handles * HOOK_CHARGE))
            .filter(|charge| *charge <= MAX_START_CHARGE)
            .ok_or(Refusal::ArgsTooLarge)?;
        Ok(Self {
            bytes,
            handles,
            charge,
        })
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub const fn handles(&self) -> usize {
        self.handles
    }
    pub const fn charge(&self) -> usize {
        self.charge
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::script_contract::{
        PausePolicy, ReconnectPolicy, RequestClass, SessionFamily, TakeoverPolicy,
        GATHER_SETTINGS_MAX_BYTES, MAX_START_CHARGE,
    };

    #[test]
    fn every_kind_policy_matches_r5_8_5_columns_including_walk_carry() {
        for kind in NativeKind::ALL {
            let policy = kind.policy();
            assert_eq!(policy.args_max_charge(), MAX_START_CHARGE);
            match kind {
                NativeKind::Gather => {
                    assert_eq!(policy.class(), RequestClass::Session(SessionFamily::Gather));
                    assert_eq!(policy.pause(), PausePolicy::CancelOperation);
                    assert_eq!(policy.takeover(), TakeoverPolicy::SignalGather);
                    assert_eq!(policy.reconnect(), ReconnectPolicy::Resume);
                    assert_eq!(policy.sub_operation(), Some(ReconnectPolicy::Interrupt));
                    assert_eq!(policy.commands(), Some(ReconnectPolicy::Carry));
                    assert_eq!(
                        policy.encoded_settings_max(),
                        Some(GATHER_SETTINGS_MAX_BYTES)
                    );
                }
                NativeKind::Combat => {
                    assert_eq!(policy.class(), RequestClass::Session(SessionFamily::Combat));
                    assert_eq!(policy.pause(), PausePolicy::CancelOperation);
                    assert_eq!(policy.takeover(), TakeoverPolicy::CancelCombat);
                    assert_eq!(policy.reconnect(), ReconnectPolicy::Resume);
                    assert_eq!(policy.sub_operation(), Some(ReconnectPolicy::Interrupt));
                    assert_eq!(policy.commands(), Some(ReconnectPolicy::Carry));
                    assert_eq!(policy.encoded_settings_max(), None);
                }
                NativeKind::Hunt => {
                    assert_eq!(policy.class(), RequestClass::Session(SessionFamily::Hunt));
                    assert_eq!(policy.pause(), PausePolicy::Freeze);
                    assert_eq!(policy.takeover(), TakeoverPolicy::None);
                    assert_eq!(policy.reconnect(), ReconnectPolicy::Resume);
                    assert_eq!(policy.sub_operation(), None);
                    assert_eq!(policy.commands(), None);
                    assert_eq!(policy.encoded_settings_max(), None);
                }
                NativeKind::Quester => {
                    assert_eq!(
                        policy.class(),
                        RequestClass::Session(SessionFamily::Quester)
                    );
                    assert_eq!(policy.pause(), PausePolicy::Freeze);
                    assert_eq!(policy.takeover(), TakeoverPolicy::None);
                    assert_eq!(policy.reconnect(), ReconnectPolicy::Resume);
                    assert_eq!(policy.sub_operation(), Some(ReconnectPolicy::Interrupt));
                    assert_eq!(policy.commands(), None);
                    assert_eq!(policy.encoded_settings_max(), None);
                }
                NativeKind::DeathRecovery => {
                    assert_eq!(
                        policy.class(),
                        RequestClass::Session(SessionFamily::DeathRecovery)
                    );
                    assert_eq!(policy.pause(), PausePolicy::Freeze);
                    assert_eq!(policy.takeover(), TakeoverPolicy::None);
                    assert_eq!(policy.reconnect(), ReconnectPolicy::Resume);
                    assert_eq!(policy.sub_operation(), None);
                    assert_eq!(policy.commands(), None);
                    assert_eq!(policy.encoded_settings_max(), None);
                }
                NativeKind::QuestProgress => {
                    assert_eq!(
                        policy.class(),
                        RequestClass::Session(SessionFamily::Progress)
                    );
                    assert_eq!(policy.pause(), PausePolicy::Freeze);
                    assert_eq!(policy.takeover(), TakeoverPolicy::None);
                    assert_eq!(policy.reconnect(), ReconnectPolicy::Carry);
                    assert_eq!(policy.sub_operation(), None);
                    assert_eq!(policy.commands(), Some(ReconnectPolicy::Carry));
                    assert_eq!(policy.encoded_settings_max(), None);
                }
                NativeKind::Walk => {
                    assert_eq!(policy.class(), RequestClass::Capability);
                    assert_eq!(policy.pause(), PausePolicy::Freeze);
                    assert_eq!(policy.takeover(), TakeoverPolicy::InterruptWalk);
                    assert_eq!(policy.reconnect(), ReconnectPolicy::Resume);
                    assert_eq!(policy.sub_operation(), None);
                    assert_eq!(
                        policy.commands(),
                        Some(ReconnectPolicy::Carry),
                        "R5-8.5: the host carries the undispatched walk-control request (≤ HELD_WALKS)"
                    );
                    assert_eq!(policy.encoded_settings_max(), None);
                }
                _ => {
                    assert_eq!(policy.class(), RequestClass::Capability);
                    assert_eq!(policy.pause(), PausePolicy::Freeze);
                    assert_eq!(policy.takeover(), TakeoverPolicy::None);
                    assert_eq!(policy.reconnect(), ReconnectPolicy::Resume);
                    assert_eq!(policy.sub_operation(), None);
                    assert_eq!(policy.commands(), None);
                    assert_eq!(policy.encoded_settings_max(), None);
                }
            }
        }
        assert_eq!(NativeKind::ALL.len(), 25);
    }
}
