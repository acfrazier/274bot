use super::ContractKey;
use crate::prot::{packet_metadata, PacketCategory};
use client::io::ClientRevision;

/// Slot-scoped lineage for an encrypted packet or a local-only input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputLineage {
    /// Input whose owning request is not the active content request.
    Foreign,
    /// A named native-content request and its request identity.
    ContentRequest(u64),
    /// A declared advanced operation; it invalidates expectations for every packet.
    AdvancedRequest(u64),
}

/// The session and input epochs that bind an expectation to current slot state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerOrderedInputStamp {
    session_generation: u64,
    input_epoch: u64,
    invalidation_epoch: u64,
}

impl ServerOrderedInputStamp {
    /// Return the session generation observed by the slot.
    pub fn session_generation(self) -> u64 {
        self.session_generation
    }

    /// Return the USER_EVENT input epoch.
    pub fn input_epoch(self) -> u64 {
        self.input_epoch
    }

    /// Return the foreign/advanced invalidation generation.
    pub fn invalidation_epoch(self) -> u64 {
        self.invalidation_epoch
    }
}

/// One bounded, one-use content expectation owned by a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerOrderedExpectation {
    run_generation: u64,
    predecessor_request_id: u64,
    input_epoch: u64,
    invalidation_epoch: u64,
    expires_tick: u32,
    contract: ContractKey,
    phase: u8,
}

impl ServerOrderedExpectation {
    /// Return the request whose observed phase created this expectation.
    pub fn predecessor_request_id(self) -> u64 {
        self.predecessor_request_id
    }

    /// Return the contract that owns the phase.
    pub fn contract(self) -> ContractKey {
        self.contract
    }

    /// Return the contract-local phase number.
    pub fn phase(self) -> u8 {
        self.phase
    }

    /// Return the observed-tick deadline.
    pub fn expires_tick(self) -> u32 {
        self.expires_tick
    }
}

/// A compact identity for a received page used by the continuation fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerOrderedPageStamp {
    acknowledgement: u64,
    fingerprint: u64,
    phase: u8,
}

impl ServerOrderedPageStamp {
    /// Build a page stamp for one of the sixteen bounded contract phases.
    pub fn new(acknowledgement: u64, fingerprint: u64, phase: u8) -> Option<Self> {
        (phase < 16).then_some(Self {
            acknowledgement,
            fingerprint,
            phase,
        })
    }

    /// Return the PageAcknowledgement sequence supplied by the dialogue core.
    pub fn acknowledgement(self) -> u64 {
        self.acknowledgement
    }

    /// Return the exact received-page fingerprint.
    pub fn fingerprint(self) -> u64 {
        self.fingerprint
    }

    /// Return the contract-local phase number.
    pub fn phase(self) -> u8 {
        self.phase
    }
}

/// Ordered proof stage for retiring a failed continuation fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ContinueResetStage {
    /// No reset is in progress.
    Inactive,
    /// The old request must receive a complete chat-closure observation.
    AwaitClosure,
    /// The closure is proven; an owned Talk-to must be reserved before dispatch.
    AwaitOwnedTalkTo,
    /// The new Talk-to was dispatched; its initial page must be observed.
    AwaitInitialPage,
    /// The owned initial page is recorded and can retire the failed-chain fence.
    InitialPageObserved,
    /// A failed or invalidated attempt keeps the fence until an ordered reset.
    ResetRequired,
}

/// Fixed-size current-tick accounting, expectation and continuation-fence state.
/// It contains no snapshot copies, heap allocations, rate limit or retry queue.
#[derive(Debug, Clone, Copy, Default)]
pub struct ServerOrderedInputState {
    observed_tick: Option<u32>,
    session_generation: u64,
    packet_count: u64,
    input_epoch: u64,
    invalidation_epoch: u64,
    category_counts: [u32; 3],
    expectation: Option<ServerOrderedExpectation>,
    continue_exclusion: Option<ContinueExclusion>,
}

#[derive(Debug, Clone, Copy)]
struct ContinueExclusion {
    run_generation: u64,
    request_id: u64,
    start_fingerprint: u64,
    acknowledgement: u64,
    start_acknowledgement: u64,
    reset_request_id: u64,
    reset_input_epoch: u64,
    reset_page_fingerprint: u64,
    reset_deadline_tick: u32,
    contract: ContractKey,
    consumed_phase_mask: u16,
    phase_and_reset_stage: u8,
}

impl ContinueExclusion {
    // Sixteen contract phases and six reset stages fit in one byte. This
    // preserves the measured fixed slot budget without losing page identity.
    fn reset_stage(self) -> ContinueResetStage {
        match self.phase_and_reset_stage >> 4 {
            0 => ContinueResetStage::Inactive,
            1 => ContinueResetStage::AwaitClosure,
            2 => ContinueResetStage::AwaitOwnedTalkTo,
            3 => ContinueResetStage::AwaitInitialPage,
            4 => ContinueResetStage::InitialPageObserved,
            5 => ContinueResetStage::ResetRequired,
            _ => unreachable!("only ContinueResetStage values enter the upper nibble"),
        }
    }

