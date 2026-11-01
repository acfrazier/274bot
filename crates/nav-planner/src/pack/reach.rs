//! v18 reachability section codec: labels, graph, placements, and stands.

use crate::reachability::{
    check_base_tables, check_cell_runs, check_chunk, check_closure_run, check_coverage_rows,
    check_csr_shape, check_geometry_constants, check_geometry_shape, check_placement_rows,
    check_stand_order, check_stand_rows, AtlasGeometry, CellRun, Chunk, ClosureRun,
    ConditionalJoin, CoverageRow, ExceptionalTransition, PlacementRow, ReachabilityAtlas,
    SessionTemplate, StandRow, TeleportTemplate,
};

use super::coord::unpack_tile_key;
use super::{read_u32, PackError, PackRead};

const LABELS_KINDS: [u32; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
const GRAPH_KINDS: [u32; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
const ONE_KIND: [u32; 1] = [1];
const LABELS_STRIDES: [u32; 8] = [24, 20, 4, 1, 8, 16, 8, 8];
const GRAPH_STRIDES: [u32; 10] = [4, 4, 4, 8, 4, 8, 4, 8, 12, 28];
const PLACEMENT_STRIDE: [u32; 1] = [24];
const STAND_STRIDE: [u32; 1] = [8];

#[derive(Clone, Copy)]
struct DirEnt {
    offset: u32,
    count: u32,
    stride: u32,
}

fn u32_slice(bytes: &[u8]) -> Result<u32, PackError> {
    let slice = bytes.get(..4).ok_or(PackError::Truncated)?;
    let mut arr = [0u8; 4];
    arr.copy_from_slice(slice);
    Ok(u32::from_le_bytes(arr))
}

fn i32_slice(bytes: &[u8]) -> Result<i32, PackError> {
    let slice = bytes.get(..4).ok_or(PackError::Truncated)?;
    let mut arr = [0u8; 4];
    arr.copy_from_slice(slice);
    Ok(i32::from_le_bytes(arr))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, PackError> {
    u32_slice(bytes.get(at..).ok_or(PackError::Truncated)?)
}

fn i32_at(bytes: &[u8], at: usize) -> Result<i32, PackError> {
    i32_slice(bytes.get(at..).ok_or(PackError::Truncated)?)
}

/// Encode the four length-prefixed reach sections.
pub fn encode_atlas_sections(atlas: &ReachabilityAtlas) -> Result<Vec<u8>, PackError> {
    let labels = encode_labels(atlas)?;
    let graph = encode_graph(atlas)?;
    let placements = encode_placements(atlas)?;
    let stands = encode_stands(atlas)?;
    let mut out = Vec::new();
    write_len_prefixed(&mut out, &labels)?;
    write_len_prefixed(&mut out, &graph)?;
    write_len_prefixed(&mut out, &placements)?;
    write_len_prefixed(&mut out, &stands)?;
    Ok(out)
}

/// Decode the four length-prefixed reach sections from the current reader.
pub(crate) fn decode_atlas_sections<R: PackRead>(
    r: &mut R,
) -> Result<ReachabilityAtlas, PackError> {
    let labels = read_len_prefixed(r)?;
    let graph = read_len_prefixed(r)?;
    let placements = read_len_prefixed(r)?;
    let stands = read_len_prefixed(r)?;
    let mut atlas = decode_labels(&labels)?;
    decode_graph(&graph, &mut atlas)?;
    decode_placements(&placements, &mut atlas)?;
    decode_stands(&stands, &mut atlas)?;
    Ok(atlas)
}

fn write_len_prefixed(out: &mut Vec<u8>, payload: &[u8]) -> Result<(), PackError> {
    let len = u32::try_from(payload.len())
        .map_err(|_| PackError::BadLength("reach section length exceeds u32".into()))?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
    Ok(())
}

fn read_len_prefixed<R: PackRead>(r: &mut R) -> Result<Vec<u8>, PackError> {
    let len = read_u32(r)? as usize;
    if len > r.remaining() {
        return Err(PackError::Truncated);
    }
    let mut buf = vec![0u8; len];
    r.read_bytes_exact(&mut buf)?;
    Ok(buf)
}

fn encode_section(kinds: &[u32], tables: &[(u32, Vec<u8>)]) -> Result<Vec<u8>, PackError> {
    if kinds.len() != tables.len() {
        return Err(PackError::BadLength(
            "reach directory kind count does not match tables".into(),
        ));
    }
    let n = u32::try_from(tables.len())
        .map_err(|_| PackError::BadLength("reach subtable count exceeds u32".into()))?;
    let dir_bytes = 4u32
        .checked_add(
            n.checked_mul(16)
                .ok_or_else(|| PackError::BadLength("reach directory size overflows".into()))?,
        )
        .ok_or_else(|| PackError::BadLength("reach directory size overflows".into()))?;
    let mut offset = dir_bytes;
    let mut directory = Vec::new();
    directory.extend_from_slice(&n.to_le_bytes());
    for (i, &(stride, ref bytes)) in tables.iter().enumerate() {
        if stride == 0 {
            return Err(PackError::BadLength("reach table stride is zero".into()));
        }
        if bytes.len() % stride as usize != 0 {
            return Err(PackError::BadLength(
                "reach table length is not a multiple of stride".into(),
            ));
        }
        let count = u32::try_from(bytes.len() / stride as usize)
            .map_err(|_| PackError::BadLength("reach table count exceeds u32".into()))?;
        directory.extend_from_slice(&kinds[i].to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(&count.to_le_bytes());
        directory.extend_from_slice(&stride.to_le_bytes());
        offset = offset
            .checked_add(
                u32::try_from(bytes.len())
                    .map_err(|_| PackError::BadLength("reach table length exceeds u32".into()))?,
            )
            .ok_or_else(|| PackError::BadLength("reach table offset overflows".into()))?;
    }
    let mut out = directory;
    for (_, bytes) in tables {
        out.extend_from_slice(bytes);
    }
    Ok(out)
}

fn decode_directory(
    payload: &[u8],
    expected: &[u32],
    expected_strides: &[u32],
) -> Result<(Vec<DirEnt>, usize), PackError> {
    if payload.len() < 4 {
        return Err(PackError::Truncated);
    }
    let n = u32_at(payload, 0)? as usize;
    if n != expected.len() {
        return Err(PackError::BadLength(format!(
            "reach directory has {n} tables, expected {}",
            expected.len()
        )));
    }
    let dir_end = 4usize
        .checked_add(
            n.checked_mul(16)
                .ok_or_else(|| PackError::BadLength("reach directory size overflows".into()))?,
        )
        .ok_or_else(|| PackError::BadLength("reach directory size overflows".into()))?;
    if payload.len() < dir_end {
        return Err(PackError::Truncated);
    }
    let mut ents = Vec::with_capacity(n);
    let mut seen = Vec::new();
    for (i, &expected_kind) in expected.iter().enumerate() {
        let at = 4 + i * 16;
        let kind = u32_at(payload, at)?;
        let offset = u32_at(payload, at + 4)?;
        let count = u32_at(payload, at + 8)?;
        let stride = u32_at(payload, at + 12)?;
        if kind != expected_kind {
            return Err(PackError::BadLength(format!(
                "reach table {i} has kind {kind}, expected {expected_kind}"
            )));
        }
        if seen.contains(&kind) {
            return Err(PackError::BadLength(format!(
                "reach directory repeats kind {kind}"
            )));
        }
        seen.push(kind);
        if stride != expected_strides[i] {
            return Err(PackError::BadLength(format!(
                "reach table {i} has stride {stride}, expected {}",
                expected_strides[i]
            )));
        }
        ents.push(DirEnt {
            offset,
            count,
            stride,
        });
    }
    let mut cursor = dir_end as u32;
    for ent in &ents {
        if ent.offset != cursor {
            return Err(PackError::BadLength(
                "reach tables are not contiguous after the directory".into(),
            ));
        }
        let bytes = (ent.count as u64)
            .checked_mul(ent.stride as u64)
            .ok_or_else(|| PackError::BadLength("reach table size overflows".into()))?;
        let end = (ent.offset as u64)
            .checked_add(bytes)
            .ok_or_else(|| PackError::BadLength("reach table end overflows".into()))?;
        if end > payload.len() as u64 {
            return Err(PackError::Truncated);
        }
        cursor = u32::try_from(end)
            .map_err(|_| PackError::BadLength("reach table end exceeds u32".into()))?;
    }
    if cursor as usize != payload.len() {
        return Err(PackError::BadLength(
            "reach section has trailing bytes after its tables".into(),
        ));
    }
    Ok((ents, dir_end))
}

fn table_bytes(payload: &[u8], ent: DirEnt) -> Result<&[u8], PackError> {
    let start = ent.offset as usize;
    let len = (ent.count as usize)
        .checked_mul(ent.stride as usize)
        .ok_or_else(|| PackError::BadLength("reach table size overflows".into()))?;
    payload.get(start..start + len).ok_or(PackError::Truncated)
}

fn encode_labels(atlas: &ReachabilityAtlas) -> Result<Vec<u8>, PackError> {
    let g = &atlas.geometry;
    check_geometry_constants(g)?;
    let mut geometry = Vec::with_capacity(24);
    geometry.extend_from_slice(&g.origin_x.to_le_bytes());
    geometry.extend_from_slice(&g.origin_z.to_le_bytes());
    geometry.extend_from_slice(&g.width.to_le_bytes());
    geometry.extend_from_slice(&g.height.to_le_bytes());
    geometry.extend_from_slice(&g.planes.to_le_bytes());
    geometry.extend_from_slice(&g.chunk_side.to_le_bytes());
    let mut chunks = Vec::new();
    for chunk in atlas.chunks.iter() {
        chunks.extend_from_slice(&chunk.palette_start.to_le_bytes());
        chunks.extend_from_slice(&chunk.palette_count.to_le_bytes());
        chunks.extend_from_slice(&chunk.index_start.to_le_bytes());
        chunks.extend_from_slice(&chunk.index_len.to_le_bytes());
        chunks.push(chunk.bit_width);
        chunks.extend_from_slice(&[0, 0, 0]);
    }
    let mut palette = Vec::new();
    for &label in atlas.palette.iter() {
        palette.extend_from_slice(&label.to_le_bytes());
    }
    let mut coverage = Vec::new();
    for row in atlas.coverage.iter() {
        coverage.extend_from_slice(&row.tile.to_le_bytes());
        coverage.extend_from_slice(&row.reason.to_le_bytes());
    }
    let mut exceptional = Vec::new();
    for row in atlas.exceptional.iter() {
        exceptional.extend_from_slice(&row.source.to_le_bytes());
        exceptional.extend_from_slice(&row.destination.to_le_bytes());
        exceptional.extend_from_slice(&row.edge.to_le_bytes());
        exceptional.extend_from_slice(&row.reason.to_le_bytes());
    }
    let mut paint = Vec::new();
    for row in atlas.legacy_paint.iter() {
        paint.extend_from_slice(&row.first.to_le_bytes());
        paint.extend_from_slice(&row.count.to_le_bytes());
    }
    let mut scatter = Vec::new();
    for row in atlas.scatter.iter() {
        scatter.extend_from_slice(&row.first.to_le_bytes());
        scatter.extend_from_slice(&row.count.to_le_bytes());
    }
    encode_section(
        &LABELS_KINDS,
        &[
            (24, geometry),
            (20, chunks),
            (4, palette),
            (1, atlas.indexes.to_vec()),
            (8, coverage),
            (16, exceptional),
            (8, paint),
            (8, scatter),
        ],
    )
}

fn decode_labels(payload: &[u8]) -> Result<ReachabilityAtlas, PackError> {
    let (ents, _) = decode_directory(payload, &LABELS_KINDS, &LABELS_STRIDES)?;
    let row = table_bytes(payload, ents[0])?;
    if row.len() != 24 {
        return Err(PackError::BadLength(
            "atlas geometry table must have one 24-byte row".into(),
        ));
    }
    let geometry = AtlasGeometry {
        origin_x: i32_at(row, 0)?,
        origin_z: i32_at(row, 4)?,
        width: u32_at(row, 8)?,
        height: u32_at(row, 12)?,
        planes: u32_at(row, 16)?,
        chunk_side: u32_at(row, 20)?,
    };
    let chunk_bytes = table_bytes(payload, ents[1])?;
    check_geometry_shape(&geometry, ents[1].count as usize)?;
    let mut chunks = Vec::with_capacity(ents[1].count as usize);
    for row in chunk_bytes.as_chunks::<20>().0 {
        if row[17] != 0 || row[18] != 0 || row[19] != 0 {
            return Err(PackError::BadLength(
                "atlas chunk reserved bytes must be zero".into(),
            ));
        }
        chunks.push(Chunk {
            palette_start: u32_at(row, 0)?,
            palette_count: u32_at(row, 4)?,
            index_start: u32_at(row, 8)?,
            index_len: u32_at(row, 12)?,
            bit_width: row[16],
        });
    }
    let palette_bytes = table_bytes(payload, ents[2])?;
    let mut palette = Vec::with_capacity(ents[2].count as usize);
    for row in palette_bytes.as_chunks::<4>().0 {
        palette.push(u32_slice(row)?);
    }
    let indexes = table_bytes(payload, ents[3])?.to_vec();
    for chunk in &chunks {
        check_chunk(chunk, &palette, &indexes)?;
    }
    Ok(ReachabilityAtlas {
        geometry,
        chunks: chunks.into_boxed_slice(),
        palette: palette.into_boxed_slice(),
        indexes: indexes.into_boxed_slice(),
        coverage: read_coverage(table_bytes(payload, ents[4])?)?,
        exceptional: read_exceptional(table_bytes(payload, ents[5])?)?,
        legacy_paint: read_runs(table_bytes(payload, ents[6])?)?,
        scatter: read_runs(table_bytes(payload, ents[7])?)?,
        base_to_condensed: Box::from([]),
        base_sizes: Box::from([]),
        movement_offsets: Box::from([0]),
        movement_runs: Box::from([]),
        transport_offsets: Box::from([0]),
        transport_runs: Box::from([]),
        conditional_offsets: Box::from([0]),
        conditional_joins: Box::from([]),
        teleports: Box::from([]),
        sessions: Box::from([]),
        placements: Box::from([]),
        stands: Box::from([]),
    })
}

fn read_coverage(bytes: &[u8]) -> Result<Box<[CoverageRow]>, PackError> {
    let mut rows = Vec::with_capacity(bytes.len() / 8);
    for row in bytes.as_chunks::<8>().0 {
        rows.push(CoverageRow {
            tile: u32_at(row, 0)?,
            reason: u32_at(row, 4)?,
        });
    }
    check_coverage_rows(&rows)?;
    Ok(rows.into_boxed_slice())
}

fn read_exceptional(bytes: &[u8]) -> Result<Box<[ExceptionalTransition]>, PackError> {
    let mut rows = Vec::with_capacity(bytes.len() / 16);
    for row in bytes.as_chunks::<16>().0 {
        let source = u32_at(row, 0)?;
        let destination = u32_at(row, 4)?;
        unpack_tile_key(source)?;
        unpack_tile_key(destination)?;
        rows.push(ExceptionalTransition {
            source,
            destination,
            edge: u32_at(row, 8)?,
            reason: u32_at(row, 12)?,
        });
    }
    Ok(rows.into_boxed_slice())
}

fn read_runs(bytes: &[u8]) -> Result<Box<[CellRun]>, PackError> {
    let mut rows = Vec::with_capacity(bytes.len() / 8);
    for row in bytes.as_chunks::<8>().0 {
        rows.push(CellRun {
            first: u32_at(row, 0)?,
            count: u32_at(row, 4)?,
        });
    }
    check_cell_runs(&rows)?;
    Ok(rows.into_boxed_slice())
}

fn encode_u32_table(values: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

fn encode_graph(atlas: &ReachabilityAtlas) -> Result<Vec<u8>, PackError> {
    let mut movement_runs = Vec::new();
    for row in atlas.movement_runs.iter() {
        movement_runs.extend_from_slice(&row.first.to_le_bytes());
        movement_runs.extend_from_slice(&row.count.to_le_bytes());
    }
    let mut transport_runs = Vec::new();
    for row in atlas.transport_runs.iter() {
        transport_runs.extend_from_slice(&row.first.to_le_bytes());
        transport_runs.extend_from_slice(&row.count.to_le_bytes());
    }
    let mut joins = Vec::new();
    for row in atlas.conditional_joins.iter() {
        joins.extend_from_slice(&row.destination.to_le_bytes());
        joins.extend_from_slice(&row.edge.to_le_bytes());
    }
    let mut teleports = Vec::new();
    for row in atlas.teleports.iter() {
        teleports.extend_from_slice(&row.teleport.to_le_bytes());
        teleports.extend_from_slice(&row.landing_base.to_le_bytes());
        teleports.extend_from_slice(&row.reason.to_le_bytes());
    }
    let mut sessions = Vec::new();
    for row in atlas.sessions.iter() {
        sessions.extend_from_slice(&row.kind.to_le_bytes());
        sessions.extend_from_slice(&row.source_base.to_le_bytes());
        sessions.extend_from_slice(&row.landing_base.to_le_bytes());
        sessions.extend_from_slice(&row.source_tile.to_le_bytes());
        sessions.extend_from_slice(&row.landing_tile.to_le_bytes());
        sessions.extend_from_slice(&row.selector.to_le_bytes());
        sessions.extend_from_slice(&row.reason.to_le_bytes());
    }
    encode_section(
        &GRAPH_KINDS,
        &[
            (4, encode_u32_table(&atlas.base_to_condensed)),
            (4, encode_u32_table(&atlas.base_sizes)),
            (4, encode_u32_table(&atlas.movement_offsets)),
            (8, movement_runs),
            (4, encode_u32_table(&atlas.transport_offsets)),
            (8, transport_runs),
            (4, encode_u32_table(&atlas.conditional_offsets)),
            (8, joins),
            (12, teleports),
            (28, sessions),
        ],
    )
}

fn decode_u32_table(bytes: &[u8]) -> Result<Box<[u32]>, PackError> {
    if !bytes.len().is_multiple_of(4) {
        return Err(PackError::BadLength(
            "reach u32 table length is not aligned".into(),
        ));
    }
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|row| u32_slice(row))
        .collect::<Result<Vec<_>, _>>()?
        .into_boxed_slice())
}

fn decode_graph(payload: &[u8], atlas: &mut ReachabilityAtlas) -> Result<(), PackError> {
    let (ents, _) = decode_directory(payload, &GRAPH_KINDS, &GRAPH_STRIDES)?;
    atlas.base_to_condensed = decode_u32_table(table_bytes(payload, ents[0])?)?;
    atlas.base_sizes = decode_u32_table(table_bytes(payload, ents[1])?)?;
    check_base_tables(&atlas.base_to_condensed, &atlas.base_sizes)?;
    atlas.movement_offsets = decode_u32_table(table_bytes(payload, ents[2])?)?;
    atlas.movement_runs = read_closure_runs(table_bytes(payload, ents[3])?)?;
    atlas.transport_offsets = decode_u32_table(table_bytes(payload, ents[4])?)?;
    atlas.transport_runs = read_closure_runs(table_bytes(payload, ents[5])?)?;
    atlas.conditional_offsets = decode_u32_table(table_bytes(payload, ents[6])?)?;
    atlas.conditional_joins = read_joins(table_bytes(payload, ents[7])?)?;
    atlas.teleports = read_teleports(table_bytes(payload, ents[8])?)?;
    atlas.sessions = read_sessions(table_bytes(payload, ents[9])?)?;
    let base_count = atlas.base_to_condensed.len();
    let condensed = atlas.condensed_count() as usize;
    check_csr_shape(
        &atlas.movement_offsets,
        atlas.movement_runs.len(),
        base_count,
        "movement",
    )?;
    check_csr_shape(
        &atlas.transport_offsets,
        atlas.transport_runs.len(),
        condensed,
        "transport",
    )?;
    check_csr_shape(
        &atlas.conditional_offsets,
        atlas.conditional_joins.len(),
        condensed,
        "conditional",
    )?;
    Ok(())
}

fn read_closure_runs(bytes: &[u8]) -> Result<Box<[ClosureRun]>, PackError> {
    let mut rows = Vec::with_capacity(bytes.len() / 8);
    for row in bytes.as_chunks::<8>().0 {
        let run = ClosureRun {
            first: u32_at(row, 0)?,
            count: u32_at(row, 4)?,
        };
        check_closure_run(&run)?;
        rows.push(run);
    }
    Ok(rows.into_boxed_slice())
}

fn read_joins(bytes: &[u8]) -> Result<Box<[ConditionalJoin]>, PackError> {
    let mut rows = Vec::with_capacity(bytes.len() / 8);
    for row in bytes.as_chunks::<8>().0 {
        rows.push(ConditionalJoin {
            destination: u32_at(row, 0)?,
            edge: u32_at(row, 4)?,
        });
    }
    Ok(rows.into_boxed_slice())
}

fn read_teleports(bytes: &[u8]) -> Result<Box<[TeleportTemplate]>, PackError> {
    let mut rows = Vec::with_capacity(bytes.len() / 12);
    for row in bytes.as_chunks::<12>().0 {
        rows.push(TeleportTemplate {
            teleport: u32_at(row, 0)?,
            landing_base: u32_at(row, 4)?,
            reason: u32_at(row, 8)?,
        });
    }
    Ok(rows.into_boxed_slice())
}

fn read_sessions(bytes: &[u8]) -> Result<Box<[SessionTemplate]>, PackError> {
    let mut rows = Vec::with_capacity(bytes.len() / 28);
    for row in bytes.as_chunks::<28>().0 {
        let source_tile = u32_at(row, 12)?;
        let landing_tile = u32_at(row, 16)?;
        unpack_tile_key(source_tile)?;
        unpack_tile_key(landing_tile)?;
        rows.push(SessionTemplate {
            kind: u32_at(row, 0)?,
            source_base: u32_at(row, 4)?,
            landing_base: u32_at(row, 8)?,
            source_tile,
            landing_tile,
            selector: u32_at(row, 20)?,
            reason: u32_at(row, 24)?,
        });
    }
    Ok(rows.into_boxed_slice())
}

fn encode_placements(atlas: &ReachabilityAtlas) -> Result<Vec<u8>, PackError> {
    let mut bytes = Vec::new();
    for row in atlas.placements.iter() {
        bytes.extend_from_slice(&row.origin.to_le_bytes());
        bytes.extend_from_slice(&row.loc_id.to_le_bytes());
        bytes.push(row.shape);
        bytes.push(row.angle);
        bytes.push(row.coverage);
        bytes.push(row.trigger);
        bytes.extend_from_slice(&row.stand_start.to_le_bytes());
        bytes.extend_from_slice(&row.stand_count.to_le_bytes());
        bytes.extend_from_slice(&row.detail.to_le_bytes());
    }
    encode_section(&ONE_KIND, &[(24, bytes)])
}

fn decode_placements(payload: &[u8], atlas: &mut ReachabilityAtlas) -> Result<(), PackError> {
    let (ents, _) = decode_directory(payload, &ONE_KIND, &PLACEMENT_STRIDE)?;
    let bytes = table_bytes(payload, ents[0])?;
    let mut rows = Vec::with_capacity(bytes.len() / 24);
    for row in bytes.as_chunks::<24>().0 {
        rows.push(PlacementRow {
            origin: u32_at(row, 0)?,
            loc_id: i32_at(row, 4)?,
            shape: row[8],
            angle: row[9],
            coverage: row[10],
            trigger: row[11],
            stand_start: u32_at(row, 12)?,
            stand_count: u32_at(row, 16)?,
            detail: u32_at(row, 20)?,
        });
    }
    check_placement_rows(&rows)?;
    atlas.placements = rows.into_boxed_slice();
    Ok(())
}

fn encode_stands(atlas: &ReachabilityAtlas) -> Result<Vec<u8>, PackError> {
    let mut bytes = Vec::new();
    for row in atlas.stands.iter() {
        bytes.extend_from_slice(&row.tile.to_le_bytes());
        bytes.extend_from_slice(&row.base.to_le_bytes());
    }
    encode_section(&ONE_KIND, &[(8, bytes)])
}

fn decode_stands(payload: &[u8], atlas: &mut ReachabilityAtlas) -> Result<(), PackError> {
    let (ents, _) = decode_directory(payload, &ONE_KIND, &STAND_STRIDE)?;
    let bytes = table_bytes(payload, ents[0])?;
    let mut rows = Vec::with_capacity(bytes.len() / 8);
    for row in bytes.as_chunks::<8>().0 {
        rows.push(StandRow {
            tile: u32_at(row, 0)?,
            base: u32_at(row, 4)?,
        });
    }
    check_stand_rows(&rows, atlas.base_to_condensed.len())?;
    atlas.stands = rows.into_boxed_slice();
    for place in atlas.placements.iter() {
        check_stand_order(atlas.stands_for(place)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{decode_atlas_sections, encode_atlas_sections};
    use crate::collision::{pack_walk, WorldCollision};
    use crate::pack::{pack_tile_key, PackError};
    use crate::reachability::{
        CellRun, Chunk, ClosureRun, CoverageRow, PlacementRow, ReachabilityAtlas, StandRow,
        PLACEMENT_COMPLETE, TRIGGER_OP,
    };
    use crate::transport::TransportGraph;
    use api::snapshot::WorldTile;
    use std::io::Cursor;

    fn tiny_collision() -> WorldCollision {
        let (walk, blocked) = pack_walk(&[0; 8]);
        WorldCollision {
            origin: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            width: 2,
            height: 1,
            walk,
            blocked,
            flags: None,
        }
    }

    #[test]
    fn unlabeled_atlas_roundtrips() {
        let collision = tiny_collision();
        let atlas = ReachabilityAtlas::unlabeled(&collision).unwrap();
        let bytes = encode_atlas_sections(&atlas).unwrap();
        let decoded = decode_atlas_sections(&mut Cursor::new(bytes.as_slice())).unwrap();
        assert_eq!(decoded.geometry.width, 2);
        assert_eq!(decoded.geometry.height, 1);
        assert_eq!(decoded.base_count(), 0);
        assert_eq!(
            decoded
                .base_at(WorldTile {
                    x: 3200,
                    z: 3200,
                    level: 0
                })
                .unwrap(),
            None
        );
    }

    fn key() -> u32 {
        pack_tile_key(WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        })
        .unwrap()
    }

    /// One base component, no placements: the control both sides accept.
    fn one_component() -> ReachabilityAtlas {
        let mut atlas = ReachabilityAtlas::unlabeled(&tiny_collision()).unwrap();
        atlas.base_to_condensed = Box::from([0]);
        atlas.base_sizes = Box::from([1]);
        atlas.movement_offsets = Box::from([0, 1]);
        atlas.movement_runs = Box::from([ClosureRun { first: 0, count: 1 }]);
        atlas.transport_offsets = Box::from([0, 1]);
        atlas.transport_runs = Box::from([ClosureRun { first: 0, count: 1 }]);
        atlas.conditional_offsets = Box::from([0, 0]);
        atlas
    }

    fn placement() -> PlacementRow {
        PlacementRow {
            origin: key(),
            loc_id: 42,
            shape: 0,
            angle: 0,
            coverage: PLACEMENT_COMPLETE,
            trigger: TRIGGER_OP,
            stand_start: 0,
            stand_count: 0,
            detail: 0,
        }
    }

    /// The atlas the producer validates and the reader decodes. A refusal
    /// must carry the same message on both sides.
    fn verdicts(atlas: &ReachabilityAtlas) -> (Result<(), String>, Result<(), String>) {
        let message = |error: PackError| match error {
            PackError::BadLength(message) => message,
            other => format!("{other:?}"),
        };
        let produced = atlas
            .validate(&tiny_collision(), &TransportGraph::default())
            .map_err(message);
        let bytes = encode_atlas_sections(atlas).unwrap();
        let read = decode_atlas_sections(&mut Cursor::new(bytes.as_slice()))
            .map(|_| ())
            .map_err(message);
        (produced, read)
    }

    /// Every table invariant the reader checks while it decodes is checked by
    /// `validate` with the same message, so the writer (which calls
    /// `validate`) cannot produce what the reader refuses. Each case breaks
    /// exactly one invariant of a table the reader checks itself.
    #[test]
    fn validate_and_the_reader_refuse_the_same_tables() {
        assert_eq!(verdicts(&one_component()), (Ok(()), Ok(())));
        type Break = fn(&mut ReachabilityAtlas);
        let cases: &[(&str, Break, &str)] = &[
            (
                "bit width above the minimum",
                |a| a.chunks[0].bit_width = 1,
                "bit width is not the minimum",
            ),
            (
                "palette range past the palette",
                |a| a.chunks[0].palette_start = 1,
                "palette range is out of bounds",
            ),
            (
                "unsorted palette",
                |a| {
                    a.palette = Box::from([1, 0]);
                    a.indexes = vec![0; 512].into_boxed_slice();
                    a.chunks[0] = Chunk {
                        palette_start: 0,
                        palette_count: 2,
                        index_start: 0,
                        index_len: 512,
                        bit_width: 1,
                    };
                },
                "palette is not sorted unique",
            ),
            (
                "index range past the index table",
                |a| {
                    a.chunks[0] = Chunk {
                        palette_start: 0,
                        palette_count: 2,
                        index_start: 0,
                        index_len: 512,
                        bit_width: 1,
                    };
                    a.palette = Box::from([0, 1]);
                },
                "index range is out of bounds",
            ),
            (
                "uniform chunk with an index payload",
                |a| {
                    a.indexes = Box::from([0]);
                    a.chunks[0].index_len = 1;
                },
                "uniform atlas chunk must have empty index payload",
            ),
            (
                "inexact index length",
                |a| {
                    a.palette = Box::from([0, 1]);
                    a.indexes = vec![0; 512].into_boxed_slice();
                    a.chunks[0] = Chunk {
                        palette_start: 0,
                        palette_count: 2,
                        index_start: 0,
                        index_len: 511,
                        bit_width: 1,
                    };
                },
                "index length is not exact",
            ),
            (
                "zero-count paint run",
                |a| a.legacy_paint = Box::from([CellRun { first: 0, count: 0 }]),
                "atlas run count is zero",
            ),
            (
                "overlapping scatter runs",
                |a| {
                    a.scatter = Box::from([
                        CellRun { first: 0, count: 2 },
                        CellRun { first: 1, count: 1 },
                    ]);
                },
                "sorted disjoint maximal",
            ),
            (
                "unsorted coverage rows",
                |a| {
                    let tile = key();
                    a.coverage = Box::from([
                        CoverageRow { tile, reason: 3 },
                        CoverageRow { tile, reason: 2 },
                    ]);
                },
                "sorted unique tile/reason pairs",
            ),
            (
                "unknown coverage reason",
                |a| {
                    a.coverage = Box::from([CoverageRow {
                        tile: key(),
                        reason: 9,
                    }]);
                },
                "unknown reason",
            ),
            (
                "base map and size tables differ",
                |a| a.base_sizes = Box::from([]),
                "differ in length",
            ),
            (
                "movement offsets not from zero to the run count",
                |a| a.movement_offsets = Box::from([0, 0]),
                "movement CSR offsets",
            ),
            (
                "transport offsets of the wrong length",
                |a| a.transport_offsets = Box::from([0]),
                "transport CSR offsets",
            ),
            (
                "conditional offsets of the wrong length",
                |a| a.conditional_offsets = Box::from([0]),
                "conditional CSR offsets",
            ),
            (
                "empty closure run",
                |a| a.movement_runs = Box::from([ClosureRun { first: 0, count: 0 }]),
                "closure run is empty",
            ),
            (
                "placement angle past 3",
                |a| {
                    a.placements = Box::from([PlacementRow {
                        angle: 4,
                        ..placement()
                    }]);
                },
                "angle is outside 0..3",
            ),
            (
                "duplicate placements",
                |a| a.placements = Box::from([placement(), placement()]),
                "placements must be sorted unique",
            ),
            (
                "stand component past the base count",
                |a| a.stands = Box::from([StandRow { tile: key(), base: 5 }]),
                "stand component is out of range",
            ),
            (
                "duplicate stand tiles in one placement",
                |a| {
                    a.placements = Box::from([PlacementRow {
                        stand_count: 2,
                        ..placement()
                    }]);
                    a.stands = Box::from([
                        StandRow { tile: key(), base: 0 },
                        StandRow { tile: key(), base: 0 },
                    ]);
                },
                "placement stands must be sorted unique tiles",
            ),
            (
                "stand range past the stand table",
                |a| {
                    a.placements = Box::from([PlacementRow {
                        stand_count: 1,
                        ..placement()
                    }]);
                },
                "stand range is out of bounds",
            ),
            (
                "short chunk directory",
                |a| {
                    let mut chunks = a.chunks.to_vec();
                    chunks.pop();
                    a.chunks = chunks.into_boxed_slice();
                },
                "chunk directory count does not match geometry",
            ),
        ];
        for (name, break_it, wanted) in cases {
            let mut atlas = one_component();
            break_it(&mut atlas);
            let (produced, read) = verdicts(&atlas);
            let Err(produced_message) = &produced else {
                panic!("{name}: validate accepted a table the reader refuses ({read:?})");
            };
            assert!(produced_message.contains(wanted), "{name}: {produced_message}");
            assert_eq!(produced, read, "{name}");
        }
    }
}
