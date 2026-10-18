use super::*;
use crate::map::identity::Digest;
use crate::map::producer::{ClientMapInput, ProducerStep};
use api::identity::{Cancellation, Deadline};
use api::work::{
    MonotonicClock, SharedCancellation, SliceBudget, StepContext, SystemClock, WorkPhase,
};
use client::io::cache_289::synthetic_jag;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

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
    let mut sink = AtomicImageTileSink::new(&root, |_: TileKey| None);
    let payload = sink.write_tile(key, &png).unwrap().unwrap();
    complete_tile(
        key,
        payload,
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
    let payload = sink.write_tile(key, &png).unwrap().unwrap();
    complete_tile(
        key,
        payload,
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

/// A varied tile, so its IDAT spans most of the file.
fn patterned_png() -> Vec<u8> {
    let rgba: Vec<u8> = (0..TILE_RGBA_BYTES)
        .map(|i| (i.wrapping_mul(31) ^ (i >> 7)) as u8)
        .collect();
    encode_png(&rgba).unwrap()
}

fn cursor_to_image_plan<S: ImageTileSink<Error = MapError>>(
    input: ClientMapInput<'_>,
    directory: &Path,
    sink: &mut S,
    clock: &SystemClock,
    cancellation: &SharedCancellation,
) -> ImageBakeCursor {
    let identity = image_identity(input).unwrap();
    let mut cursor =
        ImageBakeCursor::new(input, Arc::new(directory.to_path_buf()), identity).unwrap();
    for _ in 0..50_000 {
        if cursor.plan().is_some() {
            return cursor;
        }
        let mut context = raster_context(clock, cancellation, 64);
        match cursor.step(&mut context, sink).unwrap() {
            ProducerStep::Yielded => {}
            ProducerStep::Completed(_) => panic!("image cursor completed before plan exposure"),
            ProducerStep::Cancelled => panic!("image cursor cancelled before plan exposure"),
        }
        assert!(context.units_this_slice() <= 64);
        assert!(!context.continued_after_yield());
    }
    panic!("image cursor never exposed its plan");
}

fn finish_cursor_adoption<S: ImageTileSink<Error = MapError>>(
    cursor: &mut ImageBakeCursor,
    sink: &mut S,
    clock: &SystemClock,
    cancellation: &SharedCancellation,
) {
    let planned_tiles = cursor.plan().unwrap().tiles().len();
    for _ in 0..50_000 {
        if cursor.adoption_index == planned_tiles {
            return;
        }
        let mut context = raster_context(clock, cancellation, 1);
        match cursor.step(&mut context, sink).unwrap() {
            ProducerStep::Yielded => {}
            ProducerStep::Completed(_) => panic!("image cursor completed during tile adoption"),
            ProducerStep::Cancelled => panic!("image cursor cancelled during tile adoption"),
        }
        assert!(context.units_this_slice() <= 1);
        assert!(!context.continued_after_yield());
    }
    panic!("image cursor did not finish its adoption scan");
}

fn write_existing_tile(directory: &Path, key: TileKey, bytes: &[u8]) {
    let path = directory.join(key.relative_path().unwrap());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn resume_adopts_only_disk_tiles_that_fully_decode() {
    let snapshot = RasterSnapshot::new();
    let png = patterned_png();
    let mut crc_damaged = png.clone();
    crc_damaged[png.len() / 2] ^= 0x5a;
    let clock = SystemClock;
    let cancellation = SharedCancellation::new();

    let output = snapshot.output("adopt");
    let recorded = std::cell::Cell::new(None::<(TileKey, PayloadReceipt)>);
    let mut sink = AtomicImageTileSink::new(&output, |key| {
        recorded
            .get()
            .and_then(|(recorded_key, receipt)| (key == recorded_key).then_some(receipt))
    });
    let mut cursor =
        cursor_to_image_plan(snapshot.input(), &output, &mut sink, &clock, &cancellation);
    let keys: Vec<_> = cursor.plan().unwrap().tiles().collect();
    assert_eq!(keys.len(), 3);
    let (valid, truncated, crc) = (keys[0], keys[1], keys[2]);
    let expected = PayloadReceipt {
        bytes: png.len() as u32,
        sha256: Digest::of(&png),
    };
    recorded.set(Some((valid, expected)));
    write_existing_tile(&output, valid, &png);
    write_existing_tile(&output, truncated, &png[..png.len() / 2]);
    write_existing_tile(&output, crc, &crc_damaged);
    finish_cursor_adoption(&mut cursor, &mut sink, &clock, &cancellation);
    assert_eq!(cursor.completed_keys, BTreeSet::from([valid]));
    assert_eq!(
        cursor.completed,
        vec![CompletedUnit {
            key: UnitKey::Terrain { tile: valid },
            payload: expected,
        }]
    );
    assert_eq!(cursor.completed_bytes, png.len() as u64);

    let mismatch_output = snapshot.output("adopt-mismatch");
    let recorded = std::cell::Cell::new(None::<(TileKey, PayloadReceipt)>);
    let mut sink = AtomicImageTileSink::new(&mismatch_output, |key| {
        recorded
            .get()
            .and_then(|(recorded_key, receipt)| (key == recorded_key).then_some(receipt))
    });
    let mut cursor = cursor_to_image_plan(
        snapshot.input(),
        &mismatch_output,
        &mut sink,
        &clock,
        &cancellation,
    );
    let keys: Vec<_> = cursor.plan().unwrap().tiles().collect();
    let valid = keys[0];
    recorded.set(Some((valid, expected)));
    let other = encode_png(&vec![7u8; TILE_RGBA_BYTES]).unwrap();
    write_existing_tile(&mismatch_output, valid, &other);
    write_existing_tile(&mismatch_output, keys[1], b"not a png");
    finish_cursor_adoption(&mut cursor, &mut sink, &clock, &cancellation);
    assert!(cursor.completed.is_empty());
    assert!(cursor.completed_keys.is_empty());
    assert_eq!(cursor.completed_bytes, 0);
}

static NEXT_RASTER_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct RasterSnapshot(PathBuf);

impl RasterSnapshot {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "274bot-raster-cursor-{}-{}",
            std::process::id(),
            NEXT_RASTER_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();

        let loc_dat = [0, 1, 0];
        let loc_idx = [0, 1, 0, 0];
        let flo_dat = [0, 1, 1, 0xff, 0, 0, 0];
        std::fs::write(
            root.join("config"),
            synthetic_jag(&[
                ("loc.dat", &loc_dat),
                ("loc.idx", &loc_idx),
                ("flo.dat", &flo_dat),
            ]),
        )
        .unwrap();
        std::fs::write(root.join("media"), synthetic_jag(&[])).unwrap();
        std::fs::write(root.join("textures"), synthetic_jag(&[])).unwrap();

        let mut map_index = Vec::new();
        for (square, land_file) in [((50u16 << 8) | 50, 0u16), ((51u16 << 8) | 50, 1u16)] {
            map_index.extend(square.to_be_bytes());
            map_index.extend(land_file.to_be_bytes());
            map_index.extend(u16::MAX.to_be_bytes());
            map_index.push(0);
        }
        std::fs::write(
            root.join("versionlist"),
            synthetic_jag(&[("map_index", map_index.as_slice())]),
        )
        .unwrap();

        let mut maps = Vec::new();
        for id in 0..2u32 {
            // The first cell has an underlay opcode and still needs its
            // terminating zero; all remaining cells use one zero each.
            let mut land = vec![0; 4 * 64 * 64 + 1];
            land[0] = 82;
            maps.extend(id.to_le_bytes());
            maps.extend((land.len() as u32).to_le_bytes());
            maps.extend(land);
        }
        std::fs::write(root.join("maps.bin"), maps).unwrap();
        Self(root)
    }

    fn input(&self) -> ClientMapInput<'_> {
        ClientMapInput::new(289, Digest([0x31; 32]), &self.0, &self.0).unwrap()
    }

    fn output(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
}

impl Drop for RasterSnapshot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RasterTestPhase;

impl WorkPhase for RasterTestPhase {
    fn index(self) -> u16 {
        0
    }
}

fn raster_context<'a>(
    clock: &'a SystemClock,
    cancellation: &'a SharedCancellation,
    units: u32,
) -> StepContext<'a, RasterTestPhase> {
    let budget = SliceBudget::new(
        NonZeroU32::new(units).unwrap(),
        Deadline::after(clock.now(), Duration::from_secs(60)),
    );
    StepContext::new(clock, budget, cancellation)
}

struct RasterStepClock {
    origin: Instant,
    tick: Duration,
    reads: AtomicUsize,
}

impl RasterStepClock {
    fn new(tick: Duration) -> Self {
        Self {
            origin: Instant::now(),
            tick,
            reads: AtomicUsize::new(0),
        }
    }

    fn context<'a>(
        &'a self,
        cancellation: &'a SharedCancellation,
        units: u32,
        duration: Duration,
    ) -> StepContext<'a, RasterTestPhase> {
        StepContext::new(
            self,
            SliceBudget::new(
                NonZeroU32::new(units).unwrap(),
                Deadline::after(self.origin, duration),
            ),
            cancellation,
        )
    }
}

impl MonotonicClock for RasterStepClock {
    fn now(&self) -> Instant {
        let index = self.reads.fetch_add(1, Ordering::Relaxed);
        self.origin + self.tick * u32::try_from(index).unwrap()
    }
}

fn terrain_cell_planner(snapshot: &RasterSnapshot) -> RasterPlanCursor {
    let mut planner = RasterPlanCursor::new(snapshot.input(), None).unwrap();
    for _ in 0..10_000 {
        if matches!(planner.phase, RasterPlanPhase::TerrainCell) {
            return planner;
        }
        planner.advance().unwrap();
    }
    panic!("raster plan did not reach terrain cells");
}

#[test]
fn immutable_raster_capacity_cache_matches_exact_accounting() {
    let snapshot = RasterSnapshot::new();
    let mut planner = terrain_cell_planner(&snapshot);
    let RasterContext {
        mut definitions,
        mut assets,
        reader,
        ..
    } = planner.context.take().unwrap();
    definitions.locs.reserve(13);
    definitions.floors.reserve(17);
    let mut sprite = Pix8::new(2, 2, vec![0, 0x123456]);
    sprite.data.reserve(37);
    sprite.bpal.reserve(9);
    assets.mapscenes.push(Some(sprite));
    assets.mapscenes.reserve(11);
    let expected = raster_resources_bytes(&definitions, &assets);
    let mut context = RasterContext::new(definitions, assets, reader);
    assert_eq!(context.resources_bytes, expected);
    for index in 0..context.reader.entries.len() {
        let entry = context.reader.entries[index];
        let _land = context.reader.read_land(entry).unwrap();
        assert_eq!(
            context.resources_bytes,
            raster_resources_bytes(&context.definitions, &context.assets)
        );
        assert_eq!(
            context.tracked_bytes(),
            expected + context.reader.tracked_bytes() as u64
        );
    }
}

