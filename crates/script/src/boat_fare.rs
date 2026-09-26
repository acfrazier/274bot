//! Frozen Karamja boat-fare recovery and `Traversal.walkTo`, the `walk-to`
//! [`crate::machine`] family.
//!
//! Frozen `Traversal.walkTo` (`Traversal.ts:80–95`): one `WalkExecutor`
//! walk; on a `failed`/`unreachable` outcome `recoverBoatFare`
//! (`karamjaRecovery.ts`), then one more walk. The baked leg of
//! `walkResilient` is the same `Traversal.walkTo` (`Traversal.ts:160–170`),
//! so [`crate::walk::Resilient`] embeds [`Recover`] too.
//!
//! `missingBoatFare` (`karamjaRecovery.ts:16–20`): standing on Karamja
//! (`onIsland`), walking off it, fewer than 30 coins, and the failed walk's
//! only missing gate item is coins making up the fare. Frozen names the
//! shortfall (`bankPlan.ts:98–113`, `coins + missing.count === 30`); the host
//! navigator posts the edge's required count on `walk_missing_carry`
//! (`nav::router::missing_item_reqs`), so the host test is one row, coins
//! (995), count 30. The rows are only posted for a `NoPath` failure, as
//! frozen only explains an `unreachable` one (`WalkExecutor.ts:466–490`).
//!
//! The recovery is the banana-plantation job, ordinary 289 content
//! (`quest_hunt/scripts/luthas.rs2`: employment offered while
//! `%hunt_store_employed` is clear, 30 coins once `%crate_bananas = 10`):
//! walk to Luthas and `talkStrict` him; with the fare still short,
//! `fillCrate` (`piratestreasure/karamja.ts:79–113`: read the crate, pick the
//! shortfall from the grove, pack each banana, re-read) and `talkStrict`
//! again. Frozen primitives ported here: `openDialogue` / `talkChoosingBy`
//! with no rules (`primitives.ts:227–253, 276–318`), `driveChoice` /
//! `driveUntil` / `useOnLoc` / `settleScene` (`prompts.ts:30–93, 316–345`),
//! `pickBananas` (`karamja.ts:45–74`) and `searchBananaCrate` /
//! `readCrateMessages` (`crate.ts:16–60`). `openDialogue`'s
//! `Reach.entityOp` is [`NpcReach`]. `driveUntil`'s `prayerUpkeep` has no
//! host quest-prayer state and is not called; `Sustain.run` is the
//! embedding family's per-tick `sustain` pump. The host posts no chat-modal
//! text, so the strict talk's no-match line cannot quote what the NPC said.

use crate::machine::{Begin, Call, Cx, Family, Reply, Step};
use crate::observed;
use crate::reach_entity::{
    chat_mark, chat_state, npc_talkable, NpcReach, NpcReachOpts, TalkExpect,
};
use crate::shim::InteractReq;
use crate::walk::{here, interrupted, Resilient, Walk};
use api::snapshot::WorldTile;
use serde::Deserialize;
use serde_json::json;
use std::cell::Cell;
use std::collections::VecDeque;
use std::time::Duration;

/// Frozen `BOAT_FARE` (`karamjaRecovery.ts:10`).
pub(crate) const BOAT_FARE: i32 = 30;
/// Frozen `PT_ID.COINS` / `PT_ID.BANANA` (`areas.ts`).
const COINS: i32 = 995;
const BANANA: i32 = 1963;
/// Frozen `PT_LOC.BANANA_CRATE`.
const BANANA_CRATE: i32 = 2072;
/// Frozen `BANANA_TREE_IDS`: the five trees still bearing fruit.
const BANANA_TREE_IDS: [i32; 5] = [2073, 2074, 2075, 2076, 2077];
/// Frozen `CRATE_FULL` (`crate.ts:13`).
const CRATE_FULL: i32 = 10;
const LUTHAS: &str = "Luthas";
/// Frozen `LUTHAS.anchor`.
const LUTHAS_ANCHOR: WorldTile = WorldTile {
    x: 2939,
    z: 3154,
    level: 0,
};
/// Frozen `LUTHAS.prefer`.
const LUTHAS_PREFER: &[&str] = &[
    "Could you offer me employment on your plantation?",
    "Thank you, I'll be on my way",
    "No, the crate isn't full yet.",
];
/// Frozen `PT_TILE.BANANA_CRATE` / `PT_TILE.BANANA_GROVE`.
const CRATE_TILE: WorldTile = WorldTile {
    x: 2943,
    z: 3151,
    level: 0,
};
const GROVE_TILE: WorldTile = WorldTile {
    x: 2926,
    z: 3160,
    level: 0,
};
/// Frozen talk walk bound (`karamjaRecovery.ts:37`).
const TALK_WALK_MS: u64 = 120_000;
/// Frozen `walkResilient(..., { attempts: 3, timeoutMs: 180_000 })` legs.
const LEG_ATTEMPTS: u32 = 3;
const LEG_MS: u64 = 180_000;
/// Frozen `settleScene` (`prompts.ts:30–32`).
const SETTLE_TICKS: u32 = 2;
/// Frozen `searchBananaCrate` message wait (`crate.ts:52`).
const CRATE_READ_MS: u64 = 8_000;
/// Frozen `pickBananas` loop bound and per-pick wait (`karamja.ts:53, 69`).
const PICK_ATTEMPTS: u32 = 40;
const PICK_MS: u64 = 4_000;
/// Frozen `driveUntil` default bound (`prompts.ts:77`).
const DRIVE_UNTIL_MS: u64 = 30_000;
/// Frozen `driveChoice` / `talkChoosingBy` loop bounds.
const CHOICE_PAGES: u32 = 60;
const TALK_PAGES: u32 = crate::dialog::DRIVE_STEPS;

