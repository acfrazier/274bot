use std::collections::HashSet;
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
/// ids are never looked up, so they cannot enter the table. NPC access
/// identities come from the shared game-data content rule in `bank_npcs.json`.
/// Stands sort by tile for a deterministic wire.
pub fn derive_banks(
    content_root: &Path,
    game_data_dir: &Path,
    revision: u16,
) -> Result<Vec<BankStand>, crate::GenerationError> {
    let npc_accesses = read_npc_bank_accesses(game_data_dir, revision)?;
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
    for teller in npc_accesses {
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
                choose: teller.choose.clone(),
            },
        }));
    }
    banks.sort_by_key(|b| (b.tile.level, b.tile.x, b.tile.z));
    Ok(banks)
}

#[derive(serde::Deserialize)]
struct NpcBankAccess {
    id: i32,
    name: String,
    op: i32,
    choose: Option<String>,
}

#[derive(serde::Deserialize)]
struct NpcBankRows {
    rows: Vec<NpcBankAccess>,
}

#[derive(serde::Deserialize)]
struct NpcBankFacts {
    schema: u16,
    revision: u16,
    facts: NpcBankRows,
}

fn read_npc_bank_accesses(
    game_data_dir: &Path,
    revision: u16,
) -> Result<Vec<NpcBankAccess>, crate::GenerationError> {
    let path = game_data_dir
        .join(revision.to_string())
        .join("bank_npcs.json");
    let bytes = fs::read(&path).map_err(|source| {
        crate::GenerationError::SourceInput(crate::SourceInputError::Io {
            context: crate::SourceInputContext::GameData,
            path: path.clone(),
            source,
        })
    })?;
    let invalid = |detail| crate::GenerationError::SelectedGameDataUnavailable {
        revision,
        detail: format!("NPC bank facts {}: {detail}", path.display()),
    };
    let facts: NpcBankFacts =
        serde_json::from_slice(&bytes).map_err(|error| invalid(format!("{error}")))?;
    if facts.schema != 1 || facts.revision != revision {
        return Err(invalid("schema or revision mismatch".into()));
    }
    let mut ids = HashSet::new();
    for access in &facts.facts.rows {
        if access.id < 0
            || !(1..=5).contains(&access.op)
            || access.name.trim().is_empty()
            || access
                .choose
                .as_ref()
                .is_some_and(|choice| choice.trim().is_empty())
            || !ids.insert(access.id)
        {
            return Err(invalid("invalid or duplicate access".into()));
        }
    }
    Ok(facts.facts.rows)
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
