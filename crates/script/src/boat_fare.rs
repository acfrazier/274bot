//! Frozen Karamja boat-fare recovery and `Traversal.walkTo`, the `walk-to`
//! [`crate::machine`] family.
//!
//! Frozen `Traversal.walkTo` (`Traversal.ts:80–95`): one `WalkExecutor`
//! walk; on a `failed`/`unreachable` outcome `recoverBoatFare`
//! (`karamjaRecovery.ts`), then one more walk. The baked leg of
//! `walkResilient` is the same `Traversal.walkTo` (`Traversal.ts:160–170`),
//! so [`crate::walk::Resilient`] embeds [`Recover`] too.
//!
//! `WalkOptions` (`WalkExecutor.ts:113–139, 226–245`): `radius` defaults to
//! 2 and `timeoutMs` to 300 s; teleports are `resolveWalkUseTeleports` (an
//! explicit false wins) gated on `policy.distanceBeforeTeleport`;
//! `maxExpansions` is accepted: it bounds the frozen PathFinder, and the host
//! router searches to its own bound (4,000,000 expansions), never less;
//! `bankItemCounts` is not a host input (the BankBudget fetch reads the live
//! bank). `avoidZones` rectangles ride the walk request and every host
//! search of the walk keeps out of them; a catalog zone id is refused.
//! Teleport id lists, `useShips`/`useShortcuts` false, `pathFollow` and
//! `forceRepath` have no host wire and are refused loud.
//! The caller's `Sustain.run()` runs once a tick while walking, as frozen
//! runs it every follow pass (`WalkExecutor.ts:844–853`).
//!
//! `missingBoatFare` (`karamjaRecovery.ts:16–20`): on Karamja, walking off
//! the island, and the only missing carry row is the selected coins item with
//! a required count above the amount held. Frozen names the shortfall
//! (`bankPlan.ts:98–113`); the host uses the edge's required total count from
//! `walk_missing_carry` (`nav::router::missing_item_reqs`). The rows are only
//! posted for a `NoPath` failure, as frozen only explains an `unreachable`
//! one (`WalkExecutor.ts:466–490`).
//!
//! The recovery is the banana-plantation job derived from selected content
//! (`quest_hunt/scripts/luthas.rs2` and `piratestreasure/karamja.ts`): talk to
//! Luthas, fill the configured crate from the fruiting grove, then talk again.
//! Frozen primitives ported here: `openDialogue` / `talkChoosingBy`
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
use crate::shim::{InspectAvoidWire, InteractReq};
use crate::supply_v2;
use crate::walk::{
    avoid_refusal, here, interrupted, resolve_teleports, teleport_span_allows, Resilient, Walk,
};
use api::snapshot::WorldTile;
use serde::Deserialize;
use serde_json::json;
use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

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

struct TreeAction {
    id: i32,
    action: String,
}

struct BoatFacts {
    coins: i32,
    banana: i32,
    crate_loc: i32,
    tree_actions: Vec<TreeAction>,
    luthas_name: String,
    luthas_anchor: WorldTile,
    crate_tile: WorldTile,
    grove_tile: WorldTile,
    crate_capacity: i32,
    fare: i32,
    crate_search: String,
    preferences: [String; 3],
}

