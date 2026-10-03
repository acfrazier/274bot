use super::select::AvoidedTile;
use api::gather_methods::{known_rows, GatherCatalog, SceneRegionInput, TargetClass};
use api::selected::{Knowledge, Truth};
use api::snapshot::WorldTile;

#[derive(Debug, Clone, Copy, Default)]
pub struct WidenCursor {
    best: Option<WorldTile>,
    ring: u8,
    square: u8,
    pub searched: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct TriedGroup {
    pub tile: WorldTile,
    pub until: u32,
}

impl Default for TriedGroup {
    fn default() -> Self {
        Self {
            tile: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            until: 0,
        }
    }
}

pub enum SearchResult {
    Pending,
    Found(WorldTile),
    Exhausted,
}

pub struct SearchExclusions<'a> {
    pub avoided: &'a [AvoidedTile],
    pub tried: &'a [TriedGroup; 4],
}

impl WidenCursor {
    pub fn reset(&mut self) {
        *self = Self {
            ring: 1,
            ..Self::default()
        };
    }

    /// Exactly one map square intersected with the current distance ring per poll.
    pub fn poll(
        &mut self,
        catalog: &GatherCatalog,
        methods: &[usize],
        start: WorldTile,
        radius: u16,
        exclusions: SearchExclusions<'_>,
        now: u64,
    ) -> SearchResult {
        let SearchExclusions { avoided, tried } = exclusions;
        if self.ring == 0 {
            self.reset();
        }
        let (region, last) = ring_slice(start, self.ring, self.square);
        let inner = i32::from(self.ring.saturating_sub(1)) * 32;
        for &index in methods {
            let method = &catalog.methods()[index];
            let Ok(spots) = catalog.spots(method, &region) else {
                continue;
            };
            for spot in spots {
                let d = distance(start, spot.origin);
                if (self.ring > 1 && d <= inner)
                    || catalog.access(method, spot).unwrap_or(Truth::False) != Truth::True
                    || !known_rows(&method.targets).iter().any(|target| {
                        target.entity == spot.entity
                            && target.class == TargetClass::Resource
                            && matches!(target.respawn, Knowledge::Known(_))
                    })
                    || avoided
                        .iter()
                        .any(|entry| u64::from(entry.until) > now && entry.tile == spot.origin)
                    || tried.iter().any(|entry| {
                        u64::from(entry.until) > now
                            && distance(entry.tile, spot.origin) <= i32::from(radius)
                    })
                {
                    continue;
                }
                if self.best.is_none_or(|best| {
                    (d, spot.origin.x, spot.origin.z) < (distance(start, best), best.x, best.z)
                }) {
                    self.best = Some(spot.origin);
                }
            }
        }
        self.square += 1;
        if !last {
            return SearchResult::Pending;
        }
        self.searched = self.ring * 32;
        if let Some(best) = self.best.take() {
            let region = SceneRegionInput {
                min_x: best.x.saturating_sub(i32::from(radius)),
                min_z: best.z.saturating_sub(i32::from(radius)),
                max_x: best.x.saturating_add(i32::from(radius)),
                max_z: best.z.saturating_add(i32::from(radius)),
                level: best.level,
            };
            let (mut x, mut z, mut n) = (0i64, 0i64, 0i64);
            for &index in methods {
                let method = &catalog.methods()[index];
                let Ok(spots) = catalog.spots(method, &region) else {
                    continue;
                };
                for spot in spots {
                    if catalog.access(method, spot).unwrap_or(Truth::False) == Truth::True
                        && known_rows(&method.targets).iter().any(|target| {
                            target.entity == spot.entity
                                && target.class == TargetClass::Resource
                                && matches!(target.respawn, Knowledge::Known(_))
                        })
                        && !avoided
                            .iter()
                            .any(|entry| u64::from(entry.until) > now && entry.tile == spot.origin)
                        && !tried.iter().any(|entry| {
                            u64::from(entry.until) > now
                                && distance(entry.tile, spot.origin) <= i32::from(radius)
                        })
                    {
                        x += i64::from(spot.origin.x);
                        z += i64::from(spot.origin.z);
                        n += 1;
                    }
                }
            }
            return SearchResult::Found(if n == 0 {
                best
            } else {
                WorldTile {
                    x: (x / n) as i32,
                    z: (z / n) as i32,
                    level: best.level,
                }
            });
        }
        if self.ring == 4 {
            return SearchResult::Exhausted;
        }
        self.ring += 1;
        self.square = 0;
        SearchResult::Pending
    }
}

pub fn remember_group(tried: &mut [TriedGroup; 4], tile: WorldTile, now: u64, until: u64) -> bool {
    for entry in tried.iter_mut() {
        if u64::from(entry.until) <= now {
            *entry = TriedGroup::default();
        }
    }
    if until <= now {
        return true;
    }
    let Some(entry) = tried
        .iter_mut()
        .find(|entry| entry.until == 0 || entry.tile == tile)
    else {
        return false;
    };
    *entry = TriedGroup {
        tile,
        until: until.min(u64::from(u32::MAX)) as u32,
    };
    true
}

fn distance(a: WorldTile, b: WorldTile) -> i32 {
    if a.level != b.level {
        i32::MAX
    } else {
        a.x.abs_diff(b.x)
            .max(a.z.abs_diff(b.z))
            .min(i32::MAX as u32) as i32
    }
}

