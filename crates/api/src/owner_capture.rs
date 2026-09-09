//! Direct per-bot owner capture: budgeted scalar census helpers (feature
//! `memory-owner-capture`). Schema `direct-owner-v1`. No product ownership
//! retention; formulas count capacities, not RSS.

use std::time::Instant;

/// Schema id for JSONL rows.
pub const SCHEMA: &str = "direct-owner-v1";

/// Hard observer limits (plan §4).
pub const MAX_FIELD_ROWS: usize = 128;
pub const MAX_VISITS: u32 = 262_144;
pub const VISIT_CHECK_EVERY: u32 = 256;
pub const FRAGMENT_DEADLINE_NS: u64 = 5_000_000; // 5 ms
pub const COW_SCRATCH_CAP: usize = 16_384;
pub const COW_SCRATCH_BYTES_CAP: usize = 512 * 1024;
pub const MAILBOX_BYTES_CAP: usize = 256 * 1024;
pub const OWNER_JSONL_MAX: usize = 256 * 1024;
pub const RUN_OUTPUT_MAX: usize = 64 * 1024 * 1024;

/// Why a field is incomplete or unknown. Never encode unknown as zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Reason {
    Ok = 0,
    BudgetVisits = 1,
    BudgetDeadline = 2,
    BudgetRows = 3,
    CowScratchFull = 4,
    Overflow = 5,
    StaleFrame = 6,
    Missing = 7,
    Duplicate = 8,
    WrongEpoch = 9,
    Incomplete = 10,
    Malformed = 11,
    IdentityDrift = 12,
    CapDrift = 13,
    OpaqueUnknown = 14,
    FeatureOff = 15,
    RuntimeOff = 16,
    MailboxFull = 17,
    Partial = 18,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Ok => "ok",
            Reason::BudgetVisits => "budget_visits",
            Reason::BudgetDeadline => "budget_deadline",
            Reason::BudgetRows => "budget_rows",
            Reason::CowScratchFull => "cow_scratch_full",
            Reason::Overflow => "overflow",
            Reason::StaleFrame => "stale_frame",
            Reason::Missing => "missing",
            Reason::Duplicate => "duplicate",
            Reason::WrongEpoch => "wrong_epoch",
            Reason::Incomplete => "incomplete",
            Reason::Malformed => "malformed",
            Reason::IdentityDrift => "identity_drift",
            Reason::CapDrift => "cap_drift",
            Reason::OpaqueUnknown => "opaque_unknown",
            Reason::FeatureOff => "feature_off",
            Reason::RuntimeOff => "runtime_off",
            Reason::MailboxFull => "mailbox_full",
            Reason::Partial => "partial",
        }
    }
}

/// One fixed family/field accounting row (scalar only).
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct FieldRow {
    pub owner: &'static str,
    pub field: &'static str,
    pub complete: bool,
    pub reason: Reason,
    pub element_len: Option<u64>,
    pub element_capacity: Option<u64>,
    pub occupied_count: Option<u64>,
    pub occupied_element_bytes: Option<u64>,
    pub capacity_bytes: Option<u64>,
    pub nested_capacity_bytes: Option<u64>,
    pub box_count: Option<u64>,
    pub box_bytes: Option<u64>,
    pub holder_count: Option<u64>,
    pub shared: Option<bool>,
    pub elapsed_observer_ns: u64,
}

impl FieldRow {
    pub fn ok(
        owner: &'static str,
        field: &'static str,
        element_len: u64,
        element_capacity: u64,
        occupied_count: u64,
        occupied_element_bytes: u64,
        capacity_bytes: u64,
        nested_capacity_bytes: u64,
        box_count: u64,
        box_bytes: u64,
        elapsed_observer_ns: u64,
    ) -> Self {
        Self {
            owner,
            field,
            complete: true,
            reason: Reason::Ok,
            element_len: Some(element_len),
            element_capacity: Some(element_capacity),
            occupied_count: Some(occupied_count),
            occupied_element_bytes: Some(occupied_element_bytes),
            capacity_bytes: Some(capacity_bytes),
            nested_capacity_bytes: Some(nested_capacity_bytes),
            box_count: Some(box_count),
            box_bytes: Some(box_bytes),
            holder_count: None,
            shared: None,
            elapsed_observer_ns,
        }
    }

