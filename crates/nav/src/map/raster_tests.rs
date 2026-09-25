use super::*;

fn pixel(rgba: &[u8], x: usize, y: usize) -> [u8; 4] {
    let offset = (y * TILE_PIXELS as usize + x) * 4;
    rgba[offset..offset + 4].try_into().unwrap()
}

fn location_definition(mapscene: Option<u16>, active: bool) -> RasterLocDefinition {
    RasterLocDefinition {
        width: 1,
        length: 1,
        active,
        mapscene,
    }
}

fn placement(shape: u8, rotation: u8) -> LocPlacement {
    LocPlacement {
        id: 0,
        plane: 0,
        x: 0,
        z: 0,
        shape,
        rotation,
    }
}

#[test]
fn floor_masks_select_overlay_and_rotate_in_native_orientation() {
    let mut rgba = vec![0; TILE_RGBA_BYTES];
    let underlay = 0x102030;
    let overlay = 0xa0b0c0;

    draw_floor_cell(&mut rgba, 1, 1, Some(underlay), Some(overlay), 0, 0);
    for y in 1..5 {
        for x in 1..5 {
            assert_eq!(pixel(&rgba, x, y), [0xa0, 0xb0, 0xc0, 0xff]);
        }
    }

    rgba.fill(0);
    draw_floor_cell(&mut rgba, 1, 1, Some(underlay), Some(overlay), 1, 0);
    assert_eq!(pixel(&rgba, 1, 1), [0xa0, 0xb0, 0xc0, 0xff]);
    assert_eq!(pixel(&rgba, 2, 1), [0x10, 0x20, 0x30, 0xff]);
    assert_eq!(pixel(&rgba, 2, 2), [0xa0, 0xb0, 0xc0, 0xff]);
    assert_eq!(pixel(&rgba, 4, 3), [0x10, 0x20, 0x30, 0xff]);

    rgba.fill(0);
    draw_floor_cell(&mut rgba, 1, 1, Some(underlay), Some(overlay), 1, 1);
    assert_eq!(pixel(&rgba, 1, 1), [0xa0, 0xb0, 0xc0, 0xff]);
    assert_eq!(pixel(&rgba, 4, 1), [0xa0, 0xb0, 0xc0, 0xff]);
    assert_eq!(pixel(&rgba, 4, 4), [0x10, 0x20, 0x30, 0xff]);

    rgba.fill(0);
    draw_floor_cell(&mut rgba, 1, 1, Some(underlay), None, 7, 3);
    for y in 1..5 {
        for x in 1..5 {
            assert_eq!(pixel(&rgba, x, y), [0x10, 0x20, 0x30, 0xff]);
        }
    }
}

#[test]
fn native_l_walls_turn_clockwise_and_wall_decor_does_not_stamp_mapscene() {
    let assets = RasterAssets {
        mapscenes: vec![None],
        texture_colours: [0; 50],
    };
    let mut rgba = vec![0; TILE_RGBA_BYTES];
    draw_location(
        &mut rgba,
        0,
        1,
        0,
        0,
        placement(2, 0),
        &location_definition(None, false),
        &assets,
    )
    .unwrap();
    for offset in 0..4 {
        assert_eq!(pixel(&rgba, 1, 1 + offset), [0xee, 0xee, 0xee, 0xff]);
        assert_eq!(pixel(&rgba, 1 + offset, 1), [0xee, 0xee, 0xee, 0xff]);
    }
    assert_eq!(pixel(&rgba, 2, 4), [0; 4]);
    assert_eq!(pixel(&rgba, 4, 2), [0; 4]);

    let mut sprite = Pix8::new(1, 1, vec![0, 0x123456]);
    sprite.data[0] = 1;
    let assets = RasterAssets {
        mapscenes: vec![Some(sprite)],
        texture_colours: [0; 50],
    };
    rgba.fill(0);
    draw_location(
        &mut rgba,
        0,
        1,
        0,
        0,
        placement(4, 0),
        &location_definition(Some(0), false),
        &assets,
    )
    .unwrap();
    assert!(rgba.iter().all(|byte| *byte == 0));

    draw_location(
        &mut rgba,
        0,
        1,
        0,
        0,
        placement(10, 0),
        &location_definition(Some(0), false),
        &assets,
    )
    .unwrap();
    assert_eq!(pixel(&rgba, 2, 2), [0x12, 0x34, 0x56, 0xff]);
}

#[test]
fn transparent_downsample_uses_alpha_weighted_colour() {
    assert_eq!(
        average_rgba([
            [200, 100, 50, 255],
            [0, 255, 0, 0],
            [0, 0, 255, 0],
            [255, 255, 255, 0],
        ]),
        [200, 100, 50, 64]
    );
    assert_eq!(average_rgba([[1, 2, 3, 0]; 4]), [0; 4]);
}

#[test]
fn textured_overlays_follow_native_texture_precedence() {
    let floor = |colour, texture| FloorDefinition {
        colour,
        texture,
        saturation: 0,
        lightness: 0,
        chroma: 1,
        underlay_hue: 0,
    };
    let definitions = Definitions {
        locs: Vec::new(),
        floors: vec![
            floor(0xff00ff, Some(0)),
            floor(0xabcdef, Some(1)),
            floor(0xff00ff, None),
            floor(0x123456, None),
        ],
    };
    let mut texture_colours = [0; 50];
    texture_colours[0] = 0x654321;
    let assets = RasterAssets {
        mapscenes: Vec::new(),
        texture_colours,
    };

    assert_eq!(overlay_colour(&definitions, &assets, 1), Some(0x654321));
    assert_eq!(overlay_colour(&definitions, &assets, 2), None);
    assert_eq!(overlay_colour(&definitions, &assets, 3), None);
    assert_eq!(overlay_colour(&definitions, &assets, 4), Some(0x123456));
}
