use api::selected::FactStrings;
use api::snapshot::WorldTile;

use crate::router::AvoidRect;
use crate::transport::WildernessRules;
use crate::zones::{Zone, ZoneClass, ZoneGroup, ZoneKind, ZoneTable, NO_GROUP, NO_SHAPE};

use super::{read_i32, read_key, read_u16, read_u32, read_u64, read_u8, PackError, PackRead};

pub const MAX_PACKED_INDEX: usize = 32_767;

pub(super) fn read_table<R: PackRead>(
    r: &mut R,
    origin: WorldTile,
    cols: u32,
    rows: u32,
    wilderness: &WildernessRules,
    keys: &mut FactStrings,
) -> Result<Option<ZoneTable>, PackError> {
    let kind_count = read_count(r, u16::MAX as usize, 15, "zone kind")?;
    let mut kinds = Vec::with_capacity(kind_count);
    for _ in 0..kind_count {
        let npc_id = read_i32(r)?;
        let vislevel = read_u16(r)?;
        let flags = read_u8(r)?;
        if npc_id < -1 || flags & !0b11 != 0 {
            return Err(PackError::BadLength(
                "invalid zone kind identity or flags".into(),
            ));
        }
        let id = read_key(r, keys)?;
        let label = read_key(r, keys)?;
        kinds.push(ZoneKind::new(
            id.0,
            label.0,
            npc_id,
            vislevel,
            flags & 1 != 0,
            flags & 2 != 0,
        ));
    }

    let zone_count = read_count(r, MAX_PACKED_INDEX, 14, "zone")?;
    let mut zones = Vec::with_capacity(zone_count);
    let mut east_extents = Vec::with_capacity(zone_count);
    for _ in 0..zone_count {
        match read_u8(r)? {
            0 => {
                let spawn_x = read_i32(r)?;
                let spawn_z = read_i32(r)?;
                let east_extent = read_u8(r)?;
                let level = read_u8(r)?;
                let class = read_class(read_u8(r)?)?;
                let kind = read_u16(r)?;
                if level > 3 {
                    return Err(PackError::BadLength(
                        "NPC zone level is out of range".into(),
                    ));
                }
                let kind_row = kinds.get(usize::from(kind)).ok_or_else(|| {
                    PackError::BadLength("zone kind index is out of range".into())
                })?;
                if kind_row.npc_id < 0 {
                    return Err(PackError::BadLength("NPC zone kind has npc_id -1".into()));
                }
                let cap = match class {
                    ZoneClass::Always => u16::MAX,
                    ZoneClass::LevelRule => kind_row
                        .vislevel
                        .checked_mul(2)
                        .ok_or_else(|| PackError::BadLength("NPC zone cap overflows u16".into()))?,
                };
                zones.push(Zone::npc(
                    WorldTile {
                        x: spawn_x,
                        z: spawn_z,
                        level: i32::from(level),
                    },
                    east_extent,
                    class,
                    cap,
                    kind,
                ));
                east_extents.push(east_extent);
            }
            1 => {
                let rect = AvoidRect {
                    min_x: read_i32(r)?,
                    max_x: read_i32(r)?,
                    min_z: read_i32(r)?,
                    max_z: read_i32(r)?,
                    level: None,
                };
                let level = read_u8(r)?;
                let class = read_class(read_u8(r)?)?;
                let kind = read_u16(r)?;
                if level > 3 || class != ZoneClass::Always {
                    return Err(PackError::BadLength(
                        "invalid hazard zone class or level".into(),
                    ));
                }
                let kind_row = kinds.get(usize::from(kind)).ok_or_else(|| {
                    PackError::BadLength("zone kind index is out of range".into())
                })?;
                if kind_row.npc_id != -1 {
                    return Err(PackError::BadLength(
                        "hazard zone kind does not have npc_id -1".into(),
                    ));
                }
                zones.push(Zone::hazard(rect, level, kind));
                east_extents.push(0);
            }
            tag => {
                return Err(PackError::BadLength(format!(
                    "zone tag {tag} is not 0 or 1"
                )));
            }
        }
    }

    let group_count = read_count(r, MAX_PACKED_INDEX, 29, "zone group")?;
    let mut groups = Vec::with_capacity(group_count);
    for group_index in 0..group_count {
        let id = read_key(r, keys)?;
        let label = read_key(r, keys)?;
        let rect = AvoidRect {
            min_x: read_i32(r)?,
            max_x: read_i32(r)?,
            min_z: read_i32(r)?,
            max_z: read_i32(r)?,
            level: match read_u8(r)? as i8 {
                -1 => None,
                level @ 0..=3 => Some(i32::from(level)),
                level => {
                    return Err(PackError::BadLength(format!(
                        "zone group level {level} is invalid"
                    )))
                }
            },
        };
        let member_count = read_count(r, zone_count, 2, "zone group member")?;
        let mut members = Vec::with_capacity(member_count);
        for _ in 0..member_count {
            let member = read_u16(r)?;
            let zone = zones
                .get_mut(usize::from(member))
                .ok_or_else(|| PackError::BadLength("zone group member is out of range".into()))?;
            if zone.group != NO_GROUP {
                return Err(PackError::BadLength(
                    "zone belongs to more than one group".into(),
                ));
            }
            zone.group = u16::try_from(group_index).map_err(|_| {
                PackError::BadLength("zone group index exceeds packed limit".into())
            })?;
            members.push(member);
        }
        groups.push(ZoneGroup::new(
            id.0,
            label.0,
            rect,
            members.into_boxed_slice(),
        ));
    }

    let carve_count = read_count(r, u32::MAX as usize, 18, "zone carve")?;
    let mut carves = Vec::with_capacity(carve_count);
    for _ in 0..carve_count {
        let zone_index = read_u16(r)?;
        let zone = zones
            .get(usize::from(zone_index))
            .ok_or_else(|| PackError::BadLength("carve zone index is out of range".into()))?;
        carves.push((
            zone_index,
            AvoidRect {
                min_x: read_i32(r)?,
                max_x: read_i32(r)?,
                min_z: read_i32(r)?,
                max_z: read_i32(r)?,
                level: Some(i32::from(zone.level)),
            },
        ));
    }

    let shape_count = read_count(r, MAX_PACKED_INDEX, 11, "zone shape")?;
    if shape_count > zone_count {
        return Err(PackError::BadLength(
            "zone shape count exceeds zone count".into(),
        ));
    }
    let mut shapes = Vec::with_capacity(shape_count);
    for _ in 0..shape_count {
        let zone_index = read_u16(r)?;
        let north_extent = read_u8(r)?;
        let bits = read_u64(r)?;
        let zone = zones
            .get_mut(usize::from(zone_index))
            .ok_or_else(|| PackError::BadLength("shape zone index is out of range".into()))?;
        let kind = kinds
            .get(usize::from(zone.kind))
            .ok_or_else(|| PackError::BadLength("shape kind index is out of range".into()))?;
        if kind.npc_id < 0 || zone.shape != NO_SHAPE {
            return Err(PackError::BadLength(
                "zone shape is duplicated or belongs to a hazard".into(),
            ));
        }
        let east_extent = *east_extents.get(usize::from(zone_index)).ok_or_else(|| {
            PackError::BadLength("shape east-extent index is out of range".into())
        })?;
        if !(1..=6).contains(&east_extent) || !(1..=6).contains(&north_extent) {
            return Err(PackError::BadLength(
                "zone shape footprint exceeds 8x8 bounds".into(),
            ));
        }
        zone.min_x = zone.spawn_x.saturating_sub(1);
        zone.min_z = zone.spawn_z.saturating_sub(1);
        zone.max_x = zone
            .spawn_x
            .checked_add(i32::from(east_extent))
            .ok_or_else(|| PackError::BadLength("zone shape bounds overflow x".into()))?;
        zone.max_z = zone
            .spawn_z
            .checked_add(i32::from(north_extent))
            .ok_or_else(|| PackError::BadLength("zone shape bounds overflow z".into()))?;
        zone.shape = u16::try_from(shapes.len())
            .map_err(|_| PackError::BadLength("zone shape index exceeds packed limit".into()))?;
        shapes.push(bits);
    }

    ZoneTable::from_parts(
        zones, kinds, groups, carves, shapes, origin, cols, rows, wilderness,
    )
    .map(Some)
    .map_err(|error| PackError::BadLength(format!("invalid zone table: {error}")))
}

fn read_count<R: PackRead>(
    r: &mut R,
    max: usize,
    min_row_bytes: usize,
    what: &str,
) -> Result<usize, PackError> {
    let count = usize::try_from(read_u32(r)?)
        .map_err(|_| PackError::BadLength(format!("{what} count exceeds usize")))?;
    if count > max || count > r.remaining() / min_row_bytes {
        return Err(PackError::BadLength(format!(
            "{what} count {count} exceeds its packed bound or remaining bytes"
        )));
    }
    Ok(count)
}

fn read_class(value: u8) -> Result<ZoneClass, PackError> {
    match value {
        0 => Ok(ZoneClass::Always),
        1 => Ok(ZoneClass::LevelRule),
        _ => Err(PackError::BadLength(format!(
            "zone class {value} is invalid"
        ))),
    }
}
