//! Permanent Engine A proof cells for Gatherer recovery at real Catherby.
//!
//! Four independent, fixed-deadline cells cover a transient return stall, the
//! already-working Oyster third-transfer sequence, a scoped one-shot refused
//! deposit, and an explicitly permanent full-stack denial. The cells seed only
//! their own account and never create the fishing resource or a world actor.
//!
//! Each receipt keeps per-tick inventory/bank packet generations, the full
//! inventory/bank observations, status transitions, and parsed native deposit
//! dispatch receipts. The Oyster cell is a baseline control: it requires the
//! exact previously observed three-transfer sequence and records that it did
//! not need a recovery intervention.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::{GameSnapshot, WorldTile};
use parking_lot::Mutex;
use script::native::{HostAuthority, ScriptStatus, StatusValue};
use vault::{Profile, ProfileSettings};

use super::bank_core_live::{check_prerequisites, frame, live_profile, Cell, Prep};
use super::SharedClientTemplate;

const REQUEST: &str = "GATHERER-RECOVERY";
const NAME_PREFIX: &str = "gr24";
const CATHERBY_AREA: &str = "site (2855,3423,0) r32";
const CELL_BOUND: Duration = Duration::from_secs(300);
const PREPARATION_BOUND: Duration = Duration::from_secs(180);
const OVERALL_BOUND: Duration = Duration::from_secs(480);
const FAILURE_RETENTION: Duration = Duration::from_secs(12);
const MIN_RETURN_INTERFERENCE_WRITES: usize = 5;

const SEED_TILE: WorldTile = WorldTile {
    x: 2840,
    z: 3436,
    level: 0,
};
const RETURN_INTERFERENCE_EVEN: WorldTile = WorldTile {
    x: 2845,
    z: 3431,
    level: 0,
};
const RETURN_INTERFERENCE_ODD: WorldTile = WorldTile {
    x: 2845,
    z: 3432,
    level: 0,
};
const HARPOON: i32 = 311;
const SHARK: i32 = 383;
const BIG_NET: i32 = 305;
const MACKEREL: i32 = 353;
const COD: i32 = 341;
const OYSTER: i32 = 407;
const BASS: i32 = 363;

const NORMAL_SEED: &[&str] = &[
    "setstat fishing 99",
    "setstat defence 99",
    "setstat hitpoints 99",
    "give harpoon 1",
    "give raw_shark 27",
    "tele 0,44,53,24,44",
];

const OYSTER_SEED: &[&str] = &[
    "setstat fishing 99",
    "setstat defence 99",
    "setstat hitpoints 99",
    "give big_net 1",
    "give raw_mackerel 1",
    "give raw_cod 1",
    "give oystershell 1",
    "give raw_bass 24",
    "givebank raw_mackerel 1",
    "givebank raw_cod 1",
    "givebank oystershell 523",
    "givebank raw_bass 1",
    "givebank airrune 1",
    "givebank waterrune 1",
    "givebank earthrune 1",
    "givebank firerune 1",
    "givebank mindrune 1",
    "givebank bodyrune 1",
    "givebank cosmicrune 1",
    "givebank chaosrune 1",
    "givebank naturerune 1",
    "givebank deathrune 1",
    "givebank lawrune 1",
    "givebank soulrune 1",
    "givebank bloodrune 1",
    "givebank bronze_bar 1",
    "givebank iron_bar 1",
    "givebank steel_bar 1",
    "givebank mithril_bar 1",
    "givebank adamantite_bar 1",
    "givebank runite_bar 1",
    "givebank silver_bar 1",
    "givebank gold_bar 1",
    "givebank copper_ore 1",
    "givebank tin_ore 1",
    "givebank iron_ore 1",
    "givebank silver_ore 1",
    "givebank gold_ore 1",
    "givebank mithril_ore 1",
    "givebank adamantite_ore 1",
    "givebank logs 1",
    "givebank oak_logs 1",
    "givebank willow_logs 1",
    "givebank maple_logs 1",
    "tele 0,44,53,24,44",
];

const SATURATED_SEED: &[&str] = &[
    "setstat fishing 99",
    "setstat defence 99",
    "setstat hitpoints 99",
    "give big_net 1",
    "give raw_mackerel 1",
    "give raw_cod 1",
    "give oystershell 1",
    "give raw_bass 24",
    "givebank oystershell 2147483647",
    "tele 0,44,53,24,44",
];

#[derive(Default)]
struct Log(Mutex<Vec<String>>);

impl api::hostlog::Sink for Log {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        let mut lines = self.0.lock();
        if lines.len() < 100_000 {
            lines.push(format!("slot={:?} {}", record.slot, record.message));
        }
    }
}

static LOG: Log = Log(Mutex::new(Vec::new()));
static LIVE_RUN_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, serde::Serialize)]
struct DepositFaultEvidence {
    account: String,
    run: String,
    action_id: u64,
    request_id: u64,
    dispatch_tick: u64,
    bank_generation: u64,
    transfer: u8,
    click_tick: u64,
    sequence: u64,
    item_id: i32,
    slot: i32,
    component: i32,
    option: String,
    operation: i32,
    main_before: i32,
    side_before: Option<i32>,
    bank_before: Option<i32>,
    main_revision: Option<u64>,
    side_revision: Option<u64>,
    accepted: bool,
    emitted_packets: u8,
}

struct DepositFaultPlan {
    item_id: i32,
    evidence: Mutex<Option<DepositFaultEvidence>>,
}

static DEPOSIT_FAULTS: LazyLock<Mutex<HashMap<String, Arc<DepositFaultPlan>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn deposit_faults() -> &'static Mutex<HashMap<String, Arc<DepositFaultPlan>>> {
    &DEPOSIT_FAULTS
}

/// Per-account test-only fixture: refuse one real pending Deposit All request
/// before any packet is sent. The caller still completes that action through
/// its ordinary HostAuthority and InteractionReceipt path.
struct ArmedDepositFault {
    account: String,
    plan: Arc<DepositFaultPlan>,
}

impl ArmedDepositFault {
    fn new(account: &str, item_id: i32) -> Self {
        let plan = Arc::new(DepositFaultPlan {
            item_id,
            evidence: Mutex::new(None),
        });
        let previous = deposit_faults()
            .lock()
            .insert(account.to_owned(), Arc::clone(&plan));
        assert!(
            previous.is_none(),
            "deposit fixture already armed for {account}"
        );
        Self {
            account: account.to_owned(),
            plan,
        }
    }

    fn evidence(&self) -> Option<DepositFaultEvidence> {
        self.plan.evidence.lock().clone()
    }
}

impl Drop for ArmedDepositFault {
    fn drop(&mut self) {
        let mut armed = deposit_faults().lock();
        if armed
            .get(&self.account)
            .is_some_and(|plan| Arc::ptr_eq(plan, &self.plan))
        {
            armed.remove(&self.account);
        }
    }
}

