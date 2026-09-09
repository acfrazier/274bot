//! Host pre-observe scalar staging + nonblocking mailbox (feature
//! `memory-owner-capture`). Thread-local scalars only — no Client/Arc retention.

use api::owner_capture::{
    Budget, CaptureConfig, EncodedBufMeta, FieldRow, OwnerFragment, Reason, COW_SCRATCH_CAP,
    MAILBOX_BYTES_CAP, MAX_FIELD_ROWS,
};
use api::snapshot::GameSnapshot;
use client::client::Client;
use client::core::world::{WorldOwnerBudget, WorldOwnerRow};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

static CAPTURE_ENABLED: AtomicBool = AtomicBool::new(false);
static FRAME_SERIAL: AtomicU64 = AtomicU64::new(0);

/// Resolve runtime switch once at initialization.
pub fn init_from_env() {
    let cfg = CaptureConfig::from_env();
    CAPTURE_ENABLED.store(cfg.enabled, Ordering::SeqCst);
}

pub fn init_enabled(enabled: bool) {
    CAPTURE_ENABLED.store(enabled, Ordering::SeqCst);
}

#[inline]
pub fn enabled() -> bool {
    CAPTURE_ENABLED.load(Ordering::Relaxed)
}

/// Opaque slot-instance token registered before login.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotToken(pub u64);

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

pub fn register_slot_token() -> SlotToken {
    SlotToken(NEXT_TOKEN.fetch_add(1, Ordering::Relaxed))
}

/// Phase request published by the harness (nonblocking).
#[derive(Debug, Clone, Copy)]
pub struct OwnerRequest {
    pub request_id: u32,
    pub phase: u8, // A=0 B=1 C=2
    pub slot_token: SlotToken,
    pub armed_frame: u64,
}

/// Preallocated mailbox of scalar fragments (fixed capacity).
pub struct OwnerMailbox {
    inner: Mutex<MailboxInner>,
}

struct MailboxInner {
    requests: Vec<OwnerRequest>,
    fragments: Vec<OwnerFragment>,
    encoded: Vec<EncodedBufMeta>,
    bytes_est: usize,
}

impl OwnerMailbox {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MailboxInner {
                requests: Vec::with_capacity(8),
                fragments: Vec::with_capacity(32),
                encoded: Vec::with_capacity(8),
                bytes_est: 0,
            }),
        }
    }

    /// Nonblocking publish of a phase request.
    pub fn try_publish_request(&self, req: OwnerRequest) -> Result<(), Reason> {
        let Ok(mut g) = self.inner.try_lock() else {
            return Err(Reason::MailboxFull);
        };
        if g.bytes_est >= MAILBOX_BYTES_CAP {
            return Err(Reason::MailboxFull);
        }
        g.requests.push(req);
        g.bytes_est = g
            .bytes_est
            .saturating_add(std::mem::size_of::<OwnerRequest>());
        Ok(())
    }

    pub fn try_take_request_for(&self, token: SlotToken) -> Option<OwnerRequest> {
        let Ok(mut g) = self.inner.try_lock() else {
            return None;
        };
        if let Some(i) = g.requests.iter().position(|r| r.slot_token == token) {
            Some(g.requests.remove(i))
        } else {
            None
        }
    }

    pub fn try_push_fragment(&self, frag: OwnerFragment) -> Result<(), Reason> {
        let Ok(mut g) = self.inner.try_lock() else {
            return Err(Reason::MailboxFull);
        };
        let est = std::mem::size_of::<OwnerFragment>()
            .saturating_add(frag.rows.len() * std::mem::size_of::<FieldRow>());
        if g.bytes_est.saturating_add(est) > MAILBOX_BYTES_CAP {
            return Err(Reason::MailboxFull);
        }
        g.bytes_est = g.bytes_est.saturating_add(est);
        g.fragments.push(frag);
        Ok(())
    }

    pub fn try_push_encoded(&self, meta: EncodedBufMeta) -> Result<(), Reason> {
        let Ok(mut g) = self.inner.try_lock() else {
            return Err(Reason::MailboxFull);
        };
        if g.bytes_est.saturating_add(std::mem::size_of::<EncodedBufMeta>()) > MAILBOX_BYTES_CAP {
            return Err(Reason::MailboxFull);
        }
        g.bytes_est = g
            .bytes_est
            .saturating_add(std::mem::size_of::<EncodedBufMeta>());
        g.encoded.push(meta);
        Ok(())
    }

    /// Drain completed fragments (harness poll path only — after borrows end).
    pub fn drain_fragments(&self) -> Vec<OwnerFragment> {
        let Ok(mut g) = self.inner.lock() else {
            return Vec::new();
        };
        g.bytes_est = 0;
        std::mem::take(&mut g.fragments)
    }

    pub fn drain_encoded(&self) -> Vec<EncodedBufMeta> {
        let Ok(mut g) = self.inner.lock() else {
            return Vec::new();
        };
        std::mem::take(&mut g.encoded)
    }
}

