use api::selected::FactStrings;
use api::snapshot::WorldTile;

use crate::router::AvoidRect;
use crate::transport::WildernessRules;
use crate::zones::{Zone, ZoneClass, ZoneGroup, ZoneKind, ZoneTable, NO_GROUP, NO_SHAPE};

use super::{read_i32, read_key, read_u16, read_u32, read_u64, read_u8, PackError, PackRead};

const MAX_PACKED_INDEX: usize = 32_767;

pub(super) fn wire_size(table: Option<&ZoneTable>) -> Result<usize, PackError> {
    let Some(table) = table else {
        return Ok(20);
    };

    packed_count(table.kinds().len(), u16::MAX as usize, "zone kind")?;
    packed_count(table.zones().len(), MAX_PACKED_INDEX, "zone")?;
    packed_count(table.groups().len(), MAX_PACKED_INDEX, "zone group")?;
    packed_count(table.carves().len(), u32::MAX as usize, "zone carve")?;
    packed_count(table.shapes().len(), MAX_PACKED_INDEX, "zone shape")?;
    if table.shapes().len() > table.zones().len() {
        return Err(PackError::BadLength(
            "zone shape count exceeds zone count".into(),
        ));
    }

    let mut bytes = 20usize;
    for kind in table.kinds() {
        bytes = add_size(bytes, 7, "zone kind fields")?;
        bytes = add_size(bytes, string_size(&kind.id, "zone kind id")?, "zone kind")?;
        bytes = add_size(
            bytes,
            string_size(&kind.label, "zone kind label")?,
            "zone kind",
        )?;
    }
    let mut shaped_zones = 0usize;
    for zone in table.zones() {
        let kind = table
            .kinds()
            .get(usize::from(zone.kind))
            .ok_or_else(|| PackError::BadLength("zone kind index is out of range".into()))?;
        if zone.level > 3 {
            return Err(PackError::BadLength("zone level is out of range".into()));
        }
        if zone.shape == NO_SHAPE {
            if kind.npc_id < 0 {
                if kind.npc_id != -1 || zone.class != ZoneClass::Always {
                    return Err(PackError::BadLength(
                        "hazard zone identity or class is invalid".into(),
                    ));
                }
                bytes = add_size(bytes, 21, "hazard zone")?;
            } else {
                let expected_cap = expected_cap(kind, zone.class)?;
                if zone.cap != expected_cap {
                    return Err(PackError::BadLength(
                        "zone cap differs from its kind".into(),
                    ));
                }
                npc_east_extent(zone)?;
                bytes = add_size(bytes, 14, "NPC zone")?;
            }
        } else {
            if kind.npc_id < 0 {
                return Err(PackError::BadLength(
                    "zone shape belongs to a hazard".into(),
                ));
            }
            if table.shapes().get(usize::from(zone.shape)).is_none() {
                return Err(PackError::BadLength(
                    "zone shape index is out of range".into(),
                ));
            }
            let expected_cap = expected_cap(kind, zone.class)?;
            if zone.cap != expected_cap {
                return Err(PackError::BadLength(
                    "zone cap differs from its kind".into(),
                ));
            }
            npc_east_extent(zone)?;
            npc_north_extent(zone)?;
            shaped_zones = shaped_zones
                .checked_add(1)
                .ok_or_else(|| PackError::BadLength("zone shape count overflows".into()))?;
            bytes = add_size(bytes, 14, "shaped NPC zone")?;
        }
    }
    if shaped_zones != table.shapes().len() {
        return Err(PackError::BadLength(
            "zone shape count differs from shaped zones".into(),
        ));
    }
    for group in table.groups() {
        packed_count(
            group.members.len(),
            table.zones().len(),
            "zone group member",
        )?;
        bytes = add_size(bytes, 21, "zone group fields")?;
        bytes = add_size(
            bytes,
            string_size(&group.id, "zone group id")?,
            "zone group",
        )?;
        bytes = add_size(
            bytes,
            string_size(&group.label, "zone group label")?,
            "zone group",
        )?;
        bytes = add_size(
            bytes,
            group
                .members
                .len()
                .checked_mul(2)
                .ok_or_else(|| PackError::BadLength("zone group size overflows".into()))?,
            "zone group",
        )?;
        if let Some(level) = group.rect.level {
            if !(0..=3).contains(&level) {
                return Err(PackError::BadLength(format!(
                    "zone group level {level} is invalid"
                )));
            }
        }
    }
    for (zone, _) in table.carves() {
        if usize::from(*zone) >= table.zones().len() {
            return Err(PackError::BadLength(
                "carve zone index is out of range".into(),
            ));
        }
    }
    bytes = add_size(
        bytes,
        table
            .carves()
            .len()
            .checked_mul(18)
            .ok_or_else(|| PackError::BadLength("zone carve size overflows".into()))?,
        "zone carves",
    )?;
    bytes = add_size(
        bytes,
        table
            .shapes()
            .len()
            .checked_mul(11)
            .ok_or_else(|| PackError::BadLength("zone shape size overflows".into()))?,
        "zone shapes",
    )?;
    Ok(bytes)
}

