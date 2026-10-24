//! Pure producer entry points. The host owns leases, locking, cancellation,
//! publication, and cache eviction; this module only borrows verified inputs.

pub use super::catalogue::{
    CatalogueCursor, CatalogueProgress, CatalogueResult, CatalogueStage, CatalogueStats,
};
use super::formats::ClientPois;
use super::identity::{CatalogueIdentity, CataloguePolicy, Digest};
use super::{MapError, Text};
use api::identity::Deadline;
use api::work::{
    MonotonicClock, SharedCancellation, SliceBudget, StepContext, SystemClock, WorkCharge,
    WorkPhase,
};
use sha2::{Digest as _, Sha256};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Borrowed view of one verified prepared client snapshot.
///
/// The host must retain the cloned `Arc<PreparedRuntimeCache>` that owns these
/// paths for the entire `'a` borrow and every producer call using it. A path is
/// never a lease: this type deliberately cannot outlive its caller's owner and
/// the map producer never stores or discovers a global/default cache path.
#[derive(Debug, Clone, Copy)]
pub struct ClientMapInput<'a> {
    pub revision: u16,
    pub content: Digest,
    pub jag_dir: &'a Path,
    pub snapshot_dir: &'a Path,
}

impl<'a> ClientMapInput<'a> {
    pub fn new(
        revision: u16,
        content: Digest,
        jag_dir: &'a Path,
        snapshot_dir: &'a Path,
    ) -> Result<Self, MapError> {
        super::revision(revision)?;
        if jag_dir.as_os_str().is_empty() || snapshot_dir.as_os_str().is_empty() {
            return Err(MapError::Path);
        }
        Ok(Self {
            revision,
            content,
            jag_dir,
            snapshot_dir,
        })
    }
}

/// Owned input paths for a resumable producer cursor. These are paths only;
/// each operation opens and closes its own file before returning.
#[derive(Debug)]
pub(crate) struct OwnedClientMapInput {
    revision: u16,
    content: Digest,
    jag_dir: PathBuf,
    snapshot_dir: PathBuf,
}

impl OwnedClientMapInput {
    pub(crate) fn copy(input: ClientMapInput<'_>) -> Result<Self, MapError> {
        ClientMapInput::new(
            input.revision,
            input.content,
            input.jag_dir,
            input.snapshot_dir,
        )?;
        Ok(Self {
            revision: input.revision,
            content: input.content,
            jag_dir: input.jag_dir.to_owned(),
            snapshot_dir: input.snapshot_dir.to_owned(),
        })
    }

    pub(crate) fn view(&self) -> ClientMapInput<'_> {
        ClientMapInput {
            revision: self.revision,
            content: self.content,
            jag_dir: &self.jag_dir,
            snapshot_dir: &self.snapshot_dir,
        }
    }

    pub(crate) fn revision(&self) -> u16 {
        self.revision
    }
}

/// Checked typed-work arithmetic shared by the producer cursors and the host
/// admission adapter. Overflow rejects the input instead of saturating.
pub(crate) fn charge_product(count: usize, bytes_each: usize) -> Result<usize, MapError> {
    count
        .checked_mul(bytes_each)
        .ok_or(MapError::Limit("producer work charge"))
}

pub(crate) fn charge_sum(parts: impl IntoIterator<Item = usize>) -> Result<usize, MapError> {
    parts.into_iter().try_fold(0usize, |sum, part| {
        sum.checked_add(part)
            .ok_or(MapError::Limit("producer work charge"))
    })
}

pub(crate) fn make_work_charge(
    input_bytes: usize,
    scratch_bytes: usize,
    result_bytes: usize,
) -> Result<WorkCharge, MapError> {
    let charge = WorkCharge::new(input_bytes, scratch_bytes, result_bytes, 1);
    charge
        .total_bytes()
        .map(|_| charge)
        .ok_or(MapError::Limit("producer work charge"))
}

/// One bounded producer step's externally visible result.
#[derive(Debug, PartialEq, Eq)]
pub enum ProducerStep<T> {
    /// The cursor is saved and can be resumed on a later WorkAdmission turn.
    Yielded,
    /// The owned producer result is ready.
    Completed(T),
    /// The current WorkAdmission step observed cancellation.
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SyncPhase;

impl WorkPhase for SyncPhase {
    fn index(self) -> u16 {
        0
    }
}

/// Drive a public cursor from a synchronous convenience API using the same
/// ordinary finite slice budget as WorkAdmission.
pub(crate) fn drive_sync<T>(
    mut step: impl FnMut(&mut StepContext<'_, SyncPhase>) -> Result<ProducerStep<T>, MapError>,
) -> Result<T, MapError> {
    const UNITS: u32 = 64;
    const SOFT_MILLIS: u64 = 10;

    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    loop {
        let budget = SliceBudget::new(
            NonZeroU32::new(UNITS).expect("the ordinary sync allowance is nonzero"),
            Deadline::after(clock.now(), Duration::from_millis(SOFT_MILLIS)),
        );
        let mut context = StepContext::<SyncPhase>::new(&clock, budget, &cancellation);
        match step(&mut context)? {
            ProducerStep::Yielded => {}
            ProducerStep::Completed(value) => return Ok(value),
            ProducerStep::Cancelled => {
                return Err(MapError::Invalid("synchronous map cursor cancelled"));
            }
        }
    }
}

pub fn catalogue_policy() -> CataloguePolicy {
    CataloguePolicy {
        algorithm: Text::new("client-loc-mapfunction-v1").expect("static policy text"),
        producer_sources: source_digest(&[
            include_bytes!("catalogue.rs"),
            include_bytes!("client_cache.rs"),
            include_bytes!("poi.rs"),
            include_bytes!("records.rs"),
            include_bytes!("producer.rs"),
            include_bytes!("../collision.rs"),
            include_bytes!("../../../../vendor/fr-client-rust/crates/client/src/map_cache.rs"),
            include_bytes!(
                "../../../../vendor/fr-client-rust/crates/client/src/config/loc_type.rs"
            ),
            include_bytes!(
                "../../../../vendor/fr-client-rust/crates/client/src/config/npc_type.rs"
            ),
            include_bytes!("../../../../crates/api/src/game_data.rs"),
            include_bytes!("../../../../crates/api/data/game-data/manifest.json"),
        ]),
    }
}

pub fn catalogue_identity(input: ClientMapInput<'_>) -> CatalogueIdentity {
    CatalogueIdentity {
        revision: input.revision,
        content: input.content,
        policy: catalogue_policy().identity(),
    }
}

pub fn derive_catalogue(
    input: ClientMapInput<'_>,
) -> Result<(ClientPois, CatalogueStats), MapError> {
    let mut cursor = CatalogueCursor::new(input)?;
    let result = drive_sync(|context| cursor.step(context))?;
    Ok((result.pois, result.stats))
}

pub(crate) fn source_digest(sources: &[&[u8]]) -> Digest {
    let mut hash = Sha256::new();
    hash.update(b"274bot.map.producer-sources\0");
    for source in sources {
        hash.update((source.len() as u64).to_be_bytes());
        hash.update(source);
    }
    Digest(hash.finalize().into())
}