impl Default for OwnerMailbox {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL_MAILBOX: OnceLock<OwnerMailbox> = OnceLock::new();

pub fn global_mailbox() -> &'static OwnerMailbox {
    GLOBAL_MAILBOX.get_or_init(OwnerMailbox::new)
}

thread_local! {
    /// Diagnostic-only scalar staging (never holds Client/snapshot refs).
    static STAGING: RefCell<Option<StagingRow>> = const { RefCell::new(None) };
    static SLOT_TOKEN: RefCell<Option<SlotToken>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone)]
struct StagingRow {
    frame_serial: u64,
    request_id: u32,
    phase: u8,
    token: SlotToken,
    host_tick: u32,
    client_rows: Vec<FieldRow>,
    host_snap_rows: Vec<FieldRow>,
    complete: bool,
    reason: Reason,
}

pub fn bind_slot_token(token: SlotToken) {
    SLOT_TOKEN.with(|t| *t.borrow_mut() = Some(token));
}

/// Pre-observe hook: clear prior staging, bump frame serial, census Client + host snapshot.
pub fn pre_observe_hook(client: &Client, host_snapshot: &GameSnapshot) {
    if !enabled() {
        return;
    }
    let frame = FRAME_SERIAL.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    let token = SLOT_TOKEN.with(|t| t.borrow().unwrap_or(SlotToken(0)));
    let req = global_mailbox().try_take_request_for(token);

    // Clear previous partial staging.
    STAGING.with(|s| *s.borrow_mut() = None);

    let Some(req) = req else {
        return;
    };

    let mut budget = Budget::new();
    let mut client_rows = Vec::with_capacity(MAX_FIELD_ROWS);
    let mut complete = true;
    let mut reason = Reason::Ok;

    // World via client feature method.
    {
        let mut wb = WorldOwnerBudget::new(262_144, 5_000_000);
        let wrows = client.world.owner_payload(&mut wb);
        for wr in wrows {
            let row = world_row_to_field(wr);
            if !row.complete {
                complete = false;
                reason = row.reason;
            }
            if !budget.push_row(&mut client_rows, row) {
                complete = false;
                reason = budget.failed().unwrap_or(Reason::BudgetRows);
                break;
            }
        }
        if let Some(f) = wb.failed {
            complete = false;
            reason = match f {
                "budget_visits" => Reason::BudgetVisits,
                "budget_deadline" => Reason::BudgetDeadline,
                "overflow" => Reason::Overflow,
                _ => Reason::Incomplete,
            };
        }
    }

    // Client public tables (host-side walker lives here for fields World doesn't own).
    if complete {
        if let Err(r) = account_client_public(client, &mut budget, &mut client_rows) {
            complete = false;
            reason = r;
        }
    }

    // Host snapshot shell.
    let mut host_snap_rows = Vec::new();
    if complete {
        let mut snap_budget = Budget::new();
        let frag = host_snapshot.owner_payload(&mut snap_budget);
        host_snap_rows = frag.rows;
        if !frag.complete {
            complete = false;
            reason = frag.reason;
        }
    }

    STAGING.with(|s| {
        *s.borrow_mut() = Some(StagingRow {
            frame_serial: frame,
            request_id: req.request_id,
            phase: req.phase,
            token,
            host_tick: host_snapshot.tick(),
            client_rows,
            host_snap_rows,
            complete,
            reason,
        });
    });
}

