//! Revision-377 LocDefs built with the engine's server-then-client LocType decode.
//!
//! The server engine initializes each LocType from `server/loc.dat`, overlays
//! `client/config/loc.dat`, and runs `postDecode`. The 377 client decoder has a
//! different `breakroutefinding` postpass, so its final `blockwalk` and
//! `blockrange` values are not collision authority.

use api::obj_names::LocDefs;
use client::config::config_377::{DecodeError377, DecodeErrorKind377, Reader377};
use client::config::LocType;

/// Build collision definitions with the exact engine field defaults, opcode
/// order, field values, and active postpass; leave client `loc_types` intact
/// for navpois. The engine's server count bounds the LocType table it exposes.
pub(crate) fn from_server_and_client(
    loc_types: &mut [LocType],
    server_dat: &[u8],
    client_dat: Option<&[u8]>,
) -> Result<LocDefs, DecodeError377> {
    let mut server = Reader377::new(server_dat, "server/loc.dat", 0);
    let server_count = server.g2()? as usize;
    let Some(client_dat) = client_dat else {
        return Err(DecodeError377 {
            table: "loc.dat",
            record: 0,
            offset: 0,
            kind: DecodeErrorKind377::MissingMember,
        });
    };
    let mut client = Reader377::new(client_dat, "loc.dat", 0);
    let client_count = client.g2()? as usize;
    if client_count != loc_types.len() {
        return Err(client.error(DecodeErrorKind377::InvalidValue(
            "client loc.dat count does not match decoded cache table",
        )));
    }
    if server_count > loc_types.len() {
        return Err(server.error(DecodeErrorKind377::InvalidValue(
            "server loc.dat count exceeds decoded client table",
        )));
    }

    let mut records = Vec::with_capacity(server_count);
    for id in 0..server_count {
        server.record = id;
        client.record = id;
        let mut fields = EngineLocFields::default();
        decode_loc_record(&mut server, &mut fields)?;
        decode_loc_record(&mut client, &mut fields)?;
        let Some(loc_type) = loc_types.get(id) else {
            return Err(server.error(DecodeErrorKind377::InvalidValue(
                "server loc id is outside decoded client table",
            )));
        };
        if loc_type.id != id as i32 {
            return Err(server.error(DecodeErrorKind377::InvalidValue(
                "decoded client loc id does not match server table ordinal",
            )));
        }
        records.push(LocCollisionRecord {
            engine: fields.collision_fields(),
            client: CollisionFields::from(loc_type),
        });
    }

    for (loc_type, record) in loc_types.iter_mut().zip(&records) {
        apply_collision_fields(loc_type, record.engine);
    }
    let definitions = LocDefs::from_locs(&loc_types[..server_count]);
    for (loc_type, record) in loc_types.iter_mut().zip(&records) {
        apply_collision_fields(loc_type, record.client);
    }
    Ok(definitions)
}

#[derive(Clone, Copy)]
struct LocCollisionRecord {
    engine: CollisionFields,
    client: CollisionFields,
}

#[derive(Clone, Copy)]
struct CollisionFields {
    width: i32,
    length: i32,
    block_walk: bool,
    block_range: bool,
    active: bool,
}

impl CollisionFields {
    fn from(loc_type: &LocType) -> Self {
        Self {
            width: loc_type.width,
            length: loc_type.length,
            block_walk: loc_type.blockwalk,
            block_range: loc_type.blockrange,
            active: loc_type.active,
        }
    }
}

fn apply_collision_fields(loc_type: &mut LocType, fields: CollisionFields) {
    loc_type.width = fields.width;
    loc_type.length = fields.length;
    loc_type.blockwalk = fields.block_walk;
    loc_type.blockrange = fields.block_range;
    loc_type.active = fields.active;
}

