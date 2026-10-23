//! The slot's single encrypted-input observation point.
//!
//! This state outlives script owners, holds and reconnects. Packet counters
//! describe committed host input. They never admit or refuse input.

use std::sync::{Arc, Mutex, MutexGuard};

use api::interact::server_ordered::{InputLineage, ServerOrderedInputState};
use api::selected::ClientRevision;
use client::io::PacketObserver;

use crate::SlotInput;

pub(crate) struct InputObservation {
    state: Mutex<ObservationState>,
}

#[derive(Clone, Copy)]
enum LineageKind {
    Foreign,
    Content,
    Advanced,
}

struct ObservationState {
    inputs: ServerOrderedInputState,
    revision: ClientRevision,
    lineage: LineageKind,
    request_id: u64,
    committed_packets: u64,
}

/// A poisoned observation lock still yields its state: a panicking observer must never wedge the slot.
fn lock_unpoisoned(state: &Mutex<ObservationState>) -> MutexGuard<'_, ObservationState> {
    state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Default for InputObservation {
    fn default() -> Self {
        Self {
            state: Mutex::new(ObservationState {
                inputs: ServerOrderedInputState::default(),
                revision: ClientRevision::R274,
                lineage: LineageKind::Foreign,
                request_id: 0,
                committed_packets: 0,
            }),
        }
    }
}

/// A frame-scoped attribution. It grants no input authority.
pub struct InputLineageGuard<'a> {
    input: &'a SlotInput,
}

impl Drop for InputLineageGuard<'_> {
    fn drop(&mut self) {
        let mut state = lock_unpoisoned(&self.input.input_observation.state);
        state.lineage = LineageKind::Foreign;
        state.request_id = 0;
    }
}

impl SlotInput {
    /// Bind the one callback used by native, compat and manual client input.
    pub fn bind_outbound_observer(self: &Arc<Self>, client: &mut client::client::Client) {
        self.observe_input_session(client.revision(), client.gens.session);
        let observer: Arc<dyn PacketObserver> = self.clone();
        client.out.set_observer(Some(observer));
    }

    /// Reset only when a new PLAYER_INFO tick is observed. A session change
    /// invalidates ownership but does not create a tick or erase totals.
    pub fn observe_input_tick(&self, revision: ClientRevision, tick: u64, session: u64) {
        let mut state = lock_unpoisoned(&self.input_observation.state);
        state.revision = revision;
        state.inputs.observe_session(session);
        state.inputs.observe_tick(tick);
    }

    /// Bind session identity without inventing a newly received PLAYER_INFO.
    pub fn observe_input_session(&self, revision: ClientRevision, session: u64) {
        let mut state = lock_unpoisoned(&self.input_observation.state);
        state.revision = revision;
        state.inputs.observe_session(session);
    }

    /// Attribute this request's writes without reserving rate capacity.
    pub fn input_lineage(&self, lineage: InputLineage) -> InputLineageGuard<'_> {
        let mut state = lock_unpoisoned(&self.input_observation.state);
        (state.lineage, state.request_id) = match lineage {
            InputLineage::Foreign => (LineageKind::Foreign, 0),
            InputLineage::ContentRequest(id) => (LineageKind::Content, id),
            InputLineage::AdvancedRequest(id) => (LineageKind::Advanced, id),
        };
        InputLineageGuard { input: self }
    }

    /// Inspect or update bounded content ownership without copying a snapshot.
    /// The closure must not emit packets, because the sink takes the same lock.
    pub fn with_server_ordered_input<T>(
        &self,
        apply: impl FnOnce(&mut ServerOrderedInputState) -> T,
    ) -> T {
        apply(&mut lock_unpoisoned(&self.input_observation.state).inputs)
    }

    /// A foreign local edit invalidates content ownership even with no packet.
    pub fn foreign_local_input(&self) {
        self.with_server_ordered_input(ServerOrderedInputState::foreign_local_input);
    }
}

impl PacketObserver for SlotInput {
    fn observed_tick(&self, tick: u64, session: u64) {
        let mut state = lock_unpoisoned(&self.input_observation.state);
        state.inputs.observe_session(session);
        state.inputs.observe_tick(tick);
    }

    fn foreign_local_input(&self) {
        SlotInput::foreign_local_input(self);
    }

