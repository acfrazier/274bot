use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::*;
use crate::router::{find_with, FindOptions, Leg};
use crate::transport::{TransportEdge, TransportKind};
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::{Client, ClientConfig};
use client::config::if_type::{ComponentType, IfType, IfTypeMut};
use client::config::varp_type::VarpType;
use client::io::ServerProt;

/// A cache-less client: `Cache::default()`, no network, no ifaces.
/// Everything the snapshot families need is planted by hand. The
/// cache dir is a unique scratch dir — a real cache (e.g. stray
/// `config`/`interface` jags under `/tmp`) would seed real ifaces
/// and break the first-`TYPE_INV` reads.
fn client() -> Client {
    let cache_dir = std::env::temp_dir().join(format!(
        "274bot-nav-worldstate-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: cache_dir.to_str().unwrap().into(),
        members: true,
        lowmem: false,
    })
}

/// A toll-shaped edge: an Al Kharid border-gate crossing that costs
/// 10 coins, plus one requirement of each other kind so the gating
/// test covers all five vectors.
fn gated_edge() -> TransportEdge {
    TransportEdge {
        kind: TransportKind::Door,
        at: WorldTile {
            x: 3268,
            z: 3227,
            level: 0,
        },
        to: WorldTile {
            x: 3269,
            z: 3227,
            level: 0,
        },
        loc_id: 2882,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: vec![(6, 25)],  // Magic 25 (spell teleports)
        item_req: vec![(995, 10)], // the 10-coin toll
        quest_req: vec!["Rune Mysteries".to_string()],
        varp_req: vec![(150, 160)], // Grand Tree complete
        worn_req: vec![1712],       // a charged glory
        members_req: false,
    }
}

