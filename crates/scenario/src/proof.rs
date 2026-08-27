//! Proof predicates: named assertions over a `GameSnapshot`. A step's
//! evidence arm and a scenario's terminal proof are the same type, so the
//! JSON evidence can name exactly which predicate passed or failed.

use api::obj_names::ObjNames;
use api::snapshot::GameSnapshot;
use nav::arrival::arrived;
use nav::tile::Tile;

/// A named predicate over observable game state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proof {
    /// `has_item("Bones")`: inventory holds `count` of the obj named
    /// `name`, resolved through the runner's shared obj table.
    Item { name: &'static str, count: i32 },
    /// `arrived(x, z, level)`: standing on the tile (or adjacent to it
    /// when the dest is solid).
    Arrived { x: i32, z: i32, level: i32 },
    /// `stat(id) >= min`: a decoded stat-family value. The body decodes
    /// only run energy today (`id == 16`); other ids never hold.
    Stat { id: i32, min: i32 },
    /// A `MESSAGE_GAME`/`MESSAGE_PRIVATE` line containing `needle`.
    Chat { needle: &'static str },
    /// An NPC of `r#type` stands on the tile.
    NpcAt {
        r#type: usize,
        x: i32,
        z: i32,
    },
}

impl Proof {
    /// The predicate's evidence name (also the JSON `predicate` field).
    pub fn name(&self) -> String {
        match self {
            Proof::Item { name, count } => format!("has_item({name})>={count}"),
            Proof::Arrived { x, z, level } => format!("arrived({x},{z},{level})"),
            Proof::Stat { id, min } => format!("stat({id})>={min}"),
            Proof::Chat { needle } => format!("chat(contains \"{needle}\")"),
            Proof::NpcAt { r#type, x, z } => format!("npc({type})@({x},{z})"),
        }
    }

    /// Whether the predicate holds on `snap`. `names` resolves `Item`
    /// predicates (name → obj id); `None` when the runner has no obj
    /// table, which makes every `Item` predicate fail.
    pub fn check(&self, snap: &GameSnapshot, names: Option<&ObjNames>) -> bool {
        match self {
            Proof::Item { name, count } => names
                .and_then(|n| n.by_name(name))
                .is_some_and(|id| snap.inv_count(id) >= *count),
            Proof::Arrived { x, z, level } => snap.tile().is_some_and(|(tx, tz, tl)| {
                arrived(
                    Tile {
                        x: tx,
                        z: tz,
                        level: tl,
                    },
                    Tile {
                        x: *x,
                        z: *z,
                        level: *level,
                    },
                    true,
                )
            }),
            Proof::Stat { id, min } => stat_value(snap, *id).is_some_and(|v| v >= *min),
            Proof::Chat { needle } => snap.chat().is_some_and(|c| c.contains(needle)),
            Proof::NpcAt { r#type, x, z } => snap
                .npcs()
                .iter()
                .any(|n| n.r#type == Some(*r#type) && n.x == *x && n.z == *z),
        }
    }
}

/// A decoded stat-family value, `None` for ids the body does not decode.
/// Today only run energy (stat 16) is decoded on the body; the rest of
/// the 0–24 stat table is a later decode.
fn stat_value(snap: &GameSnapshot, id: i32) -> Option<i32> {
    match id {
        16 => Some(snap.runenergy()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::snapshot::GameSnapshot;
    use client::client::{Client, ClientConfig, ClientNpc};
    use client::config::if_type::{ComponentType, IfType};
    use client::config::ObjType;
    use client::dash3d::ClientPlayer;
    use client::io::ServerProt;

    fn cfg() -> ClientConfig {
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        }
    }

    /// A synthetic mainland client: tile (3220, 3212, 0), one Bones in
    /// the inv iface, run energy 42, a chat line, ingame scene 2.
    fn seeded() -> Client {
        let mut c = Client::new(cfg());
        c.ingame = true;
        c.scene_state = 2;
        c.map_build_base_x = 3200;
        c.map_build_base_z = 3200;
        c.local_player = Some(ClientPlayer::at(20, 12));
        c.runenergy = 42;
        // The client's iface template already has the TYPE_INV widget;
        // fill it the way the server's `UPDATE_INV_FULL` does.
        match c.ifaces.iter_mut().flatten().find(|f| f.r#type == ComponentType::TYPE_INV) {
            Some(inv) => {
                inv.link_obj_type = Some(vec![526, 995]);
                inv.link_obj_number = Some(vec![1, 100]);
            }
            None => c.ifaces.push(Some(IfType {
                r#type: ComponentType::TYPE_INV,
                link_obj_type: Some(vec![526, 995]),
                link_obj_number: Some(vec![1, 100]),
                ..Default::default()
            })),
        }
        c.chat_text[0] = "Welcome to RuneScape".into();
        let mut npc = ClientNpc::default();
        npc.r#type = Some(708);
        npc.entity.x = 100;
        npc.entity.z = 200;
        c.npc[3] = Some(npc);
        c.npc_ids[0] = 3;
        c.npc_count = 1;
        for prot in [
            ServerProt::PLAYER_INFO,
            ServerProt::UPDATE_INV_FULL,
            ServerProt::UPDATE_RUNENERGY,
            ServerProt::MESSAGE_GAME,
            ServerProt::REBUILD_NORMAL,
            ServerProt::NPC_INFO,
        ] {
            c.bump_gens(prot);
        }
        c
    }

    fn snap(c: &mut Client) -> GameSnapshot {
        let mut s = GameSnapshot::new();
        s.rebuild(c);
        s
    }

    fn names() -> ObjNames {
        ObjNames::from_objs(&[ObjType {
            id: 526,
            name: "Bones".into(),
            ..Default::default()
        }])
    }

    #[test]
    fn item_resolves_by_name_through_the_obj_table() {
        let s = snap(&mut seeded());
        assert!(Proof::Item {
            name: "Bones",
            count: 1
        }
        .check(&s, Some(&names())));
        assert!(!Proof::Item {
            name: "Bones",
            count: 2
        }
        .check(&s, Some(&names())));
        assert!(!Proof::Item {
            name: "Coins",
            count: 1
        }
        .check(&s, Some(&names())));
        // No obj table: every item predicate fails, never fakes a hit.
        assert!(!Proof::Item {
            name: "Bones",
            count: 1
        }
        .check(&s, None));
    }

    #[test]
    fn arrived_matches_the_player_world_tile() {
        let s = snap(&mut seeded());
        assert!(Proof::Arrived {
            x: 3220,
            z: 3212,
            level: 0
        }
        .check(&s, None));
        assert!(!Proof::Arrived {
            x: 3220,
            z: 3211,
            level: 0
        }
        .check(&s, None));
        assert_eq!(
            Proof::Arrived {
                x: 3220,
                z: 3212,
                level: 0
            }
            .name(),
            "arrived(3220,3212,0)"
        );
    }

    #[test]
    fn stat_16_is_run_energy_others_never_hold() {
        let s = snap(&mut seeded());
        assert!(Proof::Stat { id: 16, min: 42 }.check(&s, None));
        assert!(!Proof::Stat { id: 16, min: 43 }.check(&s, None));
        // stat(7) is not decoded on the body yet — honest false.
        assert!(!Proof::Stat { id: 7, min: 1 }.check(&s, None));
    }

    #[test]
    fn chat_matches_the_ring_head_line() {
        let s = snap(&mut seeded());
        assert!(Proof::Chat {
            needle: "Welcome"
        }
        .check(&s, None));
        assert!(!Proof::Chat {
            needle: "arrived"
        }
        .check(&s, None));
    }

    #[test]
    fn npc_at_matches_type_and_tile() {
        let s = snap(&mut seeded());
        assert!(Proof::NpcAt {
            r#type: 708,
            x: 100,
            z: 200
        }
        .check(&s, None));
        assert!(!Proof::NpcAt {
            r#type: 709,
            x: 100,
            z: 200
        }
        .check(&s, None));
    }
}
