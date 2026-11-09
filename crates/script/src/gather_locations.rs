//! Borrowed preset-location helpers shared by compiled callers and Load adapters.
//!
//! These presets are not the Gatherer's selected-catalog Start/Site/Auto/Custom
//! policy. No catalog, world snapshot, or selected location is retained here.

use api::snapshot::WorldTile;
use std::borrow::Cow;
use std::convert::Infallible;
use std::ops::ControlFlow;

pub const DEFAULT_BOOTH_NAME: &str = "Bank booth";
pub const DEFAULT_BOOTH_OP: &str = "Use-quickly";
pub const MAP_SQUARE: f64 = 64.0;
/// Legacy alias kept for saves that stored `Auto` (`gatheringLocations.ts:86`).
pub const AUTO_LEGACY: &str = "Auto";
/// Rank every camp by distance (`gatheringLocations.ts:82`).
pub const USE_CLOSEST: &str = "Use Closest";
/// Fish or gather around the start tile, no preset (`gatheringLocations.ts:83`).
pub const USE_START_POSITION: &str = "Use Start Position";
/// Gather around a custom tile, no preset (`gatheringLocations.ts:84`).
pub const USE_CUSTOM_POSITION: &str = "Use Custom Position";
/// Legacy power-drop value kept so a saved `None` is not re-parsed (`gatheringLocations.ts:89`).
pub const NONE_LEGACY: &str = "None";
pub const OPTION_PREFIX: [&str; 5] = [
    AUTO_LEGACY,
    USE_CLOSEST,
    USE_START_POSITION,
    USE_CUSTOM_POSITION,
    NONE_LEGACY,
];
/// Camp membership radius when a named camp sets none (`gatheringLocations.ts:47`).
pub const DEFAULT_CAMP_RADIUS: f64 = 64.0;
/// Soft prefer-near-player radius for named camps (`gatheringLocations.ts:53`).
pub const DEFAULT_CHASE_RADIUS: f64 = 40.0;
/// Smallest radius either resolver returns (`gatheringLocations.ts:57,62`).
pub const MIN_RADIUS: f64 = 2.0;
const CUSTOM_POSITION: &str = "use custom position";
pub const MAX_ALIAS_LEN: usize = CUSTOM_POSITION.len();

/// JavaScript `Math.max` of two numbers: NaN wins, as `Math.max(2, NaN)` is NaN.
pub fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

/// JavaScript `Math.min` of two numbers: NaN wins.
pub fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// Frozen `resolveCampRadius` / `resolveChaseRadius` (`gatheringLocations.ts:55-63`): a finite input, else the
/// fallback, floored and at least [`MIN_RADIUS`]. `finite_input` is `Some` only for a finite number; the fallback
/// is the caller's already-numeric default.
pub fn resolve_radius(finite_input: Option<f64>, fallback: f64) -> f64 {
    js_max(MIN_RADIUS, finite_input.unwrap_or(fallback).floor())
}

/// The immutable facts required from a native preset row.
pub trait Location {
    fn name(&self) -> &str;
    fn spot(&self) -> WorldTile;
}

impl Location for api::gather_camps::GatherCamp {
    fn name(&self) -> &str {
        self.name
    }