#[test]
fn completed_prince_ali_rescue_waives_only_the_real_alkharid_toll() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let at = WorldTile {
        x: 3268,
        z: 3227,
        level: 0,
    };
    let toll = world
        .graph
        .edges
        .iter()
        .find(|e| e.loc_id == 2882 && e.at == at && e.to.x > at.x && e.item_req == vec![(995, 10)])
        .expect("real pack contains the paid eastbound Al Kharid toll crossing");
    let free = world
        .graph
        .edges
        .iter()
        .find(|e| e.loc_id == 2882 && e.at == at && e.to == toll.to && e.item_req.is_empty())
        .expect("real pack contains the quest-waived crossing alongside the paid one");
    assert_eq!(free.quest_req, ["Prince Ali Rescue"]);
    assert!(
        free.varp_req.is_empty(),
        "princequest is not transmitted live"
    );
    let unpaid = WorldState::empty();
    assert!(!unpaid.allows(toll));
    assert!(!unpaid.allows(free), "an unknown quest still needs coins");
    let injected_varp = WorldState {
        varps: HashMap::from([(273, 110)]),
        ..WorldState::empty()
    };
    assert!(
        !injected_varp.allows(free),
        "a fabricated non-transmitted varp cannot waive the toll"
    );
    let short = WorldState {
        varps: HashMap::from([(273, 100)]),
        inv: HashMap::from([(995, 9)]),
        ..WorldState::empty()
    };
    assert!(!short.allows(free));
    assert!(!short.allows(toll));
    // The quest journal is the fact the live client carries: a green
    // quest-list component becomes a snapshot quest state without varps.
    let mut c = client();
    c.side_icon[2] = 700;
    c.set_iface(
        700,
        IfType {
            children: Some(vec![701]),
            ..Default::default()
        },
    );
    c.set_iface(
        701,
        IfType {
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        701,
        IfTypeMut {
            text: free.quest_req[0].clone(),
            colour: QUEST_COMPLETE_COLOUR,
            ..Default::default()
        },
    );
    c.bump_gens(ServerProt::IF_OPENMAIN);
    let mut snapshot = GameSnapshot::new();
    assert!(snapshot.rebuild(&c));
    let completed = WorldState::from_snapshot(&snapshot);
    assert!(completed.varps.is_empty());
    assert!(completed.quests.contains("Prince Ali Rescue"));
    assert!(
        !completed.allows(toll),
        "the paid edge always requires coins"
    );
    assert!(
        completed.allows(free),
        "the live journal fact enables the free alternative"
    );
    let paid = WorldState {
        inv: HashMap::from([(995, 10)]),
        ..WorldState::empty()
    };
    assert!(paid.allows(toll));
    assert!(!paid.allows(free));
    let south_gate = world
        .graph
        .edges
        .iter()
        .find(|edge| {
            edge.loc_id == 2883 && edge.item_req.is_empty() && edge.quest_req == free.quest_req
        })
        .expect("other Al Kharid toll gate also has a quest-waived crossing");
    assert!(!unpaid.allows(south_gate));
    assert!(completed.allows(south_gate));
    // Isolate the real packed edge in a sealed two-side corridor so an
    // unrelated long-world detour cannot count as crossing this toll gate.
    let mut flags = vec![0u32; 4 * 5 * 5];
    for z in 0..5 {
        flags[z * 5 + 2] = crate::collision::SQ_BLOCKED;
    }
    let (walk, blocked) = crate::collision::pack_walk(&flags);
    let collision = crate::collision::WorldCollision {
        origin: WorldTile {
            x: 3266,
            z: 3225,
            level: 0,
        },
        width: 5,
        height: 5,
        walk,
        blocked,
        flags: None,
    };
    let graph = crate::transport::TransportGraph {
        edges: vec![free.clone(), toll.clone()],
        at: HashMap::from([(at, vec![0, 1])]),
        teleports: vec![],
    };
    let from = WorldTile {
        x: 3267,
        z: 3227,
        level: 0,
    };
    let to = toll.to;
    let route = find_with(
        &collision,
        &graph,
        from,
        to,
        FindOptions::default(),
        &completed,
    )
    .expect("quest journal admits the toll route with no coins");
    assert!(route
        .legs
        .iter()
        .any(|leg| matches!(leg, Leg::Transport { edge } if edge.item_req.is_empty() && edge.quest_req == free.quest_req)));
    assert!(
        find_with(
            &collision,
            &graph,
            from,
            to,
            FindOptions::default(),
            &unpaid
        )
        .is_err(),
        "without quest and coins the gate cannot be crossed"
    );
    assert!(
        find_with(&collision, &graph, from, to, FindOptions::default(), &short).is_err(),
        "neither nine coins nor an untransmitted varp admits a crossing"
    );
    assert!(
        find_with(&collision, &graph, from, to, FindOptions::default(), &paid).is_ok(),
        "ten coins still admit the paid alternative"
    );
    assert_eq!(
        crate::router::find_missing_item_reqs(
            &collision,
            &graph,
            from,
            to,
            FindOptions::default(),
            &unpaid
        ),
        Some(vec![crate::router::MissingReq::Carry {
            id: 995,
            count: 10
        }])
    );
    assert_eq!(
        crate::router::find_missing_item_reqs(
            &collision,
            &graph,
            from,
            to,
            FindOptions::default(),
            &short
        ),
        Some(vec![crate::router::MissingReq::Carry {
            id: 995,
            count: 10
        }])
    );
    assert_eq!(
        crate::router::find_missing_item_reqs(
            &collision,
            &graph,
            from,
            to,
            FindOptions::default(),
            &completed
        ),
        Some(vec![]),
        "bank-fetch diagnosis must not request waived toll coins"
    );
    let mut other = toll.clone();
    other.loc_id = 4031;
    assert!(
        !completed.allows(&other),
        "quest must not waive another gate's coins"
    );
}

