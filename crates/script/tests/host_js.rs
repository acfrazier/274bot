// Host JS living index.d.ts — our verbs, not rs2b0t names.
use script::host_js::{host_js_path, render_host_js_dts, write_host_js_dts};

#[test]
fn host_js_dts_is_fresh() {
    let path = host_js_path();
    let on_disk =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let rendered = render_host_js_dts();
    assert_eq!(
        on_disk, rendered,
        "host-js/index.d.ts is stale; run: cargo test -p script --test host_js regen_host_js -- --ignored"
    );
}

#[test]
fn host_js_dts_includes_required_interfaces() {
    let src = render_host_js_dts();
    assert!(src.contains("export interface FindOptions"));
    assert!(src.contains("allow_wilderness"));
    assert!(
        src.contains("wilderness"),
        "FindOptions JSDoc must mention wilderness exclusion"
    );
    assert!(src.contains("export interface Camera"));
    assert!(src.contains("orbit_yaw"));
    assert!(src.contains("export interface ShopStockRow"));
    assert!(src.contains("export interface ReachQueryView"));
    assert!(!src.contains("exact_rank: number[]"));
    assert!(!src.contains("adjacent_rank: number[]"));
    assert!(src.contains("reach: ReachQueryView"));
    assert!(src.contains("walkable(input: { tile: WorldTile } | WorldTile): HelperResult<boolean>"));
    assert!(
        src.contains("canStep(input: { from: WorldTile; to: WorldTile }): HelperResult<boolean>")
    );
    assert!(src.contains(
        "canReach(input: { tile: WorldTile; adjacentOk?: boolean; maxSteps?: number }): HelperResult<boolean>"
    ));
    assert!(src.contains("export interface CollisionFlagsView"));
    assert!(src.contains("export interface CollisionView"));
    assert!(src.contains("collision: CollisionView"));
    assert!(src.contains(
        "lineOfSight(input: { from: WorldTile; to: WorldTile; size?: number }): HelperResult<boolean>"
    ));
    // Hunt families are begin + await with typed inputs: no effect
    // stepping and no `Record<string, unknown>` projection bags.
    assert!(src.contains("fightBegin(site: HuntSite): HelperResult<{ token: number }>"));
    assert!(
        src.contains("fightRun(input: HuntToken, hooks?: HuntHooks): Promise<HuntOutcome<null>>")
    );
    assert!(src.contains(
        "bankRun(site: HuntSite, opts?: HuntBankOptions, hooks?: HuntHooks): Promise<HuntOutcome<boolean>>"
    ));
    assert!(src.contains("export interface HuntSite {"));
    assert!(src.contains("export interface HuntHooks {"));
    assert!(interface_block(&src, "HuntHooks").contains("inArea?(tile: WorldTile): boolean;"));
    for gone in [
        "fightNext",
        "bankNext",
        "cellNext",
        "Record<string, unknown>",
        "FightStep",
        "BankStep",
    ] {
        assert!(!src.contains(gone), "hunt effect stepping is gone: {gone}");
    }
    assert!(src.contains("export interface QuestStatusRow"));
    assert!(src.contains("quest_statuses: QuestStatusRow[] | null"));
    assert!(src.contains("slot?: number"));
    assert!(src.contains("source_item_id?: number | null"));
    assert!(src.contains("source_item_slot?: number | null"));
    assert!(src.contains("target_item_id?: number | null"));
    assert!(src.contains("target_item_slot?: number | null"));
    assert!(
        !src.contains("Game.teleport"),
        "must not export Game.teleport"
    );
    assert!(
        !src.contains("teleport("),
        "must not export teleport string-table helper"
    );
    assert!(src.contains("export interface NativeApi"));
    assert!(src.contains("export type HelperResult"));
    assert!(src.contains("export interface PrayerClearCounts"));
    assert!(src.contains("prayerPoints(): HelperResult<number>"));
    assert!(src.contains("foodCount(input: { items: ItemRow[]; foodName: string })"));
    assert!(src.contains("escapeRunesFor(input: { id: string })"));
    assert!(src.contains(
        "questIdentity(input: { name: string } | { id: string }): HelperResult<QuestIdentityRow>"
    ));
    assert!(src.contains(
        "questPrereqs(input: { id: string; name?: string }): HelperResult<QuestRequirements>"
    ));
    assert!(!src.contains("questIdentity(input: { name: string } | { id: string }): Promise"));
    assert!(!src.contains("questPrereqs(input: { id: string; name?: string }): Promise"));
    assert!(src.contains("export interface ClueRow {"));
    assert!(src.contains("params: Array<{ key: string; value: string }>;"));
    assert!(src.contains("access?: 'constrained';"));
    assert!(src.contains("export interface PackPlanInput {"));
    assert!(src.contains("  rewardSlots?: number;"));
    assert!(!src.contains("clue: { row(input: { id: number } | { alias: string }): Promise"));
    assert!(!src.contains("heldStep(input"));
    assert!(!src.contains("heldStep(): Promise"));
    assert!(!src.contains("packPlan(input: PackPlanInput): Promise"));
    assert!(!src.contains("hardKit(input: { attack: number; lostCity: boolean; items: { id: number; count: number }[] }): Promise"));
    // Hard-kit is a status, not a kit read: no snapshot or inventory page.
    assert!(!src.contains("hardKit(input: { includeBank"));
    assert!(!src.contains("keep(input: { name: string; extra?: string[] }): Promise"));
    // Keep is a predicate, not a bank or inventory read.
    assert!(!src.contains("keep(input: { name: string; extra?: string[]; bank"));
    assert!(!src.contains("keep(input: { name: string; items"));
    assert!(!src.contains("clueRow("));
    assert!(src.contains(
        "sceneLocs(input: { ids: number[]; limit: number; region?: SceneRegionInput }): HelperResult<SceneProjection>"
    ));
    assert!(src.contains(
        "sceneNpcs(input: { types: number[]; actions: string[]; limit: number; region?: SceneRegionInput }): HelperResult<SceneProjection>"
    ));
    assert!(!src.contains(
        "sceneLocs(input: { ids: number[]; limit: number; region?: SceneRegionInput }): Promise"
    ));
    assert!(!src.contains(
        "sceneNpcs(input: { types: number[]; actions: string[]; limit: number; region?: SceneRegionInput }): Promise"
    ));
    assert!(src.contains("export interface SceneRegionInput"));
    assert!(src.contains("export interface SceneBounds"));
    assert!(src.contains("export interface SceneProjectionRow"));
    assert!(src.contains("export interface SceneProjection {"));
    assert!(src.contains("as_of_sequence: number;"));
    assert!(src.contains("truncated: boolean;"));
    assert!(src.contains(
        "questStatus(input: { name: string }): HelperResult<{ status: 'notStarted' | 'inProgress' | 'complete' | 'unknown'; as_of_sequence: number }>"
    ));
    assert!(!src.contains("questStatus(input: { name: string }): Promise"));
    assert!(!src.contains("questStatus(input: { name: string }): HelperResult<QuestStatusRow>"));
    assert!(src.contains("export interface LoadoutInput"));
    assert!(src.contains("export interface PotionPlan"));
    assert!(src.contains("foodOf(input: { loadout: LoadoutInput | null; fallback: string })"));
    assert!(src.contains("potionToSip(input: { plans: PotionPlan[]; held: number[]; levels: Array<{ skill: string; base: number; effective: number }> })"));
    assert!(src.contains(
        "prayerSet(input: { name: string; on: boolean }): Promise<HelperResult<boolean>>"
    ));
    assert!(src.contains("prayerClear(): Promise<HelperResult<PrayerClearCounts>>"));
    assert!(
        !src.contains("prayerSetBegin"),
        "named async Set/Clear is the private-lifecycle exception; no Begin/Next"
    );
    assert!(src.contains("export type NativeOp"));
    assert!(src.contains("op: 'walk-nearest-bank'"));
    assert!(src.contains("{ op: 'deposit'; name: string}"));
    assert!(src.contains("op: 'walk'"));
    assert!(src.contains("op: 'walk-near'"));
    assert!(
        !src.contains("export type NativeOp =\n  | { op: 'walk'"),
        "v2 NativeOp must not dump the full InteractReq union"
    );

    let native = native_snapshot_block(&src);
    assert!(
        native.contains("npcs: SceneEntity[]"),
        "typed NativeSnapshot.npcs required"
    );
    assert!(native.contains("self_target_kind: number"));
    assert!(native.contains("self_target_index: number"));
    assert!(
        native.contains("Packet-time NPC_INFO"),
        "npcs must document packet-time freshness"
    );
    assert!(
        native.contains("size<1 is unavailable"),
        "npcs must document unavailable size"
    );
    assert!(
        !native.contains("locs:") && !native.contains("players:") && !native.contains("ground:"),
        "locs/players/ground stay off NativeSnapshot: {native}"
    );
}