/// Test-only host seam called after the slot has validated that the queued
/// action and its `BankDepositTrace` match. Returning true suppresses only this
/// account's first matching seeded-item Deposit All packet; the existing
/// execution path then posts the same request authority with `accepted=false`.
#[cfg(test)]
pub(super) fn refuse_deposit_once(
    account: &str,
    authority: &HostAuthority,
    trace: &script::native::BankDepositTrace,
    tick: u64,
) -> bool {
    if !authority.live() || trace.operation != 4 || trace.option.as_ref() != "Deposit All" {
        return false;
    }
    let plan = deposit_faults().lock().get(account).cloned();
    let Some(plan) = plan.filter(|plan| plan.item_id == trace.id) else {
        return false;
    };
    let mut evidence = plan.evidence.lock();
    if evidence.is_some() {
        return false;
    }
    *evidence = Some(DepositFaultEvidence {
        account: account.to_owned(),
        run: format!("{:?}", authority.run()),
        action_id: authority.action_id().get(),
        request_id: authority.request_id().get(),
        dispatch_tick: tick,
        bank_generation: trace.generation,
        transfer: trace.transfer,
        click_tick: trace.click_tick,
        sequence: trace.sequence,
        item_id: trace.id,
        slot: trace.slot,
        component: trace.component,
        option: trace.option.to_string(),
        operation: trace.operation,
        main_before: trace.main_before,
        side_before: trace.side_before,
        bank_before: trace.bank_before,
        main_revision: trace.main_revision,
        side_revision: trace.side_revision,
        accepted: false,
        emitted_packets: 0,
    });
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    ReturnStall,
    OysterBaseline,
    RefusedDeposit,
    SaturatedStack,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::ReturnStall => "return-stall-recovery",
            Self::OysterBaseline => "oyster-third-transfer-baseline",
            Self::RefusedDeposit => "refused-deposit-recovery",
            Self::SaturatedStack => "saturated-oyster-stack",
        }
    }

    fn seed(self) -> &'static [&'static str] {
        match self {
            Self::ReturnStall | Self::RefusedDeposit => NORMAL_SEED,
            Self::OysterBaseline => OYSTER_SEED,
            Self::SaturatedStack => SATURATED_SEED,
        }
    }

    fn uses_big_net(self) -> bool {
        matches!(self, Self::OysterBaseline | Self::SaturatedStack)
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct DispatchReceipt {
    bank_generation: u64,
    transfer: u8,
    request_id: u64,
    click_tick: u64,
    tick: u64,
    item_id: i32,
    slot: i32,
    component: i32,
    option: String,
    operation: i32,
    accepted: bool,
    emitted_packets: u64,
}

fn token<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let marker = format!("{field}=");
    line.split_whitespace()
        .rev()
        .find_map(|token| token.strip_prefix(marker.as_str()))
}

fn number<T: std::str::FromStr>(line: &str, field: &str) -> Option<T> {
    token(line, field)?.parse().ok()
}

fn parse_dispatch(line: &str) -> Option<DispatchReceipt> {
    if !line.contains("bank-deposit-dispatch ") {
        return None;
    }
    let option = line
        .split_once(" option=")?
        .1
        .split_once(" operation=")?
        .0
        .to_owned();
    Some(DispatchReceipt {
        bank_generation: number(line, "bank_generation")?,
        transfer: number(line, "transfer")?,
        request_id: number(line, "request")?,
        click_tick: number(line, "click_tick")?,
        tick: number(line, "tick")?,
        item_id: number(line, "id")?,
        slot: number(line, "slot")?,
        component: number(line, "component")?,
        option,
        operation: number(line, "operation")?,
        accepted: token(line, "accepted")?.parse().ok()?,
        emitted_packets: number(line, "emitted_packets")?,
    })
}

fn dispatch_receipts(lines: &[String]) -> Vec<DispatchReceipt> {
    lines
        .iter()
        .filter_map(|line| parse_dispatch(line))
        .collect()
}

fn item_count(frame: &serde_json::Value, container: &str, item_id: i32) -> i64 {
    frame[container].as_array().map_or(0, |items| {
        items
            .iter()
            .filter(|item| item["id"].as_i64() == Some(i64::from(item_id)))
            .filter_map(|item| item["count"].as_i64())
            .sum()
    })
}

#[test]
fn gatherer_recovery_multislot_settlement_requires_the_entire_conserved_count() {
    let mut frames = vec![
        serde_json::json!({
            "bank_session_generation": 1,
            "inventory_revision": 7,
            "bank_revision": 3,
            "inventory": [{"id": SHARK, "count": 1}, {"id": SHARK, "count": 1}],
            "bank": [{"id": SHARK, "count": 10}],
        }),
        serde_json::json!({
            "bank_session_generation": 1,
            "inventory_revision": 8,
            "bank_revision": 4,
            "inventory": [],
            "bank": [{"id": SHARK, "count": 12}],
        }),
    ];
    assert_eq!(item_count(&frames[0], "inventory", SHARK), 2);
    assert!(transfer_settled(&frames, SHARK));
    frames[1]["bank"][0]["count"] = serde_json::json!(11);
    assert!(!transfer_settled(&frames, SHARK));
    frames[1]["bank"][0]["count"] = serde_json::json!(12);
    frames[1]["inventory_revision"] = serde_json::json!(7);
    assert!(!transfer_settled(&frames, SHARK));
}

fn transfer_settled(frames: &[serde_json::Value], item_id: i32) -> bool {
    let Some((before_index, before)) = frames.iter().enumerate().find(|(_, frame)| {
        frame["bank_session_generation"]
            .as_u64()
            .is_some_and(|generation| generation > 0)
            && item_count(frame, "inventory", item_id) > 0
    }) else {
        return false;
    };
    let Some(session) = before["bank_session_generation"].as_u64() else {
        return false;
    };
    let Some(before_inventory_revision) = before["inventory_revision"].as_u64() else {
        return false;
    };
    let Some(before_bank_revision) = before["bank_revision"].as_u64() else {
        return false;
    };
    let held_before = item_count(before, "inventory", item_id);
    let bank_before = item_count(before, "bank", item_id);
    frames.iter().skip(before_index + 1).any(|after| {
        after["bank_session_generation"].as_u64() == Some(session)
            && item_count(after, "inventory", item_id) == 0
            && item_count(after, "bank", item_id) == bank_before + held_before
            && after["inventory_revision"]
                .as_u64()
                .is_some_and(|revision| revision != before_inventory_revision)
            && after["bank_revision"]
                .as_u64()
                .is_some_and(|revision| revision != before_bank_revision)
    })
}

fn oyster_sequence_settled(dispatches: &[DispatchReceipt], frames: &[serde_json::Value]) -> bool {
    let expected = [
        (1, MACKEREL, 1, 2006, 4),
        (2, COD, 2, 2006, 4),
        (3, OYSTER, 3, 2006, 4),
    ];
    if dispatches.len() < expected.len()
        || !dispatches.iter().zip(expected).all(
            |(actual, (transfer, item_id, slot, component, operation))| {
                actual.bank_generation > 0
                    && actual.transfer == transfer
                    && actual.item_id == item_id
                    && actual.slot == slot
                    && actual.component == component
                    && actual.operation == operation
                    && actual.option == "Deposit All"
                    && actual.accepted
                    && actual.emitted_packets > 0
            },
        )
    {
        return false;
    }
    [MACKEREL, COD, OYSTER]
        .into_iter()
        .all(|item_id| transfer_settled(frames, item_id))
}

