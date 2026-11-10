use std::collections::BTreeMap;
use std::path::Path;

use crate::{LocTypeId, VarpId};

use super::bytes::{self, Cursor};
use super::{ConfigValue, Enum, LoadError, LocConfig, Packs, Param, Varbit, Varp};

const INT_TYPE: u8 = b'i';
const STRING_TYPE: u8 = b's';

pub(super) fn load(server_dir: &Path, client_varbits: &[u8]) -> Result<Packs, LoadError> {
    let params = decode_file(server_dir, "param.dat", decode_param)?;
    let varps = decode_file(server_dir, "varp.dat", decode_varp)?;
    let varbits = decode_varbits(server_dir, client_varbits)?;
    let varns = decode_file(server_dir, "varn.dat", |cursor| {
        decode_var_string(cursor, "varn")
    })?;
    let varss = decode_file(server_dir, "vars.dat", |cursor| {
        decode_var_string(cursor, "vars")
    })?;
    let invs = decode_file(server_dir, "inv.dat", decode_inv)?;
    let enums = decode_file(server_dir, "enum.dat", decode_enum)?;
    let locs = decode_file(server_dir, "loc.dat", decode_loc)?;

    Ok(Packs {
        params: index_i32(params),
        varps: varps
            .into_iter()
            .enumerate()
            .map(|(id, value)| (VarpId(id as u16), value))
            .collect(),
        varbits: varbits
            .into_iter()
            .enumerate()
            .map(|(id, value)| (id as u16, value))
            .collect(),
        varns: varns
            .into_iter()
            .enumerate()
            .map(|(id, value)| (id as u16, value))
            .collect(),
        varss: varss
            .into_iter()
            .enumerate()
            .map(|(id, value)| (id as u16, value))
            .collect(),
        invs: index_i32(invs),
        enums: index_i32(enums),
        locs: locs
            .into_iter()
            .enumerate()
            .map(|(id, value)| (LocTypeId(id as i32), value))
            .collect(),
    })
}

