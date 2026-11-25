//! One table-driven conformance test for the public v2 native API.
//!
//! The member inventory comes from the host-owned declaration generator
//! (`render_host_js_dts`), parsed with the TypeScript parser: `NativeApi`, its
//! service and handle interfaces, namespace objects, handles returned by
//! calls, and every `NativeOp` variant with its declared field set. Each
//! public path has exactly one authored row. A new member, overload, op
//! variant or op field without a row fails with its exact ID before any case
//! runs; a row whose member is gone fails as stale.
//!
//! Each runnable row uses a manifest-bearing source prepared by the real
//! origin check and isolate shim. Rows that need selected game data report an
//! explicit hosted-CI skip when those local assets are absent; coverage still
//! requires exactly one authored row for every public path.
//!
//! Design: `SCRIPT-MANIFEST-DESIGN-1022/design-final2.md` section M7.1.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use deno_ast::swc::ast::{
    Decl, Expr, ModuleDecl, ModuleItem, TsEntityName, TsFnParam, TsInterfaceDecl, TsLit, TsType,
    TsTypeElement, TsUnionOrIntersectionType,
};
use deno_ast::{MediaType, ParseParams, ProgramRef};
use script::api_combat::{CombatEnd, CombatSessionRequest, TargetSpec};
use script::api_gather::{GatherEnd, GatherPage, GatherPhase};
use script::api_progress::ProgressPage;
use script::interact::InteractReq;
use script::native::request::NativeKind;
use script::native::seat::{SeatEntry, SeatResult, SeatTerminal};
use script::native::seat_rows::{SeatLiveDetails, SeatPageInput, SeatResultValue, SeatStartArgs};
use script::native::snapshot_source::{
    BankApproachInput, BankSelectionInput, BankStandInput, CollisionViewInput, ItemRowInput,
    MainModalTextsInput, NativeFactsInput, NearestBoothInput, QuestStatusInput, ReachViewInput,
    RouteInspectFactsInput, RouteInspectTerminalInput, SceneEntityInput, SnapshotInput, StatInput,
    TileInput, VarpInput,
};
use script_js::{LoadIsolate, LoadShape};
use serde_json::{json, Value};

mod common;

// ---------------------------------------------------------------------------
// Inventory from the generated declarations.
// ---------------------------------------------------------------------------

/// The public member paths and the per-op declared field sets.
#[derive(Debug, Default, PartialEq, Eq)]
struct Inventory {
    members: BTreeSet<String>,
    /// `op` -> field names; optional fields end with `?`. `op` itself is the
    /// discriminant and is not listed.
    op_fields: BTreeMap<String, BTreeSet<String>>,
}

fn generated_dts() -> String {
    script_js::tooling::host_js::render_host_js_dts()
}

fn inventory(dts: &str) -> Inventory {
    let parsed = deno_ast::parse_module(ParseParams {
        specifier: deno_ast::ModuleSpecifier::parse("file:///index.d.ts")
            .expect("static declaration URL is valid"),
        text: dts.to_owned().into(),
        media_type: MediaType::Dts,
        capture_tokens: false,
        scope_analysis: false,
        maybe_syntax: None,
    })
    .expect("generated declarations parse");
    let ProgramRef::Module(module) = parsed.program_ref() else {
        panic!("generated declarations are a module");
    };
    let mut interfaces: HashMap<String, TsInterfaceDecl> = HashMap::new();
    let mut aliases: HashMap<String, TsType> = HashMap::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) = item else {
            continue;
        };
        match &export.decl {
            Decl::TsInterface(interface) => {
                interfaces.insert(interface.id.sym.to_string(), (**interface).clone());
            }
            Decl::TsTypeAlias(alias) => {
                aliases.insert(alias.id.sym.to_string(), (*alias.type_ann).clone());
            }
            _ => {}
        }
    }
    let mut inventory = Inventory::default();
    let api = interfaces
        .get("NativeApi")
        .expect("generated declarations export NativeApi");
    walk_members(
        "NativeApi",
        &api.body.body,
        &interfaces,
        &aliases,
        &mut inventory,
        &mut Vec::new(),
    );
    inventory
}

fn member_name(key: &Expr) -> String {
    match key {
        Expr::Ident(ident) => ident.sym.to_string(),
        Expr::Lit(deno_ast::swc::ast::Lit::Str(text)) => text.value.to_string(),
        other => panic!("unexpected member key {other:?}"),
    }
}

fn type_ref_name(ty: &TsType) -> Option<String> {
    match ty {
        TsType::TsTypeRef(reference) => match &reference.type_name {
            TsEntityName::Ident(ident) => Some(ident.sym.to_string()),
            TsEntityName::TsQualifiedName(_) => None,
        },
        _ => None,
    }
}

/// A service or handle interface whose members are public paths.
fn service_interface<'a>(
    ty: &TsType,
    interfaces: &'a HashMap<String, TsInterfaceDecl>,
) -> Option<(String, &'a TsInterfaceDecl)> {
    let name = type_ref_name(ty)?;
    if !name.starts_with("Native") {
        return None;
    }
    interfaces.get(&name).map(|interface| (name, interface))
}

fn is_namespace(members: &[TsTypeElement]) -> bool {
    members
        .iter()
        .any(|member| matches!(member, TsTypeElement::TsMethodSignature(_)))
}

fn walk_members(
    owner: &str,
    members: &[TsTypeElement],
    interfaces: &HashMap<String, TsInterfaceDecl>,
    aliases: &HashMap<String, TsType>,
    inventory: &mut Inventory,
    visiting: &mut Vec<String>,
) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for member in members {
        match member {
            TsTypeElement::TsPropertySignature(property) => {
                let name = member_name(&property.key);
                let path = format!("{owner}.{name}");
                let ty = property.type_ann.as_ref().map(|ann| &*ann.type_ann);
                match ty {
                    Some(TsType::TsTypeLit(literal)) if is_namespace(&literal.members) => {
                        walk_members(
                            &path,
                            &literal.members,
                            interfaces,
                            aliases,
                            inventory,
                            visiting,
                        );
                    }
                    Some(ty) => match service_interface(ty, interfaces) {
                        Some((interface_name, interface))
                            if !visiting.contains(&interface_name) =>
                        {
                            visiting.push(interface_name);
                            walk_members(
                                &path,
                                &interface.body.body,
                                interfaces,
                                aliases,
                                inventory,
                                visiting,
                            );
                            visiting.pop();
                        }
                        _ => {
                            inventory.members.insert(path);
                        }
                    },
                    None => {
                        inventory.members.insert(path);
                    }
                }
            }
            TsTypeElement::TsMethodSignature(method) => {
                let name = member_name(&method.key);
                let count = seen.entry(name.clone()).or_default();
                *count += 1;
                let path = if *count == 1 {
                    format!("{owner}.{name}")
                } else {
                    format!("{owner}.{name}#{count}")
                };
                if owner == "NativeApi" && name == "request" {
                    expand_ops(&method.params, aliases, inventory);
                    continue;
                }
                inventory.members.insert(path.clone());
                let returns = method.type_ann.as_ref().map(|ann| &*ann.type_ann);
                if let Some((interface_name, interface)) =
                    returns.and_then(|ty| service_interface(ty, interfaces))
                {
                    if !visiting.contains(&interface_name) {
                        visiting.push(interface_name);
                        walk_members(
                            &format!("{path}()"),
                            &interface.body.body,
                            interfaces,
                            aliases,
                            inventory,
                            visiting,
                        );
                        visiting.pop();
                    }
                }
            }
            other => panic!("{owner}: unsupported declaration member {other:?}"),
        }
    }
}

fn expand_ops(params: &[TsFnParam], aliases: &HashMap<String, TsType>, inventory: &mut Inventory) {
    let [TsFnParam::Ident(op)] = params else {
        panic!("request takes one NativeOp parameter");
    };
    let alias = op
        .type_ann
        .as_ref()
        .and_then(|ann| type_ref_name(&ann.type_ann))
        .expect("request parameter names its op type");
    let Some(TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union))) =
        aliases.get(&alias)
    else {
        panic!("{alias} is a union of op variants");
    };
    for variant in &union.types {
        let TsType::TsTypeLit(literal) = &**variant else {
            panic!("{alias} variant is an object literal");
        };
        let mut op_name = None;
        let mut fields = BTreeSet::new();
        for member in &literal.members {
            let TsTypeElement::TsPropertySignature(property) = member else {
                panic!("{alias} variant member is a property");
            };
            let name = member_name(&property.key);
            if name == "op" {
                if let Some(TsType::TsLitType(lit)) =
                    property.type_ann.as_ref().map(|ann| &*ann.type_ann)
                {
                    if let TsLit::Str(text) = &lit.lit {
                        op_name = Some(text.value.to_string());
                    }
                }
                continue;
            }
            fields.insert(if property.optional {
                format!("{name}?")
            } else {
                name
            });
        }
        let op_name = op_name.expect("op variant has a string discriminant");
        inventory
            .members
            .insert(format!("NativeApi.request[op={op_name}]"));
        assert!(
            inventory
                .op_fields
                .insert(op_name.clone(), fields)
                .is_none(),
            "duplicate op variant {op_name}"
        );
    }
}

/// Compare the generated inventory with the authored rows and op field sets.
/// Every difference names its exact ID.
fn coverage_errors(
    inventory: &Inventory,
    rows: &[Row],
    op_fields: &[(&str, &[&str])],
) -> Vec<String> {
    let mut errors = Vec::new();
    let mut ids = BTreeSet::new();
    for row in rows {
        if !ids.insert(row.id) {
            errors.push(format!("duplicate row {}", row.id));
        }
    }
    for member in &inventory.members {
        if !ids.contains(member.as_str()) {
            errors.push(format!("missing row {member}"));
        }
    }
    for id in &ids {
        if !inventory.members.contains(*id) {
            errors.push(format!("stale row {id}"));
        }
    }
    let mut authored = BTreeMap::new();
    for (op, fields) in op_fields {
        let set: BTreeSet<String> = fields.iter().map(|field| (*field).to_owned()).collect();
        if authored.insert((*op).to_owned(), set).is_some() {
            errors.push(format!("duplicate op field set {op}"));
        }
    }
    for (op, fields) in &inventory.op_fields {
        let Some(expected) = authored.get(op) else {
            errors.push(format!("missing op field set NativeApi.request[op={op}]"));
            continue;
        };
        for field in fields.difference(expected) {
            errors.push(format!(
                "missing field NativeApi.request[op={op}].fields.{field}"
            ));
        }
        for field in expected.difference(fields) {
            errors.push(format!(
                "stale field NativeApi.request[op={op}].fields.{field}"
            ));
        }
    }
    for op in authored.keys() {
        if !inventory.op_fields.contains_key(op) {
            errors.push(format!("stale op field set NativeApi.request[op={op}]"));
        }
    }
    errors
}

// ---------------------------------------------------------------------------
// The harness: one real isolate per row.
// ---------------------------------------------------------------------------

/// What one case observed.
#[derive(Debug)]
struct Observed {
    /// The JSON value the case body returned, or `{ "thrown": message }`.
    value: Value,
    /// Every script-issued gameplay request in the raw interact batches
    /// (taken before host decoding). Host-owned watchdog lifecycle facts and
    /// native inspect-consumption acknowledgements are not API output.
    /// InspectAck is generated when the host consumes a posted inspect page;
    /// it is not callable from the script-facing API.
    requests: Vec<InteractReq>,
    logs: Vec<String>,
    paint: Option<script::paint::ScriptPaint>,
    stopped: bool,
    stop_reason: Option<String>,
}

/// Inputs posted before the tick that runs a case body.
#[derive(Clone, Copy)]
struct Setup {
    snapshot: fn(u64, &mut SnapshotInput<'static>),
    facts: fn(u64) -> NativeFactsInput<'static>,
    native_facts: bool,
    settings: fn() -> Value,
    manifest_settings: &'static str,
    game_data: fn() -> Option<Arc<api::game_data::SelectedGameData>>,
    requires_selected_game_data: bool,
}

const PLAIN: Setup = Setup {
    snapshot: |_, _| {},
    facts: |_| NativeFactsInput::default(),
    native_facts: false,
    settings: || json!({}),
    manifest_settings: "{}",
    game_data: || None,
    requires_selected_game_data: false,
};

fn revision_data() -> Option<Arc<api::game_data::SelectedGameData>> {
    Some(
        api::game_data::for_revision(client::io::ClientRevision::R289)
            .expect("R289 selected game data"),
    )
}

const CONTENT: Setup = Setup {
    game_data: revision_data,
    requires_selected_game_data: true,
    ..PLAIN
};
fn hunt_snapshot(_: u64, input: &mut SnapshotInput<'static>) {
    input.here = None;
}

const HUNT_SETUP: Setup = Setup {
    snapshot: hunt_snapshot,
    ..PLAIN
};

/// Run a body in one real manifest-form isolate. Async bodies can be advanced
/// by posting additional ticks; setup facts may select values by tick.
fn observe(setup: &Setup, body: &str) -> Observed {
    observe_ticks(setup, body, 1, false)
}

fn observe_async(setup: &Setup, body: &str, ticks: u64) -> Observed {
    observe_ticks(setup, body, ticks, true)
}
fn wait_ready(iso: LoadIsolate, context: &str) -> LoadIsolate {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        match iso.poll_ready() {
            script_js::Ready::Ready => return iso,
            script_js::Ready::Failed(error) => panic!("{context}: {error}"),
            script_js::Ready::Pending if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            script_js::Ready::Pending => panic!("{context} did not settle"),
        }
    }
}

fn interact_requests(iso: &LoadIsolate) -> Vec<InteractReq> {
    iso.take_interact_wire_for_test()
        .iter()
        .flat_map(|batch| {
            script_js::wire::decode_interact_batch(batch).expect("interact batch verifies")
        })
        .filter(|request| !request.is_watchdog_lifecycle())
        .filter(|request| !matches!(request, InteractReq::InspectAck { .. }))
        .collect()
}

fn post_native_tick(
    iso: &LoadIsolate,
    tick: u64,
    mut input: SnapshotInput<'static>,
    facts: &NativeFactsInput<'_>,
) {
    input.tick = tick;
    assert!(
        iso.post_snapshot(script_js::wire::encode_snapshot_with_native(&input, *facts)),
        "native snapshot tick {tick} was refused"
    );
    assert!(
        iso.probe("true").is_ok(),
        "snapshot for tick {tick} must be applied before dispatch"
    );
    iso.on_game_tick(tick);
    assert!(
        iso.probe("true").is_ok(),
        "native tick {tick} must complete before observations"
    );
}

