use super::{
    BankWithdrawMode, ContractError, ContractKey, ServerOrderedCatalog, ServerOrderedInputError,
    ServerOrderedInputSnapshot,
};
use crate::{
    debug_commands::DebugCatalog,
    interact::Driver,
    prot::{PacketCategory, PacketError, PacketMetadata, Send},
    selected::SelectedPin,
    snapshot::{GameSnapshot, ItemContainer, ItemView, LocView, NpcView, WorldTile},
};
use client::io::{ClientProt, ClientRevision};

const MAX_ROWS: usize = 5;
const MAX_ADVANCED_BYTES: usize = 8 * 1024;
/// The fixed expectation phase used for bank count-dialog resumes.
pub const BANK_COUNT_PHASE: u8 = 1;

/// One exact inventory row selected by a native content request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InventoryItemReference {
    item_id: i32,
    slot: i32,
    component_id: i32,
    inventory_generation: u64,
}

impl InventoryItemReference {
    /// Bind one item identity to the received inventory packet generation.
    pub fn new(
        item_id: i32,
        slot: i32,
        component_id: i32,
        inventory_generation: u64,
    ) -> Result<Self, ServerOrderedError> {
        if item_id < 0 || slot < 0 || component_id < 0 {
            return Err(ServerOrderedError::InvalidShape);
        }
        Ok(Self {
            item_id,
            slot,
            component_id,
            inventory_generation,
        })
    }

    /// Build a reference from one received inventory row and its packet stamp.
    pub fn from_snapshot(snapshot: &GameSnapshot, item: &ItemView) -> Option<Self> {
        if item.container != ItemContainer::Inventory {
            return None;
        }
        Some(Self {
            item_id: item.def.id,
            slot: item.slot,
            component_id: item.component_id,
            inventory_generation: snapshot.inventory_packet_generation()?,
        })
    }

    /// Return the selected object identity.
    pub fn item_id(self) -> i32 {
        self.item_id
    }

    /// Return the exact inventory slot.
    pub fn slot(self) -> i32 {
        self.slot
    }

    /// Return the selected inventory component identity.
    pub fn component_id(self) -> i32 {
        self.component_id
    }

    /// Return the received inventory packet generation.
    pub fn inventory_generation(self) -> u64 {
        self.inventory_generation
    }
}

/// An exact location row, including its packed typecode and loc generations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExactLocReference {
    id: i32,
    typecode: i32,
    tile: WorldTile,
    scene_generation: u64,
    loc_static_generation: u64,
    operation: u8,
}

impl ExactLocReference {
    /// Bind one selected loc, scene position and one-based operation slot.
    pub fn new(
        id: i32,
        typecode: i32,
        tile: WorldTile,
        scene_generation: u64,
        loc_static_generation: u64,
        operation: u8,
    ) -> Result<Self, ServerOrderedError> {
        if id < 0 || typecode < 0 || !(1..=5).contains(&operation) {
            return Err(ServerOrderedError::InvalidShape);
        }
        Ok(Self {
            id,
            typecode,
            tile,
            scene_generation,
            loc_static_generation,
            operation,
        })
    }

    /// Build a reference from one observed loc and its current scene stamps.
    pub fn from_snapshot(
        snapshot: &GameSnapshot,
        loc: &LocView,
        operation: u8,
    ) -> Result<Self, ServerOrderedError> {
        Self::new(
            loc.id,
            loc.typecode,
            loc.tile,
            snapshot.scene_generation(),
            snapshot.loc_static_generation(),
            operation,
        )
    }
}

/// An exact NPC slot and definition, tied to the received NPC packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExactNpcReference {
    index: usize,
    type_id: i32,
    tile: WorldTile,
    npc_info_generation: u64,
    operation: u8,
}

impl ExactNpcReference {
    /// Bind one observed NPC slot and one-based operation slot.
    pub fn new(
        index: usize,
        type_id: i32,
        tile: WorldTile,
        npc_info_generation: u64,
        operation: u8,
    ) -> Result<Self, ServerOrderedError> {
        if type_id < 0 || !(1..=5).contains(&operation) {
            return Err(ServerOrderedError::InvalidShape);
        }
        Ok(Self {
            index,
            type_id,
            tile,
            npc_info_generation,
            operation,
        })
    }

    /// Build a reference from one observed NPC and its current packet stamp.
    pub fn from_snapshot(
        snapshot: &GameSnapshot,
        npc: &NpcView,
        operation: u8,
    ) -> Result<Self, ServerOrderedError> {
        let type_id = npc
            .r#type
            .and_then(|type_id| i32::try_from(type_id).ok())
            .ok_or(ServerOrderedError::StaleTarget)?;
        Self::new(
            npc.index,
            type_id,
            npc.network,
            snapshot.gens().npc_info,
            operation,
        )
    }
}

/// An exact resource target for the KnifeArm's terminal operation row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExactLocOrNpcRef {
    /// One exact location operation.
    Loc(ExactLocReference),
    /// One exact NPC operation. Current unavailable transform proofs refuse it.
    Npc(ExactNpcReference),
}

/// The closed shape for a KnifeArm request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnifeArmShape {
    /// Use the selected Make control, then click one exact resource target.
    Continuing {
        /// The resource target and its exact operation.
        click: ExactLocOrNpcRef,
    },
    /// Open the Make menu only; the native machine owns its later click.
    Ending,
}

