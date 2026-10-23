//! Bounded native client-cache decoding shared by the terrain and catalogue
//! producers. Records are read through a seek index, never retained as a world.

use super::poi::Footprint;
use super::producer::ClientMapInput;
use super::records::{MapRecordIndex, MapRecordIndexBuilder};
use super::MapError;
use client::config::{FloType, LocType};
use client::graphics::Pix8;
use client::io::JagFile;
use client::map_cache::{
    decode_map_index, terrain_height, texture_average, visit_land, visit_locations, LocPlacement,
    MapCacheError as ClientMapError, MapIndexEntry, MAP_PLANES, MAP_SQUARE_SIZE,
};
use std::fs::File;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

pub(crate) const MAX_JAG_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const MAX_JAG_UNPACKED_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_JAG_INDEX_TRACKED_BYTES: usize =
    u16::MAX as usize * (4 * std::mem::size_of::<i32>() + std::mem::size_of::<Option<Vec<u8>>>());
pub(crate) const MAX_JAG_ARCHIVE_RETAINED_BYTES: u64 =
    MAX_JAG_UNPACKED_BYTES + MAX_JAG_INDEX_TRACKED_BYTES as u64;
pub(crate) const MAX_JAG_READ_PEAK_BYTES: u64 =
    MAX_JAG_ARCHIVE_RETAINED_BYTES + 2 * MAX_JAG_UNPACKED_BYTES;
pub(crate) const MAX_JAG_MEMBER_PEAK_BYTES: u64 =
    MAX_JAG_ARCHIVE_RETAINED_BYTES + 4 * MAX_JAG_UNPACKED_BYTES;
pub(crate) const MAX_MAP_INDEX_ROWS: usize = 256 * 256;
pub(crate) const MAPSCENE_COUNT: usize = 50;
pub(crate) const TEXTURE_COUNT: usize = 50;
const CELL_COUNT: usize = MAP_PLANES as usize * MAP_SQUARE_SIZE as usize * MAP_SQUARE_SIZE as usize;

#[derive(Debug, Clone, Copy)]
pub(crate) struct RasterLocDefinition {
    pub width: u8,
    pub length: u8,
    pub active: bool,
    pub mapscene: Option<u16>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FloorDefinition {
    pub colour: u32,
    pub texture: Option<u8>,
    pub saturation: i32,
    pub lightness: i32,
    pub chroma: i32,
    pub underlay_hue: i32,
}

#[derive(Debug)]
pub(crate) struct Definitions {
    pub locs: Vec<RasterLocDefinition>,
    pub floors: Vec<FloorDefinition>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct GroundCell {
    pub height: i32,
    pub flags: u8,
    pub underlay: u8,
    pub overlay: u8,
    pub overlay_shape: u8,
    pub overlay_rotation: u8,
}

#[derive(Debug)]
pub(crate) struct LandSquare {
    pub square_x: i32,
    pub square_z: i32,
    cells: Vec<GroundCell>,
}

impl LandSquare {
    pub(super) fn decode(square_x: i32, square_z: i32, bytes: &[u8]) -> Result<Self, MapError> {
        let mut cells = vec![GroundCell::default(); CELL_COUNT];
        visit_land(bytes, |encoded| {
            let index = cell_index(encoded.plane, encoded.x, encoded.z);
            let previous = if encoded.plane == 0 {
                0
            } else {
                cells[cell_index(encoded.plane - 1, encoded.x, encoded.z)].height
            };
            let world_x = square_x * i32::from(MAP_SQUARE_SIZE) + i32::from(encoded.x);
            let world_z = square_z * i32::from(MAP_SQUARE_SIZE) + i32::from(encoded.z);
            let height = match encoded.explicit_height {
                Some(height) if encoded.plane == 0 => -i32::from(height) * 8,
                Some(height) => previous - i32::from(height) * 8,
                None if encoded.plane == 0 => terrain_height(world_x, world_z),
                None => previous - 240,
            };
            cells[index] = GroundCell {
                height,
                flags: encoded.flags,
                underlay: encoded.underlay,
                overlay: encoded.overlay,
                overlay_shape: encoded.overlay_shape,
                overlay_rotation: encoded.overlay_rotation,
            };
        })
        .map_err(map_client_error)?;
        Ok(Self {
            square_x,
            square_z,
            cells,
        })
    }

    pub(crate) fn cell(&self, plane: u8, x: u8, z: u8) -> GroundCell {
        self.cells[cell_index(plane, x, z)]
    }

