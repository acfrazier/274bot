//! Reachability guard for the gather sites. Every published site must have a
//! tile beside its resource spots that nav's real search reaches from
//! Lumbridge for a maxed account: every skill at 99, members, every quest the
//! pack's edges name complete, the varps and items those edges read, a large
//! coin stack, and only the worn items the site itself declares. A site no
//! such account reaches is not offered and must be hidden (a `drop` row in
//! `SITE_OVERRIDES`, tools/game-data/extractors/gathering.ts) or carry the
//! requirement that explains it.
//!
//! The account is built from the pack's own edges, so each edge's gate is met
//! by construction: quest-complete gates read `WorldState::quests`, varp gates
//! the largest value any edge reads, item gates the largest count, skill gates
//! 99. The pack carries no quest-stage gates, so no progress evidence is needed.
//!
//! The gatherer stands beside a resource (tree, rock, spot), so the target set
//! is the standable tiles of the region grown by one tile. One flood per
//! distinct worn-requirement set and origin answers every site of the set at
//! once. Origins are Lumbridge plus every landing of a teleport the account
//! may take: a teleport leaves from any tile, so a place reachable after a
//! teleport is reachable. nav's own metered teleport search exhausts its node
//! budget on a flood this wide, which is why the landings are origins here.
//! The danger-zone table is left out of the graph: zones are a user opt-out
//! and each zone-restricted target would otherwise cost its own full flood.
//!
//! A second flood runs on the same pack with every requirement stripped (the
//! open graph), over the published sites and the hidden ones together. Three
//! checks follow. A published site the open graph reaches that the maxed
//! account does not means the account setup here is wrong, or the site needs
//! something it does not declare. A published site the open graph cannot reach
//! is a gap in the pack that no account crosses: hide it. A hidden site the
//! open graph reaches means a later nav pack gained its entrance: restore it.
//! The hidden sites' regions come from the generator
//! (`tools/game-data/gather-sites-hidden.json`), so a hidden id stays checked
//! after it leaves the published data. Every 0.2.0 id is published or hidden,
//! never both and never reused.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use api::game_data::{self, GatherSiteOption, GatherSiteRequirement};
use api::gather_methods::SceneRegionInput;
use api::selected::ClientRevision;
use api::WorldTile;
use nav::router::{find_many_with, FindOptions, TargetError};
use nav::world::NavWorld;
use nav::WorldState;

const LUMBRIDGE: WorldTile = WorldTile {
    x: 3222,
    z: 3218,
    level: 0,
};

/// Stat ids nav reads at 99 (`WorldState::stats`). Ids past the real skills are
/// never read by a walk, so setting them is harmless.
const SKILL_IDS: std::ops::RangeInclusive<i32> = 0..=30;
const COINS: i32 = 995;
const COIN_STACK: i32 = 1_000_000;
/// Tiles beside a region's resource spots that a gatherer can stand on.
const MARGIN: i32 = 1;

/// The sites published in 0.2.0 (id -> region), saved with the generator tests.
const SITES_0_2_0: &str = include_str!("../../../tools/game-data/gather-sites-0.2.0.json");
/// The hidden sites' regions per revision, written by the generator.
const SITES_HIDDEN: &str = include_str!("../../../tools/game-data/gather-sites-hidden.json");

/// The worn items a site declares it needs. Skill and quest requirements are
/// already met by the maxed character.
fn worn_for(site: &GatherSiteOption) -> Vec<i32> {
    let mut worn: Vec<i32> = site
        .requires
        .iter()
        .filter_map(|requirement| match requirement {
            GatherSiteRequirement::Worn { item, .. } => Some(*item),
            GatherSiteRequirement::Skill { .. } | GatherSiteRequirement::Quest { .. } => None,
        })
        .collect();
    worn.sort_unstable();
    worn.dedup();
    worn
}

/// The maxed account, from the pack's own edges and teleports. Worn items are
/// left to the caller.
fn maxed_state(world: &NavWorld) -> WorldState {
    let mut state = WorldState::empty().with_map_members(true);
    state.stats = SKILL_IDS.map(|skill| (skill, 99)).collect();
    state.combat_level = Some(126);
    let mut varps: HashMap<i32, i32> = HashMap::new();
    let mut items: HashMap<i32, i32> = HashMap::new();
    for edge in world.graph.edges.iter().chain(&world.graph.teleports) {
        state.quests.extend(edge.quest_req.iter().cloned());
        for &(varp, min) in &edge.varp_req {
            let value = varps.entry(varp).or_insert(min);
            *value = (*value).max(min);
        }
        for &(item, count) in edge.item_req.iter().chain(&edge.consumed_req) {
            let value = items.entry(item).or_insert(count);
            *value = (*value).max(count);
        }
    }
    state.varps = varps;
    state.inv = items;
    state.inv.insert(COINS, COIN_STACK);
    state
}