fn observe_ticks(setup: &Setup, body: &str, ticks: u64, asynchronous: bool) -> Observed {
    let call = if asynchronous {
        format!(
            "export async function tick(api) {{\n\
             if (globalThis.__ran) return;\n\
             globalThis.__ran = true;\n\
             try {{ const value = await (async () => {{\n{body}\n}})();\n\
               globalThis.__probe = JSON.stringify(value === undefined ? null : value); }}\n\
             catch (e) {{ globalThis.__probe = JSON.stringify({{ thrown: String((e && e.message) || e) }}); }}\n\
             }}\n"
        )
    } else {
        format!(
            "export function tick(api) {{\n\
             if (globalThis.__ran) return;\n\
             globalThis.__ran = true;\n\
             try {{ const value = (() => {{\n{body}\n}})();\n\
               globalThis.__probe = JSON.stringify(value === undefined ? null : value); }}\n\
             catch (e) {{ globalThis.__probe = JSON.stringify({{ thrown: String((e && e.message) || e) }}); }}\n\
             }}\n"
        )
    };
    let source = script_js::NativeFixture::new("conformance")
        .settings(setup.manifest_settings)
        .source(&call);
    let game_data = (setup.game_data)();
    let iso = if let Some(data) = game_data {
        LoadIsolate::spawn_with_content(
            source,
            LoadShape::NativeTick,
            Vec::new(),
            Some(data),
            Arc::new(api::named_banks::NamedBankFacts::empty()),
        )
        .expect("spawn conformance isolate with content")
    } else {
        LoadIsolate::spawn(source, LoadShape::NativeTick, Vec::new())
            .expect("spawn conformance isolate")
    };
    let iso = wait_ready(iso, "conformance isolate setup");
    if let Value::Object(bag) = (setup.settings)() {
        iso.post_settings_bag(&bag);
    }
    let mut tick_barrier_error = None;
    for tick in 1..=ticks {
        let mut input = common::ingame_snapshot();
        input.tick = tick;
        (setup.snapshot)(tick, &mut input);
        if setup.native_facts {
            let accepted = iso.post_snapshot(script_js::wire::encode_snapshot_with_native(
                &input,
                (setup.facts)(tick),
            ));
            assert!(accepted, "native snapshot tick {tick} was refused");
        } else {
            common::post_snapshot_input(&iso, &input);
        }
        assert!(
            iso.probe("true").is_ok(),
            "snapshot for tick {tick} must be applied before dispatch"
        );
        iso.on_game_tick(tick);
        if let Err(error) = iso.probe("true") {
            tick_barrier_error = Some(error.to_string());
        }
    }
    let stop_receipt = iso.script_stop_receipt();
    let stopped = iso.stopped();
    let value = if stopped {
        assert!(
            stop_receipt.is_some(),
            "isolate stopped without a host stop receipt"
        );
        Value::Null
    } else {
        let raw = iso
            .probe("globalThis.__probe")
            .expect("read conformance probe");
        match raw.as_str() {
            Some(text) => serde_json::from_str(text).expect("conformance probe is JSON"),
            None => {
                let ran = iso.probe("globalThis.__ran").unwrap_or(Value::Null);
                let logs = iso.drain_logs();
                let lifecycle = iso.drain_lifecycle();
                panic!(
                    "case body did not finish in its tick(s): {raw:?}; ran={ran:?}; tick barrier error={tick_barrier_error:?}; logs={logs:?}; lifecycle={lifecycle:?}"
                );
            }
        }
    };
    let requests = interact_requests(&iso);
    let logs = iso.drain_logs();
    let paint = iso.paint().map(|paint| (*paint).clone());
    iso.join();
    Observed {
        value,
        requests,
        logs,
        paint,
        stopped,
        stop_reason: stop_receipt.map(|receipt| receipt.reason),
    }
}

// ---------------------------------------------------------------------------
// Rows.
// ---------------------------------------------------------------------------

/// The observable kind of one public path.
enum Case {
    /// A synchronous read: `body` returns this JSON and sends no request.
    Read {
        setup: Setup,
        body: &'static str,
        expect: Value,
    },
    /// `body` sends exactly these requests, in order, in one tick.
    Send {
        setup: Setup,
        body: &'static str,
        expect: Vec<InteractReq>,
    },
    /// A driver whose observations require selected local game data.
    GameDataDrive(fn()),
    /// A member whose observable outcome needs its own driver (paint, logs,
    /// stop, promise settlement, posted completions).
    Drive(fn()),
}

struct Row {
    /// The public path, for example `NativeApi.request[op=walk]`.
    id: &'static str,
    /// Why this case exists and what it proves.
    why: &'static str,
    case: Case,
}

fn run_row(row: &Row) {
    match &row.case {
        Case::Read {
            setup,
            body,
            expect,
        } => {
            let observed = observe(setup, body);
            assert_eq!(observed.value, *expect, "{}: {}", row.id, row.why);
            assert!(
                observed.requests.is_empty(),
                "{}: a read sent {:?}",
                row.id,
                observed.requests
            );
        }
        Case::Send {
            setup,
            body,
            expect,
        } => {
            let observed = observe(setup, body);
            assert_eq!(
                observed.value,
                Value::Null,
                "{}: the request body threw",
                row.id
            );
            assert_eq!(observed.requests, *expect, "{}: {}", row.id, row.why);
        }
        Case::Drive(drive) => drive(),
        Case::GameDataDrive(drive) => drive(),
    }
}

fn row_requires_selected_game_data(row: &Row) -> bool {
    match &row.case {
        Case::Read { setup, .. } | Case::Send { setup, .. } => setup.requires_selected_game_data,
        Case::Drive(_) => false,
        Case::GameDataDrive(_) => true,
    }
}

const SETTINGS_SETUP: Setup = Setup {
    snapshot: |_, _| {},
    settings: || json!({ "name": "Bones", "count": 7.5, "on": true }),
    manifest_settings: "{ name: { type: 'string', default: 'Ashes' }, \
                        count: { type: 'number', default: 2 }, \
                        on: { type: 'boolean', default: false } }",
    ..PLAIN
};

fn read_row(
    id: &'static str,
    why: &'static str,
    setup: Setup,
    body: &'static str,
    expect: Value,
) -> Row {
    Row {
        id,
        why,
        case: Case::Read {
            setup,
            body,
            expect,
        },
    }
}

fn send_row(
    id: &'static str,
    why: &'static str,
    body: &'static str,
    expect: Vec<InteractReq>,
) -> Row {
    Row {
        id,
        why,
        case: Case::Send {
            setup: PLAIN,
            body,
            expect,
        },
    }
}

const SNAPSHOT_TILE: TileInput = TileInput {
    x: 3179,
    z: 3118,
    level: 2,
};

const SNAPSHOT_ITEMS: [ItemRowInput<'static>; 1] = [ItemRowInput {
    name: Some("Rune platebody"),
    count: 3,
    id: 1123,
    ops: &[],
    noted: true,
    cert: 1201,
    component_id: 583,
    slot: 5,
}];

const SNAPSHOT_STATS: [StatInput<'static>; 1] = [StatInput {
    index: 2,
    name: "attack",
    xp: 12_345,
    base: 41,
    effective: 43,
}];

const SNAPSHOT_NPCS: [SceneEntityInput<'static>; 1] = [SceneEntityInput {
    index: 17,
    id: 90,
    name: Some("Conformance guard"),
    x: 3180,
    z: 3117,
    level: 2,
    distance: 4,
    health: 7,
    max_health: 12,
    in_combat: true,
    animating: false,
    actions: &[],
    reachable: true,
    reachable_adj: false,
    combat_level: 29,
    target_kind: 2,
    target_index: 18,
    size: 1,
    nx: 3180,
    nz: 3117,
    shape: 0,
    angle: 0,
}];

const SNAPSHOT_BANKS: [BankStandInput<'static>; 1] = [BankStandInput {
    name: "Conformance teller",
    x: 3181,
    z: 3119,
    level: 2,
    kind: "npc",
    op: 2,
    choose: Some("Bank"),
}];

const SNAPSHOT_BOOTH: NearestBoothInput<'static> = NearestBoothInput {
    x: 3182,
    z: 3119,
    level: 2,
    id: 2213,
    name: "Bank booth",
    op: "Use-quickly",
};

const SNAPSHOT_BANK_APPROACHES: [BankApproachInput; 1] = [BankApproachInput {
    loc_id: 2213,
    x: 3182,
    z: 3119,
    level: 2,
    can_operate: true,
    dest_ok: true,
    dest_x: 3181,
    dest_z: 3119,
    dest_level: 2,
}];

const SNAPSHOT_HOPS: [script::native::snapshot_source::InspectHopInput<'static>; 1] =
    [script::native::snapshot_source::InspectHopInput {
        kind: "boat",
        loc_id: 445,
        loc_name: "Conformance ferry",
        action: "Pay-fare",
        option: 3,
        from_x: 3179,
        from_z: 3118,
        from_level: 2,
        to_x: 3189,
        to_z: 3128,
        to_level: 1,
        ticks: 4,
    }];

static GATHER_PAGE: GatherPage = GatherPage {
    token: 319,
    phase: GatherPhase::Running,
    status: None,
};

const SNAPSHOT_TILES: [TileInput; 1] = [SNAPSHOT_TILE];
fn snapshot_setup(_: u64, input: &mut SnapshotInput<'static>) {
    input.here = Some(SNAPSHOT_TILE);
    input.ingame = true;
    input.inv = &SNAPSHOT_ITEMS;
    input.inv_size = 19;
    input.stats = &SNAPSHOT_STATS;
    input.booths = &SNAPSHOT_TILES;
    input.nearest_booth = Some(SNAPSHOT_BOOTH);
    input.banks = &SNAPSHOT_BANKS;
    input.bank = &SNAPSHOT_ITEMS;
    input.bank_side = &SNAPSHOT_ITEMS;
    input.bank_open = true;
    input.bank_loaded = true;
    input.bank_generation = 123;
    input.count_dialog_open = true;
    input.withdraw_x_result_seq = 409;
    input.withdraw_x_result = true;
    input.bank_op_result_seq = 84;
    input.bank_op_result = true;
    input.hold = false;
    input.ours = true;
    input.npcs = &SNAPSHOT_NPCS;
    input.self_target_kind = 2;
    input.self_target_index = 18;
    input.user_move_intent_seq = 301;
    input.walk_outcome_cancel_reason = script::native::WalkCancelReason::UserInput;
}

fn snapshot_facts(_: u64) -> NativeFactsInput<'static> {
    NativeFactsInput {
        bank_approaches: Some(&SNAPSHOT_BANK_APPROACHES),
        puzzle_board: Some(script::native::snapshot_source::PuzzleBoardInput {
            component_id: 583,
            size: 4,
            items: &SNAPSHOT_ITEMS,
            generation: 88,
        }),
        route_inspect: RouteInspectFactsInput {
            latest: RouteInspectTerminalInput {
                seq: 49,
                generation: 12,
                request_id: 13,
                ok: true,
                reason: Some("reached"),
                bank_planned: true,
                ticks: 4.5,
                terminal: Some(TileInput {
                    x: 3189,
                    z: 3128,
                    level: 1,
                }),
                hops: &SNAPSHOT_HOPS,
            },
            prev: RouteInspectTerminalInput {
                seq: 51,
                generation: 14,
                request_id: 15,
                ok: false,
                reason: Some("no-path"),
                bank_planned: false,
                ticks: 6.5,
                terminal: None,
                hops: &SNAPSHOT_HOPS,
            },
            running_id: 61,
            pending_id: 62,
            accepted_id: 63,
            replaced_id: 64,
            replaced_prev_id: 65,
            refused_id: 66,
            refused_id_2: 67,
            refused_id_3: 68,
            unobserved: 69,
        },
        walk_outcome_seq: 31,
        walk_outcome_generation: 32,
        walk_outcome_failed: true,
        walk_outcome_x: 3184,
        walk_outcome_z: 3194,
        walk_outcome_level: 1,
        walk_outcome_radius: 7,
        walk_outcome_allow_teleports: true,
        walk_outcome_request_id: 33,
        collision: Some(CollisionViewInput {
            available: true,
            base_x: 3170,
            base_z: 3100,
            level: 2,
            width: 1,
            height: 2,
            flags: &[0x1234, 0x5678],
        }),
        bank_selection: BankSelectionInput {
            request_id: 111,
            generation: 112,
            bank_index: -1,
            kind: 2,
        },
        api_gather: Some(&GATHER_PAGE),
        api_combat: Some(&GATHER_PAGE),
        bank_snapshot_generation: Some(303),
        ..NativeFactsInput::default()
    }
}

const SNAPSHOT_SETUP: Setup = Setup {
    snapshot: snapshot_setup,
    facts: snapshot_facts,
    native_facts: true,
    ..PLAIN
};

const MOVEMENT_WALKABLE: [u32; 3] = [1, 0, 0];
const MOVEMENT_REACHABLE: [u32; 3] = [0, 1, 0];
const MOVEMENT_ADJACENT: [u32; 3] = [0, 33, 0];
const MOVEMENT_STEP: [u8; 72] = [2; 72];

const fn movement_ranks(exact: bool) -> [u16; 72] {
    let mut ranks = [u16::MAX; 72];
    if exact {
        ranks[0] = 0;
        ranks[32] = 6;
    } else {
        ranks[0] = 0;
        ranks[1] = 0;
        ranks[32] = 5;
    }
    ranks
}

const MOVEMENT_EXACT_RANKS: [u16; 72] = movement_ranks(true);
const MOVEMENT_ADJACENT_RANKS: [u16; 72] = movement_ranks(false);

fn movement_snapshot(_: u64, input: &mut SnapshotInput<'static>) {
    input.reach = ReachViewInput {
        available: true,
        base_x: 3200,
        base_z: 3200,
        level: 0,
        width: 9,
        height: 8,
        walkable: &MOVEMENT_WALKABLE,
        reachable: &MOVEMENT_REACHABLE,
        reachable_adj: &MOVEMENT_ADJACENT,
        exact_rank: &MOVEMENT_EXACT_RANKS,
        adjacent_rank: &MOVEMENT_ADJACENT_RANKS,
        step: &MOVEMENT_STEP,
        canlight: &[],
        stamp: 11,
    };
}

const MOVEMENT_SETUP: Setup = Setup {
    snapshot: movement_snapshot,
    ..PLAIN
};

const LOS_FLAGS: [i32; 256] = [0; 256];

fn line_of_sight_facts(_: u64) -> NativeFactsInput<'static> {
    NativeFactsInput {
        collision: Some(CollisionViewInput {
            available: true,
            base_x: 3200,
            base_z: 3200,
            level: 0,
            width: 16,
            height: 16,
            flags: &LOS_FLAGS,
        }),
        ..NativeFactsInput::default()
    }
}

const LINE_OF_SIGHT_SETUP: Setup = Setup {
    facts: line_of_sight_facts,
    native_facts: true,
    ..PLAIN
};

const PRAYER_STATS: [StatInput<'static>; 1] = [StatInput {
    index: 5,
    name: "prayer",
    xp: 0,
    base: 43,
    effective: 43,
}];
const PRAYER_VARPS: [VarpInput; 1] = [VarpInput {
    index: 97,
    value: 0,
}];

fn prayer_snapshot(_: u64, input: &mut SnapshotInput<'static>) {
    input.stats = &PRAYER_STATS;
    input.varps = &PRAYER_VARPS;
}