impl BoatFacts {
    fn selected() -> Result<Rc<Self>, String> {
        let data = supply_v2::selected_data()
            .ok_or_else(|| supply_v2::GAME_DATA_UNAVAILABLE.to_string())?;
        let facts = data.karamja().ok_or_else(|| {
            "Karamja recovery facts are unavailable for selected content".to_string()
        })?;
        let coins = data
            .item_by_alias("coins")
            .ok_or_else(|| "selected content has no coins item".to_string())?
            .id;
        let banana = data
            .item_by_alias("banana")
            .ok_or_else(|| "selected content has no banana item".to_string())?
            .id;
        let luthas = data
            .npc_by_config(&facts.luthas_spawn.config)
            .ok_or_else(|| "selected content has no Karamja plantation NPC".to_string())?;
        if !luthas
            .ops
            .iter()
            .any(|action| action.eq_ignore_ascii_case("Talk-to"))
        {
            return Err("selected Luthas has no Talk-to operation".to_string());
        }
        let luthas_name = luthas
            .display
            .as_deref()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| "selected Luthas has no display name".to_string())?
            .to_string();
        let crate_loc = data
            .loc_by_config(&facts.crate_spawn.config)
            .ok_or_else(|| "selected content has no Karamja crate loc".to_string())?;
        let crate_search = crate_loc
            .ops
            .first()
            .cloned()
            .ok_or_else(|| "selected Karamja crate has no operation".to_string())?;
        let mut tree_actions: Vec<TreeAction> = Vec::with_capacity(facts.banana_tree_configs.len());
        for config in &facts.banana_tree_configs {
            let loc = data
                .loc_by_config(config)
                .ok_or_else(|| format!("selected content has no banana tree loc {config}"))?;
            let action = loc
                .ops
                .first()
                .cloned()
                .ok_or_else(|| format!("selected banana tree {config} has no operation"))?;
            if let Some(existing) = tree_actions.iter().find(|tree| tree.id == loc.id) {
                if existing.action != action {
                    return Err(
                        "selected banana tree states disagree on their operation".to_string()
                    );
                }
            } else {
                tree_actions.push(TreeAction { id: loc.id, action });
            }
        }
        let tile = |spawn: &api::game_data::KaramjaSpawn| WorldTile {
            x: spawn.x,
            z: spawn.z,
            level: spawn.plane,
        };
        let grove = facts
            .banana_tree_spawns
            .first()
            .ok_or_else(|| "selected content has no banana tree placement".to_string())?;
        Ok(Rc::new(Self {
            coins,
            banana,
            crate_loc: crate_loc.id,
            tree_actions,
            luthas_name,
            luthas_anchor: tile(&facts.luthas_spawn),
            crate_tile: tile(&facts.crate_spawn),
            grove_tile: tile(grove),
            crate_capacity: facts.crate_capacity,
            fare: facts.coin_payout,
            crate_search,
            preferences: [
                facts.dialogue.employment.clone(),
                facts.dialogue.paid.clone(),
                facts.dialogue.incomplete.clone(),
            ],
        }))
    }

    fn is_fruiting_tree(&self, row: &observed::SceneRow) -> bool {
        self.tree_actions
            .iter()
            .any(|tree| row.id == tree.id && has_op(row, &tree.action))
    }

    fn tree_action(&self, id: i32) -> Option<&str> {
        self.tree_actions
            .iter()
            .find(|tree| tree.id == id)
            .map(|tree| tree.action.as_str())
    }
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