    pub const fn unknown(
        owner: &'static str,
        field: &'static str,
        reason: Reason,
        elapsed_ns: u64,
    ) -> Self {
        Self {
            owner,
            field,
            complete: false,
            reason,
            element_len: None,
            element_capacity: None,
            occupied_count: None,
            occupied_element_bytes: None,
            capacity_bytes: None,
            nested_capacity_bytes: None,
            box_count: None,
            box_bytes: None,
            holder_count: None,
            shared: None,
            elapsed_observer_ns: elapsed_ns,
        }
    }

    pub fn incomplete(
        owner: &'static str,
        field: &'static str,
        reason: Reason,
        elapsed_ns: u64,
    ) -> Self {
        Self::unknown(owner, field, reason, elapsed_ns)
    }
}

/// Inline bounded storage; moving or cloning scalar rows cannot allocate.
#[derive(Debug, Clone)]
pub struct FieldRows {
    storage: [FieldRow; MAX_FIELD_ROWS],
    len: usize,
}

impl Default for FieldRows {
    fn default() -> Self {
        Self {
            storage: [FieldRow::unknown("", "", Reason::Missing, 0); MAX_FIELD_ROWS],
            len: 0,
        }
    }
}

impl FieldRows {
    pub fn capacity(&self) -> usize {
        MAX_FIELD_ROWS
    }
    pub fn push(&mut self, row: FieldRow) {
        assert!(self.len < MAX_FIELD_ROWS, "unbudgeted field row");
        self.storage[self.len] = row;
        self.len += 1;
    }
    pub fn append(&mut self, other: &mut Self) {
        self.extend(other.iter().copied());
        other.len = 0;
    }
}

impl std::ops::Deref for FieldRows {
    type Target = [FieldRow];
    fn deref(&self) -> &Self::Target {
        &self.storage[..self.len]
    }
}

impl Extend<FieldRow> for FieldRows {
    fn extend<T: IntoIterator<Item = FieldRow>>(&mut self, rows: T) {
        for row in rows {
            self.push(row);
        }
    }
}

impl IntoIterator for FieldRows {
    type Item = FieldRow;
    type IntoIter = std::iter::Take<std::array::IntoIter<FieldRow, MAX_FIELD_ROWS>>;
    fn into_iter(self) -> Self::IntoIter {
        self.storage.into_iter().take(self.len)
    }
}

/// Cooperative walk budget for one fragment.
#[derive(Debug)]
pub struct Budget {
    start: Instant,
    visits: u32,
    rows: usize,
    failed: Option<Reason>,
}

impl Budget {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            visits: 0,
            rows: 0,
            failed: None,
        }
    }

    pub fn failed(&self) -> Option<Reason> {
        self.failed
    }

    pub fn visits(&self) -> u32 {
        self.visits
    }

    pub fn started_at(&self) -> Instant {
        self.start
    }

    pub fn import_visits(&mut self, visits: u32) {
        self.visits = visits;
        if visits > MAX_VISITS {
            self.failed = Some(Reason::BudgetVisits);
        }
        if self.elapsed_ns() > FRAGMENT_DEADLINE_NS {
            self.failed = Some(Reason::BudgetDeadline);
        }
    }

    pub fn elapsed_ns(&self) -> u64 {
        self.start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
    }

    /// Record one visited entry/node. Returns false when the fragment must stop.
    pub fn visit(&mut self) -> bool {
        if self.failed.is_some() {
            return false;
        }
        self.visits = self.visits.saturating_add(1);
        if self.visits > MAX_VISITS {
            self.failed = Some(Reason::BudgetVisits);
            return false;
        }
        if self.visits % VISIT_CHECK_EVERY == 0 && self.elapsed_ns() > FRAGMENT_DEADLINE_NS {
            self.failed = Some(Reason::BudgetDeadline);
            return false;
        }
        true
    }

    /// Check deadline before a linked-square step or expensive sub-walk.
    pub fn check_deadline(&mut self) -> bool {
        if self.failed.is_some() {
            return false;
        }
        if self.elapsed_ns() > FRAGMENT_DEADLINE_NS {
            self.failed = Some(Reason::BudgetDeadline);
            return false;
        }
        true
    }

    pub fn push_row(&mut self, rows: &mut FieldRows, row: FieldRow) -> bool {
        if self.failed.is_some() {
            return false;
        }
        if self.rows >= MAX_FIELD_ROWS || rows.len() >= MAX_FIELD_ROWS {
            self.failed = Some(Reason::BudgetRows);
            return false;
        }
        rows.push(row);
        self.rows += 1;
        true
    }

    pub fn checked_add(a: u64, b: u64) -> Result<u64, Reason> {
        a.checked_add(b).ok_or(Reason::Overflow)
    }

    pub fn checked_mul(a: u64, b: u64) -> Result<u64, Reason> {
        a.checked_mul(b).ok_or(Reason::Overflow)
    }
}