fn parent_row_cursor(snapshot: &RasterSnapshot) -> ImageBakeCursor {
    let input = snapshot.input();
    let mut cursor = ImageBakeCursor::new(
        input,
        Arc::new(snapshot.output("parent-rows")),
        image_identity(input).unwrap(),
    )
    .unwrap();
    cursor.planner = None;
    cursor.phase = RasterBakePhase::RasterizeParentRow;
    cursor.rgba = vec![0; TILE_RGBA_BYTES];
    cursor
}

#[test]
fn raster_producers_check_once_at_entry_and_after_every_unit() {
    let snapshot = RasterSnapshot::new();
    let cancellation = SharedCancellation::new();
    let units = 16;
    let clock = RasterStepClock::new(Duration::ZERO);
    let mut context = clock.context(&cancellation, units, Duration::from_secs(1));
    let mut planner = terrain_cell_planner(&snapshot);
    assert!(matches!(
        planner.step(&mut context).unwrap(),
        ProducerStep::Yielded
    ));
    assert_eq!(context.units_this_slice(), units);
    assert_eq!(planner.terrain_cell_index, units as usize);
    assert_eq!(clock.reads.load(Ordering::Relaxed), units as usize + 1);
    assert!(!context.continued_after_yield());
    assert!(!context.exceeded_allowance());

    let clock = RasterStepClock::new(Duration::ZERO);
    let mut context = clock.context(&cancellation, units, Duration::from_secs(1));
    let mut cursor = parent_row_cursor(&snapshot);
    let mut sink = TestImageTileSink::new(snapshot.output("unused-sink"));
    assert!(matches!(
        cursor.step(&mut context, &mut sink).unwrap(),
        ProducerStep::Yielded
    ));
    assert_eq!(context.units_this_slice(), units);
    assert_eq!(cursor.parent_row, units as usize);
    assert_eq!(clock.reads.load(Ordering::Relaxed), units as usize + 1);
    assert!(!context.continued_after_yield());
    assert!(!context.exceeded_allowance());
    assert_eq!(sink.write_attempts, 0);
}

#[test]
fn raster_producer_deadlines_still_stop_after_one_unit_crosses_the_deadline() {
    let snapshot = RasterSnapshot::new();
    let cancellation = SharedCancellation::new();
    let tick = Duration::from_micros(2);
    let duration = Duration::from_micros(5);
    let clock = RasterStepClock::new(tick);
    let mut context = clock.context(&cancellation, 64, duration);
    let mut planner = terrain_cell_planner(&snapshot);
    assert!(matches!(
        planner.step(&mut context).unwrap(),
        ProducerStep::Yielded
    ));
    assert_eq!(context.units_this_slice(), 3);
    assert_eq!(planner.terrain_cell_index, 3);
    assert!(!context.continued_after_yield());
    let last_check = tick * (clock.reads.load(Ordering::Relaxed) - 1) as u32;
    assert!(last_check >= duration);
    assert!(last_check - duration < tick);

    let clock = RasterStepClock::new(tick);
    let mut context = clock.context(&cancellation, 64, duration);
    let mut cursor = parent_row_cursor(&snapshot);
    let mut sink = TestImageTileSink::new(snapshot.output("deadline-sink"));
    assert!(matches!(
        cursor.step(&mut context, &mut sink).unwrap(),
        ProducerStep::Yielded
    ));
    assert_eq!(context.units_this_slice(), 3);
    assert_eq!(cursor.parent_row, 3);
    assert!(!context.continued_after_yield());
    let last_check = tick * (clock.reads.load(Ordering::Relaxed) - 1) as u32;
    assert!(last_check >= duration);
    assert!(last_check - duration < tick);
    assert_eq!(sink.write_attempts, 0);
}

struct TestImageTileSink {
    directory: PathBuf,
    backpressure_once: bool,
    fail_once: bool,
    write_attempts: usize,
    writes: usize,
}

impl TestImageTileSink {
    fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            backpressure_once: false,
            fail_once: false,
            write_attempts: 0,
            writes: 0,
        }
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
enum TestSinkError {
    #[error("injected sink failure")]
    Injected,
    #[error("atomic tile write failed")]
    AtomicWrite,
}

#[test]
fn image_bake_error_implements_std_error() {
    fn assert_error<T: std::error::Error>() {}
    assert_error::<super::ImageBakeError<TestSinkError>>();
}

impl ImageTileSink for TestImageTileSink {
    type Error = TestSinkError;

    fn checkpointed(&mut self, _key: TileKey) -> Option<PayloadReceipt> {
        None
    }

    fn write_tile(
        &mut self,
        key: TileKey,
        png: &[u8],
    ) -> Result<Option<PayloadReceipt>, Self::Error> {
        self.write_attempts += 1;
        if self.fail_once {
            self.fail_once = false;
            return Err(TestSinkError::Injected);
        }
        if self.backpressure_once {
            self.backpressure_once = false;
            return Ok(None);
        }
        let path = self.directory.join(
            key.relative_path()
                .map_err(|_| TestSinkError::AtomicWrite)?,
        );
        atomic_write(&path, png).map_err(|_| TestSinkError::AtomicWrite)?;
        self.writes += 1;
        Ok(Some(PayloadReceipt {
            bytes: png.len() as u32,
            sha256: Digest::of(png),
        }))
    }
}

fn reach_first_base_write<S: ImageTileSink>(
    cursor: &mut ImageBakeCursor,
    sink: &mut S,
    clock: &SystemClock,
    cancellation: &SharedCancellation,
) where
    S::Error: std::fmt::Debug,
{
    for _ in 0..50_000 {
        if matches!(&cursor.phase, RasterBakePhase::WriteBase) {
            return;
        }
        let mut context = raster_context(clock, cancellation, 1);
        match cursor.step(&mut context, sink).unwrap() {
            ProducerStep::Yielded => {}
            ProducerStep::Completed(_) => panic!("image cursor completed before base write"),
            ProducerStep::Cancelled => panic!("image cursor cancelled before base write"),
        }
        assert!(context.units_this_slice() <= 1);
        assert!(!context.continued_after_yield());
    }
    panic!("image cursor never reached the first base write");
}

fn reach_first_parent_write<S: ImageTileSink>(
    cursor: &mut ImageBakeCursor,
    sink: &mut S,
    clock: &SystemClock,
    cancellation: &SharedCancellation,
) where
    S::Error: std::fmt::Debug,
{
    for _ in 0..50_000 {
        if matches!(&cursor.phase, RasterBakePhase::WriteParent) {
            return;
        }
        let mut context = raster_context(clock, cancellation, 1);
        match cursor.step(&mut context, sink).unwrap() {
            ProducerStep::Yielded => {}
            ProducerStep::Completed(_) => panic!("image cursor completed before parent write"),
            ProducerStep::Cancelled => panic!("image cursor cancelled before parent write"),
        }
        assert!(context.units_this_slice() <= 1);
        assert!(!context.continued_after_yield());
    }
    panic!("image cursor never reached the first parent write");
}

#[test]
fn image_cursor_retries_tile_write_after_sink_backpressure() {
    let snapshot = RasterSnapshot::new();
    let output = snapshot.output("backpressure");
    let input = snapshot.input();
    let identity = image_identity(input).unwrap();
    let mut cursor = ImageBakeCursor::new(input, Arc::new(output.clone()), identity).unwrap();
    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    let mut sink = TestImageTileSink::new(output.clone());
    sink.backpressure_once = true;
    reach_first_base_write(&mut cursor, &mut sink, &clock, &cancellation);
    let key = cursor.current_tile.unwrap();
    let png = cursor.png.clone();
    let path = output.join(key.relative_path().unwrap());

    let mut context = raster_context(&clock, &cancellation, 1);
    assert!(matches!(
        cursor.step(&mut context, &mut sink).unwrap(),
        ProducerStep::Yielded
    ));
    assert_eq!(context.units_this_slice(), 0);
    assert!(matches!(&cursor.phase, RasterBakePhase::WriteBase));
    assert_eq!(cursor.current_tile, Some(key));
    assert_eq!(cursor.png, png);
    assert!(!path.exists());
    assert_eq!(sink.write_attempts, 1);

    let mut context = raster_context(&clock, &cancellation, 1);
    assert!(matches!(
        cursor.step(&mut context, &mut sink).unwrap(),
        ProducerStep::Yielded
    ));
    assert_eq!(context.units_this_slice(), 1);
    assert_eq!(cursor.current_tile, None);
    assert_eq!(std::fs::read(path).unwrap(), png);
    assert_eq!(sink.write_attempts, 2);
    assert_eq!(sink.writes, 1);
    assert!(cursor.completed_keys.contains(&key));
}

#[test]
fn image_cursor_retries_parent_write_after_sink_backpressure() {
    let snapshot = RasterSnapshot::new();
    let output = snapshot.output("parent-backpressure");
    let input = snapshot.input();
    let identity = image_identity(input).unwrap();
    let mut cursor = ImageBakeCursor::new(input, Arc::new(output.clone()), identity).unwrap();
    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    let mut sink = TestImageTileSink::new(output.clone());
    reach_first_parent_write(&mut cursor, &mut sink, &clock, &cancellation);
    let key = cursor.current_tile.unwrap();
    let png = cursor.png.clone();
    let path = output.join(key.relative_path().unwrap());
    assert!(!path.exists());
    let writes_before = sink.writes;
    sink.backpressure_once = true;

    let mut context = raster_context(&clock, &cancellation, 1);
    assert!(matches!(
        cursor.step(&mut context, &mut sink).unwrap(),
        ProducerStep::Yielded
    ));
    assert_eq!(context.units_this_slice(), 0);
    assert!(matches!(&cursor.phase, RasterBakePhase::WriteParent));
    assert_eq!(cursor.current_tile, Some(key));
    assert_eq!(cursor.png, png);
    assert!(!path.exists());
    assert_eq!(sink.writes, writes_before);

    let mut context = raster_context(&clock, &cancellation, 1);
    assert!(matches!(
        cursor.step(&mut context, &mut sink).unwrap(),
        ProducerStep::Yielded
    ));
    assert_eq!(context.units_this_slice(), 1);
    assert_eq!(cursor.current_tile, None);
    assert_eq!(std::fs::read(path).unwrap(), png);
    assert_eq!(sink.writes, writes_before + 1);
    assert!(cursor.completed_keys.contains(&key));
}