/// Lumbridge plus the distinct landings of every teleport `state` may take.
fn origins(world: &NavWorld, state: &WorldState) -> Vec<WorldTile> {
    let mut seen = HashSet::from([LUMBRIDGE]);
    let mut origins = vec![LUMBRIDGE];
    for edge in world
        .graph
        .teleports
        .iter()
        .filter(|edge| state.allows(edge))
    {
        if seen.insert(edge.to) {
            origins.push(edge.to);
        }
    }
    origins
}

/// The 289 nav pack for the ignored reachability test, from `NAV_PACK`.
fn real_nav_pack() -> PathBuf {
    PathBuf::from(std::env::var_os("NAV_PACK").expect("set NAV_PACK to the real 289 nav pack"))
}

/// The real pack with its flags, without the danger-zone table.
fn load_world(pack: &std::path::Path) -> NavWorld {
    let mut world = NavWorld::load_pack(pack).expect("real 289 pack");
    let (_origin, _walls, _height, flags) = nav::pack::decode_flags_sidecar(
        &std::fs::read(pack.with_extension("navflags")).expect("raw flags"),
    )
    .expect("decode flags");
    world.collision.attach_flags(flags);
    world.graph.zones = None;
    world
}

/// The same pack with every requirement removed: what no account can reach.
fn open_world(pack: &std::path::Path) -> NavWorld {
    let mut world = load_world(pack);
    let graph = &mut world.graph;
    for edge in graph.edges.iter_mut().chain(graph.teleports.iter_mut()) {
        edge.skill_req.clear();
        edge.item_req.clear();
        edge.consumed_req.clear();
        edge.quest_req.clear();
        edge.varp_req.clear();
        edge.worn_req.clear();
        edge.worn_all_req.clear();
        edge.members_req = false;
        edge.wildy_cap = None;
        edge.quest_gates = None;
    }
    world
}

/// A region to reach: a published site, or a hidden one from the generator's list.
struct Target<'a> {
    id: &'a str,
    region: SceneRegionInput,
}

impl<'a> From<&'a GatherSiteOption> for Target<'a> {
    fn from(site: &'a GatherSiteOption) -> Self {
        Target {
            id: site.id.as_str(),
            region: site.region,
        }
    }
}

/// Ids of the `targets` with a standable tile beside their region that a
/// search from `origins(world, state)` reaches. Panics when a search is
/// undecided.
fn reached<'a>(world: &NavWorld, state: &WorldState, targets: &[Target<'a>]) -> HashSet<&'a str> {
    let mut tiles = Vec::new();
    let mut owner = Vec::new();
    for (index, target) in targets.iter().enumerate() {
        let region = &target.region;
        for x in (region.min_x - MARGIN)..=(region.max_x + MARGIN) {
            for z in (region.min_z - MARGIN)..=(region.max_z + MARGIN) {
                let tile = WorldTile {
                    x,
                    z,
                    level: region.level,
                };
                if world.collision.standable(tile) {
                    tiles.push(tile);
                    owner.push(index);
                }
            }
        }
    }
    let mut reachable = HashSet::new();
    if tiles.is_empty() {
        return reachable;
    }
    // Teleports are the origins, so the flood itself walks and uses transports only.
    let options = FindOptions {
        allow_wilderness: true,
        ..FindOptions::default()
    };
    for origin in origins(world, state) {
        let routes = find_many_with(
            &world.collision,
            &world.graph,
            origin,
            &tiles,
            options,
            state,
        );
        assert!(
            routes.complete(),
            "the search from {origin:?} was cut short"
        );
        for (result, &index) in routes.results().iter().zip(&owner) {
            match result {
                Ok(_) => {
                    reachable.insert(targets[index].id);
                }
                Err(TargetError::NoPath) => {}
                Err(error) => {
                    panic!("the search from {origin:?} did not decide a target: {error:?}")
                }
            }
        }
    }
    reachable
}

/// The regions of the sites the generator hides (id -> `[min_x, min_z, max_x,
/// max_z, level]`), for the 289 revision.
fn hidden_sites() -> Vec<(String, SceneRegionInput)> {
    let mut by_revision: BTreeMap<String, BTreeMap<String, [i32; 5]>> =
        serde_json::from_str(SITES_HIDDEN).expect("hidden site snapshot");
    by_revision
        .remove("289")
        .expect("289 hidden sites")
        .into_iter()
        .map(|(id, [min_x, min_z, max_x, max_z, level])| {
            (
                id,
                SceneRegionInput {
                    min_x,
                    min_z,
                    max_x,
                    max_z,
                    level,
                },
            )
        })
        .collect()
}

