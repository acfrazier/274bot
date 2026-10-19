//! Borrowed preset-location helpers shared by compiled callers and Load adapters.
//!
//! These presets are not the Gatherer's selected-catalog Start/Site/Auto/Custom
//! policy. No catalog, world snapshot, or selected location is retained here.

use api::snapshot::WorldTile;
use std::borrow::Cow;
use std::convert::Infallible;
use std::ops::ControlFlow;

pub(crate) const DEFAULT_BOOTH_NAME: &str = "Bank booth";
pub(crate) const DEFAULT_BOOTH_OP: &str = "Use-quickly";
pub(crate) const MAP_SQUARE: f64 = 64.0;
pub(crate) const OPTION_PREFIX: [&str; 5] = [
    "Auto",
    "Use Closest",
    "Use Start Position",
    "Use Custom Position",
    "None",
];
const CUSTOM_POSITION: &str = "use custom position";
pub(crate) const MAX_ALIAS_LEN: usize = CUSTOM_POSITION.len();

/// The immutable facts required from a native preset row.
pub trait Location {
    fn name(&self) -> &str;
    fn spot(&self) -> WorldTile;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Resolution {
    Freeform,
    Closest,
    Auto,
    Named,
}

/// The caller supplies equality so V8 can keep its original normalized value,
/// including non-string results from a caller's `toLowerCase` method.
pub(crate) fn resolution(mut equals: impl FnMut(&str) -> bool) -> Resolution {
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
pub(crate) enum Axis {
    X,
    Z,
}

/// Keep the same level/x/z short circuit for native tiles and foreign getters.
pub(crate) fn same_square_with<E>(
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

pub(crate) fn booth_name<T>(value: Option<T>, default: impl FnOnce(&'static str) -> T) -> T {
    value.unwrap_or_else(|| default(DEFAULT_BOOTH_NAME))
}

pub(crate) fn booth_op<T>(value: Option<T>, default: impl FnOnce(&'static str) -> T) -> T {
    value.unwrap_or_else(|| default(DEFAULT_BOOTH_OP))
}

/// Missing fields use the shared defaults. Present empty strings stay empty.
pub fn booth_fields<'a>(name: Option<&'a str>, op: Option<&'a str>) -> (&'a str, &'a str) {
    (booth_name(name, |value| value), booth_op(op, |value| value))
}

/// The planar bank-distance calculation also used by the Load bank adapter.
pub(crate) fn distance_delta(dx: f64, dz: f64) -> f64 {
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
pub(crate) trait Resolver {
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

pub(crate) fn resolve_with<R: Resolver>(
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
}