    fn spot(&self) -> WorldTile {
        self.spot
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Freeform,
    Closest,
    Auto,
    Named,
}

/// The caller supplies equality so V8 can keep its original normalized value,
/// including non-string results from a caller's `toLowerCase` method.
pub fn resolution(mut equals: impl FnMut(&str) -> bool) -> Resolution {
    if equals(CUSTOM_POSITION) || equals("use start position") {
        Resolution::Freeform
    } else if equals("use closest") {
        Resolution::Closest
    } else if equals("auto") {
        Resolution::Auto
    } else {
        Resolution::Named
    }
}

#[derive(Clone, Copy)]
pub enum Axis {
    X,
    Z,
}

/// Keep the same level/x/z short circuit for native tiles and foreign getters.
pub fn same_square_with<E>(
    same_level: bool,
    mut same_chunk: impl FnMut(Axis) -> Result<bool, E>,
) -> Result<bool, E> {
    Ok(same_level && same_chunk(Axis::X)? && same_chunk(Axis::Z)?)
}

/// Equal plane and equal floor-divided 64-tile map-square coordinates.
pub fn same_map_square(a: WorldTile, b: WorldTile) -> bool {
    same_square_with(a.level == b.level, |axis| {
        let (a, b) = match axis {
            Axis::X => (a.x, b.x),
            Axis::Z => (a.z, b.z),
        };
        Ok::<_, Infallible>(a.div_euclid(MAP_SQUARE as i32) == b.div_euclid(MAP_SQUARE as i32))
    })
    .unwrap_or_else(|never| match never {})
}

/// Standard names followed by row names, without copying the table or names.
pub fn location_options<'a, T: Location + 'a>(rows: &'a [T]) -> impl Iterator<Item = &'a str> {
    OPTION_PREFIX
        .into_iter()
        .chain(rows.iter().map(|row| row.name()))
}

pub fn booth_name<T>(value: Option<T>, default: impl FnOnce(&'static str) -> T) -> T {
    value.unwrap_or_else(|| default(DEFAULT_BOOTH_NAME))
}

pub fn booth_op<T>(value: Option<T>, default: impl FnOnce(&'static str) -> T) -> T {
    value.unwrap_or_else(|| default(DEFAULT_BOOTH_OP))
}

/// Missing fields use the shared defaults. Present empty strings stay empty.
pub fn booth_fields<'a>(name: Option<&'a str>, op: Option<&'a str>) -> (&'a str, &'a str) {
    (booth_name(name, |value| value), booth_op(op, |value| value))
}

/// The planar bank-distance calculation also used by the Load bank adapter.
pub fn distance_delta(dx: f64, dz: f64) -> f64 {
    dx.hypot(dz)
}

pub(crate) struct Nearest {
    distance: f64,
}

impl Nearest {
    pub(crate) fn new(first_distance: f64) -> Self {
        Self {
            distance: first_distance,
        }
    }

    /// Strict improvement preserves the first tie and a first NaN distance.
    pub(crate) fn consider(&mut self, distance: f64) -> bool {
        if distance < self.distance {
            self.distance = distance;
            true
        } else {
            false
        }
    }
}

/// Pool access stays with its owner: borrowed native rows or observable V8
/// methods/getters. The driver below alone selects the pool and ranks rows.
pub trait Resolver {
    type Pool;
    type Row;
    type Error;

    fn pool(&mut self, local_only: bool) -> Result<Self::Pool, Self::Error>;
    fn first(&mut self, pool: &mut Self::Pool) -> Result<Option<(Self::Row, f64)>, Self::Error>;
    /// Visit one candidate, retaining it only if the shared rank prefers it.
    /// This lets V8 release the iteration's temporary handles before returning.
    fn next(
        &mut self,
        pool: &mut Self::Pool,
        prefer: impl FnOnce(f64) -> bool,
    ) -> Result<ControlFlow<(), Option<Self::Row>>, Self::Error>;
    fn find_named(&mut self) -> Result<Option<Self::Row>, Self::Error>;
}

pub fn resolve_with<R: Resolver>(
    mode: Resolution,
    resolver: &mut R,
) -> Result<Option<R::Row>, R::Error> {
    match mode {
        Resolution::Freeform => Ok(None),
        Resolution::Named => resolver.find_named(),
        Resolution::Closest | Resolution::Auto => {
            let mut pool = resolver.pool(mode == Resolution::Auto)?;
            let Some((mut best, distance)) = resolver.first(&mut pool)? else {
                return Ok(None);
            };
            let mut rank = Nearest::new(distance);
            while let ControlFlow::Continue(candidate) =
                resolver.next(&mut pool, |distance| rank.consider(distance))?
            {
                if let Some(row) = candidate {
                    best = row;
                }
            }
            Ok(Some(best))
        }
    }
}

struct NativePool<'a, T> {
    rows: std::slice::Iter<'a, T>,
    local_only: bool,
}

struct NativeResolver<'a, 's, T> {
    setting: &'s str,
    start: WorldTile,
    rows: &'a [T],
}

impl<'a, T: Location> NativeResolver<'a, '_, T> {
    fn candidate(&self, pool: &mut NativePool<'a, T>) -> Option<&'a T> {
        pool.rows
            .find(|row| !pool.local_only || same_map_square(self.start, row.spot()))
    }
}

impl<'a, T: Location> Resolver for NativeResolver<'a, '_, T> {
    type Pool = NativePool<'a, T>;
    type Row = &'a T;
    type Error = Infallible;

    fn pool(&mut self, local_only: bool) -> Result<Self::Pool, Self::Error> {
        Ok(NativePool {
            rows: self.rows.iter(),
            local_only,
        })
    }

    fn first(&mut self, pool: &mut Self::Pool) -> Result<Option<(Self::Row, f64)>, Self::Error> {
        Ok(self
            .candidate(pool)
            .map(|row| (row, distance(self.start, row.spot()))))
    }

    fn next(
        &mut self,
        pool: &mut Self::Pool,
        prefer: impl FnOnce(f64) -> bool,
    ) -> Result<ControlFlow<(), Option<Self::Row>>, Self::Error> {
        let Some(row) = self.candidate(pool) else {
            return Ok(ControlFlow::Break(()));
        };
        Ok(ControlFlow::Continue(
            prefer(distance(self.start, row.spot())).then_some(row),
        ))
    }

    fn find_named(&mut self) -> Result<Option<Self::Row>, Self::Error> {
        let normalized = if self.setting.is_ascii() {
            Cow::Borrowed(self.setting)
        } else {
            lowercase(self.setting)
        };
        Ok(self.rows.iter().find(|row| {
            let name = row.name();
            if name.is_ascii() {
                name.eq_ignore_ascii_case(&normalized)
            } else {
                lowercase(name).eq_ignore_ascii_case(&normalized)
            }
        }))
    }
}

fn distance(from: WorldTile, to: WorldTile) -> f64 {
    distance_delta(
        f64::from(to.x) - f64::from(from.x),
        f64::from(to.z) - f64::from(from.z),
    )
}

/// Resolve a preset without allocating a row pool or copying a selected row.
/// Auto is local to the start map square and plane, with no remote fallback.
/// Closest ranks every row by planar distance, with stable first-row ties.
/// Start/custom aliases return None. Other names are case-insensitive matches.
pub fn resolve<'a, T: Location>(setting: &str, start: WorldTile, rows: &'a [T]) -> Option<&'a T> {
    let setting = setting.trim_matches(ecma_whitespace);
    let mode = if setting.len() > MAX_ALIAS_LEN {
        Resolution::Named
    } else {
        resolution(|alias| setting.eq_ignore_ascii_case(alias))
    };
    resolve_with(
        mode,
        &mut NativeResolver {
            setting,
            start,
            rows,
        },
    )
    .unwrap_or_else(|never| match never {})
}