    fn set_reset_stage(&mut self, stage: ContinueResetStage) {
        self.phase_and_reset_stage = self.start_phase() | ((stage as u8) << 4);
    }

    fn start_phase(self) -> u8 {
        self.phase_and_reset_stage & 0x0f
    }
}

/// Immutable, copyable view of slot input accounting and ownership state.
#[derive(Debug, Clone, Copy)]
pub struct ServerOrderedInputSnapshot {
    observed_tick: Option<u32>,
    session_generation: u64,
    packet_count: u64,
    input_epoch: u64,
    invalidation_epoch: u64,
    category_counts: [u32; 3],
    expectation: Option<ServerOrderedExpectation>,
    continue_exclusion: Option<ContinueExclusion>,
}

impl ServerOrderedInputState {
    /// Invalidate ownership from an old session without inventing a tick edge.
    pub fn observe_session(&mut self, session_generation: u64) {
        if self.session_generation != session_generation {
            self.invalidate_content_expectation();
            self.continue_exclusion = None;
            self.session_generation = session_generation;
        }
    }

    /// Observe a PLAYER_INFO tick edge. Repeated wakes preserve counts.
    pub fn observe_tick(&mut self, tick: u64) {
        let Ok(tick) = u32::try_from(tick) else {
            self.observed_tick = None;
            self.invalidate_content_expectation();
            return;
        };
        if self.observed_tick.is_some_and(|observed| observed != tick) {
            self.category_counts = [0; 3];
        }
        self.observed_tick = Some(tick);
        if self
            .expectation
            .is_some_and(|expectation| tick > expectation.expires_tick)
        {
            self.invalidate_content_expectation();
        }
        if self.continue_exclusion.is_some_and(|fence| {
            !matches!(
                fence.reset_stage(),
                ContinueResetStage::Inactive | ContinueResetStage::ResetRequired
            ) && tick > fence.reset_deadline_tick
        }) {
            self.clear_reset_attempt();
        }
    }

    /// Return a copied view suitable for dispatch after releasing the slot mutex.
    pub fn snapshot(&self) -> ServerOrderedInputSnapshot {
        ServerOrderedInputSnapshot {
            observed_tick: self.observed_tick,
            session_generation: self.session_generation,
            packet_count: self.packet_count,
            input_epoch: self.input_epoch,
            invalidation_epoch: self.invalidation_epoch,
            category_counts: self.category_counts,
            expectation: self.expectation,
            continue_exclusion: self.continue_exclusion,
        }
    }

    /// Return the current expectation binding stamp.
    pub fn stamp(&self) -> ServerOrderedInputStamp {
        ServerOrderedInputStamp {
            session_generation: self.session_generation,
            input_epoch: self.input_epoch,
            invalidation_epoch: self.invalidation_epoch,
        }
    }

    /// Count an actual encrypted named packet and invalidate foreign ownership.
    /// Housekeeping and unknown opcodes do not enter input accounting.
    pub fn record_opcode(
        &mut self,
        revision: ClientRevision,
        opcode: u8,
        lineage: InputLineage,
    ) -> Option<PacketCategory> {
        let metadata = packet_metadata(revision, opcode)?;
        let advanced = matches!(lineage, InputLineage::AdvancedRequest(_));
        if advanced {
            self.invalidate_content_expectation();
        }
        if metadata.category == PacketCategory::Housekeeping {
            return Some(metadata.category);
        }

        let foreign_content_request = match lineage {
            InputLineage::Foreign => true,
            InputLineage::ContentRequest(request_id) => {
                self.expectation
                    .is_some_and(|expectation| expectation.predecessor_request_id != request_id)
                    || self.continue_exclusion.is_some_and(|fence| {
                        fence.reset_stage() != ContinueResetStage::Inactive
                            && request_id != fence.request_id
                            && request_id != fence.reset_request_id
                    })
            }
            InputLineage::AdvancedRequest(_) => false,
        };
        if foreign_content_request {
            self.invalidate_content_expectation();
        }

        self.packet_count = self.packet_count.saturating_add(1);
        match metadata.category {
            PacketCategory::UserEvent => {
                self.category_counts[0] = self.category_counts[0].saturating_add(1);
                self.input_epoch = self.input_epoch.saturating_add(1);
            }
            PacketCategory::ClientEvent => {
                self.category_counts[1] = self.category_counts[1].saturating_add(1);
            }
            PacketCategory::RestrictedEvent => {
                self.category_counts[2] = self.category_counts[2].saturating_add(1);
            }
            PacketCategory::Housekeeping => {}
        }
        Some(metadata.category)
    }