impl Default for Budget {
    fn default() -> Self {
        Self::new()
    }
}

/// V(v) = capacity * size_of::<T>().
#[inline]
pub fn vec_capacity_bytes<T>(v: &Vec<T>) -> Result<(u64, u64, u64), Reason> {
    let len = v.len() as u64;
    let cap = v.capacity() as u64;
    let elem = std::mem::size_of::<T>() as u64;
    let bytes = Budget::checked_mul(cap, elem)?;
    let occ = Budget::checked_mul(len, elem)?;
    Ok((len, cap, bytes.max(occ).saturating_sub(0).max(bytes))) // capacity_bytes = cap*size
}

#[inline]
pub fn v_bytes<T>(v: &Vec<T>) -> Result<u64, Reason> {
    Budget::checked_mul(v.capacity() as u64, std::mem::size_of::<T>() as u64)
}

#[inline]
pub fn occ_bytes<T>(v: &Vec<T>) -> Result<u64, Reason> {
    Budget::checked_mul(v.len() as u64, std::mem::size_of::<T>() as u64)
}

/// S(s) = String capacity (logical content is len).
#[inline]
pub fn string_capacity_bytes(s: &str) -> u64 {
    // String capacity is on the owned String; &str has no capacity.
    s.len() as u64
}

#[inline]
pub fn owned_string_capacity_bytes(s: &String) -> u64 {
    s.capacity() as u64
}

#[inline]
pub fn opt_string_capacity_bytes(s: &Option<String>) -> u64 {
    s.as_ref().map(owned_string_capacity_bytes).unwrap_or(0)
}

/// Actions Vec<Option<String>>: V + S for each Some.
pub fn actions_capacity_bytes(actions: &[Option<String>]) -> Result<(u64, u64, u64), Reason> {
    // Prefer capacity when the slice is a Vec; callers pass &Vec via as_slice lose capacity.
    // This helper is for slice-shaped inputs used by legacy helpers; capacity-aware
    // walkers should call `actions_capacity_bytes_vec`.
    let mut nested = 0u64;
    for a in actions {
        nested = Budget::checked_add(nested, opt_string_capacity_bytes(a))?;
    }
    let header = Budget::checked_mul(
        actions.len() as u64,
        std::mem::size_of::<Option<String>>() as u64,
    )?;
    Ok((
        actions.len() as u64,
        actions.len() as u64,
        header.saturating_add(nested),
    ))
}

pub fn actions_capacity_bytes_vec(
    actions: &Vec<Option<String>>,
) -> Result<(u64, u64, u64, u64), Reason> {
    let len = actions.len() as u64;
    let cap = actions.capacity() as u64;
    let header = Budget::checked_mul(cap, std::mem::size_of::<Option<String>>() as u64)?;
    let mut nested = 0u64;
    let mut occupied = 0u64;
    for a in actions {
        if let Some(s) = a {
            occupied = Budget::checked_add(occupied, 1)?;
            nested = Budget::checked_add(nested, owned_string_capacity_bytes(s))?;
        }
    }
    Ok((len, cap, header, nested))
}

