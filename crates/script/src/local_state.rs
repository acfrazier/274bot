//! The slot's hand on the process local-state service (T5.2,
//! `21-census-ownership-dag.md` §4.2): the neutral [`LocalStatePort`]
//! capability plus the durable identity the slot reads and writes under.
//!
//! The composition root creates the service and binds one [`SlotState`]
//! per slot ([`crate::SlotScript::bind_local_state`]); the native tick
//! receives the same capability ([`crate::native::NativeTick::state`]), so
//! native cards never rediscover roots or open files. `script` never
//! resolves `HOME`, never opens a default store, and never reads a file
//! for these domains: a cold domain answers [`ErrorCode::NotReady`] and
//! the caller retries on a later turn, or refuses its own work with that
//! typed reason.

use std::sync::Arc;

use api::loadout::Loadout;
use api::local_state::{
    AccountKey, ContentScope, DomainIdentity, DomainLease, ErrorCode, LocalStateDomain,
    LocalStatePort, StoreVersion,
};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

/// The capability one slot holds: the port and the identity its leases
/// and submissions carry. Private fields: the identity is the
/// composition's, never rewritten by a consumer.
pub struct SlotState {
    port: Arc<dyn LocalStatePort + Send + Sync>,
    account: AccountKey,
    scope: ContentScope,
    /// The operator rows last leased, shared by every consumer of this
    /// slot until the store's version moves: one copy and one digest per
    /// version, not per lease.
    loadouts: Mutex<Option<OperatorLoadouts>>,
}

impl SlotState {
    /// Bind `port` for `account` under `scope`.
    pub fn new(
        port: Arc<dyn LocalStatePort + Send + Sync>,
        account: AccountKey,
        scope: ContentScope,
    ) -> Self {
        Self {
            port,
            account,
            scope,
            loadouts: Mutex::new(None),
        }
    }

    /// The port itself, for a consumer that submits or observes.
    pub fn port(&self) -> &(dyn LocalStatePort + Send + Sync) {
        &*self.port
    }

    /// The account the slot reads and writes as.
    pub fn account(&self) -> &AccountKey {
        &self.account
    }

    /// The content scope the slot observes under.
    pub fn scope(&self) -> &ContentScope {
        &self.scope
    }

    /// The durable identity of `domain` for this slot.
    pub fn identity(&self, domain: LocalStateDomain) -> DomainIdentity {
        DomainIdentity::new(domain, self.account.clone(), self.scope.clone())
    }

    /// The current read lease of `domain` for this slot.
    pub fn lease(&self, domain: LocalStateDomain) -> Result<DomainLease, ErrorCode> {
        self.port.lease(&self.identity(domain))
    }

    /// The operator's loadout rows the slot may plan with, at their
    /// current version. `NotReady` while the domain is cold; a refused file
    /// answers its typed failure. The rows of one version are shared: a
    /// later call at the same version hands out the same `Arc`.
    pub fn operator_loadouts(&self) -> Result<OperatorLoadouts, ErrorCode> {
        let lease = self.lease(LocalStateDomain::LoadoutsReferences)?;
        let mut last = self.loadouts.lock();
        if let Some(last) = last.as_ref().filter(|last| last.version == lease.version()) {
            return Ok(last.clone());
        }
        let current = OperatorLoadouts::from_lease(&lease);
        *last = Some(current.clone());
        Ok(current)
    }
}

/// The operator's loadout rows at one version: the immutable input the
/// posted-isolate model and the quester's compile take, shared without
/// copying (`Arc<[Loadout]>`) and keyed for the compile cache by the
/// content digest of the rows, so two processes or two services at the
/// same version with different files never share a compiled path.
#[derive(Debug, Clone)]
pub struct OperatorLoadouts {
    rows: Arc<[Loadout]>,
    version: StoreVersion,
    digest: [u8; 32],
}

impl OperatorLoadouts {
    /// No capability was bound: a composition that opened no state
    /// domains (the legacy direct connection, a bare test slot) plans
    /// with no operator rows. Explicitly absent, not "not ready".
    pub fn absent() -> Self {
        Self::from_rows(Arc::from(Vec::new()), StoreVersion::INITIAL)
    }

    /// The rows a `loadouts-references` lease carries. A lease of another
    /// domain carries no rows.
    pub fn from_lease(lease: &DomainLease) -> Self {
        let rows = lease.snapshot().as_loadouts().unwrap_or(&[]);
        Self::from_rows(Arc::from(rows.to_vec()), lease.version())
    }

    /// Rows a caller already owns (a harness scenario's fixture loadouts,
    /// a test), at `version`.
    pub fn from_rows(rows: Arc<[Loadout]>, version: StoreVersion) -> Self {
        let mut digest = Sha256::new();
        for row in rows.iter() {
            // The wire form is the identity of a row; a serialization
            // failure cannot happen for an owned `Loadout`.
            if let Ok(bytes) = serde_json::to_vec(row) {
                digest.update(&bytes);
                digest.update([0]);
            }
        }
        Self {
            rows,
            version,
            digest: digest.finalize().into(),
        }
    }

    pub fn rows(&self) -> &[Loadout] {
        &self.rows
    }

    /// The rows, shared.
    pub fn shared(&self) -> Arc<[Loadout]> {
        Arc::clone(&self.rows)
    }

    pub fn version(&self) -> StoreVersion {
        self.version
    }

    /// The content digest of the rows: a compile-cache key component.
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::identity::{DurableTimestamp, RequestId};
    use api::local_state::{DomainEdit, LeaseSnapshot, SubmitReceipt, WriteSettlement};

    /// A port whose loadouts lease is always `rows` at `version`, counting
    /// the leases it hands out.
    struct Port {
        version: Mutex<StoreVersion>,
        rows: Vec<Loadout>,
    }

    impl LocalStatePort for Port {
        fn lease(&self, identity: &DomainIdentity) -> Result<DomainLease, ErrorCode> {
            DomainLease::try_new(
                identity.clone(),
                *self.version.lock(),
                DurableTimestamp::from_unix_seconds(1),
                LeaseSnapshot::loadouts(self.rows.clone()),
            )
        }
        fn submit(&self, edit: DomainEdit) -> SubmitReceipt {
            SubmitReceipt::Refused(ErrorCode::Busy {
                domain: edit.identity().domain(),
            })
        }
        fn observe(&self, _: RequestId) -> Option<WriteSettlement> {
            None
        }
    }

    /// T5.2 r2 item 7: the operator rows of one store version are copied and
    /// digested once per slot, then shared, not once per lease.
    #[test]
    fn one_version_of_the_operator_rows_is_shared_not_copied_per_lease() {
        let port = Arc::new(Port {
            version: Mutex::new(StoreVersion::INITIAL),
            rows: vec![Loadout::new("melee")],
        });
        let state = SlotState::new(
            Arc::clone(&port) as Arc<dyn LocalStatePort + Send + Sync>,
            AccountKey::try_new("local-289", "alice").unwrap(),
            ContentScope::try_new(api::selected::ClientRevision::R289, "test-content").unwrap(),
        );
        let first = state.operator_loadouts().unwrap();
        let again = state.operator_loadouts().unwrap();
        assert!(Arc::ptr_eq(&first.shared(), &again.shared()));

        let next = StoreVersion::INITIAL.checked_next().unwrap();
        *port.version.lock() = next;
        let moved = state.operator_loadouts().unwrap();
        assert!(!Arc::ptr_eq(&first.shared(), &moved.shared()));
        assert_eq!(moved.version(), next);
        assert_eq!(moved.rows(), first.rows());
    }
}
