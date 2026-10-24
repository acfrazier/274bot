use std::collections::HashMap;
use std::fs;
use std::path::Path;

use api::snapshot::WorldTile;
use nav_planner::pack::{BankAccess, BankStand, PackError};

pub(super) const BANK_BOOTH_CONFIG: &str = "scripts/interface_bank/configs/bank_booth.loc";

/// Write the bank stand table: count u32le, then per stand a
/// length-prefixed name, the `x/z/level` tile i32le, and the access (u8
/// tag 0 = Booth `op` i32le, 1 = Npc length-prefixed name + `op` i32le +
/// an optional dialog choice: presence u8 then a length-prefixed string).
pub(super) fn write_bank_stands(out: &mut Vec<u8>, banks: &[BankStand]) -> Result<(), PackError> {
    out.extend_from_slice(
        &u32::try_from(banks.len())
            .map_err(|_| PackError::BadLength("bank stand count exceeds u32".into()))?
            .to_le_bytes(),
    );
    for b in banks {
        write_name(out, &b.name)?;
        for v in [b.tile.x, b.tile.z, b.tile.level] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        match &b.access {
            BankAccess::Booth { op } => {
                out.push(0);
                out.extend_from_slice(&op.to_le_bytes());
            }
            BankAccess::Npc { name, op, choose } => {
                out.push(1);
                write_name(out, name)?;
                out.extend_from_slice(&op.to_le_bytes());
                match choose {
                    Some(c) => {
                        out.push(1);
                        write_name(out, c)?;
                    }
                    None => out.push(0),
                }
            }
        }
    }
    Ok(())
}


/// A length-prefixed UTF-8 string (the bank stand name fields).
pub(super) fn write_name(out: &mut Vec<u8>, s: &str) -> Result<(), PackError> {
    out.extend_from_slice(
        &u32::try_from(s.len())
            .map_err(|_| PackError::BadLength("bank stand name length exceeds u32".into()))?
            .to_le_bytes(),
    );
    out.extend_from_slice(s.as_bytes());
    Ok(())
}


/// Bake the bank stand table from the Server content tree (the maps
/// dir's parent): the same jm2 LOC pass the collision bake uses
/// ([`crate::transport`]'s placement reader). Every `bankbooth` loc
/// placement becomes a [`BankAccess::Booth`] stand — named from the
/// `[bankbooth]` block of `scripts/interface_bank/configs/bank_booth.loc`
/// and accessed with the Use-quickly op (2, `[oploc2,bankbooth]`). The
/// closed-booth (`bankboothclosed`) and tutorial (`newbiebankbooth`) loc
/// ids are never looked up, so they cannot enter the table. NPC teller
/// stands (`category=bank_teller` in `*.npc` configs) join from the jm2
/// `==== NPC ====` placements: the packed op is the config's `Bank` slot
/// when present, else 3 (`[opnpc3,_bank_teller]`). Stands sort by tile
/// for a deterministic wire. A tree with tellers and no booths still
/// emits the NPC stands.
pub fn derive_banks(content_root: &Path) -> Vec<BankStand> {
    let mut banks = Vec::new();
    let loc_ids = crate::transport::loc_ids_by_name(content_root);
    if let Some(&booth_id) = loc_ids.get("bankbooth") {
        let name = bank_booth_name(content_root);
        let positions = crate::transport::loc_positions(content_root);
        if let Some(placements) = positions.get(&booth_id) {
            banks.extend(placements.iter().map(|p| BankStand {
                name: name.clone(),
                tile: WorldTile {
                    x: p.x,
                    z: p.z,
                    level: p.level,
                },
                access: BankAccess::Booth { op: 2 },
            }));
        }
    }
    let npc_hits = crate::transport::npc_positions(content_root);
    for teller in bank_tellers(content_root) {
        let Some(hits) = npc_hits.get(&teller.id) else {
            continue;
        };
        banks.extend(hits.iter().map(|p| BankStand {
            name: teller.name.clone(),
            tile: WorldTile {
                x: p.x,
                z: p.z,
                level: p.level,
            },
            access: BankAccess::Npc {
                name: teller.name.clone(),
                op: teller.op,
                choose: None,
            },
        }));
    }
    banks.sort_by_key(|b| (b.tile.level, b.tile.x, b.tile.z));
    banks
}