thread_local! {
    /// Frozen module-level `recovering` (`karamjaRecovery.ts:22`).
    static RECOVERING: Cell<bool> = const { Cell::new(false) };
}

/// Holds [`RECOVERING`] for one recovery's life, however it ends.
struct Guard;

impl Guard {
    fn take() -> Option<Self> {
        RECOVERING.with(|flag| (!flag.replace(true)).then_some(Self))
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        RECOVERING.with(|flag| flag.set(false));
    }
}

/// Frozen `onIsland` (`karamjaRecovery.ts:12–14`).
fn on_island(tile: WorldTile) -> bool {
    tile.level == 0 && (2700..3000).contains(&tile.x) && (2880..=3255).contains(&tile.z)
}

/// Frozen `Inventory.countById(id)` from the posted backpack.
fn held(id: i32) -> i32 {
    observed::with(|scene| {
        scene.since_login().inv().map_or(0, |rows| {
            rows.iter()
                .filter(|row| row.id == id)
                .map(|row| row.count)
                .sum()
        })
    })
}

/// Frozen `Inventory.isFull()`.
fn inventory_full() -> bool {
    observed::with(|scene| {
        let session = scene.since_login();
        let size = session.inv_size().unwrap_or(0);
        let used = session.inv().map_or(0, |rows| {
            rows.iter()
                .filter(|row| row.name.as_deref().is_some_and(|name| !name.is_empty()))
                .count()
        });
        size > 0 && used >= usize::try_from(size).unwrap_or(usize::MAX)
    })
}

/// Frozen `missingBoatFare` over the host's posted walk shorts.
pub(crate) fn missing_boat_fare(dest: WorldTile) -> bool {
    let Some(from) = here() else {
        return false;
    };
    on_island(from)
        && !on_island(dest)
        && held(COINS) < BOAT_FARE
        && observed::with(|scene| {
            scene
                .since_login()
                .walk_missing_carry()
                .is_some_and(|rows| {
                    matches!(rows.as_slice(), [row] if row.id == COINS && row.count == BOAT_FARE)
                })
        })
}

/// One posted loc the frozen query picked: its op target.
struct LocHit {
    id: i32,
    name: String,
    x: i32,
    z: i32,
    level: i32,
}

/// Frozen `Locs.query()...within(within).nearest()` over the posted locs.
fn nearest_loc(within: i32, keep: impl Fn(&observed::SceneRow) -> bool) -> Option<LocHit> {
    observed::with(|scene| {
        scene.since_login().locs().and_then(|rows| {
            rows.iter()
                .filter(|row| row.distance <= within && keep(row))
                .min_by_key(|row| row.distance)
                .map(|row| LocHit {
                    id: row.id,
                    name: row.name_or_empty().to_string(),
                    x: row.x,
                    z: row.z,
                    level: row.level,
                })
        })
    })
}

fn has_op(row: &observed::SceneRow, op: &str) -> bool {
    row.actions.iter().any(|have| have.eq_ignore_ascii_case(op))
}

/// Frozen `loc.interact('Search')`: the op on this exact row.
fn search(loc: &LocHit, cx: &mut Cx<'_>) {
    cx.emit(InteractReq::Loc {
        x: loc.x,
        z: loc.z,
        level: loc.level,
        action: "Search".into(),
        id: Some(loc.id),
    });
}

/// Frozen `crate.ts` `CrateState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CrateState {
    rum: bool,
    bananas: i32,
}

