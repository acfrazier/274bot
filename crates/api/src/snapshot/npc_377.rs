//! Java 377 Client.java:10521-10546 and NpcType.java:359-372.
//! Resolve exactly one current menu form. Keep the placed NPC type as identity.
use client::client::Client;
use client::config::NpcType;

pub(super) fn menu_form(client: &Client, placed: usize) -> Option<&NpcType> {
    let cache = &client.cache;
    let fields = &cache.revision_377.as_ref()?.config.npcs;
    let root = fields.get(placed)?;
    let current = if let Some(morph) = &root.morph {
        let selector = if morph.varbit != -1 {
            let bit = cache.varbits.get(usize::try_from(morph.varbit).ok()?)?;
            if bit.startbit < 0 || bit.endbit < bit.startbit || bit.endbit > 31 {
                return None;
            }
            let value = *client.var.get(usize::try_from(bit.basevar).ok()?)?;
            let mask = (u32::MAX >> (31 - (bit.endbit - bit.startbit))) as i32;
            (value >> bit.startbit) & mask
        } else {
            *client.var.get(usize::try_from(morph.varp).ok()?)?
        };
        usize::try_from(*morph.targets.get(usize::try_from(selector).ok()?)?).ok()?
    } else {
        placed
    };
    if !fields.get(current)?.field_1434 {
        return None;
    }
    cache.npcs.get(current)
}