/// A bank operation whose count phase can resume only from its owned expectation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountResumeContract {
    /// Withdraw X requires the exact posted item/note mode.
    BankWithdrawX {
        /// The mode observed by the bank phase that created the expectation.
        expected_mode: BankWithdrawMode,
    },
    /// Deposit X requires the exact bank listener generation.
    BankDepositX,
}

impl CountResumeContract {
    fn key(self) -> ContractKey {
        match self {
            Self::BankWithdrawX { .. } => ContractKey::BankWithdrawX,
            Self::BankDepositX => ContractKey::BankDepositX,
        }
    }
}

/// The selected modal that a terminal close is allowed to affect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedModalRoot {
    /// The bank root and its selected side interface.
    Bank,
}

/// The named cleanup effect authorizing a terminal close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseModalReason {
    /// Finish a bank-owned terminal exit, including weak-queue effects.
    TerminalBankExit,
}

/// Closed native-content requests. Advanced packet operations are surface-only.
#[derive(Debug)]
pub enum ServerOrderedRequest {
    /// Open the Make menu and optionally perform one exact resource click.
    KnifeArm {
        /// The knife row selected for use.
        knife: InventoryItemReference,
        /// The normal-log row used on the knife.
        logs: InventoryItemReference,
        /// Ending or continuing recipe shape.
        shape: KnifeArmShape,
    },
    /// Resume an exact, still-owned bank count phase.
    CountResume {
        /// The content contract that created the expectation.
        contract: CountResumeContract,
        /// The request whose observed phase owns this count.
        predecessor_request_id: u64,
        /// The run that owns the bank phase.
        run_generation: u64,
        /// The bank open-session generation captured by the owner.
        bank_session_generation: u64,
        /// The positive requested bank delta.
        value: i32,
    },
    /// Close only for the selected bank's terminal cleanup reason.
    CloseModal {
        /// The selected modal root expected by the cleanup contract.
        expected_root: ExpectedModalRoot,
        /// The named owner cleanup reason.
        reason: CloseModalReason,
    },
}

impl ServerOrderedRequest {
    /// Return the number of logical rows in this closed request shape.
    pub fn row_count(&self) -> u8 {
        match self {
            Self::KnifeArm {
                shape: KnifeArmShape::Continuing { .. },
                ..
            } => 3,
            Self::KnifeArm {
                shape: KnifeArmShape::Ending,
                ..
            } => 2,
            Self::CountResume { .. } | Self::CloseModal { .. } => 1,
        }
    }
    /// Validate the static shape and its typed packets for one protocol revision.
    ///
    /// This check does not inspect a snapshot, selected catalog, or live authority.
    pub fn validate_shape(&self, revision: ClientRevision) -> Result<(), ServerOrderedError> {
        if self.row_count() == 0 || usize::from(self.row_count()) > MAX_ROWS {
            return Err(ServerOrderedError::InvalidShape);
        }
        match self {
            Self::KnifeArm { knife, logs, shape } => {
                if [knife, logs]
                    .into_iter()
                    .any(|item| item.item_id < 0 || item.slot < 0 || item.component_id < 0)
                {
                    return Err(ServerOrderedError::InvalidShape);
                }
                Send::opheldu(
                    logs.item_id,
                    logs.slot,
                    logs.component_id,
                    knife.item_id,
                    knife.slot,
                    knife.component_id,
                )?
                .validate_for_revision(revision)?;
                Send::if_button(0)?.validate_for_revision(revision)?;
                if let KnifeArmShape::Continuing { click } = shape {
                    match click {
                        ExactLocOrNpcRef::Loc(reference) => {
                            loc_packet(
                                reference.operation,
                                reference.tile.x,
                                reference.tile.z,
                                reference.id,
                            )?
                            .validate_for_revision(revision)?;
                        }
                        ExactLocOrNpcRef::Npc(reference) => {
                            let index = i32::try_from(reference.index)
                                .map_err(|_| ServerOrderedError::InvalidShape)?;
                            npc_packet(reference.operation, index)?
                                .validate_for_revision(revision)?;
                        }
                    }
                }
            }
            Self::CountResume { value, .. } => {
                if *value <= 0 {
                    return Err(ServerOrderedError::InvalidShape);
                }
                Send::count_dialog(*value).validate_for_revision(revision)?;
            }
            Self::CloseModal {
                expected_root,
                reason,
            } => {
                if *expected_root != ExpectedModalRoot::Bank
                    || *reason != CloseModalReason::TerminalBankExit
                {
                    return Err(ServerOrderedError::InvalidShape);
                }
                Send::close_modal().validate_for_revision(revision)?;
            }
        }
        Ok(())
    }

    /// Validate a bounded advanced batch without checking its surface authority.
    pub fn validate_advanced_shape(
        revision: ClientRevision,
        operations: &[Send],
    ) -> Result<(), ServerOrderedError> {
        validate_advanced_rows(revision, operations).map_err(|(_, error)| error)
    }
}

/// A row's dispatch result, in original request order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowDispatch {
    /// The row was not reached.
    NotAttempted,
    /// Every packet in the row was committed to the encrypted sink.
    HostQueued {
        /// Actual encrypted packets committed for this row.
        wire_events: u8,
    },
    /// The row was refused, with any committed prefix retained.
    Refused {
        /// The typed refusal cause.
        reason: ServerOrderedError,
    },
    /// A preceding row failed, so this dependent row was not attempted.
    SkippedDependency,
}