fn native_snapshot_block(src: &str) -> &str {
    interface_block(src, "NativeSnapshot")
}

/// One `export interface X { … }` block from the rendered declarations.
fn interface_block<'a>(src: &'a str, name: &str) -> &'a str {
    let start = src
        .find(&format!("export interface {name} {{"))
        .unwrap_or_else(|| panic!("missing {name}"));
    let rest = &src[start..];
    let end = rest
        .find("\n}\n")
        .unwrap_or_else(|| panic!("unclosed {name}"));
    &rest[..=end]
}

/// One `export type X = …;` block from the rendered declarations.
fn type_block<'a>(src: &'a str, name: &str) -> &'a str {
    let start = src
        .find(&format!("export type {name} ="))
        .unwrap_or_else(|| panic!("missing {name}"));
    let rest = &src[start..];
    let end = rest
        .find(";\n")
        .unwrap_or_else(|| panic!("unclosed {name}"));
    &rest[..end]
}

/// `HostHandle.interact` is the queue `Bank`/`Banking` push onto: the bank
/// shim queues `walk-nearest-bank` to reach a stand and `withdraw-load` to
/// fill the pack (`bank.js`, `periodic_bank.js`). A union without them types
/// a queue narrower than the one the isolate drains. `inspect-ack` stays off
/// it: that ack is the isolate→host reply, never a script request.
#[test]
fn interact_union_publishes_the_banked_queue_ops() {
    let src = render_host_js_dts();
    let union = type_block(&src, "InteractReq");
    assert!(union.contains("| { op: 'walk-nearest-bank'}"), "{union}");
    assert!(
        union.contains("| { op: 'withdraw-load'; name: string; bank_generation: number}"),
        "{union}"
    );
    assert!(
        !union.contains("inspect-ack"),
        "the isolate consume-ack stays off the published queue: {union}"
    );
}

