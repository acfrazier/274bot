use std::fs;
use std::path::{Path, PathBuf};

use super::LoadError;

pub(super) fn read_file(path: &Path) -> Result<Vec<u8>, LoadError> {
    fs::read(path).map_err(|source| LoadError::io(path, 0, source))
}

pub(super) fn read_utf8(path: &Path) -> Result<String, LoadError> {
    let bytes = read_file(path)?;
    String::from_utf8(bytes).map_err(|error| {
        LoadError::invalid(
            path,
            error.utf8_error().valid_up_to(),
            "source is not valid UTF-8",
        )
    })
}

/// Replace comments with spaces while preserving byte offsets and line boundaries.
pub(super) fn mask_comments(path: &Path, source: &str) -> Result<Vec<u8>, LoadError> {
    let mut bytes = source.as_bytes().to_vec();
    let mut index = 0;
    let mut block_start = None;
    let mut quote = None;
    let mut escaped = false;

    while index < bytes.len() {
        if block_start.is_some() {
            if bytes.get(index..index + 2) == Some(b"*/") {
                bytes[index] = b' ';
                bytes[index + 1] = b' ';
                index += 2;
                block_start = None;
            } else {
                if bytes[index] != b'\n' && bytes[index] != b'\r' {
                    bytes[index] = b' ';
                }
                index += 1;
            }
        } else if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if bytes[index] == b'\\' {
                escaped = true;
            } else if bytes[index] == delimiter {
                quote = None;
            }
            index += 1;
        } else if matches!(bytes[index], b'\'' | b'"' | b'`') {
            quote = Some(bytes[index]);
            index += 1;
        } else if bytes.get(index..index + 2) == Some(b"//") {
            while index < bytes.len() && bytes[index] != b'\n' {
                if bytes[index] != b'\r' {
                    bytes[index] = b' ';
                }
                index += 1;
            }
        } else if bytes.get(index..index + 2) == Some(b"/*") {
            block_start = Some(index);
            bytes[index] = b' ';
            bytes[index + 1] = b' ';
            index += 2;
        } else {
            index += 1;
        }
    }

    if let Some(offset) = block_start {
        return Err(LoadError::invalid(
            path,
            offset,
            "unterminated block comment",
        ));
    }
    Ok(bytes)
}

pub(super) struct Cursor<'a> {
    path: &'a Path,
    bytes: &'a [u8],
    base: usize,
    position: usize,
}

impl<'a> Cursor<'a> {
    pub(super) const fn new(path: &'a Path, bytes: &'a [u8], base: usize) -> Self {
        Self {
            path,
            bytes,
            base,
            position: 0,
        }
    }

    pub(super) const fn offset(&self) -> usize {
        self.base.saturating_add(self.position)
    }

    pub(super) const fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    pub(super) fn take(&mut self, length: usize) -> Result<&'a [u8], LoadError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| LoadError::invalid(self.path, self.offset(), "field length overflow"))?;
        if end > self.bytes.len() {
            return Err(LoadError::truncated(
                self.path,
                self.offset(),
                length,
                self.remaining(),
            ));
        }
        let start = self.position;
        self.position = end;
        Ok(&self.bytes[start..end])
    }

    pub(super) fn take_with_offset(
        &mut self,
        length: usize,
    ) -> Result<(usize, &'a [u8]), LoadError> {
        let offset = self.offset();
        let bytes = self.take(length)?;
        Ok((offset, bytes))
    }

    pub(super) fn skip(&mut self, length: usize) -> Result<(), LoadError> {
        self.take(length).map(|_| ())
    }

    pub(super) fn u8(&mut self) -> Result<u8, LoadError> {
        let bytes = self.take(1)?;
        Ok(bytes[0])
    }

    pub(super) fn u16(&mut self) -> Result<u16, LoadError> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub(super) fn i16(&mut self) -> Result<i16, LoadError> {
        let bytes = self.take(2)?;
        Ok(i16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub(super) fn u24(&mut self) -> Result<u32, LoadError> {
        let bytes = self.take(3)?;
        Ok((u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]))
    }

    pub(super) fn i32(&mut self) -> Result<i32, LoadError> {
        let bytes = self.take(4)?;
        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(super) fn string(&mut self, terminator: u8) -> Result<Box<str>, LoadError> {
        let start = self.position;
        let Some(length) = self.bytes[start..]
            .iter()
            .position(|byte| *byte == terminator)
        else {
            return Err(LoadError::invalid(
                self.path,
                self.offset(),
                "unterminated string",
            ));
        };
        let end = start + length;
        let mut result = String::with_capacity(length);
        for byte in &self.bytes[start..end] {
            result.push(char::from(*byte));
        }
        self.position = end + 1;
        Ok(result.into_boxed_str())
    }

    pub(super) fn finish(self) -> Result<(), LoadError> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(LoadError::invalid(
                self.path,
                self.offset(),
                format!("{} trailing bytes", self.remaining()).into_boxed_str(),
            ))
        }
    }

    pub(super) fn invalid(&self, message: impl Into<Box<str>>) -> LoadError {
        LoadError::invalid(self.path, self.offset(), message)
    }
    pub(super) fn invalid_at(&self, offset: usize, message: impl Into<Box<str>>) -> LoadError {
        LoadError::invalid(self.path, offset, message)
    }

    pub(super) fn unsupported_code(
        &self,
        pack: &'static str,
        code: u8,
        offset: usize,
    ) -> LoadError {
        LoadError::unsupported_code(self.path, offset, pack, code)
    }
}

pub(super) fn client_varbit_path() -> PathBuf {
    PathBuf::from("client/config/varbit.dat")
}
