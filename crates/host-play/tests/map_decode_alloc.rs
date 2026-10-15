use std::cell::Cell;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use api::identity::Deadline;
use api::work::{
    CooperativeJob, MonotonicClock, SharedCancellation, SliceBudget, StepContext, StepOutcome,
    WorkCharge, WalkMapDecodePhase,
};
use host_play::map_bind::{
    WalkMapDecodeFailure, WalkMapDecodeJob, WALK_MAP_DECODE_PNG_LIMIT,
    WALK_MAP_DECODE_SCRATCH_LIMIT,
};
use nav::map::formats::ImageManifest;
use nav::map::spatial::{TILE_PIXELS, TILE_RGBA_BYTES};
use serde::Deserialize;

#[derive(Deserialize)]
struct DeclaredIdentity {
    identity: nav::map::identity::ImageIdentity,
}

struct YieldAfterDecode {
    start: Instant,
    reads: Cell<usize>,
}

impl MonotonicClock for YieldAfterDecode {
    fn now(&self) -> Instant {
        let read = self.reads.get();
        self.reads.set(read + 1);
        if read < 2 {
            self.start
        } else {
            self.start + Duration::from_secs(120)
        }
    }
}

struct FixedClock(Instant);

impl MonotonicClock for FixedClock {
    fn now(&self) -> Instant {
        self.0
    }
}

fn slice_budget(start: Instant) -> SliceBudget {
    SliceBudget::new(
        NonZeroU32::new(64).expect("nonzero work quantum"),
        Deadline::at(start + Duration::from_secs(60)),
    )
}

fn declared_charge(png_bytes: usize) -> WorkCharge {
    WorkCharge::new(
        png_bytes
            .checked_add(std::mem::size_of::<WalkMapDecodeJob>())
            .expect("bounded input charge"),
        WALK_MAP_DECODE_SCRATCH_LIMIT,
        TILE_RGBA_BYTES,
        0,
    )
}

fn maximum_declared_charge() -> WorkCharge {
    declared_charge(WALK_MAP_DECODE_PNG_LIMIT)
}

fn largest_published_tile() -> Vec<u8> {
    let root = std::path::PathBuf::from(
        std::env::var_os("BOT_WALK_MAP_DECODE_CORPUS")
            .expect("BOT_WALK_MAP_DECODE_CORPUS must name the published image artifact"),
    );
    let manifest_bytes =
        std::fs::read(root.join("manifest.json")).expect("read published image manifest");
    let declared: DeclaredIdentity =
        serde_json::from_slice(&manifest_bytes).expect("read declared image identity");
    let manifest =
        ImageManifest::decode(&manifest_bytes, declared.identity).expect("validate image manifest");
    let receipt = manifest
        .tiles
        .as_slice()
        .iter()
        .max_by_key(|tile| tile.payload.bytes)
        .expect("published image has terrain tiles");
    let path = root.join(receipt.key.relative_path().expect("validated tile path"));
    let png = std::fs::read(path).expect("read largest published terrain PNG");
    let verified_len = receipt
        .verify_png(&png)
        .expect("published PNG matches its manifest receipt");
    assert_eq!(verified_len, TILE_RGBA_BYTES);
    assert!(png.len() <= WALK_MAP_DECODE_PNG_LIMIT);
    png
}

