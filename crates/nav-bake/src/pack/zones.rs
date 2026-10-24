use nav_planner::pack::{PackError, MAX_PACKED_INDEX};
use nav_planner::zones::{Zone, ZoneClass, ZoneKind, ZoneTable, NO_SHAPE};
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