/// A typed dispatch failure. It contains no host rate-limit outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ServerOrderedError {
    /// The selected catalog or recipe is unavailable.
    #[error("server-ordered contract is unavailable")]
    ContractUnavailable,
    /// Selected cache/content identity does not match the live pin.
    #[error("server-ordered selected pin does not match")]
    PinMismatch,
    /// Packet, catalog, and live session revisions differ.
    #[error("server-ordered revision does not match")]
    RevisionMismatch,
    /// The request shape or one of its typed fields is invalid.
    #[error("server-ordered request shape is invalid")]
    InvalidShape,
    /// A packet cannot be represented by the selected named protocol table.
    #[error("server-ordered packet is unsupported")]
    UnsupportedPacket,
    /// An exact target or selected inventory row is stale.
    #[error("server-ordered target is stale")]
    StaleTarget,
    /// An observed modal conflicts with the requested action.
    #[error("server-ordered modal is stale")]
    StaleModal,
    /// A competing local prompt makes the request ambiguous.
    #[error("server-ordered request conflicts with a local prompt")]
    ConflictingPrompt,
    /// No owned phase exists for a dependent request.
    #[error("server-ordered expectation is missing")]
    MissingExpectation,
    /// The observed-tick deadline has passed.
    #[error("server-ordered expectation has expired")]
    ExpiredExpectation,
    /// The predecessor, epoch, session, or request lineage no longer matches.
    #[error("server-ordered expectation was invalidated")]
    InvalidatedExpectation,
    /// The live authority predicate denied a packet.
    #[error("server-ordered authority was revoked")]
    AuthorityDenied,
    /// The external script lacks the advanced packet declaration.
    #[error("advanced packet operations are not declared")]
    PermissionDenied,
    /// The existing emission-frame scheduler did not admit this request.
    #[error("server-ordered emission budget is exhausted")]
    BudgetExhausted,
    /// The normal encrypted sink cannot expose exact packet commits.
    #[error("server-ordered packet observation is unavailable")]
    ObservationUnavailable,
    /// The driver cannot commit through its normal encrypted sink.
    #[error("server-ordered transport is unavailable")]
    TransportUnavailable,
    /// A typed packet failed validation or encoding.
    #[error("server-ordered packet encoding failed")]
    EncodingFailed,
    /// Dispatch was cancelled before the next packet.
    #[error("server-ordered dispatch was cancelled")]
    Cancelled,
    /// The owner or run no longer matches the pending request.
    #[error("server-ordered owner is stale")]
    StaleOwner,
    /// Advanced operations exceed the bounded request storage limit.
    #[error("advanced packet batch exceeds its bounded shape")]
    RequestTooLarge,
    /// Destructive debug commands have no confirmation channel on this surface.
    #[error("destructive cheat confirmation is unavailable")]
    DestructiveCheatConfirmationUnavailable,
}

impl From<ContractError> for ServerOrderedError {
    fn from(error: ContractError) -> Self {
        match error {
            ContractError::PinMismatch => Self::PinMismatch,
            _ => Self::ContractUnavailable,
        }
    }
}

impl From<ServerOrderedInputError> for ServerOrderedError {
    fn from(error: ServerOrderedInputError) -> Self {
        match error {
            ServerOrderedInputError::MissingObservation => Self::StaleOwner,
            ServerOrderedInputError::MissingExpectation => Self::MissingExpectation,
            ServerOrderedInputError::ExpiredExpectation => Self::ExpiredExpectation,
            ServerOrderedInputError::InvalidatedExpectation
            | ServerOrderedInputError::ContinueFenceActive
            | ServerOrderedInputError::ContinueFenceMissing => Self::InvalidatedExpectation,
        }
    }
}

impl From<PacketError> for ServerOrderedError {
    fn from(error: PacketError) -> Self {
        match error {
            PacketError::RevisionMismatch => Self::RevisionMismatch,
            PacketError::InvalidShape(_) | PacketError::BodyTooLong { .. } => {
                Self::UnsupportedPacket
            }
            PacketError::IntegerOutOfRange { .. }
            | PacketError::InvalidName(_)
            | PacketError::InvalidAbuseReason(_)
            | PacketError::InvalidText
            | PacketError::InvalidCheat(_) => Self::EncodingFailed,
        }
    }
}

/// One exact receipt for a server-ordered content or advanced request.
///
/// Receipt contract: `tick-manipulation-spec-r4.md`, section 3.1.
/// Run and session identity stay in the completion envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerOrderedDispatch {
    /// The one request identity used by the owner and completion envelope.
    pub request_id: u64,
    /// The observed tick of this host drain frame.
    pub dispatch_tick: u64,
    /// Logical row results in request order; unused entries remain `NotAttempted`.
    pub rows: [RowDispatch; MAX_ROWS],
    /// Number of logical rows in this request.
    pub row_count: u8,
    /// Fully queued logical-row prefix.
    pub dispatched_prefix: u8,
    /// Every encrypted packet committed, including a partly queued row.
    pub wire_events: u8,
    /// Terminal refusal, if one stopped dispatch.
    pub terminal: Option<ServerOrderedError>,
}

impl ServerOrderedDispatch {
    /// Build a zero-wire preflight refusal for a request not yet dispatched.
    pub fn refused(
        request_id: u64,
        dispatch_tick: u64,
        row_count: u8,
        dispatched_prefix: u8,
        reason: ServerOrderedError,
    ) -> Self {
        let row_count = row_count.min(MAX_ROWS as u8);
        let mut rows = [RowDispatch::NotAttempted; MAX_ROWS];
        if row_count > 0 {
            rows[0] = RowDispatch::Refused { reason };
            for row in rows.iter_mut().take(usize::from(row_count)).skip(1) {
                *row = RowDispatch::SkippedDependency;
            }
        }
        Self {
            request_id,
            dispatch_tick,
            rows,
            row_count,
            dispatched_prefix,
            wire_events: 0,
            terminal: Some(reason),
        }
    }
}