    pub(crate) fn link_below(&self, x: u8, z: u8) -> bool {
        self.cell(1, x, z).flags & 0x2 != 0
    }
    pub(crate) fn cells_capacity_bytes(&self) -> usize {
        self.cells.capacity() * std::mem::size_of::<GroundCell>()
    }
}

#[derive(Clone)]
pub(crate) struct RasterAssets {
    pub mapscenes: Vec<Option<Pix8>>,
    pub texture_colours: [u32; TEXTURE_COUNT],
}

/// Incremental archive/index setup. Each `advance` performs one bounded
/// operation: a guarded JAG open, one member read, map-index decode, one
/// maps.bin header scan, or the final bounded sort. It never retains a File.
pub(crate) struct CacheReaderCursor {
    jag_dir: PathBuf,
    snapshot_dir: PathBuf,
    phase: CacheReaderPhase,
}

enum CacheReaderPhase {
    ReadVersionList,
    ReadMapIndex(JagFile),
    DecodeMapIndex(Vec<u8>),
    ScanMapRecords {
        entries: Vec<MapIndexEntry>,
        builder: MapRecordIndexBuilder,
    },
    Complete,
}

impl CacheReaderCursor {
    pub(crate) fn new(input: ClientMapInput<'_>) -> Self {
        Self {
            jag_dir: input.jag_dir.to_owned(),
            snapshot_dir: input.snapshot_dir.to_owned(),
            phase: CacheReaderPhase::ReadVersionList,
        }
    }