const PRAYER_SETUP: Setup = Setup {
    snapshot: prayer_snapshot,
    game_data: revision_data,
    requires_selected_game_data: true,
    ..PLAIN
};
const PRAYER_STATS_SETUP: Setup = Setup {
    snapshot: prayer_snapshot,
    ..PLAIN
};
const PRAYER_VARPS_SET_ON: [VarpInput; 1] = [VarpInput {
    index: 97,
    value: 1,
}];
const PRAYER_VARPS_BOTH_ON: [VarpInput; 2] = [
    VarpInput {
        index: 96,
        value: 1,
    },
    VarpInput {
        index: 97,
        value: 1,
    },
];
const PRAYER_VARPS_MELEE_ONLY: [VarpInput; 2] = [
    VarpInput {
        index: 96,
        value: 0,
    },
    VarpInput {
        index: 97,
        value: 1,
    },
];
const PRAYER_VARPS_BOTH_OFF: [VarpInput; 2] = [
    VarpInput {
        index: 96,
        value: 0,
    },
    VarpInput {
        index: 97,
        value: 0,
    },
];

fn prayer_set_snapshot(tick: u64, input: &mut SnapshotInput<'static>) {
    input.stats = &PRAYER_STATS;
    input.varps = if tick == 1 {
        &PRAYER_VARPS
    } else {
        &PRAYER_VARPS_SET_ON
    };
}

const PRAYER_SET_SETUP: Setup = Setup {
    snapshot: prayer_set_snapshot,
    game_data: revision_data,
    requires_selected_game_data: true,
    ..PLAIN
};

fn prayer_clear_snapshot(tick: u64, input: &mut SnapshotInput<'static>) {
    input.stats = &PRAYER_STATS;
    input.varps = match tick {
        1 => &PRAYER_VARPS_BOTH_ON,
        2 => &PRAYER_VARPS_MELEE_ONLY,
        _ => &PRAYER_VARPS_BOTH_OFF,
    };
}

const PRAYER_CLEAR_SETUP: Setup = Setup {
    snapshot: prayer_clear_snapshot,
    game_data: revision_data,
    requires_selected_game_data: true,
    ..PLAIN
};

const CLUE_CASKET: [ItemRowInput<'static>; 1] = [ItemRowInput {
    name: Some("Casket"),
    count: 1,
    id: 3531,
    ops: &[],
    noted: false,
    cert: -1,
    component_id: -1,
    slot: 3,
}];

fn clue_snapshot(_: u64, input: &mut SnapshotInput<'static>) {
    input.inv = &CLUE_CASKET;
}

fn clue_facts(_: u64) -> NativeFactsInput<'static> {
    NativeFactsInput {
        inv_ready: true,
        ..NativeFactsInput::default()
    }
}

const CLUE_SETUP: Setup = Setup {
    snapshot: clue_snapshot,
    facts: clue_facts,
    native_facts: true,
    game_data: revision_data,
    requires_selected_game_data: true,
    ..PLAIN
};

const CLUE_EMPTY_SETUP: Setup = Setup {
    facts: clue_facts,
    native_facts: true,
    game_data: revision_data,
    requires_selected_game_data: true,
    ..PLAIN
};
const JOURNAL_ROWS: [QuestStatusInput<'static>; 1] = [QuestStatusInput {
    name: "Cook's Assistant",
    status: "notStarted",
    component_id: Some(1234),
}];

fn journal_snapshot(tick: u64, input: &mut SnapshotInput<'static>) {
    input.main_modal_id = if (2..=3).contains(&tick) { 77 } else { -1 };
}

static JOURNAL_OPEN_TEXTS: std::sync::LazyLock<[String; 1]> =
    std::sync::LazyLock::new(|| ["@dre@The Cook's Quest".to_owned()]);

fn journal_facts(tick: u64) -> NativeFactsInput<'static> {
    let (root, texts): (i32, &[String]) = if (2..=3).contains(&tick) {
        (77, &JOURNAL_OPEN_TEXTS[..])
    } else {
        (-1, &[])
    };
    NativeFactsInput {
        quest_statuses: Some(&JOURNAL_ROWS),
        main_modal_texts: Some(MainModalTextsInput { root, texts }),
        ..NativeFactsInput::default()
    }
}

const JOURNAL_SETUP: Setup = Setup {
    snapshot: journal_snapshot,
    facts: journal_facts,
    native_facts: true,
    ..PLAIN
};

const GATHER_RUNNING_PAGE: GatherPage = GatherPage {
    token: 1,
    phase: GatherPhase::Running,
    status: None,
};
const GATHER_CLEAR_PAGE: GatherPage = GatherPage {
    token: 0,
    phase: GatherPhase::Running,
    status: None,
};
static GATHER_TERMINAL: std::sync::LazyLock<GatherEnd> =
    std::sync::LazyLock::new(|| GatherEnd::Refused {
        token: 1,
        reason: Arc::from("conformance-refused"),
    });

fn gather_facts(tick: u64) -> NativeFactsInput<'static> {
    NativeFactsInput {
        api_gather: Some(if tick == 1 {
            &GATHER_RUNNING_PAGE
        } else {
            &GATHER_CLEAR_PAGE
        }),
        api_gather_outcome: (tick >= 2).then_some(&*GATHER_TERMINAL),
        ..NativeFactsInput::default()
    }
}

const GATHER_SETUP: Setup = Setup {
    facts: gather_facts,
    native_facts: true,
    ..PLAIN
};

const COMBAT_RUNNING_PAGE: GatherPage = GatherPage {
    token: 1,
    phase: GatherPhase::Running,
    status: None,
};
const COMBAT_CLEAR_PAGE: GatherPage = GatherPage {
    token: 0,
    phase: GatherPhase::Running,
    status: None,
};
static COMBAT_TERMINAL: CombatEnd = CombatEnd::Stopped { token: 1 };

fn combat_facts(tick: u64) -> NativeFactsInput<'static> {
    NativeFactsInput {
        api_combat: Some(if tick == 1 {
            &COMBAT_RUNNING_PAGE
        } else {
            &COMBAT_CLEAR_PAGE
        }),
        api_combat_outcome: (tick >= 2).then_some(&COMBAT_TERMINAL),
        ..NativeFactsInput::default()
    }
}

const COMBAT_SETUP: Setup = Setup {
    facts: combat_facts,
    native_facts: true,
    ..PLAIN
};

static PROGRESS_TERMINAL: std::sync::LazyLock<ProgressPage> =
    std::sync::LazyLock::new(|| ProgressPage::Refused {
        token: 1,
        reason: Arc::from("conformance-refused"),
    });

fn progress_facts(tick: u64) -> NativeFactsInput<'static> {
    NativeFactsInput {
        api_progress: (tick >= 2).then_some(&*PROGRESS_TERMINAL),
        ..NativeFactsInput::default()
    }
}

const PROGRESS_SETUP: Setup = Setup {
    facts: progress_facts,
    native_facts: true,
    ..PLAIN
};

fn drive_stop() {
    let observed = observe(&PLAIN, "api.stop('conformance-stop'); return 'stopped';");
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
    assert!(observed.stopped, "stop was not received");
    assert_eq!(observed.stop_reason.as_deref(), Some("conformance-stop"));
}

fn assert_paint_frame(observed: Observed, value: Value, title: Option<&str>, lines: &[&str]) {
    assert_eq!(observed.value, value);
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
    let paint = observed.paint.expect("paint frame reaches the host");
    assert_eq!(paint.title.as_deref(), title);
    assert_eq!(paint.accent.as_deref(), Some("#123456"));
    assert_eq!(paint.lines, lines);
}

fn drive_paint_begin() {
    let observed = observe(
        &PLAIN,
        "const frame=api.paint.begin({accent:'#123456'}); const valid=['title','row','gap','end'].every(key=>typeof frame[key]==='function'); frame.title('Begin frame'); frame.end(); return valid;",
    );
    assert_paint_frame(observed, json!(true), Some("Begin frame"), &[]);
}

fn drive_paint_title() {
    let observed = observe(
        &PLAIN,
        "const frame=api.paint.begin({accent:'#123456'}); const same=frame.title('Conformance')===frame; frame.end(); return same;",
    );
    assert_paint_frame(observed, json!(true), Some("Conformance"), &[]);
}

fn drive_paint_row() {
    let observed = observe(
        &PLAIN,
        "const frame=api.paint.begin({accent:'#123456'}); const same=frame.row('phase','ready')===frame; frame.end(); return same;",
    );
    assert_paint_frame(observed, json!(true), None, &["phase | ready"]);
}

fn drive_paint_gap() {
    let observed = observe(
        &PLAIN,
        "const frame=api.paint.begin({accent:'#123456'}); const same=frame.gap()===frame; frame.end(); return same;",
    );
    assert_paint_frame(observed, json!(true), None, &[""]);
}

fn drive_paint_end() {
    let observed = observe(
        &PLAIN,
        "const frame=api.paint.begin({accent:'#123456'}); frame.row('end','sentinel'); const result=frame.end(); return result===undefined;",
    );
    assert_paint_frame(observed, json!(true), None, &["end | sentinel"]);
}

fn drive_log() {
    let observed = observe(&PLAIN, "api.log('conformance-log 7');");
    assert!(
        observed
            .logs
            .iter()
            .any(|line| line.contains("conformance-log 7")),
        "{:?}",
        observed.logs
    );
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}
fn settings_isolate(name: &str, settings: &str, body: &str, bag: Value) -> LoadIsolate {
    let source = script_js::NativeFixture::new(name)
        .settings(settings)
        .source(body);
    let iso = LoadIsolate::spawn(source, LoadShape::NativeTick, Vec::new())
        .expect("spawn NativeSettings conformance isolate");
    let iso = wait_ready(iso, "NativeSettings conformance isolate");
    iso.post_settings_bag(
        bag.as_object()
            .expect("NativeSettings fixture is an object"),
    );
    assert!(
        iso.probe("true").is_ok(),
        "NativeSettings bag must be installed before the first tick"
    );
    iso
}

fn settings_output(iso: &LoadIsolate) -> Value {
    let raw = iso
        .probe("globalThis.__probe")
        .expect("read NativeSettings conformance output");
    serde_json::from_str(
        raw.as_str()
            .expect("NativeSettings output is a JSON string"),
    )
    .expect("NativeSettings output parses as JSON")
}

fn drive_settings_list() {
    let body = r#"
export function tick(api) {
  if (api.tick === 1) {
    const names = api.settings.list('names');
    globalThis.__oldNames = names;
    names.push('caller');
    const fallback = ['fallback'];
    const fallbackValue = api.settings.list('missing', fallback);
    fallbackValue.push('caller');
    globalThis.__fallbackSource = fallback;
    globalThis.__beforeReplacement = {
      legacy: api.settings.list('legacyNames'),
      afterMutation: api.settings.list('names'),
      wrongFallback: api.settings.list('wrongList', ['wrong-fallback']),
      wrongDefault: api.settings.list('wrongList'),
      missingFallback: api.settings.list('missing', ['missing-fallback']),
      missingDefault: api.settings.list('missing'),
    };
  } else if (api.tick === 2) {
    globalThis.__probe = JSON.stringify({
      oldNames: globalThis.__oldNames,
      replacement: api.settings.list('names'),
      beforeReplacement: globalThis.__beforeReplacement,
      fallbackSource: globalThis.__fallbackSource,
    });
  }
}
"#;
    let legacy = script::coerce_setting_value("list", &json!("legacy-a, legacy-b"));
    let mut initial = serde_json::Map::new();
    initial.insert("names".into(), json!(["alice", "bob"]));
    initial.insert("legacyNames".into(), legacy);
    initial.insert("wrongList".into(), json!(["valid", 7]));
    let iso = settings_isolate(
        "settings-list",
        "{ names: { type: 'string[]', default: [] }, legacyNames: { type: 'string[]', default: [] }, wrongList: { type: 'string[]', default: [] } }",
        body,
        Value::Object(initial),
    );
    post_native_tick(
        &iso,
        1,
        common::ingame_snapshot(),
        &NativeFactsInput::default(),
    );

    let mut replacement = serde_json::Map::new();
    replacement.insert("names".into(), json!(["carol"]));
    iso.post_settings_bag(&replacement);
    assert!(iso.probe("true").is_ok(), "replacement settings must apply");
    post_native_tick(
        &iso,
        2,
        common::ingame_snapshot(),
        &NativeFactsInput::default(),
    );

    assert_eq!(
        settings_output(&iso),
        json!({
            "oldNames": ["alice", "bob", "caller"],
            "replacement": ["carol"],
            "beforeReplacement": {
                "legacy": ["legacy-a", "legacy-b"],
                "afterMutation": ["alice", "bob"],
                "wrongFallback": ["wrong-fallback"],
                "wrongDefault": [],
                "missingFallback": ["missing-fallback"],
                "missingDefault": []
            },
            "fallbackSource": ["fallback"]
        })
    );
    assert!(interact_requests(&iso).is_empty());
    assert!(iso.drain_logs().is_empty());
    iso.join();
}