fn refused_deposit_recovered(
    fault: &DepositFaultEvidence,
    dispatches: &[DispatchReceipt],
    frames: &[serde_json::Value],
) -> bool {
    let refusal_logged = dispatches.iter().any(|dispatch| {
        dispatch.request_id == fault.request_id
            && dispatch.tick == fault.dispatch_tick
            && dispatch.bank_generation == fault.bank_generation
            && dispatch.transfer == fault.transfer
            && dispatch.click_tick == fault.click_tick
            && dispatch.item_id == fault.item_id
            && dispatch.slot == fault.slot
            && dispatch.component == fault.component
            && dispatch.option == fault.option
            && dispatch.operation == fault.operation
            && !dispatch.accepted
            && dispatch.emitted_packets == 0
    });
    let retry = dispatches.iter().find(|dispatch| {
        dispatch.item_id == fault.item_id
            && dispatch.request_id != fault.request_id
            && dispatch.tick > fault.dispatch_tick
            && dispatch.bank_generation == fault.bank_generation
            && dispatch.transfer == fault.transfer
            && dispatch.click_tick > fault.click_tick
            && dispatch.accepted
            && dispatch.emitted_packets > 0
    });
    refusal_logged
        && !fault.accepted
        && fault.emitted_packets == 0
        && retry.is_some_and(|_| transfer_settled(frames, fault.item_id))
}

#[derive(Debug, Default)]
struct ReturnFault {
    armed: bool,
    active: bool,
    activated: bool,
    writes: Vec<serde_json::Value>,
    initial_request: Option<u64>,
    recovery_requests: Vec<u64>,
    recovery_event: Option<serde_json::Value>,
    ceased_after_recovery: bool,
    ceased_tick: Option<u32>,
}

#[derive(Debug, Default)]
struct Recorder {
    frames: Vec<serde_json::Value>,
    statuses: Vec<serde_json::Value>,
    target_verified: bool,
    return_fault: ReturnFault,
    saw_bank_return: bool,
    bank_return_completed: bool,
    catch_after_bank_return: bool,
    return_yielded_baseline: Option<i64>,
    return_xp_baseline: Option<i64>,
    return_status: Option<serde_json::Value>,
}

fn text(status: &ScriptStatus, key: &str) -> String {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            StatusValue::Text(value) => Some(value.to_string()),
            _ => None,
        })
        .unwrap_or_default()
}

fn integer_value(status: &ScriptStatus, key: &str) -> Option<i64> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            StatusValue::Integer(value) => Some(*value),
            _ => None,
        })
}

fn integer(status: &ScriptStatus, key: &str) -> i64 {
    integer_value(status, key).unwrap_or(0)
}

fn fishing_xp(snapshot: &GameSnapshot) -> Option<i64> {
    snapshot
        .stats()
        .iter()
        .find(|stat| stat.name.eq_ignore_ascii_case("fishing"))
        .map(|stat| i64::from(stat.xp))
}

fn status_json(
    status: &ScriptStatus,
    tick: Option<u32>,
    tile: Option<(i32, i32, i32)>,
) -> serde_json::Value {
    serde_json::json!({
        "tick": tick,
        "tile": tile_array(tile),
        "run": format!("{:?}", status.run),
        "phase": format!("{:?}", status.phase),
        "method": text(status, "method"),
        "area": text(status, "area"),
        "target": text(status, "target"),
        "bank": text(status, "bank"),
        "last_event": text(status, "last_event"),
        "trips": integer(status, "trips"),
        "yielded": integer(status, "yielded"),
        "failure": status.failure.as_ref().map(|failure| serde_json::json!({
            "code": failure.code.to_string(),
            "message": failure.message.to_string(),
        })),
    })
}

fn latest_tick(record: &Recorder) -> Option<u32> {
    record
        .frames
        .last()
        .and_then(|frame| frame["tick"].as_u64())
        .and_then(|tick| u32::try_from(tick).ok())
}

fn tile_from_json(tile: &serde_json::Value) -> Option<(i32, i32, i32)> {
    let tile = tile.as_array()?;
    Some((
        i32::try_from(tile.first()?.as_i64()?).ok()?,
        i32::try_from(tile.get(1)?.as_i64()?).ok()?,
        i32::try_from(tile.get(2)?.as_i64()?).ok()?,
    ))
}

fn latest_tile(record: &Recorder) -> Option<(i32, i32, i32)> {
    tile_from_json(record.frames.last()?.get("tile")?)
}

fn normal_seed_ready(snapshot: &GameSnapshot) -> bool {
    snapshot.tile() == Some((SEED_TILE.x, SEED_TILE.z, SEED_TILE.level))
        && snapshot
            .inventory()
            .iter()
            .any(|item| item.def.id == HARPOON)
        && snapshot
            .inventory()
            .iter()
            .filter(|item| item.def.id == SHARK)
            .count()
            == 27
}

fn big_net_seed_ready(snapshot: &GameSnapshot) -> bool {
    snapshot.tile() == Some((SEED_TILE.x, SEED_TILE.z, SEED_TILE.level))
        && snapshot
            .inventory()
            .iter()
            .any(|item| item.def.id == BIG_NET)
        && snapshot
            .inventory()
            .iter()
            .any(|item| item.def.id == OYSTER)
        && snapshot.inventory().len() == 28
}

fn mode_settings(mode: Mode) -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "skill": "Fishing",
        "fishingMethod": if mode.uses_big_net() {
            "fishing.memberfish.op1"
        } else {
            "fishing.memberfish.op3"
        },
        "location": "Site",
        "site": "fishing.catherby",
        "radius": 32,
        "disposition": "Bank",
        "bank": "Nearest",
        "allowTeleports": false,
        "allowWilderness": false,
    })
    .as_object()
    .expect("settings are an object")
    .clone()
}

fn tile_array(tile: Option<(i32, i32, i32)>) -> Option<[i32; 3]> {
    tile.map(|(x, z, level)| [x, z, level])
}

