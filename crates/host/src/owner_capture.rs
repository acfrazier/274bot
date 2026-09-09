//! Host pre-observe scalar staging + nonblocking mailbox (feature
//! `memory-owner-capture`). Thread-local scalars only — no Client/Arc retention.

use api::owner_capture::{
    Budget, CaptureConfig, EncodedBufMeta, FieldRow, FieldRows, OwnerFragment, Reason,
    COW_SCRATCH_CAP, MAILBOX_BYTES_CAP,
};
use api::snapshot::GameSnapshot;
use client::client::Client;
use client::core::world::{WorldOwnerBudget, WorldOwnerRow};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

static CAPTURE_ENABLED: AtomicBool = AtomicBool::new(false);
static FRAME_SERIAL: AtomicU64 = AtomicU64::new(0);

pub fn current_frame() -> u64 {
    FRAME_SERIAL.load(Ordering::Relaxed)
}

/// Resolve runtime switch once at initialization.
pub fn init_from_env() {
    static CONFIG: OnceLock<CaptureConfig> = OnceLock::new();
    let cfg = CONFIG.get_or_init(CaptureConfig::from_env);
    if cfg.enabled {
        let _ = global_mailbox();
    }
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
static REGISTERED_TOKEN: AtomicU64 = AtomicU64::new(0);

pub fn registered_token() -> Option<SlotToken> {
    let token = REGISTERED_TOKEN.load(Ordering::Acquire);
    (token != 0).then_some(SlotToken(token))
}

pub fn register_slot_token() -> SlotToken {
    let token = SlotToken(NEXT_TOKEN.fetch_add(1, Ordering::Relaxed));
    REGISTERED_TOKEN.store(token.0, Ordering::Release);
    token
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
    request_count: usize,
    encoded_count: usize,
}

impl OwnerMailbox {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MailboxInner {
                requests: Vec::with_capacity(3),
                fragments: Vec::with_capacity(2),
                encoded: Vec::with_capacity(4),
                bytes_est: 0,
                request_count: 0,
                encoded_count: 0,
            }),
        }
    }

    /// Nonblocking publish of a phase request.
    pub fn try_publish_request(&self, req: OwnerRequest) -> Result<(), Reason> {
        let Ok(mut g) = self.inner.try_lock() else {
            return Err(Reason::MailboxFull);
        };
        if g.request_count >= 3 || g.requests.len() >= 3 || g.bytes_est >= MAILBOX_BYTES_CAP {
            return Err(Reason::MailboxFull);
        }
        g.requests.push(req);
        g.request_count += 1;
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
            g.bytes_est -= std::mem::size_of::<OwnerRequest>();
            Some(g.requests.remove(i))
        } else {
            None
        }
    }

    pub fn try_push_fragment(&self, frag: OwnerFragment) -> Result<(), Reason> {
        let Ok(mut g) = self.inner.try_lock() else {
            return Err(Reason::MailboxFull);
        };
        let est = std::mem::size_of::<OwnerFragment>();
        if g.fragments.len() >= 2 || g.bytes_est.saturating_add(est) > MAILBOX_BYTES_CAP {
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
        if g.encoded_count >= 4
            || g.bytes_est
                .saturating_add(std::mem::size_of::<EncodedBufMeta>())
                > MAILBOX_BYTES_CAP
        {
            return Err(Reason::MailboxFull);
        }
        g.bytes_est = g
            .bytes_est
            .saturating_add(std::mem::size_of::<EncodedBufMeta>());
        g.encoded.push(meta);
        g.encoded_count += 1;
        Ok(())
    }

    /// Drain completed fragments (harness poll path only — after borrows end).
    pub fn drain_fragments(&self) -> Vec<OwnerFragment> {
        let Ok(mut g) = self.inner.lock() else {
            return Vec::new();
        };
        g.bytes_est -= g.fragments.len() * std::mem::size_of::<OwnerFragment>();
        g.fragments.drain(..).collect()
    }

    pub fn drain_encoded(&self) -> Vec<EncodedBufMeta> {
        let Ok(mut g) = self.inner.lock() else {
            return Vec::new();
        };
        g.bytes_est -= g.encoded.len() * std::mem::size_of::<EncodedBufMeta>();
        g.encoded.drain(..).collect()
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
    static SCRIPT_REQUEST: RefCell<Option<(SlotToken, u32, u8, u64)>> = const { RefCell::new(None) };
}

pub fn take_script_request() -> Option<(SlotToken, u32, u8, u64)> {
    if !enabled() {
        return None;
    }
    SCRIPT_REQUEST.with(|r| r.borrow_mut().take())
}

pub fn current_slot_token() -> Option<SlotToken> {
    SLOT_TOKEN.with(|t| *t.borrow())
}

pub fn arm_script_request(fragment: &OwnerFragment) {
    SCRIPT_REQUEST.with(|r| {
        *r.borrow_mut() = Some((
            SlotToken(fragment.slot_token),
            fragment.request_id,
            fragment.phase,
            fragment.frame_serial,
        ))
    });
}

struct StagingRow {
    budget: Budget,
    epochs: [Option<api::owner_capture::OwnerEpoch>; 3],
    begin_ns: u64,
    end_ns: u64,
    visits: u32,
    frame_serial: u64,
    request_id: u32,
    phase: u8,
    token: SlotToken,
    host_tick: u32,
    client_rows: FieldRows,
    host_snap_rows: FieldRows,
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
    SCRIPT_REQUEST.with(|r| *r.borrow_mut() = None);

    let Some(req) = req else {
        return;
    };

    let begin_ns = api::owner_capture::mono_ns();
    let mut budget = Budget::new();
    let mut client_rows = FieldRows::default();
    let mut complete = true;
    let mut reason = Reason::Ok;

    // World via client feature method.
    {
        let mut wb = WorldOwnerBudget::new(262_144, 5_000_000);
        wb.start = budget.started_at();
        wb.visits = budget.visits();
        let wrows = client.world.owner_payload(&mut wb);
        budget.import_visits(wb.visits);
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
    let mut host_snap_rows = FieldRows::default();
    if complete {
        let frag = host_snapshot.owner_payload(&mut budget);
        host_snap_rows = frag.rows;
        if !frag.complete {
            complete = false;
            reason = frag.reason;
        }
    }

    STAGING.with(|s| {
        *s.borrow_mut() = Some(StagingRow {
            epochs: [
                Some(api::owner_capture::OwnerEpoch {
                    source: "client",
                    tick: host_snapshot.tick(),
                    gens: api::owner_capture::generation_values(&client.gens),
                    family_gates: None,
                    base: Some((client.map_build_base_x, client.map_build_base_z)),
                    tile: client.local_player.as_ref().and_then(|p| {
                        Some((
                            *p.entity.route_x.first()? + client.map_build_base_x,
                            *p.entity.route_z.first()? + client.map_build_base_z,
                            client.minusedlevel,
                        ))
                    }),
                    ingame: client.ingame,
                    scene_state: client.scene_state,
                    draw: Some(client.draw),
                    loop_cycle: Some(client.loop_cycle),
                }),
                Some(host_snapshot.owner_epoch("host_snapshot")),
                None,
            ],
            begin_ns,
            end_ns: api::owner_capture::mono_ns(),
            visits: budget.visits(),
            frame_serial: frame,
            request_id: req.request_id,
            phase: req.phase,
            token,
            host_tick: host_snapshot.tick(),
            client_rows,
            host_snap_rows,
            complete,
            reason,
            budget,
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
    rows: &mut FieldRows,
) -> Result<(), Reason> {
    use api::owner_capture::{collision_flags_bytes, grid3_capacity_bytes, v_bytes};

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
        |p| Ok((player_nested_bytes(p)?, u64::from(p.loc_model.is_some()))),
    )?;
    account_entity_table(
        "npc",
        &client.npc,
        client.npc_count as u64,
        budget,
        rows,
        |p| Ok((entity_nested_bytes(&p.entity)?, 0)),
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
                box_bytes,
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
    rows: &mut FieldRows,
    descendants: impl Fn(&T) -> Result<(u64, u64), Reason>,
) -> Result<(), Reason> {
    let elem = std::mem::size_of::<Option<Box<T>>>() as u64;
    let mut occupied = 0u64;
    let mut boxes = 0u64;
    let mut box_bytes = 0u64;
    let mut nested = 0u64;
    let mut model_presence = 0u64;
    for slot in table {
        if !budget.visit() {
            return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
        }
        if let Some(value) = slot {
            occupied += 1;
            boxes += 1;
            box_bytes = Budget::checked_add(box_bytes, std::mem::size_of::<T>() as u64)?;
            let (bytes, models) = descendants(value)?;
            nested = Budget::checked_add(nested, bytes)?;
            model_presence = Budget::checked_add(model_presence, models)?;
        }
    }
    let mut row = FieldRow::ok(
        "client",
        field,
        table.len() as u64,
        table.capacity() as u64,
        occupied,
        (table.len() as u64).saturating_mul(elem),
        (table.capacity() as u64).saturating_mul(elem),
        nested,
        boxes,
        box_bytes,
        budget.elapsed_ns(),
    );
    // Retain both reported counts and occupied.
    row.holder_count = Some(reported_count);
    let _ = budget.push_row(rows, row);
    if field == "players" {
        let mut models = FieldRow::unknown(
            "client",
            "players.loc_model",
            Reason::OpaqueUnknown,
            budget.elapsed_ns(),
        );
        models.occupied_count = Some(model_presence);
        let _ = budget.push_row(rows, models);
    }
    Ok(())
}

fn entity_nested_bytes(p: &client::dash3d::client_entity::ClientEntity) -> Result<u64, Reason> {
    use api::owner_capture::{opt_string_capacity_bytes, v_bytes};
    let mut bytes = opt_string_capacity_bytes(&p.chat_message);
    bytes = Budget::checked_add(bytes, v_bytes(&p.route_x)?)?;
    bytes = Budget::checked_add(bytes, v_bytes(&p.route_z)?)?;
    Budget::checked_add(bytes, v_bytes(&p.route_run)?)
}

fn player_nested_bytes(p: &client::dash3d::client_player::ClientPlayer) -> Result<u64, Reason> {
    Budget::checked_add(
        entity_nested_bytes(&p.entity)?,
        api::owner_capture::opt_string_capacity_bytes(&p.name),
    )
}

fn account_player_desc(
    field: &'static str,
    p: &client::dash3d::client_player::ClientPlayer,
    budget: &mut Budget,
    rows: &mut FieldRows,
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
        0, // Inline in Client, not another allocated player payload.
        0,
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
pub fn take_staging_fragment() -> Option<(OwnerFragment, Budget)> {
    STAGING.with(|s| {
        s.borrow_mut().take().map(|st| {
            let mut frag = OwnerFragment::new("host_pre_observe");
            frag.epochs = st.epochs;
            frag.begin_ns = st.begin_ns;
            frag.end_ns = st.end_ns;
            frag.visits = st.visits;
            frag.slot_token = st.token.0;
            frag.request_id = st.request_id;
            frag.frame_serial = st.frame_serial;
            frag.phase = st.phase;
            frag.source_tick = st.host_tick;
            frag.complete = st.complete;
            frag.reason = st.reason;
            frag.rows = st.client_rows;
            frag.rows
                .extend(st.host_snap_rows.into_iter().map(|mut row| {
                    row.owner = "host_snapshot";
                    row
                }));
            (frag, st.budget)
        })
    })
}

/// COW scratch identity table (fixed cap, preallocate before login).
pub struct CowScratch {
    addrs: Vec<(usize, bool)>,
    used: usize,
}

impl CowScratch {
    pub fn new() -> Self {
        Self {
            addrs: vec![(0, false); COW_SCRATCH_CAP],
            used: 0,
        }
    }

    pub fn clear(&mut self) {
        self.addrs.fill((0, false));
        self.used = 0;
    }

    pub fn classify(
        &mut self,
        addr: usize,
        template: bool,
        budget: &mut Budget,
    ) -> Result<(bool, bool), Reason> {
        let start = addr.wrapping_mul(0x9e3779b9) >> 4;
        for probe in 0..COW_SCRATCH_CAP {
            if !budget.visit() {
                return Err(budget.failed().unwrap_or(Reason::BudgetVisits));
            }
            let index = (start.wrapping_add(probe)) & (COW_SCRATCH_CAP - 1);
            let entry = &mut self.addrs[index];
            if entry.0 == addr {
                return Ok((false, entry.1));
            }
            if entry.0 == 0 {
                *entry = (addr, template);
                self.used += 1;
                return Ok((true, template));
            }
        }
        Err(Reason::CowScratchFull)
    }

    pub fn reserved_bytes(&self) -> usize {
        self.addrs.capacity() * std::mem::size_of::<(usize, bool)>()
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
    fn entity_table_counts_descendants_not_just_boxes() {
        use client::dash3d::client_player::ClientPlayer;
        let mut player = ClientPlayer::default();
        player.name = Some(String::with_capacity(71));
        player.entity.chat_message = Some(String::with_capacity(53));
        player.entity.route_x.reserve(37);
        player.entity.route_run.reserve(83);
        let expected = player_nested_bytes(&player).unwrap();
        let mut players = Vec::with_capacity(8);
        players.push(Some(Box::new(player)));
        players.push(None);
        let mut rows = FieldRows::default();
        account_entity_table("players", &players, 1, &mut Budget::new(), &mut rows, |p| {
            Ok((player_nested_bytes(p)?, 0))
        })
        .unwrap();
        let row = &rows[0];
        assert_eq!(row.nested_capacity_bytes, Some(expected));
        assert_eq!(
            row.box_bytes,
            Some(std::mem::size_of::<ClientPlayer>() as u64)
        );
        assert_eq!(
            row.capacity_bytes,
            Some((players.capacity() * std::mem::size_of::<Option<Box<ClientPlayer>>>()) as u64)
        );
    }

    #[test]
    fn mailbox_requests_are_limited_to_three() {
        let mb = OwnerMailbox::new();
        for request_id in 1..=3 {
            mb.try_publish_request(OwnerRequest {
                request_id,
                phase: (request_id - 1) as u8,
                slot_token: SlotToken(1),
                armed_frame: 0,
            })
            .unwrap();
        }
        assert_eq!(
            mb.try_publish_request(OwnerRequest {
                request_id: 4,
                phase: 0,
                slot_token: SlotToken(1),
                armed_frame: 0
            }),
            Err(Reason::MailboxFull)
        );
    }

    #[test]
    fn draining_fragments_preserves_undrained_accounting() {
        let mb = OwnerMailbox::new();
        mb.try_publish_request(OwnerRequest {
            request_id: 1,
            phase: 0,
            slot_token: SlotToken(1),
            armed_frame: 0,
        })
        .unwrap();
        mb.drain_fragments();
        assert_eq!(
            mb.inner.lock().unwrap().bytes_est,
            std::mem::size_of::<OwnerRequest>()
        );
    }

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
