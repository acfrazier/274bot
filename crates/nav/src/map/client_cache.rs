//! Bounded native client-cache decoding shared by the terrain and catalogue
//! producers. Records are read through a seek index, never retained as a world.

use super::poi::Footprint;
use super::producer::ClientMapInput;
use super::records::MapRecordIndex;
use super::spatial::WorldBounds;
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
use std::path::Path;

const MAX_JAG_BYTES: u64 = 8 * 1024 * 1024;
const MAPSCENE_COUNT: usize = 50;
const TEXTURE_COUNT: usize = 50;
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
    fn decode(square_x: i32, square_z: i32, bytes: &[u8]) -> Result<Self, MapError> {
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

pub(crate) struct CacheReader {
    pub entries: Vec<MapIndexEntry>,
    maps: File,
    records: MapRecordIndex,
    buffer: Vec<u8>,
}

impl CacheReader {
    pub(crate) fn open(input: ClientMapInput<'_>) -> Result<Self, MapError> {
        let versionlist = read_jag(&input.jag_dir.join("versionlist"), "versionlist JAG")?;
        let map_index = versionlist
            .read("map_index")
            .ok_or(MapError::Invalid("missing versionlist map_index"))?;
        let entries = decode_map_index(&map_index).map_err(map_client_error)?;
        let mut maps = File::open(input.snapshot_dir.join("maps.bin"))?;
        let records = MapRecordIndex::read(&mut maps)?;
        Ok(Self {
            entries,
            maps,
            records,
            buffer: Vec::new(),
        })
    }

    pub(crate) fn extent(&self) -> Result<WorldBounds, MapError> {
        let first = self
            .entries
            .first()
            .ok_or(MapError::Invalid("empty map index"))?;
        let mut west = i32::from(first.square_x);
        let mut south = i32::from(first.square_z);
        let mut east = west + 1;
        let mut north = south + 1;
        for entry in &self.entries[1..] {
            west = west.min(i32::from(entry.square_x));
            south = south.min(i32::from(entry.square_z));
            east = east.max(i32::from(entry.square_x) + 1);
            north = north.max(i32::from(entry.square_z) + 1);
        }
        Ok(WorldBounds {
            west: west * i32::from(MAP_SQUARE_SIZE),
            south: south * i32::from(MAP_SQUARE_SIZE),
            east: east * i32::from(MAP_SQUARE_SIZE),
            north: north * i32::from(MAP_SQUARE_SIZE),
        })
    }

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
        if entry.land_file == u16::MAX
            || !self.records.read_record(
                &mut self.maps,
                u32::from(entry.land_file),
                &mut self.buffer,
            )?
        {
            return Err(MapError::Invalid("missing listed land record"));
        }
        LandSquare::decode(
            i32::from(entry.square_x),
            i32::from(entry.square_z),
            &self.buffer,
        )
    }

    pub(crate) fn visit_locs(
        &mut self,
        entry: MapIndexEntry,
        visit: impl FnMut(LocPlacement),
    ) -> Result<(), MapError> {
        if entry.loc_file == u16::MAX {
            return Ok(());
        }
        if !self
            .records
            .read_record(&mut self.maps, u32::from(entry.loc_file), &mut self.buffer)?
        {
            return Err(MapError::Invalid("missing listed location record"));
        }
        visit_locations(&self.buffer, visit).map_err(map_client_error)
    }

    pub(crate) fn max_buffer_bytes(&self) -> u32 {
        self.buffer.capacity().min(u32::MAX as usize) as u32
    }

    pub(crate) fn tracked_bytes(&self) -> usize {
        self.entries.capacity() * std::mem::size_of::<MapIndexEntry>()
            + std::mem::size_of_val(self.records.records())
            + self.buffer.capacity()
    }
}

pub(crate) fn load_config(input: ClientMapInput<'_>) -> Result<JagFile, MapError> {
    read_jag(&input.jag_dir.join("config"), "config JAG")
}

pub(crate) fn load_definitions(input: ClientMapInput<'_>) -> Result<Definitions, MapError> {
    let jag = load_config(input)?;
    let decoded_locs = catch_unwind(AssertUnwindSafe(|| LocType::unpack(&jag)))
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
    let decoded_floors = catch_unwind(AssertUnwindSafe(|| FloType::unpack(&jag)))
        .map_err(|_| MapError::Invalid("floor definitions"))?;
    if decoded_floors.is_empty() {
        return Err(MapError::Invalid("missing floor definitions"));
    }
    let floors = decoded_floors
        .into_iter()
        .map(|floor| FloorDefinition {
            colour: floor.colour as u32 & 0x00ff_ffff,
            texture: optional_u8(floor.texture),
            saturation: floor.saturation,
            lightness: floor.lightness,
            chroma: floor.chroma,
            underlay_hue: floor.underlay_hue,
        })
        .collect();
    Ok(Definitions { locs, floors })
}

pub(crate) fn load_raster_assets(input: ClientMapInput<'_>) -> Result<RasterAssets, MapError> {
    let media = read_jag(&input.jag_dir.join("media"), "media JAG")?;
    let mut mapscenes = Vec::with_capacity(MAPSCENE_COUNT);
    for id in 0..MAPSCENE_COUNT {
        let sprite = catch_unwind(AssertUnwindSafe(|| {
            Pix8::depack(&media, "mapscene", id as i32)
        }))
        .map_err(|_| MapError::Invalid("mapscene sprite"))?;
        mapscenes.push(sprite);
    }

    let textures = read_jag(&input.jag_dir.join("textures"), "textures JAG")?;
    let mut texture_colours = [0; TEXTURE_COUNT];
    for (id, colour) in texture_colours.iter_mut().enumerate() {
        let texture = catch_unwind(AssertUnwindSafe(|| {
            Pix8::depack(&textures, &id.to_string(), 0)
        }))
        .map_err(|_| MapError::Invalid("texture sprite"))?;
        if let Some(texture) = texture {
            *colour = texture_average(&texture);
        }
    }
    Ok(RasterAssets {
        mapscenes,
        texture_colours,
    })
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

fn read_jag(path: &Path, what: &'static str) -> Result<JagFile, MapError> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() == 0 || metadata.len() > MAX_JAG_BYTES {
        return Err(MapError::Limit(what));
    }
    let bytes = std::fs::read(path)?;
    catch_unwind(AssertUnwindSafe(|| JagFile::new(bytes))).map_err(|_| MapError::Invalid(what))
}

fn map_client_error(error: ClientMapError) -> MapError {
    match error {
        ClientMapError::Truncated(_) => MapError::Truncated,
        ClientMapError::Invalid(what) => MapError::Invalid(what),
        ClientMapError::Duplicate(what) => MapError::Duplicate(what),
        ClientMapError::Limit(what) => MapError::Limit(what),
    }
}
