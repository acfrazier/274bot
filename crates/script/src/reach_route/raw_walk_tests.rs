//! The real revision-289 client's own walk, for the tests that hold the posted
//! flood's route against what the client sends: its breadth-first route over a
//! painted collision map and the movement packet it writes for that route
//! (`Client::tryMove`, `client.rs:5252-5626`), with the scenes to paint.

use api::snapshot::SceneView;
use client::client::{Client, ClientConfig};
use client::config::Cache;
use client::io::ClientRevision;
use std::sync::Arc;

/// One click of the client: its whole route, and the waypoints of the one
/// movement packet it wrote for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Click {
    /// Every tile of the client's route, from the tile it left to the target.
    pub route: Vec<(i32, i32)>,
    /// The waypoints of the packet, in walking order.
    pub waypoints: Vec<(i32, i32)>,
}

/// A real revision-289 client with a painted collision map and no scene. Tile
/// coordinates are build-area coordinates, which are world coordinates here
/// (the build base is zero).
pub(crate) struct RawWalk(Client);

impl RawWalk {
    pub(crate) fn new() -> Self {
        Self(Client::from_shared_with_revision(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 43594,
                // No cache is read: the walk needs no archives.
                cache_dir: "/nonexistent/274bot-test-cache".into(),
                members: true,
                lowmem: true,
            },
            Arc::new(Cache::default()),
            Arc::new(Vec::new()),
            Vec::new(),
            ClientRevision::R289,
        ))
    }

    /// Close every tile of level 0 except `open`: a closed tile has every
    /// collision bit set, an open one none.
    pub(crate) fn open_only(&mut self, open: impl IntoIterator<Item = (i32, i32)>) {
        self.0.minusedlevel = 0;
        self.0.map_build_base_x = 0;
        self.0.map_build_base_z = 0;
        for column in &mut self.0.collision[0].flags {
            column.fill(-1);
        }
        for (x, z) in open {
            let (x, z) = (usize::try_from(x).unwrap(), usize::try_from(z).unwrap());
            self.0.collision[0].flags[x][z] = 0;
        }
    }

    /// The scene the host posts for this client's collision: the client's own
    /// flags, as `GameSnapshot` projects them (`snapshot/decode.rs:1346-1355`).
    pub(crate) fn scene(&self) -> SceneView {
        let cmap = &self.0.collision[0];
        SceneView {
            available: true,
            base_x: self.0.map_build_base_x,
            base_z: self.0.map_build_base_z,
            level: 0,
            width: cmap.size_x,
            height: cmap.size_z,
            collision_flags: cmap.flags.iter().flatten().copied().collect(),
        }
    }

    /// The click the client makes from `from` to the exact tile `to`: `None`
    /// when it refuses it (it has no route).
    pub(crate) fn click(&mut self, from: (i32, i32), to: (i32, i32)) -> Option<Click> {
        self.0.out.pos = 0;
        if !self
            .0
            .tryMove(from.0, from.1, to.0, to.1, false, 0, 0, 0, 0, 0, 0)
        {
            return None;
        }
        let route = self.0.try_move_path.clone();
        let end = self.0.out.pos;
        let bytes = &self.0.out.data()[..end];
        // opcode, length, run flag, start x, start z, then one signed offset
        // pair from the start for every further waypoint.
        let count = (usize::from(bytes[1]) - 3) / 2;
        assert_eq!(bytes.len(), 2 + usize::from(bytes[1]), "one whole packet");
        let start = (
            i32::from(u16::from_be_bytes([bytes[3], bytes[4]])),
            i32::from(u16::from_be_bytes([bytes[5], bytes[6]])),
        );
        let mut waypoints = vec![start];
        for index in 1..count {
            let at = 7 + 2 * (index - 1);
            waypoints.push((
                start.0 + i32::from(bytes[at] as i8),
                start.1 + i32::from(bytes[at + 1] as i8),
            ));
        }
        Some(Click { route, waypoints })
    }
}

/// What the engine does with the waypoints of one packet: it walks a straight
/// leg to each in turn, one step a tick (`PathingEntity.ts:135-149`). The steps
/// it takes and the tile it stands on after them.
pub(crate) fn walk(from: (i32, i32), waypoints: &[(i32, i32)]) -> (u32, (i32, i32)) {
    let (mut steps, mut at) = (0u32, from);
    for &to in waypoints {
        let (dx, dz) = (to.0 - at.0, to.1 - at.1);
        assert!(
            dx == 0 || dz == 0 || dx.abs() == dz.abs(),
            "a leg {at:?} to {to:?} is a straight line"
        );
        steps += u32::try_from(dx.abs().max(dz.abs())).unwrap();
        at = to;
    }
    (steps, at)
}

/// The corners of a route, as the client collects them: the tiles where the
/// way changes, from the start to the end (`client.rs:5497-5521`).
pub(crate) fn corners(route: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let way = |i: usize| (route[i].0 - route[i - 1].0, route[i].1 - route[i - 1].1);
    (1..route.len() - 1)
        .filter(|&i| way(i) != way(i + 1))
        .map(|i| route[i])
        .collect()
}

/// A deterministic stream of numbers for the scenes below.
pub(crate) struct Prng(pub u64);

impl Prng {
    fn advance(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(crate) fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.advance() % u64::try_from(bound).unwrap()).unwrap()
    }
}

/// The open tiles of a maze of one-tile corridors over the build area: a
/// depth-first carve over the odd tiles. Every route in it winds, and most
/// take far more than 25 corners.
pub(crate) fn maze(rng: &mut Prng) -> Vec<(i32, i32)> {
    const CELLS: i32 = 50;
    let cell = |cx: i32, cz: i32| (1 + 2 * cx, 1 + 2 * cz);
    let mut seen = vec![false; (CELLS * CELLS) as usize];
    let mut open = vec![cell(0, 0)];
    let mut stack = vec![(0i32, 0i32)];
    seen[0] = true;
    while let Some(&(cx, cz)) = stack.last() {
        let next: Vec<(i32, i32)> = [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .into_iter()
            .map(|(dx, dz)| (cx + dx, cz + dz))
            .filter(|&(nx, nz)| {
                (0..CELLS).contains(&nx)
                    && (0..CELLS).contains(&nz)
                    && !seen[(nx * CELLS + nz) as usize]
            })
            .collect();
        if next.is_empty() {
            stack.pop();
            continue;
        }
        let (nx, nz) = next[rng.below(next.len())];
        seen[(nx * CELLS + nz) as usize] = true;
        let (from, to) = (cell(cx, cz), cell(nx, nz));
        open.push(((from.0 + to.0) / 2, (from.1 + to.1) / 2));
        open.push(to);
        stack.push((nx, nz));
    }
    open
}

/// The open tiles of open ground with `percent` in a hundred of its tiles
/// closed at random: diagonals, short detours, and tiles with no route.
pub(crate) fn scattered(rng: &mut Prng, percent: usize) -> Vec<(i32, i32)> {
    (1..103)
        .flat_map(|x| (1..103).map(move |z| (x, z)))
        .filter(|_| rng.below(100) >= percent)
        .collect()
}