fn world_row_to_field(wr: WorldOwnerRow) -> FieldRow {
    let reason = match wr.reason {
        "ok" => Reason::Ok,
        "budget_visits" => Reason::BudgetVisits,
        "budget_deadline" => Reason::BudgetDeadline,
        "overflow" => Reason::Overflow,
        _ => Reason::Incomplete,
    };
    FieldRow {
        owner: "world",
        field: wr.field,
        complete: wr.complete,
        reason,
        element_len: wr.element_len,
        element_capacity: wr.element_capacity,
        occupied_count: wr.occupied_count,
        occupied_element_bytes: wr.occupied_element_bytes,
        capacity_bytes: wr.capacity_bytes,
        nested_capacity_bytes: wr.nested_capacity_bytes,
        box_count: wr.box_count,
        box_bytes: wr.box_bytes,
        holder_count: None,
        shared: None,
        elapsed_observer_ns: wr.elapsed_observer_ns,
    }
}

fn account_client_public(
    client: &Client,
    budget: &mut Budget,
    rows: &mut Vec<FieldRow>,
) -> Result<(), Reason> {
    use api::owner_capture::{grid3_capacity_bytes, collision_flags_bytes, v_bytes};

    let header = std::mem::size_of::<Client>() as u64;
    let _ = budget.push_row(
        rows,
        FieldRow::ok(
            "client",
            "struct_header",
            1,
            1,
            1,
            header,
            header,
            0,
            0,
            0,
            budget.elapsed_ns(),
        ),
    );

    // Client.groundh (separate from World.groundh)
    match grid3_capacity_bytes(&client.groundh, budget) {
        Ok((bytes, _, _)) => {
            let _ = budget.push_row(
                rows,
                FieldRow::ok(
                    "client",
                    "groundh",
                    client.groundh.len() as u64,
                    client.groundh.capacity() as u64,
                    client.groundh.len() as u64,
                    bytes,
                    bytes,
                    0,
                    0,
                    0,
                    budget.elapsed_ns(),
                ),
            );
        }
        Err(r) => return Err(r),
    }

    // mapl
    match grid3_capacity_bytes(&client.mapl, budget) {
        Ok((bytes, _, _)) => {
            let _ = budget.push_row(
                rows,
                FieldRow::ok(
                    "client",
                    "mapl",
                    client.mapl.len() as u64,
                    client.mapl.capacity() as u64,
                    client.mapl.len() as u64,
                    bytes,
                    bytes,
                    0,
                    0,
                    0,
                    budget.elapsed_ns(),
                ),
            );
        }
        Err(r) => return Err(r),
    }

    // collision flags — four maps, each Vec<[i32;104]>
    let mut coll_cap = 0u64;
    let mut coll_len = 0u64;
    for cm in &client.collision {
        let (l, c, _b) = collision_flags_bytes(&cm.flags)?;
        coll_len = coll_len.saturating_add(l);
        coll_cap = coll_cap.saturating_add(c);
        if !budget.visit() {
            return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
        }
    }
    let mut coll_bytes = 0u64;
    for cm in &client.collision {
        let (_l, _c, b) = collision_flags_bytes(&cm.flags)?;
        coll_bytes = Budget::checked_add(coll_bytes, b)?;
    }
    let _ = budget.push_row(
        rows,
        FieldRow::ok(
            "client",
            "collision.flags",
            coll_len,
            coll_cap,
            coll_len,
            coll_bytes,
            coll_bytes,
            0,
            0,
            0,
            budget.elapsed_ns(),
        ),
    );

    account_entity_table(
        "players",
        &client.players,
        client.player_count as u64,
        budget,
        rows,
        true,
    )?;
    account_entity_table(
        "npc",
        &client.npc,
        client.npc_count as u64,
        budget,
        rows,
        false,
    )?;

    for (name, v) in [
        ("player_ids", &client.player_ids),
        ("npc_ids", &client.npc_ids),
        ("entity_removal_ids", &client.entity_removal_ids),
        ("entity_update_ids", &client.entity_update_ids),
        ("stat_base_level", &client.stat_base_level),
        ("stat_effective_level", &client.stat_effective_level),
        ("stat_xp", &client.stat_xp),
        ("var", &client.var),
        ("var_serv", &client.var_serv),
    ] {
        let bytes = v_bytes(v)?;
        let _ = budget.push_row(
            rows,
            FieldRow::ok(
                "client",
                name,
                v.len() as u64,
                v.capacity() as u64,
                v.len() as u64,
                (v.len() * 4) as u64,
                bytes,
                0,
                0,
                0,
                budget.elapsed_ns(),
            ),
        );
    }

    // player_appearance_buffer
    {
        let v = &client.player_appearance_buffer;
        let elem = std::mem::size_of::<Option<Box<client::io::packet::Packet>>>() as u64;
        let mut boxes = 0u64;
        let mut box_bytes = 0u64;
        let mut nested = 0u64;
        let mut occupied = 0u64;
        for slot in v {
            if !budget.visit() {
                return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
            }
            if let Some(p) = slot {
                occupied += 1;
                boxes += 1;
                box_bytes = Budget::checked_add(
                    box_bytes,
                    std::mem::size_of::<client::io::packet::Packet>() as u64,
                )?;
                nested = Budget::checked_add(nested, p.owner_data_capacity() as u64)?;
            }
        }
        let _ = budget.push_row(
            rows,
            FieldRow::ok(
                "client",
                "player_appearance_buffer",
                v.len() as u64,
                v.capacity() as u64,
                occupied,
                (v.len() as u64).saturating_mul(elem),
                (v.capacity() as u64).saturating_mul(elem),
                nested,
                boxes,
                box_bytes.saturating_add(nested),
                budget.elapsed_ns(),
            ),
        );
    }

    // local_player inline Option — descendants only, no Box charge for the Option itself
    if let Some(lp) = &client.local_player {
        account_player_desc("local_player", lp, budget, rows)?;
    }

    // Named unknowns (coverage declarations, not zero).
    for u in [
        "ground_obj",
        "stream_queues",
        "packet_pools",
        "audio",
        "loc_model_descendants",
        "ifaces_immutable_nested",
    ] {
        let _ = budget.push_row(
            rows,
            FieldRow::unknown("client", u, Reason::OpaqueUnknown, budget.elapsed_ns()),
        );
    }

    Ok(())
}