    /// Advance exactly one setup unit. File handles are invocation-local.
    pub(crate) fn advance(&mut self) -> Result<Option<CacheReader>, MapError> {
        let phase = std::mem::replace(&mut self.phase, CacheReaderPhase::Complete);
        match phase {
            CacheReaderPhase::ReadVersionList => {
                let jag = read_jag(&self.jag_dir.join("versionlist"), "versionlist JAG")?;
                self.phase = CacheReaderPhase::ReadMapIndex(jag);
            }
            CacheReaderPhase::ReadMapIndex(jag) => {
                let index = jag
                    .read("map_index")
                    .ok_or(MapError::Invalid("missing versionlist map_index"))?;
                self.phase = CacheReaderPhase::DecodeMapIndex(index);
            }
            CacheReaderPhase::DecodeMapIndex(bytes) => {
                let entries = decode_map_index(&bytes).map_err(map_client_error)?;
                self.phase = CacheReaderPhase::ScanMapRecords {
                    entries,
                    builder: MapRecordIndexBuilder::default(),
                };
            }
            CacheReaderPhase::ScanMapRecords {
                entries,
                mut builder,
            } => {
                if builder.is_complete() {
                    let records = builder.finish()?;
                    return Ok(Some(CacheReader {
                        entries,
                        maps_path: self.snapshot_dir.join("maps.bin"),
                        records,
                        buffer: Vec::new(),
                        #[cfg(test)]
                        land_decode_calls: 0,
                        #[cfg(test)]
                        loc_read_calls: 0,
                    }));
                }
                let mut maps = File::open(self.snapshot_dir.join("maps.bin"))?;
                builder.step(&mut maps)?;
                drop(maps);
                self.phase = CacheReaderPhase::ScanMapRecords { entries, builder };
            }
            CacheReaderPhase::Complete => {
                return Err(MapError::Invalid("cache reader cursor complete"));
            }
        }
        Ok(None)
    }
}

impl CacheReaderCursor {
    pub(crate) fn tracked_bytes(&self) -> usize {
        match &self.phase {
            CacheReaderPhase::ReadVersionList | CacheReaderPhase::Complete => 0,
            CacheReaderPhase::ReadMapIndex(_) => MAX_JAG_READ_PEAK_BYTES as usize,
            CacheReaderPhase::DecodeMapIndex(bytes) => bytes.capacity(),
            CacheReaderPhase::ScanMapRecords { entries, builder } => {
                entries.capacity() * std::mem::size_of::<MapIndexEntry>() + builder.tracked_bytes()
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct CacheReader {
    pub entries: Vec<MapIndexEntry>,
    maps_path: PathBuf,
    records: MapRecordIndex,
    buffer: Vec<u8>,
    #[cfg(test)]
    land_decode_calls: usize,
    #[cfg(test)]
    loc_read_calls: usize,
}

impl CacheReader {
    pub(crate) fn entry(&self, square_x: i32, square_z: i32) -> Option<MapIndexEntry> {
        let x = u8::try_from(square_x).ok()?;
        let z = u8::try_from(square_z).ok()?;
        let packed = u16::from(x) << 8 | u16::from(z);
        self.entries
            .binary_search_by_key(&packed, |entry| entry.packed_square())
            .ok()
            .map(|index| self.entries[index])
    }

    pub(crate) fn read_land(&mut self, entry: MapIndexEntry) -> Result<LandSquare, MapError> {
        if entry.land_file == u16::MAX || !self.read_map_record(u32::from(entry.land_file))? {
            return Err(MapError::Invalid("missing listed land record"));
        }
        let land = LandSquare::decode(
            i32::from(entry.square_x),
            i32::from(entry.square_z),
            &self.buffer,
        )?;
        #[cfg(test)]
        {
            self.land_decode_calls += 1;
        }
        Ok(land)
    }

    pub(crate) fn visit_locs(
        &mut self,
        entry: MapIndexEntry,
        visit: impl FnMut(LocPlacement),
    ) -> Result<(), MapError> {
        if entry.loc_file == u16::MAX {
            return Ok(());
        }
        if !self.read_map_record(u32::from(entry.loc_file))? {
            return Err(MapError::Invalid("missing listed location record"));
        }
        visit_locations(&self.buffer, visit).map_err(map_client_error)
    }

    pub(crate) fn read_locs(
        &mut self,
        entry: MapIndexEntry,
    ) -> Result<Vec<LocPlacement>, MapError> {
        let mut placements = Vec::new();
        self.visit_locs(entry, |placement| placements.push(placement))?;
        #[cfg(test)]
        {
            self.loc_read_calls += 1;
        }
        Ok(placements)
    }

    fn read_map_record(&mut self, id: u32) -> Result<bool, MapError> {
        let mut maps = File::open(&self.maps_path)?;
        self.records.read_record(&mut maps, id, &mut self.buffer)
    }

    #[cfg(test)]
    pub(crate) fn decoded_read_counts(&self) -> (usize, usize) {
        (self.land_decode_calls, self.loc_read_calls)
    }

    pub(crate) fn max_buffer_bytes(&self) -> u32 {
        self.buffer.capacity().min(u32::MAX as usize) as u32
    }

    pub(crate) fn tracked_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<MapIndexEntry>()
            + self.records.tracked_bytes()
            + self.buffer.capacity()
    }
}

pub(crate) fn load_config(input: ClientMapInput<'_>) -> Result<JagFile, MapError> {
    read_jag(&input.jag_dir.join("config"), "config JAG")
}

pub(crate) fn load_raster_locs(jag: &JagFile) -> Result<Vec<RasterLocDefinition>, MapError> {
    let decoded_locs = catch_unwind(AssertUnwindSafe(|| LocType::unpack(jag)))
        .map_err(|_| MapError::Invalid("location definitions"))?;
    if decoded_locs.is_empty() {
        return Err(MapError::Invalid("missing location definitions"));
    }
    let mut locs = Vec::with_capacity(decoded_locs.len());
    for loc in decoded_locs {
        let width = u8::try_from(loc.width).map_err(|_| MapError::Invalid("location width"))?;
        let length = u8::try_from(loc.length).map_err(|_| MapError::Invalid("location length"))?;
        Footprint { width, length }.validate()?;
        locs.push(RasterLocDefinition {
            width,
            length,
            active: loc.active,
            mapscene: optional_u16(loc.mapscene, "mapscene id")?,
        });
    }
    Ok(locs)
}

pub(crate) fn load_floor_definitions(jag: &JagFile) -> Result<Vec<FloorDefinition>, MapError> {
    let decoded = catch_unwind(AssertUnwindSafe(|| FloType::unpack(jag)))
        .map_err(|_| MapError::Invalid("floor definitions"))?;
    if decoded.is_empty() {
        return Err(MapError::Invalid("missing floor definitions"));
    }
    Ok(decoded
        .into_iter()
        .map(|floor| FloorDefinition {
            colour: floor.colour as u32 & 0x00ff_ffff,
            texture: optional_u8(floor.texture),
            saturation: floor.saturation,
            lightness: floor.lightness,
            chroma: floor.chroma,
            underlay_hue: floor.underlay_hue,
        })
        .collect())
}

pub(crate) fn depack_mapscene(jag: &JagFile, id: usize) -> Result<Option<Pix8>, MapError> {
    catch_unwind(AssertUnwindSafe(|| {
        Pix8::depack(jag, "mapscene", id as i32)
    }))
    .map_err(|_| MapError::Invalid("mapscene sprite"))
}

const TEXTURE_NAMES: [&str; TEXTURE_COUNT] = [
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
    "17", "18", "19", "20", "21", "22", "23", "24", "25", "26", "27", "28", "29", "30", "31", "32",
    "33", "34", "35", "36", "37", "38", "39", "40", "41", "42", "43", "44", "45", "46", "47", "48",
    "49",
];

pub(crate) fn depack_texture(jag: &JagFile, id: usize) -> Result<Option<Pix8>, MapError> {
    let name = TEXTURE_NAMES
        .get(id)
        .copied()
        .ok_or(MapError::Invalid("texture id"))?;
    catch_unwind(AssertUnwindSafe(|| Pix8::depack(jag, name, 0)))
        .map_err(|_| MapError::Invalid("texture sprite"))
}

pub(crate) fn average_texture(texture: &Pix8) -> u32 {
    texture_average(texture)
}

fn optional_u16(value: i32, what: &'static str) -> Result<Option<u16>, MapError> {
    if value == -1 {
        Ok(None)
    } else {
        u16::try_from(value)
            .map(Some)
            .map_err(|_| MapError::Invalid(what))
    }
}

fn optional_u8(value: i32) -> Option<u8> {
    u8::try_from(value).ok()
}

fn cell_index(plane: u8, x: u8, z: u8) -> usize {
    usize::from(plane) * 64 * 64 + usize::from(x) * 64 + usize::from(z)
}

pub(crate) fn read_jag(path: &Path, what: &'static str) -> Result<JagFile, MapError> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() == 0 || metadata.len() > MAX_JAG_BYTES {
        return Err(MapError::Limit(what));
    }
    let bytes = std::fs::read(path)?;
    if bytes.len() as u64 > MAX_JAG_BYTES {
        return Err(MapError::Limit(what));
    }
    validate_jag_header(&bytes)?;
    catch_unwind(AssertUnwindSafe(|| JagFile::new(bytes))).map_err(|_| MapError::Invalid(what))
}

/// Check outer and inner declared sizes before native JagFile construction or
/// member decompression. Inputs are the immutable revision's CRC-verified JAGs.
fn validate_jag_header(bytes: &[u8]) -> Result<(), MapError> {
    if bytes.len() < 6 {
        return Err(MapError::Truncated);
    }
    let declared_unpacked = read_u24(&bytes[..3]);
    let declared_packed = read_u24(&bytes[3..6]);
    if declared_unpacked > 0x00ff_ffff || u64::from(declared_unpacked) > MAX_JAG_UNPACKED_BYTES {
        return Err(MapError::Limit("JAG declared unpacked bytes"));
    }
    if u64::from(declared_packed) > MAX_JAG_BYTES {
        return Err(MapError::Limit("JAG declared packed bytes"));
    }
    let available = bytes.len().saturating_sub(6);
    if declared_packed as usize > available {
        return Err(MapError::Truncated);
    }
    if declared_unpacked != declared_packed {
        return Ok(());
    }
    if available < 2 {
        return Err(MapError::Truncated);
    }
    let file_count = usize::from(u16::from_be_bytes([bytes[6], bytes[7]]));
    let table_bytes = 2usize
        .checked_add(
            file_count
                .checked_mul(10)
                .ok_or(MapError::Limit("JAG index bytes"))?,
        )
        .ok_or(MapError::Limit("JAG index bytes"))?;
    if table_bytes > declared_packed as usize {
        return Err(MapError::Truncated);
    }
    let table_end = 6 + table_bytes;
    let mut packed_members = 0usize;
    for row in bytes[8..table_end].as_chunks::<10>().0 {
        let member_unpacked = read_u24(&row[4..7]);
        let member_packed = read_u24(&row[7..10]);
        if u64::from(member_unpacked) > MAX_JAG_UNPACKED_BYTES {
            return Err(MapError::Limit("JAG member unpacked bytes"));
        }
        if u64::from(member_packed) > MAX_JAG_BYTES {
            return Err(MapError::Limit("JAG member packed bytes"));
        }
        packed_members = packed_members
            .checked_add(member_packed as usize)
            .ok_or(MapError::Limit("JAG member extent"))?;
    }
    if packed_members > declared_packed as usize - table_bytes {
        return Err(MapError::Truncated);
    }
    Ok(())
}

fn read_u24(bytes: &[u8]) -> u32 {
    (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2])
}

fn map_client_error(error: ClientMapError) -> MapError {
    match error {
        ClientMapError::Truncated(_) => MapError::Truncated,
        ClientMapError::Invalid(what) => MapError::Invalid(what),
        ClientMapError::Duplicate(what) => MapError::Duplicate(what),
        ClientMapError::Limit(what) => MapError::Limit(what),
    }
}