    /// Invalidate ownership for local input that may encode no packet.
    pub fn foreign_local_input(&mut self) {
        self.invalidate_content_expectation();
    }

    /// Bind one observed content phase to its run, session and current input epochs.
    pub fn bind_expectation(
        &mut self,
        run_generation: u64,
        predecessor_request_id: u64,
        contract: ContractKey,
        phase: u8,
        expires_tick: u32,
    ) -> Result<(), ServerOrderedInputError> {
        let current_tick = self
            .observed_tick
            .ok_or(ServerOrderedInputError::MissingObservation)?;
        if expires_tick < current_tick {
            return Err(ServerOrderedInputError::ExpiredExpectation);
        }
        self.expectation = Some(ServerOrderedExpectation {
            run_generation,
            predecessor_request_id,
            input_epoch: self.input_epoch,
            invalidation_epoch: self.invalidation_epoch,
            expires_tick,
            contract,
            phase,
        });
        Ok(())
    }

    /// Require the exact active predecessor phase and current ownership epochs.
    pub fn require_expectation(
        &self,
        run_generation: u64,
        contract: ContractKey,
        phase: u8,
        predecessor_request_id: u64,
        now_tick: u32,
    ) -> Result<ServerOrderedExpectation, ServerOrderedInputError> {
        let expectation = self
            .expectation
            .ok_or(ServerOrderedInputError::MissingExpectation)?;
        if self.observed_tick.is_some_and(|tick| now_tick > tick)
            || now_tick > expectation.expires_tick
        {
            return Err(ServerOrderedInputError::ExpiredExpectation);
        }
        if self.observed_tick.is_none()
            || expectation.run_generation != run_generation
            || expectation.input_epoch != self.input_epoch
            || expectation.invalidation_epoch != self.invalidation_epoch
            || expectation.contract != contract
            || expectation.phase != phase
            || expectation.predecessor_request_id != predecessor_request_id
        {
            return Err(ServerOrderedInputError::InvalidatedExpectation);
        }
        Ok(expectation)
    }

    /// Consume the active expectation only when its predecessor matches.
    pub fn retire_expectation(&mut self, predecessor_request_id: u64) -> bool {
        if self
            .expectation
            .is_some_and(|expectation| expectation.predecessor_request_id == predecessor_request_id)
        {
            self.expectation = None;
            true
        } else {
            false
        }
    }

    /// Record a continuation exclusion before a request can encode RESUME_PAUSEBUTTON.
    pub fn begin_continue_exclusion(
        &mut self,
        run_generation: u64,
        request_id: u64,
        contract: ContractKey,
        start: ServerOrderedPageStamp,
    ) -> Result<(), ServerOrderedInputError> {
        if self.observed_tick.is_none() {
            return Err(ServerOrderedInputError::MissingObservation);
        }
        if self.continue_exclusion.is_some() {
            return Err(ServerOrderedInputError::ContinueFenceActive);
        }
        self.continue_exclusion = Some(ContinueExclusion {
            run_generation,
            request_id,
            start_fingerprint: start.fingerprint,
            acknowledgement: start.acknowledgement,
            start_acknowledgement: start.acknowledgement,
            reset_request_id: 0,
            reset_input_epoch: 0,
            reset_page_fingerprint: 0,
            reset_deadline_tick: 0,
            contract,
            consumed_phase_mask: 1u16 << start.phase,
            phase_and_reset_stage: start.phase,
        });
        Ok(())
    }

    /// Whether a page is already consumed by the active contract fence.
    pub fn continue_page_excluded(&self, page: ServerOrderedPageStamp) -> bool {
        self.continue_exclusion.is_some_and(|fence| {
            fence.start_fingerprint == page.fingerprint
                || fence.consumed_phase_mask & (1u16 << page.phase) != 0
        })
    }

    /// Record a newly acknowledged, contract-named successor phase without retiring the fence.
    pub fn record_continue_successor(
        &mut self,
        page: ServerOrderedPageStamp,
    ) -> Result<(), ServerOrderedInputError> {
        let fence = self
            .continue_exclusion
            .as_mut()
            .ok_or(ServerOrderedInputError::ContinueFenceMissing)?;
        if page.acknowledgement <= fence.acknowledgement
            || fence.consumed_phase_mask & (1u16 << page.phase) != 0
        {
            return Err(ServerOrderedInputError::InvalidatedExpectation);
        }
        fence.consumed_phase_mask |= 1u16 << page.phase;
        fence.acknowledgement = page.acknowledgement;
        Ok(())
    }