struct EngineLocFields {
    width: i32,
    length: i32,
    block_walk: bool,
    block_range: bool,
    active: i32,
    models_present: bool,
    shapes_present: bool,
    first_shape: Option<i32>,
    actions_present: bool,
}

impl Default for EngineLocFields {
    fn default() -> Self {
        Self {
            width: 1,
            length: 1,
            block_walk: true,
            block_range: true,
            active: -1,
            models_present: false,
            shapes_present: false,
            first_shape: None,
            actions_present: false,
        }
    }
}

impl EngineLocFields {
    fn collision_fields(&self) -> CollisionFields {
        let active = if self.active != -1 {
            self.active != 0
        } else {
            (self.models_present && (!self.shapes_present || self.first_shape == Some(10)))
                || self.actions_present
        };
        CollisionFields {
            width: self.width,
            length: self.length,
            block_walk: self.block_walk,
            block_range: self.block_range,
            active,
        }
    }
}

fn decode_loc_record(
    reader: &mut Reader377<'_>,
    fields: &mut EngineLocFields,
) -> Result<(), DecodeError377> {
    loop {
        let opcode = reader.g1()?;
        if opcode == 0 {
            return Ok(());
        }
        match opcode {
            1 => {
                let count = reader.g1()? as usize;
                fields.models_present = true;
                fields.shapes_present = true;
                fields.first_shape = None;
                for index in 0..count {
                    reader.g2()?;
                    let shape = reader.g1()?;
                    if index == 0 {
                        fields.first_shape = Some(shape);
                    }
                }
            }
            2 | 3 | 250 => skip_jag_string(reader)?,
            30..=34 => {
                fields.actions_present = true;
                skip_jag_string(reader)?;
            }
            5 => {
                let count = reader.g1()? as usize;
                fields.models_present = true;
                fields.shapes_present = false;
                fields.first_shape = None;
                reader.take(count * 2)?;
            }
            14 => fields.width = reader.g1()?,
            15 => fields.length = reader.g1()?,
            17 => fields.block_walk = false,
            18 => fields.block_range = false,
            19 => fields.active = reader.g1()?,
            21 | 22 | 23 | 25 | 62 | 64 | 73 | 74 => {}
            24 | 60 | 61 | 65..=68 => {
                reader.g2()?;
            }
            28 | 69 | 75 => {
                reader.g1()?;
            }
            29 | 39 => {
                reader.g1b()?;
            }
            40 => {
                let count = reader.g1()? as usize;
                reader.take(count * 4)?;
            }
            70..=72 => {
                reader.g2b()?;
            }
            77 => {
                reader.g2()?;
                reader.g2()?;
                let count = reader.g1()? as usize + 1;
                reader.take(count * 2)?;
            }
            249 => {
                let count = reader.g1()? as usize;
                for _ in 0..count {
                    reader.g3()?;
                    if reader.g1()? == 1 {
                        skip_jag_string(reader)?;
                    } else {
                        reader.take(4)?;
                    }
                }
            }
            _ => return Err(reader.unknown(opcode)),
        }
    }
}