/// Host-owned dispatch inputs. The authority predicate is checked before each packet.
pub struct ServerOrderedDispatchContext<'a> {
    /// The sole request identity for this dispatch.
    pub request_id: u64,
    /// The observed tick of the drain frame.
    pub dispatch_tick: u64,
    /// Detached slot input accounting and expectation state.
    pub input: ServerOrderedInputSnapshot,
    /// Selected debug-command facts for advanced cheat validation, if admitted.
    pub debug: Option<&'a DebugCatalog>,
    /// Live authority callback, checked immediately before each actual packet.
    pub authority: &'a mut dyn FnMut() -> Result<(), ServerOrderedError>,
}

/// Shared stateless planner and dispatcher for both request modes.
pub struct ServerOrderedInputs;

impl ServerOrderedInputs {
    /// Preflight and dispatch one named native-content request.
    pub fn dispatch(
        catalog: &ServerOrderedCatalog,
        pin: &SelectedPin,
        snapshot: &GameSnapshot,
        driver: &mut dyn Driver,
        request: &ServerOrderedRequest,
        context: ServerOrderedDispatchContext<'_>,
    ) -> ServerOrderedDispatch {
        let row_count = request.row_count();
        if let Err(error) =
            validate_dispatch_context(snapshot, driver, context.input, context.dispatch_tick)
        {
            return ServerOrderedDispatch::refused(
                context.request_id,
                context.dispatch_tick,
                row_count,
                0,
                error,
            );
        }
        if let Err(error) = (context.authority)() {
            return ServerOrderedDispatch::refused(
                context.request_id,
                context.dispatch_tick,
                row_count,
                0,
                error,
            );
        }
        if let Err(error) = catalog.facts.validate(pin) {
            return ServerOrderedDispatch::refused(
                context.request_id,
                context.dispatch_tick,
                row_count,
                0,
                error.into(),
            );
        }
        if catalog.revision() != pin.revision.as_i32() || driver.revision() != pin.revision {
            return ServerOrderedDispatch::refused(
                context.request_id,
                context.dispatch_tick,
                row_count,
                0,
                ServerOrderedError::RevisionMismatch,
            );
        }
        if let Err(error) = request.validate_shape(pin.revision) {
            return ServerOrderedDispatch::refused_at(
                context.request_id,
                context.dispatch_tick,
                row_count,
                0,
                error,
            );
        }
        let plan =
            match build_content_plan(catalog, pin.revision, snapshot, driver, request, &context) {
                Ok(plan) => plan,
                Err((row, error)) => {
                    return ServerOrderedDispatch::refused_at(
                        context.request_id,
                        context.dispatch_tick,
                        row_count,
                        row,
                        error,
                    )
                }
            };
        dispatch_plan(plan, pin.revision, driver, context)
    }

    /// Preflight and dispatch a sealed surface's typed packet operations.
    pub fn dispatch_advanced(
        snapshot: &GameSnapshot,
        driver: &mut dyn Driver,
        operations: &[Send],
        context: ServerOrderedDispatchContext<'_>,
    ) -> ServerOrderedDispatch {
        let row_count = operations.len().min(MAX_ROWS) as u8;
        if let Err(error) =
            validate_dispatch_context(snapshot, driver, context.input, context.dispatch_tick)
        {
            return ServerOrderedDispatch::refused(
                context.request_id,
                context.dispatch_tick,
                row_count,
                0,
                error,
            );
        }
        if let Err(error) = (context.authority)() {
            return ServerOrderedDispatch::refused(
                context.request_id,
                context.dispatch_tick,
                row_count,
                0,
                error,
            );
        }
        let revision = driver.revision();
        if let Err((row, error)) = validate_advanced_rows(revision, operations) {
            return ServerOrderedDispatch::refused_at(
                context.request_id,
                context.dispatch_tick,
                row_count,
                row,
                error,
            );
        }
        for (row, packet) in operations.iter().enumerate() {
            if packet.named_packet() == ClientProt::CLIENT_CHEAT {
                let Some(debug) = context.debug else {
                    return ServerOrderedDispatch::refused_at(
                        context.request_id,
                        context.dispatch_tick,
                        row_count,
                        row as u8,
                        ServerOrderedError::PermissionDenied,
                    );
                };
                if debug.revision() != revision {
                    return ServerOrderedDispatch::refused_at(
                        context.request_id,
                        context.dispatch_tick,
                        row_count,
                        row as u8,
                        ServerOrderedError::RevisionMismatch,
                    );
                }
                if driver.cheat_admission() != client::CheatAdmission::Granted {
                    return ServerOrderedDispatch::refused_at(
                        context.request_id,
                        context.dispatch_tick,
                        row_count,
                        row as u8,
                        ServerOrderedError::PermissionDenied,
                    );
                }

                let Some(command) = packet.body().strip_suffix(&[10]) else {
                    return ServerOrderedDispatch::refused_at(
                        context.request_id,
                        context.dispatch_tick,
                        row_count,
                        row as u8,
                        ServerOrderedError::EncodingFailed,
                    );
                };
                let Ok(command) = std::str::from_utf8(command) else {
                    return ServerOrderedDispatch::refused_at(
                        context.request_id,
                        context.dispatch_tick,
                        row_count,
                        row as u8,
                        ServerOrderedError::EncodingFailed,
                    );
                };
                let command_name = command.split(' ').next().unwrap_or_default();
                let Some(command) = debug
                    .commands()
                    .iter()
                    .find(|entry| entry.name == command_name)
                else {
                    return ServerOrderedDispatch::refused_at(
                        context.request_id,
                        context.dispatch_tick,
                        row_count,
                        row as u8,
                        ServerOrderedError::PermissionDenied,
                    );
                };
                if command.destructive {
                    return ServerOrderedDispatch::refused_at(
                        context.request_id,
                        context.dispatch_tick,
                        row_count,
                        row as u8,
                        ServerOrderedError::DestructiveCheatConfirmationUnavailable,
                    );
                }
            }
        }

        dispatch_advanced_rows(operations, revision, driver, context)
    }
}

