use super::*;

/// One bank stand on the v8 wire: a named interact target — either a
/// booth loc or a teller NPC — that opens a bank. `tile` is the interact
/// target (often unstandable: a booth loc, or a teller behind a counter).
/// BankBudget walks to a standable access neighbour
/// ([`crate::bank_fetch::bank_access_tiles`]) and uses the `access` op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankStand {
    /// The stand's display name ("Bank booth", the teller's NPC name).
    pub name: String,
    /// The interact tile (the booth loc tile or the NPC's tile).
    pub tile: WorldTile,
    /// How the stand is used.
    pub access: BankAccess,
}

/// How a [`BankStand`] is activated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BankAccess {
    /// A `bankbooth` loc: use `op` on it to open the bank (2 = the
    /// `Use-quickly` op of `scripts/interface_bank/configs/bank_booth.loc`).
    Booth { op: i32 },
    /// A teller NPC: use `op` on the named NPC to open the bank
    /// (`choose` is the dialog option text when the op itself only starts
    /// the dialogue, not the bank).
    Npc {
        name: String,
        op: i32,
        choose: Option<String>,
    },
}

/// Fewest bytes one [`BankStand`] can occupy on the v8 wire (name len
/// prefix + empty name + tile + access tag + op) — a preallocation cap.
pub(super) const MIN_BANK_BYTES: usize = 4 + 12 + 1 + 4;

/// Read the bank stand table from the v8 wire.
pub(super) fn read_bank_stands<R: PackRead>(r: &mut R) -> Result<Vec<BankStand>, PackError> {
    let n = read_u32(r)? as usize;
    let remaining = r.remaining();
    let mut out = Vec::with_capacity(n.min(remaining / MIN_BANK_BYTES));
    for _ in 0..n {
        let name = read_name(r)?;
        let tile = WorldTile {
            x: read_i32(r)?,
            z: read_i32(r)?,
            level: read_i32(r)?,
        };
        let access = match read_u8(r)? {
            0 => BankAccess::Booth { op: read_i32(r)? },
            1 => {
                let npc = read_name(r)?;
                let op = read_i32(r)?;
                let choose = if read_u8(r)? != 0 {
                    Some(read_name(r)?)
                } else {
                    None
                };
                BankAccess::Npc {
                    name: npc,
                    op,
                    choose,
                }
            }
            tag => {
                return Err(PackError::BadLength(format!(
                    "unknown bank access tag {tag}"
                )))
            }
        };
        out.push(BankStand { name, tile, access });
    }
    Ok(out)
}

/// Read a length-prefixed UTF-8 string from the v8 bank table.
pub(super) fn read_name<R: PackRead>(r: &mut R) -> Result<String, PackError> {
    let len = read_u32(r)? as usize;
    if len > r.remaining() {
        return Err(PackError::Truncated);
    }
    let mut buf = vec![0u8; len];
    r.read_bytes_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|_| PackError::BadLength("bank stand name is not UTF-8".into()))
}