fn decode_file<T>(
    server_dir: &Path,
    filename: &str,
    decode: impl FnMut(&mut Cursor<'_>) -> Result<T, LoadError>,
) -> Result<Vec<T>, LoadError> {
    let path = server_dir.join(filename);
    let bytes = bytes::read_file(&path)?;
    decode_records(&path, &bytes, decode)
}

// Record counts and code-0 terminators follow src/cache/config/ConfigType.ts:14-23.
fn decode_records<T>(
    path: &Path,
    bytes: &[u8],
    mut decode: impl FnMut(&mut Cursor<'_>) -> Result<T, LoadError>,
) -> Result<Vec<T>, LoadError> {
    let mut cursor = Cursor::new(path, bytes, 0);
    let count = cursor.u16()?;
    let mut records = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        records.push(decode(&mut cursor)?);
    }
    cursor.finish()?;
    Ok(records)
}

fn index_i32<T>(records: Vec<T>) -> BTreeMap<i32, T> {
    records
        .into_iter()
        .enumerate()
        .map(|(id, record)| (id as i32, record))
        .collect()
}

// Field layout mirrors src/cache/config/ParamType.ts:66-80 and src/cache/config/ParamHelper.ts:26-39.
fn decode_param(cursor: &mut Cursor<'_>) -> Result<Param, LoadError> {
    let mut type_code = INT_TYPE;
    let mut int_default = -1;
    let mut string_default = None;
    loop {
        let code_offset = cursor.offset();
        match cursor.u8()? {
            0 => break,
            1 => type_code = cursor.u8()?,
            2 => int_default = cursor.i32()?,
            4 => {}
            5 => string_default = Some(cursor.string(10)?),
            250 => {
                let _debug_name = cursor.string(10)?;
            }
            code => return Err(cursor.unsupported_code("param", code, code_offset)),
        }
    }
    Ok(Param {
        string: type_code == STRING_TYPE,
        int_default,
        string_default: match string_default {
            Some(value) => value,
            None => "null".into(),
        },
    })
}

// Field layout mirrors src/cache/config/VarPlayerType.ts:88-104.
fn decode_varp(cursor: &mut Cursor<'_>) -> Result<Varp, LoadError> {
    let mut type_code = INT_TYPE;
    let mut protect = true;
    let mut transmit = false;
    let mut name = String::new().into_boxed_str();
    loop {
        let code_offset = cursor.offset();
        match cursor.u8()? {
            0 => break,
            1 => {
                let _scope = cursor.u8()?;
            }
            2 => type_code = cursor.u8()?,
            4 => protect = false,
            5 => {
                let _client_code = cursor.u16()?;
            }
            6 => transmit = true,
            250 => name = cursor.string(10)?,
            code => return Err(cursor.unsupported_code("varp", code, code_offset)),
        }
    }
    Ok(Varp {
        string: type_code == STRING_TYPE,
        protect,
        transmit,
        name,
    })
}

// Field layout mirrors src/cache/config/VarNpcType.ts:65-73 and src/cache/config/VarSharedType.ts:64-76.
fn decode_var_string(cursor: &mut Cursor<'_>, pack: &'static str) -> Result<bool, LoadError> {
    let mut type_code = INT_TYPE;
    loop {
        let code_offset = cursor.offset();
        match cursor.u8()? {
            0 => break,
            1 => type_code = cursor.u8()?,
            250 => {
                let _debug_name = cursor.string(10)?;
            }
            code => return Err(cursor.unsupported_code(pack, code, code_offset)),
        }
    }
    Ok(type_code == STRING_TYPE)
}

fn decode_varbits(server_dir: &Path, client_bytes: &[u8]) -> Result<Vec<Varbit>, LoadError> {
    let server_path = server_dir.join("varbit.dat");
    let server_bytes = bytes::read_file(&server_path)?;
    decode_varbit_bytes(&server_path, &server_bytes, client_bytes)
}

// The two records correspond to src/cache/config/VarBitType.ts:23-42,72-82.
fn decode_varbit_bytes(
    server_path: &Path,
    server_bytes: &[u8],
    client_bytes: &[u8],
) -> Result<Vec<Varbit>, LoadError> {
    let client_path = bytes::client_varbit_path();
    let mut server = Cursor::new(server_path, server_bytes, 0);
    let mut client = Cursor::new(&client_path, client_bytes, 0);
    let server_count = server.u16()?;
    let client_count = client.u16()?;
    if server_count != client_count {
        return Err(LoadError::invalid(
            &client_path,
            0,
            format!(
                "client varbit count {client_count} does not match server count {server_count}"
            ),
        ));
    }

    let mut varbits = Vec::with_capacity(usize::from(server_count));
    for _ in 0..server_count {
        let name = decode_varbit_server_record(&mut server)?;
        let (base, lo, hi) = decode_varbit_client_record(&mut client)?;
        varbits.push(Varbit {
            base: VarpId(base),
            lo,
            hi,
            name,
        });
    }
    server.finish()?;
    client.finish()?;
    Ok(varbits)
}

fn decode_varbit_server_record(cursor: &mut Cursor<'_>) -> Result<Box<str>, LoadError> {
    let mut name = String::new().into_boxed_str();
    loop {
        let code_offset = cursor.offset();
        match cursor.u8()? {
            0 => return Ok(name),
            250 => name = cursor.string(10)?,
            code => return Err(cursor.unsupported_code("server varbit", code, code_offset)),
        }
    }
}

fn decode_varbit_client_record(cursor: &mut Cursor<'_>) -> Result<(u16, u8, u8), LoadError> {
    let mut fields = None;
    loop {
        let code_offset = cursor.offset();
        match cursor.u8()? {
            0 => break,
            1 => {
                let base = cursor.u16()?;
                let range_offset = cursor.offset();
                let lo = cursor.u8()?;
                let hi = cursor.u8()?;
                if lo > hi || hi >= 32 {
                    return Err(cursor.invalid_at(
                        range_offset,
                        format!("invalid varbit bit range {lo}..={hi}"),
                    ));
                }
                fields = Some((base, lo, hi));
            }
            code => return Err(cursor.unsupported_code("client varbit", code, code_offset)),
        }
    }
    fields.ok_or_else(|| cursor.invalid("client varbit record has no code 1 range"))
}

// Field layout mirrors src/cache/config/InvType.ts:84-118.
fn decode_inv(cursor: &mut Cursor<'_>) -> Result<Box<str>, LoadError> {
    let mut name = String::new().into_boxed_str();
    loop {
        let code_offset = cursor.offset();
        match cursor.u8()? {
            0 => return Ok(name),
            1 => {
                let _scope = cursor.u8()?;
            }
            2 => {
                let _size = cursor.u16()?;
            }
            3 | 5 | 6 | 7 | 8 | 9 => {}
            4 => {
                let count = cursor.u8()?;
                for _ in 0..count {
                    let _stock_obj = cursor.u16()?;
                    let _stock_count = cursor.u16()?;
                    let _stock_rate = cursor.i32()?;
                }
            }
            250 => name = cursor.string(10)?,
            code => return Err(cursor.unsupported_code("inv", code, code_offset)),
        }
    }
}

// Field layout mirrors src/cache/config/EnumType.ts:68-94.
fn decode_enum(cursor: &mut Cursor<'_>) -> Result<Enum, LoadError> {
    let mut input = INT_TYPE;
    let mut output = INT_TYPE;
    let mut int_default = 0;
    let mut string_default = "null".into();
    let mut values = BTreeMap::new();
    loop {
        let code_offset = cursor.offset();
        match cursor.u8()? {
            0 => break,
            1 => input = cursor.u8()?,
            2 => output = cursor.u8()?,
            3 => string_default = cursor.string(10)?,
            4 => int_default = cursor.i32()?,
            5 => {
                let count = cursor.u16()?;
                for _ in 0..count {
                    values.insert(cursor.i32()?, ConfigValue::Text(cursor.string(10)?));
                }
            }
            6 => {
                let count = cursor.u16()?;
                for _ in 0..count {
                    values.insert(cursor.i32()?, ConfigValue::Int(cursor.i32()?));
                }
            }
            250 => {
                let _debug_name = cursor.string(10)?;
            }
            code => return Err(cursor.unsupported_code("enum", code, code_offset)),
        }
    }
    Ok(Enum {
        input,
        output,
        int_default,
        string_default,
        values,
    })
}

// The supported server loc fields are codes 61, 249 and 250 (src/cache/config/LocType.ts:178-179,217-220).
fn decode_loc(cursor: &mut Cursor<'_>) -> Result<LocConfig, LoadError> {
    let mut config = LocConfig::default();
    loop {
        let code_offset = cursor.offset();
        match cursor.u8()? {
            0 => return Ok(config),
            61 => config.category = Some(i32::from(cursor.u16()?)),
            249 => config.params = decode_params(cursor)?,
            250 => config.name = cursor.string(10)?,
            code => return Err(cursor.unsupported_code("loc", code, code_offset)),
        }
    }
}

// Code 249 uses src/cache/config/ParamHelper.ts:26-39.
fn decode_params(cursor: &mut Cursor<'_>) -> Result<BTreeMap<i32, ConfigValue>, LoadError> {
    let count = cursor.u8()?;
    let mut params = BTreeMap::new();
    for _ in 0..count {
        let key = cursor.u24()? as i32;
        let string = cursor.u8()? == 1;
        let value = if string {
            ConfigValue::Text(cursor.string(10)?)
        } else {
            ConfigValue::Int(cursor.i32()?)
        };
        params.insert(key, value);
    }
    Ok(params)
}