/// The declared MakeButton key is the posted one: `make_product_array` posts
/// `comId` on each button and the ClientAdapter reader maps `b.comId`. A
/// `com_id` declaration types a field no posted row carries.
#[test]
fn make_button_publishes_the_posted_component_key() {
    let src = render_host_js_dts();
    let button = interface_block(&src, "MakeButton");
    assert!(button.contains("comId: number;"), "{button}");
    assert!(!button.contains("com_id"), "{button}");
}

/// Gather-only consumer compile gate (pinned TypeScript 5.8.3): a temporary
/// consumer exercises every slice-A member positively and pins one
/// `@ts-expect-error` invalid-settings case. The full sample/quest consumer
/// belongs to slice C, where its dependencies exist.
#[test]
#[ignore = "requires npx and TypeScript 5.8.3"]
fn tsc_gather_consumer_uses_every_slice_a_member() {
    use std::process::Command;

    let dir = std::env::temp_dir().join(format!("host-js-gather-consumer-{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("clear gather consumer dir");
    }
    std::fs::create_dir_all(&dir).expect("create gather consumer dir");
    std::fs::write(dir.join("gather.d.ts"), render_host_js_dts())
        .expect("write gather declarations");
    std::fs::write(dir.join("consumer.ts"), GATHER_CONSUMER).expect("write gather consumer");
    let output = Command::new("npx")
        .args(["-p", "typescript@5.8.3", "--yes", "tsc"])
        .args([
            "--noEmit",
            "--strict",
            "--target",
            "ES2022",
            "--module",
            "ESNext",
            "--moduleResolution",
            "Bundler",
            "--skipLibCheck",
            "false",
        ])
        .arg(dir.join("consumer.ts"))
        .output()
        .unwrap_or_else(|e| panic!("tsc gather consumer failed to spawn: {e}"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    std::fs::remove_dir_all(&dir).expect("remove gather consumer dir");
    assert!(
        output.status.success(),
        "gather consumer failed:\n{stdout}\n{stderr}"
    );
}

/// Temporary slice-A consumer fixture: uses `api.gather.run/stop`,
/// `api.snapshot.gather`, and every session/status/outcome type, plus one
/// rejected invalid-settings row.
const GATHER_CONSUMER: &str = r#"
import type {
  GatherCounts,
  GatherEnd,
  GatherFailure,
  GatherOutcome,
  GatherSession,
  GatherSettings,
  GatherStatus,
  NativeApi,
} from "./gather";

declare const api: NativeApi;

async function drive(): Promise<GatherOutcome> {
  const full: GatherSettings = {
    skill: 'Mining',
    woodcuttingResources: ['normal'],
    miningResources: ['copper', 'tin'],
    fishingMethod: 'fishing.saltfish.op1',
    targetPreference: 'Nearest',
    location: 'Custom',
    customTile: { x: 1, z: 2, level: 0 },
    radius: 12,
    disposition: 'Power',
    allowTeleports: false,
    allowWilderness: false,
    deathPolicy: 'Stop',
    maxDeaths: 2,
  };
  const outcome = await api.gather.run(full);
  const defaults = await api.gather.run();
  void defaults;
  const stopped = api.gather.stop();
  if (!stopped.ok) {
    throw new Error(stopped.error);
  }
  const session: GatherSession | null = api.snapshot.gather;
  const status: GatherStatus | null = session?.status ?? null;
  if (status !== null) {
    const text: string[] = [
      status.skill, status.method, status.phase, status.area, status.target,
      status.tool, status.bank, status.excluded_targets, status.last_event,
    ];
    const numbers: number[] = [
      status.bait, status.food, status.coins, status.yielded, status.dropped,
      status.deposited, status.trips, status.xp, status.xp_per_hour,
      status.last_progress, status.deaths, status.absent, status.zone_gated,
    ];
    void text;
    void numbers;
  }
  return outcome;
}

function narrow(outcome: GatherOutcome): string {
  if (outcome.kind === 'done') {
    const end: GatherEnd = outcome.value;
    if (end.end === 'blocked') {
      const failure: GatherFailure = end.failure;
      const counts: GatherCounts = end.counts;
      return `${failure.code}:${counts.yielded}`;
    }
    return end.end;
  }
  return outcome.reason;
}

void drive;
void narrow;

// @ts-expect-error radius is a number, not a string
const invalid: GatherSettings = { radius: 'wide' };
void invalid;
"#;

/// Quest progress consumer compile gate (pinned TypeScript 5.8.3): a
/// temporary consumer exercises both slice-B calls and every row field
/// positively and pins one `@ts-expect-error` non-object case.
#[test]
#[ignore = "requires npx and TypeScript 5.8.3"]
fn tsc_quest_progress_consumer_uses_both_calls() {
    use std::process::Command;

    let dir =
        std::env::temp_dir().join(format!("host-js-progress-consumer-{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("clear progress consumer dir");
    }
    std::fs::create_dir_all(&dir).expect("create progress consumer dir");
    std::fs::write(dir.join("quest_progress.d.ts"), render_host_js_dts())
        .expect("write progress declarations");
    std::fs::write(dir.join("consumer.ts"), QUEST_PROGRESS_CONSUMER)
        .expect("write progress consumer");
    let output = Command::new("npx")
        .args(["-p", "typescript@5.8.3", "--yes", "tsc"])
        .args([
            "--noEmit",
            "--strict",
            "--target",
            "ES2022",
            "--module",
            "ESNext",
            "--moduleResolution",
            "Bundler",
            "--skipLibCheck",
            "false",
        ])
        .arg(dir.join("consumer.ts"))
        .output()
        .unwrap_or_else(|e| panic!("tsc progress consumer failed to spawn: {e}"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    std::fs::remove_dir_all(&dir).expect("remove progress consumer dir");
    assert!(
        output.status.success(),
        "progress consumer failed:\n{stdout}\n{stderr}"
    );
}

/// Temporary slice-B consumer fixture: uses `api.questPaths`,
/// `api.questProgress`, and every path/row field, plus one rejected
/// non-object row.
const QUEST_PROGRESS_CONSUMER: &str = r#"
import type {
  Known,
  NativeApi,
  QuestPathRow,
  QuestProgressEnd,
  QuestProgressOutcome,
  QuestProgressRow,
  Truth3,
} from "./quest_progress";

declare const api: NativeApi;

function paths(): QuestPathRow[] | null {
  const out = api.questPaths();
  if (!out.ok) {
    const error: string = out.error;
    void error;
    return null;
  }
  for (const row of out.value.rows) {
    const id: string = row.id;
    const display: string = row.display;
    const journal: boolean = row.journal;
    const stages: string[] = row.stages;
    void [id, display, journal, stages];
  }
  return out.value.rows;
}

async function progress(quest: string): Promise<QuestProgressRow | null> {
  const outcome: QuestProgressOutcome = await api.questProgress({ quest });
  if (outcome.kind === 'done') {
    const end: QuestProgressEnd = outcome.value;
    if (end.end === 'done') {
      const row: QuestProgressRow = end.row;
      const questId: string = row.quest;
      const display: string = row.display;
      const colour: 'notStarted' | 'inProgress' | 'complete' | 'unknown' = row.colour;
      const stage: Known<string> = row.stage;
      const stageText: string = stage.state === 'known' ? stage.value : stage.gap;
      const complete: Truth3 = row.complete;
      const rule: Known<string> = row.rule;
      const ruleText: string = rule.state === 'known' ? rule.value : rule.gap;
      const exhaustive: Truth3 = complete === 'true' ? 'false' : complete;
      for (const flag of row.flags) {
        const name: string = flag.flag;
        const truth: Truth3 = flag.truth;
        const count: number | null = flag.count;
        void [name, truth, count];
      }
      const run: number = row.evidence.run;
      const session: number = row.evidence.session;
      const tick: number = row.evidence.tick;
      const sequence: number = row.evidence.sequence;
      const journalRead: boolean = row.journal_read;
      const binding: string = row.binding;
      const role: string | null = row.role;
      void [questId, display, colour, stageText, ruleText, exhaustive, run, session, tick, sequence, journalRead, binding, role];
      return row;
    }
    const reason: string = end.reason;
    void reason;
    return null;
  }
  if (outcome.kind === 'refused') {
    const reason: string = outcome.reason;
    void reason;
    return null;
  }
  const aborted: 'reset' | 'superseded' | 'terminated' | 'unknown' = outcome.reason;
  void aborted;
  return null;
}

void paths();
for (const quest of ['cook', 'sheep', 'runemysteries', 'romeojuliet']) {
  void progress(quest);
}

// @ts-expect-error questProgress takes an object, not a bare string
void api.questProgress('cook');
"#;

/// Sample gate (pinned TypeScript 5.8.3): the authoritative
/// `examples/gather_quest_v2.ts` must type-check with and without `--strict`.
/// Its checked-in `.js` counterpart must match the compiler's emitted output.
#[test]
#[ignore = "requires npx and TypeScript 5.8.3"]
fn tsc_gather_quest_sample_type_checks() {
    use std::process::Command;

    let dir = std::env::temp_dir().join(format!(
        "host-js-gather-quest-sample-{}",
        std::process::id()
    ));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("clear gather-quest sample dir");
    }
    // Preserve the sample's relative `../host-js/index.d.ts` import.
    let host_js = dir.join("host-js");
    let examples = dir.join("examples");
    std::fs::create_dir_all(&host_js).expect("create sample host-js dir");
    std::fs::create_dir_all(&examples).expect("create sample examples dir");
    std::fs::write(host_js.join("index.d.ts"), render_host_js_dts())
        .expect("write sample declarations");
    std::fs::write(
        examples.join("gather_quest_v2.ts"),
        include_str!("../examples/gather_quest_v2.ts"),
    )
    .expect("write gather-quest sample");
    let common_flags = [
        "--target",
        "ES2022",
        "--module",
        "ESNext",
        "--moduleResolution",
        "Bundler",
        "--skipLibCheck",
        "false",
    ];
    let sample = examples.join("gather_quest_v2.ts");
    let strict_output = Command::new("npx")
        .args(["-p", "typescript@5.8.3", "--yes", "tsc"])
        .args(common_flags)
        .args(["--noEmit", "--strict"])
        .arg(&sample)
        .output()
        .unwrap_or_else(|e| panic!("strict tsc gather-quest sample failed to spawn: {e}"));
    assert!(
        strict_output.status.success(),
        "strict gather-quest compile failed:\n{}\n{}",
        String::from_utf8_lossy(&strict_output.stdout),
        String::from_utf8_lossy(&strict_output.stderr)
    );

    let emit_dir = dir.join("emit");
    let emitted_output = Command::new("npx")
        .args(["-p", "typescript@5.8.3", "--yes", "tsc"])
        .args(common_flags)
        .arg("--outDir")
        .arg(&emit_dir)
        .arg(&sample)
        .output()
        .unwrap_or_else(|e| panic!("tsc gather-quest sample emit failed to spawn: {e}"));
    assert!(
        emitted_output.status.success(),
        "gather-quest emit failed:\n{}\n{}",
        String::from_utf8_lossy(&emitted_output.stdout),
        String::from_utf8_lossy(&emitted_output.stderr)
    );
    let emitted = std::fs::read(emit_dir.join("gather_quest_v2.js"))
        .expect("read emitted gather-quest JavaScript");
    assert_eq!(
        emitted.as_slice(),
        include_bytes!("../examples/gather_quest_v2.js"),
        "checked-in gather_quest_v2.js must match the pinned TypeScript emit"
    );
    std::fs::remove_dir_all(&dir).expect("remove gather-quest sample dir");
}

/// Writes `host-js/index.d.ts` from the host verb tables.
#[test]
#[ignore]
fn regen_host_js() {
    write_host_js_dts().expect("write host-js/index.d.ts");
    eprintln!("wrote {}", host_js_path().display());
}