/// Count the production cooperative job itself on this thread. The panel's
/// real-content differential fixture exercises the same job through the
/// shared WorkAdmission controller; allocation-counter is thread-local.
#[test]
#[ignore = "requires BOT_WALK_MAP_DECODE_CORPUS to point at a published image artifact"]
fn largest_producer_png_peak_and_release_fit_declared_charge() {
    let png = largest_published_tile();
    let png_bytes = png.len();
    let input: Arc<[u8]> = Arc::from(png);
    let keep_input_alive = Arc::clone(&input);
    let actual_charge = declared_charge(input.len());
    let maximum_charge = maximum_declared_charge();
    assert!(
        actual_charge
            .total_bytes()
            .expect("actual decoder charge is bounded")
            <= maximum_charge
                .total_bytes()
                .expect("maximum decoder charge is bounded"),
        "actual charge must be inside the maximum declared PNG charge"
    );

    let mut job = WalkMapDecodeJob::new(input);
    let start = Instant::now();
    let cancellation = SharedCancellation::default();
    let yield_clock = YieldAfterDecode {
        start,
        reads: Cell::new(0),
    };
    let fixed_clock = FixedClock(start);
    let mut first = StepContext::<WalkMapDecodePhase>::new(
        &yield_clock,
        slice_budget(start),
        &cancellation,
    );
    let mut second = StepContext::<WalkMapDecodePhase>::new(
        &fixed_clock,
        slice_budget(start),
        &cancellation,
    );
    let mut output_len = None;
    let allocations = allocation_counter::measure(|| {
        assert!(matches!(job.step(&mut first), StepOutcome::Yielded));
        assert_eq!(
            job.retained().bytes(),
            std::mem::size_of::<WalkMapDecodeJob>() + TILE_RGBA_BYTES,
            "a yielded decode retains exactly its result and job state"
        );
        match job.step(&mut second) {
            StepOutcome::Completed(rgba) => output_len = Some(rgba.len()),
            StepOutcome::Failed(error) => panic!("production PNG decode failed: {error}"),
            StepOutcome::Yielded => panic!("second bounded slice should publish the result"),
        }
        drop(job);
    });
    drop(keep_input_alive);

    assert_eq!(output_len, Some(TILE_RGBA_BYTES));
    assert_eq!(allocations.bytes_current, 0, "result and decoder scratch are released");
    assert_eq!(allocations.count_current, 0, "decoder allocations are released");
    let charged_live_peak = u64::try_from(
        actual_charge.scratch_bytes() + actual_charge.result_bytes(),
    )
    .expect("decoder live charge fits u64");
    assert!(
        allocations.bytes_max <= charged_live_peak,
        "production job peak {}B exceeds charged scratch + result {}B (total charge {:?})",
        allocations.bytes_max,
        charged_live_peak,
        actual_charge.total_bytes()
    );
    eprintln!(
        "largest producer PNG={}B, peak job allocations={}B, declared max charge={:?}",
        png_bytes,
        allocations.bytes_max,
        maximum_charge.total_bytes()
    );
}

fn encode_rgba_png() -> Vec<u8> {
    let pixels = vec![0u8; TILE_RGBA_BYTES];
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, TILE_PIXELS, TILE_PIXELS);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("write PNG header");
        writer.write_image_data(&pixels).expect("write PNG pixels");
    }
    png
}

fn encode_ztxt_bomb(text: String) -> Vec<u8> {
    let pixels = vec![0u8; TILE_RGBA_BYTES];
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, TILE_PIXELS, TILE_PIXELS);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .add_ztxt_chunk(String::from("profile"), text)
            .expect("encode compressed text chunk");
        let mut writer = encoder.write_header().expect("write PNG header");
        writer.write_image_data(&pixels).expect("write PNG pixels");
    }
    png
}

fn find_chunk_payload<'a>(png: &'a [u8], wanted: &[u8; 4]) -> &'a [u8] {
    let mut offset = 8usize;
    loop {
        let length = u32::from_be_bytes(
            png[offset..offset + 4]
                .try_into()
                .expect("PNG chunk length"),
        ) as usize;
        let end = offset + length + 12;
        if &png[offset + 4..offset + 8] == wanted {
            return &png[offset + 8..offset + 8 + length];
        }
        assert!(end <= png.len(), "requested chunk exists before IEND");
        offset = end;
    }
}

fn encode_iccp_bomb(ztxt_png: &[u8]) -> Vec<u8> {
    // zTXt and iCCP both carry keyword/name, NUL, compression method 0, and
    // a zlib stream; re-emit the same bounded-compression profile with a valid CRC.
    let profile = find_chunk_payload(ztxt_png, b"zTXt");
    let pixels = vec![0u8; TILE_RGBA_BYTES];
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, TILE_PIXELS, TILE_PIXELS);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("write PNG header");
        writer
            .write_chunk(png::chunk::iCCP, profile)
            .expect("write compressed ICC chunk");
        writer.write_image_data(&pixels).expect("write PNG pixels");
    }
    png
}

fn exif_description(size: usize) -> Vec<u8> {
    assert!(size >= 27);
    let mut tiff = vec![0u8; size];
    tiff[0..2].copy_from_slice(b"II");
    tiff[2..4].copy_from_slice(&42u16.to_le_bytes());
    tiff[4..8].copy_from_slice(&8u32.to_le_bytes());
    tiff[8..10].copy_from_slice(&1u16.to_le_bytes());
    tiff[10..12].copy_from_slice(&270u16.to_le_bytes());
    tiff[12..14].copy_from_slice(&2u16.to_le_bytes());
    tiff[14..18].copy_from_slice(&u32::try_from(size - 26).unwrap().to_le_bytes());
    tiff[18..22].copy_from_slice(&26u32.to_le_bytes());
    let text_end = size - 1;
    tiff[26..text_end].fill(b'x');
    tiff
}