/// Frozen `missingBoatFare` over the host's posted walk shorts. The failed
/// native route must report the selected content's full-crate coin payout.
fn missing_boat_fare(dest: WorldTile, facts: &BoatFacts) -> Option<i32> {
    let from = here()?;
    if !on_island(from) || on_island(dest) {
        return None;
    }
    let coins_held = held(facts.coins);
    observed::with(|scene| {
        scene
            .since_login()
            .walk_missing_carry()
            .and_then(|rows| match rows.as_slice() {
                [row]
                    if row.id == facts.coins
                        && row.count == facts.fare
                        && row.count > coins_held =>
                {
                    Some(row.count)
                }
                _ => None,
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

/// Frozen `loc.interact` against the operation exposed by selected loc data.
fn search(loc: &LocHit, action: &str, cx: &mut Cx<'_>) {
    cx.emit(InteractReq::Loc {
        x: loc.x,
        z: loc.z,
        level: loc.level,
        action: action.into(),
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
fn read_crate_messages(text: &str, capacity: i32) -> Option<CrateState> {
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
            bananas: capacity,
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
    fn new(strict: bool, prefer: &[String]) -> Self {
        Self {
            strict,
            prefer: prefer.to_vec(),
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
        if self.strict && interrupted(cx) {
            return Some(false);
        }
        if cont {
            cx.emit(InteractReq::ContinueDialog { component_id: None });
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

/// Frozen strict Luthas dialogue using selected NPC identity and choices.
enum Talk {
    Open(Box<NpcReach>),
    Drive(Pages),
}

impl Talk {
    /// Frozen `openDialogue`'s synchronous head (`primitives.ts:228–237`).
    fn start(facts: &BoatFacts, logs: &mut VecDeque<String>) -> Result<Self, bool> {
        let (modal, cont) = chat_state();
        if modal != -1 || cont {
            return Ok(Self::Drive(Pages::new(true, &facts.preferences)));
        }
        if !npc_talkable(&facts.luthas_name) {
            logs.push_back(format!("no '{}' nearby to talk to", facts.luthas_name));
            return Err(false);
        }
        Ok(Self::Open(Box::new(NpcReach::new(
            &facts.luthas_name,
            NpcReachOpts {
                expect: TalkExpect::DialogReady,
                expect_ms: crate::dialog::DIALOGUE_OPEN_MS,
                retry_after_timeout: false,
                probe_unreachable: true,
                skip_click_when_expected: true,
            },
        ))))
    }

    fn step(
        &mut self,
        facts: &BoatFacts,
        cx: &mut Cx<'_>,
        logs: &mut VecDeque<String>,
    ) -> Option<bool> {
        match self {
            Self::Open(reach) => {
                let status = reach.step(cx);
                while let Some(line) = reach.pop_log() {
                    logs.push_back(line);
                }
                match status? {
                    "done" => {
                        *self = Self::Drive(Pages::new(true, &facts.preferences));
                        self.step(facts, cx, logs)
                    }
                    _ => {
                        logs.push_back(format!("'{}' never opened a dialogue", facts.luthas_name));
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
    fn start(
        facts: &BoatFacts,
        cx: &mut Cx<'_>,
        logs: &mut VecDeque<String>,
    ) -> Result<Self, Option<CrateState>> {
        match leg(facts.crate_tile, 2, cx) {
            Ok(walk) => Ok(Self::Walk(walk)),
            Err(true) => Ok(Self::Settle(SETTLE_TICKS)),
            Err(false) => {
                logs.push_back("could not reach the plantation crate".into());
                Err(None)
            }
        }
    }

    fn step(
        &mut self,
        facts: &BoatFacts,
        cx: &mut Cx<'_>,
        logs: &mut VecDeque<String>,
    ) -> Option<Option<CrateState>> {
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
                let Some(crate_loc) = nearest_loc(6, |row| {
                    row.id == facts.crate_loc && has_op(row, &facts.crate_search)
                }) else {
                    logs.push_back("no searchable Crate at the plantation".into());
                    return Some(None);
                };
                let mark = chat_mark();
                search(&crate_loc, &facts.crate_search, cx);
                cx.clock().arm(CRATE_READ_MS);
                *self = Self::Read { mark };
                None
            }
            Self::Read { mark } => {
                let said = said_since(*mark);
                let state = read_crate_messages(&said, facts.crate_capacity);
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
    fn start(
        facts: &BoatFacts,
        want: i32,
        cx: &mut Cx<'_>,
        logs: &mut VecDeque<String>,
    ) -> Result<Self, i32> {
        if held(facts.banana) >= want {
            return Err(held(facts.banana));
        }
        let phase = match leg(facts.grove_tile, 4, cx) {
            Ok(walk) => PickPhase::Walk(walk),
            Err(true) => PickPhase::Settle(SETTLE_TICKS),
            Err(false) => {
                logs.push_back("could not reach the banana grove".into());
                return Err(held(facts.banana));
            }
        };
        let mut pick = Self {
            want,
            attempt: 0,
            phase,
        };
        if matches!(pick.phase, PickPhase::Settle(_)) {
            if let Some(done) = pick.next(facts, logs) {
                return Err(done);
            }
        }
        Ok(pick)
    }

    /// The loop head: `Some(picked)` once the loop ends.
    fn next(&mut self, facts: &BoatFacts, logs: &mut VecDeque<String>) -> Option<i32> {
        if self.attempt >= PICK_ATTEMPTS || held(facts.banana) >= self.want || inventory_full() {
            return Some(self.finish(facts, logs));
        }
        self.phase = PickPhase::Settle(SETTLE_TICKS);
        None
    }

    fn finish(&self, facts: &BoatFacts, logs: &mut VecDeque<String>) -> i32 {
        let picked = held(facts.banana);
        logs.push_back(format!("picked {picked} bananas"));
        picked
    }

    fn step(
        &mut self,
        facts: &BoatFacts,
        cx: &mut Cx<'_>,
        logs: &mut VecDeque<String>,
    ) -> Option<i32> {
        match &mut self.phase {
            PickPhase::Walk(walk) => match step_leg(walk, cx, logs)? {
                true => self.next(facts, logs),
                false => {
                    logs.push_back("could not reach the banana grove".into());
                    Some(held(facts.banana))
                }
            },
            PickPhase::Settle(left) if *left > 1 => {
                *left -= 1;
                None
            }
            PickPhase::Settle(_) => {
                let Some(tree) = nearest_loc(20, |row| facts.is_fruiting_tree(row)) else {
                    logs.push_back("no bearing Banana Tree in range of the grove anchor".into());
                    return Some(self.finish(facts, logs));
                };
                let Some(action) = facts.tree_action(tree.id) else {
                    logs.push_back("selected banana tree has no content operation".into());
                    return Some(self.finish(facts, logs));
                };
                let before = held(facts.banana);
                search(&tree, action, cx);
                cx.clock().arm(PICK_MS);
                self.phase = PickPhase::Wait { before };
                None
            }
            PickPhase::Wait { before } => {
                if held(facts.banana) <= *before && !cx.clock().bound_reached() {
                    return None;
                }
                cx.clock().deadline = None;
                self.attempt += 1;
                self.next(facts, logs)
            }
        }
    }
}

/// Frozen `driveUntil(expect, [], log)` (`prompts.ts:73–93`) with the
/// selected banana item as its `useOnLoc` goal.
struct DriveUntil {
    before: i32,
    deadline: std::time::Instant,
    choice: Option<Pages>,
}

impl DriveUntil {
    fn expect(&self, facts: &BoatFacts) -> bool {
        held(facts.banana) < self.before
    }

    fn step(
        &mut self,
        facts: &BoatFacts,
        cx: &mut Cx<'_>,
        logs: &mut VecDeque<String>,
    ) -> Option<bool> {
        if let Some(choice) = self.choice.as_mut() {
            if !choice.step(cx, logs)? {
                return Some(self.expect(facts));
            }
            self.choice = None;
            return None;
        }
        if cx.clock().now() >= self.deadline {
            return Some(self.expect(facts));
        }
        if self.expect(facts) {
            return Some(true);
        }
        let (modal, cont) = chat_state();
        if modal != -1 || cont {
            self.choice = Some(Pages::new(false, &[]));
            return self.step(facts, cx, logs);
        }
        None
    }
}

/// Frozen `useOnLoc` over selected banana and crate facts (`prompts.ts:316–345`).
enum PackOne {
    Walk { before: i32, walk: Resilient },
    Settle { before: i32, left: u32 },
    Drive(DriveUntil),
}

impl PackOne {
    fn start(facts: &BoatFacts, cx: &mut Cx<'_>) -> Result<Self, bool> {
        let before = held(facts.banana);
        match leg(facts.crate_tile, 2, cx) {
            Ok(walk) => Ok(Self::Walk { before, walk }),
            Err(true) => Ok(Self::Settle {
                before,
                left: SETTLE_TICKS,
            }),
            Err(false) => Err(false),
        }
    }

    fn step(
        &mut self,
        facts: &BoatFacts,
        cx: &mut Cx<'_>,
        logs: &mut VecDeque<String>,
    ) -> Option<bool> {
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
                let target = nearest_loc(6, |row| row.id == facts.crate_loc);
                let item = observed::with(|scene| {
                    scene.since_login().inv().and_then(|rows| {
                        rows.iter().find(|row| row.id == facts.banana).map(|row| {
                            (
                                row.name.as_deref().unwrap_or_default().to_string(),
                                row.slot,
                            )
                        })
                    })
                });
                let (Some(target), Some((name, slot))) = (target, item) else {
                    logs.push_back(format!(
                        "no crate id {} or item {} near ({},{})",
                        facts.crate_loc, facts.banana, facts.crate_tile.x, facts.crate_tile.z
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
                    source_item_id: Some(facts.banana),
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
            Self::Drive(drive) => drive.step(facts, cx, logs),
        }
    }
}

/// Frozen `fillCrate` (`karamja.ts:79–113`).
struct Fill {
    facts: Rc<BoatFacts>,
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
    fn start(
        facts: Rc<BoatFacts>,
        cx: &mut Cx<'_>,
        logs: &mut VecDeque<String>,
    ) -> Result<Self, bool> {
        let mut fill = Self {
            facts,
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

    /// The `for (pass < crate_capacity)` head, else the closing read.
    fn pass_head(&mut self, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<bool> {
        let start = SearchCrate::start(&self.facts, cx, logs);
        if self.pass >= self.facts.crate_capacity {
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
        if state.bananas >= self.facts.crate_capacity {
            return Some(true);
        }
        self.want = self.facts.crate_capacity - state.bananas;
        match Pick::start(&self.facts, self.want, cx, logs) {
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
        match leg(self.facts.crate_tile, 2, cx) {
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

    /// The pack loop head (`packed < want && held(banana) > 0`).
    fn pack_head(&mut self, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<bool> {
        if self.packed < self.want && held(self.facts.banana) > 0 {
            if let Ok(pack) = PackOne::start(&self.facts, cx) {
                self.phase = FillPhase::Pack(pack);
                return None;
            }
        }
        self.pass += 1;
        self.pass_head(cx, logs)
    }

    fn step(&mut self, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<bool> {
        match &mut self.phase {
            FillPhase::Search(search) => match search.step(&self.facts, cx, logs)? {
                None => Some(false),
                Some(state) => self.after_read(state, cx, logs),
            },
            FillPhase::Final(search) => Some(
                search
                    .step(&self.facts, cx, logs)?
                    .is_some_and(|state| state.bananas >= self.facts.crate_capacity),
            ),
            FillPhase::Pick(pick) => {
                let picked = pick.step(&self.facts, cx, logs)?;
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
                if !pack.step(&self.facts, cx, logs)? {
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
    facts: Rc<BoatFacts>,
    required_coins: i32,
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
        if RECOVERING.with(Cell::get) || interrupted(cx) {
            return Err(false);
        }
        let Ok(facts) = BoatFacts::selected() else {
            return Err(false);
        };
        let Some(required_coins) = missing_boat_fare(dest, &facts) else {
            return Err(false);
        };
        if inventory_full() && held(facts.banana) == 0 {
            logs.push_back("boat fare recovery needs one free inventory slot".into());
            return Err(false);
        }
        let guard = Guard::take().ok_or(false)?;
        logs.push_back(format!(
            "no boat fare: earning {required_coins} coins at {}'s plantation",
            facts.luthas_name
        ));
        let stage = Self::talk(&facts, false, cx, logs)?;
        Ok(Self {
            facts,
            required_coins,
            stage,
            _guard: guard,
        })
    }

    /// Frozen `talk()` (`karamjaRecovery.ts:36–41`).
    fn talk(
        facts: &BoatFacts,
        second: bool,
        cx: &mut Cx<'_>,
        logs: &mut VecDeque<String>,
    ) -> Result<Stage, bool> {
        if interrupted(cx) {
            return Err(false);
        }
        match Walk::begin(facts.luthas_anchor, 2, TALK_WALK_MS, false, cx) {
            Ok(walk) => Ok(Stage::TalkWalk { second, walk }),
            Err(true) => Ok(Stage::Talk {
                second,
                talk: Talk::start(facts, logs)?,
            }),
            Err(false) => Err(false),
        }
    }

    /// The op that stops the native walk this recovery has in flight.
    pub(crate) fn release(&self) -> Option<InteractReq> {
        match &self.stage {
            Stage::TalkWalk { walk, .. } => Some(walk.release()),
            Stage::Fill(fill) => match &fill.phase {
                FillPhase::Search(SearchCrate::Walk(walk))
                | FillPhase::Final(SearchCrate::Walk(walk))
                | FillPhase::WalkCrate(walk)
                | FillPhase::Pick(Pick {
                    phase: PickPhase::Walk(walk),
                    ..
                })
                | FillPhase::Pack(PackOne::Walk { walk, .. }) => walk.release(),
                _ => None,
            },
            Stage::Talk {
                talk: Talk::Open(reach),
                ..
            } => reach.release(),
            Stage::Talk { .. } => None,
        }
    }

    /// `Some(fare_held)` once the recovery ended.
    pub(crate) fn step(&mut self, cx: &mut Cx<'_>, logs: &mut VecDeque<String>) -> Option<bool> {
        if interrupted(cx) {
            if let Some(stop) = self.release() {
                cx.emit(stop);
            }
            return Some(false);
        }
        let next = match &mut self.stage {
            Stage::TalkWalk { second, walk } => {
                if !walk.step(cx)? {
                    return Some(false);
                }
                Stage::Talk {
                    second: *second,
                    talk: match Talk::start(&self.facts, logs) {
                        Ok(talk) => talk,
                        Err(done) => return Some(done),
                    },
                }
            }
            Stage::Talk { second, talk } => {
                if !talk.step(&self.facts, cx, logs)? {
                    return Some(false);
                }
                let coins = held(self.facts.coins);
                if *second || coins >= self.required_coins {
                    return Some(coins >= self.required_coins);
                }
                if interrupted(cx) {
                    return Some(false);
                }
                match Fill::start(self.facts.clone(), cx, logs) {
                    Ok(fill) => Stage::Fill(fill),
                    Err(_) => return Some(false),
                }
            }
            Stage::Fill(fill) => {
                if !fill.step(cx, logs)? {
                    return Some(false);
                }
                match Self::talk(&self.facts, true, cx, logs) {
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

/// Frozen `PathPolicy` (`types.ts:147–161`) as the shim marshals it:
/// the id lists as their lengths.
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WalkToPolicy {
    #[serde(default)]
    use_teleports: Option<bool>,
    #[serde(default)]
    distance_before_teleport: Option<i32>,
    #[serde(default)]
    allow_teleport_ids: usize,
    #[serde(default)]
    deny_teleport_ids: usize,
    #[serde(default)]
    use_ships: Option<bool>,
    #[serde(default)]
    use_shortcuts: Option<bool>,
}

/// Frozen `WalkOptions` (`WalkExecutor.ts:113–139`). `log` is the hook.
/// `bankItemCounts` is only the bank planner's input: the host BankBudget
/// fetch reads the live bank itself (FENCE 2026-09-18), so it is not a
/// host input.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WalkToArgs {
    tile: TileArg,
    #[serde(default)]
    radius: Option<i32>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    use_teleport_catalog: Option<bool>,
    #[serde(default)]
    policy: WalkToPolicy,
    /// Frozen `avoidZones`: rectangles and known catalog ids are resolved
    /// at the host arm site with the endpoints and combat level.
    #[serde(default)]
    avoid_zones: Vec<InspectAvoidWire>,
    /// Per-walk named danger-zone exemptions.
    #[serde(default)]
    cross_zones: Vec<String>,
    /// Whether the caller passed `pathFollow` overrides.
    #[serde(default)]
    path_follow: bool,
    #[serde(default)]
    force_repath: bool,
}

/// Frozen `opts?.radius ?? 2` (`WalkExecutor.ts:227`).
const WALK_TO_RADIUS: i32 = 2;
/// Frozen `opts?.timeoutMs ?? 300_000` (`WalkExecutor.ts:228`).
const WALK_TO_MS: u64 = 300_000;

impl WalkToArgs {
    /// Options the host walk cannot honor are refused, never dropped.
    fn refusal(&self) -> Option<String> {
        if let Some(reason) = avoid_refusal(&self.avoid_zones) {
            return Some(reason);
        }
        if self.cross_zones.len() > 8 {
            return Some("crossZones: more than 8 zone ids".into());
        }
        if self.policy.allow_teleport_ids > 0 || self.policy.deny_teleport_ids > 0 {
            return Some("policy.allowTeleportIds/denyTeleportIds: the host router has no teleport id filter".into());
        }
        if self.policy.use_ships == Some(false) || self.policy.use_shortcuts == Some(false) {
            return Some("policy.useShips/useShortcuts false: the host router cannot exclude ships or shortcuts".into());
        }
        if self.path_follow {
            return Some("pathFollow: the host follow has no stall/deviation overrides".into());
        }
        if self.force_repath {
            return Some("forceRepath: the host walk has no forced repath of a live route".into());
        }
        None
    }

    /// Frozen `resolveWalkUseTeleports` (`WalkExecutor.ts:165–177`) gated on
    /// `policy.distanceBeforeTeleport` over the planar span from here
    /// (`policy.ts:61–74`; frozen default 0).
    fn allow_teleports(&self, dest: WorldTile) -> bool {
        resolve_teleports(self.use_teleport_catalog, self.policy.use_teleports)
            && here().is_some_and(|me| {
                teleport_span_allows(self.policy.distance_before_teleport.unwrap_or(0), me, dest)
            })
    }
}

enum WalkToPhase {
    Walking { walk: Walk, retried: bool },
    Recover(Box<Recover>),
}

/// Frozen `Traversal.walkTo(dest, opts)` (`Traversal.ts:80–95`).
pub(crate) struct WalkTo {
    dest: WorldTile,
    radius: i32,
    timeout_ms: u64,
    allow_teleports: bool,
    avoid: Vec<InspectAvoidWire>,
    cross: Vec<String>,
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
    const WALKING_OPERATION: bool = true;
    const CALLBACKS: &'static [&'static str] = &["log", "sustain"];
    /// Frozen `log(...)` is not awaited; `await Sustain.run()` is.
    const SYNC_HOOKS: &'static [usize] = &[LOG];
    type Args = WalkToArgs;
    type Output = bool;

    fn begin(args: WalkToArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        if let Some(reason) = args.refusal() {
            return Begin::Refuse(reason);
        }
        let dest = WorldTile {
            x: args.tile.x,
            z: args.tile.z,
            level: args.tile.level,
        };
        let radius = args.radius.unwrap_or(WALK_TO_RADIUS);
        let timeout_ms = args.timeout_ms.unwrap_or(WALK_TO_MS);
        let allow_teleports = args.allow_teleports(dest);
        let avoid = args.avoid_zones;
        let cross = args.cross_zones;
        match Walk::begin_avoiding(
            dest,
            radius,
            timeout_ms,
            allow_teleports,
            avoid.clone(),
            cross.clone(),
            cx,
        ) {
            Ok(walk) => Begin::Run(Self {
                dest,
                radius,
                timeout_ms,
                allow_teleports,
                avoid,
                cross,
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
        if cx.user_move_intent_interrupted() {
            if let Some(stop) = Family::release(self) {
                cx.emit(stop);
            }
            return Step::Done(false);
        }
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
            // Frozen follow pass: `EventSignal.pending()` ends the walk
            // before `Sustain.run()` (`WalkExecutor.ts:844–853, 359–362`),
            // the re-walk and the recovery included; the host route stops.
            if interrupted(cx) {
                self.interrupt(cx);
                continue;
            }
            // Frozen `await Sustain.run()` on every follow pass
            // (`WalkExecutor.ts:844–853`) and at the recovery's points
            // (`karamja.ts:55, 91`; `Traversal.ts:130` in its resilient
            // legs), once a tick in every phase.
            if !self.pumped && cx.has(SUSTAIN) {
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

    fn release(&self) -> Option<InteractReq> {
        match &self.phase {
            WalkToPhase::Walking { walk, .. } => Some(walk.release()),
            WalkToPhase::Recover(recover) => recover.release(),
        }
    }
}

impl WalkTo {
    /// Frozen `'interrupted'`: the walk returns false and stops its walker.
    fn interrupt(&mut self, cx: &mut Cx<'_>) {
        if let Some(stop) = Family::release(self) {
            cx.emit(stop);
        }
        if matches!(self.phase, WalkToPhase::Walking { .. }) {
            self.logs
                .push_back("walk interrupted — a random event is being handled".into());
        }
        self.result = Some(false);
    }

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
                    Ok(recover) => self.phase = WalkToPhase::Recover(Box::new(recover)),
                    Err(_) => self.result = Some(false),
                }
            }
            WalkToPhase::Recover(recover) => match recover.step(cx, &mut self.logs) {
                None => {}
                Some(false) => self.result = Some(false),
                Some(true) => match Walk::begin_avoiding(
                    self.dest,
                    self.radius,
                    self.timeout_ms,
                    self.allow_teleports,
                    self.avoid.clone(),
                    self.cross.clone(),
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
        let capacity = 13;
        let read = |text: &str| read_crate_messages(text, capacity);
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
                bananas: capacity
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
