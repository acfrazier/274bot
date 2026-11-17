//! Cache-family selection for map imagery. Legacy decoders remain unchanged.

use super::client_cache::{self, FloorDefinition, RasterLocDefinition};
use super::poi::Footprint;
use super::MapError;
use client::config::config_377::{DecodeError377, FloType377, LocType377, Reader377};
use client::io::JagFile;

pub(super) fn validate(revision: u16) -> Result<(), MapError> {
    match revision {
        274 | 289 | 377 => Ok(()),
        _ => Err(MapError::Invalid("revision")),
    }
}

pub(super) fn load_raster_locs(
    revision: u16,
    jag: &JagFile,
) -> Result<Vec<RasterLocDefinition>, MapError> {
    match revision {
        274 | 289 => client_cache::load_raster_locs(jag),
        377 => {
            let data = jag
                .read("loc.dat")
                .ok_or(MapError::Invalid("missing loc.dat"))?;
            let index = jag
                .read("loc.idx")
                .ok_or(MapError::Invalid("missing loc.idx"))?;
            let mut records = Reader377::new(&data, "loc.dat", 0);
            let mut index = Reader377::new(&index, "loc.idx", 0);
            let count = index.g2().map_err(decode_error)? as usize;
            if records.g2().map_err(decode_error)? as usize != count {
                return Err(MapError::Invalid("location count"));
            }
            if count == 0 {
                return Err(MapError::Invalid("missing location definitions"));
            }
            let mut locs = Vec::with_capacity(count);
            for id in 0..count {
                records.record = id;
                index.record = id;
                let size = index.g2().map_err(decode_error)? as usize;
                let bytes = records.take(size).map_err(decode_error)?;
                let loc = LocType377::decode(&mut Reader377::new(bytes, "loc.dat", id))
                    .map_err(decode_error)?;
                let width =
                    u8::try_from(loc.width).map_err(|_| MapError::Invalid("location width"))?;
                let length =
                    u8::try_from(loc.length).map_err(|_| MapError::Invalid("location length"))?;
                Footprint { width, length }.validate()?;
                let mapscene = match loc.mapscene {
                    -1 => None,
                    value => {
                        Some(u16::try_from(value).map_err(|_| MapError::Invalid("mapscene id"))?)
                    }
                };
                locs.push(RasterLocDefinition {
                    width,
                    length,
                    active: loc.active,
                    mapscene,
                });
            }
            index.finish().map_err(decode_error)?;
            records.finish().map_err(decode_error)?;
            Ok(locs)
        }
        _ => Err(MapError::Invalid("revision")),
    }
}

pub(super) fn load_floor_definitions(
    revision: u16,
    jag: &JagFile,
) -> Result<Vec<FloorDefinition>, MapError> {
    match revision {
        274 | 289 => client_cache::load_floor_definitions(jag),
        377 => {
            let data = jag
                .read("flo.dat")
                .ok_or(MapError::Invalid("missing flo.dat"))?;
            let mut records = Reader377::new(&data, "flo.dat", 0);
            let count = records.g2().map_err(decode_error)? as usize;
            if count == 0 {
                return Err(MapError::Invalid("missing floor definitions"));
            }
            let mut floors = Vec::with_capacity(count);
            // The raster consumes unjittered HSL, never the randomized overlay_hsl.
            let mut random = || 0.5;
            for id in 0..count {
                records.record = id;
                let floor = FloType377::decode(&mut records, &mut random).map_err(decode_error)?;
                floors.push(FloorDefinition {
                    colour: floor.colour as u32 & 0x00ff_ffff,
                    texture: u8::try_from(floor.texture).ok(),
                    saturation: floor.saturation,
                    lightness: floor.lightness,
                    chroma: floor.chroma,
                    underlay_hue: floor.underlay_hue,
                });
            }
            records.finish().map_err(decode_error)?;
            Ok(floors)
        }
        _ => Err(MapError::Invalid("revision")),
    }
}

fn decode_error(error: DecodeError377) -> MapError {
    MapError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
#[path = "cache_revision_tests.rs"]
mod tests;