/// `allows` passes only when every requirement kind is satisfied;
/// each missing or short fact fails the edge on its own.
#[test]
fn allows_requires_every_req_kind() {
    let e = gated_edge();
    let mut s = WorldState {
        inv: HashMap::from([(995, 10)]),
        worn: HashSet::from([1712]),
        stats: HashMap::from([(6, 25)]),
        varps: HashMap::from([(150, 160)]),
        quests: HashSet::from(["Rune Mysteries".to_string()]),
        map_members: false,
    };
    assert!(s.allows(&e), "all facts present");
    // One missing fact at a time, each failing closed.
    s.inv.remove(&995);
    assert!(!s.allows(&e), "no coins -> toll edge refused");
    s.inv.insert(995, 9);
    assert!(!s.allows(&e), "9 coins < 10 -> still refused");
    s.inv.insert(995, 10);
    s.worn.clear();
    assert!(!s.allows(&e), "nothing worn -> refused");
    s.worn.insert(1712);
    s.stats.clear();
    assert!(!s.allows(&e), "no stats -> refused");
    s.stats.insert(6, 24);
    assert!(!s.allows(&e), "Magic 24 < 25 -> refused");
    s.stats.insert(6, 25);
    s.varps.clear();
    assert!(!s.allows(&e), "no varps -> refused");
    s.varps.insert(150, 159);
    assert!(!s.allows(&e), "varp 159 < 160 -> refused");
    s.varps.insert(150, 160);
    s.quests.clear();
    assert!(!s.allows(&e), "quest not done -> refused");
    s.quests.insert("Rune Mysteries".to_string());
    assert!(s.allows(&e), "all facts back -> passes");
}

/// `worn_req` is any-of: a slash-weapon web hop lists every blade
/// with a slash anim; wearing one of them is enough. Empty is no
/// worn gate. A single Dramen staff stays a one-id list.
#[test]
fn worn_req_passes_when_any_listed_obj_is_worn() {
    let mut e = gated_edge();
    e.skill_req.clear();
    e.item_req.clear();
    e.quest_req.clear();
    e.varp_req.clear();
    e.worn_req = vec![1277, 1321]; // bronze sword, bronze scimitar
    let mut s = WorldState::empty();
    assert!(!s.allows(&e), "nothing worn");
    s.worn.insert(1321);
    assert!(s.allows(&e), "one of the listed blades is enough");
    s.worn.clear();
    s.worn.insert(946);
    assert!(!s.allows(&e), "an unlisted obj does not count");
    e.worn_req.clear();
    assert!(
        WorldState::empty().allows(&e),
        "empty worn_req is not a gate"
    );
}

/// An empty state proves nothing: even a req-free edge passes, every
/// gated edge is refused.
#[test]
fn empty_state_allows_nothing_gated() {
    let e = gated_edge();
    assert!(!WorldState::empty().allows(&e));
    let free = TransportEdge {
        skill_req: vec![],
        item_req: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        ..gated_edge()
    };
    assert!(
        WorldState::empty().allows(&free),
        "req-free edges stay usable"
    );
}