/// Frozen `readCrateMessages` (`crate.ts:16–33`).
fn read_crate_messages(text: &str) -> Option<CrateState> {
    let text = text.to_lowercase();
    let rum = text.contains("there is some rum in here")
        || text.contains("there is also some rum stashed in here");
    if text.contains("the crate is completely empty") {
        return Some(CrateState {
            rum: false,
            bananas: 0,
        });
    }
    if text.contains("the crate is full of bananas") {
        return Some(CrateState {
            rum,
            bananas: CRATE_FULL,
        });
    }
    // `/the crate has (\d+) bananas? inside/`
    let mut rest = text.as_str();
    while let Some(at) = rest.find("the crate has ") {
        let after = &rest[at + "the crate has ".len()..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            let tail = &after[digits..];
            if tail.starts_with(" bananas inside") || tail.starts_with(" banana inside") {
                if let Ok(bananas) = after[..digits].parse() {
                    return Some(CrateState { rum, bananas });
                }
            }
        }
        rest = after;
    }
    rum.then_some(CrateState { rum, bananas: 0 })
}

/// The game lines since `mark`, joined (frozen `said().join(' ')`).
fn said_since(mark: i32) -> String {
    observed::with(|scene| {
        scene
            .since_login()
            .chat_lines()
            .map_or_else(String::new, |lines| {
                lines
                    .iter()
                    .filter(|line| line.seq > mark)
                    .map(|line| &*line.text)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
    })
}

/// A resilient leg of the recovery: its own walks never recover again.
fn leg(dest: WorldTile, radius: i32, cx: &mut Cx<'_>) -> Result<Resilient, bool> {
    Resilient::new(dest, radius, LEG_MS, Some(LEG_ATTEMPTS), false)
        .without_boat_fare()
        .start(cx)
}

/// Step a resilient leg, moving its log lines to `logs`.
fn step_leg(walk: &mut Resilient, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<bool> {
    let done = walk.step(cx);
    while let Some(line) = walk.pop_log() {
        logs.push_back(line);
    }
    done
}

/// Page phases shared by [`Pages`].
enum PagePhase {
    Head,
    /// Frozen `ChatDialog.continue` / `chooseOption` ack wait, then the
    /// frozen `delayTicks(then)`.
    Ack {
        before: i32,
        then: u32,
    },
    Ticks(u32),
}

/// Frozen `talkChoosingBy(npc, [], prefer)`'s page loop (`strict`) or
/// `driveChoice(prefer)` (`prompts.ts:40–64`).
struct Pages {
    strict: bool,
    prefer: Vec<String>,
    pages: u32,
    phase: PagePhase,
}

impl Pages {
    fn new(strict: bool, prefer: &'static [&'static str]) -> Self {
        Self {
            strict,
            prefer: prefer.iter().map(|line| (*line).to_string()).collect(),
            pages: 0,
            phase: PagePhase::Head,
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<bool> {
        loop {
            match std::mem::replace(&mut self.phase, PagePhase::Head) {
                PagePhase::Ack { before, then } => {
                    let (modal, cont) = chat_state();
                    self.phase = if modal != before || cont || cx.clock().bound_reached() {
                        cx.clock().deadline = None;
                        PagePhase::Ticks(then)
                    } else {
                        PagePhase::Ack { before, then }
                    };
                    return None;
                }
                PagePhase::Ticks(left) if left > 1 => {
                    self.phase = PagePhase::Ticks(left - 1);
                    return None;
                }
                PagePhase::Ticks(_) => {}
                PagePhase::Head => return self.head(cx, logs),
            }
        }
    }

    fn head(&mut self, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<bool> {
        let (modal, cont) = chat_state();
        let limit = if self.strict {
            TALK_PAGES
        } else {
            CHOICE_PAGES
        };
        if self.pages >= limit {
            return Some(modal == -1);
        }
        self.pages += 1;
        if self.strict && interrupted() {
            return Some(false);
        }
        if cont {
            cx.emit(InteractReq::ContinueDialog);
            cx.clock().arm(crate::dialog::PAGE_ACK_MS);
            self.phase = PagePhase::Ack {
                before: modal,
                then: 1,
            };
            return None;
        }
        let pick = observed::with(|scene| {
            let options = scene.since_login().chat_options()?;
            if options.is_empty() {
                return None;
            }
            Some(
                crate::dialog::pick_preferred(options, &self.prefer)
                    .ok_or_else(|| options.join(" | ")),
            )
        });
        match pick {
            Some(Ok(index)) => {
                cx.emit(InteractReq::Answer {
                    option: i32::try_from(index + 1).unwrap_or(i32::MAX),
                });
                cx.clock().arm(crate::dialog::PAGE_ACK_MS);
                self.phase = PagePhase::Ack {
                    before: modal,
                    then: 2,
                };
                None
            }
            Some(Err(options)) => {
                logs.push_back(if self.strict {
                    format!("no rule or preference matched [{options}] after \"\"")
                } else {
                    format!("no preferred option in [{options}]")
                });
                Some(false)
            }
            None if modal == -1 => Some(true),
            None => {
                self.phase = PagePhase::Ticks(1);
                None
            }
        }
    }
}

/// Frozen `talkStrict(LUTHAS.npc, LUTHAS.prefer, log)`.
enum Talk {
    Open(NpcReach),
    Drive(Pages),
}

impl Talk {
    /// Frozen `openDialogue`'s synchronous head (`primitives.ts:228–237`).
    fn start(logs: &mut VecDeque<String>) -> Result<Self, bool> {
        let (modal, cont) = chat_state();
        if modal != -1 || cont {
            return Ok(Self::Drive(Pages::new(true, LUTHAS_PREFER)));
        }
        if !npc_talkable(LUTHAS) {
            logs.push_back(format!("no '{LUTHAS}' nearby to talk to"));
            return Err(false);
        }
        Ok(Self::Open(NpcReach::new(
            LUTHAS,
            NpcReachOpts {
                expect: TalkExpect::DialogReady,
                expect_ms: crate::dialog::DIALOGUE_OPEN_MS,
                retry_after_timeout: false,
                probe_unreachable: true,
                skip_click_when_expected: true,
            },
        )))
    }

    fn step(&mut self, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<bool> {
        match self {
            Self::Open(reach) => {
                let status = reach.step(cx);
                while let Some(line) = reach.pop_log() {
                    logs.push_back(line);
                }
                match status? {
                    "done" => {
                        *self = Self::Drive(Pages::new(true, LUTHAS_PREFER));
                        self.step(cx, logs)
                    }
                    _ => {
                        logs.push_back(format!("'{LUTHAS}' never opened a dialogue"));
                        Some(false)
                    }
                }
            }
            Self::Drive(pages) => pages.step(cx, logs),
        }
    }
}

/// Frozen `searchBananaCrate` (`crate.ts:36–60`).
enum SearchCrate {
    Walk(Resilient),
    Settle(u32),
    Read { mark: i32 },
}

impl SearchCrate {
    fn start(cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Result<Self, Option<CrateState>> {
        match leg(CRATE_TILE, 2, cx) {
            Ok(walk) => Ok(Self::Walk(walk)),
            Err(true) => Ok(Self::Settle(SETTLE_TICKS)),
            Err(false) => {
                logs.push_back("could not reach the plantation crate".into());
                Err(None)
            }
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<Option<CrateState>> {
        match self {
            Self::Walk(walk) => match step_leg(walk, cx, logs)? {
                true => {
                    *self = Self::Settle(SETTLE_TICKS);
                    None
                }
                false => {
                    logs.push_back("could not reach the plantation crate".into());
                    Some(None)
                }
            },
            Self::Settle(left) if *left > 1 => {
                *left -= 1;
                None
            }
            Self::Settle(_) => {
                let Some(crate_loc) =
                    nearest_loc(6, |row| row.id == BANANA_CRATE && has_op(row, "Search"))
                else {
                    logs.push_back("no searchable Crate at the plantation".into());
                    return Some(None);
                };
                let mark = chat_mark();
                search(&crate_loc, cx);
                cx.clock().arm(CRATE_READ_MS);
                *self = Self::Read { mark };
                None
            }
            Self::Read { mark } => {
                let said = said_since(*mark);
                let state = read_crate_messages(&said);
                if state.is_none() && !cx.clock().bound_reached() {
                    return None;
                }
                logs.push_back(match state {
                    Some(state) => format!(
                        "plantation crate: rum={} bananas={}",
                        state.rum, state.bananas
                    ),
                    None => {
                        let lines = observed::with(|scene| {
                            scene
                                .since_login()
                                .chat_lines()
                                .map_or_else(String::new, |lines| {
                                    lines
                                        .iter()
                                        .filter(|line| line.seq > *mark)
                                        .map(|line| &*line.text)
                                        .collect::<Vec<_>>()
                                        .join(" | ")
                                })
                        });
                        format!("plantation crate said nothing readable: [{lines}]")
                    }
                });
                Some(state)
            }
        }
    }
}

/// Frozen `pickBananas(want)` (`karamja.ts:45–74`).
struct Pick {
    want: i32,
    attempt: u32,
    phase: PickPhase,
}

enum PickPhase {
    Walk(Resilient),
    Settle(u32),
    Wait { before: i32 },
}

impl Pick {
    fn start(want: i32, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Result<Self, i32> {
        if held(BANANA) >= want {
            return Err(held(BANANA));
        }
        let phase = match leg(GROVE_TILE, 4, cx) {
            Ok(walk) => PickPhase::Walk(walk),
            Err(true) => PickPhase::Settle(SETTLE_TICKS),
            Err(false) => {
                logs.push_back("could not reach the banana grove".into());
                return Err(held(BANANA));
            }
        };
        let mut pick = Self {
            want,
            attempt: 0,
            phase,
        };
        if matches!(pick.phase, PickPhase::Settle(_)) {
            if let Some(done) = pick.next(logs) {
                return Err(done);
            }
        }
        Ok(pick)
    }

    /// The loop head: `Some(picked)` once the loop ends.
    fn next(&mut self, logs: &mut VecDeque<String>) -> Option<i32> {
        if self.attempt >= PICK_ATTEMPTS || held(BANANA) >= self.want || inventory_full() {
            return Some(self.finish(logs));
        }
        self.phase = PickPhase::Settle(SETTLE_TICKS);
        None
    }

    fn finish(&self, logs: &mut VecDeque<String>) -> i32 {
        let picked = held(BANANA);
        logs.push_back(format!("picked {picked} bananas"));
        picked
    }

    fn step(&mut self, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<i32> {
        match &mut self.phase {
            PickPhase::Walk(walk) => match step_leg(walk, cx, logs)? {
                true => self.next(logs),
                false => {
                    logs.push_back("could not reach the banana grove".into());
                    Some(held(BANANA))
                }
            },
            PickPhase::Settle(left) if *left > 1 => {
                *left -= 1;
                None
            }
            PickPhase::Settle(_) => {
                let Some(tree) = nearest_loc(20, |row| {
                    BANANA_TREE_IDS.contains(&row.id) && has_op(row, "Search")
                }) else {
                    logs.push_back("no bearing Banana Tree in range of the grove anchor".into());
                    return Some(self.finish(logs));
                };
                let before = held(BANANA);
                search(&tree, cx);
                cx.clock().arm(PICK_MS);
                self.phase = PickPhase::Wait { before };
                None
            }
            PickPhase::Wait { before } => {
                if held(BANANA) <= *before && !cx.clock().bound_reached() {
                    return None;
                }
                cx.clock().deadline = None;
                self.attempt += 1;
                self.next(logs)
            }
        }
    }
}

/// Frozen `driveUntil(expect, [], log)` (`prompts.ts:73–93`) with the
/// `useOnLoc` goal `held(BANANA) < before`. Each step is one pass of the
/// loop after its `delayTicks(1)`.
struct DriveUntil {
    before: i32,
    deadline: std::time::Instant,
    choice: Option<Pages>,
}

impl DriveUntil {
    fn expect(&self) -> bool {
        held(BANANA) < self.before
    }

    fn step(&mut self, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<bool> {
        if let Some(choice) = self.choice.as_mut() {
            if !choice.step(cx, logs)? {
                return Some(self.expect());
            }
            self.choice = None;
            return None;
        }
        if cx.clock().now() >= self.deadline {
            return Some(self.expect());
        }
        if self.expect() {
            return Some(true);
        }
        let (modal, cont) = chat_state();
        if modal != -1 || cont {
            self.choice = Some(Pages::new(false, &[]));
            return self.step(cx, logs);
        }
        None
    }
}

/// Frozen `useOnLoc(BANANA, crate, [], held < before)` (`prompts.ts:316–345`).
enum PackOne {
    Walk { before: i32, walk: Resilient },
    Settle { before: i32, left: u32 },
    Drive(DriveUntil),
}

impl PackOne {
    fn start(cx: &mut Cx<'_>) -> Result<Self, bool> {
        let before = held(BANANA);
        match leg(CRATE_TILE, 2, cx) {
            Ok(walk) => Ok(Self::Walk { before, walk }),
            Err(true) => Ok(Self::Settle {
                before,
                left: SETTLE_TICKS,
            }),
            Err(false) => Err(false),
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<bool> {
        match self {
            Self::Walk { before, walk } => {
                if !step_leg(walk, cx, logs)? {
                    return Some(false);
                }
                *self = Self::Settle {
                    before: *before,
                    left: SETTLE_TICKS,
                };
                None
            }
            Self::Settle { left, .. } if *left > 1 => {
                *left -= 1;
                None
            }
            Self::Settle { before, .. } => {
                let before = *before;
                let target = nearest_loc(6, |row| {
                    row.id == BANANA_CRATE && row.name_or_empty().eq_ignore_ascii_case("Crate")
                });
                let item = observed::with(|scene| {
                    scene.since_login().inv().and_then(|rows| {
                        rows.iter().find(|row| row.id == BANANA).map(|row| {
                            (
                                row.name.as_deref().unwrap_or_default().to_string(),
                                row.slot,
                            )
                        })
                    })
                });
                let (Some(target), Some((name, slot))) = (target, item) else {
                    logs.push_back(format!(
                        "no 'Crate' id {BANANA_CRATE} or no item {BANANA} to use on it near ({},{})",
                        CRATE_TILE.x, CRATE_TILE.z
                    ));
                    return Some(false);
                };
                cx.emit(InteractReq::UseOn {
                    name,
                    kind: "loc".into(),
                    target_name: Some(target.name),
                    x: target.x,
                    z: target.z,
                    level: target.level,
                    index: None,
                    source_item_id: Some(BANANA),
                    source_item_slot: slot,
                    target_item_id: None,
                    target_item_slot: None,
                });
                *self = Self::Drive(DriveUntil {
                    before,
                    deadline: cx.clock().now() + Duration::from_millis(DRIVE_UNTIL_MS),
                    choice: None,
                });
                None
            }
            Self::Drive(drive) => drive.step(cx, logs),
        }
    }
}

/// Frozen `fillCrate` (`karamja.ts:79–113`).
struct Fill {
    pass: i32,
    want: i32,
    packed: i32,
    phase: FillPhase,
}

enum FillPhase {
    Search(SearchCrate),
    Pick(Pick),
    WalkCrate(Resilient),
    Settle(u32),
    Pack(PackOne),
    Final(SearchCrate),
}

impl Fill {
    fn start(cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Result<Self, bool> {
        let mut fill = Self {
            pass: 0,
            want: 0,
            packed: 0,
            phase: FillPhase::Settle(0),
        };
        match fill.pass_head(cx, logs) {
            Some(done) => Err(done),
            None => Ok(fill),
        }
    }

    /// The `for (pass < CRATE_FULL)` head, else the closing read.
    fn pass_head(&mut self, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<bool> {
        let start = SearchCrate::start(cx, logs);
        if self.pass >= CRATE_FULL {
            return match start {
                Ok(search) => {
                    self.phase = FillPhase::Final(search);
                    None
                }
                Err(_) => Some(false),
            };
        }
        match start {
            Ok(search) => {
                self.phase = FillPhase::Search(search);
                None
            }
            Err(_) => Some(false),
        }
    }

    fn after_read(
        &mut self,
        state: CrateState,
        cx: &mut Cx<'_>,
        logs: &mut VecDeque<String>,
    ) -> Option<bool> {
        if state.bananas >= CRATE_FULL {
            return Some(true);
        }
        self.want = CRATE_FULL - state.bananas;
        match Pick::start(self.want, cx, logs) {
            Ok(pick) => {
                self.phase = FillPhase::Pick(pick);
                None
            }
            Err(picked) => self.after_pick(picked, cx, logs),
        }
    }

    fn after_pick(
        &mut self,
        picked: i32,
        cx: &mut Cx<'_>,
        logs: &mut VecDeque<String>,
    ) -> Option<bool> {
        if picked == 0 {
            logs.push_back("the grove gave up no bananas".into());
            return Some(false);
        }
        match leg(CRATE_TILE, 2, cx) {
            Ok(walk) => {
                self.phase = FillPhase::WalkCrate(walk);
                None
            }
            Err(true) => {
                self.phase = FillPhase::Settle(SETTLE_TICKS);
                None
            }
            Err(false) => Some(false),
        }
    }

    /// The pack loop head (`packed < want && held(BANANA) > 0`).
    fn pack_head(&mut self, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<bool> {
        if self.packed < self.want && held(BANANA) > 0 {
            if let Ok(pack) = PackOne::start(cx) {
                self.phase = FillPhase::Pack(pack);
                return None;
            }
        }
        self.pass += 1;
        self.pass_head(cx, logs)
    }

    fn step(&mut self, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<bool> {
        match &mut self.phase {
            FillPhase::Search(search) => match search.step(cx, logs)? {
                None => Some(false),
                Some(state) => self.after_read(state, cx, logs),
            },
            FillPhase::Final(search) => Some(
                search
                    .step(cx, logs)?
                    .is_some_and(|state| state.bananas >= CRATE_FULL),
            ),
            FillPhase::Pick(pick) => {
                let picked = pick.step(cx, logs)?;
                self.after_pick(picked, cx, logs)
            }
            FillPhase::WalkCrate(walk) => {
                if !step_leg(walk, cx, logs)? {
                    return Some(false);
                }
                self.phase = FillPhase::Settle(SETTLE_TICKS);
                None
            }
            FillPhase::Settle(left) if *left > 1 => {
                *left -= 1;
                None
            }
            FillPhase::Settle(_) => {
                self.packed = 0;
                self.pack_head(cx, logs)
            }
            FillPhase::Pack(pack) => {
                if !pack.step(cx, logs)? {
                    self.packed = self.want;
                } else {
                    self.packed += 1;
                }
                self.pack_head(cx, logs)
            }
        }
    }
}

enum Stage {
    TalkWalk { second: bool, walk: Walk },
    Talk { second: bool, talk: Talk },
    Fill(Fill),
}

/// Frozen `recoverBoatFare(dest, missing, log)` (`karamjaRecovery.ts:24–58`).
/// `Some(true)` once the pack holds the fare.
pub(crate) struct Recover {
    stage: Stage,
    _guard: Guard,
}

impl Recover {
    /// Frozen `recoverBoatFare`'s gate and first talk leg. `Err(false)` when
    /// no recovery applies or it cannot start; log lines go to `logs`.
    pub(crate) fn start(
        dest: WorldTile,
        cx: &mut Cx<'_>,
        logs: &mut VecDeque<String>,
    ) -> Result<Self, bool> {
        if RECOVERING.with(Cell::get) || interrupted() || !missing_boat_fare(dest) {
            return Err(false);
        }
        if inventory_full() && held(BANANA) == 0 {
            logs.push_back("boat fare recovery needs one free inventory slot".into());
            return Err(false);
        }
        let guard = Guard::take().ok_or(false)?;
        logs.push_back("no boat fare: earning 30 coins at Luthas's plantation".into());
        let stage = Self::talk(false, cx, logs)?;
        Ok(Self {
            stage,
            _guard: guard,
        })
    }

    /// Frozen `talk()` (`karamjaRecovery.ts:36–41`).
    fn talk(second: bool, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Result<Stage, bool> {
        if interrupted() {
            return Err(false);
        }
        match Walk::begin(LUTHAS_ANCHOR, 2, TALK_WALK_MS, false, cx) {
            Ok(walk) => Ok(Stage::TalkWalk { second, walk }),
            Err(true) => Ok(Stage::Talk {
                second,
                talk: Talk::start(logs)?,
            }),
            Err(false) => Err(false),
        }
    }

    /// Stop any native walk this recovery has in flight.
    pub(crate) fn abort(&self, cx: &mut Cx<'_>) {
        match &self.stage {
            Stage::TalkWalk { walk, .. } => walk.abort(cx),
            Stage::Fill(fill) => match &fill.phase {
                FillPhase::Search(SearchCrate::Walk(walk))
                | FillPhase::Final(SearchCrate::Walk(walk))
                | FillPhase::WalkCrate(walk)
                | FillPhase::Pick(Pick {
                    phase: PickPhase::Walk(walk),
                    ..
                })
                | FillPhase::Pack(PackOne::Walk { walk, .. }) => walk.abort(cx),
                _ => {}
            },
            Stage::Talk { .. } => {}
        }
    }

    /// `Some(fare_held)` once the recovery ended.
    pub(crate) fn step(&mut self, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<bool> {
        if interrupted() {
            self.abort(cx);
            return Some(false);
        }
        let next = match &mut self.stage {
            Stage::TalkWalk { second, walk } => {
                if !walk.step(cx)? {
                    return Some(false);
                }
                Stage::Talk {
                    second: *second,
                    talk: match Talk::start(logs) {
                        Ok(talk) => talk,
                        Err(done) => return Some(done),
                    },
                }
            }
            Stage::Talk { second, talk } => {
                if !talk.step(cx, logs)? {
                    return Some(false);
                }
                if *second || held(COINS) >= BOAT_FARE {
                    return Some(held(COINS) >= BOAT_FARE);
                }
                if interrupted() {
                    return Some(false);
                }
                match Fill::start(cx, logs) {
                    Ok(fill) => Stage::Fill(fill),
                    Err(_) => return Some(false),
                }
            }
            Stage::Fill(fill) => {
                if !fill.step(cx, logs)? {
                    return Some(false);
                }
                match Self::talk(true, cx, logs) {
                    Ok(stage) => stage,
                    Err(done) => return Some(done),
                }
            }
        };
        self.stage = next;
        None
    }
}

#[derive(Clone, Copy, Deserialize)]
struct TileArg {
    x: i32,
    z: i32,
    #[serde(default)]
    level: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WalkToArgs {
    tile: TileArg,
    #[serde(default)]
    radius: i32,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    allow_teleports: bool,
}

/// The host `Traversal.walkTo` wait bound when the caller names none.
const WALK_TO_MS: u64 = 60_000;

enum WalkToPhase {
    Walking { walk: Walk, retried: bool },
    Recover(Recover),
}

/// Frozen `Traversal.walkTo(dest, opts)` (`Traversal.ts:80–95`).
pub(crate) struct WalkTo {
    dest: WorldTile,
    radius: i32,
    timeout_ms: u64,
    allow_teleports: bool,
    phase: WalkToPhase,
    logs: VecDeque<String>,
    result: Option<bool>,
    pumped: bool,
    waiting: bool,
}

const LOG: usize = 0;
const SUSTAIN: usize = 1;

impl Family for WalkTo {
    const NAME: &'static str = "walk-to";
    const CALLBACKS: &'static [&'static str] = &["log", "sustain"];
    /// Frozen `log(...)` is not awaited; `await Sustain.run()` is.
    const SYNC_HOOKS: &'static [usize] = &[LOG];
    type Args = WalkToArgs;
    type Output = bool;

    fn begin(args: WalkToArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let dest = WorldTile {
            x: args.tile.x,
            z: args.tile.z,
            level: args.tile.level,
        };
        let timeout_ms = args.timeout_ms.unwrap_or(WALK_TO_MS);
        match Walk::begin(dest, args.radius, timeout_ms, args.allow_teleports, cx) {
            Ok(walk) => Begin::Run(Self {
                dest,
                radius: args.radius,
                timeout_ms,
                allow_teleports: args.allow_teleports,
                phase: WalkToPhase::Walking {
                    walk,
                    retried: false,
                },
                logs: VecDeque::new(),
                result: None,
                pumped: false,
                waiting: false,
            }),
            Err(arrived) => Begin::Done(arrived),
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        if let Some(Reply::Threw(thrown)) = cx.reply() {
            return Step::Fail(thrown);
        }
        loop {
            if let Some(line) = self.logs.pop_front() {
                if cx.has(LOG) {
                    return Step::Call(Call {
                        hook: LOG,
                        args: vec![json!(line)],
                    });
                }
                continue;
            }
            if let Some(result) = self.result {
                return Step::Done(result);
            }
            if std::mem::take(&mut self.waiting) {
                self.pumped = false;
                return Step::Wait;
            }
            // The recovery's `await Sustain.run()` points (`karamja.ts:55,
            // 91`; `Traversal.ts:130` in its resilient legs), once a tick.
            if matches!(self.phase, WalkToPhase::Recover(_)) && !self.pumped && cx.has(SUSTAIN) {
                self.pumped = true;
                return Step::Call(Call {
                    hook: SUSTAIN,
                    args: Vec::new(),
                });
            }
            self.advance(cx);
            if self.result.is_none() && self.logs.is_empty() {
                self.pumped = false;
                return Step::Wait;
            }
            if self.result.is_none() {
                self.waiting = true;
            }
        }
    }
}

impl WalkTo {
    fn advance(&mut self, cx: &mut Cx<'_>) {
        match &mut self.phase {
            WalkToPhase::Walking { walk, retried } => {
                let Some(value) = walk.step(cx) else {
                    return;
                };
                if value || *retried {
                    self.result = Some(value);
                    return;
                }
                match Recover::start(self.dest, cx, &mut self.logs) {
                    Ok(recover) => self.phase = WalkToPhase::Recover(recover),
                    Err(_) => self.result = Some(false),
                }
            }
            WalkToPhase::Recover(recover) => match recover.step(cx, &mut self.logs) {
                None => {}
                Some(false) => self.result = Some(false),
                Some(true) => match Walk::begin(
                    self.dest,
                    self.radius,
                    self.timeout_ms,
                    self.allow_teleports,
                    cx,
                ) {
                    Ok(walk) => {
                        self.phase = WalkToPhase::Walking {
                            walk,
                            retried: true,
                        }
                    }
                    Err(arrived) => self.result = Some(arrived),
                },
            },
        }
    }
}

#[cfg(test)]
#[path = "boat_fare_tests.rs"]
mod tests;

#[cfg(test)]
mod crate_message_tests {
    use super::*;

    #[test]
    fn crate_messages_read_as_banana_crate_rs2_prints_them() {
        let read = |text: &str| read_crate_messages(text);
        assert_eq!(
            read("The crate is completely empty."),
            Some(CrateState {
                rum: false,
                bananas: 0
            })
        );
        assert_eq!(
            read("The crate has 1 banana inside. There is also some rum stashed in here too."),
            Some(CrateState {
                rum: true,
                bananas: 1
            })
        );
        assert_eq!(
            read("The crate has 7 bananas inside."),
            Some(CrateState {
                rum: false,
                bananas: 7
            })
        );
        assert_eq!(
            read("The crate is full of bananas."),
            Some(CrateState {
                rum: false,
                bananas: 10
            })
        );
        assert_eq!(
            read("There is some rum in here, although with no bananas to cover it."),
            Some(CrateState {
                rum: true,
                bananas: 0
            })
        );
        assert_eq!(read("You pick a banana."), None);
        assert_eq!(
            read("The crate has bananas inside."),
            None,
            "no count, no read"
        );
    }

    #[test]
    fn on_island_is_the_frozen_karamja_box() {
        let t = |x, z, level| WorldTile { x, z, level };
        assert!(on_island(t(2954, 3147, 0)));
        assert!(on_island(t(2700, 2880, 0)));
        assert!(on_island(t(2999, 3255, 0)));
        assert!(!on_island(t(3000, 3200, 0)));
        assert!(!on_island(t(2954, 3256, 0)));
        assert!(!on_island(t(2954, 3147, 1)));
    }
}
