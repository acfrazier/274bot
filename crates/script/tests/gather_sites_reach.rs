//! Reachability guard for the published gather sites. Every site must have a
//! tile beside its resource spots that nav's real search reaches from
//! Lumbridge for a maxed account: every skill at 99, members, every quest the
//! pack's edges name complete, the varps and items those edges read, a large
//! coin stack, and only the worn items the site itself declares. A site no
//! such account reaches is not offered and must be dropped (or carry the
//! requirement that explains it).
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
//! open graph): what even that cannot reach is a gap in the pack, and no
//! account reaches it. Two checks follow. A site the open graph reaches that
//! the maxed account does not means the account setup here is wrong, or the
//! site needs something it does not declare; that is judged first, on every
//! 0.2.0 site, before any new site. A new site the open graph cannot reach is
//! not offered. A 0.2.0 site the open graph cannot reach keeps its id (saved
//! Gatherer cards store it) and is only listed.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use api::game_data::{self, GatherSiteOption, GatherSiteRequirement};
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

/// Ids of the `sites` with a standable tile beside their region that a search
/// from `origins(world, state)` reaches. Panics when a search is undecided.
fn reached<'a>(
    world: &NavWorld,
    state: &WorldState,
    sites: &[&'a GatherSiteOption],
) -> HashSet<&'a str> {
    let mut targets = Vec::new();
    let mut owner = Vec::new();
    for (index, site) in sites.iter().enumerate() {
        let region = &site.region;
        for x in (region.min_x - MARGIN)..=(region.max_x + MARGIN) {
            for z in (region.min_z - MARGIN)..=(region.max_z + MARGIN) {
                let tile = WorldTile {
                    x,
                    z,
                    level: region.level,
                };
                if world.collision.standable(tile) {
                    targets.push(tile);
                    owner.push(index);
                }
            }
        }
    }
    let mut reachable = HashSet::new();
    if targets.is_empty() {
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
            &targets,
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
                    reachable.insert(sites[index].id.as_str());
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

#[test]
#[ignore = "requires the real 289 nav pack in NAV_PACK (with its .navflags beside it)"]
fn every_published_gather_site_is_reachable_from_lumbridge() {
    let selected = game_data::for_revision(ClientRevision::R289).expect("289 selected data");
    let pack = real_nav_pack();
    let world = load_world(&pack);
    let started = Instant::now();

    // The maxed account: one flood per distinct worn-requirement set.
    let mut groups: BTreeMap<Vec<i32>, Vec<&GatherSiteOption>> = BTreeMap::new();
    for site in selected.gather_sites() {
        groups.entry(worn_for(site)).or_default().push(site);
    }
    let mut maxed: HashSet<&str> = HashSet::new();
    for (worn, sites) in &groups {
        let mut state = maxed_state(&world);
        state.worn = worn.iter().copied().collect();
        maxed.extend(reached(&world, &state, sites));
    }

    // The open graph, every site at once.
    let all: Vec<&GatherSiteOption> = selected.gather_sites().iter().collect();
    let open = reached(&open_world(&pack), &WorldState::empty(), &all);
    println!(
        "gather_sites_reach: {} site(s), {} worn-requirement group(s), {} reached by the maxed account, {} by the open graph, {:.1}s",
        all.len(),
        groups.len(),
        maxed.len(),
        open.len(),
        started.elapsed().as_secs_f64()
    );

    let old: HashSet<String> =
        serde_json::from_str::<BTreeMap<String, BTreeMap<String, serde_json::Value>>>(SITES_0_2_0)
            .expect("0.2.0 site snapshot")
            .remove("289")
            .expect("289 snapshot")
            .into_keys()
            .collect();
    let line = |site: &GatherSiteOption| {
        format!(
            "{} {} \"{}\" worn {:?} region {:?}",
            site.skill,
            site.id,
            site.label,
            worn_for(site),
            site.region
        )
    };
    let missing: Vec<&String> = old
        .iter()
        .filter(|id| !all.iter().any(|site| &site.id == *id))
        .collect();
    assert!(
        missing.is_empty(),
        "0.2.0 site ids missing from the published data: {missing:?}"
    );

    // Account setup first: the open graph reaches it, so the maxed account must.
    let mut blocked_old = Vec::new();
    let mut blocked_new = Vec::new();
    for site in all
        .iter()
        .filter(|site| open.contains(site.id.as_str()) && !maxed.contains(site.id.as_str()))
    {
        if old.contains(&site.id) {
            blocked_old.push(line(site));
        } else {
            blocked_new.push(line(site));
        }
    }
    assert!(
        blocked_old.is_empty(),
        "the account setup is wrong, not the sites: {} site(s) published in 0.2.0 are reachable with every requirement removed but not by the maxed account:\n{}",
        blocked_old.len(),
        blocked_old.join("\n")
    );
    assert!(
        blocked_new.is_empty(),
        "{} new gather site(s) are reachable with every requirement removed but not by the maxed account; add the requirement the walk enforces (SITE_OVERRIDES `requires`) or drop them:\n{}",
        blocked_new.len(),
        blocked_new.join("\n")
    );

    let gaps: Vec<&GatherSiteOption> = all
        .iter()
        .copied()
        .filter(|site| !open.contains(site.id.as_str()))
        .collect();
    let (grandfathered, new): (Vec<_>, Vec<_>) =
        gaps.into_iter().partition(|site| old.contains(&site.id));
    println!(
        "gather_sites_reach: {} site(s) published in 0.2.0 are unreachable on the pack for any account and keep their ids:\n{}",
        grandfathered.len(),
        grandfathered.iter().map(|site| line(site)).collect::<Vec<_>>().join("\n")
    );
    assert!(
        new.is_empty(),
        "{} new gather site(s) are unreachable on the pack for any account; drop them:\n{}",
        new.len(),
        new.iter()
            .map(|site| line(site))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