fn drive_settings_tile() {
    let body = r#"
export function tick(api) {
  if (api.tick === 1) {
    const destination = api.settings.tile('destination');
    globalThis.__oldDestination = destination;
    destination.x = -1;
    const fallback = { x: 8, z: 9, level: 2 };
    const fallbackValue = api.settings.tile('missing', fallback);
    fallbackValue.z = 99;
    globalThis.__fallbackSource = fallback;
    globalThis.__beforeReplacement = {
      afterMutation: api.settings.tile('destination'),
      wrongFallback: api.settings.tile('wrongTile', { x: 1, z: 2, level: 0 }),
      wrongDefault: api.settings.tile('wrongTile'),
      missingFallback: api.settings.tile('missing', { x: 3, z: 4, level: 1 }),
      missingDefault: api.settings.tile('missing'),
    };
  } else if (api.tick === 2) {
    globalThis.__probe = JSON.stringify({
      oldDestination: globalThis.__oldDestination,
      replacement: api.settings.tile('destination'),
      beforeReplacement: globalThis.__beforeReplacement,
      fallbackSource: globalThis.__fallbackSource,
    });
  }
}
"#;
    let mut initial = serde_json::Map::new();
    initial.insert(
        "destination".into(),
        json!({ "x": 3200, "z": 3201, "level": 1 }),
    );
    initial.insert(
        "wrongTile".into(),
        json!({ "x": "bad", "z": 2, "level": 0 }),
    );
    let iso = settings_isolate(
        "settings-tile",
        "{ destination: { type: 'tile', default: { x: 1, z: 2, level: 0 } }, wrongTile: { type: 'tile', default: { x: 1, z: 2, level: 0 } } }",
        body,
        Value::Object(initial),
    );
    post_native_tick(
        &iso,
        1,
        common::ingame_snapshot(),
        &NativeFactsInput::default(),
    );

    let mut replacement = serde_json::Map::new();
    replacement.insert(
        "destination".into(),
        json!({ "x": 3300, "z": 3301, "level": 2 }),
    );
    iso.post_settings_bag(&replacement);
    assert!(iso.probe("true").is_ok(), "replacement settings must apply");
    post_native_tick(
        &iso,
        2,
        common::ingame_snapshot(),
        &NativeFactsInput::default(),
    );

    assert_eq!(
        settings_output(&iso),
        json!({
            "oldDestination": { "x": -1, "z": 3201, "level": 1 },
            "replacement": { "x": 3300, "z": 3301, "level": 2 },
            "beforeReplacement": {
                "afterMutation": { "x": 3200, "z": 3201, "level": 1 },
                "wrongFallback": { "x": 1, "z": 2, "level": 0 },
                "wrongDefault": null,
                "missingFallback": { "x": 3, "z": 4, "level": 1 },
                "missingDefault": null
            },
            "fallbackSource": { "x": 8, "z": 9, "level": 2 }
        })
    );
    assert!(interact_requests(&iso).is_empty());
    assert!(iso.drain_logs().is_empty());
    iso.join();
}
fn drive_bank_nearest_reachable() {
    use script::bank::request::BankSeatOperation;

    let source = script_js::native_fixture(
        r#"
import { bankRoutine } from '../../api/combat/hunting/combat.js';
const site = { boxes: [{ minX: 3220, maxX: 3224, minZ: 3220, maxZ: 3224, level: 0 }] };
let started = false;
globalThis.__bankValues = null;
globalThis.__rootDone = false;
export function tick(api) {
    if (started) return;
    started = true;
    const root = bankRoutine(globalThis.__rs2b0t_host, site, {
        leave: async () => {
            const result = await api.bankNearestReachable({
                from: { x: 3222, z: 3222, level: 0 },
                allow_wilderness: true,
                use_mage_bank: false,
                use_zanaris_bank: true,
            });
            globalThis.__bankValues = result;
            return result.ok;
        },
    });
    root.then(() => { globalThis.__rootDone = true; });
}
"#,
    );
    let execution = api::identity::ExecutionId::first(api::selected::RunKey {
        slot: 94,
        run: 1,
        session: 1,
    });
    let iso =
        LoadIsolate::spawn_with_execution(source, LoadShape::NativeTick, Vec::new(), execution)
            .expect("spawn execution-owned bank isolate");
    let iso = wait_ready(iso, "bank isolate setup");
    post_native_tick(
        &iso,
        1,
        common::ingame_snapshot(),
        &NativeFactsInput::default(),
    );
    let batches = iso.drain_seat_batches();
    let bank = batches
        .iter()
        .flat_map(|batch| batch.batch.starts.iter())
        .find(|start| start.kind == NativeKind::BankPlan)
        .expect("bankNearestReachable starts a typed bank Select seat");
    assert_eq!(bank.parent, None);
    let bank_id = bank.id;
    assert!(bank_id.get() > 0);
    let SeatStartArgs::Bank(args) = &bank.args else {
        panic!("bank start carries typed bank arguments: {:?}", bank.args);
    };
    assert_eq!(args.operation(), BankSeatOperation::Select);
    assert_eq!(
        args.stand(),
        Some(api::snapshot::WorldTile {
            x: 3222,
            z: 3222,
            level: 0,
        })
    );
    assert_eq!(args.allow_wilderness(), Some(true));
    assert_eq!(args.use_mage_bank(), Some(false));
    assert_eq!(args.use_zanaris_bank(), Some(true));

    let live = [SeatEntry {
        id: bank_id,
        parent: None,
        kind: NativeKind::BankPlan,
        origin: api::script_contract::ChildOrigin::Started,
        depth: 1,
        decisions_consumed: 0,
        prompt: None,
    }];
    let details = [SeatLiveDetails {
        id: bank_id,
        phase: script::native::NativePhase::Waiting,
        prompt: None,
    }];
    let live_page = SeatPageInput::new(&live, &[], &details, 0).expect("bank live page");
    post_native_tick(
        &iso,
        2,
        common::ingame_snapshot(),
        &NativeFactsInput {
            seat_page: Some(live_page),
            ..NativeFactsInput::default()
        },
    );

    let terminal = [SeatResult {
        id: bank_id,
        seq: 1,
        outcome: SeatTerminal::Done,
        prior_parent: None,
        session: Some(bank_id),
        authority_revoked: false,
    }];
    let bank_value_bytes = script::channel::encode(&Value::Null).expect("encode null bank result");
    let bank_value = [SeatResultValue {
        id: bank_id,
        value: &bank_value_bytes,
    }];
    let terminal_page = SeatPageInput::new(&[], &terminal, &[], 0)
        .expect("bank terminal page")
        .with_result_values(&bank_value)
        .expect("bank result value");
    post_native_tick(
        &iso,
        3,
        common::ingame_snapshot(),
        &NativeFactsInput {
            seat_page: Some(terminal_page),
            ..NativeFactsInput::default()
        },
    );
    assert_eq!(
        iso.probe("globalThis.__bankValues")
            .expect("bank result probe succeeds"),
        json!({"ok":true,"value":null})
    );
    assert!(iso
        .probe("globalThis.__rootDone")
        .expect("bank root completion probe succeeds")
        .as_bool()
        .expect("bank root completion is boolean"));
    assert!(interact_requests(&iso).is_empty());
    iso.join();
}

fn drive_inspect_route_requests() {
    let explicit = observe(
        &PLAIN,
        "api.request({op:'inspect-route',from:{x:101,z:102,level:3},to:{x:201,z:202,level:4},allow_teleports:true,allow_wilderness:true,allow_bank_fetch:true,avoid:[{minX:11,maxX:12,minZ:13,maxZ:14,level:2}],request_id:77,max_expansions:99});",
    );
    assert_eq!(
        explicit.value,
        Value::Null,
        "inspect-route request returns void"
    );
    assert!(
        explicit.requests.is_empty(),
        "caller-invented inspect id does not create a host job: {:?}",
        explicit.requests
    );

    let preview = observe(
        &PLAIN,
        "api.request({op:'inspect-route',from:{x:301,z:302,level:1},to:{x:401,z:402,level:0}});",
    );
    assert_eq!(
        preview.value,
        Value::Null,
        "inspect-route request returns void"
    );
    assert_eq!(
        preview.requests,
        vec![InteractReq::InspectRoute {
            x: 401,
            z: 402,
            level: 0,
            from_x: 301,
            from_z: 302,
            from_level: 1,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            avoid: vec![],
            cross: vec![],
            request_id: 0,
            max_expansions: None,
        }]
    );
}

fn drive_inspect_begin() {
    let observed = observe(
        &PLAIN,
        "return api.inspectBegin({from:{x:3179,z:3118,level:2},to:{x:3189,z:3128,level:1},allow_teleports:true,allow_wilderness:true,allow_bank_fetch:true,avoid:[{minX:3100,maxX:3101,minZ:3200,maxZ:3201,level:2}],maxExpansions:700});",
    );
    let token = observed
        .value
        .as_u64()
        .expect("inspect begin returns an isolate token");
    assert!(token > 0);
    assert_eq!(
        observed.requests,
        vec![InteractReq::InspectRoute {
            x: 3189,
            z: 3128,
            level: 1,
            from_x: 3179,
            from_z: 3118,
            from_level: 2,
            allow_teleports: true,
            allow_wilderness: true,
            allow_bank_fetch: true,
            avoid: vec![script::interact::InspectAvoidWire::Rect {
                min_x: 3100,
                max_x: 3101,
                min_z: 3200,
                max_z: 3201,
                level: Some(2),
            }],
            cross: vec![],
            request_id: token,
            max_expansions: Some(700),
        }]
    );
}

struct InspectTerminalObservation {
    token: u64,
    before_settled: bool,
    after_settled: bool,
    before_value: Value,
    value: Value,
    requests: Vec<InteractReq>,
}

fn inspect_terminal_facts(token: u64) -> NativeFactsInput<'static> {
    NativeFactsInput {
        route_inspect: RouteInspectFactsInput {
            latest: RouteInspectTerminalInput {
                seq: 49,
                generation: 12,
                request_id: token,
                ok: true,
                reason: Some("reached"),
                bank_planned: true,
                ticks: 4.5,
                terminal: Some(TileInput {
                    x: 3189,
                    z: 3128,
                    level: 1,
                }),
                hops: &SNAPSHOT_HOPS,
            },
            ..RouteInspectFactsInput::default()
        },
        ..NativeFactsInput::default()
    }
}

fn run_inspect_terminal() -> InspectTerminalObservation {
    let source = script_js::NativeFixture::new("conformance").source(
        r#"
let started = false;
let token = 0;
globalThis.__inspectBeforeSettled = false;
globalThis.__inspectAfterSettled = false;
globalThis.__inspectBeforeValue = null;
globalThis.__inspectValue = null;
export function tick(api) {
    if (!started) {
        started = true;
        token = api.inspectBegin({
            from: { x: 3179, z: 3118, level: 2 },
            to: { x: 3189, z: 3128, level: 1 },
        });
        globalThis.__inspectToken = token;
        globalThis.__inspectBeforeSettled = api.inspectSettled(token);
        globalThis.__inspectBeforeValue = api.inspectValue(token);
        return;
    }
    globalThis.__inspectAfterSettled = api.inspectSettled(token);
    globalThis.__inspectValue = api.inspectValue(token);
}
"#,
    );
    let iso = LoadIsolate::spawn(source, LoadShape::NativeTick, Vec::new())
        .expect("spawn dynamic inspect isolate");
    let iso = wait_ready(iso, "inspect isolate setup");
    post_native_tick(
        &iso,
        1,
        common::ingame_snapshot(),
        &NativeFactsInput::default(),
    );
    let token = iso
        .probe("globalThis.__inspectToken")
        .expect("inspect token probe succeeds")
        .as_u64()
        .expect("inspect begin token is an integer");
    assert!(token > 0);
    post_native_tick(
        &iso,
        2,
        common::ingame_snapshot(),
        &inspect_terminal_facts(token),
    );
    let observation = InspectTerminalObservation {
        token,
        before_settled: iso
            .probe("globalThis.__inspectBeforeSettled")
            .expect("initial inspect status probe succeeds")
            .as_bool()
            .expect("initial inspect status is boolean"),
        after_settled: iso
            .probe("globalThis.__inspectAfterSettled")
            .expect("terminal inspect status probe succeeds")
            .as_bool()
            .expect("terminal inspect status is boolean"),
        before_value: iso
            .probe("globalThis.__inspectBeforeValue")
            .expect("initial inspect value probe succeeds"),
        value: iso
            .probe("globalThis.__inspectValue")
            .expect("terminal inspect value probe succeeds"),
        requests: interact_requests(&iso),
    };
    iso.join();
    observation
}

fn drive_inspect_settled() {
    let observed = run_inspect_terminal();
    assert!(!observed.before_settled);
    assert!(observed.after_settled);
    assert!(observed.before_value.is_null());
    assert_eq!(
        observed.requests,
        vec![InteractReq::InspectRoute {
            x: 3189,
            z: 3128,
            level: 1,
            from_x: 3179,
            from_z: 3118,
            from_level: 2,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            avoid: vec![],
            cross: vec![],
            request_id: observed.token,
            max_expansions: None,
        }]
    );
}

fn drive_inspect_value() {
    let observed = run_inspect_terminal();
    assert!(!observed.before_settled);
    assert!(observed.after_settled);
    assert!(observed.before_value.is_null());
    assert_eq!(
        observed.value,
        json!({
            "ok": true,
            "reason": "reached",
            "bankPlanned": true,
            "ticks": 4.5,
            "terminal": {"x":3189,"z":3128,"level":1},
            "hops": [{
                "kind": "boat",
                "locId": 445,
                "locName": "Conformance ferry",
                "action": "Pay-fare",
                "option": 3,
                "from": {"x":3179,"z":3118,"level":2},
                "to": {"x":3189,"z":3128,"level":1},
                "ticks": 4
            }],
            "request_id": observed.token
        })
    );
}

