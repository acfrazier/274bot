//! JSON evidence record: the snapshot at the terminal state plus the
//! predicate that passed or failed. Each scenario's PASS/FAIL prints one
//! of these (headed: to the live window's log; headless: to the test
//! output).

use std::time::Instant;

use api::obj_names::ObjNames;
use api::snapshot::GameSnapshot;
use serde::Serialize;

/// One terminal-state evidence row for a scenario run.
#[derive(Debug, Clone, Serialize)]
pub struct Evidence {
    pub scenario: String,
    /// `"PASS"` or `"FAIL"`.
    pub outcome: &'static str,
    /// The proof / arm predicate that passed or failed.
    pub predicate: String,
    /// Game ticks the run took (seed waits are not counted).
    pub ticks: u32,
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// The player's world tile at the terminal state `[x, z]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tile: Option<[i32; 2]>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inv: Vec<InvRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stat: Option<StatRow>,
    /// `scene_state` at the terminal state.
    pub scene: i32,
}

/// One inventory slot's row; `name` resolves through the runner's obj
/// table when it has one.
#[derive(Debug, Clone, Serialize)]
pub struct InvRow {
    pub id: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub count: i32,
}

/// The decoded stat-family values (run energy today).
#[derive(Debug, Clone, Serialize)]
pub struct StatRow {
    pub runenergy: i32,
}

impl Evidence {
    /// The compact JSON record both runners print on PASS/FAIL.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|e| {
            format!(
                "{{\"scenario\":{},\"outcome\":{},\"serialize_error\":{}}}",
                self.scenario, self.outcome, e
            )
        })
    }

    /// Build the terminal-state record from the snapshot + predicate.
    pub fn terminal(
        scenario: &str,
        outcome: &'static str,
        predicate: String,
        message: Option<String>,
        ticks: u32,
        snap: &GameSnapshot,
        names: Option<&ObjNames>,
        started: Instant,
    ) -> Self {
        let inv = snap
            .inv()
            .iter()
            .filter(|(_, count)| *count > 0)
            .map(|(id, count)| InvRow {
                id: *id,
                name: names.and_then(|n| n.name(*id)).map(str::to_string),
                count: *count,
            })
            .collect();
        Evidence {
            scenario: scenario.to_string(),
            outcome,
            predicate,
            message,
            ticks,
            elapsed_ms: started.elapsed().as_millis() as u64,
            tile: snap.tile().map(|(x, z, _)| [x, z]),
            inv,
            stat: Some(StatRow {
                runenergy: snap.runenergy(),
            }),
            scene: snap.scene_state(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::obj_names::ObjNames;
    use client::client::{Client, ClientConfig};
    use client::config::if_type::{ComponentType, IfType, IfTypeMut};
    use client::config::ObjType;
    use client::dash3d::ClientPlayer;
    use client::io::ServerProt;
    use std::time::Duration;

    fn cfg() -> ClientConfig {
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: false,
        }
    }

    fn seeded() -> Client {
        let mut c = Client::new(cfg());
        c.ingame = true;
        c.scene_state = 2;
        c.map_build_base_x = 3200;
        c.map_build_base_z = 3200;
        c.local_player = Some(ClientPlayer::at(20, 12));
        c.runenergy = 42;
        match c.iface_id(|f| f.r#type == ComponentType::TYPE_INV) {
            Some(id) => {
                let inv = c.iface_mut(id).unwrap();
                // stored = obj_id + 1: a real Bones id 526 stores as 527.
                inv.link_obj_type = Some(vec![527]);
                inv.link_obj_number = Some(vec![1]);
            }
            None => {
                let id = c.push_iface(IfType {
                    r#type: ComponentType::TYPE_INV,
                    ..Default::default()
                });
                c.set_iface_mut(
                    id,
                    IfTypeMut {
                        link_obj_type: Some(vec![527]),
                        link_obj_number: Some(vec![1]),
                        ..Default::default()
                    },
                );
            }
        }
        c.bump_gens(ServerProt::PLAYER_INFO);
        c.bump_gens(ServerProt::UPDATE_INV_FULL);
        c.bump_gens(ServerProt::UPDATE_RUNENERGY);
        c.bump_gens(ServerProt::REBUILD_NORMAL);
        c
    }

    fn names() -> ObjNames {
        ObjNames::from_objs(&[ObjType {
            id: 526,
            name: "Bones".into(),
            ..Default::default()
        }])
    }

    #[test]
    fn evidence_json_names_predicate_tile_inv_and_stat() {
        let mut c = seeded();
        let mut snap = GameSnapshot::new();
        snap.rebuild(&mut c);
        let started = Instant::now() - Duration::from_millis(12);
        let ev = Evidence::terminal(
            "walk",
            "PASS",
            "arrived(3220,3212,0)".into(),
            None,
            7,
            &snap,
            Some(&names()),
            started,
        );
        let json = serde_json::to_string(&ev).expect("evidence serializes");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v["scenario"], "walk");
        assert_eq!(v["outcome"], "PASS");
        assert_eq!(v["predicate"], "arrived(3220,3212,0)");
        assert_eq!(v["tile"], serde_json::json!([3220, 3212]));
        assert_eq!(v["inv"][0]["id"], 526);
        assert_eq!(v["inv"][0]["name"], "Bones");
        assert_eq!(v["inv"][0]["count"], 1);
        assert_eq!(v["stat"]["runenergy"], 42);
        assert_eq!(v["scene"], 2);
        assert!(v["elapsed_ms"].as_u64().unwrap() >= 12);
        // A small record: the message field is absent on PASS.
        assert!(v.get("message").is_none());
    }

    #[test]
    fn fail_evidence_carries_the_message() {
        let mut c = seeded();
        let mut snap = GameSnapshot::new();
        snap.rebuild(&mut c);
        let ev = Evidence::terminal(
            "walk",
            "FAIL",
            "arrived(3220,3212,0)".into(),
            Some("step 1: arrived(3220,3216,0) not seen within 90 ticks".into()),
            90,
            &snap,
            None,
            Instant::now(),
        );
        let json = serde_json::to_string(&ev).expect("evidence serializes");
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(v["outcome"], "FAIL");
        assert_eq!(
            v["message"],
            "step 1: arrived(3220,3216,0) not seen within 90 ticks"
        );
        assert!(
            v["inv"][0].get("name").is_none(),
            "no obj table -> no names"
        );
    }
}