#[test]
#[ignore = "requires the real 289 nav pack in NAV_PACK (with its .navflags beside it)"]
fn every_published_gather_site_is_reachable_and_every_hidden_one_is_not() {
    let selected = game_data::for_revision(ClientRevision::R289).expect("289 selected data");
    let pack = real_nav_pack();
    let world = load_world(&pack);
    let started = Instant::now();

    let published = selected.gather_sites();
    let hidden = hidden_sites();
    let line = |id: &str, label: &str, worn: &[i32], region: &SceneRegionInput| {
        format!("{id} \"{label}\" worn {worn:?} region {region:?}")
    };

    // A retired id is never reused: every 0.2.0 id is published or hidden, and never both.
    let old: HashSet<String> =
        serde_json::from_str::<BTreeMap<String, BTreeMap<String, serde_json::Value>>>(SITES_0_2_0)
            .expect("0.2.0 site snapshot")
            .remove("289")
            .expect("289 snapshot")
            .into_keys()
            .collect();
    let published_ids: HashSet<&str> = published.iter().map(|site| site.id.as_str()).collect();
    let hidden_ids: HashSet<&str> = hidden.iter().map(|(id, _)| id.as_str()).collect();
    let both: Vec<&&str> = published_ids.intersection(&hidden_ids).collect();
    assert!(both.is_empty(), "ids both published and hidden: {both:?}");
    let missing: Vec<&String> = old
        .iter()
        .filter(|id| !published_ids.contains(id.as_str()) && !hidden_ids.contains(id.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "0.2.0 site ids neither published nor hidden: {missing:?}"
    );

    // The maxed account: one flood per distinct worn-requirement set, published sites only.
    let mut groups: BTreeMap<Vec<i32>, Vec<Target>> = BTreeMap::new();
    for site in published {
        groups
            .entry(worn_for(site))
            .or_default()
            .push(Target::from(site));
    }
    let mut maxed: HashSet<&str> = HashSet::new();
    for (worn, targets) in &groups {
        let mut state = maxed_state(&world);
        state.worn = worn.iter().copied().collect();
        maxed.extend(reached(&world, &state, targets));
    }

    // The open graph, every published and hidden site at once.
    let all: Vec<Target> = published
        .iter()
        .map(Target::from)
        .chain(hidden.iter().map(|(id, region)| Target {
            id: id.as_str(),
            region: *region,
        }))
        .collect();
    let open = reached(&open_world(&pack), &WorldState::empty(), &all);
    println!(
        "gather_sites_reach: {} published site(s), {} worn-requirement group(s), {} reached by the maxed account, {} reached by the open graph; {} hidden site(s), {} reached by the open graph; {:.1}s",
        published.len(),
        groups.len(),
        maxed.len(),
        published.iter().filter(|site| open.contains(site.id.as_str())).count(),
        hidden.len(),
        hidden.iter().filter(|(id, _)| open.contains(id.as_str())).count(),
        started.elapsed().as_secs_f64()
    );

    // Account setup first: the open graph reaches it, so the maxed account must.
    let blocked: Vec<String> = published
        .iter()
        .filter(|site| open.contains(site.id.as_str()) && !maxed.contains(site.id.as_str()))
        .map(|site| line(&site.id, &site.label, &worn_for(site), &site.region))
        .collect();
    assert!(
        blocked.is_empty(),
        "{} gather site(s) are reachable with every requirement removed but not by the maxed account; add the requirement the walk enforces (SITE_OVERRIDES `requires`) or hide the site:\n{}",
        blocked.len(),
        blocked.join("\n")
    );

    // A published site no account reaches must be hidden.
    let stranded: Vec<String> = published
        .iter()
        .filter(|site| !open.contains(site.id.as_str()))
        .map(|site| line(&site.id, &site.label, &worn_for(site), &site.region))
        .collect();
    assert!(
        stranded.is_empty(),
        "{} published gather site(s) are unreachable on the pack for any account; hide each with a SITE_OVERRIDES drop row (tools/game-data/extractors/gathering.ts) naming the entrance the route finder lacks:\n{}",
        stranded.len(),
        stranded.join("\n")
    );

    // A hidden site the pack now reaches must come back.
    let restorable: Vec<String> = hidden
        .iter()
        .filter(|(id, _)| open.contains(id.as_str()))
        .map(|(id, region)| format!("{id} region {region:?}"))
        .collect();
    assert!(
        restorable.is_empty(),
        "{} hidden gather site(s) are reachable on the pack now (it gained the entrance): restore each by removing its SITE_OVERRIDES drop row in tools/game-data/extractors/gathering.ts, then regenerate the game data:\n{}",
        restorable.len(),
        restorable.join("\n")
    );
}