impl ServerOrderedDispatch {
    fn refused_at(
        request_id: u64,
        dispatch_tick: u64,
        row_count: u8,
        row: u8,
        reason: ServerOrderedError,
    ) -> Self {
        let row_count = row_count.min(MAX_ROWS as u8);
        let row = row.min(row_count.saturating_sub(1));
        let mut rows = [RowDispatch::NotAttempted; MAX_ROWS];
        if row_count > 0 {
            rows[usize::from(row)] = RowDispatch::Refused { reason };
            for dependent in rows
                .iter_mut()
                .take(usize::from(row_count))
                .skip(usize::from(row.saturating_add(1)))
            {
                *dependent = RowDispatch::SkippedDependency;
            }
        }
        Self {
            request_id,
            dispatch_tick,
            rows,
            row_count,
            dispatched_prefix: 0,
            wire_events: 0,
            terminal: Some(reason),
        }
    }
}

#[derive(Debug)]
struct DispatchPlan {
    rows: [Option<PlannedRow>; MAX_ROWS],
    row_count: u8,
}

impl DispatchPlan {
    fn new(row_count: u8) -> Self {
        Self {
            rows: std::array::from_fn(|_| None),
            row_count,
        }
    }

    fn set(&mut self, row: usize, plan: PlannedRow) {
        self.rows[row] = Some(plan);
    }
}

#[derive(Debug)]
enum PlannedRow {
    Packet(Send),
    KnifeClick { movement: MoveArgs, packet: Send },
}

#[derive(Debug, Clone, Copy)]
struct MoveArgs {
    src_x: i32,
    src_z: i32,
    target_x: i32,
    target_z: i32,
    width: i32,
    length: i32,
    angle: i32,
    shape: i32,
    force_approach: i32,
}

fn build_content_plan(
    catalog: &ServerOrderedCatalog,
    revision: ClientRevision,
    snapshot: &GameSnapshot,
    driver: &dyn Driver,
    request: &ServerOrderedRequest,
    context: &ServerOrderedDispatchContext<'_>,
) -> Result<DispatchPlan, (u8, ServerOrderedError)> {
    let row_count = request.row_count();
    let mut plan = DispatchPlan::new(row_count);
    match request {
        ServerOrderedRequest::KnifeArm { knife, logs, shape } => {
            let entry = catalog
                .entry(ContractKey::KnifeArm)
                .map_err(|error| (0, error.into()))?;
            if !snapshot.attached() || !snapshot.ingame() {
                return Err((0, ServerOrderedError::StaleOwner));
            }
            let modals = snapshot.modals();
            if [modals.main, modals.side, modals.chat, modals.tutorial]
                .into_iter()
                .any(|root| root == 8880)
            {
                return Err((0, ServerOrderedError::StaleModal));
            }
            let knife_view =
                resolve_inventory_item(snapshot, *knife).map_err(|error| (0, error))?;
            let logs_view = resolve_inventory_item(snapshot, *logs).map_err(|error| (0, error))?;
            let item_roles = catalog.knife();
            if knife_view.def.id != item_roles.knife().id()
                || !item_roles
                    .logs()
                    .iter()
                    .any(|identity| identity.id() == logs_view.def.id)
                || knife_view.count <= 0
                || logs_view.count <= 0
                || knife_view.slot == logs_view.slot
            {
                return Err((0, ServerOrderedError::StaleTarget));
            }
            let make = catalog
                .component("skill_multi3:make1_a")
                .map_err(|error| (1, error.into()))?;
            let use_packet = Send::opheldu(
                logs_view.def.id,
                logs_view.slot,
                logs_view.component_id,
                knife_view.def.id,
                knife_view.slot,
                knife_view.component_id,
            )
            .map_err(|error| (0, error.into()))?;
            let make_packet = Send::if_button(make.id()).map_err(|error| (1, error.into()))?;
            use_packet
                .validate_for_revision(revision)
                .map_err(|error| (0, error.into()))?;
            make_packet
                .validate_for_revision(revision)
                .map_err(|error| (1, error.into()))?;
            plan.set(0, PlannedRow::Packet(use_packet));
            plan.set(1, PlannedRow::Packet(make_packet));
            if let KnifeArmShape::Continuing { click } = shape {
                if entry.max_packet_cost() < 4 {
                    return Err((2, ServerOrderedError::ContractUnavailable));
                }
                let (movement, packet) =
                    build_click(catalog, snapshot, driver, *click).map_err(|error| (2, error))?;
                packet
                    .validate_for_revision(revision)
                    .map_err(|error| (2, error.into()))?;
                plan.set(2, PlannedRow::KnifeClick { movement, packet });
            }
        }
        ServerOrderedRequest::CountResume {
            contract,
            predecessor_request_id,
            run_generation,
            bank_session_generation,
            value,
        } => {
            let key = contract.key();
            catalog.entry(key).map_err(|error| (0, error.into()))?;
            if *value <= 0 || snapshot.bank_session_generation() != *bank_session_generation {
                return Err((0, ServerOrderedError::StaleTarget));
            }
            if !snapshot.bank_loaded() {
                return Err((0, ServerOrderedError::StaleTarget));
            }
            match contract {
                CountResumeContract::BankWithdrawX { expected_mode } => {
                    let mode_varp = catalog.bank().mode_varp().id();
                    let observed_mode = snapshot
                        .varps()
                        .iter()
                        .find(|varp| varp.index == mode_varp)
                        .map(|varp| varp.value);
                    if catalog.bank().observed_mode(observed_mode) != Some(*expected_mode) {
                        return Err((0, ServerOrderedError::InvalidatedExpectation));
                    }
                }
                CountResumeContract::BankDepositX => {}
            }
            let tick = u32::try_from(context.dispatch_tick)
                .map_err(|_| (0, ServerOrderedError::ExpiredExpectation))?;
            context
                .input
                .require_expectation(
                    *run_generation,
                    key,
                    BANK_COUNT_PHASE,
                    *predecessor_request_id,
                    tick,
                )
                .map_err(|error| (0, error.into()))?;
            let packet = Send::count_dialog(*value);
            packet
                .validate_for_revision(revision)
                .map_err(|error| (0, error.into()))?;
            plan.set(0, PlannedRow::Packet(packet));
        }
        ServerOrderedRequest::CloseModal {
            expected_root,
            reason,
        } => {
            if *expected_root != ExpectedModalRoot::Bank
                || *reason != CloseModalReason::TerminalBankExit
            {
                return Err((0, ServerOrderedError::InvalidShape));
            }
            catalog
                .entry(ContractKey::BankCloseTerminal)
                .map_err(|error| (0, error.into()))?;
            let bank_root = catalog
                .component("bank_main:inv")
                .map_err(|error| (0, error.into()))?
                .root_id();
            let side_root = catalog
                .component("bank_side:inv")
                .map_err(|error| (0, error.into()))?
                .root_id();
            let modals = snapshot.modals();
            if [modals.main, modals.side, modals.chat, modals.tutorial]
                .into_iter()
                .any(|root| root != -1 && root != bank_root && root != side_root)
            {
                return Err((0, ServerOrderedError::ConflictingPrompt));
            }
            let packet = Send::close_modal();
            packet
                .validate_for_revision(revision)
                .map_err(|error| (0, error.into()))?;
            plan.set(0, PlannedRow::Packet(packet));
        }
    }
    Ok(plan)
}