pub(super) fn write_table(out: &mut Vec<u8>, table: Option<&ZoneTable>) -> Result<(), PackError> {
    // `encode` preflights the complete table with `wire_size` before writing.
    let Some(table) = table else {
        for _ in 0..5 {
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        return Ok(());
    };
    out.extend_from_slice(&to_u32(table.kinds().len(), "zone kind count")?.to_le_bytes());
    for kind in table.kinds() {
        out.extend_from_slice(&kind.npc_id.to_le_bytes());
        out.extend_from_slice(&kind.vislevel.to_le_bytes());
        out.push(u8::from(kind.ap) | (u8::from(kind.vis_off) << 1));
        write_string(out, &kind.id)?;
        write_string(out, &kind.label)?;
    }

    out.extend_from_slice(&to_u32(table.zones().len(), "zone count")?.to_le_bytes());
    for zone in table.zones() {
        let kind = table
            .kinds()
            .get(usize::from(zone.kind))
            .ok_or_else(|| PackError::BadLength("zone kind index is out of range".into()))?;
        if kind.npc_id < 0 {
            out.push(1);
            for value in [zone.min_x, zone.max_x, zone.min_z, zone.max_z] {
                out.extend_from_slice(&value.to_le_bytes());
            }
            out.push(zone.level);
            out.push(zone.class as u8);
            out.extend_from_slice(&zone.kind.to_le_bytes());
        } else {
            let east_extent = npc_east_extent(zone)?;
            out.push(0);
            out.extend_from_slice(&zone.spawn_x.to_le_bytes());
            out.extend_from_slice(&zone.spawn_z.to_le_bytes());
            // Shaped rows reinterpret r as the east extent; the shape row
            // carries its independent north extent.
            out.push(east_extent);
            out.push(zone.level);
            out.push(zone.class as u8);
            out.extend_from_slice(&zone.kind.to_le_bytes());
        }
    }

    out.extend_from_slice(&to_u32(table.groups().len(), "zone group count")?.to_le_bytes());
    for group in table.groups() {
        write_string(out, &group.id)?;
        write_string(out, &group.label)?;
        for value in [
            group.rect.min_x,
            group.rect.max_x,
            group.rect.min_z,
            group.rect.max_z,
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        let level = match group.rect.level {
            None => -1,
            Some(level @ 0..=3) => level as i8,
            Some(level) => {
                return Err(PackError::BadLength(format!(
                    "zone group has invalid level {level}"
                )));
            }
        };
        out.push(level as u8);
        out.extend_from_slice(
            &to_u32(group.members.len(), "zone group member count")?.to_le_bytes(),
        );
        for member in group.members.iter() {
            out.extend_from_slice(&member.to_le_bytes());
        }
    }

    out.extend_from_slice(&to_u32(table.carves().len(), "zone carve count")?.to_le_bytes());
    for (zone, rect) in table.carves() {
        out.extend_from_slice(&zone.to_le_bytes());
        for value in [rect.min_x, rect.max_x, rect.min_z, rect.max_z] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }

    out.extend_from_slice(&to_u32(table.shapes().len(), "zone shape count")?.to_le_bytes());
    for (zone_index, zone) in table.zones().iter().enumerate() {
        if zone.shape == NO_SHAPE {
            continue;
        }
        out.extend_from_slice(
            &u16::try_from(zone_index)
                .map_err(|_| PackError::BadLength("zone index exceeds packed limit".into()))?
                .to_le_bytes(),
        );
        out.push(npc_north_extent(zone)?);
        let bits = table
            .shapes()
            .get(usize::from(zone.shape))
            .ok_or_else(|| PackError::BadLength("zone shape index is out of range".into()))?;
        out.extend_from_slice(&bits.to_le_bytes());
    }
    Ok(())
}

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

fn packed_count(value: usize, max: usize, what: &str) -> Result<u32, PackError> {
    if value > max {
        return Err(PackError::BadLength(format!(
            "{what} count {value} exceeds {max}"
        )));
    }
    u32::try_from(value).map_err(|_| PackError::BadLength(format!("{what} count exceeds u32")))
}

fn add_size(total: usize, amount: usize, what: &str) -> Result<usize, PackError> {
    total
        .checked_add(amount)
        .ok_or_else(|| PackError::BadLength(format!("{what} size overflows usize")))
}

fn string_size(value: &str, what: &str) -> Result<usize, PackError> {
    u32::try_from(value.len())
        .map_err(|_| PackError::BadLength(format!("{what} length exceeds u32")))?;
    add_size(4, value.len(), what)
}

fn expected_cap(kind: &ZoneKind, class: ZoneClass) -> Result<u16, PackError> {
    match class {
        ZoneClass::Always => Ok(u16::MAX),
        ZoneClass::LevelRule => kind.vislevel.checked_mul(2).ok_or_else(|| {
            PackError::BadLength("zone kind combat level exceeds packed cap".into())
        }),
    }
}

fn npc_east_extent(zone: &Zone) -> Result<u8, PackError> {
    let left = zone
        .spawn_x
        .checked_sub(zone.min_x)
        .ok_or_else(|| PackError::BadLength("zone bounds overflow x".into()))?;
    let east = zone
        .max_x
        .checked_sub(zone.spawn_x)
        .ok_or_else(|| PackError::BadLength("zone bounds overflow x".into()))?;
    let south = zone
        .spawn_z
        .checked_sub(zone.min_z)
        .ok_or_else(|| PackError::BadLength("zone bounds overflow z".into()))?;
    let north = zone
        .max_z
        .checked_sub(zone.spawn_z)
        .ok_or_else(|| PackError::BadLength("zone bounds overflow z".into()))?;
    let extent = if zone.shape == NO_SHAPE {
        if left != east || left != south || left != north {
            return Err(PackError::BadLength(
                "unshaped NPC zone bounds are not square".into(),
            ));
        }
        left
    } else {
        if left != 1 || south != 1 || !(1..=6).contains(&east) || !(1..=6).contains(&north) {
            return Err(PackError::BadLength(
                "shaped NPC zone footprint exceeds 8x8 bounds".into(),
            ));
        }
        east
    };
    u8::try_from(extent).map_err(|_| PackError::BadLength("NPC zone extent exceeds u8".into()))
}

fn npc_north_extent(zone: &Zone) -> Result<u8, PackError> {
    if zone.shape == NO_SHAPE {
        return Err(PackError::BadLength(
            "north extent requested for unshaped NPC zone".into(),
        ));
    }
    let north = zone
        .max_z
        .checked_sub(zone.spawn_z)
        .ok_or_else(|| PackError::BadLength("zone bounds overflow z".into()))?;
    if !(1..=6).contains(&north) {
        return Err(PackError::BadLength(
            "shaped NPC zone north extent exceeds 6".into(),
        ));
    }
    u8::try_from(north)
        .map_err(|_| PackError::BadLength("shaped zone north extent exceeds u8".into()))
}

fn write_string(out: &mut Vec<u8>, value: &str) -> Result<(), PackError> {
    out.extend_from_slice(
        &u32::try_from(value.len())
            .map_err(|_| PackError::BadLength("zone string length exceeds u32".into()))?
            .to_le_bytes(),
    );
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn to_u32(value: usize, what: &str) -> Result<u32, PackError> {
    packed_count(value, u32::MAX as usize, what)
}