pub fn strings_capacity_bytes_vec(v: &Vec<String>) -> Result<(u64, u64, u64, u64), Reason> {
    strings_capacity_bytes_budgeted(v, &mut Budget::new())
}

pub fn strings_capacity_bytes_budgeted(
    v: &Vec<String>,
    budget: &mut Budget,
) -> Result<(u64, u64, u64, u64), Reason> {
    let len = v.len() as u64;
    let cap = v.capacity() as u64;
    let header = Budget::checked_mul(cap, std::mem::size_of::<String>() as u64)?;
    let mut nested = 0u64;
    for s in v {
        if !budget.visit() {
            return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
        }
        nested = Budget::checked_add(nested, owned_string_capacity_bytes(s))?;
    }
    Ok((len, cap, header, nested))
}

/// 3-level grid capacity formula: outer cap * size_of mid-vec, each init mid, each init row.
pub fn grid3_capacity_bytes<T>(
    grid: &Vec<Vec<Vec<T>>>,
    budget: &mut Budget,
) -> Result<
    (
        u64, /*cap_bytes*/
        u64, /*nested not incl leaves*/
        u64, /*visits*/
    ),
    Reason,
> {
    let mut total = Budget::checked_mul(
        grid.capacity() as u64,
        std::mem::size_of::<Vec<Vec<T>>>() as u64,
    )?;
    let mut visits = 0u64;
    for level in grid {
        if !budget.visit() {
            return Err(budget.failed.unwrap_or(Reason::BudgetVisits));
        }
        visits += 1;
        total = Budget::checked_add(
            total,
            Budget::checked_mul(
                level.capacity() as u64,
                std::mem::size_of::<Vec<T>>() as u64,
            )?,
        )?;
        for row in level {
            if !budget.visit() {
                return Err(budget.failed.unwrap_or(Reason::BudgetVisits));
            }
            visits += 1;
            total = Budget::checked_add(
                total,
                Budget::checked_mul(row.capacity() as u64, std::mem::size_of::<T>() as u64)?,
            )?;
        }
    }
    Ok((total, 0, visits))
}

/// Fixed-size row collision flags: capacity * size_of<[i32; N]>.
pub fn collision_flags_bytes(flags: &Vec<[i32; 104]>) -> Result<(u64, u64, u64), Reason> {
    let len = flags.len() as u64;
    let cap = flags.capacity() as u64;
    let bytes = Budget::checked_mul(cap, std::mem::size_of::<[i32; 104]>() as u64)?;
    Ok((len, cap, bytes))
}

/// Fragment of owner rows plus epoch metadata (scalar bridge).
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct OwnerEpoch {
    pub source: &'static str,
    pub tick: u32,
    pub gens: [u64; 11],
    pub family_gates: Option<[u64; 25]>,
    pub base: Option<(i32, i32)>,
    pub tile: Option<(i32, i32, i32)>,
    pub ingame: bool,
    pub scene_state: i32,
    pub draw: Option<bool>,
    pub loop_cycle: Option<i32>,
}

pub fn generation_values(g: &client::client::ClientGens) -> [u64; 11] {
    [
        g.npc, g.player, g.inv, g.varp, g.stat, g.chat, g.scene, g.iface, g.camera, g.map_flag,
        g.world,
    ]
}

pub fn mono_ns() -> u64 {
    static ORIGIN: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    ORIGIN
        .get_or_init(Instant::now)
        .elapsed()
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64
}

/// Fragment of owner rows plus epoch metadata (scalar bridge).
#[derive(Debug, Clone)]
pub struct OwnerFragment {
    pub epochs: [Option<OwnerEpoch>; 3],
    pub script_state: Option<&'static str>,
    pub fingerprint_present: Option<bool>,
    pub slot_token: u64,
    pub request_id: u32,
    pub frame_serial: u64,
    pub phase: u8, // 0=A,1=B,2=C
    pub source: &'static str,
    pub source_tick: u32,
    pub complete: bool,
    pub reason: Reason,
    pub rows: FieldRows,
    pub begin_ns: u64,
    pub end_ns: u64,
    pub visits: u32,
    /// Coverage declarations for omitted/unknown owners.
    pub coverage_unknown: &'static [&'static str],
}