fn resolve_inventory_item(
    snapshot: &GameSnapshot,
    reference: InventoryItemReference,
) -> Result<&ItemView, ServerOrderedError> {
    if snapshot.inventory_packet_generation() != Some(reference.inventory_generation) {
        return Err(ServerOrderedError::StaleTarget);
    }
    snapshot
        .inventory()
        .iter()
        .find(|item| {
            item.container == ItemContainer::Inventory
                && item.def.id == reference.item_id
                && item.slot == reference.slot
                && item.component_id == reference.component_id
        })
        .ok_or(ServerOrderedError::StaleTarget)
}

fn build_click(
    catalog: &ServerOrderedCatalog,
    snapshot: &GameSnapshot,
    driver: &dyn Driver,
    click: ExactLocOrNpcRef,
) -> Result<(MoveArgs, Send), ServerOrderedError> {
    let (tile, width, length, angle, shape, force_approach, packet) = match click {
        ExactLocOrNpcRef::Loc(reference) => {
            if reference.scene_generation != snapshot.scene_generation()
                || reference.loc_static_generation != snapshot.loc_static_generation()
            {
                return Err(ServerOrderedError::StaleTarget);
            }
            let loc = snapshot
                .locs()
                .iter()
                .find(|loc| {
                    loc.id == reference.id
                        && loc.typecode == reference.typecode
                        && loc.tile == reference.tile
                        && loc.active
                        && loc
                            .actions
                            .get(usize::from(reference.operation - 1))
                            .is_some_and(Option::is_some)
                })
                .ok_or(ServerOrderedError::StaleTarget)?;
            let packet = loc_packet(reference.operation, loc.tile.x, loc.tile.z, loc.id)?;
            (
                loc.tile,
                loc.width,
                loc.length,
                loc.angle,
                loc.shape,
                loc.force_approach,
                packet,
            )
        }
        ExactLocOrNpcRef::Npc(reference) => {
            if snapshot.gens().npc_info != reference.npc_info_generation {
                return Err(ServerOrderedError::StaleTarget);
            }
            let npc = snapshot
                .npcs()
                .iter()
                .find(|npc| {
                    npc.index == reference.index
                        && npc.r#type.and_then(|id| i32::try_from(id).ok())
                            == Some(reference.type_id)
                        && npc.network == reference.tile
                        && npc
                            .actions
                            .get(usize::from(reference.operation - 1))
                            .is_some_and(Option::is_some)
                })
                .ok_or(ServerOrderedError::StaleTarget)?;
            if catalog
                .npc_operation(reference.type_id, reference.operation)
                .is_none()
            {
                return Err(ServerOrderedError::ContractUnavailable);
            }
            let index = i32::try_from(npc.index).map_err(|_| ServerOrderedError::StaleTarget)?;
            (
                npc.network,
                npc.size,
                npc.size,
                0,
                0,
                0,
                npc_packet(reference.operation, index)?,
            )
        }
    };
    let scene = snapshot.scene();
    if !scene.available || tile.level != scene.level {
        return Err(ServerOrderedError::StaleTarget);
    }
    let (target_x, target_z) = scene_coords(driver, tile);
    let (src_x, src_z) = driver
        .local_route()
        .ok_or(ServerOrderedError::StaleTarget)?;
    if !(0..scene.width).contains(&target_x)
        || !(0..scene.height).contains(&target_z)
        || !(0..scene.width).contains(&src_x)
        || !(0..scene.height).contains(&src_z)
    {
        return Err(ServerOrderedError::StaleTarget);
    }
    Ok((
        MoveArgs {
            src_x,
            src_z,
            target_x,
            target_z,
            width,
            length,
            angle,
            shape,
            force_approach,
        },
        packet,
    ))
}