fn frame_json(
    snapshot: &GameSnapshot,
    fault_active: bool,
    sent_teleport: bool,
) -> serde_json::Value {
    let local = snapshot.local_player();
    serde_json::json!({
        "tick": snapshot.tick(),
        "tile": tile_array(snapshot.tile()),
        "network": local.map(|player| player.player.network),
        "moving": local.map(|player| player.player.actor.moving),
        "bank_component": snapshot.bank_component_id(),
        "bank_session_generation": snapshot.bank_session_generation(),
        "inventory_revision": snapshot.inventory_packet_generation(),
        "bank_revision": snapshot.bank_snapshot_generation(),
        "bank_side_revision": snapshot.bank_side_packet_generation(),
        "inventory_capacity": snapshot.inventory_size(),
        "inventory": snapshot.inventory().iter().map(|item| serde_json::json!({
            "id": item.def.id,
            "count": item.count,
            "slot": item.slot,
            "component": item.component_id,
        })).collect::<Vec<_>>(),
        "bank": snapshot.bank().iter().map(|item| serde_json::json!({
            "id": item.def.id,
            "count": item.count,
            "slot": item.slot,
            "component": item.component_id,
        })).collect::<Vec<_>>(),
        "bank_side": snapshot.bank_side().iter().map(|item| serde_json::json!({
            "id": item.def.id,
            "count": item.count,
            "slot": item.slot,
            "component": item.component_id,
        })).collect::<Vec<_>>(),
        "fishing_xp": fishing_xp(snapshot),
        "chat": snapshot.chat_lines().iter().take(12).map(|line| serde_json::json!({
            "sequence": line.sequence,
            "text": line.text,
        })).collect::<Vec<_>>(),
        "return_interference_active": fault_active,
        "return_interference_teleport_sent": sent_teleport,
    })
}

fn fish_site_verified(status: &ScriptStatus) -> bool {
    text(status, "method") == "0_44_53_memberfish" && text(status, "area") == CATHERBY_AREA
}

fn tile_in_fishing_site(tile: (i32, i32, i32)) -> bool {
    let (x, z, level) = tile;
    let dx = i64::from(x) - 2855;
    let dz = i64::from(z) - 3423;
    level == 0 && dx * dx + dz * dz <= 32 * 32
}

fn fish_site_arrived(status: &ScriptStatus, tile: Option<(i32, i32, i32)>) -> bool {
    fish_site_verified(status) && tile.is_some_and(tile_in_fishing_site)
}

fn receipt_has_site_arrival(receipt: &serde_json::Value) -> bool {
    let status = &receipt["return_status"];
    status["method"] == "0_44_53_memberfish"
        && status["area"].as_str() == Some(CATHERBY_AREA)
        && tile_from_json(&status["tile"]).is_some_and(tile_in_fishing_site)
}

fn return_trigger_ready(status: &ScriptStatus, tile: Option<(i32, i32, i32)>) -> bool {
    text(status, "bank").ends_with("; Return")
        && fish_site_verified(status)
        && tile.is_some_and(|(x, z, level)| {
            // Match the diagnosed final-hop fixture, after its accepted send
            // from (2835,3436), not an earlier hop from the bank.
            level == 0 && (2837..=2843).contains(&x) && z <= 3436
        })
}

fn failure_json(status: &ScriptStatus) -> Option<serde_json::Value> {
    status.failure.as_ref().map(|failure| {
        serde_json::json!({
            "code": failure.code.to_string(),
            "message": failure.message.to_string(),
        })
    })
}

fn message_has_full_stack(failure: &serde_json::Value) -> bool {
    failure["message"]
        .as_str()
        .is_some_and(|message| message == "bank deposit refused: full stack in bank")
}

fn saturated_stack_preflight(receipt: &serde_json::Value) -> bool {
    let bank_has_max_stack = receipt["facts"].as_array().is_some_and(|frames| {
        frames
            .iter()
            .any(|frame| item_count(frame, "bank", OYSTER) == i64::from(i32::MAX))
    });
    let no_oyster_dispatch = receipt["dispatches"].as_array().is_some_and(|dispatches| {
        dispatches
            .iter()
            .all(|dispatch| dispatch["item_id"].as_i64() != Some(i64::from(OYSTER)))
    });
    bank_has_max_stack && no_oyster_dispatch
}

fn receipt_has_exact_oyster_sequence(receipt: &serde_json::Value) -> bool {
    let Some(dispatches) = receipt["dispatches"].as_array() else {
        return false;
    };
    let Some(bank_generation) = dispatches
        .first()
        .and_then(|dispatch| dispatch["bank_generation"].as_u64())
    else {
        return false;
    };
    let expected = [
        (1_u64, MACKEREL, 1, 2006, 4),
        (2, COD, 2, 2006, 4),
        (3, OYSTER, 3, 2006, 4),
    ];
    bank_generation > 0
        && dispatches.len() >= expected.len()
        && dispatches.iter().zip(expected).all(
            |(actual, (transfer, item_id, slot, component, operation))| {
                actual["bank_generation"].as_u64() == Some(bank_generation)
                    && actual["request_id"]
                        .as_u64()
                        .is_some_and(|request| request > 0)
                    && actual["transfer"].as_u64() == Some(transfer)
                    && actual["item_id"].as_i64() == Some(i64::from(item_id))
                    && actual["slot"].as_i64() == Some(i64::from(slot))
                    && actual["component"].as_i64() == Some(i64::from(component))
                    && actual["operation"].as_i64() == Some(i64::from(operation))
                    && actual["option"] == "Deposit All"
                    && actual["accepted"] == true
                    && actual["emitted_packets"]
                        .as_u64()
                        .is_some_and(|count| count > 0)
            },
        )
}
fn valid_outcome(mode: Mode, receipt: &serde_json::Value) -> bool {
    if receipt["request"] != REQUEST
        || receipt["mode"] != mode.name()
        || receipt["cell_elapsed_ms"]
            .as_u64()
            .is_none_or(|elapsed| elapsed > CELL_BOUND.as_millis() as u64)
    {
        return false;
    }
    match mode {
        Mode::ReturnStall => {
            receipt["result"] == "bank-return-and-real-catch"
                && receipt["target_verified"] == true
                && receipt["return_fault"]["armed"] == true
                && receipt["return_fault"]["activated"] == true
                && receipt["return_fault"]["active_writes"]
                    .as_u64()
                    .is_some_and(|writes| writes >= MIN_RETURN_INTERFERENCE_WRITES as u64)
                && receipt["return_fault"]["recovery_event_observed"] == true
                && receipt["return_fault"]["recovery_actions"].as_i64() == Some(1)
                && receipt["return_fault"]["initial_native_request_id"]
                    .as_u64()
                    .is_some_and(|initial| {
                        initial != 0
                            && receipt["return_fault"]["recovery_native_request_ids"]
                                .as_array()
                                .is_some_and(|requests| {
                                    requests.len() == 1
                                        && requests[0].as_u64().is_some_and(|request| {
                                            request != 0 && request != initial
                                        })
                                })
                    })
                && receipt["return_fault"]["interference_ceased_after_recovery"] == true
                && receipt["bank_return_completed"] == true
                && receipt["bank_return_completed_after_recovery"] == true
                && receipt_has_site_arrival(receipt)
                && receipt["real_catch_after_bank_return"] == true
        }
        Mode::OysterBaseline => {
            receipt["result"] == "bank-return-and-real-catch"
                && receipt["target_verified"] == true
                && receipt["oyster_sequence"]["settled"] == true
                && receipt["oyster_sequence"]["baseline_behavior_unchanged"] == true
                && receipt_has_exact_oyster_sequence(receipt)
                && receipt["bank_return_completed"] == true
                && receipt_has_site_arrival(receipt)
                && receipt["real_catch_after_bank_return"] == true
        }
        Mode::RefusedDeposit => {
            let fault = &receipt["deposit_fault"];
            let details = &fault["fault"];
            let recovered = &receipt["deposit_recovery"];
            receipt["result"] == "bank-return-and-real-catch"
                && receipt["target_verified"] == true
                && fault["armed"] == true
                && fault["activated"] == true
                && fault["accepted"] == false
                && fault["emitted_packets"].as_u64() == Some(0)
                && fault["request_id"]
                    .as_u64()
                    .is_some_and(|request| request > 0)
                && fault["action_id"].as_u64().is_some_and(|action| action > 0)
                && details["action_id"].as_u64() == fault["action_id"].as_u64()
                && details["request_id"].as_u64() == fault["request_id"].as_u64()
                && details["run"].as_str().is_some_and(|run| !run.is_empty())
                && details["account"] == receipt["account"]
                && details["item_id"].as_i64() == Some(i64::from(SHARK))
                && details["component"].as_i64() == Some(2006)
                && details["option"] == "Deposit All"
                && details["operation"].as_i64() == Some(4)
                && details["bank_generation"]
                    .as_u64()
                    .is_some_and(|generation| generation > 0)
                && details["main_before"].as_i64() == Some(27)
                && details["slot"].as_i64().is_some_and(|slot| slot > 0)
                && details["transfer"].as_u64() == Some(1)
                && details["click_tick"].as_u64().is_some_and(|tick| tick > 0)
                && details["dispatch_tick"]
                    .as_u64()
                    .is_some_and(|tick| tick > 0)
                && details["accepted"] == false
                && details["emitted_packets"].as_u64() == Some(0)
                && recovered["same_item_retried"] == true
                && recovered["transfer_settled"] == true
                && receipt["bank_return_completed"] == true
                && receipt_has_site_arrival(receipt)
                && receipt["real_catch_after_bank_return"] == true
        }
        Mode::SaturatedStack => {
            receipt["result"] == "blocked-for-12s"
                && receipt["settings"]["site"] == "fishing.catherby"
                && receipt["blocked_for_ms"]
                    .as_u64()
                    .is_some_and(|elapsed| elapsed >= FAILURE_RETENTION.as_millis() as u64)
                && message_has_full_stack(&receipt["failure"])
                && saturated_stack_preflight(receipt)
        }
    }
}