#[test]
fn image_cursor_preserves_typed_sink_error_without_writing_tile() {
    let snapshot = RasterSnapshot::new();
    let output = snapshot.output("sink-error");
    let input = snapshot.input();
    let identity = image_identity(input).unwrap();
    let mut cursor = ImageBakeCursor::new(input, Arc::new(output.clone()), identity).unwrap();
    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    let mut sink = TestImageTileSink::new(output.clone());
    sink.fail_once = true;
    reach_first_base_write(&mut cursor, &mut sink, &clock, &cancellation);
    let key = cursor.current_tile.unwrap();
    let png = cursor.png.clone();
    let path = output.join(key.relative_path().unwrap());

    let mut context = raster_context(&clock, &cancellation, 64);
    assert!(matches!(
        cursor.step(&mut context, &mut sink),
        Err(ImageBakeError::Sink(TestSinkError::Injected))
    ));
    assert_eq!(context.units_this_slice(), 0);
    assert!(matches!(&cursor.phase, RasterBakePhase::WriteBase));
    assert_eq!(cursor.current_tile, Some(key));
    assert_eq!(cursor.png, png);
    assert!(!path.exists());
    assert!(std::fs::read_dir(&output).unwrap().next().is_none());
    assert_eq!(sink.write_attempts, 1);
    assert_eq!(sink.writes, 0);
}

#[test]
fn image_cursor_yields_without_repeating_and_matches_sync_tile_bytes() {
    let snapshot = RasterSnapshot::new();
    let sync_dir = snapshot.output("sync");
    let cursor_dir = snapshot.output("cursor");
    let input = snapshot.input();
    let expected = match bake_images_into(input, &sync_dir, |_| None, |_| true).unwrap() {
        ImageBakeOutcome::Complete(report) => report,
        ImageBakeOutcome::Paused(_) => panic!("sync raster unexpectedly paused"),
    };
    let identity = image_identity(input).unwrap();
    let mut cursor = ImageBakeCursor::new(input, Arc::new(cursor_dir.clone()), identity).unwrap();
    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    let mut sink = AtomicImageTileSink::new(&cursor_dir, |_: TileKey| None);
    let mut planned_yield = false;
    let mut completed_keys = Vec::new();
    let mut planning_squares_seen = false;
    let mut last_planning_squares = 0;
    let mut saw_base_terrain = false;
    let mut saw_downsample = false;
    let mut saw_publishing = false;

    let report = loop {
        let mut context = raster_context(&clock, &cancellation, 64);
        let step = cursor.step(&mut context, &mut sink).unwrap();
        assert!(context.units_this_slice() <= 64);
        assert!(!context.continued_after_yield());
        let progress = cursor.progress();
        if let Some(total) = progress.planning_squares_total {
            assert_eq!(total, 2, "planning reports the source map-square count");
            assert!(progress.planning_squares_completed <= total);
            assert!(progress.planning_squares_completed >= last_planning_squares);
            last_planning_squares = progress.planning_squares_completed;
            planning_squares_seen = true;
        }
        match progress.stage {
            RasterStage::ReadingCache => {}
            RasterStage::BaseTerrain { .. } => {
                assert!(progress.total_tiles > 0);
                assert!(progress.completed_tiles <= progress.total_tiles);
                saw_base_terrain = true;
            }
            RasterStage::Downsample { .. } => {
                assert!(progress.total_tiles > 0);
                assert!(progress.completed_tiles <= progress.total_tiles);
                saw_downsample = true;
            }
            RasterStage::Publishing => {
                assert!(progress.total_tiles > 0);
                assert_eq!(progress.completed_tiles, progress.total_tiles);
                saw_publishing = true;
            }
        }
        if cursor.plan().is_some() && !planned_yield {
            assert!(matches!(&step, ProducerStep::Yielded));
            assert_eq!(cursor.planned_units(), Some(3));
            assert_eq!(progress.total_tiles, 3);
            assert!(std::fs::read_dir(&cursor_dir).unwrap().next().is_none());
            planned_yield = true;
        }
        for unit in cursor.take_completed_units() {
            assert!(
                !completed_keys.contains(&unit.key),
                "tile completion was reported twice: {:?}",
                unit.key
            );
            completed_keys.push(unit.key);
        }
        match step {
            ProducerStep::Yielded => {}
            ProducerStep::Completed(report) => break report,
            ProducerStep::Cancelled => panic!("unexpected image cursor cancellation"),
        }
    };

    assert!(planned_yield, "planner did not expose its completed plan");
    assert!(
        planning_squares_seen,
        "planner never exposed a source-square total"
    );
    assert_eq!(last_planning_squares, 2);
    assert!(
        saw_base_terrain,
        "base terrain progress omitted its tile total"
    );
    assert!(saw_downsample, "downsample progress omitted its tile total");
    assert!(saw_publishing, "publishing progress omitted its tile total");

    assert_eq!(
        completed_keys.len(),
        expected.manifest.tiles.as_slice().len()
    );
    assert_eq!(
        report.manifest.encode().unwrap(),
        expected.manifest.encode().unwrap()
    );
    for receipt in report.manifest.tiles.as_slice() {
        let path = receipt.key.relative_path().unwrap();
        assert_eq!(
            std::fs::read(sync_dir.join(&path)).unwrap(),
            std::fs::read(cursor_dir.join(path)).unwrap()
        );
    }
}

#[test]
fn image_cursor_cancels_at_each_progress_stage_and_before_publication() {
    for target in [
        RasterStage::ReadingCache,
        RasterStage::BaseTerrain { plane: 0 },
        RasterStage::Downsample { plane: 0, lod: 1 },
        RasterStage::Publishing,
    ] {
        let snapshot = RasterSnapshot::new();
        let output = snapshot.output("cancelled");
        let input = snapshot.input();
        let identity = image_identity(input).unwrap();
        let mut cursor = ImageBakeCursor::new(input, Arc::new(output.clone()), identity).unwrap();
        let clock = SystemClock;
        let cancellation = SharedCancellation::new();
        let mut sink = AtomicImageTileSink::new(&output, |_: TileKey| None);
        let mut cancelled = false;

        for _ in 0..10_000 {
            if cursor.progress().stage == target {
                cancellation.register(Cancellation::Operator).unwrap();
                let mut context = raster_context(&clock, &cancellation, 1);
                assert!(matches!(
                    cursor.step(&mut context, &mut sink),
                    Ok(ProducerStep::Cancelled)
                ));
                assert!(!output.join("manifest.json").exists());
                cancelled = true;
                break;
            }
            let units = if cursor.plan().is_some() { 1 } else { 64 };
            let mut context = raster_context(&clock, &cancellation, units);
            match cursor.step(&mut context, &mut sink).unwrap() {
                ProducerStep::Yielded => {
                    assert!(context.units_this_slice() <= units);
                    assert!(!context.continued_after_yield());
                }
                ProducerStep::Completed(_) => {
                    panic!("image cursor completed before cancellation at {target:?}")
                }
                ProducerStep::Cancelled => panic!("unexpected image cursor cancellation"),
            }
        }
        assert!(cancelled, "image cursor never reached {target:?}");
    }
}
#[cfg(test)]
mod native_map_unit_qualification {
    use super::*;
    use client::config::{FloType, LocType, NpcType};
    use client::io::JagFile;
    use client::map_cache::{
        decode_map_index, visit_locations, MapIndexEntry, MAP_INDEX_ROW_BYTES, MAP_PLANES,
        MAP_SQUARE_SIZE,
    };
    use std::fs::{self, File};
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    const U24_MAX: usize = 0x00ff_ffff;
    const REAL_ARCHIVES: [&str; 4] = ["config", "media", "textures", "versionlist"];
    const PNG_DECODER_DEFAULT_LIMIT_BYTES: u64 = 64 * 1024 * 1024;
    const PNG_DECODER_INITIAL_DATA_BYTES: u64 = 128 * 1024;
    const PNG_DECODER_CHUNK_BUFFER_BYTES: u64 = 4 * 1024;
    const LAND_CELL_COUNT: usize =
        MAP_PLANES as usize * MAP_SQUARE_SIZE as usize * MAP_SQUARE_SIZE as usize;

    static NEXT_NATIVE_FIXTURE: AtomicUsize = AtomicUsize::new(0);

    struct NativeFixture(PathBuf);