fn encode_exif_png(exif: &[u8]) -> Vec<u8> {
    let pixels = vec![0u8; TILE_RGBA_BYTES];
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, TILE_PIXELS, TILE_PIXELS);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("write PNG header");
        writer
            .write_chunk(png::chunk::eXIf, exif)
            .expect("write bounded Exif chunk");
        writer.write_image_data(&pixels).expect("write PNG pixels");
    }
    png
}

fn expect_bounded_decode(png: Vec<u8>, expected_pixels: &[u8]) {
    assert!(png.len() <= WALK_MAP_DECODE_PNG_LIMIT);
    let input: Arc<[u8]> = Arc::from(png);
    let keep_input_alive = Arc::clone(&input);
    let actual_charge = declared_charge(input.len());
    assert!(
        actual_charge.total_bytes().unwrap() <= maximum_declared_charge().total_bytes().unwrap()
    );
    let mut job = WalkMapDecodeJob::new(input);
    let start = Instant::now();
    let clock = FixedClock(start);
    let cancellation = SharedCancellation::default();
    let mut context = StepContext::<WalkMapDecodePhase>::new(
        &clock,
        slice_budget(start),
        &cancellation,
    );
    let allocations = allocation_counter::measure(|| {
        match job.step(&mut context) {
            StepOutcome::Completed(rgba) => assert_eq!(rgba.as_slice(), expected_pixels),
            StepOutcome::Failed(error) => panic!("bounded metadata PNG failed: {error}"),
            StepOutcome::Yielded => panic!("bounded metadata PNG should finish in one slice"),
        }
        drop(job);
    });
    drop(keep_input_alive);

    assert_eq!(allocations.bytes_current, 0, "result and parser allocations are released");
    assert_eq!(allocations.count_current, 0);
    let charged_live_peak = u64::try_from(
        actual_charge.scratch_bytes() + actual_charge.result_bytes(),
    )
    .expect("metadata decode charge fits u64");
    assert!(
        allocations.bytes_max <= charged_live_peak,
        "metadata PNG peak {}B exceeds charged scratch + result {}B",
        allocations.bytes_max,
        charged_live_peak
    );
}

fn expect_rejected_without_decoder_allocations(
    png: Vec<u8>,
    expected: WalkMapDecodeFailure,
) {
    assert!(png.len() <= WALK_MAP_DECODE_PNG_LIMIT);
    let input: Arc<[u8]> = Arc::from(png);
    let keep_input_alive = Arc::clone(&input);
    let mut job = WalkMapDecodeJob::new(input);
    let start = Instant::now();
    let clock = FixedClock(start);
    let cancellation = SharedCancellation::default();
    let mut context = StepContext::<WalkMapDecodePhase>::new(
        &clock,
        slice_budget(start),
        &cancellation,
    );
    let mut error = None;
    let allocations = allocation_counter::measure(|| {
        match job.step(&mut context) {
            StepOutcome::Failed(actual) => error = Some(actual),
            StepOutcome::Completed(_) => panic!("non-producer PNG was accepted"),
            StepOutcome::Yielded => panic!("small preflight must settle in one slice"),
        }
        drop(job);
    });
    drop(keep_input_alive);
    assert_eq!(error, Some(expected));
    assert_eq!(allocations.bytes_total, 0, "preflight must precede PNG allocations");
    assert_eq!(allocations.bytes_current, 0);
}

#[test]
fn alternate_color_depth_and_interlace_modes_fail_before_conversion() {
    let producer_shape = encode_rgba_png();
    for (offset, value) in [(24usize, 16u8), (25, 3), (28, 1)] {
        let mut alternate = producer_shape.clone();
        alternate[offset] = value;
        expect_rejected_without_decoder_allocations(
            alternate,
            WalkMapDecodeFailure::DimensionsOrFormat,
        );
    }
}

#[test]
fn ancillary_metadata_preserves_pixels_with_bounded_allocations() {
    let expected_pixels = vec![0u8; TILE_RGBA_BYTES];
    let large_text = String::from("x").repeat(400 * 1024);
    let ztxt = encode_ztxt_bomb(large_text);
    let iccp = encode_iccp_bomb(&ztxt);
    let exif = encode_exif_png(&exif_description(250 * 1024));

    for png in [ztxt, iccp, exif] {
        expect_bounded_decode(png, &expected_pixels);
    }
}
