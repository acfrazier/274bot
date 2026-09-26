//! Offset-only maps.bin access: u32le id, u32le length, raw payload. Never slurp
//! the archive or retain all placements. B decodes one record and emits POI
//! candidates through its visitor; D keeps the prepared-cache Arc alive.
use super::MapError;
use std::io::{Read, Seek, SeekFrom};

pub const MAX_MAP_RECORD_BYTES: usize = 256 * 1024;
pub const MAX_MAP_RECORDS: usize = 65_536;

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
impl MapRecordIndex {
    /// Scans eight-byte headers, seeking over payloads. Size/count/extent checks
    /// precede payload allocation; records may be unordered on disk.
    pub fn read(input: &mut (impl Read + Seek)) -> Result<Self, MapError> {
        let length = input.seek(SeekFrom::End(0))?;
        input.seek(SeekFrom::Start(0))?;
        let mut records = Vec::new();
        let mut position = 0u64;
        while position < length {
            if length - position < 8 {
                return Err(MapError::Truncated);
            }
            if records.len() == MAX_MAP_RECORDS {
                return Err(MapError::Limit("maps.bin record count"));
            }
            let mut header = [0; 8];
            input.read_exact(&mut header)?;
            let id = u32::from_le_bytes(header[..4].try_into().unwrap());
            let bytes = u32::from_le_bytes(header[4..].try_into().unwrap());
            if bytes == 0 {
                return Err(MapError::Invalid("empty map record"));
            }
            if bytes as usize > MAX_MAP_RECORD_BYTES {
                return Err(MapError::Limit("map record bytes"));
            }
            let offset = position + 8;
            let end = offset
                .checked_add(u64::from(bytes))
                .ok_or(MapError::Limit("map record extent"))?;
            if end > length {
                return Err(MapError::Truncated);
            }
            records.push(RecordOffset { id, offset, bytes });
            input.seek(SeekFrom::Start(end))?;
            position = end;
        }
        records.sort_unstable_by_key(|record| record.id);
        if records.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err(MapError::Duplicate("map record id"));
        }
        Ok(Self { records })
    }
    pub fn records(&self) -> &[RecordOffset] {
        &self.records
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
