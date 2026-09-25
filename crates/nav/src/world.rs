//! The router's world, loaded from the baked nav pack: the whole-world
//! [`WorldCollision`] walk surface plus the transport [`TransportGraph`]
//! and the content-derived bank stand table ([`crate::pack::BankStand`]).
//! The v8 pack file stores the compact packed walk surface and the
//! transport edges, so the Dijkstra router ([`crate::router::find`])
//! consumes one artifact — live harnesses load this and route on the
//! packed collision. The legacy 274N grid pack (boolean walk bytes +
//! doors) still loads through [`NavWorld::from_grid`] as a fallback for
//! old `.navpack` files.

use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;

use crate::collision::WorldCollision;
use crate::grid::StepGrid;
use crate::pack::{decode, decode_grid, BankStand, PackError};
use crate::transport::{TransportEdge, TransportGraph, TransportKind};

/// A fully-blocked stamp: every directional `PL_WALK_*` mask, so the
/// router's directional step test rejects the tile from any direction
/// (a 274N grid walk byte is boolean; there are no half-open faces to keep).
const BLOCKED: u32 = CollisionFlag::SQ_BLOCKED as u32 | CollisionFlag::WALK_BLOCK_FLAGS as u32;

/// The whole-world collision + transport graph + bank stand table the
/// router consumes.
pub struct NavWorld {
    pub collision: WorldCollision,
    pub graph: TransportGraph,
    banks: Vec<BankStand>,
    named_banks: OnceLock<Arc<api::named_banks::NamedBankFacts>>,
}

impl NavWorld {
    /// The baked bank stands ([`BankStand`]) the banking session walks to
    /// and opens, ordered by tile.
    pub fn banks(&self) -> &[BankStand] {
        &self.banks
    }

    /// Bind the complete frozen roster to this revision's access placements and
    /// the loaded collision. Unresolved entries remain air-fallback candidates.
    pub fn named_bank_facts(
        &self,
        data: Option<&api::game_data::SelectedGameData>,
    ) -> Arc<api::named_banks::NamedBankFacts> {
        Arc::clone(self.named_banks.get_or_init(|| Arc::new(crate::named_banks::resolve(
            api::named_banks::BANK_CATALOG,
            data.and_then(|data| data.bank_placements()).map_or(&[], |facts| facts.rows.as_slice()),
            |tile| self.collision.standable(tile),
        ))))
    }

    /// Decode already-read pack bytes into the router's world. Whole-world
    /// packs (collision + transport graph + bank stands) load directly;
    /// legacy 274N grid packs fall back to [`Self::from_grid`] on the same
    /// buffer. The fallback triggers on bad magic only — a `274V` pack with
    /// a stale version stays [`PackError::BadVersion`] so the operator
    /// rebakes. This never rereads a pathname.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PackError> {
        match decode(bytes) {
            Ok((collision, graph, banks)) => Ok(Self::from_parts(collision, graph, banks)),
            Err(PackError::BadMagic) => Ok(Self::from_grid(&decode_grid(bytes)?)),
            Err(e) => Err(e),
        }
    }

    /// Load the baked nav pack (`$NAV_PACK` or the default path) into the
    /// router's world. One filesystem read plus [`Self::from_bytes`].
    pub fn load_pack(path: &Path) -> Result<Self, PackError> {
        let bytes = std::fs::read(path).map_err(PackError::Io)?;
        Self::from_bytes(&bytes)
    }

    /// Build the router's world from already-decoded parts (the pack
    /// decode and external fixtures share this; `banks` is the baked
    /// stand table [`Self::banks`] reads).
    pub fn from_parts(
        collision: WorldCollision,
        graph: TransportGraph,
        banks: Vec<BankStand>,
    ) -> Self {
        NavWorld {
            collision,
            graph,
            banks,
            named_banks: OnceLock::new(),
        }
    }

    /// `$NAV_PACK` or `~/.274bot/274bot.navpack` (same default `nav-pack` writes).
    #[cfg(test)]
    pub(crate) fn default_pack_path() -> PathBuf {
        match std::env::var("NAV_PACK") {
            Ok(p) => PathBuf::from(p),
            Err(_) => match client::operator_home() {
                Ok(home) => PathBuf::from(format!("{home}/.274bot/274bot.navpack")),
                Err(_) => PathBuf::from(".274bot/274bot.navpack"),
            },
        }
    }

    /// Whole-world pack proofs. GitHub has no pack and no Server tree —
    /// skip instead of panicking. Local with a rebake still runs them.
    #[cfg(test)]
    pub(crate) fn load_default_pack_or_skip() -> Option<Self> {
        let path = Self::default_pack_path();
        match Self::load_pack(&path) {
            Ok(w) => Some(w),
            Err(e) => {
                eprintln!("SKIP: no nav pack at {} ({e:?})", path.display());
                None
            }
        }
    }

    /// Derive the router's world from a 274N grid pack. Walkable tiles
    /// carry no flags; every blocked tile carries [`BLOCKED`] (the 274N
    /// grid has no per-direction data, so a blocked tile is a wall on all
    /// faces). Each pack door edge becomes a 1-tick `Door` transport
    /// edge; the pack stores both directions, so the graph indexes `at`
    /// exactly as the grid authored them.
    pub fn from_grid(grid: &StepGrid) -> Self {
        // The 274N grid is one level-0 plane; the 4-plane buffer keeps
        // upper-level lookups on their own (empty) planes instead of
        // panicking or reusing level 0.
        let plane = grid.width * grid.height;
        let mut flags = vec![0u32; 4 * plane];
        for z in 0..grid.height {
            for x in 0..grid.width {
                let t = crate::tile::Tile {
                    x: grid.origin.x + x as i32,
                    z: grid.origin.z + z as i32,
                    level: grid.origin.level,
                };
                flags[z * grid.width + x] = if grid.walkable(t) { 0 } else { BLOCKED };
            }
        }
        let (walk, blocked) = crate::collision::pack_walk(&flags);
        let collision = WorldCollision {
            origin: WorldTile {
                x: grid.origin.x,
                z: grid.origin.z,
                level: grid.origin.level,
            },
            width: grid.width,
            height: grid.height,
            walk,
            blocked,
            flags: None,
        };
        let mut graph = TransportGraph::default();
        for d in &grid.doors {
            let i = graph.edges.len();
            graph.edges.push(TransportEdge {
                kind: TransportKind::Door,
                at: WorldTile {
                    x: d.from.x,
                    z: d.from.z,
                    level: d.from.level,
                },
                to: WorldTile {
                    x: d.to.x,
                    z: d.to.z,
                    level: d.to.level,
                },
                loc_id: d.loc_id,
                option: 1,
                ticks: 1,
                dir: None,
                open_loc_id: None,
                skill_req: vec![],
                item_req: vec![],
                quest_req: vec![],
                varp_req: vec![],
                worn_req: vec![],
                members_req: false,
                wildy_cap: None,
            });
            graph.at.entry(graph.edges[i].at).or_default().push(i);
        }
        NavWorld {
            collision,
            graph,
            banks: Vec::new(),
            named_banks: OnceLock::new(),
        }
    }
}

#[cfg(test)]
#[path = "world_tests.rs"]
mod tests;
