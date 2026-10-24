//! Offset-only maps.bin access: u32le id, u32le length, raw payload. Never slurp
//! the archive or retain all placements. B decodes one record and emits POI
//! candidates through its visitor; D keeps the prepared-cache Arc alive.
use super::MapError;
use std::io::{Read, Seek, SeekFrom};

pub const MAX_MAP_RECORD_BYTES: usize = 256 * 1024;
pub const MAX_MAP_RECORDS: usize = 65_536;
/// Every decoded placement consumes at least a one-byte position delta and
/// one-byte info byte; this is a conservative count bound for one 256 KiB row.
pub const MAX_MAP_LOC_PLACEMENTS_PER_RECORD: usize = MAX_MAP_RECORD_BYTES / 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordOffset {
    pub id: u32,
    pub offset: u64,
    pub bytes: u32,
}
#[derive(Debug)]
pub struct MapRecordIndex {
    records: Vec<RecordOffset>,
}

/// Incremental maps.bin header scan. One call to [`Self::step`] reads at most
/// one eight-byte header; the file handle belongs to the caller and is never
/// retained. The input is capped at [`MAX_MAP_RECORDS`] rows and
/// [`MAX_MAP_RECORD_BYTES`] per payload.
#[derive(Debug, Default)]
pub(crate) struct MapRecordIndexBuilder {
    length: Option<u64>,
    position: u64,
    records: Vec<RecordOffset>,
}

impl MapRecordIndexBuilder {
    pub(crate) fn is_complete(&self) -> bool {
        self.length.is_some_and(|length| self.position == length)
    }

    pub(crate) fn tracked_bytes(&self) -> usize {
        self.records.capacity() * std::mem::size_of::<RecordOffset>()
    }

    /// Read at most one index unit. The first call discovers file length; each
    /// following call validates and appends one record header.
    pub(crate) fn step(&mut self, input: &mut (impl Read + Seek)) -> Result<(), MapError> {
        if self.length.is_none() {
            let length = input.seek(SeekFrom::End(0))?;
            input.seek(SeekFrom::Start(0))?;
            self.length = Some(length);
            return Ok(());
        }
        let length = self.length.ok_or(MapError::Invalid("map record index"))?;
        if self.position == length {
            return Ok(());
        }
        if length - self.position < 8 {
            return Err(MapError::Truncated);
        }
        if self.records.len() == MAX_MAP_RECORDS {
            return Err(MapError::Limit("maps.bin record count"));
        }
        input.seek(SeekFrom::Start(self.position))?;
        let mut header = [0; 8];
        input.read_exact(&mut header)?;
        let [id_0, id_1, id_2, id_3, bytes_0, bytes_1, bytes_2, bytes_3] = header;
        let id = u32::from_le_bytes([id_0, id_1, id_2, id_3]);
        let bytes = u32::from_le_bytes([bytes_0, bytes_1, bytes_2, bytes_3]);
        if bytes == 0 {
            return Err(MapError::Invalid("empty map record"));
        }
        if bytes as usize > MAX_MAP_RECORD_BYTES {
            return Err(MapError::Limit("map record bytes"));
        }
        let offset = self.position + 8;
        let end = offset
            .checked_add(u64::from(bytes))
            .ok_or(MapError::Limit("map record extent"))?;
        if end > length {
            return Err(MapError::Truncated);
        }
        self.records.push(RecordOffset { id, offset, bytes });
        input.seek(SeekFrom::Start(end))?;
        self.position = end;
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<MapRecordIndex, MapError> {
        if !self.is_complete() {
            return Err(MapError::Invalid("incomplete map record index"));
        }
        self.records.sort_unstable_by_key(|record| record.id);
        if self.records.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err(MapError::Duplicate("map record id"));
        }
        Ok(MapRecordIndex {
            records: self.records,
        })
    }
}

impl MapRecordIndex {
    /// Synchronously drive the same one-header cursor used by cooperative
    /// producers. Records are sorted by id only after the bounded scan.
    pub fn read(input: &mut (impl Read + Seek)) -> Result<Self, MapError> {
        let mut builder = MapRecordIndexBuilder::default();
        while !builder.is_complete() {
            builder.step(input)?;
        }
        builder.finish()
    }
    pub fn records(&self) -> &[RecordOffset] {
        &self.records
    }
    pub(crate) fn tracked_bytes(&self) -> usize {
        self.records.capacity() * std::mem::size_of::<RecordOffset>()
    }
    pub fn get(&self, id: u32) -> Option<RecordOffset> {
        self.records
            .binary_search_by_key(&id, |record| record.id)
            .ok()
            .map(|index| self.records[index])
    }
    /// A reusable, one-record buffer. Missing IDs are absent, not fabricated
    /// empty/blocked map squares. No decompression or client World is involved.
    pub fn read_record(
        &self,
        input: &mut (impl Read + Seek),
        id: u32,
        buffer: &mut Vec<u8>,
    ) -> Result<bool, MapError> {
        let Some(record) = self.get(id) else {
            return Ok(false);
        };
        input.seek(SeekFrom::Start(record.offset))?;
        buffer.resize(record.bytes as usize, 0);
        input.read_exact(buffer).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                MapError::Truncated
            } else {
                e.into()
            }
        })?;
        Ok(true)
    }
}
