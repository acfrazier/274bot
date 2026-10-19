//! Borrowed preset-location helpers shared by compiled callers and Load adapters.
//!
//! These presets are not the Gatherer's selected-catalog Start/Site/Auto/Custom
//! policy. No catalog, world snapshot, or selected location is retained here.

use api::snapshot::WorldTile;
use std::borrow::Cow;
use std::convert::Infallible;

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
    match mode {
        Resolution::Freeform => None,
        mode @ (Resolution::Closest | Resolution::Auto) => {
            let mut candidates = rows
                .iter()
                .filter(|row| mode != Resolution::Auto || same_map_square(start, row.spot()));
            let mut best = candidates.next()?;
            let mut rank = Nearest::new(distance(start, best.spot()));
            for row in candidates {
                if rank.consider(distance(start, row.spot())) {
                    best = row;
                }
            }
            Some(best)
        }
        Resolution::Named => {
            let normalized = if setting.is_ascii() {
                Cow::Borrowed(setting)
            } else {
                lowercase(setting)
            };
            rows.iter().find(|row| {
                let name = row.name();
                if name.is_ascii() {
                    name.eq_ignore_ascii_case(&normalized)
                } else {
                    lowercase(name).eq_ignore_ascii_case(&normalized)
                }
            })
        }
    }
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
}