    /// Retire a fence only after its accepted terminal successor is acknowledged.
    pub fn retire_continue_exclusion(&mut self, terminal_page: ServerOrderedPageStamp) -> bool {
        let Some(fence) = self.continue_exclusion else {
            return false;
        };
        if fence.reset_stage() == ContinueResetStage::Inactive
            && fence.acknowledgement > fence.start_acknowledgement
            && fence.acknowledgement == terminal_page.acknowledgement
            && terminal_page.phase != fence.start_phase()
            && terminal_page.fingerprint != fence.start_fingerprint
            && fence.consumed_phase_mask & (1u16 << terminal_page.phase) != 0
        {
            self.continue_exclusion = None;
            true
        } else {
            false
        }
    }

    /// Begin the required closure → owned Talk-to → initial-page reset.
    pub fn begin_continue_reset(
        &mut self,
        terminal_request_id: u64,
        expires_tick: u32,
    ) -> Result<(), ServerOrderedInputError> {
        let current_tick = self
            .observed_tick
            .ok_or(ServerOrderedInputError::MissingObservation)?;
        if expires_tick < current_tick {
            return Err(ServerOrderedInputError::ExpiredExpectation);
        }
        let fence = self
            .continue_exclusion
            .as_mut()
            .ok_or(ServerOrderedInputError::ContinueFenceMissing)?;
        if terminal_request_id != fence.request_id {
            return Err(ServerOrderedInputError::InvalidatedExpectation);
        }
        fence.set_reset_stage(ContinueResetStage::AwaitClosure);
        fence.reset_request_id = 0;
        fence.reset_input_epoch = 0;
        fence.reset_page_fingerprint = 0;
        fence.reset_deadline_tick = expires_tick;
        Ok(())
    }

    /// Accept a complete post-terminal observation proving chat and Continue are absent.
    pub fn observe_continue_reset_closure(
        &mut self,
        terminal_request_id: u64,
        closure_acknowledgement: u64,
        complete: bool,
        chat_modal_visible: bool,
        continue_visible: bool,
    ) -> bool {
        let Some(fence) = self.continue_exclusion.as_mut() else {
            return false;
        };
        if fence.reset_stage() != ContinueResetStage::AwaitClosure
            || terminal_request_id != fence.request_id
            || !complete
            || chat_modal_visible
            || continue_visible
            || closure_acknowledgement <= fence.acknowledgement
        {
            return false;
        }
        fence.acknowledgement = closure_acknowledgement;
        fence.set_reset_stage(ContinueResetStage::AwaitOwnedTalkTo);
        true
    }

    /// Reserve the exact owned Talk-to identity before its packet reaches the sink.
    pub fn reserve_continue_reset_talk_to(
        &mut self,
        run_generation: u64,
        request_id: u64,
        dispatch_tick: u32,
    ) -> Result<ServerOrderedInputStamp, ServerOrderedInputError> {
        let fence = self
            .continue_exclusion
            .as_mut()
            .ok_or(ServerOrderedInputError::ContinueFenceMissing)?;
        if fence.reset_stage() != ContinueResetStage::AwaitOwnedTalkTo
            || fence.run_generation != run_generation
            || request_id == fence.request_id
            || request_id == 0
            || self.observed_tick != Some(dispatch_tick)
            || dispatch_tick > fence.reset_deadline_tick
        {
            return Err(ServerOrderedInputError::InvalidatedExpectation);
        }
        fence.reset_request_id = request_id;
        fence.reset_input_epoch = self.input_epoch;
        Ok(self.stamp())
    }

    /// Confirm the reserved Talk-to's queued packet under the current session and epochs.
    pub fn record_continue_reset_talk_to(
        &mut self,
        run_generation: u64,
        request_id: u64,
        stamp: ServerOrderedInputStamp,
        dispatch_tick: u32,
    ) -> Result<(), ServerOrderedInputError> {
        let fence = self
            .continue_exclusion
            .as_mut()
            .ok_or(ServerOrderedInputError::ContinueFenceMissing)?;
        if fence.reset_stage() != ContinueResetStage::AwaitOwnedTalkTo
            || fence.run_generation != run_generation
            || request_id != fence.reset_request_id
            || self.observed_tick.is_none()
            || self.observed_tick != Some(dispatch_tick)
            || stamp.session_generation != self.session_generation
            || stamp.input_epoch <= fence.reset_input_epoch
            || stamp.input_epoch != self.input_epoch
            || stamp.invalidation_epoch != self.invalidation_epoch
            || dispatch_tick > fence.reset_deadline_tick
        {
            return Err(ServerOrderedInputError::InvalidatedExpectation);
        }
        fence.reset_input_epoch = stamp.input_epoch;
        fence.set_reset_stage(ContinueResetStage::AwaitInitialPage);
        Ok(())
    }