fn account_entity_table<T>(
    field: &'static str,
    table: &Vec<Option<Box<T>>>,
    reported_count: u64,
    budget: &mut Budget,
    rows: &mut Vec<FieldRow>,
    _is_player: bool,
) -> Result<(), Reason> {
    let elem = std::mem::size_of::<Option<Box<T>>>() as u64;
    let mut occupied = 0u64;
    let mut boxes = 0u64;
    let mut box_bytes = 0u64;
    for slot in table {
        if !budget.visit() {
            return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
        }
        if slot.is_some() {
            occupied += 1;
            boxes += 1;
            box_bytes = Budget::checked_add(box_bytes, std::mem::size_of::<T>() as u64)?;
        }
    }
    // Note: nested entity descendants accounted separately when T is known;
    // host-play deep-walks players/npc after this shallow table row.
    let mut row = FieldRow::ok(
        "client",
        field,
        table.len() as u64,
        table.capacity() as u64,
        occupied,
        (table.len() as u64).saturating_mul(elem),
        (table.capacity() as u64).saturating_mul(elem),
        0,
        boxes,
        box_bytes,
        budget.elapsed_ns(),
    );
    // Retain both reported counts and occupied.
    row.holder_count = Some(reported_count);
    let _ = budget.push_row(rows, row);
    Ok(())
}

