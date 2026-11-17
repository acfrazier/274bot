use super::*;
use client::io::cache_289::synthetic_jag;

fn config(locs: &[&[u8]], floors: &[u8]) -> JagFile {
    let mut data = (locs.len() as u16).to_be_bytes().to_vec();
    let mut index = data.clone();
    for record in locs {
        data.extend_from_slice(record);
        index.extend_from_slice(&(record.len() as u16).to_be_bytes());
    }
    JagFile::new(synthetic_jag(&[
        ("loc.dat", &data),
        ("loc.idx", &index),
        ("flo.dat", floors),
    ]))
}

#[test]
fn revision_377_locs_use_indexed_records_and_native_active_postpass() {
    let jag = config(
        &[
            &[14, 2, 15, 3, 68, 0, 7, 77, 255, 255, 0, 1, 0, 255, 255, 0],
            &[1, 1, 0, 42, 10, 0],
            &[30, b'h', b'i', b'd', b'd', b'e', b'n', 10, 0],
            &[19, 0, 5, 1, 0, 42, 0],
        ],
        &[0, 1, 0],
    );
    let locs = load_raster_locs(377, &jag).unwrap();
    assert_eq!(locs.len(), 4);
    assert_eq!(
        (locs[0].width, locs[0].length, locs[0].mapscene),
        (2, 3, Some(7))
    );
    assert!(!locs[0].active);
    assert!(locs[1].active);
    assert!(
        locs[2].active,
        "an allocated hidden-action array activates the loc"
    );
    assert!(!locs[3].active, "opcode 19 suppresses the model postpass");
}

#[test]
fn revision_377_floors_keep_unjittered_hsl_and_opcode_seven_chroma() {
    let jag = config(
        &[&[0]],
        &[0, 2, 1, 255, 0, 0, 2, 4, 0, 1, 0, 255, 0, 7, 0, 0, 255, 0],
    );
    let floors = load_floor_definitions(377, &jag).unwrap();
    assert_eq!(floors.len(), 2);
    let red = floors[0];
    assert_eq!((red.colour, red.texture), (0xff0000, Some(4)));
    assert_eq!(
        (red.saturation, red.lightness, red.chroma, red.underlay_hue),
        (255, 127, 255, 0)
    );
    let green = floors[1];
    assert_eq!((green.colour, green.texture), (0x00ff00, None));
    assert_eq!(
        (
            green.saturation,
            green.lightness,
            green.chroma,
            green.underlay_hue
        ),
        (255, 127, 85, 85)
    );
}

#[test]
fn revision_377_rejects_malformed_definitions_without_partial_results() {
    for record in [&[14][..], &[0, 0], &[255, 0], &[14, 0, 0]] {
        assert!(load_raster_locs(377, &config(&[record], &[0, 1, 0])).is_err());
    }
    for floors in [&[0, 1, 1, 255][..], &[0, 1, 0, 0], &[0, 1, 255, 0], &[0, 0]] {
        assert!(load_floor_definitions(377, &config(&[&[0]], floors)).is_err());
    }
    for (data, index) in [
        (&[0, 2, 0][..], &[0, 1, 0, 1][..]),
        (&[0, 1, 0], &[0, 1, 0, 2]),
        (&[0, 1, 0, 0], &[0, 1, 0, 1]),
        (&[0, 1, 0], &[0, 1, 0, 1, 0]),
        (&[0, 0], &[0, 0]),
    ] {
        let jag = JagFile::new(synthetic_jag(&[("loc.dat", data), ("loc.idx", index)]));
        assert!(load_raster_locs(377, &jag).is_err());
    }
    let empty = JagFile::new(synthetic_jag(&[]));
    assert!(load_raster_locs(377, &empty).is_err());
    assert!(load_floor_definitions(377, &empty).is_err());
    assert!(load_raster_locs(378, &empty).is_err());
    assert!(load_floor_definitions(378, &empty).is_err());
}