fn skip_jag_string(reader: &mut Reader377<'_>) -> Result<(), DecodeError377> {
    let length = reader.data[reader.pos..]
        .iter()
        .position(|&byte| byte == 10)
        .ok_or_else(|| reader.error(DecodeErrorKind377::UnexpectedEof))?;
    reader.take(length + 1)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::bake_from_maps;
    use client::dash3d::CollisionFlag;
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    struct FixtureDir(PathBuf);

    impl FixtureDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "274bot-nav-377-collision-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for FixtureDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn loc_dat(count: usize, overrides: &[(usize, &[u8])]) -> Vec<u8> {
        let mut records = vec![&[][..]; count];
        for &(id, record) in overrides {
            records[id] = record;
        }
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&u16::try_from(count).unwrap().to_be_bytes());
        for record in records {
            bytes.extend_from_slice(record);
            bytes.push(0);
        }
        bytes
    }

    fn client_loc_types(count: usize) -> Vec<LocType> {
        (0..count)
            .map(|id| LocType {
                id: id as i32,
                ..LocType::default()
            })
            .collect()
    }

    #[test]
    fn server_377_locdefs_drive_collision_bake_for_3550_and_6950_to_6953() {
        const COUNT: usize = 6_956;
        let server = loc_dat(
            COUNT,
            &[
                (3550, &[18, 74]),
                (6950, &[74]),
                (6951, &[74]),
                (6952, &[74]),
                (6953, &[1, 1, 0, 1, 10]),
                (6954, &[14, 2, 15, 3, 1, 1, 0, 1, 10]),
                (6955, &[1, 1, 0, 1, 10]),
            ],
        );
        let client = loc_dat(
            COUNT,
            &[
                (3550, &[18, 74]),
                (6950, &[74]),
                (6951, &[74]),
                (6952, &[74]),
                (6953, &[74, 19, 0]),
                (6954, &[14, 3, 15, 4, 19, 2, 74]),
                (6955, &[74]),
            ],
        );
        let mut client_locs = client_loc_types(COUNT);
        for id in [3550, 6950, 6951, 6952, 6953, 6954, 6955] {
            client_locs[id].blockwalk = false;
            client_locs[id].blockrange = false;
        }
        client_locs[6954].width = 3;
        client_locs[6954].length = 4;
        client_locs[6954].active = false;
        client_locs[6954].name = "client-side name".into();
        client_locs[6954].op = vec![Some("Examine".into())];
        client_locs[6954].forceapproach = 2;
        client_locs[6955].active = true;

        let loc_defs = from_server_and_client(&mut client_locs, &server, Some(&client)).unwrap();
        let wall = loc_defs.loc(3550).unwrap();
        assert!(wall.block_walk);
        assert!(!wall.block_range);
        for id in 6950..=6952 {
            let def = loc_defs.loc(id).unwrap();
            assert!(def.block_walk, "loc {id}");
            assert!(def.block_range, "loc {id}");
        }
        let explicit_inactive = loc_defs.loc(6953).unwrap();
        assert!(explicit_inactive.block_walk);
        assert!(explicit_inactive.block_range);
        assert!(!explicit_inactive.active);
        let overridden = loc_defs.loc(6954).unwrap();
        assert_eq!((overridden.width, overridden.length), (3, 4));
        assert!(overridden.block_walk);
        assert!(overridden.block_range);
        assert!(overridden.active, "server opcode 19 uses engine truthiness");
        assert_eq!(overridden.name.as_deref(), Some("client-side name"));
        assert_eq!(overridden.ops.len(), 1);
        assert_eq!(overridden.ops[0], "Examine");
        assert_eq!(overridden.force_approach, 2);
        assert!(
            loc_defs.loc(6955).unwrap().active,
            "model shape 10 defaults active"
        );

        // The separately returned client table still carries its client-decoded
        // collision values; only LocDefs receives the engine's source composition.
        assert!(!client_locs[6950].blockwalk);
        assert!(!client_locs[6950].blockrange);
        assert!(!client_locs[6954].active);
        assert_eq!((client_locs[6954].width, client_locs[6954].length), (3, 4));

        let fixture = FixtureDir::new("bake-seam");
        let map = "==== MAP ====\n==== LOC ====\n\
0 2 2: 3550 0 0\n\
0 10 10: 6950 10 0\n\
0 15 10: 6951 10 0\n\
0 20 10: 6952 10 0\n\
0 25 10: 6953 10 0\n\
0 30 10: 6954 10 0\n\
0 40 10: 6955 22 0\n";
        fs::write(fixture.0.join("m0_0.jm2"), map).unwrap();
        let collision = bake_from_maps(&fixture.0, &loc_defs, &HashSet::new()).unwrap();
        assert_ne!(
            collision.flag(2, 2, 0) & CollisionFlag::W_W as u32,
            0,
            "3550 is a walk-blocking wall even though its client breakroutefinding postpass clears blockwalk"
        );
        assert_eq!(collision.flag(2, 2, 0) & CollisionFlag::V_W as u32, 0);
        for (x, z) in [(10, 10), (15, 10), (20, 10), (25, 10)] {
            let flag = collision.flag(x, z, 0);
            assert_ne!(flag & CollisionFlag::WALK_SCENERY as u32, 0);
            assert_ne!(flag & CollisionFlag::VIS_SCENERY as u32, 0);
        }
        assert_ne!(
            collision.flag(30, 10, 0) & CollisionFlag::WALK_SCENERY as u32,
            0
        );
        assert_ne!(
            collision.flag(32, 13, 0) & CollisionFlag::WALK_SCENERY as u32,
            0
        );
        assert_eq!(
            collision.flag(33, 13, 0) & CollisionFlag::WALK_SCENERY as u32,
            0
        );
        assert_ne!(collision.flag(40, 10, 0) & CollisionFlag::WR_GRND as u32, 0);
    }

    #[test]
    fn engine_loc_decoder_consumes_supported_noncollision_payloads() {
        let server = loc_dat(
            2,
            &[
                (
                    0,
                    &[
                        1, 1, 0, 1, 10, 2, b'n', 10, 3, b'd', 10, 5, 1, 0, 2, 14, 2, 15, 3, 17, 18,
                        19, 1, 21, 22, 23, 24, 0, 1, 25, 28, 16, 29, 255, 39, 255, 30, b'h', 10,
                        40, 1, 0, 1, 0, 2, 60, 0, 1, 61, 0, 2, 62, 64, 65, 0, 128, 66, 0, 128, 67,
                        0, 128, 68, 0, 3, 69, 4, 70, 0, 1, 71, 0, 2, 72, 0, 3, 73, 74, 75, 1, 77,
                        0, 1, 0, 2, 1, 0, 1, 0, 3, 249, 2, 0, 0, 1, 1, b's', 10, 0, 0, 2, 0, 0, 0,
                        0, 42, 250, b'n', 10,
                    ],
                ),
                (1, &[14, 6, 18]),
            ],
        );
        let client = loc_dat(2, &[(0, &[14, 4, 15, 5, 19, 2]), (1, &[14, 7])]);
        let mut locs = client_loc_types(2);
        let definitions = from_server_and_client(&mut locs, &server, Some(&client)).unwrap();
        let first = definitions.loc(0).unwrap();
        assert_eq!((first.width, first.length), (4, 5));
        assert!(!first.block_walk);
        assert!(!first.block_range);
        assert!(first.active);
        let second = definitions.loc(1).unwrap();
        assert_eq!(second.width, 7);
        assert!(second.block_walk);
        assert!(!second.block_range);
    }

    #[test]
    fn engine_active_default_uses_actions_and_model_shape_presence() {
        let server = loc_dat(
            4,
            &[
                (0, &[30, b'a', 10]),
                (1, &[5, 0]),
                (2, &[1, 0]),
                (3, &[1, 1, 0, 1, 10]),
            ],
        );
        let client = loc_dat(4, &[(1, &[1, 0]), (2, &[5, 0])]);
        let mut locs = client_loc_types(4);
        let definitions = from_server_and_client(&mut locs, &server, Some(&client)).unwrap();

        assert!(
            definitions.loc(0).unwrap().active,
            "any action opcode allocates the engine's op array and activates by default"
        );
        assert!(
            !definitions.loc(1).unwrap().active,
            "an empty opcode-1 model list still has an empty shapes array"
        );
        assert!(
            definitions.loc(2).unwrap().active,
            "an empty opcode-5 model list has null shapes"
        );
        assert!(
            definitions.loc(3).unwrap().active,
            "opcode-1 first model shape 10 activates by default"
        );
    }
}
