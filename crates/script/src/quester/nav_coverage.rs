//! Compile-time coordinate gate against the canonical 289 nav-pack input set.
//! The nav baker verifies this inventory against every `maps/*.jm2`; using the
//! same checked list prevents a Path from authoring a coordinate outside the
//! baked pack without making compiled scripts depend on the router crate.

const REQUIRED_289: &str = include_str!("../../../nav/src/required-content-289.tsv");

pub fn covered_289(x: i32, z: i32) -> bool {
    if x < 0 || z < 0 {
        return false;
    }
    let mx = x / 64;
    let mz = z / 64;
    REQUIRED_289.lines().any(|line| {
        let mut columns = line.split('\t');
        if columns.next() != Some("map") {
            return false;
        }
        let Some(path) = columns.next() else {
            return false;
        };
        let Some(stem) = path
            .strip_prefix("maps/m")
            .and_then(|path| path.strip_suffix(".jm2"))
        else {
            return false;
        };
        let Some((x, z)) = stem.split_once('_') else {
            return false;
        };
        x.parse::<i32>() == Ok(mx) && z.parse::<i32>() == Ok(mz)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_quest_tiles_are_covered_and_void_is_not() {
        for (x, z) in [(3166, 3305), (3197, 3266), (3103, 9572), (3253, 3402)] {
            assert!(covered_289(x, z), "{x},{z}");
        }
        assert!(!covered_289(0, 0));
        assert!(!covered_289(-1, 3300));
    }
}