fn ring_slice(start: WorldTile, ring: u8, square: u8) -> (SceneRegionInput, bool) {
    let radius = i32::from(ring) * 32;
    let min_x = start.x.saturating_sub(radius);
    let min_z = start.z.saturating_sub(radius);
    let max_x = start.x.saturating_add(radius);
    let max_z = start.z.saturating_add(radius);
    let first_x = min_x.div_euclid(64);
    let first_z = min_z.div_euclid(64);
    let width = max_x.div_euclid(64) - first_x + 1;
    let height = max_z.div_euclid(64) - first_z + 1;
    let x = (first_x + i32::from(square) % width) * 64;
    let z = (first_z + i32::from(square) / width) * 64;
    (
        SceneRegionInput {
            min_x: min_x.max(x),
            min_z: min_z.max(z),
            max_x: max_x.min(x + 63),
            max_z: max_z.min(z + 63),
            level: start.level,
        },
        i32::from(square) + 1 == width * height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_ring_slices_do_not_admit_far_parts_of_touched_squares() {
        let start = WorldTile {
            x: 3231,
            z: 3231,
            level: 0,
        };
        for ring in 1..=4 {
            let mut square = 0;
            let mut seen_33 = false;
            let mut seen_91 = false;
            loop {
                let (r, last) = ring_slice(start, ring, square);
                let contains =
                    |x| x >= r.min_x && x <= r.max_x && start.z >= r.min_z && start.z <= r.max_z;
                seen_33 |= contains(3264);
                seen_91 |= contains(3140);
                if last {
                    break;
                }
                square += 1;
            }
            assert_eq!(seen_33, ring >= 2);
            assert_eq!(seen_91, ring >= 3);
            assert!(square < 25);
        }
    }

    #[test]
    fn fifth_unexpired_group_refuses_but_expired_groups_are_reusable() {
        let mut tried = [TriedGroup::default(); 4];
        for x in 0..4 {
            assert!(remember_group(
                &mut tried,
                WorldTile { x, z: 0, level: 0 },
                10,
                100
            ));
        }
        assert!(!remember_group(
            &mut tried,
            WorldTile {
                x: 5,
                z: 0,
                level: 0
            },
            20,
            100
        ));
        assert!(remember_group(
            &mut tried,
            WorldTile {
                x: 5,
                z: 0,
                level: 0
            },
            100,
            200
        ));
        assert_eq!(tried.iter().filter(|entry| entry.until > 100).count(), 1);
    }

    #[test]
    fn search_skips_more_than_thirty_two_exhausted_placements_without_allocation() {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let catalog =
            api::selected::FamilyPreparation::run(move |worker| selected.prepare_gathering(worker))
                .unwrap()
                .join()
                .unwrap()
                .unwrap();
        let index = catalog
            .methods()
            .iter()
            .position(|method| method.id.0.as_ref() == "woodcutting.normal")
            .unwrap();
        let start = WorldTile {
            x: 3190,
            z: 3245,
            level: 0,
        };
        let region = SceneRegionInput {
            min_x: start.x - 32,
            min_z: start.z - 32,
            max_x: start.x + 32,
            max_z: start.z + 32,
            level: 0,
        };
        assert!(
            catalog
                .spots(&catalog.methods()[index], &region)
                .unwrap()
                .count()
                > 32
        );
        let mut tried = [TriedGroup::default(); 4];
        assert!(remember_group(&mut tried, start, 1, 1000));
        let mut cursor = WidenCursor::default();
        let mut found = None;
        let allocation = allocation_counter::measure(|| {
            for _ in 0..60 {
                match cursor.poll(
                    &catalog,
                    &[index],
                    start,
                    32,
                    SearchExclusions {
                        avoided: &[],
                        tried: &tried,
                    },
                    2,
                ) {
                    SearchResult::Pending => {}
                    SearchResult::Found(tile) => {
                        found = Some(tile);
                        break;
                    }
                    SearchResult::Exhausted => panic!("another tree group exists"),
                }
            }
        });
        let found = found.expect("search must continue past the excluded nearest placements");
        assert!(distance(start, found) > 32);
        assert!(cursor.searched >= 64);
        assert_eq!(allocation.count_total, 0);
    }

    #[test]
    fn search_exhausts_all_four_rings_and_retry_reset_starts_at_one() {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let catalog =
            api::selected::FamilyPreparation::run(move |worker| selected.prepare_gathering(worker))
                .unwrap()
                .join()
                .unwrap()
                .unwrap();
        let index = catalog
            .methods()
            .iter()
            .position(|method| method.id.0.as_ref() == "woodcutting.normal")
            .unwrap();
        let start = WorldTile {
            x: 100,
            z: 100,
            level: 3,
        };
        let mut cursor = WidenCursor::default();
        let mut exhausted = false;
        for _ in 0..60 {
            if matches!(
                cursor.poll(
                    &catalog,
                    &[index],
                    start,
                    12,
                    SearchExclusions {
                        avoided: &[],
                        tried: &[TriedGroup::default(); 4],
                    },
                    1
                ),
                SearchResult::Exhausted
            ) {
                exhausted = true;
                break;
            }
        }
        assert!(exhausted);
        assert_eq!(cursor.searched, 128);
        cursor.reset();
        assert!(matches!(
            cursor.poll(
                &catalog,
                &[index],
                start,
                12,
                SearchExclusions {
                    avoided: &[],
                    tried: &[TriedGroup::default(); 4],
                },
                2
            ),
            SearchResult::Pending
        ));
        assert_eq!(cursor.ring, 1);
        assert_eq!(cursor.square, 1);
    }
}