fn scene_coords(driver: &dyn Driver, tile: WorldTile) -> (i32, i32) {
    let (base_x, base_z) = driver.build_base();
    (tile.x - base_x, tile.z - base_z)
}

fn loc_packet(operation: u8, x: i32, z: i32, loc_id: i32) -> Result<Send, ServerOrderedError> {
    let packet = match operation {
        1 => Send::oploc1(x, z, loc_id),
        2 => Send::oploc2(x, z, loc_id),
        3 => Send::oploc3(x, z, loc_id),
        4 => Send::oploc4(x, z, loc_id),
        5 => Send::oploc5(x, z, loc_id),
        _ => return Err(ServerOrderedError::InvalidShape),
    };
    packet.map_err(Into::into)
}

fn npc_packet(operation: u8, index: i32) -> Result<Send, ServerOrderedError> {
    let packet = match operation {
        1 => Send::opnpc1(index),
        2 => Send::opnpc2(index),
        3 => Send::opnpc3(index),
        4 => Send::opnpc4(index),
        5 => Send::opnpc5(index),
        _ => return Err(ServerOrderedError::InvalidShape),
    };
    packet.map_err(Into::into)
}

struct EmitFailure {
    reason: ServerOrderedError,
    wire_events: u8,
}

fn dispatch_plan(
    plan: DispatchPlan,
    revision: ClientRevision,
    driver: &mut dyn Driver,
    mut context: ServerOrderedDispatchContext<'_>,
) -> ServerOrderedDispatch {
    let mut receipt = ServerOrderedDispatch {
        request_id: context.request_id,
        dispatch_tick: context.dispatch_tick,
        rows: [RowDispatch::NotAttempted; MAX_ROWS],
        row_count: plan.row_count,
        dispatched_prefix: 0,
        wire_events: 0,
        terminal: None,
    };
    for index in 0..usize::from(plan.row_count) {
        let Some(row) = plan.rows[index].as_ref() else {
            receipt.rows[index] = RowDispatch::Refused {
                reason: ServerOrderedError::InvalidShape,
            };
            receipt.terminal = Some(ServerOrderedError::InvalidShape);
            mark_dependencies_skipped(&mut receipt, index + 1);
            break;
        };
        let result = match row {
            PlannedRow::Packet(packet) => emit_packet(driver, packet, revision, &mut context),
            PlannedRow::KnifeClick { movement, packet } => {
                emit_knife_click(driver, *movement, packet, revision, &mut context)
            }
        };
        let (wire_events, result) = match result {
            Ok(wire_events) => (wire_events, Ok(())),
            Err(failure) => (failure.wire_events, Err(failure.reason)),
        };
        receipt.wire_events = receipt.wire_events.saturating_add(wire_events);
        match result {
            Ok(()) => {
                receipt.rows[index] = RowDispatch::HostQueued { wire_events };
                receipt.dispatched_prefix = receipt.dispatched_prefix.saturating_add(1);
            }
            Err(reason) => {
                receipt.rows[index] = RowDispatch::Refused { reason };
                receipt.terminal = Some(reason);
                mark_dependencies_skipped(&mut receipt, index + 1);
                break;
            }
        }
    }
    receipt
}

fn dispatch_advanced_rows(
    operations: &[Send],
    revision: ClientRevision,
    driver: &mut dyn Driver,
    mut context: ServerOrderedDispatchContext<'_>,
) -> ServerOrderedDispatch {
    let mut receipt = ServerOrderedDispatch {
        request_id: context.request_id,
        dispatch_tick: context.dispatch_tick,
        rows: [RowDispatch::NotAttempted; MAX_ROWS],
        row_count: operations.len() as u8,
        dispatched_prefix: 0,
        wire_events: 0,
        terminal: None,
    };
    for (index, packet) in operations.iter().enumerate() {
        match emit_packet(driver, packet, revision, &mut context) {
            Ok(wire_events) => {
                receipt.rows[index] = RowDispatch::HostQueued { wire_events };
                receipt.wire_events = receipt.wire_events.saturating_add(wire_events);
                receipt.dispatched_prefix = receipt.dispatched_prefix.saturating_add(1);
            }
            Err(failure) => {
                receipt.rows[index] = RowDispatch::Refused {
                    reason: failure.reason,
                };
                receipt.wire_events = receipt.wire_events.saturating_add(failure.wire_events);
                receipt.terminal = Some(failure.reason);
                mark_dependencies_skipped(&mut receipt, index + 1);
                break;
            }
        }
    }
    receipt
}