    fn packet_body_length(&self, opcode: u8) -> i16 {
        let revision = lock_unpoisoned(&self.input_observation.state).revision;
        api::prot::packet_metadata(revision, opcode).map_or(-3, |metadata| metadata.length as i16)
    }

    fn packet_committed(&self, opcode: u8, _encoded_length: usize) {
        let mut state = lock_unpoisoned(&self.input_observation.state);
        state.committed_packets = state
            .committed_packets
            .checked_add(1)
            .expect("one committed packet per nanosecond takes 584 years to exhaust u64");
        let revision = state.revision;
        let lineage = match state.lineage {
            LineageKind::Foreign => InputLineage::Foreign,
            LineageKind::Content => InputLineage::ContentRequest(state.request_id),
            LineageKind::Advanced => InputLineage::AdvancedRequest(state.request_id),
        };
        state.inputs.record_opcode(revision, opcode, lineage);
    }

    fn committed_packet_count(&self) -> Option<u64> {
        Some(lock_unpoisoned(&self.input_observation.state).committed_packets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_slot_observation_state_has_no_heap_and_pins_fixed_memory() {
        assert_eq!(std::mem::size_of::<ServerOrderedInputState>(), 168);
        assert_eq!(std::mem::size_of::<ObservationState>(), 192);
        #[cfg(target_os = "macos")]
        assert_eq!(std::mem::size_of::<InputObservation>(), 208);
        #[cfg(target_os = "linux")]
        assert_eq!(std::mem::size_of::<InputObservation>(), 200);
        assert_eq!(std::mem::size_of::<InputLineageGuard<'_>>(), 8);
        let input = SlotInput::new();
        let before = input.with_server_ordered_input(|state| state.snapshot());
        input.observe_input_session(ClientRevision::R289, 1);
        input.observe_input_tick(ClientRevision::R289, 1, 1);
        input.foreign_local_input();
        let after = input.with_server_ordered_input(|state| state.snapshot());
        assert_eq!(before.packet_count(), after.packet_count());
        eprintln!(
            "S3 memory: input={} observation={} mutex-owner={} lineage-guard={}; observer state heap=0",
            std::mem::size_of::<ServerOrderedInputState>(),
            std::mem::size_of::<ObservationState>(),
            std::mem::size_of::<InputObservation>(),
            std::mem::size_of::<InputLineageGuard<'_>>(),
        );
    }

    #[test]
    fn outbound_observer_allocates_once_and_commits_allocate_nothing() {
        let input = SlotInput::new();
        input.observe_input_tick(ClientRevision::R289, 1, 1);
        let observer: Arc<dyn PacketObserver> = input.clone();
        let mut packet = client::io::Packet::new(vec![0; 256]);
        let close = api::prot::Send::close_modal();
        close
            .write_for_revision(ClientRevision::R289, &mut packet)
            .unwrap();
        packet.pos = 0;

        let installed = allocation_counter::measure(|| {
            packet.set_observer(Some(Arc::clone(&observer)));
        });
        assert_eq!(installed.count_total, 1);
        assert_eq!(installed.bytes_total, 32);
        assert_eq!(installed.bytes_current, 32);

        let rebound = allocation_counter::measure(|| {
            packet.set_observer(Some(Arc::clone(&observer)));
        });
        assert_eq!(rebound.count_total, 0);
        assert_eq!(rebound.bytes_current, 0);

        let committed = allocation_counter::measure(|| {
            for _ in 0..128 {
                close
                    .write_for_revision(ClientRevision::R289, &mut packet)
                    .unwrap();
                packet.finish_packet();
            }
        });
        assert_eq!(committed.count_total, 0);
        assert_eq!(committed.bytes_current, 0);
        assert_eq!(packet.committed_packet_count(), Some(128));
        assert_eq!(
            input.with_server_ordered_input(|state| state.snapshot().packet_count()),
            128
        );

        let removed = allocation_counter::measure(|| packet.set_observer(None));
        assert_eq!(removed.count_total, 0);
        assert_eq!(removed.bytes_current, -32);
        assert!(!packet.has_observer());
        eprintln!(
            "S3 observer heap: one install={}B; rebind={} allocations; 128 commits={} allocations; retained after detach=0B",
            installed.bytes_current, rebound.count_total, committed.count_total,
        );
    }
}