fn drive_bank_run() {
    let observed = observe_async(&PLAIN, "return await api.bankRun(null);", 1);
    assert_eq!(
        observed.value,
        json!({"kind":"refused","reason":"invalid-args"})
    );
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_cell_run() {
    let observed = observe_async(&PLAIN, "return await api.cellRun(null);", 1);
    assert_eq!(
        observed.value,
        json!({"kind":"refused","reason":"invalid-args"})
    );
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_clue_held_step() {
    let held = observe(&CLUE_SETUP, "return api.clue.heldStep();");
    assert_eq!(
        held.value,
        json!({"ok":true,"value":{"alias":"trail_clue_hard_sextant016_casket","id":3531,"role":"casket","params":[]}})
    );
    assert!(held.requests.is_empty(), "{:?}", held.requests);

    let empty = observe(&CLUE_EMPTY_SETUP, "return api.clue.heldStep();");
    assert_eq!(empty.value, json!({"ok":false,"error":"none-held"}));
    assert!(empty.requests.is_empty(), "{:?}", empty.requests);
}

fn drive_clue_run() {
    let observed = observe_async(
        &CLUE_EMPTY_SETUP,
        "return await api.clue.run({token:1});",
        2,
    );
    assert_eq!(observed.value, json!({"kind":"refused","reason":"stale"}));
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_combat_fight() {
    let observed = observe_async(
        &COMBAT_SETUP,
        "return await api.combat.fight({target:{npc:'cow'}});",
        3,
    );
    assert_eq!(
        observed.value,
        json!({"kind":"done","value":{"end":"stopped","token":1}})
    );

    let request = CombatSessionRequest {
        target: TargetSpec::Names(vec![Arc::from("cow")].into_boxed_slice()),
        ..CombatSessionRequest::default()
    };
    assert_eq!(
        observed.requests,
        vec![InteractReq::CombatFight {
            request_id: 1,
            request: Arc::new(request),
        }]
    );
}

fn drive_combat_stop() {
    let observed = observe(&PLAIN, "return api.combat.stop();");
    assert_eq!(observed.value, json!({"ok":false,"error":"no-session"}));
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_enter_run() {
    let observed = observe_async(&PLAIN, "return await api.enterRun({token:999},{});", 1);
    assert_eq!(
        observed.value,
        json!({"kind":"refused","reason":"unknown token"})
    );
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_fight_run() {
    let observed = observe_async(
        &HUNT_SETUP,
        "const site={key:'conformance',target:'Goblin',safespots:[{x:1,z:1,level:0}],meleeAnchor:{x:1,z:1,level:0},boxes:[{minX:0,maxX:10,minZ:0,maxZ:10,level:0}]}; const token=api.fightBegin(site).value.token; const statuses=[]; const result=await api.fightRun({token},{died:()=>true,setStatus:message=>statuses.push(message)}); return {result,statuses};",
        3,
    );
    assert_eq!(
        observed.value,
        json!({"result":{"kind":"done","value":null},"statuses":["fighting goblins"]})
    );
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_gather_run() {
    let observed = observe_async(&GATHER_SETUP, "return await api.gather.run({});", 3);
    assert_eq!(
        observed.value,
        json!({"kind":"done","value":{"end":"refused","token":1,"reason":"conformance-refused"}})
    );
    assert_eq!(
        observed.requests,
        vec![InteractReq::GatherRun {
            request_id: 1,
            settings: Arc::new(serde_json::Map::new()),
        }]
    );
}

fn drive_gather_stop() {
    let observed = observe(&PLAIN, "return api.gather.stop();");
    assert_eq!(observed.value, json!({"ok":false,"error":"no-session"}));
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_hold_run() {
    let observed = observe_async(&PLAIN, "return await api.holdRun({token:999},{});", 1);
    assert_eq!(
        observed.value,
        json!({"kind":"refused","reason":"unknown token"})
    );
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_key_run() {
    let observed = observe_async(&PLAIN, "return await api.keyRun(null,{});", 1);
    assert_eq!(
        observed.value,
        json!({"kind":"refused","reason":"invalid-args"})
    );
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_leave_run() {
    let observed = observe_async(&PLAIN, "return await api.leaveRun(null,{});", 1);
    assert_eq!(
        observed.value,
        json!({"kind":"refused","reason":"invalid-args"})
    );
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_prayer_set() {
    let observed = observe_async(
        &PRAYER_SET_SETUP,
        "return [await api.prayerSet({name:'Protect from Melee',on:true}),await api.prayerSet({name:'Nope',on:true})];",
        3,
    );
    assert_eq!(
        observed.value,
        json!([
            {"ok":true,"value":true},
            {"ok":false,"error":"unknown-prayer"}
        ])
    );
    assert_eq!(
        observed.requests,
        vec![InteractReq::IfButton { component_id: 5623 }]
    );
}

fn drive_prayer_clear() {
    let observed = observe_async(&PRAYER_CLEAR_SETUP, "return await api.prayerClear();", 4);
    assert_eq!(
        observed.value,
        json!({"ok":true,"value":{"clicked":2,"timed_out":0}})
    );
    assert_eq!(
        observed.requests,
        vec![
            InteractReq::IfButton { component_id: 5622 },
            InteractReq::IfButton { component_id: 5623 }
        ]
    );
}

fn drive_quest_paths() {
    let index: script::quester::queue::ReleaseIndex =
        serde_json::from_str(script::quester::compile::INDEX_JSON)
            .expect("released quest path index");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../script/paths/289");
    let expected_rows: Vec<_> = index
        .paths
        .into_iter()
        .filter(|entry| entry.unavailable.is_none())
        .map(|entry| {
            let file = entry.file.expect("released path file");
            let document: script::quester::path::PathDocument =
                serde_json::from_slice(&std::fs::read(root.join(file)).expect("read path source"))
                    .expect("parse path source");
            let role = &document.roles[0];
            let mut stages = Vec::new();
            for role in &document.roles {
                let mut role_stages: Vec<_> = role
                    .sequences
                    .iter()
                    .map(|sequence| sequence.stage.0.as_ref())
                    .collect();
                role_stages.sort_by_key(|stage| {
                    stage
                        .rsplit_once(':')
                        .and_then(|(_, ordinal)| ordinal.parse::<u32>().ok())
                        .unwrap_or(u32::MAX)
                });
                stages.extend(role_stages);
            }
            json!({
                "id": entry.id,
                "display": document.display_name,
                "journal": !role
                    .progress
                    .as_ref()
                    .expect("quest path role has progress metadata")
                    .rules
                    .is_empty(),
                "draft": entry.status == Some(script::quester::queue::ReleasePathStatus::Draft),
                "stages": stages,
            })
        })
        .collect();
    let observed = observe(&PLAIN, "return api.questPaths();");
    assert_eq!(
        observed.value,
        json!({"ok":true,"value":{"rows":expected_rows}})
    );
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_quest_status() {
    let observed = observe(
        &PLAIN,
        "return [api.questStatus({name:'Death Plateau'}),api.questStatus({name:7})];",
    );
    assert_eq!(
        observed.value,
        json!([
            {"ok":false,"error":"quest-tab-unbound"},
            {"ok":false,"error":"invalid-args"}
        ])
    );
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_gather_methods() {
    let observed = observe(
        &CONTENT,
        "return [api.gatherMethods({skill:'woodcutting'}),api.gatherMethods({skill:'woods'})];",
    );
    assert!(observed.value[0]["ok"]
        .as_bool()
        .expect("gather result is boolean"));
    let value = &observed.value[0]["value"];
    let rows = value["rows"]
        .as_array()
        .expect("gather methods returns rows");
    assert!(!rows.is_empty(), "R289 includes woodcutting methods");
    assert_eq!(rows[0]["id"], "woodcutting.normal");
    assert_eq!(rows[0]["skill"], "woodcutting");
    assert_eq!(rows[0]["resource_key"], "normal");
    assert_eq!(rows[0]["resources"], json!(["normal"]));
    assert_eq!(rows[0]["op"], json!({"slot":1,"label":"Chop down"}));
    assert!(
        rows[0]["loc_ids"]
            .as_array()
            .expect("method reports location aliases")
            .iter()
            .any(|loc| loc["alias"] == "tree"),
        "R289 normal_tree_table includes the tree location"
    );
    assert!(value["coverage"].is_array());
    assert_eq!(
        observed.value[1],
        json!({"ok":false,"error":"unknown-skill"})
    );
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_quest_journal_begin() {
    let observed = observe(
        &JOURNAL_SETUP,
        "return [api.questJournalBegin({name:\"Cook's Assistant\"}),api.questJournalBegin({name:''})];",
    );
    assert!(observed.value[0]["ok"]
        .as_bool()
        .expect("journal begin result is boolean"));
    let token = observed.value[0]["value"]["token"]
        .as_u64()
        .expect("journal begin returns an isolate token");
    assert!(token > 0);
    assert_eq!(
        observed.value[1],
        json!({"ok":false,"error":"invalid-args"})
    );
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_quest_journal_run() {
    let observed = observe_async(
        &JOURNAL_SETUP,
        "const begin=api.questJournalBegin({name:\"Cook's Assistant\"}); if(!begin.ok) return begin; const token=begin.value.token; const done=await api.questJournalRun(begin.value); const invalid=await api.questJournalRun({token:-1}); return {token,done,invalid};",
        4,
    );
    let token = observed.value["token"]
        .as_u64()
        .expect("journal run reports its begin token");
    assert!(token > 0);
    assert_eq!(
        observed.value,
        json!({
            "token": token,
            "done":{
                "kind":"done",
                "value":{
                    "kind":"done",
                    "token": token,
                    "lines":["@dre@The Cook's Quest"],
                    "root":77,
                    "as_of_sequence":2,
                    "closed_as_of_sequence":4
                }
            },
            "invalid":{"kind":"refused","reason":"invalid-args"}
        })
    );
    assert_eq!(
        observed.requests,
        vec![
            InteractReq::IfButton { component_id: 1234 },
            InteractReq::CloseModal
        ]
    );
}

fn drive_quest_progress() {
    let observed = observe_async(
        &PROGRESS_SETUP,
        "return await api.questProgress({quest:'death'});",
        3,
    );
    assert_eq!(
        observed.value,
        json!({"kind":"done","value":{"end":"refused","token":1,"reason":"conformance-refused"}})
    );
    assert_eq!(
        observed.requests,
        vec![InteractReq::ProgressRead {
            request_id: 1,
            name: "death".into(),
            journal_only: false,
        }]
    );
}

fn drive_range_loadout() {
    let observed = observe(
        &CONTENT,
        "return [api.rangeLoadoutOf({weapon:'Bronze dart',ammo:'Iron arrow'}),api.rangeLoadoutOf({weapon:7,ammo:'Iron arrow'})];",
    );
    assert_eq!(
        observed.value,
        json!([
            {"ok":true,"value":{"weapon":"Bronze dart","projectile":"Bronze dart","thrown":true}},
            {"ok":false,"error":"invalid-args"}
        ])
    );
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_retreat_run() {
    let source = script_js::NativeFixture::new("conformance").source(
        r#"
const site = {
    key: 't',
    target: 'Goblin',
    safespots: [{ x: 1, z: 1, level: 0 }],
    meleeAnchor: { x: 1, z: 1, level: 0 },
    boxes: [{ minX: 0, maxX: 10, minZ: 0, maxZ: 10, level: 0 }],
};
let started = false;
globalThis.__statuses = [];
globalThis.__retreat = null;
export async function tick(api) {
    if (started) return;
    started = true;
    const status = (message) => globalThis.__statuses.push(message);
    const token = api.retreatBegin(site).value.token;
    globalThis.__retreat = await api.retreatRun({ token }, { setStatus: status });
}
"#,
    );
    let iso = LoadIsolate::spawn(source, LoadShape::NativeTick, Vec::new())
        .expect("spawn retreat isolate");
    let iso = wait_ready(iso, "retreat isolate setup");
    for tick in 1..=2 {
        let mut input = common::ingame_snapshot();
        input.tick = tick;
        (HUNT_SETUP.snapshot)(tick, &mut input);
        common::post_snapshot_input(&iso, &input);
        assert!(
            iso.probe("true").is_ok(),
            "snapshot for retreat tick {tick} must be applied"
        );
        iso.on_game_tick(tick);
    }
    assert_eq!(
        iso.probe("globalThis.__retreat")
            .expect("retreat result probe succeeds"),
        json!({"kind":"done","value":null})
    );
    assert_eq!(
        iso.probe("globalThis.__statuses")
            .expect("retreat status probe succeeds"),
        json!([])
    );
    let requests = interact_requests(&iso);
    assert!(
        requests.is_empty(),
        "retreat without a posted tile does not move: {requests:?}"
    );
    iso.join();
}

fn drive_scene_locs() {
    let observed = observe(
        &PLAIN,
        "return [api.sceneLocs({ids:[2213],limit:8}),api.sceneLocs({ids:[2213],limit:0})];",
    );
    assert_eq!(
        observed.value,
        json!([
            {"ok":false,"error":"snapshot-unavailable"},
            {"ok":false,"error":"invalid-args"}
        ])
    );
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_scene_npcs() {
    let observed = observe(
        &PLAIN,
        "return [api.sceneNpcs({types:[90],actions:['Attack'],limit:8}),api.sceneNpcs({types:[90],actions:['Attack'],limit:0})];",
    );
    assert_eq!(
        observed.value,
        json!([
            {"ok":false,"error":"snapshot-unavailable"},
            {"ok":false,"error":"invalid-args"}
        ])
    );
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}

fn drive_walkspot_run() {
    let observed = observe_async(&PLAIN, "return await api.walkspotRun({token:999},{});", 1);
    assert_eq!(
        observed.value,
        json!({"kind":"refused","reason":"unknown token"})
    );
    assert!(observed.requests.is_empty(), "{:?}", observed.requests);
}
fn snapshot_rows() -> Vec<Row> {
    let item = json!({
        "name": "Rune platebody",
        "count": 3,
        "id": 1123,
        "ops": [],
        "noted": true,
        "cert": 1201,
        "component_id": 583,
        "slot": 5,
    });
    let route_hop = json!({
        "kind": "boat",
        "locId": 445,
        "locName": "Conformance ferry",
        "action": "Pay-fare",
        "option": 3,
        "from": {"x": 3179, "z": 3118, "level": 2},
        "to": {"x": 3189, "z": 3128, "level": 1},
        "ticks": 4,
    });
    vec![
        read_row("NativeApi.snapshot.bank", "the posted bank item preserves identity and slot", SNAPSHOT_SETUP, "return api.snapshot.bank[0];", item.clone()),
        read_row("NativeApi.snapshot.bank_approaches", "bank destination readiness is copied from the host facts page", SNAPSHOT_SETUP, "return api.snapshot.bank_approaches;", json!([{"loc_id":2213,"x":3182,"z":3119,"level":2,"can_operate":true,"dest_ok":true,"dest_x":3181,"dest_z":3119,"dest_level":2}])),
        read_row("NativeApi.snapshot.bank_generation", "bank action generation is the posted session identity", SNAPSHOT_SETUP, "return api.snapshot.bank_generation;", json!(123)),
        read_row("NativeApi.snapshot.bank_loaded", "bank content-loaded state is posted", SNAPSHOT_SETUP, "return api.snapshot.bank_loaded;", json!(true)),
        read_row("NativeApi.snapshot.bank_op_result", "bank operation result is independently posted", SNAPSHOT_SETUP, "return api.snapshot.bank_op_result;", json!(true)),
        read_row("NativeApi.snapshot.bank_op_result_seq", "bank operation result sequence is preserved", SNAPSHOT_SETUP, "return api.snapshot.bank_op_result_seq;", json!(84)),
        read_row("NativeApi.snapshot.bank_open", "bank-open state is posted", SNAPSHOT_SETUP, "return api.snapshot.bank_open;", json!(true)),
        read_row("NativeApi.snapshot.bank_selection", "select-only bank result retains correlation and reachable kind without inventing a named bank", SNAPSHOT_SETUP, "return api.snapshot.bank_selection;", json!({"request_id":111,"generation":112,"kind":"reachable","bank":null})),
        read_row("NativeApi.snapshot.bank_side", "the side-container row retains its posted item identity", SNAPSHOT_SETUP, "return api.snapshot.bank_side[0];", item.clone()),
        read_row("NativeApi.snapshot.bank_snapshot_generation", "bank item packet generation is distinct from the session generation", SNAPSHOT_SETUP, "return api.snapshot.bank_snapshot_generation;", json!(303)),
        read_row("NativeApi.snapshot.banks", "packed bank stand preserves teller access metadata", SNAPSHOT_SETUP, "return api.snapshot.banks[0];", json!({"name":"Conformance teller","x":3181,"z":3119,"level":2,"kind":"npc","op":2,"choose":"Bank"})),
        read_row("NativeApi.snapshot.collision", "the collision view exposes its posted bounds and raw flags", SNAPSHOT_SETUP, "const c=api.snapshot.collision; return {available:c.available,base_x:c.base_x,base_z:c.base_z,level:c.level,width:c.width,height:c.height,flags:[c.flags.length,c.flags.at(0),c.flags.at(1)]};", json!({"available":true,"base_x":3170,"base_z":3100,"level":2,"width":1,"height":2,"flags":[2,0x1234,0x5678]})),
        read_row("NativeApi.snapshot.combat", "the live combat page carries token and phase with no unpublished status", SNAPSHOT_SETUP, "return api.snapshot.combat;", json!({"token":319,"phase":"running","status":null})),
        read_row("NativeApi.snapshot.count_dialog_open", "count-dialog visibility is a posted bank fact", SNAPSHOT_SETUP, "return api.snapshot.count_dialog_open;", json!(true)),
        read_row("NativeApi.snapshot.gather", "the live gather page carries token and phase with no unpublished status", SNAPSHOT_SETUP, "return api.snapshot.gather;", json!({"token":319,"phase":"running","status":null})),
        read_row("NativeApi.snapshot.here", "the posted world tile retains all coordinates", SNAPSHOT_SETUP, "return api.snapshot.here;", json!({"x":3179,"z":3118,"level":2})),
        read_row("NativeApi.snapshot.ingame", "the player-info in-game bit is a host-owned snapshot value", SNAPSHOT_SETUP, "return api.snapshot.ingame;", json!(true)),
        read_row("NativeApi.snapshot.inv", "inventory rows preserve the exact item and slot fields", SNAPSHOT_SETUP, "return api.snapshot.inv[0];", item.clone()),
        read_row("NativeApi.snapshot.inv_size", "inventory capacity is distinct from row count", SNAPSHOT_SETUP, "return api.snapshot.inv_size;", json!(19)),
        read_row("NativeApi.snapshot.nearest_booth", "the host-picked nearest booth includes its action", SNAPSHOT_SETUP, "return api.snapshot.nearest_booth;", json!({"x":3182,"z":3119,"level":2,"id":2213,"name":"Bank booth","op":"Use-quickly"})),
        read_row("NativeApi.snapshot.npcs", "NPC observation preserves packet-time identity and target facts", SNAPSHOT_SETUP, "return api.snapshot.npcs[0];", json!({"index":17,"id":90,"name":"Conformance guard","x":3180,"z":3117,"level":2,"tile":{"x":3180,"z":3117,"level":2},"distance":4,"health":7,"max_health":12,"totalHealth":12,"in_combat":true,"animating":false,"actions":[],"reachable":true,"reachable_adj":false,"combat_level":29,"target_kind":2,"target_index":18,"faceEntity":32786,"size":1,"nx":3180,"nz":3117,"network_x":3180,"network_z":3117})),
        read_row("NativeApi.snapshot.puzzle_board", "board rows and the observed container identity arrive as one native page", SNAPSHOT_SETUP, "return api.snapshot.puzzle_board;", json!({"component_id":583,"size":4,"items":[item]})),
        read_row("NativeApi.snapshot.puzzle_board_generation", "puzzle board session generation is paired with its rows", SNAPSHOT_SETUP, "return api.snapshot.puzzle_board_generation;", json!(88)),
        read_row("NativeApi.snapshot.route_inspect_accepted_id", "the accepted inspect token is copied from host facts", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_accepted_id;", json!(63)),
        read_row("NativeApi.snapshot.route_inspect_bank_planned", "the latest inspect's bank plan bit is preserved", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_bank_planned;", json!(true)),
        read_row("NativeApi.snapshot.route_inspect_generation", "latest inspect generation is preserved", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_generation;", json!(12)),
        read_row("NativeApi.snapshot.route_inspect_hops", "latest inspect hops retain full endpoints and travel metadata", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_hops;", json!([route_hop.clone()])),
        read_row("NativeApi.snapshot.route_inspect_ok", "the latest inspect terminal status is preserved", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_ok;", json!(true)),
        read_row("NativeApi.snapshot.route_inspect_pending_id", "pending inspect correlation is preserved", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_pending_id;", json!(62)),
        read_row("NativeApi.snapshot.route_inspect_prev_bank_planned", "the previous inspect retains its own bank-plan bit", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_prev_bank_planned;", json!(false)),
        read_row("NativeApi.snapshot.route_inspect_prev_generation", "previous inspect generation is separate from the latest page", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_prev_generation;", json!(14)),
        read_row("NativeApi.snapshot.route_inspect_prev_hops", "previous inspect retains its own hop list", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_prev_hops;", json!([route_hop])),
        read_row("NativeApi.snapshot.route_inspect_prev_ok", "previous inspect terminal status is retained", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_prev_ok;", json!(false)),
        read_row("NativeApi.snapshot.route_inspect_prev_reason", "previous inspect refusal reason is retained", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_prev_reason;", json!("no-path")),
        read_row("NativeApi.snapshot.route_inspect_prev_request_id", "previous inspect request correlation is retained", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_prev_request_id;", json!(15)),
        read_row("NativeApi.snapshot.route_inspect_prev_seq", "previous inspect sequence is distinct from latest", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_prev_seq;", json!(51)),
        read_row("NativeApi.snapshot.route_inspect_prev_ticks", "previous inspect tick estimate is preserved", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_prev_ticks;", json!(6.5)),
        read_row("NativeApi.snapshot.route_inspect_reason", "latest inspect reason is preserved", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_reason;", json!("reached")),
        read_row("NativeApi.snapshot.route_inspect_refused_id", "first refused inspect token is preserved", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_refused_id;", json!(66)),
        read_row("NativeApi.snapshot.route_inspect_refused_id_2", "second refused inspect token is preserved", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_refused_id_2;", json!(67)),
        read_row("NativeApi.snapshot.route_inspect_refused_id_3", "third refused inspect token is preserved", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_refused_id_3;", json!(68)),
        read_row("NativeApi.snapshot.route_inspect_replaced_id", "replacement inspect token is preserved", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_replaced_id;", json!(64)),
        read_row("NativeApi.snapshot.route_inspect_replaced_prev_id", "replaced previous inspect token is preserved", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_replaced_prev_id;", json!(65)),
        read_row("NativeApi.snapshot.route_inspect_request_id", "latest inspect request correlation is preserved", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_request_id;", json!(13)),
        read_row("NativeApi.snapshot.route_inspect_running_id", "running inspect token is preserved", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_running_id;", json!(61)),
        read_row("NativeApi.snapshot.route_inspect_seq", "latest inspect sequence is preserved", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_seq;", json!(49)),
        read_row("NativeApi.snapshot.route_inspect_ticks", "latest inspect tick estimate is preserved", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_ticks;", json!(4.5)),
        read_row("NativeApi.snapshot.route_inspect_unobserved", "inspect obligation count is preserved", SNAPSHOT_SETUP, "return api.snapshot.route_inspect_unobserved;", json!(69)),
        read_row("NativeApi.snapshot.self_target_index", "the local target index is preserved even when nonzero", SNAPSHOT_SETUP, "return api.snapshot.self_target_index;", json!(18)),
        read_row("NativeApi.snapshot.self_target_kind", "the local target kind uses the documented player discriminator", SNAPSHOT_SETUP, "return api.snapshot.self_target_kind;", json!(2)),
        read_row("NativeApi.snapshot.stats", "stat snapshots preserve index, xp, base, and effective values", SNAPSHOT_SETUP, "return api.snapshot.stats[0];", json!({"index":2,"name":"attack","xp":12345,"base":41,"effective":43})),
        read_row("NativeApi.snapshot.user_move_intent_seq", "qualifying user movement intent is independent of walk completion", SNAPSHOT_SETUP, "return api.snapshot.user_move_intent_seq;", json!(301)),
        read_row("NativeApi.snapshot.walk_outcome_allow_teleports", "walk completion preserves its teleport permission", SNAPSHOT_SETUP, "return api.snapshot.walk_outcome_allow_teleports;", json!(true)),
        read_row("NativeApi.snapshot.walk_outcome_cancel_reason", "walk completion distinguishes a user takeover", SNAPSHOT_SETUP, "return api.snapshot.walk_outcome_cancel_reason;", json!("user-input")),
        read_row("NativeApi.snapshot.walk_outcome_failed", "failed walk outcome bit is preserved", SNAPSHOT_SETUP, "return api.snapshot.walk_outcome_failed;", json!(true)),
        read_row("NativeApi.snapshot.walk_outcome_generation", "walk completion generation is preserved", SNAPSHOT_SETUP, "return api.snapshot.walk_outcome_generation;", json!(32)),
        read_row("NativeApi.snapshot.walk_outcome_level", "walk completion destination plane is preserved", SNAPSHOT_SETUP, "return api.snapshot.walk_outcome_level;", json!(1)),
        read_row("NativeApi.snapshot.walk_outcome_radius", "walk completion radius is preserved", SNAPSHOT_SETUP, "return api.snapshot.walk_outcome_radius;", json!(7)),
        read_row("NativeApi.snapshot.walk_outcome_request_id", "walk completion correlation is preserved", SNAPSHOT_SETUP, "return api.snapshot.walk_outcome_request_id;", json!(33)),
        read_row("NativeApi.snapshot.walk_outcome_seq", "walk completion sequence is preserved", SNAPSHOT_SETUP, "return api.snapshot.walk_outcome_seq;", json!(31)),
        read_row("NativeApi.snapshot.walk_outcome_x", "walk completion destination x is preserved", SNAPSHOT_SETUP, "return api.snapshot.walk_outcome_x;", json!(3184)),
        read_row("NativeApi.snapshot.walk_outcome_z", "walk completion destination z is preserved", SNAPSHOT_SETUP, "return api.snapshot.walk_outcome_z;", json!(3194)),
        read_row("NativeApi.snapshot.withdraw_x_result", "withdraw-X completion boolean is preserved", SNAPSHOT_SETUP, "return api.snapshot.withdraw_x_result;", json!(true)),
        read_row("NativeApi.snapshot.withdraw_x_result_seq", "withdraw-X completion sequence is preserved", SNAPSHOT_SETUP, "return api.snapshot.withdraw_x_result_seq;", json!(409)),
    ]
}
fn request_rows() -> Vec<Row> {
    vec![
        send_row(
            "NativeApi.request[op=deposit]",
            "deposit encodes the exact item name and leaves the compat-only id absent",
            "api.request({op:'deposit',name:'Rune platebody'});",
            vec![InteractReq::Deposit {
                name: "Rune platebody".into(),
                id: None,
            }],
        ),
        send_row(
            "NativeApi.request[op=held]",
            "held interaction encodes the exact item name and action",
            "api.request({op:'held',name:'Rune platebody',action:'Wear'});",
            vec![InteractReq::Held {
                name: "Rune platebody".into(),
                action: "Wear".into(),
                slot: None,
                target_item_id: None,
            }],
        ),
        Row {
            id: "NativeApi.request[op=inspect-route]",
            why: "caller-invented inspect ids are ignored; preview id zero is encoded",
            case: Case::Drive(drive_inspect_route_requests),
        },
        send_row(
            "NativeApi.request[op=open-booth]",
            "open-booth preserves nondefault coordinates and both optional access strings",
            "api.request({op:'open-booth',x:401,z:402,level:2,id:2213,name:'Bank booth',action:'Use-quickly'}); api.request({op:'open-booth',x:501,z:502,level:1,id:2214});",
            vec![
                InteractReq::OpenBooth {
                    x: 401,
                    z: 402,
                    level: 2,
                    id: 2213,
                    name: Some("Bank booth".into()),
                    action: Some("Use-quickly".into()),
                },
                InteractReq::OpenBooth {
                    x: 501,
                    z: 502,
                    level: 1,
                    id: 2214,
                    name: None,
                    action: None,
                },
            ],
        ),
        send_row(
            "NativeApi.request[op=open-stand]",
            "open-stand carries packed kind, name, operation slot, and choice when present",
            "api.request({op:'open-stand',x:601,z:602,level:3,kind:'npc',name:'Banker',stand_op:4,choose:'Bank'}); api.request({op:'open-stand',x:701,z:702,level:1,kind:'booth'});",
            vec![
                InteractReq::OpenStand {
                    x: 601,
                    z: 602,
                    level: 3,
                    kind: "npc".into(),
                    name: Some("Banker".into()),
                    stand_op: Some(4),
                    choose: Some("Bank".into()),
                },
                InteractReq::OpenStand {
                    x: 701,
                    z: 702,
                    level: 1,
                    kind: "booth".into(),
                    name: None,
                    stand_op: None,
                    choose: None,
                },
            ],
        ),
        send_row(
            "NativeApi.request[op=puzzle-move]",
            "puzzle move preserves piece, slot, component, and board generation",
            "api.request({op:'puzzle-move',id:2145,slot:6,component:583,generation:88});",
            vec![InteractReq::PuzzleMove {
                id: 2145,
                slot: 6,
                component: 583,
                generation: 88,
            }],
        ),
        send_row(
            "NativeApi.request[op=set-note-mode]",
            "note mode carries the selected boolean without host-side policy",
            "api.request({op:'set-note-mode',on:true});",
            vec![InteractReq::SetNoteMode { on: true }],
        ),
        send_row(
            "NativeApi.request[op=walk-near]",
            "walk-near encodes all supplied flags and request id, then clears omitted optionals",
            "api.request({op:'walk-near',x:801,z:802,level:3,radius:5,allow_teleports:true,allow_wilderness:true,allow_bank_fetch:true,request_id:88}); api.request({op:'walk-near',x:901,z:902,level:1,radius:2});",
            vec![
                InteractReq::WalkNear {
                    x: 801,
                    z: 802,
                    level: 3,
                    radius: 5,
                    compat_tile_intent: false,
                    allow_teleports: true,
                    allow_wilderness: true,
                    allow_bank_fetch: true,
                    allow_prereq_dialogue: None,
                    allow_quest_start: None,
                    request_id: 88,
                    track_door_crossing: false,
                    avoid: vec![],
                    cross: vec![],
                },
                InteractReq::WalkNear {
                    x: 901,
                    z: 902,
                    level: 1,
                    radius: 2,
                    compat_tile_intent: false,
                    allow_teleports: false,
                    allow_wilderness: false,
                    allow_bank_fetch: false,
                    allow_prereq_dialogue: None,
                    allow_quest_start: None,
                    request_id: 0,
                    track_door_crossing: false,
                    avoid: vec![],
                    cross: vec![],
                },
            ],
        ),
        send_row(
            "NativeApi.request[op=walk-nearest-bank]",
            "walk-nearest-bank is the zero-payload bank-selection request",
            "api.request({op:'walk-nearest-bank'});",
            vec![InteractReq::WalkNearestBank],
        ),
        send_row(
            "NativeApi.request[op=walk]",
            "walk encodes every supplied field and applies false/zero defaults when optionals are omitted",
            "api.request({op:'walk',x:1001,z:1002,level:3,allow_teleports:true,allow_wilderness:true,allow_bank_fetch:true,request_id:91}); api.request({op:'walk',x:1101,z:1102,level:1});",
            vec![
                InteractReq::Walk {
                    x: 1001,
                    z: 1002,
                    level: 3,
                    allow_teleports: true,
                    allow_wilderness: true,
                    allow_bank_fetch: true,
                    allow_prereq_dialogue: None,
                    allow_quest_start: None,
                    request_id: 91,
                    track_door_crossing: false,
                    avoid: vec![],
                    cross: vec![],
                },
                InteractReq::Walk {
                    x: 1101,
                    z: 1102,
                    level: 1,
                    allow_teleports: false,
                    allow_wilderness: false,
                    allow_bank_fetch: false,
                    allow_prereq_dialogue: None,
                    allow_quest_start: None,
                    request_id: 0,
                    track_door_crossing: false,
                    avoid: vec![],
                    cross: vec![],
                },
            ],
        ),
        send_row(
            "NativeApi.request[op=withdraw-x]",
            "withdraw-X preserves item identity, count, action, and current bank generation",
            "api.request({op:'withdraw-x',name:'Lobster',count:17,bank_item_id:379,lands_as_id:379,action:'Withdraw-X',bank_generation:123});",
            vec![InteractReq::WithdrawX {
                name: "Lobster".into(),
                count: 17,
                bank_item_id: 379,
                lands_as_id: 379,
                action: "Withdraw-X".into(),
                bank_generation: 123,
            }],
        ),
        send_row(
            "NativeApi.request[op=withdraw]",
            "withdraw preserves exact item and action strings",
            "api.request({op:'withdraw',name:'Lobster',action:'Withdraw-5'});",
            vec![InteractReq::Withdraw {
                name: "Lobster".into(),
                action: "Withdraw-5".into(),
            }],
        ),
    ]
}

fn helper_rows() -> Vec<Row> {
    vec![
        Row {
            id: "NativeApi.bankNearestReachable",
            why: "the typed bank seat settles from its posted terminal result",
            case: Case::Drive(drive_bank_nearest_reachable),
        },
        Row {
            id: "NativeApi.bankRun",
            why: "the one-shot bank session resolves its own invalid-site refusal",
            case: Case::Drive(drive_bank_run),
        },
        read_row(
            "NativeApi.boostFaded",
            "a seven-level boost is faded and an invalid floor is refused",
            PLAIN,
            "return [api.boostFaded({base:70,effective:77}),api.boostFaded({base:70,effective:77,floor:'bad'})];",
            json!([{"ok":true,"value":true},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.canReach",
            "the posted reachable rank is true and malformed input is refused",
            MOVEMENT_SETUP,
            "return [api.canReach({tile:{x:3204,z:3200,level:0},maxSteps:20}),api.canReach(null)];",
            json!([{"ok":true,"value":true},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.canStep",
            "the posted step byte permits the adjacent tile and malformed input is refused",
            MOVEMENT_SETUP,
            "return [api.canStep({from:{x:3200,z:3200,level:0},to:{x:3201,z:3200,level:0}}),api.canStep(null)];",
            json!([{"ok":true,"value":true},{"ok":false,"error":"invalid-args"}]),
        ),
        Row {
            id: "NativeApi.cellRun",
            why: "cell run settles the stale-token refusal without inventing a host action",
            case: Case::Drive(drive_cell_run),
        },
        read_row(
            "NativeApi.clue.row",
            "the landed casket row is returned exactly and an unknown id is refused",
            CONTENT,
            "return [api.clue.row({id:3531}),api.clue.row({id:0})];",
            json!([
                {"ok":true,"value":{"alias":"trail_clue_hard_sextant016_casket","id":3531,"role":"casket","params":[]}},
                {"ok":false,"error":"unknown-id"}
            ]),
        ),
        read_row(
            "NativeApi.clue.hardKit",
            "a complete caller-supplied kit is ready and low attack reports its first failure",
            PLAIN,
            "return [api.clue.hardKit({attack:60,lostCity:true,items:[{id:1231,count:1},{id:185,count:1},{id:385,count:15}]}),api.clue.hardKit({attack:59,lostCity:false,items:[]})];",
            json!([{"ok":true,"value":{"status":"ready"}},{"ok":false,"error":"attack"}]),
        ),
        read_row(
            "NativeApi.clue.keep",
            "the fixed spade identity is kept and food is not kept",
            PLAIN,
            "return [api.clue.keep({name:'Spade'}),api.clue.keep({name:'Shark'})];",
            json!([{"ok":true,"value":{"keep":true}},{"ok":true,"value":{"keep":false}}]),
        ),
        read_row(
            "NativeApi.clue.packPlan",
            "pack targets use the caller's arithmetic and report no-room as a refusal",
            PLAIN,
            "return [api.clue.packPlan({hostWant:20,freeSlots:5,reserveSlots:3,perCast:3,weaponName:'Rune scimitar',casketAlias:'trail_clue_hard_sextant001_casket'}),api.clue.packPlan({hostWant:20,freeSlots:0})];",
            json!([
                {"ok":true,"value":{"coordToolSlots":3,"teleportCasts":20,"food":2,"runeTarget":60,"weaponNeeded":true,"rewardSlots":6}},
                {"ok":false,"error":"no-room"}
            ]),
        ),
        Row {
            id: "NativeApi.clue.heldStep",
            why: "held-step returns the posted first membership row and refuses an empty pack",
            case: Case::GameDataDrive(drive_clue_held_step),
        },
        read_row(
            "NativeApi.clue.retry",
            "retry clears the clue latch and returns the documented receipt",
            PLAIN,
            "return api.clue.retry();",
            json!({"ok":true,"value":{"cleared":true}}),
        ),
        read_row(
            "NativeApi.clue.begin",
            "begin refuses when the posted pack has no held clue",
            CONTENT,
            "return api.clue.begin();",
            json!({"ok":false,"error":"none-held"}),
        ),
        Row {
            id: "NativeApi.clue.run",
            why: "one awaited clue run settles the stale-token refusal",
            case: Case::GameDataDrive(drive_clue_run),
        },
        Row {
            id: "NativeApi.combat.fight",
            why: "one combat session sends its typed request and accepts its posted terminal",
            case: Case::Drive(drive_combat_fight),
        },
        Row {
            id: "NativeApi.combat.stop",
            why: "stop without a live fight returns no-session and sends no request",
            case: Case::Drive(drive_combat_stop),
        },
        read_row(
            "NativeApi.combatKeepNames",
            "selected combat data adds the food and exact spell runes while rejecting bad extras",
            CONTENT,
            "return [api.combatKeepNames({food:'Lobster',style:'mage',spell:'Wind Strike',extra:['Mind rune','Air rune']}),api.combatKeepNames({food:'Lobster',extra:['Coins',1]})];",
            json!([{"ok":true,"value":["lobster","Mind rune","Air rune","Mind rune","Air rune"]},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.enterBegin",
            "a valid site allocates a token and a missing site is refused",
            PLAIN,
            "return [api.enterBegin({key:'conformance',boxes:[{minX:0,maxX:10,minZ:0,maxZ:10,level:0}]}),api.enterBegin(null)];",
            json!([{"ok":true,"value":{"token":1}},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.enterValidate",
            "an unknown token is a valid false validation and malformed input is refused",
            PLAIN,
            "return [api.enterValidate({token:999},{}),api.enterValidate(null,{})];",
            json!([{"ok":true,"value":false},{"ok":false,"error":"invalid-args"}]),
        ),
        Row {
            id: "NativeApi.enterRun",
            why: "one enter session settles the stale-token refusal",
            case: Case::Drive(drive_enter_run),
        },
        read_row(
            "NativeApi.escapeRunesFor",
            "Varrock teleport uses its declared rune recipe and unknown destinations are refused",
            CONTENT,
            "return [api.escapeRunesFor({id:'varrock'}),api.escapeRunesFor({id:'narnia'})];",
            json!([
                {"ok":true,"value":{"runes":[{"rune":"Fire rune","count":1},{"rune":"Air rune","count":3},{"rune":"Law rune","count":1}],"level":25,"label":"Varrock teleport"}},
                {"ok":false,"error":"unknown-id"}
            ]),
        ),
        read_row(
            "NativeApi.fightBegin",
            "a valid target site allocates a token and a missing site is refused",
            PLAIN,
            "return [api.fightBegin({key:'conformance',target:'Goblin',boxes:[{minX:0,maxX:10,minZ:0,maxZ:10,level:0}]}),api.fightBegin(null)];",
            json!([{"ok":true,"value":{"token":1}},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.fightBlocksLoot",
            "an unknown fight token does not block loot and malformed input is refused",
            PLAIN,
            "return [api.fightBlocksLoot({token:999},{}),api.fightBlocksLoot(null,{})];",
            json!([{"ok":true,"value":false},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.fightInterruptWatch",
            "interrupt-watch accepts an unknown token and refuses malformed input",
            PLAIN,
            "return [api.fightInterruptWatch({token:999}),api.fightInterruptWatch(null)];",
            json!([{"ok":true,"value":null},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.fightReset",
            "reset accepts an unknown token and refuses malformed input",
            PLAIN,
            "return [api.fightReset({token:999}),api.fightReset(null)];",
            json!([{"ok":true,"value":null},{"ok":false,"error":"invalid-args"}]),
        ),
        Row {
            id: "NativeApi.fightRun",
            why: "one hunt task settles when its driver reports that the character died",
            case: Case::Drive(drive_fight_run),
        },
        read_row(
            "NativeApi.fightValidate",
            "an unknown fight token is a valid false validation and malformed input is refused",
            PLAIN,
            "return [api.fightValidate({token:999},{}),api.fightValidate(null,{})];",
            json!([{"ok":true,"value":false},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.foodCount",
            "food count counts matching rows rather than stack quantity and rejects nonarrays",
            CONTENT,
            "return [api.foodCount({items:[{name:'Lobster',count:10},{name:'Lobster',count:2}],foodName:'Lobster'}),api.foodCount({items:'bad',foodName:'Lobster'})];",
            json!([{"ok":true,"value":2},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.foodHealAmount",
            "Bread has the selected four-point heal and an unknown food is refused",
            CONTENT,
            "return [api.foodHealAmount({foodName:'Bread'}),api.foodHealAmount({foodName:'Cabbage'})];",
            json!([{"ok":true,"value":4},{"ok":false,"error":"unknown-food"}]),
        ),
        read_row(
            "NativeApi.foodOf",
            "food selection picks the first supported carry item and uses fallback otherwise",
            CONTENT,
            "return [api.foodOf({loadout:{carry:[{item:'Lobster',qty:4},{item:'Cabbage',qty:2}]},fallback:'Trout'}),api.foodOf({loadout:null,fallback:'Trout'}),api.foodOf({loadout:null,fallback:1})];",
            json!([{"ok":true,"value":"Lobster"},{"ok":true,"value":"Trout"},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.gearOf",
            "gear follows the fixed slot order and rejects a nonstring worn value",
            PLAIN,
            "return [api.gearOf({loadout:{worn:{torso:'Rune chainbody',righthand:'Rune scimitar',hat:'Rune full helm',mystery:'Ignored slot'},unassigned:['old helm',''],carry:[]}}),api.gearOf({loadout:{worn:{hat:1}}})];",
            json!([{"ok":true,"value":["Rune full helm","Rune scimitar","Rune chainbody","old helm"]},{"ok":false,"error":"invalid-args"}]),
        ),
        Row {
            id: "NativeApi.gather.run",
            why: "one gather session sends its run request and settles its posted terminal",
            case: Case::Drive(drive_gather_run),
        },
        Row {
            id: "NativeApi.gather.stop",
            why: "stop without a live gather returns no-session and sends no request",
            case: Case::Drive(drive_gather_stop),
        },
        Row {
            id: "NativeApi.gatherMethods",
            why: "the loaded R289 woodcutting page includes the normal tree method and refuses an unknown skill",
            case: Case::GameDataDrive(drive_gather_methods),
        },
        read_row(
            "NativeApi.gatherPlacements",
            "a missing region is refused before catalog lookup",
            PLAIN,
            "return api.gatherPlacements({resource:'oak',limit:64});",
            json!({"ok":false,"error":"missing-region"}),
        ),
        read_row(
            "NativeApi.gatherResource",
            "an unknown resource key is refused rather than reported as an empty page",
            CONTENT,
            "return api.gatherResource({name:'not-a-resource'});",
            json!({"ok":false,"error":"unknown-resource"}),
        ),
        read_row(
            "NativeApi.holdBegin",
            "a valid site allocates a token and a missing site is refused",
            PLAIN,
            "return [api.holdBegin({key:'conformance',boxes:[{minX:0,maxX:10,minZ:0,maxZ:10,level:0}]}),api.holdBegin(null)];",
            json!([{"ok":true,"value":{"token":1}},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.holdValidate",
            "an unknown hold token is a valid false validation and malformed input is refused",
            PLAIN,
            "return [api.holdValidate({token:999},{}),api.holdValidate(null,{})];",
            json!([{"ok":true,"value":false},{"ok":false,"error":"invalid-args"}]),
        ),
        Row {
            id: "NativeApi.holdRun",
            why: "one hold task settles its stale-token refusal",
            case: Case::Drive(drive_hold_run),
        },
        Row {
            id: "NativeApi.inspectBegin",
            why: "inspect begin allocates a token and emits its matching route request",
            case: Case::Drive(drive_inspect_begin),
        },
        Row {
            id: "NativeApi.inspectSettled",
            why: "a matching posted terminal changes inspect settled from false to true",
            case: Case::Drive(drive_inspect_settled),
        },
        Row {
            id: "NativeApi.inspectValue",
            why: "a matching posted terminal exposes its route result and hop",
            case: Case::Drive(drive_inspect_value),
        },
        Row {
            id: "NativeApi.keyRun",
            why: "one key session settles the stale-token refusal",
            case: Case::Drive(drive_key_run),
        },
        Row {
            id: "NativeApi.leaveRun",
            why: "one leave session settles the invalid-site refusal",
            case: Case::Drive(drive_leave_run),
        },
        read_row(
            "NativeApi.lineOfSight",
            "open collision flags allow line of sight and malformed input is refused",
            LINE_OF_SIGHT_SETUP,
            "return [api.lineOfSight({from:{x:3201,z:3201,level:0},to:{x:3203,z:3201,level:0},size:1}),api.lineOfSight(null)];",
            json!([{"ok":true,"value":true},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.plannedPotions",
            "empty carry uses the two declared defaults and a malformed carry is refused",
            PLAIN,
            "return [api.plannedPotions({carry:[]}),api.plannedPotions({carry:'bad'})];",
            json!([
                {"ok":true,"value":[{"skill":"attack","short":"Att","flask":"Super attack(3)","doses":["Super attack(4)","Super attack(3)","Super attack(2)","Super attack(1)"],"want":1},{"skill":"strength","short":"Str","flask":"Super strength(3)","doses":["Super strength(4)","Super strength(3)","Super strength(2)","Super strength(1)"],"want":1}]},
                {"ok":false,"error":"invalid-args"}
            ]),
        ),
        read_row(
            "NativeApi.potionToSip",
            "an empty plan has no sip target and missing arrays are refused",
            PLAIN,
            "return [api.potionToSip({plans:[],held:[],levels:[]}),api.potionToSip({held:[],levels:[]})];",
            json!([{"ok":true,"value":null},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.prayerPoints",
            "posted prayer level supplies current points",
            PRAYER_STATS_SETUP,
            "return api.prayerPoints();",
            json!({"ok":true,"value":43}),
        ),
        read_row(
            "NativeApi.prayerMax",
            "posted prayer base level supplies the maximum",
            PRAYER_STATS_SETUP,
            "return api.prayerMax();",
            json!({"ok":true,"value":43}),
        ),
        read_row(
            "NativeApi.prayerFull",
            "equal current and maximum points report full",
            PRAYER_STATS_SETUP,
            "return api.prayerFull();",
            json!({"ok":true,"value":true}),
        ),
        read_row(
            "NativeApi.prayerKnown",
            "selected prayer facts recognize a known name and return false for an unknown name",
            PRAYER_SETUP,
            "return [api.prayerKnown({name:'Protect from Melee'}),api.prayerKnown({name:'Nope'}),api.prayerKnown({name:7})];",
            json!([{"ok":true,"value":true},{"ok":true,"value":false},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.prayerAvailable",
            "the posted level makes Protect from Melee available and malformed input is refused",
            PRAYER_SETUP,
            "return [api.prayerAvailable({name:'Protect from Melee'}),api.prayerAvailable(null)];",
            json!([{"ok":true,"value":true},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.prayerActive",
            "the posted inactive varp reports false and malformed input is refused",
            PRAYER_SETUP,
            "return [api.prayerActive({name:'Protect from Melee'}),api.prayerActive(null)];",
            json!([{"ok":true,"value":false},{"ok":false,"error":"invalid-args"}]),
        ),
        Row {
            id: "NativeApi.prayerSet",
            why: "unknown prayer settles the async helper refusal without a click",
            case: Case::GameDataDrive(drive_prayer_set),
        },
        Row {
            id: "NativeApi.prayerClear",
            why: "two posted active prayers each emit a clear click and settle inactive",
            case: Case::GameDataDrive(drive_prayer_clear),
        },
        read_row(
            "NativeApi.questIdentity",
            "Death Plateau identity returns the pinned row and an unknown id is refused",
            CONTENT,
            "return [api.questIdentity({id:'death'}),api.questIdentity({id:'not-a-quest'})];",
            json!([
                {"ok":true,"value":{"id":"death","component":"death","display":"Death Plateau","varp":"death_equiproom","varp_id":314,"complete":80,"quest_points":1,"unknown_sides":[],"requirements":{"qualification":"partial","skills":[],"items":[],"empty_must_have":true,"unknown_as_satisfied":false}}},
                {"ok":false,"error":"unknown-quest"}
            ]),
        ),
        Row {
            id: "NativeApi.questJournalBegin",
            why: "a posted quest row admits a token and a blank name is refused",
            case: Case::Drive(drive_quest_journal_begin),
        },
        read_row(
            "NativeApi.questPrereqs",
            "Death Plateau requirements return the pinned requirements object",
            CONTENT,
            "return [api.questPrereqs({id:'death'}),api.questPrereqs({id:'not-a-quest'})];",
            json!([
                {"ok":true,"value":{"qualification":"partial","skills":[],"items":[],"empty_must_have":true,"unknown_as_satisfied":false}},
                {"ok":false,"error":"unknown-quest"}
            ]),
        ),
        Row {
            id: "NativeApi.questPaths",
            why: "the released Path index returns the authored complete page",
            case: Case::Drive(drive_quest_paths),
        },
        Row {
            id: "NativeApi.questStatus",
            why: "an unbound quest tab is distinct from a quest miss",
            case: Case::Drive(drive_quest_status),
        },
        Row {
            id: "NativeApi.questJournalRun",
            why: "one journal run clicks the posted quest and closes the rendered journal",
            case: Case::Drive(drive_quest_journal_run),
        },
        Row {
            id: "NativeApi.questProgress",
            why: "one progress read sends its request and settles the posted terminal",
            case: Case::Drive(drive_quest_progress),
        },
        Row {
            id: "NativeApi.rangeLoadoutOf",
            why: "a selected dart becomes the projectile and malformed input is refused",
            case: Case::GameDataDrive(drive_range_loadout),
        },
        read_row(
            "NativeApi.runesPerCast",
            "an air staff removes the Air rune and an unknown spell returns null",
            CONTENT,
            "return [api.runesPerCast({spellName:'Wind Strike',wielded:['Staff of air']}),api.runesPerCast({spellName:'Not a spell',wielded:[]}),api.runesPerCast({spellName:'Wind Strike'})];",
            json!([{"ok":true,"value":[{"rune":"Mind rune","count":1}]},{"ok":true,"value":null},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.retreatBegin",
            "a valid site allocates a token and a missing site is refused",
            PLAIN,
            "return [api.retreatBegin({key:'conformance',boxes:[{minX:0,maxX:10,minZ:0,maxZ:10,level:0}]}),api.retreatBegin(null)];",
            json!([{"ok":true,"value":{"token":1}},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.retreatValidate",
            "an unknown retreat token is a valid false validation and malformed input is refused",
            PLAIN,
            "return [api.retreatValidate({token:999},{}),api.retreatValidate(null,{})];",
            json!([{"ok":true,"value":false},{"ok":false,"error":"invalid-args"}]),
        ),
        Row {
            id: "NativeApi.retreatRun",
            why: "retreat without a posted tile settles without movement",
            case: Case::Drive(drive_retreat_run),
        },
        Row {
            id: "NativeApi.sceneLocs",
            why: "valid scene arguments fail closed without a collision observation",
            case: Case::Drive(drive_scene_locs),
        },
        Row {
            id: "NativeApi.sceneNpcs",
            why: "valid scene arguments fail closed without a collision observation",
            case: Case::Drive(drive_scene_npcs),
        },
        read_row(
            "NativeApi.suppliesOf",
            "carried supplies retain their exact quantity and invalid quantities are refused",
            PLAIN,
            "return [api.suppliesOf({loadout:{carry:[{item:'Coins'},{item:'Lobster',qty:10}]}}),api.suppliesOf({loadout:{carry:[{item:'Coins',qty:0}]}})];",
            json!([{"ok":true,"value":[{"item":"Coins","qty":1},{"item":"Lobster","qty":10}]},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.walkable",
            "the posted walkable bit is true and an off-plane tile is false",
            MOVEMENT_SETUP,
            "return [api.walkable({tile:{x:3200,z:3200,level:0}}),api.walkable({tile:{x:3200,z:3200,level:1}}),api.walkable(null)];",
            json!([{"ok":true,"value":true},{"ok":true,"value":false},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.weaponOf",
            "the worn right-hand item wins, a blank weapon uses its fallback, and a null loadout returns null",
            PLAIN,
            "return [api.weaponOf({loadout:{worn:{righthand:'Rune scimitar'}},fallback:null}),api.weaponOf({loadout:{worn:{righthand:'  '}},fallback:'Bronze sword'}),api.weaponOf({loadout:null}),api.weaponOf({loadout:7})];",
            json!([{"ok":true,"value":"Rune scimitar"},{"ok":true,"value":"Bronze sword"},{"ok":true,"value":null},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.walkspotBegin",
            "a valid site allocates a token and a missing site is refused",
            PLAIN,
            "return [api.walkspotBegin({key:'conformance',boxes:[{minX:0,maxX:10,minZ:0,maxZ:10,level:0}]}),api.walkspotBegin(null)];",
            json!([{"ok":true,"value":{"token":1}},{"ok":false,"error":"invalid-args"}]),
        ),
        read_row(
            "NativeApi.walkspotValidate",
            "an unknown walkspot token is a valid false validation and malformed input is refused",
            PLAIN,
            "return [api.walkspotValidate({token:999},{}),api.walkspotValidate(null,{})];",
            json!([{"ok":true,"value":false},{"ok":false,"error":"invalid-args"}]),
        ),
        Row {
            id: "NativeApi.walkspotRun",
            why: "one walkspot task settles its stale-token refusal",
            case: Case::Drive(drive_walkspot_run),
        },
    ]
}
fn rows() -> Vec<Row> {
    let mut rows = vec![
        read_row(
            "NativeApi.tick",
            "the host game-tick counter starts at one for its first posted tick",
            PLAIN,
            "return api.tick;",
            json!(1),
        ),
        read_row(
            "NativeApi.settings.str",
            "posted string values win while missing keys use the declared fallback",
            SETTINGS_SETUP,
            "return [api.settings.str('name','x'),api.settings.str('absent','fb')];",
            json!(["Bones", "fb"]),
        ),
        read_row(
            "NativeApi.settings.num",
            "posted fractional numbers are retained while missing keys use fallback",
            SETTINGS_SETUP,
            "return [api.settings.num('count',0),api.settings.num('absent',9)];",
            json!([7.5, 9]),
        ),
        read_row(
            "NativeApi.settings.bool",
            "posted booleans win while missing keys use fallback",
            SETTINGS_SETUP,
            "return [api.settings.bool('on',false),api.settings.bool('absent',true)];",
            json!([true, true]),
        ),
        Row {
            id: "NativeApi.settings.list",
            why: "list reads normalized legacy and current values, rejects wrong shapes, uses fallbacks, and returns detached copies after replacement",
            case: Case::Drive(drive_settings_list),
        },
        Row {
            id: "NativeApi.settings.tile",
            why: "tile reads current values, rejects wrong shapes, uses fallbacks, and returns detached copies after replacement",
            case: Case::Drive(drive_settings_tile),
        },
        Row {
            id: "NativeApi.log",
            why: "log lines reach the host log channel with no gameplay request",
            case: Case::Drive(drive_log),
        },
        Row {
            id: "NativeApi.stop",
            why: "stop reason is recorded in the host's stop receipt",
            case: Case::Drive(drive_stop),
        },
        Row {
            id: "NativeApi.paint.begin",
            why: "begin creates a local paint frame without wire traffic",
            case: Case::Drive(drive_paint_begin),
        },
        Row {
            id: "NativeApi.paint.begin().title",
            why: "title appends the exact heading and returns the same frame",
            case: Case::Drive(drive_paint_title),
        },
        Row {
            id: "NativeApi.paint.begin().row",
            why: "row joins its columns into one paint line and returns the same frame",
            case: Case::Drive(drive_paint_row),
        },
        Row {
            id: "NativeApi.paint.begin().gap",
            why: "gap appends one empty line and returns the same frame",
            case: Case::Drive(drive_paint_gap),
        },
        Row {
            id: "NativeApi.paint.begin().end",
            why: "end publishes the recorded local frame and returns undefined",
            case: Case::Drive(drive_paint_end),
        },
        send_row(
            "NativeApi.request[op=close]",
            "the bare close op encodes one close request",
            "api.request({op:'close'});",
            vec![InteractReq::Close],
        ),
    ];
    rows.extend(snapshot_rows());
    rows.extend(request_rows());
    rows.extend(helper_rows());
    rows
}

/// Authored op field sets; never derived from the implementation.
const OP_FIELDS: &[(&str, &[&str])] = &[
    ("held", &["name", "action"]),
    ("open-booth", &["x", "z", "level", "id", "name?", "action?"]),
    (
        "open-stand",
        &["x", "z", "level", "kind", "name?", "stand_op?", "choose?"],
    ),
    ("close", &[]),
    ("set-note-mode", &["on"]),
    ("deposit", &["name"]),
    ("withdraw", &["name", "action"]),
    (
        "withdraw-x",
        &[
            "name",
            "count",
            "bank_item_id",
            "lands_as_id",
            "action",
            "bank_generation",
        ],
    ),
    ("puzzle-move", &["id", "slot", "component", "generation"]),
    (
        "walk",
        &[
            "x",
            "z",
            "level",
            "allow_teleports?",
            "allow_wilderness?",
            "allow_bank_fetch?",
            "request_id?",
        ],
    ),
    (
        "walk-near",
        &[
            "x",
            "z",
            "level",
            "radius",
            "allow_teleports?",
            "allow_wilderness?",
            "allow_bank_fetch?",
            "request_id?",
        ],
    ),
    ("walk-nearest-bank", &[]),
    (
        "inspect-route",
        &[
            "from",
            "to",
            "allow_teleports?",
            "allow_wilderness?",
            "allow_bank_fetch?",
            "avoid?",
            "request_id?",
            "max_expansions?",
        ],
    ),
];

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[test]
fn every_public_member_has_exactly_one_row() {
    let errors = coverage_errors(&inventory(&generated_dts()), &rows(), OP_FIELDS);
    assert!(
        errors.is_empty(),
        "conformance coverage:\n{}",
        errors.join("\n")
    );
}

#[test]
fn removing_a_row_fails_with_its_exact_id() {
    let mut rows = rows();
    let removed = rows.remove(0).id;
    let errors = coverage_errors(&inventory(&generated_dts()), &rows, OP_FIELDS);
    assert!(
        errors.contains(&format!("missing row {removed}")),
        "{errors:?}"
    );
}

#[test]
fn a_new_generated_member_without_a_row_fails() {
    let dts = generated_dts().replacen(
        "export interface NativeApi {\n",
        "export interface NativeApi {\n  conformanceTempMember(): void;\n",
        1,
    );
    let errors = coverage_errors(&inventory(&dts), &rows(), OP_FIELDS);
    assert!(
        errors.contains(&"missing row NativeApi.conformanceTempMember".to_owned()),
        "{errors:?}"
    );
}

#[test]
fn a_new_op_field_without_an_authored_field_fails() {
    // Mutate the `walk` variant of `NativeOp` only; the compat `InteractReq`
    // union earlier in the file also has a `walk` op.
    let generated = generated_dts();
    let union = generated
        .find("export type NativeOp =")
        .expect("generated declarations define NativeOp");
    let (head, tail) = generated.split_at(union);
    let dts = format!(
        "{head}{}",
        tail.replacen(
            "{ op: 'walk';",
            "{ op: 'walk'; conformanceProbe?: number;",
            1
        )
    );
    let errors = coverage_errors(&inventory(&dts), &rows(), OP_FIELDS);
    assert!(
        errors.contains(
            &"missing field NativeApi.request[op=walk].fields.conformanceProbe?".to_owned()
        ),
        "{errors:?}"
    );
}

#[test]
fn runtime_api_keys_match_the_declared_top_level_members() {
    let declared: BTreeSet<String> = inventory(&generated_dts())
        .members
        .iter()
        .filter_map(|path| path.strip_prefix("NativeApi."))
        .map(|rest| {
            rest.split(['.', '[', '#', '('])
                .next()
                .expect("a member path has a first segment")
                .to_owned()
        })
        .collect();
    let observed = observe(
        &PLAIN,
        "const keys = new Set(); \
         for (let o = api; o && o !== Object.prototype; o = Object.getPrototypeOf(o)) \
           for (const k of Object.getOwnPropertyNames(o)) if (k !== 'constructor') keys.add(k); \
         return [...keys].sort();",
    );
    let runtime: BTreeSet<String> = observed
        .value
        .as_array()
        .expect("runtime keys")
        .iter()
        .map(|key| key.as_str().expect("key text").to_owned())
        .collect();
    let undeclared: Vec<_> = runtime.difference(&declared).collect();
    let unimplemented: Vec<_> = declared.difference(&runtime).collect();
    assert!(
        undeclared.is_empty() && unimplemented.is_empty(),
        "runtime members without a declaration: {undeclared:?}; declared members missing at runtime: {unimplemented:?}"
    );
}

#[test]
fn every_row_runs_through_the_real_shim_or_reports_a_data_skip() {
    let inventory = inventory(&generated_dts());
    let rows = rows();
    let errors = coverage_errors(&inventory, &rows, OP_FIELDS);
    assert!(
        errors.is_empty(),
        "conformance coverage:\n{}",
        errors.join("\n")
    );
    let needs_selected_data = rows.iter().any(row_requires_selected_game_data);
    let selected_data_available = !needs_selected_data || api::test_support::game_data_available();
    let mut failures = Vec::new();
    for row in rows {
        if row_requires_selected_game_data(&row) && !selected_data_available {
            eprintln!(
                "SKIP {}: required local game-data assets are unavailable; \
                 GITHUB_ACTIONS=true does not download them",
                row.id
            );
            continue;
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_row(&row)));
        if result.is_err() {
            failures.push(row.id);
        }
    }
    assert!(failures.is_empty(), "conformance rows failed: {failures:?}");
}

#[test]
fn changing_walk_x_in_the_wire_expectation_fails_the_comparison() {
    let row = request_rows()
        .into_iter()
        .find(|row| row.id == "NativeApi.request[op=walk]")
        .expect("authored walk request row");
    let Case::Send {
        setup,
        body,
        mut expect,
    } = row.case
    else {
        panic!("walk row sends an interact request");
    };
    let InteractReq::Walk { x, .. } = &mut expect[0] else {
        panic!("walk expectation uses the walk request variant");
    };
    *x += 1;

    let observed = observe(&setup, body);
    let mismatch = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_eq!(observed.requests, expect);
    }));
    assert!(mismatch.is_err(), "a changed walk x unexpectedly matched");
}
