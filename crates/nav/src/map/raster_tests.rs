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

#[test]
fn minimap_visibility_moves_vis_below_cells_and_hides_force_high_detail_cells() {
    let ordinary = GroundCell {
        flags: 0,
        ..GroundCell::default()
    };
    let vis_below = GroundCell {
        flags: MapFlag::VIS_BELOW as u8,
        ..GroundCell::default()
    };
    let forced = GroundCell {
        flags: MapFlag::FORCE_HIGH_DETAIL as u8,
        ..GroundCell::default()
    };
    let both = GroundCell {
        flags: (MapFlag::VIS_BELOW | MapFlag::FORCE_HIGH_DETAIL) as u8,
        ..GroundCell::default()
    };

    assert_eq!(minimap_plane(1, ordinary.flags), Some(1));
    assert_eq!(minimap_plane(1, vis_below.flags), Some(0));
    assert_eq!(minimap_plane(0, vis_below.flags), None);
    assert_eq!(minimap_plane(1, forced.flags), None);
    assert_eq!(minimap_plane(1, both.flags), Some(0));

    let bridged = effective_plane(1, true).unwrap();
    assert_eq!(bridged, 0);
    assert_eq!(minimap_plane(bridged, ordinary.flags), Some(0));
    assert_eq!(minimap_plane(bridged, vis_below.flags), None);
}

#[test]
fn planning_rejects_a_tile_count_above_the_shared_json_limit() {
    assert!(validate_image_tile_count(MAX_IMAGE_TILES).is_ok());
    assert!(matches!(
        validate_image_tile_count(MAX_IMAGE_TILES + 1),
        Err(MapError::Limit("image tile count"))
    ));
}

#[test]
fn resume_rewrites_uncheckpointed_tiles_byte_identically() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "274bot-raster-batch-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let identity = ImageIdentity {
        revision: 289,
        content: Digest([0x11; 32]),
        policy: Digest([0x22; 32]),
    };
    write_checkpoint(&root, identity, BakeStage::BaseTerrain, 1, &[]).unwrap();
    let initial_checkpoint = std::fs::read(root.join("checkpoint.json")).unwrap();
    let key = TileKey {
        plane: 0,
        lod: 0,
        x: 1,
        z: 1,
    };
    let png = encode_png(&vec![0; TILE_RGBA_BYTES]).unwrap();
    let mut completed = Vec::new();
    let mut completed_keys = std::collections::BTreeSet::new();
    let mut completed_bytes = 0;
    complete_tile(
        &root,
        key,
        &png,
        &mut completed,
        &mut completed_keys,
        &mut completed_bytes,
    )
    .unwrap();
    write_checkpoint_if_batch(&root, identity, BakeStage::BaseTerrain, 1, &completed).unwrap();
    assert_eq!(
        std::fs::read(root.join("checkpoint.json")).unwrap(),
        initial_checkpoint
    );

    let checkpoint =
        Checkpoint::decode(&initial_checkpoint, ArtifactIdentity::Image(identity)).unwrap();
    assert!(checkpoint.completed.as_slice().is_empty());
    let tile_path = root.join(key.relative_path().unwrap());
    std::fs::write(&tile_path, b"uncheckpointed").unwrap();

    let mut resumed = checkpoint.completed.as_slice().to_vec();
    let mut resumed_keys = std::collections::BTreeSet::new();
    let mut resumed_bytes = 0;
    complete_tile(
        &root,
        key,
        &png,
        &mut resumed,
        &mut resumed_keys,
        &mut resumed_bytes,
    )
    .unwrap();
    assert_eq!(std::fs::read(&tile_path).unwrap(), png);
    write_checkpoint(&root, identity, BakeStage::BaseTerrain, 1, &resumed).unwrap();
    let published = Checkpoint::decode(
        &std::fs::read(root.join("checkpoint.json")).unwrap(),
        ArtifactIdentity::Image(identity),
    )
    .unwrap();
    assert_eq!(published.completed.as_slice(), resumed.as_slice());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn completed_from_disk_accepts_a_valid_png() {
    let root = std::env::temp_dir().join(format!("274bot-raster-disk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let key = TileKey {
        plane: 0,
        lod: 0,
        x: 3,
        z: 4,
    };
    let png = encode_png(&vec![20u8; TILE_RGBA_BYTES]).unwrap();
    let path = root.join(key.relative_path().unwrap());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &png).unwrap();
    let unit = completed_from_disk(&root, key).unwrap();
    assert_eq!(unit.key, UnitKey::Terrain { tile: key });
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn resume_adopts_valid_disk_tiles_the_checkpoint_never_listed() {
    // A close mid-bake checkpoints nothing, so resume must pick up every
    // valid tile already written instead of rasterizing it again.
    let root = std::env::temp_dir().join(format!("274bot-raster-adopt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let key = |x| TileKey {
        plane: 0,
        lod: 0,
        x,
        z: 7,
    };
    let (valid, corrupt, missing) = (key(1), key(2), key(3));
    let png = encode_png(&vec![40u8; TILE_RGBA_BYTES]).unwrap();
    for (tile, bytes) in [(valid, png.as_slice()), (corrupt, b"not a png".as_slice())] {
        let path = root.join(tile.relative_path().unwrap());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
    }
    let mut completed = Vec::new();
    let mut completed_keys = BTreeSet::new();
    let mut completed_bytes = 0;
    adopt_existing_tiles(
        &root,
        [valid, corrupt, missing],
        |_| false,
        &mut completed,
        &mut completed_keys,
        &mut completed_bytes,
    )
    .unwrap();
    assert_eq!(completed_keys, BTreeSet::from([valid]));
    assert_eq!(
        completed,
        vec![CompletedUnit {
            key: UnitKey::Terrain { tile: valid },
            payload: PayloadReceipt {
                bytes: png.len() as u32,
                sha256: Digest::of(&png),
            },
        }]
    );
    assert_eq!(completed_bytes, png.len() as u64);

    // A tile the checkpoint does list must still verify on disk.
    let mut completed = Vec::new();
    let mut completed_keys = BTreeSet::new();
    let mut completed_bytes = 0;
    assert!(adopt_existing_tiles(
        &root,
        [corrupt],
        |tile| tile == corrupt,
        &mut completed,
        &mut completed_keys,
        &mut completed_bytes,
    )
    .is_err());
    std::fs::remove_dir_all(&root).unwrap();
}