/// `from_snapshot` maps the snapshot's inv, equipment, stats, varps,
/// and quest-status views into the gating facts; an in-progress quest
/// (red journal text) is not complete.
#[test]
fn from_snapshot_builds_inv_worn_stats_varps_and_quests() {
    let mut c = client();

    // Inventory: a TYPE_INV iface carrying 10 coins (obj 995 →
    // stored 996, the +1 convention).
    let inv_id = c.push_iface(IfType {
        r#type: ComponentType::TYPE_INV,
        ..Default::default()
    });
    c.set_iface_mut(
        inv_id,
        IfTypeMut {
            link_obj_type: Some(vec![996, 0]),
            link_obj_number: Some(vec![10, 0]),
            ..Default::default()
        },
    );
    // Worn: the equipment tab (side 4) root with a TYPE_INV child
    // carrying a charged glory (obj 1712 → stored 1713).
    c.side_icon[4] = 500;
    c.set_iface(
        500,
        IfType {
            children: Some(vec![501]),
            ..Default::default()
        },
    );
    c.set_iface(
        501,
        IfType {
            r#type: ComponentType::TYPE_INV,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        501,
        IfTypeMut {
            link_obj_type: Some(vec![1713]),
            link_obj_number: Some(vec![1]),
            ..Default::default()
        },
    );
    // Stats: Magic (6) effective 25.
    c.stat_effective_level[6] = 25;
    // Varps: 150 → 160 (Grand Tree complete), with the defs the
    // snapshot's varp view walks.
    c.var = vec![0; 400];
    c.var[150] = 160;
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        cache.varps = (0..400).map(|_| VarpType::default()).collect();
    }
    // Quests: the quest tab (side 2) root with two TYPE_TEXT entries —
    // "Rune Mysteries" still red (not started, stored `0xF80000`),
    // "Lost City" green (stored `0xF800`, the client's if_setcolour
    // decoding of the journal's 15-bit green).
    c.side_icon[2] = 700;
    c.set_iface(
        700,
        IfType {
            children: Some(vec![701, 702]),
            ..Default::default()
        },
    );
    c.set_iface(
        701,
        IfType {
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        701,
        IfTypeMut {
            text: "Rune Mysteries".into(),
            colour: 0xF80000,
            ..Default::default()
        },
    );
    c.set_iface(
        702,
        IfType {
            r#type: ComponentType::TYPE_TEXT,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        702,
        IfTypeMut {
            text: "Lost City".into(),
            colour: QUEST_COMPLETE_COLOUR,
            ..Default::default()
        },
    );

    c.bump_gens(ServerProt::UPDATE_INV_FULL); // inv gen
    c.bump_gens(ServerProt::VARP_SMALL); // varp gen
    c.bump_gens(ServerProt::UPDATE_RUNENERGY); // stat gen
    c.bump_gens(ServerProt::IF_OPENMAIN); // iface gen (quest + equipment)

    let mut snap = GameSnapshot::new();
    assert!(snap.rebuild(&c), "every planted family rebuilds");
    let s = WorldState::from_snapshot(&snap);

    assert_eq!(s.inv.get(&995), Some(&10));
    assert!(s.worn.contains(&1712), "glory worn");
    assert_eq!(s.stats.get(&6), Some(&25));
    assert_eq!(s.varps.get(&150), Some(&160));
    assert!(s.quests.contains("Lost City"), "green quest done");
    assert!(!s.quests.contains("Rune Mysteries"), "red quest not done");

    // Gating through the built state: an edge the state proves (the
    // completed "Lost City", not the in-progress "Rune Mysteries")
    // passes; the same edge with a missing coin does not.
    let e = TransportEdge {
        quest_req: vec!["Lost City".to_string()],
        ..gated_edge()
    };
    assert!(s.allows(&e));
    let poor = WorldState {
        inv: HashMap::new(),
        ..s.clone()
    };
    assert!(!poor.allows(&e));
    assert!(
        !s.map_members,
        "from_snapshot must not invent WORLD membership"
    );
}

/// A `members_req` edge is refused until `map_members` is true.
/// BankBudget's relaxed carry/wear arm still cannot fetch membership.
#[test]
fn members_req_refuses_until_map_members_is_true() {
    let mut e = gated_edge();
    e.skill_req.clear();
    e.item_req.clear();
    e.quest_req.clear();
    e.varp_req.clear();
    e.worn_req.clear();
    e.members_req = true;
    assert!(
        !WorldState::empty().allows(&e),
        "empty state is not a members world"
    );
    assert!(
        !WorldState::empty().allows_without_carry_worn(&e),
        "BankBudget cannot fetch membership"
    );
    let open = WorldState::empty().with_map_members(true);
    assert!(open.allows(&e));
    assert!(open.allows_without_carry_worn(&e));
    e.members_req = false;
    assert!(
        WorldState::empty().allows(&e),
        "existing edges stay ungated"
    );
}
