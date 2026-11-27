//! Exact decoded provider for `exact = "packet"` File checks (LD2 §7.5).
//!
//! The role arms one bounded client observer for its selected stat,
//! inventory and varp domains while no fixture seed owns the observer. Each
//! owner poll drains it into the role journal. When a seed, relog or
//! retirement takes the observer, the journal records a packet history gap,
//! so a packet check spanning it fails instead of passing on partial history.

use api::identity::SessionKey;
use api::scenario_observe::{
    AppliedEvent, DecodedDomainEvent, DecodedDomainSelection, InventoryForm, InventoryScope,
    ScenarioObserveAdapter,
};
use client::client::Client;
use scenario::file_journal::{FileJournal, PacketInventoryForm, PacketKinds, PacketRecord};

/// Events drained per owner poll; the client queue holds the same bound.
const PACKET_DRAIN: usize = 4096;

pub(super) const KINDS: PacketKinds = PacketKinds {
    stats: true,
    inventories: true,
    vars: true,
};

pub(super) struct PacketProvider {
    observer: ScenarioObserveAdapter,
    buffer: Box<[Option<AppliedEvent>]>,
}

impl PacketProvider {
    /// Arm the provider for every client-used skill, the carried and worn
    /// inventories and the selected transmitted plain varps.
    pub(super) fn arm(
        client: &mut Client,
        session: SessionKey,
        varps: impl Iterator<Item = i32>,
    ) -> Result<Self, String> {
        let mut selection = DecodedDomainSelection::new().carried_inventory();
        for skill in 0..api::snapshot::stat_count() {
            if api::snapshot::stat_used(skill) {
                selection = selection.stat_skill(skill as i32);
            }
        }
        if let Some(worn) = api::snapshot::tab_inv_component(client, 4) {
            selection = selection.inventory_component(worn);
        }
        for varp in varps {
            selection = selection.varp(varp);
        }
        let observer = ScenarioObserveAdapter::arm(client, session, selection, PACKET_DRAIN)
            .map_err(|err| format!("packet provider unavailable: {err:?}"))?;
        Ok(Self {
            observer,
            buffer: std::iter::repeat_with(|| None).take(PACKET_DRAIN).collect(),
        })
    }

    /// Copy every queued applied event into the journal, in stamp order.
    pub(super) fn drain(
        &mut self,
        client: &mut Client,
        journal: &mut FileJournal,
    ) -> Result<(), String> {
        let drain = self.observer.drain(client, &mut self.buffer);
        if let Some(fault) = drain.fault {
            return Err(format!("packet provider fault: {fault:?}"));
        }
        if drain.remaining != 0 {
            return Err("packet provider queue exceeded one drain".into());
        }
        for event in self.buffer[..drain.written]
            .iter_mut()
            .filter_map(Option::take)
        {
            journal.observe_packet(record(event))?;
        }
        Ok(())
    }

    /// Drain, release the client observer, and mark the history gap.
    pub(super) fn release(
        mut self,
        client: &mut Client,
        journal: &mut FileJournal,
    ) -> Result<(), String> {
        let drained = self.drain(client, journal);
        self.observer.cancel(client);
        journal.set_packet_provider(PacketKinds::default());
        journal.note_packet_gap(KINDS);
        drained
    }
}

fn inventory_form(form: InventoryForm) -> PacketInventoryForm {
    match form {
        InventoryForm::Full => PacketInventoryForm::Full,
        InventoryForm::Partial => PacketInventoryForm::Partial,
        InventoryForm::StopTransmit => PacketInventoryForm::StopTransmit,
    }
}

fn record(applied: AppliedEvent) -> PacketRecord {
    let carried = applied.inventory_scope == Some(InventoryScope::Carried);
    match applied.event {
        DecodedDomainEvent::Stat {
            stamp,
            skill,
            current_level,
            wire_xp,
        } => PacketRecord::Stat {
            skill,
            level: current_level,
            xp: wire_xp,
            source_serial: stamp.source_serial,
            application_serial: stamp.application_serial,
            client_session: stamp.client_session,
        },
        DecodedDomainEvent::Varp {
            stamp, id, value, ..
        } => PacketRecord::Varp {
            id,
            value,
            source_serial: stamp.source_serial,
            application_serial: stamp.application_serial,
            client_session: stamp.client_session,
        },
        DecodedDomainEvent::InventoryMarker {
            stamp,
            component,
            form,
        } => PacketRecord::InventoryReset {
            carried,
            component,
            form: inventory_form(form),
            source_serial: stamp.source_serial,
            application_serial: stamp.application_serial,
            client_session: stamp.client_session,
        },
        DecodedDomainEvent::InventorySlot {
            stamp,
            component,
            slot,
            item_id,
            count,
            form,
        } => PacketRecord::InventorySlot {
            carried,
            component,
            slot: i32::try_from(slot).unwrap_or(i32::MAX),
            id: item_id,
            count,
            form: inventory_form(form),
            source_serial: stamp.source_serial,
            application_serial: stamp.application_serial,
            client_session: stamp.client_session,
        },
    }
}
