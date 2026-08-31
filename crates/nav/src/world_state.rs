//! The gating facts a [`crate::router::find`] search checks transport
//! edges against, built from a `GameSnapshot` at find time: inventory
//! stacks, worn items, skill levels, varps, and completed quests.
//!
//! Missing facts fail closed — [`WorldState::allows`] is false for any
//! requirement the state cannot prove, so an unpaid toll, an incomplete
//! quest, or a missing level never routes. There is no "assume yes".

use std::collections::{HashMap, HashSet};

use api::snapshot::GameSnapshot;

use crate::transport::TransportEdge;

/// The quest journal's "completed" green (`0x00FF00`): a quest-tab entry
/// painted this colour is done. The not-started red and started yellow
/// are not.
pub const QUEST_COMPLETE_COLOUR: i32 = 0x00FF00;

/// The facts a route may be gated against. [`WorldState::from_snapshot`]
/// fills it from a live `GameSnapshot`; anything the snapshot does not
/// carry stays empty and an edge needing it fails closed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorldState {
    /// obj id → stacked count carried (the snapshot's `inv` family).
    pub inv: HashMap<i32, i32>,
    /// obj ids currently worn (the snapshot's `equipment` family).
    pub worn: HashSet<i32>,
    /// skill id → effective level (the snapshot's `stats` family).
    pub stats: HashMap<i32, i32>,
    /// varp index → value (the snapshot's `varps` family).
    pub varps: HashMap<i32, i32>,
    /// Quest names completed (green in the quest journal).
    pub quests: HashSet<String>,
}

impl WorldState {
    /// The fail-closed empty state: no facts, so no requirement passes.
    pub fn empty() -> Self {
        WorldState::default()
    }

    /// Build from a `GameSnapshot`'s inv, equipment, stats, varps, and
    /// quest-status views. Families the snapshot has not loaded (an empty
    /// inv, an unopened quest tab, …) stay empty — edges needing them
    /// fail closed.
    pub fn from_snapshot(s: &GameSnapshot) -> Self {
        let mut inv = HashMap::new();
        for &(id, n) in s.inv() {
            *inv.entry(id).or_insert(0) += n;
        }
        let worn: HashSet<i32> = s
            .equipment()
            .iter()
            .filter(|it| it.count > 0)
            .map(|it| it.def.id)
            .collect();
        let stats: HashMap<i32, i32> = s
            .stats()
            .iter()
            .map(|st| (st.index, st.effective))
            .collect();
        let varps: HashMap<i32, i32> = s.varps().iter().map(|v| (v.index, v.value)).collect();
        let quests: HashSet<String> = s
            .quest_statuses()
            .iter()
            .filter(|q| q.colour == QUEST_COMPLETE_COLOUR)
            .map(|q| q.name.clone())
            .collect();
        WorldState {
            inv,
            worn,
            stats,
            varps,
            quests,
        }
    }

    /// Whether the edge's requirements are all satisfied: every
    /// `skill_req` level met, every `item_req` count carried, every
    /// `quest_req` completed, every `varp_req` value reached, and every
    /// `worn_req` obj worn. Any requirement the state cannot prove fails
    /// the edge.
    pub fn allows(&self, e: &TransportEdge) -> bool {
        e.skill_req
            .iter()
            .all(|&(skill, level)| self.stats.get(&skill).is_some_and(|&l| l >= level))
            && e
                .item_req
                .iter()
                .all(|&(id, n)| self.inv.get(&id).is_some_and(|&c| c >= n))
            && e.quest_req.iter().all(|q| self.quests.contains(q))
            && e
                .varp_req
                .iter()
                .all(|&(varp, min)| self.varps.get(&varp).is_some_and(|&v| v >= min))
            && e.worn_req.iter().all(|id| self.worn.contains(id))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    use super::*;
    use crate::transport::{TransportKind, TransportEdge};
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
            at: WorldTile { x: 3268, z: 3227, level: 0 },
            to: WorldTile { x: 3269, z: 3227, level: 0 },
            loc_id: 2882,
            option: 1,
            ticks: 1,
            dir: None,
            open_loc_id: None,
            skill_req: vec![(6, 25)], // Magic 25 (spell teleports)
            item_req: vec![(995, 10)], // the 10-coin toll
            quest_req: vec!["Rune Mysteries".to_string()],
            varp_req: vec![(150, 160)], // Grand Tree complete
            worn_req: vec![1712],       // a charged glory
        }
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
            ..gated_edge()
        };
        assert!(WorldState::empty().allows(&free), "req-free edges stay usable");
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
        c.set_iface(500, IfType { children: Some(vec![501]), ..Default::default() });
        c.set_iface(501, IfType { r#type: ComponentType::TYPE_INV, ..Default::default() });
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
        // "Rune Mysteries" still red (not started), "Lost City" green.
        c.side_icon[2] = 700;
        c.set_iface(700, IfType { children: Some(vec![701, 702]), ..Default::default() });
        c.set_iface(701, IfType { r#type: ComponentType::TYPE_TEXT, ..Default::default() });
        c.set_iface_mut(
            701,
            IfTypeMut {
                text: "Rune Mysteries".into(),
                colour: 0xFF0000,
                ..Default::default()
            },
        );
        c.set_iface(702, IfType { r#type: ComponentType::TYPE_TEXT, ..Default::default() });
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
        assert!(snap.rebuild(&mut c), "every planted family rebuilds");
        let s = WorldState::from_snapshot(&snap);

        assert_eq!(s.inv.get(&995), Some(&10));
        assert!(s.worn.contains(&1712), "glory worn");
        assert_eq!(s.stats.get(&6), Some(&25));
        assert_eq!(s.varps.get(&150), Some(&160));
        assert!(s.quests.contains("Lost City"), "green quest done");
        assert!(
            !s.quests.contains("Rune Mysteries"),
            "red quest not done"
        );

        // Gating through the built state: an edge the state proves (the
        // completed "Lost City", not the in-progress "Rune Mysteries")
        // passes; the same edge with a missing coin does not.
        let e = TransportEdge {
            quest_req: vec!["Lost City".to_string()],
            ..gated_edge()
        };
        assert!(s.allows(&e));
        let poor = WorldState { inv: HashMap::new(), ..s.clone() };
        assert!(!poor.allows(&e));
    }
}