fn lowercase(value: &str) -> Cow<'_, str> {
    if value
        .chars()
        .any(|character| character.to_lowercase().ne(std::iter::once(character)))
    {
        Cow::Owned(value.to_lowercase())
    } else {
        Cow::Borrowed(value)
    }
}

fn ecma_whitespace(character: char) -> bool {
    matches!(
        character,
        '\u{0009}'..='\u{000d}'
            | ' '
            | '\u{00a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Row {
        name: &'static str,
        spot: WorldTile,
    }

    impl Location for Row {
        fn name(&self) -> &str {
            self.name
        }

        fn spot(&self) -> WorldTile {
            self.spot
        }
    }

    const fn tile(x: i32, z: i32, level: i32) -> WorldTile {
        WorldTile { x, z, level }
    }

    fn row(name: &'static str, x: i32, z: i32, level: i32) -> Row {
        Row {
            name,
            spot: tile(x, z, level),
        }
    }

    #[test]
    fn typed_presets_keep_pool_plane_boundaries_names_and_identity() {
        let start = tile(63, 10, 0);
        let rows = [
            row("Remote", 64, 10, 0),
            row("Local", 60, 10, 0),
            row("Other plane", 63, 10, 1),
            row("None", 1, 1, 0),
        ];
        assert!(std::ptr::eq(
            resolve(" Auto ", start, &rows).unwrap(),
            &rows[1]
        ));
        assert!(std::ptr::eq(
            resolve("Use Closest", start, &rows).unwrap(),
            &rows[2]
        ));
        assert!(std::ptr::eq(
            resolve(" nOnE ", start, &rows).unwrap(),
            &rows[3]
        ));
        assert!(resolve("Use Start Position", start, &rows).is_none());
        assert!(resolve("Use Custom Position", start, &rows).is_none());
        assert!(resolve("missing", start, &rows).is_none());
        assert!(resolve("Auto", tile(128, 10, 0), &rows).is_none());
        assert!(resolve("Auto", start, &[] as &[Row]).is_none());
        assert!(resolve("Use Closest", start, &[] as &[Row]).is_none());
        assert!(same_map_square(tile(-1, -65, 2), tile(-63, -127, 2)));
        assert!(!same_map_square(tile(-64, 0, 0), tile(-65, 0, 0)));
        assert!(!same_map_square(tile(63, 0, 0), tile(64, 0, 0)));
        assert!(!same_map_square(tile(1, 1, 0), tile(1, 1, 1)));
        let ties = [row("first", 3, 4, 0), row("second", 4, 3, 8)];
        assert!(std::ptr::eq(
            resolve("Use Closest", tile(0, 0, 0), &ties).unwrap(),
            &ties[0]
        ));
        let names = [row("K", 0, 0, 0), row("ΟΣ", 0, 0, 0)];
        assert!(std::ptr::eq(
            resolve("\u{feff}K\u{feff}", start, &names).unwrap(),
            &names[0]
        ));
        assert!(std::ptr::eq(
            resolve("ος", start, &names).unwrap(),
            &names[1]
        ));
        assert!(resolve("\u{0085}K", start, &names).is_none());
    }

    #[test]
    fn typed_options_defaults_and_streaming_rank_keep_edge_cases() {
        let rows = [
            row("Birch", 0, 0, 0),
            row("Birch", 0, 0, 0),
            row("", 0, 0, 0),
        ];
        assert!(location_options(&rows).eq([
            "Auto",
            "Use Closest",
            "Use Start Position",
            "Use Custom Position",
            "None",
            "Birch",
            "Birch",
            "",
        ]));
        assert_eq!(booth_fields(None, None), ("Bank booth", "Use-quickly"));
        assert_eq!(booth_fields(Some(""), Some("")), ("", ""));
        let mut rank = Nearest::new(f64::NAN);
        assert!(!rank.consider(0.0));
        let mut rank = Nearest::new(5.0);
        assert!(!rank.consider(5.0));
        assert!(!rank.consider(f64::NAN));
        assert!(rank.consider(4.0));
        assert!(!rank.consider(4.5));
        assert_eq!(distance_delta(3.0, 4.0), 5.0);
    }

    #[test]
    fn typed_common_path_keeps_zero_cold_and_warm_rust_heap() {
        let rows = [row("Local", 60, 10, 0), row("Remote", 64, 10, 0)];
        let check = || {
            let start = tile(63, 10, 0);
            assert!(std::ptr::eq(
                resolve("Auto", start, &rows).unwrap(),
                &rows[0]
            ));
            assert!(std::ptr::eq(
                resolve(" LOCAL ", start, &rows).unwrap(),
                &rows[0]
            ));
            assert_eq!(location_options(&rows).count(), 7);
            assert_eq!(
                booth_fields(None, None),
                (DEFAULT_BOOTH_NAME, DEFAULT_BOOTH_OP)
            );
            assert!(same_map_square(start, rows[0].spot));
        };
        let cold = allocation_counter::measure(check);
        let warm = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                check();
            }
        });
        println!("Gathering typed core cold={cold:?}; warm1000={warm:?}");
        assert_eq!(cold, allocation_counter::AllocationInfo::default());
        assert_eq!(warm, allocation_counter::AllocationInfo::default());
    }

    #[test]
    fn typed_driver_reads_only_the_selected_pool_or_name_path() {
        use std::cell::RefCell;

        #[derive(Debug, PartialEq, Eq)]
        enum Read {
            Name(&'static str),
            Spot(&'static str),
        }
        struct Observed<'a> {
            row: Row,
            reads: &'a RefCell<Vec<Read>>,
        }
        impl Location for Observed<'_> {
            fn name(&self) -> &str {
                self.reads.borrow_mut().push(Read::Name(self.row.name));
                self.row.name
            }
            fn spot(&self) -> WorldTile {
                self.reads.borrow_mut().push(Read::Spot(self.row.name));
                self.row.spot
            }
        }
        let reads = RefCell::new(Vec::new());
        let rows = [
            row("Remote", 64, 10, 0),
            row("Local", 60, 10, 0),
            row("Other plane", 63, 10, 1),
        ]
        .map(|row| Observed { row, reads: &reads });
        let start = tile(63, 10, 0);
        assert!(resolve("Use Start Position", start, &rows).is_none());
        assert!(reads.borrow().is_empty());
        assert!(std::ptr::eq(
            resolve("Local", start, &rows).unwrap(),
            &rows[1]
        ));
        assert_eq!(reads.take(), [Read::Name("Remote"), Read::Name("Local")]);
        assert!(std::ptr::eq(
            resolve("Use Closest", start, &rows).unwrap(),
            &rows[2]
        ));
        assert_eq!(
            reads.take(),
            [
                Read::Spot("Remote"),
                Read::Spot("Local"),
                Read::Spot("Other plane")
            ]
        );
        assert!(std::ptr::eq(
            resolve("Auto", start, &rows).unwrap(),
            &rows[1]
        ));
        assert_eq!(
            reads.take(),
            [
                Read::Spot("Remote"),
                Read::Spot("Local"),
                Read::Spot("Local"),
                Read::Spot("Other plane")
            ]
        );
    }
    #[test]
    fn radius_resolvers_floor_and_clamp_like_the_frozen_helpers() {
        assert_eq!(resolve_radius(Some(80.9), DEFAULT_CAMP_RADIUS), 80.0);
        assert_eq!(resolve_radius(Some(-5.0), DEFAULT_CAMP_RADIUS), MIN_RADIUS);
        assert_eq!(resolve_radius(None, DEFAULT_CHASE_RADIUS), 40.0);
        assert_eq!(resolve_radius(None, 12.7), 12.0);
        assert_eq!(resolve_radius(Some(1.0), DEFAULT_CAMP_RADIUS), MIN_RADIUS);
        assert!(
            resolve_radius(None, f64::NAN).is_nan(),
            "Math.max(2, NaN) is NaN"
        );
        assert_eq!(resolve_radius(None, f64::INFINITY), f64::INFINITY);
        assert_eq!(js_max(2.0, 3.0), 3.0);
        assert!(js_min(f64::NAN, 1.0).is_nan());
        assert_eq!(js_min(2.0, 1.0), 1.0);
    }

    #[test]
    fn published_option_values_and_defaults() {
        assert_eq!(
            [
                AUTO_LEGACY,
                USE_CLOSEST,
                USE_START_POSITION,
                USE_CUSTOM_POSITION,
                NONE_LEGACY
            ],
            OPTION_PREFIX
        );
        assert_eq!((DEFAULT_CAMP_RADIUS, DEFAULT_CHASE_RADIUS), (64.0, 40.0));
        if !api::test_support::embedded_game_data_available() {
            return;
        }
        assert_eq!(
            location_options(api::gather_camps::mining_camps().expect("mining camps")).count(),
            5 + 24
        );
    }

    #[test]
    fn the_generated_camp_tables_resolve_like_any_preset_table() {
        if !api::test_support::embedded_game_data_available() {
            return;
        }
        let camps = api::gather_camps::mining_camps().expect("mining camps");
        let start = WorldTile {
            x: 3290,
            z: 3370,
            level: 0,
        };
        let closest = resolve("Use Closest", start, camps).expect("closest camp");
        assert_eq!(closest.name, "Southeast Varrock Mine");
        assert_eq!(
            resolve("auto", start, camps).map(|camp| camp.name),
            Some("Southeast Varrock Mine")
        );
        assert_eq!(
            resolve(" dwarven mine ", start, camps).map(|camp| camp.name),
            Some("Dwarven Mine")
        );
        assert!(resolve("Use Start Position", start, camps).is_none());
        assert!(resolve("Use Custom Position", start, camps).is_none());
        assert!(resolve("nowhere", start, camps).is_none());
        let remote = WorldTile {
            x: 100,
            z: 100,
            level: 0,
        };
        assert!(
            resolve("Auto", remote, camps).is_none(),
            "Auto has no remote fallback"
        );
        assert!(resolve("Use Closest", remote, camps).is_some());
    }
}