fn emit_packet(
    driver: &mut dyn Driver,
    packet: &Send,
    revision: ClientRevision,
    context: &mut ServerOrderedDispatchContext<'_>,
) -> Result<u8, EmitFailure> {
    let metadata = packet
        .validate_for_revision(revision)
        .map_err(|error| EmitFailure {
            reason: error.into(),
            wire_events: 0,
        })?;
    driver.finish_pending_packet();
    (context.authority)().map_err(|reason| EmitFailure {
        reason,
        wire_events: 0,
    })?;
    let before = driver.packet_commit_count().ok_or(EmitFailure {
        reason: ServerOrderedError::TransportUnavailable,
        wire_events: 0,
    })?;
    let result = driver.send_packet(packet);
    let after = driver.packet_commit_count().ok_or(EmitFailure {
        reason: ServerOrderedError::TransportUnavailable,
        wire_events: 0,
    })?;
    let committed = after.checked_sub(before).ok_or(EmitFailure {
        reason: ServerOrderedError::TransportUnavailable,
        wire_events: 0,
    })?;
    if committed > 1 {
        return Err(EmitFailure {
            reason: ServerOrderedError::TransportUnavailable,
            wire_events: committed.min(u64::from(u8::MAX)) as u8,
        });
    }
    if committed == 1 && metadata.category == PacketCategory::UserEvent {
        driver.note_input_activity();
    }
    match result {
        Ok(()) if committed == 1 => Ok(1),
        Ok(()) => Err(EmitFailure {
            reason: ServerOrderedError::TransportUnavailable,
            wire_events: 0,
        }),
        Err(error) => Err(EmitFailure {
            reason: error.into(),
            wire_events: committed as u8,
        }),
    }
}

fn emit_knife_click(
    driver: &mut dyn Driver,
    movement: MoveArgs,
    packet: &Send,
    revision: ClientRevision,
    context: &mut ServerOrderedDispatchContext<'_>,
) -> Result<u8, EmitFailure> {
    driver.finish_pending_packet();
    (context.authority)().map_err(|reason| EmitFailure {
        reason,
        wire_events: 0,
    })?;
    let before = driver.packet_commit_count().ok_or(EmitFailure {
        reason: ServerOrderedError::TransportUnavailable,
        wire_events: 0,
    })?;
    let routed = driver.try_move(
        movement.src_x,
        movement.src_z,
        movement.target_x,
        movement.target_z,
        false,
        movement.width,
        movement.length,
        movement.angle,
        movement.shape,
        movement.force_approach,
        0,
    );
    let after = driver.packet_commit_count().ok_or(EmitFailure {
        reason: ServerOrderedError::TransportUnavailable,
        wire_events: 0,
    })?;
    let committed = after.checked_sub(before).ok_or(EmitFailure {
        reason: ServerOrderedError::TransportUnavailable,
        wire_events: 0,
    })?;
    if committed > 1 {
        return Err(EmitFailure {
            reason: ServerOrderedError::TransportUnavailable,
            wire_events: committed.min(u64::from(u8::MAX)) as u8,
        });
    }
    if committed == 1 {
        driver.note_input_activity();
    }
    if !routed {
        return Err(EmitFailure {
            reason: ServerOrderedError::StaleTarget,
            wire_events: committed as u8,
        });
    }
    match emit_packet(driver, packet, revision, context) {
        Ok(operation_events) => Ok((committed as u8).saturating_add(operation_events)),
        Err(mut failure) => {
            failure.wire_events = (committed as u8).saturating_add(failure.wire_events);
            Err(failure)
        }
    }
}

fn mark_dependencies_skipped(receipt: &mut ServerOrderedDispatch, first: usize) {
    for row in receipt
        .rows
        .iter_mut()
        .take(usize::from(receipt.row_count))
        .skip(first)
    {
        *row = RowDispatch::SkippedDependency;
    }
}

fn validate_dispatch_context(
    snapshot: &GameSnapshot,
    driver: &dyn Driver,
    input: ServerOrderedInputSnapshot,
    dispatch_tick: u64,
) -> Result<(), ServerOrderedError> {
    if u64::from(snapshot.tick()) != dispatch_tick {
        return Err(ServerOrderedError::StaleOwner);
    }
    if input.observed_tick() != u32::try_from(dispatch_tick).ok() {
        return Err(ServerOrderedError::StaleOwner);
    }
    if driver.packet_commit_count().is_none() {
        return Err(ServerOrderedError::ObservationUnavailable);
    }
    Ok(())
}

fn encoded_packet_len(packet: &Send, metadata: PacketMetadata) -> usize {
    let prefix = match metadata.length {
        -1 => 1,
        -2 => 2,
        _ => 0,
    };
    1usize
        .saturating_add(prefix)
        .saturating_add(packet.body().len())
}
fn validate_advanced_rows(
    revision: ClientRevision,
    operations: &[Send],
) -> Result<(), (u8, ServerOrderedError)> {
    if operations.is_empty() || operations.len() > MAX_ROWS {
        return Err((0, ServerOrderedError::InvalidShape));
    }
    let mut encoded_bytes = 0usize;
    for (row, packet) in operations.iter().enumerate() {
        let metadata = packet
            .validate_for_revision(revision)
            .map_err(|error| (row as u8, error.into()))?;
        encoded_bytes = encoded_bytes.saturating_add(encoded_packet_len(packet, metadata));
        if encoded_bytes > MAX_ADVANCED_BYTES {
            return Err((row as u8, ServerOrderedError::RequestTooLarge));
        }
    }
    Ok(())
}