    /// Record the new conversation's initial page under unchanged owned input lineage.
    pub fn observe_continue_reset_initial_page(
        &mut self,
        run_generation: u64,
        request_id: u64,
        page: ServerOrderedPageStamp,
        stamp: ServerOrderedInputStamp,
        observed_tick: u32,
    ) -> bool {
        let Some(fence) = self.continue_exclusion.as_mut() else {
            return false;
        };
        if fence.reset_stage() == ContinueResetStage::AwaitInitialPage
            && page.phase != fence.start_phase()
            && fence.consumed_phase_mask & (1u16 << page.phase) != 0
        {
            self.clear_reset_attempt();
            return false;
        }
        if fence.reset_stage() != ContinueResetStage::AwaitInitialPage
            || fence.run_generation != run_generation
            || request_id != fence.reset_request_id
            || page.acknowledgement <= fence.acknowledgement
            || page.phase != fence.start_phase()
            || self.observed_tick.is_none()
            || self.observed_tick != Some(observed_tick)
            || stamp.session_generation != self.session_generation
            || stamp.input_epoch != fence.reset_input_epoch
            || stamp.input_epoch != self.input_epoch
            || stamp.invalidation_epoch != self.invalidation_epoch
            || observed_tick > fence.reset_deadline_tick
        {
            return false;
        }
        fence.reset_page_fingerprint = page.fingerprint;
        fence.acknowledgement = page.acknowledgement;
        fence.set_reset_stage(ContinueResetStage::InitialPageObserved);
        true
    }

    /// Retire a failed-chain fence only after its recorded new initial page is acknowledged.
    pub fn retire_continue_reset(&mut self, page: ServerOrderedPageStamp) -> bool {
        let Some(fence) = self.continue_exclusion else {
            return false;
        };
        if fence.reset_stage() == ContinueResetStage::InitialPageObserved
            && fence.reset_page_fingerprint == page.fingerprint
            && fence.acknowledgement == page.acknowledgement
            && page.phase == fence.start_phase()
        {
            self.continue_exclusion = None;
            true
        } else {
            false
        }
    }

    /// Invalidate a content expectation after foreign input or lifecycle loss.
    /// A continuation exclusion remains until a safe acknowledgement or session change.
    pub fn invalidate_content_expectation(&mut self) {
        self.invalidation_epoch = self.invalidation_epoch.saturating_add(1);
        self.expectation = None;
        self.clear_reset_attempt();
    }

    fn clear_reset_attempt(&mut self) {
        if let Some(fence) = self.continue_exclusion.as_mut() {
            fence.reset_request_id = 0;
            fence.reset_input_epoch = 0;
            fence.reset_page_fingerprint = 0;
            fence.reset_deadline_tick = 0;
            fence.set_reset_stage(ContinueResetStage::ResetRequired);
        }
    }
}

impl ServerOrderedInputSnapshot {
    /// Return the latest observed PLAYER_INFO tick, if initialized.
    pub fn observed_tick(self) -> Option<u32> {
        self.observed_tick
    }

    /// Return the session generation.
    pub fn session_generation(self) -> u64 {
        self.session_generation
    }

    /// Return the monotonic count of classified input packets.
    pub fn packet_count(self) -> u64 {
        self.packet_count
    }

    /// Return the USER_EVENT input epoch.
    pub fn input_epoch(self) -> u64 {
        self.input_epoch
    }

    /// Return the expectation invalidation generation.
    pub fn invalidation_epoch(self) -> u64 {
        self.invalidation_epoch
    }

    /// Return counts in USER_EVENT, CLIENT_EVENT, RESTRICTED_EVENT order.
    pub fn category_counts(self) -> [u32; 3] {
        self.category_counts
    }

    /// Return the active content expectation, if one remains stored.
    pub fn expectation(self) -> Option<ServerOrderedExpectation> {
        self.expectation
    }

    /// Return the current input binding stamp.
    pub fn stamp(self) -> ServerOrderedInputStamp {
        ServerOrderedInputStamp {
            session_generation: self.session_generation,
            input_epoch: self.input_epoch,
            invalidation_epoch: self.invalidation_epoch,
        }
    }

    /// Require the exact expectation from this detached snapshot.
    pub fn require_expectation(
        self,
        run_generation: u64,
        contract: ContractKey,
        phase: u8,
        predecessor_request_id: u64,
        now_tick: u32,
    ) -> Result<ServerOrderedExpectation, ServerOrderedInputError> {
        let expectation = self
            .expectation
            .ok_or(ServerOrderedInputError::MissingExpectation)?;
        if now_tick > expectation.expires_tick {
            return Err(ServerOrderedInputError::ExpiredExpectation);
        }
        if self.observed_tick.is_none()
            || expectation.run_generation != run_generation
            || expectation.input_epoch != self.input_epoch
            || expectation.invalidation_epoch != self.invalidation_epoch
            || expectation.contract != contract
            || expectation.phase != phase
            || expectation.predecessor_request_id != predecessor_request_id
        {
            return Err(ServerOrderedInputError::InvalidatedExpectation);
        }
        Ok(expectation)
    }

