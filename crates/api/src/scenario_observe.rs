use crate::identity::SessionKey;
#[cfg(test)]
use crate::identity::SlotKey;
use crate::snapshot::tab_inv_component;
use client::client::{Client, DecodedDomainSelection as ClientDecodedDomainSelection};
pub use client::client::{
    DecodedDomainDrain, DecodedDomainEvent, DecodedDomainFault, DecodedDomainStamp,
    DecodedDomainVarpWitness, DecodedDomainWatermark, InventoryForm, VarpForm,
};
use thiserror::Error;

#[cfg(test)]
#[path = "scenario_observe_tests.rs"]
mod tests;

/// API-side selection for bounded decoded observations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecodedDomainSelection {
    stat_skills: Vec<i32>,
    varps: Vec<i32>,
    inventory_components: Vec<i32>,
    carried_inventory: bool,
}

impl DecodedDomainSelection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stat_skill(mut self, skill: i32) -> Self {
        self.stat_skills.push(skill);
        self
    }

    pub fn varp(mut self, id: i32) -> Self {
        self.varps.push(id);
        self
    }

    pub fn inventory_component(mut self, component: i32) -> Self {
        self.inventory_components.push(component);
        self
    }

    pub fn carried_inventory(mut self) -> Self {
        self.carried_inventory = true;
        self
    }
}

/// Host-facing watermark for the selected API session and client session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScenarioObserveWatermark {
    pub session: SessionKey,
    pub client_session: u64,
    pub source_serial: u64,
    pub application_serial: u64,
}

/// The relationship of an observed inventory component to the carried tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryScope {
    Carried,
    Other,
}

/// A decoded client event bound to its host session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppliedEvent {
    pub session: SessionKey,
    pub event: DecodedDomainEvent,
    pub inventory_scope: Option<InventoryScope>,
}

/// Why the API could not arm or query a decoded observer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ScenarioObserveError {
    #[error("client decoded observer failed: {0:?}")]
    Client(DecodedDomainFault),
    #[error("the carried inventory component is unavailable")]
    MissingCarriedInventoryComponent,
}

impl From<DecodedDomainFault> for ScenarioObserveError {
    fn from(fault: DecodedDomainFault) -> Self {
        Self::Client(fault)
    }
}

/// Current watermark, including when the client observer is not armed.
pub fn current_watermark(
    client: &Client,
    session: SessionKey,
) -> Result<ScenarioObserveWatermark, ScenarioObserveError> {
    let watermark = client.decoded_domain_watermark()?;
    Ok(ScenarioObserveWatermark {
        session,
        client_session: watermark.client_session,
        source_serial: watermark.source_serial,
        application_serial: watermark.application_serial,
    })
}

/// API adapter for the bounded client-side decoded observer.
pub struct ScenarioObserveAdapter {
    session: SessionKey,
    carried_inventory_component: Option<i32>,
}

impl ScenarioObserveAdapter {
    /// Resolve selected scopes, then arm the client observer for this session.
    pub fn arm(
        client: &mut Client,
        session: SessionKey,
        selection: DecodedDomainSelection,
        capacity: usize,
    ) -> Result<Self, ScenarioObserveError> {
        let carried_inventory_component = if selection.carried_inventory {
            Some(
                tab_inv_component(client, 3)
                    .ok_or(ScenarioObserveError::MissingCarriedInventoryComponent)?,
            )
        } else {
            None
        };
        let mut client_selection = ClientDecodedDomainSelection {
            stat_skills: selection.stat_skills,
            varps: selection.varps,
            inventory_components: selection.inventory_components,
        };
        if let Some(component) = carried_inventory_component {
            client_selection.inventory_components.push(component);
        }
        client.arm_decoded_domain_observer(client_selection, capacity)?;
        Ok(Self {
            session,
            carried_inventory_component,
        })
    }

    pub fn carried_inventory_component(&self) -> Option<i32> {
        self.carried_inventory_component
    }

    /// Classify an inventory component only after the carried component resolved.
    pub fn inventory_scope(&self, component: i32) -> Option<InventoryScope> {
        self.carried_inventory_component.map(|carried| {
            if component == carried {
                InventoryScope::Carried
            } else {
                InventoryScope::Other
            }
        })
    }

    pub fn watermark(
        &self,
        client: &Client,
    ) -> Result<ScenarioObserveWatermark, ScenarioObserveError> {
        current_watermark(client, self.session)
    }

    /// Drain into caller-owned storage without an intermediate event buffer.
    pub fn drain(
        &self,
        client: &mut Client,
        output: &mut [Option<AppliedEvent>],
    ) -> DecodedDomainDrain {
        let session = self.session;
        let carried = self.carried_inventory_component;
        client.drain_decoded_domain_observer_into(output, |event| {
            let component = match event {
                DecodedDomainEvent::InventoryMarker { component, .. }
                | DecodedDomainEvent::InventorySlot { component, .. } => Some(component),
                DecodedDomainEvent::Stat { .. } | DecodedDomainEvent::Varp { .. } => None,
            };
            let inventory_scope = component.and_then(|component| {
                carried.map(|carried| {
                    if component == carried {
                        InventoryScope::Carried
                    } else {
                        InventoryScope::Other
                    }
                })
            });
            AppliedEvent {
                session,
                event,
                inventory_scope,
            }
        })
    }

    /// Cancel the client queue and release the observer.
    pub fn cancel(&self, client: &mut Client) {
        client.cancel_decoded_domain_observer();
    }
}