impl OwnerFragment {
    pub fn new(source: &'static str) -> Self {
        Self {
            epochs: [None; 3],
            script_state: None,
            fingerprint_present: None,
            slot_token: 0,
            request_id: 0,
            frame_serial: 0,
            phase: 0,
            source,
            source_tick: 0,
            complete: true,
            reason: Reason::Ok,
            rows: FieldRows::default(),
            begin_ns: 0,
            end_ns: 0,
            visits: 0,
            coverage_unknown: &[],
        }
    }
}

/// Natural encoded buffer metadata (len/capacity only).
#[derive(Debug, Clone, Copy)]
pub struct EncodedBufMeta {
    pub mono_ns: u64,
    pub slot_token: u64,
    pub kind: EncodedBufKind,
    pub len: u64,
    pub capacity: u64,
    pub frame_serial: u64,
    pub request_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodedBufKind {
    InitialKeyframe,
    FirstDelta,
    FirstPostInWindow,
}

/// Runtime switch resolved once at init: `BOT_MEMORY_OWNER_CAPTURE=1`.
#[derive(Debug, Clone, Copy)]
pub struct CaptureConfig {
    pub enabled: bool,
}

impl CaptureConfig {
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var("BOT_MEMORY_OWNER_CAPTURE").as_deref() == Ok("1"),
        }
    }

    pub const fn off() -> Self {
        Self { enabled: false }
    }
}

/// Reject simultaneous snapshot-dedup for v1 capture.
pub fn reject_dedup_combo() -> Result<(), &'static str> {
    if cfg!(feature = "snapshot-dedup") {
        Err("memory-owner-capture rejects simultaneous snapshot-dedup in v1")
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragment_row_storage_is_fixed_before_first_push() {
        let fragment = OwnerFragment::new("generated");
        assert_eq!(fragment.rows.capacity(), MAX_FIELD_ROWS);
        assert!(
            std::mem::size_of_val(&fragment.rows)
                >= MAX_FIELD_ROWS * std::mem::size_of::<FieldRow>()
        );
    }

    #[test]
    fn vec_capacity_counts_spare() {
        let mut v: Vec<i32> = Vec::with_capacity(16);
        v.push(1);
        let (len, cap, bytes) = vec_capacity_bytes(&v).unwrap();
        assert_eq!(len, 1);
        assert!(cap >= 16);
        assert_eq!(bytes, cap * 4);
    }

    #[test]
    fn actions_vec_counts_capacity_not_only_len() {
        let mut v: Vec<Option<String>> = Vec::with_capacity(8);
        v.push(Some("ab".into()));
        v.push(None);
        let (len, cap, header, nested) = actions_capacity_bytes_vec(&v).unwrap();
        assert_eq!(len, 2);
        assert!(cap >= 8);
        assert!(header >= cap * std::mem::size_of::<Option<String>>() as u64);
        assert!(nested >= 2); // "ab" capacity at least 2
    }

    #[test]
    fn budget_stops_on_visit_cap() {
        let mut b = Budget::new();
        b.visits = MAX_VISITS;
        assert!(!b.visit());
        assert_eq!(b.failed(), Some(Reason::BudgetVisits));
    }

    #[test]
    fn unknown_is_not_zero() {
        let r = FieldRow::unknown("builder", "capacity", Reason::OpaqueUnknown, 0);
        assert!(!r.complete);
        assert!(r.capacity_bytes.is_none());
        assert_eq!(r.reason, Reason::OpaqueUnknown);
    }

    #[test]
    fn collision_flags_use_array_stride() {
        let flags: Vec<[i32; 104]> = vec![[0; 104]; 4];
        let (len, cap, bytes) = collision_flags_bytes(&flags).unwrap();
        assert_eq!(len, 4);
        assert!(cap >= 4);
        assert_eq!(bytes, cap * std::mem::size_of::<[i32; 104]>() as u64);
    }
}