    impl NativeFixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "274bot-native-map-qualification-{}-{}",
                std::process::id(),
                NEXT_NATIVE_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }

        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for NativeFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct RealCache {
        revision: u16,
        jag_dir: PathBuf,
        snapshot_dir: PathBuf,
    }

    impl RealCache {
        fn required_directory(variable: &str) -> PathBuf {
            let path = std::env::var_os(variable).unwrap_or_else(|| {
                panic!("required prepared-cache input env `{variable}` is unset")
            });
            let path = PathBuf::from(path);
            assert!(
                path.is_dir(),
                "prepared-cache input env `{variable}` must name an existing directory"
            );
            path
        }

        fn new(revision: u16) -> Self {
            let jag_variable = format!("T5W_MAP_BAKE_JAG_DIR_{revision}");
            let snapshot_variable = format!("T5W_MAP_BAKE_SNAPSHOT_DIR_{revision}");
            let jag_dir = Self::required_directory(&jag_variable);
            let snapshot_dir = Self::required_directory(&snapshot_variable);
            for name in REAL_ARCHIVES {
                let path = jag_dir.join(name);
                let metadata = fs::metadata(&path).unwrap_or_else(|_| {
                    panic!("prepared revision {revision} is missing required `{name}` JAG")
                });
                assert!(
                    metadata.len() > 0 && metadata.len() <= MAX_JAG_BYTES,
                    "prepared revision {revision} `{name}` JAG is outside the guarded file-size range"
                );
                let jag = read_jag(&path, "qualification JAG").unwrap_or_else(|_| {
                    panic!("prepared revision {revision} `{name}` JAG failed guarded open")
                });
                assert!(jag.file_count > 0);
                drop(jag);
            }
            let maps = snapshot_dir.join("maps.bin");
            assert!(
                fs::metadata(&maps).is_ok_and(|metadata| metadata.len() > 0),
                "prepared revision {revision} is missing a nonempty maps.bin"
            );
            Self {
                revision,
                jag_dir,
                snapshot_dir,
            }
        }

        fn path(&self, archive: &str) -> PathBuf {
            self.jag_dir.join(archive)
        }

        fn archive_bytes(&self, archive: &str) -> usize {
            usize::try_from(fs::metadata(self.path(archive)).unwrap().len()).unwrap()
        }

        fn open_jag(&self, archive: &str) -> JagFile {
            read_jag(&self.path(archive), "qualification JAG").unwrap_or_else(|_| {
                panic!(
                    "prepared revision {} `{archive}` JAG failed open",
                    self.revision
                )
            })
        }

        fn member_len(&self, archive: &str, member: &str) -> usize {
            let jag = self.open_jag(archive);
            let bytes = jag.read(member).unwrap_or_else(|| {
                panic!(
                    "prepared revision {} `{archive}` lacks `{member}`",
                    self.revision
                )
            });
            bytes.len()
        }

        fn input(&self) -> ClientMapInput<'_> {
            ClientMapInput::new(
                self.revision,
                Digest([self.revision as u8; 32]),
                &self.jag_dir,
                &self.snapshot_dir,
            )
            .unwrap()
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one measurement case needs its input values and measured action"
    )]
    fn measure_case(
        case: &str,
        kind: &str,
        revision: &str,
        native_input_bytes: usize,
        archive_bytes: usize,
        counts: &str,
        format_bound_bytes: u64,
        source_peak_bound_bytes: Option<u64>,
        run: impl FnOnce(),
    ) {
        let started = Instant::now();
        let allocations = allocation_counter::measure(run);
        let elapsed_ns = started.elapsed().as_nanos();
        assert_eq!(
            allocations.bytes_current, 0,
            "{case} must release native output, scratch, and any measured archive owner"
        );
        if let Some(bound) = source_peak_bound_bytes {
            assert!(
                allocations.bytes_max <= bound,
                "{case} peak allocation {} exceeded source-derived bound {bound}",
                allocations.bytes_max
            );
        }
        let peak_bound = source_peak_bound_bytes
            .map(|bound| bound.to_string())
            .unwrap_or_else(|| "none".to_owned());
        let ownership_boundary = match case {
            "loc_type_unpack" | "npc_type_unpack" | "flo_type_unpack" | "jag_member_read"
            | "pix8_mapscene_load" | "pix8_texture_load" => {
                "guarded_archive_open+cached_members+native_call+owner_drop"
            }
            "land_decode" => "borrowed_record+native_decode+output_drop",
            _ => "native_call+output_and_scratch_drop",
        };
        eprintln!(
            "native_map_qualification case={case} kind={kind} revision={revision} ownership_boundary={ownership_boundary} native_input_bytes={native_input_bytes} archive_bytes={archive_bytes} counts={counts} peak_allocation_bytes={} retained_after_drop_bytes={} elapsed_ns={elapsed_ns} format_bound_bytes={format_bound_bytes} source_peak_bound_bytes={peak_bound}",
            allocations.bytes_max,
            allocations.bytes_current,
        );
    }

    fn append_u24(bytes: &mut Vec<u8>, value: usize) {
        assert!(value <= U24_MAX);
        bytes.push((value >> 16) as u8);
        bytes.push((value >> 8) as u8);
        bytes.push(value as u8);
    }

    fn maximum_open_jag() -> Vec<u8> {
        let file_count = usize::from(u16::MAX);
        let payload_len = MAX_JAG_BYTES as usize - 6;
        let table_len = 2 + file_count * 10;
        let member_bytes = payload_len - table_len;
        let mut bytes = Vec::with_capacity(MAX_JAG_BYTES as usize);
        append_u24(&mut bytes, payload_len);
        append_u24(&mut bytes, payload_len);
        bytes.extend_from_slice(&u16::MAX.to_be_bytes());
        for index in 0..file_count {
            bytes.extend_from_slice(&(index as i32).to_be_bytes());
            let member_len = if index == 0 { member_bytes } else { 0 };
            append_u24(&mut bytes, member_len);
            append_u24(&mut bytes, member_len);
        }
        bytes.resize(MAX_JAG_BYTES as usize, 0x5a);
        assert_eq!(bytes.len(), MAX_JAG_BYTES as usize);
        bytes
    }

    fn maximum_config_jag() -> (Vec<u8>, usize, usize, usize, usize) {
        let count = usize::from(u16::MAX);
        let loc_dat = vec![0; count + 2];
        let mut loc_idx = Vec::with_capacity(2 + count * 2);
        loc_idx.extend_from_slice(&u16::MAX.to_be_bytes());
        for _ in 0..count {
            loc_idx.extend_from_slice(&1u16.to_be_bytes());
        }
        let npc_dat = vec![0; count + 2];
        let mut npc_idx = Vec::with_capacity(2 + count * 2);
        npc_idx.extend_from_slice(&u16::MAX.to_be_bytes());
        for _ in 0..count {
            npc_idx.extend_from_slice(&1u16.to_be_bytes());
        }
        let mut flo_dat = Vec::with_capacity(count + 2);
        flo_dat.extend_from_slice(&u16::MAX.to_be_bytes());
        flo_dat.resize(count + 2, 0);
        let used = loc_dat.len() + loc_idx.len() + npc_dat.len() + npc_idx.len() + flo_dat.len();
        assert!(used < U24_MAX);
        let padding = vec![0; U24_MAX - used];
        let members = [
            ("loc.dat", loc_dat.as_slice()),
            ("loc.idx", loc_idx.as_slice()),
            ("npc.dat", npc_dat.as_slice()),
            ("npc.idx", npc_idx.as_slice()),
            ("flo.dat", flo_dat.as_slice()),
            ("padding", padding.as_slice()),
        ];
        let archive = synthetic_jag(&members);
        let aggregate_member_bytes = used + padding.len();
        assert_eq!(aggregate_member_bytes, U24_MAX);
        (
            archive,
            loc_idx.len(),
            npc_idx.len(),
            flo_dat.len(),
            aggregate_member_bytes,
        )
    }

    fn type_unpack_bound<T>(count: usize, downstream_bytes: usize, extra_per_record: usize) -> u64 {
        let members = MAX_JAG_MEMBER_PEAK_BYTES;
        let replaceable_strings = 2u64 * U24_MAX as u64;
        let per_record = std::mem::size_of::<T>() + downstream_bytes + extra_per_record;
        members
            + replaceable_strings
            + u64::try_from(count.checked_mul(per_record).unwrap()).unwrap()
    }

    fn loc_type_extra_per_record() -> usize {
        let max_array_bytes =
            2 * (2 * 255 * std::mem::size_of::<i32>() + 2 * 255 * std::mem::size_of::<u16>());
        max_array_bytes + 16 * std::mem::size_of::<Option<String>>()
    }

    fn npc_type_extra_per_record() -> usize {
        let max_array_bytes =
            2 * (2 * 255 * std::mem::size_of::<i32>() + 2 * 255 * std::mem::size_of::<u16>());
        max_array_bytes + 16 * std::mem::size_of::<Option<String>>()
    }

    fn qualify_real_config(cache: &RealCache) {
        let count_bound = usize::from(u16::MAX);
        let archive_bytes = cache.archive_bytes("config");

        let config = cache.open_jag("config");
        let loc_idx = config.read("loc.idx").expect("real config loc.idx");
        let loc_dat = config.read("loc.dat").expect("real config loc.dat");
        let loc_count = usize::from(u16::from_be_bytes([loc_idx[0], loc_idx[1]]));
        let loc_input_bytes = loc_idx.len() + loc_dat.len();
        drop(config);
        let loc_bound = type_unpack_bound::<LocType>(
            count_bound,
            std::mem::size_of::<RasterLocDefinition>(),
            loc_type_extra_per_record(),
        );
        measure_case(
            "loc_type_unpack",
            "real",
            &format!("r{}", cache.revision),
            loc_input_bytes,
            archive_bytes,
            &format!(
                "definitions={loc_count};count_cap={count_bound};dat_bytes={};idx_bytes={}",
                loc_dat.len(),
                loc_idx.len()
            ),
            U24_MAX as u64,
            Some(loc_bound),
            || {
                let jag = cache.open_jag("config");
                let decoded = load_raster_locs(&jag).expect("real LocType unpack");
                assert_eq!(decoded.len(), loc_count);
                assert!(decoded
                    .iter()
                    .all(|definition| definition.width > 0 && definition.length > 0));
                drop(decoded);
                drop(jag);
            },
        );

        let config = cache.open_jag("config");
        let npc_idx = config.read("npc.idx").expect("real config npc.idx");
        let npc_dat = config.read("npc.dat").expect("real config npc.dat");
        let npc_count = usize::from(u16::from_be_bytes([npc_idx[0], npc_idx[1]]));
        let npc_input_bytes = npc_idx.len() + npc_dat.len();
        drop(config);
        let npc_bound = type_unpack_bound::<NpcType>(count_bound, 0, npc_type_extra_per_record());
        measure_case(
            "npc_type_unpack",
            "real",
            &format!("r{}", cache.revision),
            npc_input_bytes,
            archive_bytes,
            &format!(
                "definitions={npc_count};count_cap={count_bound};dat_bytes={};idx_bytes={}",
                npc_dat.len(),
                npc_idx.len()
            ),
            U24_MAX as u64,
            Some(npc_bound),
            || {
                let jag = cache.open_jag("config");
                let decoded = NpcType::unpack(&jag);
                assert_eq!(decoded.len(), npc_count);
                if let Some(last) = decoded.last() {
                    assert_eq!(last.id, i32::try_from(npc_count - 1).unwrap());
                }
                drop(decoded);
                drop(jag);
            },
        );

        let config = cache.open_jag("config");
        let flo_dat = config.read("flo.dat").expect("real config flo.dat");
        let flo_count = usize::from(u16::from_be_bytes([flo_dat[0], flo_dat[1]]));
        let flo_input_bytes = flo_dat.len();
        drop(config);
        let flo_bound =
            type_unpack_bound::<FloType>(count_bound, std::mem::size_of::<FloorDefinition>(), 0);
        measure_case(
            "flo_type_unpack",
            "real",
            &format!("r{}", cache.revision),
            flo_input_bytes,
            archive_bytes,
            &format!(
                "definitions={flo_count};count_cap={count_bound};dat_bytes={}",
                flo_dat.len()
            ),
            U24_MAX as u64,
            Some(flo_bound),
            || {
                let jag = cache.open_jag("config");
                let decoded = load_floor_definitions(&jag).expect("real FloType unpack");
                assert_eq!(decoded.len(), flo_count);
                drop(decoded);
                drop(jag);
            },
        );
    }

    fn qualify_max_config(
        path: &Path,
        archive_bytes: usize,
        loc_idx_bytes: usize,
        npc_idx_bytes: usize,
        flo_dat_bytes: usize,
        aggregate_bytes: usize,
    ) {
        let definition_count = usize::from(u16::MAX);
        let loc_dat_bytes = definition_count + 2;
        let loc_native_bytes = loc_dat_bytes + loc_idx_bytes;
        let bound = type_unpack_bound::<LocType>(
            definition_count,
            std::mem::size_of::<RasterLocDefinition>(),
            loc_type_extra_per_record(),
        );
        measure_case(
            "loc_type_unpack",
            "format_max",
            "synthetic",
            loc_native_bytes,
            archive_bytes,
            &format!(
                "definitions={definition_count};count_cap={definition_count};aggregate_member_bytes={aggregate_bytes};dat_bytes={loc_dat_bytes};idx_bytes={loc_idx_bytes}"
            ),
            U24_MAX as u64,
            Some(bound),
            || {
                let jag = read_jag(path, "max config").unwrap();
                assert_eq!(jag.file_count, 6);
                let decoded = load_raster_locs(&jag).expect("maximum LocType unpack");
                assert_eq!(decoded.len(), definition_count);
                assert!(decoded.iter().all(|definition| {
                    definition.width == 1 && definition.length == 1 && !definition.active
                }));
                drop(decoded);
                drop(jag);
            },
        );

        let npc_dat_bytes = definition_count + 2;
        let npc_native_bytes = npc_dat_bytes + npc_idx_bytes;
        let bound = type_unpack_bound::<NpcType>(definition_count, 0, npc_type_extra_per_record());
        measure_case(
            "npc_type_unpack",
            "format_max",
            "synthetic",
            npc_native_bytes,
            archive_bytes,
            &format!(
                "definitions={definition_count};count_cap={definition_count};aggregate_member_bytes={aggregate_bytes};dat_bytes={npc_dat_bytes};idx_bytes={npc_idx_bytes}"
            ),
            U24_MAX as u64,
            Some(bound),
            || {
                let jag = read_jag(path, "max config").unwrap();
                let decoded = NpcType::unpack(&jag);
                assert_eq!(decoded.len(), definition_count);
                assert_eq!(decoded.first().unwrap().id, 0);
                assert_eq!(
                    decoded.last().unwrap().id,
                    i32::try_from(definition_count - 1).unwrap()
                );
                drop(decoded);
                drop(jag);
            },
        );

        let bound = type_unpack_bound::<FloType>(
            definition_count,
            std::mem::size_of::<FloorDefinition>(),
            0,
        );
        measure_case(
            "flo_type_unpack",
            "format_max",
            "synthetic",
            flo_dat_bytes,
            archive_bytes,
            &format!(
                "definitions={definition_count};count_cap={definition_count};aggregate_member_bytes={aggregate_bytes};dat_bytes={flo_dat_bytes}"
            ),
            U24_MAX as u64,
            Some(bound),
            || {
                let jag = read_jag(path, "max config").unwrap();
                let decoded = load_floor_definitions(&jag).expect("maximum FloType unpack");
                assert_eq!(decoded.len(), definition_count);
                assert!(decoded
                    .iter()
                    .all(|floor| floor.colour == 0 && floor.texture.is_none()));
                drop(decoded);
                drop(jag);
            },
        );
    }

    struct MemberPick {
        archive: String,
        member: String,
        unpacked_bytes: usize,
    }

    fn largest_real_member(cache: &RealCache) -> MemberPick {
        let mut candidates = vec![
            ("config", "loc.dat".to_owned()),
            ("config", "loc.idx".to_owned()),
            ("config", "npc.dat".to_owned()),
            ("config", "npc.idx".to_owned()),
            ("config", "flo.dat".to_owned()),
            ("media", "mapscene.dat".to_owned()),
            ("media", "index.dat".to_owned()),
            ("versionlist", "map_index".to_owned()),
            ("textures", "index.dat".to_owned()),
        ];
        for id in 0..TEXTURE_COUNT {
            candidates.push(("textures", format!("{id}.dat")));
        }
        let mut largest: Option<MemberPick> = None;
        for archive in REAL_ARCHIVES {
            let jag = cache.open_jag(archive);
            for (candidate_archive, member) in
                candidates.iter().filter(|(name, _)| *name == archive)
            {
                if let Some(bytes) = jag.read(member) {
                    let unpacked_bytes = bytes.len();
                    drop(bytes);
                    if largest
                        .as_ref()
                        .is_none_or(|current| unpacked_bytes > current.unpacked_bytes)
                    {
                        largest = Some(MemberPick {
                            archive: (*candidate_archive).to_owned(),
                            member: member.clone(),
                            unpacked_bytes,
                        });
                    }
                }
            }
            drop(jag);
        }
        largest.unwrap_or_else(|| {
            panic!(
                "prepared revision {} has no relevant JAG member",
                cache.revision
            )
        })
    }

    fn qualify_real_jag_calls(cache: &RealCache) {
        let largest_archive = REAL_ARCHIVES
            .iter()
            .copied()
            .max_by_key(|archive| cache.archive_bytes(archive))
            .unwrap();
        let archive_bytes = cache.archive_bytes(largest_archive);
        let expected_count = cache.open_jag(largest_archive).file_count;
        let path = cache.path(largest_archive);
        measure_case(
            "guarded_jag_open",
            "real",
            &format!("r{}", cache.revision),
            archive_bytes,
            archive_bytes,
            &format!("archive={largest_archive};member_count={expected_count}"),
            MAX_JAG_BYTES,
            Some(MAX_JAG_OPEN_PEAK_BYTES),
            || {
                let jag = read_jag(&path, "real JAG qualification").expect("guarded real JAG open");
                assert_eq!(jag.file_count, expected_count);
                drop(jag);
            },
        );

        let picked = largest_real_member(cache);
        let archive_bytes = cache.archive_bytes(&picked.archive);
        let member = picked.member.clone();
        let expected_bytes = picked.unpacked_bytes;
        let archive_name = picked.archive.clone();
        let member_name = picked.member.clone();
        let counts = format!(
            "archive={archive_name};member={member_name};member_unpacked_bytes={expected_bytes}"
        );
        measure_case(
            "jag_member_read",
            "real",
            &format!("r{}", cache.revision),
            expected_bytes,
            archive_bytes,
            &counts,
            U24_MAX as u64,
            Some(MAX_JAG_MEMBER_PEAK_BYTES),
            || {
                let jag = cache.open_jag(&picked.archive);
                let bytes = jag.read(&member).expect("largest real JAG member read");
                assert_eq!(bytes.len(), expected_bytes);
                drop(bytes);
                drop(jag);
            },
        );
    }

    fn make_max_map_index() -> Vec<u8> {
        let mut bytes = Vec::with_capacity(MAX_MAP_INDEX_ROWS * MAP_INDEX_ROW_BYTES);
        for square in 0..MAX_MAP_INDEX_ROWS {
            let packed = square as u16;
            bytes.extend_from_slice(&packed.to_be_bytes());
            bytes.extend_from_slice(&packed.to_be_bytes());
            bytes.extend_from_slice(&u16::MAX.to_be_bytes());
            bytes.push((square & 1) as u8);
        }
        assert_eq!(bytes.len(), MAX_MAP_INDEX_ROWS * MAP_INDEX_ROW_BYTES);
        bytes
    }

    fn map_index_peak_bound() -> u64 {
        u64::try_from(
            MAX_MAP_INDEX_ROWS * MAP_INDEX_ROW_BYTES
                + MAX_MAP_INDEX_ROWS * std::mem::size_of::<MapIndexEntry>(),
        )
        .unwrap()
    }

    fn qualify_map_index(cache: &RealCache) {
        let jag = cache.open_jag("versionlist");
        let bytes = jag.read("map_index").expect("real map_index member");
        let archive_bytes = cache.archive_bytes("versionlist");
        let row_count = bytes.len() / MAP_INDEX_ROW_BYTES;
        assert!(row_count > 0);
        let native_input_bytes = bytes.len();
        let bound = map_index_peak_bound();
        measure_case(
            "map_index_decode",
            "real",
            &format!("r{}", cache.revision),
            native_input_bytes,
            archive_bytes,
            &format!(
                "rows={row_count};row_bytes={MAP_INDEX_ROW_BYTES};row_cap={MAX_MAP_INDEX_ROWS}"
            ),
            (MAX_MAP_INDEX_ROWS * MAP_INDEX_ROW_BYTES) as u64,
            Some(bound),
            || {
                let decoded = decode_map_index(&bytes).expect("real map-index decode");
                assert_eq!(decoded.len(), row_count);
                assert!(decoded
                    .windows(2)
                    .all(|pair| pair[0].packed_square() < pair[1].packed_square()));
                drop(decoded);
            },
        );
    }

    fn qualify_max_map_index() {
        let bytes = make_max_map_index();
        let count = MAX_MAP_INDEX_ROWS;
        measure_case(
            "map_index_decode",
            "format_max",
            "synthetic",
            bytes.len(),
            0,
            &format!("rows={count};row_bytes={MAP_INDEX_ROW_BYTES};row_cap={MAX_MAP_INDEX_ROWS}"),
            (MAX_MAP_INDEX_ROWS * MAP_INDEX_ROW_BYTES) as u64,
            Some(map_index_peak_bound()),
            || {
                let decoded = decode_map_index(&bytes).expect("maximum map-index decode");
                assert_eq!(decoded.len(), count);
                assert_eq!(decoded.first().unwrap().packed_square(), 0);
                assert_eq!(decoded.last().unwrap().packed_square(), u16::MAX);
                assert_eq!(decoded.last().unwrap().land_file, u16::MAX);
                drop(decoded);
            },
        );
    }

    struct RealMapRecord {
        entry: MapIndexEntry,
        bytes: Vec<u8>,
    }

    fn largest_real_record(cache: &RealCache, locations: bool) -> RealMapRecord {
        let versionlist = cache.open_jag("versionlist");
        let index_bytes = versionlist
            .read("map_index")
            .expect("real map_index member");
        drop(versionlist);
        let entries = decode_map_index(&index_bytes).expect("real map-index decode");
        let maps_path = cache.snapshot_dir.join("maps.bin");
        let mut maps = File::open(&maps_path).expect("prepared maps.bin");
        let index = crate::map::records::MapRecordIndex::read(&mut maps)
            .expect("real maps.bin record index");
        let selected = entries
            .iter()
            .filter_map(|entry| {
                let file_id = if locations {
                    entry.loc_file
                } else {
                    entry.land_file
                };
                if file_id == u16::MAX {
                    None
                } else {
                    index
                        .get(u32::from(file_id))
                        .map(|record| (*entry, file_id, record))
                }
            })
            .max_by_key(|(_, _, record)| record.bytes)
            .unwrap_or_else(|| {
                panic!(
                    "prepared revision {} has no listed {} map record",
                    cache.revision,
                    if locations { "location" } else { "land" }
                )
            });
        let (entry, file_id, record) = selected;
        let mut payload = Vec::new();
        let mut maps = File::open(&maps_path).expect("prepared maps.bin");
        assert!(index
            .read_record(&mut maps, u32::from(file_id), &mut payload)
            .expect("real map record read"));
        assert_eq!(payload.len(), record.bytes as usize);
        RealMapRecord {
            entry,
            bytes: payload,
        }
    }

    fn land_signature(cell: GroundCell) -> (i32, u8, u8, u8, u8, u8, u8) {
        (
            cell.height,
            cell.flags,
            cell.underlay,
            cell.overlay,
            cell.overlay_shape,
            cell.overlay_rotation,
            u8::from(cell.flags & 2 != 0),
        )
    }

    fn max_land_record() -> Vec<u8> {
        let minimum_cell_bytes = 2;
        let overlay_count = (MAX_MAP_RECORD_BYTES - LAND_CELL_COUNT * minimum_cell_bytes) / 2;
        let mut bytes = Vec::with_capacity(MAX_MAP_RECORD_BYTES);
        for _ in 0..overlay_count {
            bytes.extend_from_slice(&[2, 1]);
        }
        bytes.extend_from_slice(&[1, 0]);
        for _ in 1..LAND_CELL_COUNT {
            bytes.extend_from_slice(&[1, 0]);
        }
        assert_eq!(bytes.len(), MAX_MAP_RECORD_BYTES);
        bytes
    }

    fn qualify_land_record(cache: &RealCache, record: RealMapRecord) {
        assert!(record.bytes.len() <= MAX_MAP_RECORD_BYTES);
        let entry = record.entry;
        let input_bytes = record.bytes.len();
        let square_x = i32::from(entry.square_x);
        let square_z = i32::from(entry.square_z);

        let mut expected_reader = cache_reader(cache);
        let expected = expected_reader.read_land(entry).expect("real land decode");
        let first = land_signature(expected.cell(0, 0, 0));
        let last = land_signature(expected.cell(3, 63, 63));
        drop(expected);
        drop(expected_reader);

        let archive_bytes = usize::try_from(
            fs::metadata(cache.snapshot_dir.join("maps.bin"))
                .unwrap()
                .len(),
        )
        .unwrap();
        let output_bound =
            u64::try_from(LAND_CELL_COUNT * std::mem::size_of::<GroundCell>()).unwrap();
        measure_case(
            "land_decode",
            "real",
            &format!("r{}", cache.revision),
            input_bytes,
            archive_bytes,
            &format!(
                "record_bytes={input_bytes};cells={LAND_CELL_COUNT};planes={MAP_PLANES};square_cells={}",
                usize::from(MAP_SQUARE_SIZE) * usize::from(MAP_SQUARE_SIZE)
            ),
            MAX_MAP_RECORD_BYTES as u64,
            Some(MAX_MAP_RECORD_BYTES as u64 + output_bound),
            || {
                let decoded =
                    LandSquare::decode(square_x, square_z, &record.bytes).expect("real land decode");
                assert_eq!(decoded.square_x, square_x);
                assert_eq!(decoded.square_z, square_z);
                assert_eq!(decoded.cells_capacity_bytes(), output_bound as usize);
                assert_eq!(land_signature(decoded.cell(0, 0, 0)), first);
                assert_eq!(land_signature(decoded.cell(3, 63, 63)), last);
                drop(decoded);
            },
        );
    }

    fn qualify_max_land(fixture: &NativeFixture) {
        let bytes = max_land_record();
        let (mut expected_reader, entry) = maximum_land_reader(fixture, &bytes);
        let expected = expected_reader
            .read_land(entry)
            .expect("maximum land decode");
        let first = land_signature(expected.cell(0, 0, 0));
        let last = land_signature(expected.cell(3, 63, 63));
        drop(expected);
        drop(expected_reader);

        let input_bytes = bytes.len();
        let archive_bytes = usize::try_from(
            fs::metadata(fixture.0.join("max-land-snapshot/maps.bin"))
                .unwrap()
                .len(),
        )
        .unwrap();
        let output_bound =
            u64::try_from(LAND_CELL_COUNT * std::mem::size_of::<GroundCell>()).unwrap();
        measure_case(
            "land_decode",
            "format_max",
            "synthetic",
            input_bytes,
            archive_bytes,
            &format!(
                "record_bytes={input_bytes};record_cap={MAX_MAP_RECORD_BYTES};cells={LAND_CELL_COUNT};planes={MAP_PLANES};square_cells={}",
                usize::from(MAP_SQUARE_SIZE) * usize::from(MAP_SQUARE_SIZE)
            ),
            MAX_MAP_RECORD_BYTES as u64,
            Some(MAX_MAP_RECORD_BYTES as u64 + output_bound),
            || {
                let decoded = LandSquare::decode(
                    i32::from(entry.square_x),
                    i32::from(entry.square_z),
                    &bytes,
                )
                .expect("maximum land decode");
                assert_eq!(decoded.cells_capacity_bytes(), output_bound as usize);
                assert_eq!(land_signature(decoded.cell(0, 0, 0)), first);
                assert_eq!(land_signature(decoded.cell(3, 63, 63)), last);
                drop(decoded);
            },
        );
    }

    fn visit_location_stats(bytes: &[u8]) -> (usize, Option<LocPlacement>, Option<LocPlacement>) {
        let mut count = 0usize;
        let mut first = None;
        let mut last = None;
        visit_locations(bytes, |placement| {
            if count == 0 {
                first = Some(placement);
            }
            count += 1;
            last = Some(placement);
        })
        .expect("location stream decode");
        (count, first, last)
    }

    fn max_location_record() -> Vec<u8> {
        let placement_count = (MAX_MAP_RECORD_BYTES - 4) / 2;
        let mut bytes = Vec::with_capacity(MAX_MAP_RECORD_BYTES);
        // Delta 128 encodes as two bytes and yields loc ID 127 from the initial -1.
        bytes.extend_from_slice(&[0x80, 0x80]);
        for _ in 0..placement_count {
            bytes.extend_from_slice(&[1, 0]);
        }
        bytes.extend_from_slice(&[0, 0]);
        assert_eq!(bytes.len(), MAX_MAP_RECORD_BYTES);
        assert_eq!(placement_count, 131_070);
        bytes
    }

    fn qualify_location_record(cache: &RealCache, record: RealMapRecord) {
        assert!(record.bytes.len() <= MAX_MAP_RECORD_BYTES);
        let (expected_count, expected_first, expected_last) = visit_location_stats(&record.bytes);
        let input_bytes = record.bytes.len();
        measure_case(
            "location_decode",
            "real",
            &format!("r{}", cache.revision),
            input_bytes,
            input_bytes,
            &format!(
                "record_bytes={input_bytes};placements={expected_count};placement_cap={MAX_MAP_LOC_PLACEMENTS_PER_RECORD}"
            ),
            MAX_MAP_RECORD_BYTES as u64,
            Some(0),
            || {
                let (count, first, last) = visit_location_stats(&record.bytes);
                assert_eq!(count, expected_count);
                assert_eq!(first, expected_first);
                assert_eq!(last, expected_last);
            },
        );
    }

    fn qualify_max_locations() {
        let bytes = max_location_record();
        let expected = LocPlacement {
            id: 127,
            plane: 0,
            x: 0,
            z: 0,
            shape: 0,
            rotation: 0,
        };
        let placement_count = 131_070;
        measure_case(
            "location_decode",
            "format_max",
            "synthetic",
            bytes.len(),
            0,
            &format!(
                "record_bytes={};record_cap={MAX_MAP_RECORD_BYTES};placements={placement_count};placement_cap={MAX_MAP_LOC_PLACEMENTS_PER_RECORD};one_object=true",
                bytes.len()
            ),
            MAX_MAP_RECORD_BYTES as u64,
            Some(0),
            || {
                let (count, first, last) = visit_location_stats(&bytes);
                assert_eq!(count, placement_count);
                assert_eq!(first, Some(expected));
                assert_eq!(last, Some(expected));
            },
        );
    }

    #[derive(Clone, Copy)]
    struct SpritePick {
        id: usize,
        width: i32,
        height: i32,
        pixels: usize,
        aggregate_pixels: usize,
        sprite_count: usize,
        first_pixel: i8,
        last_pixel: i8,
    }

    fn largest_real_mapscene(cache: &RealCache) -> SpritePick {
        let jag = cache.open_jag("media");
        let mut largest = None;
        let mut aggregate_pixels = 0usize;
        let mut sprite_count = 0usize;
        for id in 0..MAPSCENE_COUNT {
            if let Some(sprite) = depack_mapscene(&jag, id).expect("real mapscene depack") {
                let pixels = sprite.data.len();
                aggregate_pixels = aggregate_pixels.checked_add(pixels).unwrap();
                sprite_count += 1;
                if largest
                    .as_ref()
                    .is_none_or(|current: &SpritePick| pixels > current.pixels)
                {
                    largest = Some(SpritePick {
                        id,
                        width: sprite.wi,
                        height: sprite.hi,
                        pixels,
                        aggregate_pixels: 0,
                        sprite_count: 0,
                        first_pixel: sprite.data.first().copied().unwrap_or(0),
                        last_pixel: sprite.data.last().copied().unwrap_or(0),
                    });
                }
            }
        }
        drop(jag);
        let mut largest = largest.expect("prepared media has real mapscene pixels");
        largest.aggregate_pixels = aggregate_pixels;
        largest.sprite_count = sprite_count;
        largest
    }

    fn largest_real_texture(cache: &RealCache) -> SpritePick {
        let jag = cache.open_jag("textures");
        let mut largest = None;
        let mut aggregate_pixels = 0usize;
        let mut sprite_count = 0usize;
        for id in 0..TEXTURE_COUNT {
            if let Some(sprite) = depack_texture(&jag, id).expect("real texture depack") {
                let pixels = sprite.data.len();
                aggregate_pixels = aggregate_pixels.checked_add(pixels).unwrap();
                sprite_count += 1;
                if largest
                    .as_ref()
                    .is_none_or(|current: &SpritePick| pixels > current.pixels)
                {
                    largest = Some(SpritePick {
                        id,
                        width: sprite.wi,
                        height: sprite.hi,
                        pixels,
                        aggregate_pixels: 0,
                        sprite_count: 0,
                        first_pixel: sprite.data.first().copied().unwrap_or(0),
                        last_pixel: sprite.data.last().copied().unwrap_or(0),
                    });
                }
            }
        }
        drop(jag);
        let mut largest = largest.expect("prepared textures have real texture pixels");
        largest.aggregate_pixels = aggregate_pixels;
        largest.sprite_count = sprite_count;
        largest
    }

    fn palette_index_header(
        index: &mut Vec<u8>,
        original_width: usize,
        original_height: usize,
        width: usize,
        height: usize,
    ) {
        index.extend_from_slice(&u16::try_from(original_width).unwrap().to_be_bytes());
        index.extend_from_slice(&u16::try_from(original_height).unwrap().to_be_bytes());
        index.push(2);
        index.extend_from_slice(&[0x12, 0x34, 0x56]);
        index.extend_from_slice(&[0, 0]);
        index.extend_from_slice(&u16::try_from(width).unwrap().to_be_bytes());
        index.extend_from_slice(&u16::try_from(height).unwrap().to_be_bytes());
        index.push(0);
    }

    fn maximum_mapscene_jag() -> (Vec<u8>, usize, usize, usize) {
        let mut dimensions = vec![(0usize, 0usize); MAPSCENE_COUNT];
        dimensions[0] = (65_534, 256);
        dimensions[1] = (151, 1);
        let aggregate_pixels: usize = dimensions
            .iter()
            .map(|(width, height)| width * height)
            .sum();
        let mut index = Vec::with_capacity(8 + MAPSCENE_COUNT * 7);
        index.extend_from_slice(&u16::try_from(dimensions[0].0).unwrap().to_be_bytes());
        index.extend_from_slice(&u16::try_from(dimensions[0].1).unwrap().to_be_bytes());
        index.push(2);
        index.extend_from_slice(&[0x12, 0x34, 0x56]);
        for (width, height) in &dimensions {
            index.extend_from_slice(&[0, 0]);
            index.extend_from_slice(&u16::try_from(*width).unwrap().to_be_bytes());
            index.extend_from_slice(&u16::try_from(*height).unwrap().to_be_bytes());
            index.push(0);
        }
        let mut dat = Vec::with_capacity(2 + aggregate_pixels);
        dat.extend_from_slice(&0u16.to_be_bytes());
        dat.resize(2 + aggregate_pixels, 1);
        let member_bytes = dat.len() + index.len();
        assert_eq!(member_bytes, U24_MAX);
        let archive = synthetic_jag(&[("mapscene.dat", &dat), ("index.dat", &index)]);
        (archive, aggregate_pixels, dat.len(), index.len())
    }

    fn maximum_texture_jag() -> (Vec<u8>, usize, usize, usize) {
        let mut dimensions = vec![(0usize, 0usize); TEXTURE_COUNT];
        dimensions[0] = (65_535, 255);
        dimensions[1] = (64_940, 1);
        let aggregate_pixels: usize = dimensions
            .iter()
            .map(|(width, height)| width * height)
            .sum();
        let mut index = Vec::with_capacity(TEXTURE_COUNT * 15);
        let mut members = Vec::with_capacity(TEXTURE_COUNT + 1);
        for (id, (width, height)) in dimensions.iter().copied().enumerate() {
            let offset = u16::try_from(index.len()).unwrap();
            let mut dat = Vec::with_capacity(2 + width * height);
            dat.extend_from_slice(&offset.to_be_bytes());
            dat.resize(2 + width * height, 1);
            members.push((format!("{id}.dat"), dat));
            palette_index_header(&mut index, width, height, width, height);
        }
        let index_bytes = index.len();
        members.push(("index.dat".to_owned(), index));
        let aggregate_member_bytes: usize = members.iter().map(|(_, bytes)| bytes.len()).sum();
        assert_eq!(aggregate_member_bytes, U24_MAX);
        let refs: Vec<(&str, &[u8])> = members
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        let archive = synthetic_jag(&refs);
        (
            archive,
            aggregate_pixels,
            aggregate_member_bytes,
            index_bytes,
        )
    }

    fn sprite_peak_bound() -> u64 {
        MAX_JAG_MEMBER_PEAK_BYTES + MAX_JAG_UNPACKED_BYTES + 255 * std::mem::size_of::<i32>() as u64
    }

    fn qualify_real_sprites(cache: &RealCache) {
        let mapscene = largest_real_mapscene(cache);
        let mapscene_dat_bytes = cache.member_len("media", "mapscene.dat");
        let mapscene_index_bytes = cache.member_len("media", "index.dat");
        let media_bytes = cache.archive_bytes("media");
        measure_case(
            "pix8_mapscene_load",
            "real",
            &format!("r{}", cache.revision),
            mapscene_dat_bytes + mapscene_index_bytes,
            media_bytes,
            &format!(
                "sprite_id={};sprite_pixels={};aggregate_sprite_pixels={};sprites={};dat_bytes={mapscene_dat_bytes};index_bytes={mapscene_index_bytes}",
                mapscene.id, mapscene.pixels, mapscene.aggregate_pixels, mapscene.sprite_count
            ),
            U24_MAX as u64,
            Some(sprite_peak_bound()),
            || {
                let jag = cache.open_jag("media");
                let decoded = depack_mapscene(&jag, mapscene.id)
                    .expect("real mapscene load")
                    .expect("selected real mapscene");
                assert_eq!(decoded.wi, mapscene.width);
                assert_eq!(decoded.hi, mapscene.height);
                assert_eq!(decoded.data.len(), mapscene.pixels);
                assert_eq!(
                    decoded.data.first().copied().unwrap_or(0),
                    mapscene.first_pixel
                );
                assert_eq!(
                    decoded.data.last().copied().unwrap_or(0),
                    mapscene.last_pixel
                );
                drop(decoded);
                drop(jag);
            },
        );

        let texture = largest_real_texture(cache);
        let texture_dat_bytes = cache.member_len("textures", &format!("{}.dat", texture.id));
        let texture_index_bytes = cache.member_len("textures", "index.dat");
        let textures_bytes = cache.archive_bytes("textures");
        measure_case(
            "pix8_texture_load",
            "real",
            &format!("r{}", cache.revision),
            texture_dat_bytes + texture_index_bytes,
            textures_bytes,
            &format!(
                "texture_id={};sprite_pixels={};aggregate_sprite_pixels={};sprites={};dat_bytes={texture_dat_bytes};index_bytes={texture_index_bytes}",
                texture.id, texture.pixels, texture.aggregate_pixels, texture.sprite_count
            ),
            U24_MAX as u64,
            Some(sprite_peak_bound()),
            || {
                let jag = cache.open_jag("textures");
                let decoded = depack_texture(&jag, texture.id)
                    .expect("real texture load")
                    .expect("selected real texture");
                assert_eq!(decoded.wi, texture.width);
                assert_eq!(decoded.hi, texture.height);
                assert_eq!(decoded.data.len(), texture.pixels);
                assert_eq!(
                    decoded.data.first().copied().unwrap_or(0),
                    texture.first_pixel
                );
                assert_eq!(
                    decoded.data.last().copied().unwrap_or(0),
                    texture.last_pixel
                );
                drop(decoded);
                drop(jag);
            },
        );
    }

    fn qualify_max_sprites(fixture: &NativeFixture) {
        let (media, mapscene_pixels, mapscene_dat_bytes, mapscene_index_bytes) =
            maximum_mapscene_jag();
        let media_path = fixture.write("max-media.jag", &media);
        let media_archive_bytes = media.len();
        drop(media);
        let sprite_bound = sprite_peak_bound();
        measure_case(
            "pix8_mapscene_load",
            "format_max",
            "synthetic",
            mapscene_dat_bytes + mapscene_index_bytes,
            media_archive_bytes,
            &format!(
                "sprite_id=0;sprite_pixels={};aggregate_sprite_pixels={mapscene_pixels};sprites={MAPSCENE_COUNT};dat_bytes={mapscene_dat_bytes};index_bytes={mapscene_index_bytes}",
                65_534usize * 256
            ),
            U24_MAX as u64,
            Some(sprite_bound),
            || {
                let jag = read_jag(&media_path, "max media").unwrap();
                assert_eq!(jag.file_count, 2);
                let decoded = depack_mapscene(&jag, 0)
                    .expect("maximum mapscene load")
                    .expect("maximum mapscene present");
                assert_eq!((decoded.wi, decoded.hi), (65_534, 256));
                assert_eq!(decoded.data.len(), 65_534usize * 256);
                assert_eq!(decoded.data.first(), Some(&1));
                assert_eq!(decoded.data.last(), Some(&1));
                assert_eq!(decoded.bpal.get(1), Some(&0x123456));
                drop(decoded);
                drop(jag);
            },
        );

        let (textures, texture_pixels, texture_member_bytes, index_bytes) = maximum_texture_jag();
        let textures_path = fixture.write("max-textures.jag", &textures);
        let textures_archive_bytes = textures.len();
        drop(textures);
        let max_texture_input_bytes = 2 + 65_535usize * 255 + index_bytes;
        measure_case(
            "pix8_texture_load",
            "format_max",
            "synthetic",
            max_texture_input_bytes,
            textures_archive_bytes,
            &format!(
                "texture_id=0;sprite_pixels={};aggregate_sprite_pixels={texture_pixels};textures={TEXTURE_COUNT};aggregate_member_bytes={texture_member_bytes};index_bytes={index_bytes}",
                65_535usize * 255
            ),
            U24_MAX as u64,
            Some(sprite_bound),
            || {
                let jag = read_jag(&textures_path, "max textures").unwrap();
                assert_eq!(jag.file_count, TEXTURE_COUNT as i32 + 1);
                let decoded = depack_texture(&jag, 0)
                    .expect("maximum texture load")
                    .expect("maximum texture present");
                assert_eq!((decoded.wi, decoded.hi), (65_535, 255));
                assert_eq!(decoded.data.len(), 65_535usize * 255);
                assert_eq!(decoded.data.first(), Some(&1));
                assert_eq!(decoded.data.last(), Some(&1));
                assert_eq!(decoded.bpal.get(1), Some(&0x123456));
                drop(decoded);
                drop(jag);
            },
        );
    }

    fn cache_reader_from_input(input: ClientMapInput<'_>) -> CacheReader {
        let mut cursor = CacheReaderCursor::new(input);
        for _ in 0..=MAX_MAP_RECORDS + 8 {
            if let Some(reader) = cursor.advance().expect("cache-reader setup step") {
                return reader;
            }
        }
        panic!("cache reader exceeded the source record-count bound");
    }

    fn cache_reader(cache: &RealCache) -> CacheReader {
        cache_reader_from_input(cache.input())
    }

    fn maximum_land_reader(fixture: &NativeFixture, bytes: &[u8]) -> (CacheReader, MapIndexEntry) {
        assert!(bytes.len() <= MAX_MAP_RECORD_BYTES);
        let jag_dir = fixture.0.join("max-land-jags");
        let snapshot_dir = fixture.0.join("max-land-snapshot");
        fs::create_dir_all(&jag_dir).unwrap();
        fs::create_dir_all(&snapshot_dir).unwrap();

        let mut map_index = Vec::with_capacity(MAP_INDEX_ROW_BYTES);
        map_index.extend_from_slice(&0u16.to_be_bytes());
        map_index.extend_from_slice(&0u16.to_be_bytes());
        map_index.extend_from_slice(&u16::MAX.to_be_bytes());
        map_index.push(0);
        let versionlist = synthetic_jag(&[("map_index", &map_index)]);
        fs::write(jag_dir.join("versionlist"), versionlist).unwrap();

        let mut maps = Vec::with_capacity(8 + bytes.len());
        maps.extend_from_slice(&0u32.to_le_bytes());
        maps.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_le_bytes());
        maps.extend_from_slice(bytes);
        fs::write(snapshot_dir.join("maps.bin"), maps).unwrap();

        let input = ClientMapInput::new(274, Digest([0; 32]), &jag_dir, &snapshot_dir).unwrap();
        let reader = cache_reader_from_input(input);
        assert_eq!(reader.entries.len(), 1);
        assert_eq!(reader.entries[0].land_file, 0);
        assert_eq!(reader.entries[0].loc_file, u16::MAX);
        let entry = reader.entries[0];
        (reader, entry)
    }

    fn real_tile_rgba(cache: &RealCache) -> (Vec<u8>, u16) {
        let config = cache.open_jag("config");
        let definitions = Definitions {
            locs: load_raster_locs(&config).expect("real loc definitions for PNG input"),
            floors: load_floor_definitions(&config).expect("real floor definitions for PNG input"),
        };
        drop(config);

        let media = cache.open_jag("media");
        let mut mapscenes = Vec::with_capacity(MAPSCENE_COUNT);
        for id in 0..MAPSCENE_COUNT {
            mapscenes.push(depack_mapscene(&media, id).expect("real mapscene for PNG input"));
        }
        drop(media);
        let textures = cache.open_jag("textures");
        let mut texture_colours = [0; TEXTURE_COUNT];
        for (id, colour) in texture_colours.iter_mut().enumerate() {
            if let Some(texture) =
                depack_texture(&textures, id).expect("real texture for PNG input")
            {
                *colour = average_texture(&texture);
            }
        }
        drop(textures);
        let assets = RasterAssets {
            mapscenes,
            texture_colours,
        };

        let mut reader = cache_reader(cache);
        let entry = reader
            .entries
            .iter()
            .copied()
            .find(|entry| entry.land_file != u16::MAX)
            .expect("real map index has a listed land square");
        let land = reader.read_land(entry).expect("real land for PNG input");
        let locs = reader
            .read_locs(entry)
            .expect("real locations for PNG input");
        let key = TileKey {
            plane: 0,
            lod: 0,
            x: i32::from(entry.square_x),
            z: i32::from(entry.square_z),
        };
        let neighborhood = Neighborhood {
            squares: std::collections::BTreeMap::from([(
                (i32::from(entry.square_x), i32::from(entry.square_z)),
                SquareData { land, locs },
            )]),
        };
        let context = RasterContext::new(definitions, assets, reader);
        let (rgba, _) = render_base(&context, &neighborhood, key)
            .expect("real client square renders one fixed-size tile");
        assert_eq!(rgba.len(), TILE_RGBA_BYTES);
        (rgba, entry.packed_square())
    }

    fn random_tile_rgba() -> Vec<u8> {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut rgba = vec![0; TILE_RGBA_BYTES];
        for pixel in &mut rgba {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *pixel = (state >> 24) as u8;
        }
        rgba
    }

    fn crc32_update(mut crc: u32, bytes: &[u8]) -> u32 {
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        crc
    }

    fn pad_png_to_limit(png: &[u8]) -> Vec<u8> {
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let target_len = MAX_PNG_BYTES as usize;
        assert!(png.len() + 12 <= target_len);
        let padding_len = target_len - png.len() - 12;
        let chunk_type = *b"raNd";
        let mut crc = crc32_update(u32::MAX, &chunk_type);
        let padding = [0x5a; 256];
        let mut remaining = padding_len;
        while remaining > 0 {
            let count = remaining.min(padding.len());
            crc = crc32_update(crc, &padding[..count]);
            remaining -= count;
        }
        let crc = !crc;
        let mut output = Vec::with_capacity(target_len);
        output.extend_from_slice(&png[..8]);
        let mut offset = 8usize;
        let mut inserted = false;
        while offset < png.len() {
            let length = u32::from_be_bytes(png[offset..offset + 4].try_into().unwrap()) as usize;
            let end = offset + 12 + length;
            assert!(end <= png.len());
            output.extend_from_slice(&png[offset..end]);
            if &png[offset + 4..offset + 8] == b"IHDR" {
                output.extend_from_slice(&(padding_len as u32).to_be_bytes());
                output.extend_from_slice(&chunk_type);
                output.resize(output.len() + padding_len, 0x5a);
                output.extend_from_slice(&crc.to_be_bytes());
                inserted = true;
            }
            offset = end;
        }
        assert!(inserted);
        assert_eq!(output.len(), target_len);
        output
    }

    fn qualify_png(cache: &RealCache, rgba: &[u8], square: u16) {
        let encoded_for_decode = encode_png(rgba).expect("real tile PNG encode for decode input");
        let png_bytes = encoded_for_decode.len();
        assert!(png_bytes <= MAX_PNG_BYTES as usize);
        measure_case(
            "png_encode",
            "real",
            &format!("r{}", cache.revision),
            rgba.len(),
            0,
            &format!(
                "width={TILE_PIXELS};height={TILE_PIXELS};rgba_bytes={};square={square}",
                rgba.len()
            ),
            TILE_RGBA_BYTES as u64,
            None,
            || {
                let encoded = encode_png(rgba).expect("real tile PNG encode");
                assert!(encoded.len() <= MAX_PNG_BYTES as usize);
                assert!(encoded.starts_with(b"\x89PNG\r\n\x1a\n"));
                drop(encoded);
            },
        );
        measure_case(
            "png_decode",
            "real",
            &format!("r{}", cache.revision),
            png_bytes,
            0,
            &format!(
                "width={TILE_PIXELS};height={TILE_PIXELS};rgba_bytes={};png_bytes={png_bytes};square={square}",
                rgba.len()
            ),
            MAX_PNG_BYTES as u64,
            Some(
                PNG_DECODER_DEFAULT_LIMIT_BYTES
                    + TILE_RGBA_BYTES as u64
                    + PNG_DECODER_INITIAL_DATA_BYTES
                    + PNG_DECODER_CHUNK_BUFFER_BYTES,
            ),
            || {
                let decoded = decode_png(&encoded_for_decode).expect("real tile PNG decode");
                assert_eq!(decoded, rgba);
                drop(decoded);
            },
        );
    }

    fn qualify_max_png() {
        let rgba = random_tile_rgba();
        measure_case(
            "png_encode",
            "format_max",
            "synthetic",
            rgba.len(),
            0,
            &format!(
                "width={TILE_PIXELS};height={TILE_PIXELS};rgba_bytes={}",
                rgba.len()
            ),
            TILE_RGBA_BYTES as u64,
            None,
            || {
                let encoded = encode_png(&rgba).expect("maximum tile PNG encode");
                assert!(encoded.len() <= MAX_PNG_BYTES as usize);
                assert!(encoded.starts_with(b"\x89PNG\r\n\x1a\n"));
                drop(encoded);
            },
        );
        let encoded = encode_png(&rgba).expect("maximum tile PNG encode for decode input");
        let padded = pad_png_to_limit(&encoded);
        drop(encoded);
        assert_eq!(padded.len(), MAX_PNG_BYTES as usize);
        measure_case(
            "png_decode",
            "format_max",
            "synthetic",
            padded.len(),
            0,
            &format!(
                "width={TILE_PIXELS};height={TILE_PIXELS};rgba_bytes={};png_bytes={};png_cap={MAX_PNG_BYTES}",
                rgba.len(),
                padded.len()
            ),
            MAX_PNG_BYTES as u64,
            Some(
                PNG_DECODER_DEFAULT_LIMIT_BYTES
                    + TILE_RGBA_BYTES as u64
                    + PNG_DECODER_INITIAL_DATA_BYTES
                    + PNG_DECODER_CHUNK_BUFFER_BYTES,
            ),
            || {
                let decoded = decode_png(&padded).expect("maximum-size valid PNG decode");
                assert_eq!(decoded, rgba);
                drop(decoded);
            },
        );
    }

    #[test]
    #[ignore = "release-only native-unit qualification; requires matching prepared 274/289 cache inputs"]
    #[expect(
        clippy::assertions_on_constants,
        reason = "release-only qualification must reject debug builds"
    )]
    fn native_map_producer_release_qualification() {
        assert!(
            !cfg!(debug_assertions),
            "native map producer allocation qualification requires --release"
        );
        let real = [RealCache::new(274), RealCache::new(289)];
        let fixture = NativeFixture::new();

        let max_open_path = fixture.write("max-open.jag", &maximum_open_jag());
        let max_open_bytes = usize::try_from(fs::metadata(&max_open_path).unwrap().len()).unwrap();
        measure_case(
            "guarded_jag_open",
            "format_max",
            "synthetic",
            max_open_bytes,
            max_open_bytes,
            &format!(
                "archive_file_bytes={max_open_bytes};member_count={}",
                u16::MAX
            ),
            MAX_JAG_BYTES,
            Some(MAX_JAG_OPEN_PEAK_BYTES),
            || {
                let jag =
                    read_jag(&max_open_path, "maximum JAG").expect("maximum guarded JAG open");
                assert_eq!(jag.file_count, i32::from(u16::MAX));
                drop(jag);
            },
        );

        let member_data = vec![0x5a; U24_MAX];
        let max_member_archive = synthetic_jag(&[("payload", &member_data)]);
        let max_member_path = fixture.write("max-member.jag", &max_member_archive);
        let max_member_archive_bytes = max_member_archive.len();
        drop(max_member_archive);
        drop(member_data);
        let member = "payload".to_owned();
        let packed_member_bytes = max_member_archive_bytes - 18;
        measure_case(
            "jag_member_read",
            "format_max",
            "synthetic",
            U24_MAX,
            max_member_archive_bytes,
            &format!(
                "members=1;member_unpacked_bytes={U24_MAX};member_packed_bytes={packed_member_bytes}"
            ),
            U24_MAX as u64,
            Some(MAX_JAG_MEMBER_PEAK_BYTES),
            || {
                let jag = read_jag(&max_member_path, "maximum member").unwrap();
                let bytes = jag.read(&member).expect("maximum valid JAG member read");
                assert_eq!(bytes.len(), U24_MAX);
                assert!(bytes.iter().all(|byte| *byte == 0x5a));
                drop(bytes);
                drop(jag);
            },
        );

        let (max_config, loc_idx_bytes, npc_idx_bytes, flo_dat_bytes, aggregate_member_bytes) =
            maximum_config_jag();
        let max_config_path = fixture.write("max-config.jag", &max_config);
        let max_config_archive_bytes = max_config.len();
        drop(max_config);

        for cache in &real {
            qualify_real_config(cache);
            qualify_real_jag_calls(cache);
            qualify_map_index(cache);
            let land = largest_real_record(cache, false);
            qualify_land_record(cache, land);
            let locations = largest_real_record(cache, true);
            qualify_location_record(cache, locations);
            qualify_real_sprites(cache);
            let (rgba, square) = real_tile_rgba(cache);
            qualify_png(cache, &rgba, square);
        }

        qualify_max_config(
            &max_config_path,
            max_config_archive_bytes,
            loc_idx_bytes,
            npc_idx_bytes,
            flo_dat_bytes,
            aggregate_member_bytes,
        );
        qualify_max_map_index();
        qualify_max_land(&fixture);
        qualify_max_locations();
        qualify_max_sprites(&fixture);
        qualify_max_png();
    }
}