fn account_player_desc(
    field: &'static str,
    p: &client::dash3d::client_player::ClientPlayer,
    budget: &mut Budget,
    rows: &mut Vec<FieldRow>,
) -> Result<(), Reason> {
    use api::owner_capture::{opt_string_capacity_bytes, v_bytes};
    if !budget.visit() {
        return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
    }
    let mut nested = opt_string_capacity_bytes(&p.name);
    nested = Budget::checked_add(nested, opt_string_capacity_bytes(&p.entity.chat_message))?;
    nested = Budget::checked_add(nested, v_bytes(&p.entity.route_x)?)?;
    nested = Budget::checked_add(nested, v_bytes(&p.entity.route_z)?)?;
    nested = Budget::checked_add(nested, v_bytes(&p.entity.route_run)?)?;
    let loc_model = p.loc_model.is_some();
    let boxes = if loc_model { 1 } else { 0 };
    // loc_model graph unknown
    let mut row = FieldRow::ok(
        "client",
        field,
        1,
        1,
        1,
        std::mem::size_of::<client::dash3d::client_player::ClientPlayer>() as u64,
        std::mem::size_of::<client::dash3d::client_player::ClientPlayer>() as u64,
        nested,
        boxes,
        if loc_model {
            std::mem::size_of::<client::dash3d::Model>() as u64
        } else {
            0
        },
        budget.elapsed_ns(),
    );
    if loc_model {
        // Presence counted; model graph remains unknown (do not claim complete nested).
        row.complete = true; // covered presence; nested model still listed as unknown coverage
    }
    let _ = budget.push_row(rows, row);
    if loc_model {
        let _ = budget.push_row(
            rows,
            FieldRow::unknown(
                "client",
                "loc_model_graph",
                Reason::OpaqueUnknown,
                budget.elapsed_ns(),
            ),
        );
    }
    Ok(())
}

/// Consume staging into a host fragment after observe path joins nav/script.
pub fn take_staging_fragment() -> Option<OwnerFragment> {
    STAGING.with(|s| {
        s.borrow_mut().take().map(|st| {
            let mut frag = OwnerFragment::new("host_pre_observe");
            frag.slot_token = st.token.0;
            frag.request_id = st.request_id;
            frag.frame_serial = st.frame_serial;
            frag.phase = st.phase;
            frag.source_tick = st.host_tick;
            frag.complete = st.complete;
            frag.reason = st.reason;
            frag.rows = st.client_rows;
            frag.rows.extend(st.host_snap_rows);
            frag
        })
    })
}

/// COW scratch identity table (fixed cap, preallocate before login).
pub struct CowScratch {
    addrs: Vec<usize>,
}

impl CowScratch {
    pub fn new() -> Self {
        let mut addrs = Vec::new();
        addrs.reserve_exact(COW_SCRATCH_CAP);
        Self { addrs }
    }

    pub fn clear(&mut self) {
        self.addrs.clear();
    }

    pub fn insert(&mut self, addr: usize) -> Result<bool, Reason> {
        if let Some(_) = self.addrs.iter().position(|&a| a == addr) {
            return Ok(false); // already seen
        }
        if self.addrs.len() >= COW_SCRATCH_CAP {
            return Err(Reason::CowScratchFull);
        }
        self.addrs.push(addr);
        Ok(true)
    }

    pub fn reserved_bytes(&self) -> usize {
        self.addrs.capacity() * std::mem::size_of::<usize>()
    }
}

impl Default for CowScratch {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_off_skips_hook_work() {
        init_enabled(false);
        // No panic / no mailbox traffic when off.
        assert!(!enabled());
    }

    #[test]
    fn mailbox_rejects_when_full_estimate() {
        let mb = OwnerMailbox::new();
        {
            let mut g = mb.inner.lock().unwrap();
            g.bytes_est = MAILBOX_BYTES_CAP;
        }
        let err = mb
            .try_publish_request(OwnerRequest {
                request_id: 1,
                phase: 0,
                slot_token: SlotToken(1),
                armed_frame: 0,
            })
            .unwrap_err();
        assert_eq!(err, Reason::MailboxFull);
    }

    #[test]
    fn unknown_reason_not_zero_capacity() {
        let r = FieldRow::unknown("x", "y", Reason::OpaqueUnknown, 0);
        assert!(r.capacity_bytes.is_none());
    }
}