    /// Whether a page is already consumed by the active contract fence.
    pub fn continue_page_excluded(self, page: ServerOrderedPageStamp) -> bool {
        self.continue_exclusion.is_some_and(|fence| {
            fence.start_fingerprint == page.fingerprint
                || fence.consumed_phase_mask & (1u16 << page.phase) != 0
        })
    }

    /// Return the active continuation's selected content contract.
    pub fn continue_contract(self) -> Option<ContractKey> {
        self.continue_exclusion.map(|fence| fence.contract)
    }

    /// Return the current continuation reset stage.
    pub fn continue_reset_stage(self) -> ContinueResetStage {
        self.continue_exclusion
            .map_or(ContinueResetStage::Inactive, ContinueExclusion::reset_stage)
    }

    /// Return the current input binding stamp.
    pub fn current_tick(self) -> Option<u32> {
        self.observed_tick
    }
}

/// Typed causes for bounded input-state transition refusals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ServerOrderedInputError {
    /// No observed game tick or session has initialized the state.
    #[error("server-ordered input state has no current observation")]
    MissingObservation,
    /// A positive owned phase is required before this dependent input.
    #[error("server-ordered input has no matching owned expectation")]
    MissingExpectation,
    /// The one-use expectation exceeded its observed-tick deadline.
    #[error("server-ordered input expectation expired")]
    ExpiredExpectation,
    /// Ownership, phase, predecessor, page or epoch no longer matches.
    #[error("server-ordered input expectation was invalidated")]
    InvalidatedExpectation,
    /// An earlier continuation still owns the page exclusion.
    #[error("server-ordered continue exclusion is already active")]
    ContinueFenceActive,
    /// No continuation exclusion is active.
    #[error("server-ordered continue exclusion is absent")]
    ContinueFenceMissing,
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::{map_client_prot, ClientProt};

    #[test]
    fn per_slot_state_stays_within_compact_budget() {
        assert!(std::mem::size_of::<ServerOrderedInputState>() <= 184);
    }

    #[test]
    fn only_user_events_advance_input_epoch_and_advanced_client_events_invalidate() {
        let mut state = ServerOrderedInputState::default();
        state.observe_session(2);
        state.observe_tick(10);
        state
            .bind_expectation(7, 11, ContractKey::BankOpenDeposit, 1, 12)
            .unwrap();
        let client_event = map_client_prot(ClientRevision::R289, ClientProt::EVENT_APPLET_FOCUS);
        assert_eq!(
            state.record_opcode(
                ClientRevision::R289,
                client_event.id as u8,
                InputLineage::AdvancedRequest(12),
            ),
            Some(PacketCategory::ClientEvent)
        );
        assert_eq!(state.snapshot().input_epoch(), 0);
        assert!(state.snapshot().expectation().is_none());
        let user_event = map_client_prot(ClientRevision::R289, ClientProt::IF_BUTTON);
        assert_eq!(
            state.record_opcode(
                ClientRevision::R289,
                user_event.id as u8,
                InputLineage::Foreign,
            ),
            Some(PacketCategory::UserEvent)
        );
        assert_eq!(state.snapshot().input_epoch(), 1);
        assert_eq!(state.snapshot().category_counts(), [1, 1, 0]);
        state.observe_session(3);

        assert_eq!(state.snapshot().category_counts(), [1, 1, 0]);
        state.observe_tick(11);
        assert_eq!(state.snapshot().category_counts(), [0; 3]);
    }
    #[test]
    fn out_of_range_observation_invalidates_without_resetting_packet_counts() {
        let mut state = ServerOrderedInputState::default();
        state.observe_session(1);
        state.observe_tick(10);
        state
            .bind_expectation(7, 11, ContractKey::BankOpenDeposit, 1, 12)
            .unwrap();
        let opcode = map_client_prot(ClientRevision::R289, ClientProt::IF_BUTTON).id as u8;
        state.record_opcode(
            ClientRevision::R289,
            opcode,
            InputLineage::ContentRequest(11),
        );
        let before = state.snapshot();

        state.observe_tick(u64::MAX);

        let after = state.snapshot();
        assert_eq!(after.observed_tick(), None);
        assert!(after.expectation().is_none());
        assert_eq!(after.packet_count(), before.packet_count());
        assert_eq!(after.input_epoch(), before.input_epoch());
        assert_eq!(after.category_counts(), before.category_counts());
    }

    #[test]
    fn advanced_housekeeping_invalidates_but_foreign_housekeeping_does_not_count() {
        let mut state = ServerOrderedInputState::default();
        state.observe_session(1);
        state.observe_tick(4);
        state
            .bind_expectation(7, 11, ContractKey::BankOpenDeposit, 1, 12)
            .unwrap();
        let opcode = map_client_prot(ClientRevision::R289, ClientProt::NO_TIMEOUT).id as u8;

        assert_eq!(
            state.record_opcode(ClientRevision::R289, opcode, InputLineage::Foreign),
            Some(PacketCategory::Housekeeping)
        );
        assert!(state.snapshot().expectation().is_some());
        assert_eq!(state.snapshot().packet_count(), 0);

        assert_eq!(
            state.record_opcode(
                ClientRevision::R289,
                opcode,
                InputLineage::AdvancedRequest(99),
            ),
            Some(PacketCategory::Housekeeping)
        );
        assert!(state.snapshot().expectation().is_none());
        assert_eq!(state.snapshot().packet_count(), 0);
    }

    #[test]
    fn foreign_local_input_and_session_change_keep_fence_fail_closed() {
        let mut state = ServerOrderedInputState::default();
        state.observe_session(1);
        state.observe_tick(3);
        state
            .begin_continue_exclusion(
                8,
                9,
                ContractKey::BankOpenDeposit,
                ServerOrderedPageStamp::new(4, 100, 0).unwrap(),
            )
            .unwrap();
        state.foreign_local_input();
        assert!(state
            .snapshot()
            .continue_page_excluded(ServerOrderedPageStamp::new(4, 100, 0).unwrap()));
        state.observe_session(2);
        state.observe_tick(3);
        assert_eq!(
            state.snapshot().continue_reset_stage(),
            ContinueResetStage::Inactive
        );
        assert!(!state
            .snapshot()
            .continue_page_excluded(ServerOrderedPageStamp::new(4, 100, 0).unwrap()));
    }

    fn continuation_state() -> ServerOrderedInputState {
        let mut state = ServerOrderedInputState::default();
        state.observe_session(1);
        state.observe_tick(3);
        state
            .begin_continue_exclusion(
                8,
                9,
                ContractKey::BankOpenDeposit,
                ServerOrderedPageStamp::new(10, 100, 5).unwrap(),
            )
            .unwrap();
        state
    }

    fn reset_after_owned_talk() -> (ServerOrderedInputState, ServerOrderedInputStamp) {
        let mut state = continuation_state();
        state
            .record_continue_successor(ServerOrderedPageStamp::new(11, 101, 6).unwrap())
            .unwrap();
        state.begin_continue_reset(9, 8).unwrap();
        assert!(state.observe_continue_reset_closure(9, 12, true, false, false));
        let before = state.reserve_continue_reset_talk_to(8, 10, 3).unwrap();
        assert_eq!(
            state.record_continue_reset_talk_to(8, 10, before, 3),
            Err(ServerOrderedInputError::InvalidatedExpectation)
        );
        let opcode = map_client_prot(ClientRevision::R289, ClientProt::OPNPC1).id as u8;
        state.record_opcode(
            ClientRevision::R289,
            opcode,
            InputLineage::ContentRequest(10),
        );
        let stamp = state.stamp();
        state
            .record_continue_reset_talk_to(8, 10, stamp, 3)
            .unwrap();
        (state, stamp)
    }

    #[test]
    fn expectation_refusals_keep_exact_predecessor_phase_run_and_tick_authority() {
        let mut state = ServerOrderedInputState::default();
        assert_eq!(
            state.bind_expectation(8, 9, ContractKey::BankDepositX, 1, 8),
            Err(ServerOrderedInputError::MissingObservation)
        );
        state.observe_session(1);
        state.observe_tick(3);
        assert_eq!(
            state.bind_expectation(8, 9, ContractKey::BankDepositX, 1, 2),
            Err(ServerOrderedInputError::ExpiredExpectation)
        );
        state
            .bind_expectation(8, 9, ContractKey::BankDepositX, 1, 8)
            .unwrap();
        for (run, contract, phase, predecessor) in [
            (7, ContractKey::BankDepositX, 1, 9),
            (8, ContractKey::BankWithdrawX, 1, 9),
            (8, ContractKey::BankDepositX, 2, 9),
            (8, ContractKey::BankDepositX, 1, 10),
        ] {
            assert_eq!(
                state.require_expectation(run, contract, phase, predecessor, 3),
                Err(ServerOrderedInputError::InvalidatedExpectation)
            );
        }
        assert_eq!(
            state.require_expectation(8, ContractKey::BankDepositX, 1, 9, 4),
            Err(ServerOrderedInputError::ExpiredExpectation)
        );
        assert!(!state.retire_expectation(10));
        assert!(state.retire_expectation(9));
        assert_eq!(
            state.require_expectation(8, ContractKey::BankDepositX, 1, 9, 3),
            Err(ServerOrderedInputError::MissingExpectation)
        );
    }

    #[test]
    fn continue_fence_requires_a_new_acknowledged_non_consumed_successor() {
        let mut state = continuation_state();
        let start = ServerOrderedPageStamp::new(10, 100, 5).unwrap();
        assert!(!state.retire_continue_exclusion(start));
        assert_eq!(
            state.begin_continue_exclusion(8, 10, ContractKey::BankOpenDeposit, start),
            Err(ServerOrderedInputError::ContinueFenceActive)
        );
        for stale in [
            ServerOrderedPageStamp::new(10, 101, 6).unwrap(),
            ServerOrderedPageStamp::new(11, 101, 5).unwrap(),
        ] {
            assert_eq!(
                state.record_continue_successor(stale),
                Err(ServerOrderedInputError::InvalidatedExpectation)
            );
        }
        let successor = ServerOrderedPageStamp::new(11, 101, 6).unwrap();
        state.record_continue_successor(successor).unwrap();
        assert!(!state.retire_continue_exclusion(start));
        assert!(state.retire_continue_exclusion(successor));
        assert_eq!(
            state.record_continue_successor(successor),
            Err(ServerOrderedInputError::ContinueFenceMissing)
        );
    }

    #[test]
    fn failed_continue_reset_requires_fresh_closure_and_owned_initial_page() {
        let mut state = continuation_state();
        state.begin_continue_reset(9, 8).unwrap();
        assert_eq!(
            state.reserve_continue_reset_talk_to(8, 10, 3),
            Err(ServerOrderedInputError::InvalidatedExpectation)
        );
        for (ack, complete, chat, continue_button) in [
            (10, true, false, false),
            (11, false, false, false),
            (11, true, true, false),
            (11, true, false, true),
        ] {
            assert!(!state.observe_continue_reset_closure(9, ack, complete, chat, continue_button));
        }
        assert!(state.observe_continue_reset_closure(9, 11, true, false, false));
        assert_eq!(
            state.reserve_continue_reset_talk_to(7, 10, 3),
            Err(ServerOrderedInputError::InvalidatedExpectation)
        );

        let (mut state, stamp) = reset_after_owned_talk();
        assert!(!state.retire_continue_reset(ServerOrderedPageStamp::new(12, 0, 5).unwrap()));
        let initial = ServerOrderedPageStamp::new(13, 100, 5).unwrap();
        assert!(!state.observe_continue_reset_initial_page(8, 10, initial, stamp, 4));
        assert!(state.observe_continue_reset_initial_page(8, 10, initial, stamp, 3));
        assert_eq!(
            state.snapshot().continue_reset_stage(),
            ContinueResetStage::InitialPageObserved
        );
        assert!(state.continue_page_excluded(initial));
        assert!(state.retire_continue_reset(initial));
        assert!(!state.continue_page_excluded(initial));
    }

    #[test]
    fn consumed_late_pages_foreign_input_and_expiry_keep_failed_fences_closed() {
        for failure in ["consumed-page", "foreign", "expired", "extra-owned-input"] {
            let (mut state, stamp) = reset_after_owned_talk();
            match failure {
                "consumed-page" => {
                    assert!(!state.observe_continue_reset_initial_page(
                        8,
                        10,
                        ServerOrderedPageStamp::new(13, 101, 6).unwrap(),
                        stamp,
                        3,
                    ));
                }
                "foreign" => state.foreign_local_input(),
                "expired" => state.observe_tick(9),
                "extra-owned-input" => {
                    let opcode = map_client_prot(ClientRevision::R289, ClientProt::OPNPC1).id as u8;
                    state.record_opcode(
                        ClientRevision::R289,
                        opcode,
                        InputLineage::ContentRequest(10),
                    );
                }
                _ => unreachable!(),
            }
            let initial = ServerOrderedPageStamp::new(13, 100, 5).unwrap();
            assert!(!state.observe_continue_reset_initial_page(8, 10, initial, stamp, 3));
            assert!(!state.retire_continue_reset(initial));
            assert!(!state.retire_continue_exclusion(initial));
            assert!(state.continue_page_excluded(initial));
            if failure != "extra-owned-input" {
                assert_eq!(
                    state.snapshot().continue_reset_stage(),
                    ContinueResetStage::ResetRequired,
                    "{failure}"
                );
            }
        }
    }

    #[test]
    fn packed_fence_phase_and_reset_stage_preserve_all_sixteen_phases() {
        let stages = [
            ContinueResetStage::Inactive,
            ContinueResetStage::AwaitClosure,
            ContinueResetStage::AwaitOwnedTalkTo,
            ContinueResetStage::AwaitInitialPage,
            ContinueResetStage::InitialPageObserved,
            ContinueResetStage::ResetRequired,
        ];
        let mut state = continuation_state();
        let fence = state.continue_exclusion.as_mut().unwrap();
        for phase in 0..16 {
            fence.phase_and_reset_stage = phase;
            for stage in stages {
                fence.set_reset_stage(stage);
                assert_eq!(fence.reset_stage(), stage);
                assert_eq!(fence.start_phase(), phase);
            }
        }
        assert!(ServerOrderedPageStamp::new(1, 1, 16).is_none());
        assert_eq!(std::mem::size_of::<ServerOrderedInputState>(), 168);
    }
}