fn new_cell(mode: Mode, evidence_dir: PathBuf) -> Arc<Mutex<Cell>> {
    let seed_ready: fn(&GameSnapshot) -> bool = if mode.uses_big_net() {
        big_net_seed_ready
    } else {
        normal_seed_ready
    };
    Arc::new(Mutex::new(Cell::new(
        CELL_BOUND,
        mode.seed(),
        seed_ready,
        &[HARPOON, SHARK, BIG_NET, MACKEREL, COD, OYSTER, BASS],
        serde_json::json!({
            "request": REQUEST,
            "mode": mode.name(),
            "bounds_ms": {
                "preparation": PREPARATION_BOUND.as_millis(),
                "cell": CELL_BOUND.as_millis(),
                "overall": OVERALL_BOUND.as_millis(),
            },
            "settings": mode_settings(mode),
            "seed": mode.seed(),
        }),
        |_| true,
        evidence_dir,
    )))
}

fn run_live(mode: Mode) {
    let _run_guard = LIVE_RUN_LOCK.lock();
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"), "requires LIVE=1");
    assert_eq!(
        std::env::var("BOT_LIVE_NAME_PREFIX").as_deref(),
        Ok(NAME_PREFIX),
        "the live cell uses its four-character gr24 account prefix"
    );
    assert!(NAME_PREFIX.len() <= 4);
    let root = PathBuf::from(
        std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR names evidence root"),
    );
    assert!(root.is_absolute(), "evidence root must be absolute");
    let account = super::mint_live_names(1)
        .pop()
        .expect("mint one own live account");
    assert!(
        account.starts_with(NAME_PREFIX),
        "account must use gr24 prefix"
    );
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let evidence_dir = root.join(format!(
        "gatherer-recovery-{}-{account}-{epoch}",
        mode.name()
    ));
    std::fs::create_dir_all(&evidence_dir).expect("create live evidence directory");

    LOG.0.lock().clear();
    api::hostlog::set_debug(true);
    super::set_debug(true);
    let _ = api::hostlog::install_sink(&LOG);

    let scratch = evidence_dir.join("scratch");
    std::fs::create_dir_all(&scratch).expect("create profile scratch");
    let options = live_profile(&scratch).expect("live local-289 profile options");
    let profile = options
        .resolve(None)
        .and_then(|resolved| resolved.bind())
        .expect("bind live local-289 profile");
    assert_eq!(
        profile.client().game_host(),
        "127.0.0.1",
        "loopback Engine A only"
    );
    let cell = new_cell(mode, evidence_dir.clone());
    let deposit_fault =
        (mode == Mode::RefusedDeposit).then(|| ArmedDepositFault::new(&account, SHARK));
    let frame_cell = Arc::clone(&cell);
    let frame_account = account.clone();
    let frame_recorder = Arc::new(Mutex::new(Recorder::default()));
    let hook_recorder = Arc::clone(&frame_recorder);
    let last_tick = Mutex::new(None);
    let hook_snapshot = Mutex::new(GameSnapshot::new());
    let template = SharedClientTemplate::load(Arc::clone(&profile)).expect("client template");
    let play = super::run_with_template(
        template,
        true,
        vec![Profile {
            username: account.clone(),
            password: account.clone().into(),
            uid: 274_279_231,
            settings: ProfileSettings::default(),
        }],
        |_| (None, None),
        move |client, username, frame_result| {
            if username != frame_account {
                return;
            }
            frame(client, &frame_cell, &frame_account);
            if !client.ingame || client.scene_state != 2 {
                return;
            }
            let mut snapshot = hook_snapshot.lock();
            snapshot.rebuild(client);
            let tick = snapshot.tick();
            let mut last_tick = last_tick.lock();
            if *last_tick == Some(tick) {
                return;
            }
            *last_tick = Some(tick);
            drop(last_tick);

            let mut recorder = hook_recorder.lock();
            let fault_active = mode == Mode::ReturnStall && recorder.return_fault.active;
            let destination = if fault_active && !frame_result.hold {
                Some(if recorder.return_fault.writes.len() % 2 == 0 {
                    RETURN_INTERFERENCE_EVEN
                } else {
                    RETURN_INTERFERENCE_ODD
                })
            } else {
                None
            };
            let sent = destination.is_some_and(|tile| {
                interact::cheat(client, &interact::tele_args(0, tile.x, tile.z)).is_sent()
            });
            recorder
                .frames
                .push(frame_json(&snapshot, fault_active, sent));
            if let Some(destination) = destination {
                if sent {
                    recorder.return_fault.activated = true;
                    recorder.return_fault.writes.push(serde_json::json!({
                        "tick": tick,
                        "from": tile_array(snapshot.tile()),
                        "to": [destination.x, destination.z, destination.level],
                        "sent": true,
                    }));
                }
            }
        },
    )
    .unwrap_or_else(|error| {
        panic!("HARNESS PREREQUISITE FAILURE (before the cell deadline): {error}")
    });

    let preparation_started = cell.lock().preparation_started;
    let mut login_ready = false;
    let mut started = false;
    let mut last_status = String::new();
    let mut last_failure_key = String::new();
    let mut blocked_since = None;
    let overall_deadline = Instant::now() + OVERALL_BOUND;
    let mut result = "cell-deadline";
    let mut final_failure = None;
    let mut blocked_for_ms = 0_u64;
    let mut completion_status = None;

    loop {
        check_prerequisites(
            &play,
            &account,
            preparation_started,
            &mut login_ready,
            &cell,
        );
        let now = Instant::now();
        if now >= overall_deadline {
            break;
        }
        let cell_started = cell.lock().started;
        if cell_started.is_some_and(|at| now.duration_since(at) >= CELL_BOUND) {
            break;
        }
        if !started
            && cell.lock().phase == Prep::Ready
            && cell.lock().last_action.elapsed() >= Duration::from_secs(1)
        {
            play.script_start(
                &account,
                script::CompiledId("Gatherer"),
                mode_settings(mode),
            )
            .expect("start the native Gatherer");
            started = true;
        }
        if started {
            if let Some(status) = play.script_native_status(&account) {
                let (tick, tile) = {
                    let mut recorder = frame_recorder.lock();
                    let tick = latest_tick(&recorder);
                    let tile = latest_tile(&recorder);
                    recorder.target_verified |= fish_site_verified(&status);
                    (tick, tile)
                };
                let row = status_json(&status, tick, tile);
                let status_key = format!("{:?}", status);
                if status_key != last_status {
                    frame_recorder.lock().statuses.push(row.clone());
                    last_status = status_key;
                }

                let event = text(&status, "last_event");
                let yielded = integer(&status, "yielded");
                let xp =
                    frame_recorder.lock().frames.last().and_then(|frame| {
                        frame.get("fishing_xp").and_then(serde_json::Value::as_i64)
                    });

                if mode == Mode::ReturnStall {
                    // Observe a new native authority, not a status string or
                    // an inner Traveller retry of the same walk request.
                    let request = play
                        .navs
                        .lock()
                        .unwrap()
                        .get(&account)
                        .and_then(|nav| nav.native_walk.as_ref())
                        .filter(|owner| owner.live())
                        .map(|owner| owner.request_id().get());
                    let mut recorder = frame_recorder.lock();
                    let fault = &mut recorder.return_fault;
                    if let Some(request) = request {
                        if !fault.armed && return_trigger_ready(&status, tile) {
                            fault.armed = true;
                            fault.active = true;
                            fault.initial_request = Some(request);
                        }
                        if fault.armed
                            && text(&status, "bank").ends_with("; Return")
                            && Some(request) != fault.initial_request
                            && !fault.recovery_requests.contains(&request)
                        {
                            fault.recovery_requests.push(request);
                            if fault.recovery_requests.len() > 1 {
                                fault.ceased_after_recovery = false;
                            }
                            if fault.recovery_event.is_none() {
                                fault.recovery_event = Some(serde_json::json!({
                                    "initial_native_request_id": fault.initial_request,
                                    "recovery_native_request_id": request,
                                    "status": row,
                                }));
                                fault.active = false;
                                fault.ceased_tick = tick;
                                fault.ceased_after_recovery = fault.activated
                                    && fault.writes.len() >= MIN_RETURN_INTERFERENCE_WRITES
                                    && fault.recovery_requests.len() == 1;
                            }
                        }
                    }
                }

                {
                    let mut recorder = frame_recorder.lock();
                    let in_return = return_trigger_ready(&status, tile);
                    if in_return {
                        if !recorder.saw_bank_return {
                            recorder.return_yielded_baseline = Some(yielded);
                            recorder.return_xp_baseline = xp;
                        }
                        recorder.saw_bank_return = true;
                    }
                    let recovery_ready = match mode {
                        Mode::ReturnStall => recorder.return_fault.recovery_event.is_some(),
                        Mode::RefusedDeposit => deposit_fault
                            .as_ref()
                            .and_then(ArmedDepositFault::evidence)
                            .is_some(),
                        Mode::OysterBaseline => true,
                        Mode::SaturatedStack => false,
                    };
                    if recorder.saw_bank_return
                        && recovery_ready
                        && !in_return
                        && event.to_ascii_lowercase().contains("gathering")
                        && fish_site_arrived(&status, tile)
                    {
                        recorder.bank_return_completed = true;
                        recorder.return_status = Some(row.clone());
                    }
                    if recorder.bank_return_completed
                        && !recorder.catch_after_bank_return
                        && recorder
                            .return_yielded_baseline
                            .is_some_and(|baseline| yielded > baseline)
                        && recorder
                            .return_xp_baseline
                            .zip(xp)
                            .is_some_and(|(baseline, current)| current > baseline)
                    {
                        recorder.catch_after_bank_return = true;
                        completion_status = Some(row.clone());
                    }
                }

                if mode == Mode::SaturatedStack {
                    if let Some(failure) = failure_json(&status) {
                        if !message_has_full_stack(&failure) {
                            final_failure = Some(failure);
                            result = "blocked-with-wrong-reason";
                            break;
                        }
                        let failure_key = failure.to_string();
                        if failure_key != last_failure_key {
                            last_failure_key = failure_key;
                            blocked_since = Some(now);
                            final_failure = Some(failure);
                        }
                        if let Some(since) = blocked_since {
                            blocked_for_ms = now
                                .duration_since(since)
                                .as_millis()
                                .min(u128::from(u64::MAX))
                                as u64;
                            if now.duration_since(since) >= FAILURE_RETENTION {
                                result = "blocked-for-12s";
                                break;
                            }
                        }
                    } else {
                        blocked_since = None;
                        last_failure_key.clear();
                    }
                } else if let Some(failure) = failure_json(&status) {
                    final_failure = Some(failure);
                    result = "blocked";
                    break;
                }

                if frame_recorder.lock().catch_after_bank_return {
                    result = "bank-return-and-real-catch";
                    break;
                }
                if status.phase == script::native::NativePhase::Complete {
                    result = "completed-without-real-catch";
                    break;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let elapsed_ms = cell
        .lock()
        .started
        .map(|started| Instant::now().duration_since(started).as_millis())
        .unwrap_or_default()
        .min(u128::from(u64::MAX)) as u64;
    let logs = LOG.0.lock().clone();
    let dispatches = dispatch_receipts(&logs);
    let fault_evidence = deposit_fault.as_ref().and_then(ArmedDepositFault::evidence);
    let record = frame_recorder.lock();
    let oyster_settled = oyster_sequence_settled(&dispatches, &record.frames);
    let deposit_recovered = fault_evidence
        .as_ref()
        .is_some_and(|fault| refused_deposit_recovered(fault, &dispatches, &record.frames));
    let return_fault = serde_json::json!({
        "armed": record.return_fault.armed,
        "activated": record.return_fault.activated,
        "active_writes": record.return_fault.writes.len(),
        "writes": record.return_fault.writes,
        "recovery_event_observed": record.return_fault.recovery_event.is_some(),
        "initial_native_request_id": record.return_fault.initial_request,
        "recovery_native_request_ids": record.return_fault.recovery_requests,
        "recovery_actions": record.return_fault.recovery_requests.len(),
        "recovery_dispatch": record.return_fault.recovery_event,
        "interference_ceased_after_recovery": record.return_fault.ceased_after_recovery,
        "ceased_tick": record.return_fault.ceased_tick,
    });
    let deposit_fault_json = match fault_evidence {
        Some(fault) => serde_json::json!({
            "armed": true,
            "activated": true,
            "accepted": fault.accepted,
            "emitted_packets": fault.emitted_packets,
            "request_id": fault.request_id,
            "action_id": fault.action_id,
            "fault": fault,
        }),
        None => serde_json::json!({
            "armed": deposit_fault.is_some(),
            "activated": false,
        }),
    };
    let bank_return_completed_after_recovery = mode == Mode::ReturnStall
        && record.bank_return_completed
        && record.return_fault.recovery_event.is_some();
    let receipt = serde_json::json!({
        "request": REQUEST,
        "mode": mode.name(),
        "account": account,
        "engine": "Engine A / local 289 / loopback",
        "bounds_ms": {
            "preparation": PREPARATION_BOUND.as_millis(),
            "cell": CELL_BOUND.as_millis(),
            "overall": OVERALL_BOUND.as_millis(),
            "failure_retention": FAILURE_RETENTION.as_millis(),
        },
        "cell_elapsed_ms": elapsed_ms,
        "settings": mode_settings(mode),
        "seed": mode.seed(),
        "target_verified": record.target_verified,
        "result": result,
        "failure": final_failure,
        "blocked_for_ms": blocked_for_ms,
        "return_fault": return_fault,
        "deposit_fault": deposit_fault_json,
        "deposit_recovery": {
            "same_item_retried": deposit_recovered,
            "transfer_settled": deposit_recovered,
        },
        "oyster_sequence": {
            "settled": oyster_settled,
            "baseline_behavior_unchanged": mode == Mode::OysterBaseline,
        },
        "bank_return_completed": record.bank_return_completed,
        "bank_return_completed_after_recovery": bank_return_completed_after_recovery,
        "return_status": record.return_status,
        "real_catch_after_bank_return": record.catch_after_bank_return,
        "completion_status": completion_status,
        "dispatches": dispatches,
        "statuses": record.statuses,
        "facts": record.frames,
        "hostlog_entries": logs.len(),
    });
    drop(record);

    std::fs::write(
        evidence_dir.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).expect("serialize live receipt"),
    )
    .expect("write live receipt");
    std::fs::write(evidence_dir.join("hostlog.txt"), logs.join("\n")).expect("write live hostlog");
    std::fs::write(
        evidence_dir.join("dispatches.json"),
        serde_json::to_vec_pretty(&receipt["dispatches"]).expect("serialize dispatch receipts"),
    )
    .expect("write per-dispatch receipts");

    play.script_stop(&account);
    let accepted = valid_outcome(mode, &receipt);
    println!(
        "GATHERER-RECOVERY mode={} result={} accepted={} evidence={}",
        mode.name(),
        receipt["result"].as_str().unwrap_or("unknown"),
        accepted,
        evidence_dir.display(),
    );
    assert!(
        accepted,
        "Gatherer recovery cell failed its fail-closed oracle: {receipt}"
    );
}

fn oracle_receipt(mode: Mode) -> serde_json::Value {
    let mut receipt = serde_json::json!({
        "request": REQUEST,
        "mode": mode.name(),
        "account": "gr24fixture",
        "settings": {"site": "fishing.catherby"},
        "target_verified": true,
        "cell_elapsed_ms": 120_000,
        "result": "bank-return-and-real-catch",
        "bank_return_completed": true,
        "bank_return_completed_after_recovery": true,
        "real_catch_after_bank_return": true,
        "return_status": {
            "method": "0_44_53_memberfish",
            "area": CATHERBY_AREA,
            "tile": [2846, 3429, 0],
            "last_event": "gathering",
        },
        "return_fault": {
            "armed": true,
            "activated": true,
            "active_writes": MIN_RETURN_INTERFERENCE_WRITES,
            "recovery_event_observed": true,
            "recovery_actions": 1,
            "initial_native_request_id": 1,
            "recovery_native_request_ids": [2],
            "interference_ceased_after_recovery": true,
        },
        "deposit_fault": {
            "armed": true,
            "activated": true,
            "accepted": false,
            "emitted_packets": 0,
            "request_id": 7,
            "action_id": 11,
            "fault": {
                "account": "gr24fixture",
                "run": "RunKey",
                "action_id": 11,
                "request_id": 7,
                "dispatch_tick": 90,
                "bank_generation": 1,
                "transfer": 1,
                "click_tick": 90,
                "sequence": 90,
                "item_id": SHARK,
                "slot": 2,
                "component": 2006,
                "option": "Deposit All",
                "operation": 4,
                "main_before": 27,
                "side_before": null,
                "bank_before": 0,
                "main_revision": 1,
                "side_revision": 1,
                "accepted": false,
                "emitted_packets": 0,
            },
        },
        "deposit_recovery": {
            "same_item_retried": true,
            "transfer_settled": true,
        },
        "oyster_sequence": {
            "settled": true,
            "baseline_behavior_unchanged": true,
        },
        "dispatches": [
            {"bank_generation":1,"transfer":1,"request_id":8,"click_tick":88,"tick":88,
                "item_id":MACKEREL,"slot":1,"component":2006,"operation":4,
                "option":"Deposit All","accepted":true,"emitted_packets":1},
            {"bank_generation":1,"transfer":2,"request_id":9,"click_tick":89,"tick":89,
                "item_id":COD,"slot":2,"component":2006,"operation":4,
                "option":"Deposit All","accepted":true,"emitted_packets":1},
            {"bank_generation":1,"transfer":3,"request_id":10,"click_tick":90,"tick":90,
                "item_id":OYSTER,"slot":3,"component":2006,"operation":4,
                "option":"Deposit All","accepted":true,"emitted_packets":1},
        ],
        "failure": {"code":"bank-deposit-failed","message":"bank deposit refused: full stack in bank"},
        "blocked_for_ms": 12_000,
    });
    if mode == Mode::SaturatedStack {
        receipt["result"] = serde_json::json!("blocked-for-12s");
        receipt["dispatches"] = serde_json::json!([]);
        receipt["facts"] = serde_json::json!([{
            "bank": [{"id": OYSTER, "count": i32::MAX}]
        }]);
    }
    receipt
}

#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX=gr24, Engine A and Gatherer live evidence inputs"]
fn live_gatherer_return_stall_recovers_and_catches_engine_a() {
    run_live(Mode::ReturnStall);
}

#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX=gr24, Engine A and Gatherer live evidence inputs"]
fn live_gatherer_oyster_third_transfer_returns_and_catches_engine_a() {
    run_live(Mode::OysterBaseline);
}

#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX=gr24, Engine A and Gatherer live evidence inputs"]
fn live_gatherer_refused_deposit_recovers_and_catches_engine_a() {
    run_live(Mode::RefusedDeposit);
}

#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX=gr24, Engine A and Gatherer live evidence inputs"]
fn live_gatherer_saturated_oyster_stack_reports_full_stack_engine_a() {
    run_live(Mode::SaturatedStack);
}

#[test]
fn gatherer_recovery_dispatch_parser_uses_dispatch_fields_not_sink_metadata() {
    let line = r#"slot=Some("gr24fixture") bank-deposit-dispatch run=RunKey { slot: 1, run: 1, session: 146 } bank_generation=1 transfer=1 request=8 click_tick=49 tick=50 id=383 slot=1 component=2006 option=Deposit All operation=4 accepted=true refusal=None emitted_packets=1 opcode=Some(248)"#;
    let dispatch = parse_dispatch(line).expect("the captured dispatch shape must parse");
    assert_eq!(dispatch.slot, 1);
    assert_eq!(dispatch.item_id, SHARK);
    assert_eq!(dispatch.click_tick, 49);
    assert_eq!(dispatch.tick, 50);
    assert!(dispatch.accepted);
    assert_eq!(dispatch.emitted_packets, 1);
    assert!(parse_dispatch(&line.replace(" tick=50", "")).is_none());
    assert!(parse_dispatch(&line.replace(" slot=1 component=", " component=")).is_none());
}

#[test]
fn gatherer_recovery_site_oracle_requires_the_posted_area_and_radius() {
    let mut receipt = oracle_receipt(Mode::OysterBaseline);
    assert!(receipt_has_site_arrival(&receipt));
    for wrong_area in [
        "site(2855,3423,0) r32",
        "site (2855,3423,0) r31",
        "site (2856,3423,0) r32",
    ] {
        receipt["return_status"]["area"] = serde_json::json!(wrong_area);
        assert!(!receipt_has_site_arrival(&receipt), "{wrong_area}");
    }
}

#[test]
fn gatherer_recovery_oracle_accepts_all_four_observed_outcomes() {
    for mode in [
        Mode::ReturnStall,
        Mode::OysterBaseline,
        Mode::RefusedDeposit,
        Mode::SaturatedStack,
    ] {
        let receipt = oracle_receipt(mode);
        assert!(valid_outcome(mode, &receipt), "rejects {mode:?}: {receipt}");
    }
}

#[test]
fn gatherer_recovery_oracle_rejects_deadlines_and_unactivated_fixtures() {
    for mode in [
        Mode::ReturnStall,
        Mode::OysterBaseline,
        Mode::RefusedDeposit,
        Mode::SaturatedStack,
    ] {
        let mut deadline = oracle_receipt(mode);
        deadline["result"] = serde_json::json!("cell-deadline");
        assert!(
            !valid_outcome(mode, &deadline),
            "accepted deadline for {mode:?}"
        );
    }

    let mut return_unactivated = oracle_receipt(Mode::ReturnStall);
    return_unactivated["return_fault"]["activated"] = serde_json::json!(false);
    assert!(!valid_outcome(Mode::ReturnStall, &return_unactivated));

    let mut deposit_unactivated = oracle_receipt(Mode::RefusedDeposit);
    deposit_unactivated["deposit_fault"]["activated"] = serde_json::json!(false);
    assert!(!valid_outcome(Mode::RefusedDeposit, &deposit_unactivated));
}

#[test]
fn gatherer_recovery_oracle_rejects_normal_completion_without_induced_failure() {
    let mut normal_return = oracle_receipt(Mode::ReturnStall);
    normal_return["return_fault"]["armed"] = serde_json::json!(false);
    normal_return["return_fault"]["activated"] = serde_json::json!(false);
    normal_return["return_fault"]["recovery_event_observed"] = serde_json::json!(false);
    normal_return["return_fault"]["interference_ceased_after_recovery"] = serde_json::json!(false);
    assert!(!valid_outcome(Mode::ReturnStall, &normal_return));

    let mut normal_deposit = oracle_receipt(Mode::RefusedDeposit);
    normal_deposit["deposit_fault"]["armed"] = serde_json::json!(false);
    normal_deposit["deposit_fault"]["activated"] = serde_json::json!(false);
    assert!(!valid_outcome(Mode::RefusedDeposit, &normal_deposit));
}

#[test]
fn gatherer_recovery_oracle_requires_one_action_and_site_arrival() {
    let mut duplicate_recovery = oracle_receipt(Mode::ReturnStall);
    duplicate_recovery["return_fault"]["recovery_actions"] = serde_json::json!(2);
    assert!(!valid_outcome(Mode::ReturnStall, &duplicate_recovery));

    let mut stale_request = oracle_receipt(Mode::ReturnStall);
    stale_request["return_fault"]["recovery_native_request_ids"] = serde_json::json!([1]);
    assert!(!valid_outcome(Mode::ReturnStall, &stale_request));

    let mut no_dispatch = oracle_receipt(Mode::ReturnStall);
    no_dispatch["return_fault"]["recovery_native_request_ids"] = serde_json::json!([]);
    assert!(!valid_outcome(Mode::ReturnStall, &no_dispatch));

    let mut missing_arrival = oracle_receipt(Mode::ReturnStall);
    missing_arrival["return_status"]["tile"] = serde_json::json!([2810, 3440, 0]);
    assert!(!valid_outcome(Mode::ReturnStall, &missing_arrival));
}

#[test]
fn gatherer_recovery_oracle_rejects_wrong_capacity_reason_and_inexact_oyster_sequence() {
    let mut wrong_reason = oracle_receipt(Mode::SaturatedStack);
    wrong_reason["failure"]["message"] = serde_json::json!("bank transfer did not settle");
    assert!(!valid_outcome(Mode::SaturatedStack, &wrong_reason));

    let mut wrong_third_transfer = oracle_receipt(Mode::OysterBaseline);
    wrong_third_transfer["dispatches"][2]["component"] = serde_json::json!(2007);
    assert!(!valid_outcome(Mode::OysterBaseline, &wrong_third_transfer));
    let mut missing_saturated_stack = oracle_receipt(Mode::SaturatedStack);
    missing_saturated_stack["facts"][0]["bank"][0]["count"] = serde_json::json!(i32::MAX - 1);
    assert!(!valid_outcome(
        Mode::SaturatedStack,
        &missing_saturated_stack
    ));

    let mut sent_saturated_oyster = oracle_receipt(Mode::SaturatedStack);
    sent_saturated_oyster["dispatches"] =
        serde_json::json!([{"item_id": OYSTER, "accepted": true, "emitted_packets": 1}]);
    assert!(!valid_outcome(Mode::SaturatedStack, &sent_saturated_oyster));

    let mut different_deposit_action = oracle_receipt(Mode::RefusedDeposit);
    different_deposit_action["deposit_fault"]["fault"]["action_id"] = serde_json::json!(12);
    assert!(!valid_outcome(
        Mode::RefusedDeposit,
        &different_deposit_action
    ));
}