struct BankTeller {
    id: i32,
    name: String,
    op: i32,
}

/// NPC configs whose `category=bank_teller`, keyed by pack id. First
/// definition for an id wins.
fn bank_tellers(content_root: &Path) -> Vec<BankTeller> {
    let ids = crate::transport::npc_ids_by_name(content_root);
    if ids.is_empty() {
        return Vec::new();
    }
    let mut by_id: HashMap<i32, BankTeller> = HashMap::new();
    crate::transport::visit_npc_configs(&content_root.join("scripts"), &mut |text| {
        ingest_teller_blocks(text, &ids, &mut by_id);
    });
    let mut out: Vec<BankTeller> = by_id.into_values().collect();
    out.sort_by_key(|t| t.id);
    out
}

fn ingest_teller_blocks(
    text: &str,
    ids: &HashMap<String, i32>,
    out: &mut HashMap<i32, BankTeller>,
) {
    let mut header = None;
    let mut display = None;
    let mut ops: [Option<String>; 5] = Default::default();
    let mut teller = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            flush_teller(header.take(), display.take(), &ops, teller, ids, out);
            ops = Default::default();
            teller = false;
            header = Some(name.to_string());
            continue;
        }
        if header.is_none() {
            continue;
        }
        if let Some(v) = line.strip_prefix("name=") {
            let v = v.trim();
            if !v.is_empty() {
                display = Some(v.to_string());
            }
        } else if let Some(v) = line.strip_prefix("category=") {
            teller = v.trim() == "bank_teller";
        } else if let Some(rest) = line.strip_prefix("op") {
            if let Some((slot, value)) = rest.split_once('=') {
                if let Ok(n) = slot.parse::<usize>() {
                    if (1..=5).contains(&n) {
                        let value = value.trim();
                        ops[n - 1] = (!value.is_empty()).then(|| value.to_string());
                    }
                }
            }
        }
    }
    flush_teller(header, display, &ops, teller, ids, out);
}

fn flush_teller(
    header: Option<String>,
    display: Option<String>,
    ops: &[Option<String>; 5],
    teller: bool,
    ids: &HashMap<String, i32>,
    out: &mut HashMap<i32, BankTeller>,
) {
    if !teller {
        return;
    }
    let Some(name) = header else {
        return;
    };
    let Some(&id) = ids.get(&name) else {
        return;
    };
    out.entry(id).or_insert_with(|| {
        let op = ops
            .iter()
            .enumerate()
            .find_map(|(i, label)| {
                label
                    .as_deref()
                    .filter(|s| s.eq_ignore_ascii_case("bank"))
                    .map(|_| i as i32 + 1)
            })
            .unwrap_or(3);
        let shown = display
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| name.clone());
        BankTeller {
            id,
            name: shown,
            op,
        }
    });
}

/// The `name=` of the `[bankbooth]` block (the booth loc config's display
/// name), `"Bank booth"` when the config is missing.
pub(super) fn bank_booth_name(content_root: &Path) -> String {
    let Ok(text) = fs::read_to_string(content_root.join(BANK_BOOTH_CONFIG)) else {
        return "Bank booth".to_string();
    };
    let mut in_block = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_block = line == "[bankbooth]";
            continue;
        }
        if in_block {
            if let Some(v) = line.strip_prefix("name=") {
                let v = v.trim();
                if !v.is_empty() {
                    return v.to_string();
                }
            }
        }
    }
    "Bank booth".to_string()
}
