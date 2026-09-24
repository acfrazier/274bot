use super::*;

/// The magic side-tab index (the 2004 icon order: combat 0, stats 1,
/// quests 2, inventory 3, equipment 4, prayer 5, magic 6).
pub(super) const MAGIC_TAB: usize = 6;

/// One standard spellbook teleport: the packed landing tile the edge's
/// `to` identifies, the spellbook button's label word (`Cast @gre@<word>
/// teleport`, the 2004 spellbook text), and the 2004 spellbook component
/// id used when the live tree carries no matching button text.
#[derive(Debug, Clone, Copy)]
pub(super) struct SpellTeleport {
    dest: &'static str,
    to: WorldTile,
    fallback_com_id: i32,
}

/// The seven spell teleports `derive_transports` packs (from
/// `magic_spells.dbrow` `tele_coord` + runes). A spell edge carries no
/// widget on the wire (`loc_id` 0), so the traveller resolves the button
/// from the landing.
pub(super) const SPELL_TELEPORTS: &[SpellTeleport] = &[
    SpellTeleport {
        dest: "Varrock",
        to: WorldTile {
            x: 3213,
            z: 3424,
            level: 0,
        },
        fallback_com_id: 1164,
    },
    SpellTeleport {
        dest: "Lumbridge",
        to: WorldTile {
            x: 3221,
            z: 3218,
            level: 0,
        },
        fallback_com_id: 1167,
    },
    SpellTeleport {
        dest: "Falador",
        to: WorldTile {
            x: 2965,
            z: 3378,
            level: 0,
        },
        fallback_com_id: 1170,
    },
    SpellTeleport {
        dest: "Camelot",
        to: WorldTile {
            x: 2757,
            z: 3478,
            level: 0,
        },
        fallback_com_id: 1174,
    },
    SpellTeleport {
        dest: "Ardougne",
        to: WorldTile {
            x: 2661,
            z: 3301,
            level: 0,
        },
        fallback_com_id: 1540,
    },
    SpellTeleport {
        dest: "Watchtower",
        to: WorldTile {
            x: 2933,
            z: 4713,
            level: 2,
        },
        fallback_com_id: 1541,
    },
    SpellTeleport {
        dest: "Trollheim",
        to: WorldTile {
            x: 2890,
            z: 3679,
            level: 0,
        },
        fallback_com_id: 7455,
    },
];

/// The chat-modal choice an Npc hop answers to ride: the ride is always
/// the modal's FIRST choice (the cart drivers' "Yes please…" fare and
/// Elkoy's escort both present it first). This is the hop's dialog rule,
/// independent of the NPC op index — [`TransportEdge::option`] is the op
/// (Talk-to is op 1, the essence wizard's teleport op 3/4) and stays the
/// interact's operation. Also the fallback choice when a jewellery hop's
/// teleport list is unavailable ([`TravelOptions::teleports`]).
pub(super) const NPC_RIDE_CHOICE: i32 = 1;

/// The dialog choice a jewellery rub hop answers: the 1-based index of
/// the edge's `to` among the packed same-`loc_id` rub edges — the
/// `switch_int($choice)` case order the bake emitted (the dueling ring's
/// only sibling answers 1). Npc hops (and jewellery hops without a
/// teleport list) fall back to the modal's FIRST choice. Spirit-tree dest
/// pages use the same rule among same-`loc_id`/`at` packed siblings.
pub(super) fn dest_dialog_choice(
    leg: &Leg,
    teleports: Option<&[TransportEdge]>,
    packed: Option<&[TransportEdge]>,
) -> i32 {
    let Leg::Transport { edge } = leg else {
        return NPC_RIDE_CHOICE;
    };
    let siblings = match edge.kind {
        TransportKind::Teleport if edge.loc_id > 0 => teleports,
        TransportKind::SpiritTree => packed,
        _ => return NPC_RIDE_CHOICE,
    };
    let Some(list) = siblings else {
        return NPC_RIDE_CHOICE;
    };
    list.iter()
        .filter(|e| e.kind == edge.kind && e.loc_id == edge.loc_id)
        .filter(|e| edge.kind != TransportKind::SpiritTree || e.at == edge.at)
        .position(|e| e.to == edge.to)
        .map(|i| i as i32 + 1)
        .unwrap_or(NPC_RIDE_CHOICE)
}

/// Joined chat option texts: a new dest-list page is a different key
/// than the spirit-tree "Where can I go?" gate that opened it.
pub(super) fn chat_page_key(snapshot: &GameSnapshot) -> String {
    snapshot
        .chat_options()
        .iter()
        .map(|o| o.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The chat choice this hop presses on the current option page.
/// Jewellery dests and Npc fare use [`dest_dialog_choice`]. Adult spirit
/// trees (`spirit_tree.rs2` ent / stronghold_ent) put "No thanks, old
/// tree." first on a 2-option page; dest-index 1 there silently drops
/// the hop. The 2-option page answers 2 ("Where can I go?"); the 3-option
/// dest list then uses packed sibling order. The young tree (loc 1317)
/// is a single "Yes please." / "No thank you." — choice 1 rides.
pub(super) fn hop_dialog_choice(
    leg: &Leg,
    teleports: Option<&[TransportEdge]>,
    packed: Option<&[TransportEdge]>,
    n_options: usize,
) -> i32 {
    let Leg::Transport { edge } = leg else {
        return NPC_RIDE_CHOICE;
    };
    if edge.kind == TransportKind::SpiritTree {
        return spirit_tree_choice(leg, edge, packed, n_options);
    }
    dest_dialog_choice(leg, teleports, packed)
}

pub(super) fn spirit_tree_choice(
    leg: &Leg,
    edge: &TransportEdge,
    packed: Option<&[TransportEdge]>,
    n_options: usize,
) -> i32 {
    let n_dests = spirit_tree_dest_count(edge, packed);
    if n_dests == 1 {
        return NPC_RIDE_CHOICE;
    }
    if n_options >= 3 {
        return dest_dialog_choice(leg, None, packed);
    }
    2
}

pub(super) fn spirit_tree_dest_count(edge: &TransportEdge, packed: Option<&[TransportEdge]>) -> usize {
    let Some(list) = packed else {
        return 0;
    };
    let mut dests: Vec<WorldTile> = Vec::new();
    for e in list {
        if e.kind == TransportKind::SpiritTree && e.loc_id == edge.loc_id && !dests.contains(&e.to)
        {
            dests.push(e.to);
        }
    }
    dests.len()
}

/// `glidermap` (interface.pack 802): dest IF_BUTTON children of
/// `if_openmain(glidermap)` in `gnome_glider.rs2`.
pub(super) const GLIDER_MAP_ROOT: i32 = 802;

/// Packed glider `to` → `glidermap:com_21`..=`com_25` (interface.pack
/// 824..=828). Pad tiles match `GLIDER_PADS` / `GLIDER_HUB` in transport.rs.
pub(super) const GLIDER_DEST_BUTTONS: &[(WorldTile, i32)] = &[
    (
        WorldTile {
            x: 2971,
            z: 2969,
            level: 0,
        },
        824,
    ), // gandius — com_21
    (
        WorldTile {
            x: 2465,
            z: 3501,
            level: 3,
        },
        825,
    ), // ta_quir_priw — com_22
    (
        WorldTile {
            x: 2850,
            z: 3497,
            level: 0,
        },
        826,
    ), // sindarpos — com_23
    (
        WorldTile {
            x: 3320,
            z: 3430,
            level: 0,
        },
        827,
    ), // lemanto_andra — com_24
    (
        WorldTile {
            x: 3284,
            z: 3211,
            level: 0,
        },
        828,
    ), // kar_hewo — com_25
];

/// The glidermap dest button for a Glider hop's `to`, if that landing is
/// one of the five Gnome Air pads. Other kinds (boat `ship_journey`) have
/// no dest IF_BUTTON.
pub(super) fn dest_map_component(edge: &TransportEdge) -> Option<i32> {
    if edge.kind != TransportKind::Glider {
        return None;
    }
    GLIDER_DEST_BUTTONS
        .iter()
        .find(|(to, _)| *to == edge.to)
        .map(|(_, id)| *id)
}

/// The outcome of sending a packed teleport hop's op.
pub(super) enum TeleportSend {
    /// The op was accepted; the hop now settles `arrived(to)`.
    Sent,
    /// The interact/press was refused by the driver.
    Refused(SendReason),
    /// The hop cannot be worked yet (the charged item is not in the
    /// loaded inventory, or the driver dropped the press): keep waiting,
    /// bounded by the hop budget.
    Wait,
    /// The edge can never be executed (a spell landing outside the seven
    /// standard spellbook teleports).
    Blocked(String),
}

/// The magic-tab button of a spell teleport edge: the standard spell the
/// edge's landing names, looked up live by the spellbook button text
/// (`Cast @gre@<dest> teleport`, the 2004 label), else the 2004
/// spellbook component id. `None` when the landing is not one of the
/// seven standard spellbook teleports (a pack row this model cannot
/// execute).
pub(super) fn spell_button(snapshot: &GameSnapshot, edge: &TransportEdge) -> Option<i32> {
    let spell = SPELL_TELEPORTS.iter().find(|s| s.to == edge.to)?;
    let root = snapshot
        .side_tabs()
        .get(MAGIC_TAB)
        .map(|t| t.root_component_id)
        .unwrap_or(-1);
    let label = format!("Cast @gre@{} teleport", spell.dest);
    if root != -1 {
        let com_id = api::query::widget_search::button_by_text(snapshot, root, &label);
        if com_id != -1 {
            return Some(com_id);
        }
    }
    Some(spell.fallback_com_id)
}

/// The widget view for `com_id` in the snapshot's open roots or side
/// tabs, `None` when no live tree carries it.
pub(super) fn find_component(snapshot: &GameSnapshot, com_id: i32) -> Option<&WidgetView> {
    snapshot
        .widgets()
        .iter()
        .chain(snapshot.side_tabs().iter().flat_map(|t| t.widgets.iter()))
        .find(|w| w.component_id == com_id)
}

/// Send a packed `TransportKind::Teleport` hop's op: a held-item Rub
/// (`OP_HELD<option>` on the charged jewellery obj the edge names) or the
/// spellbook button of the standard spell the edge's landing names (a
/// gated IF_BUTTON press on the live button, else the unconditional 2004
/// fallback id). Never the WalkTo `::tele` cheat.
pub(super) fn teleport_send<D: Driver>(
    snapshot: &GameSnapshot,
    d: &mut D,
    edge: &TransportEdge,
) -> TeleportSend {
    if edge.loc_id > 0 {
        let Some(item) = snapshot
            .inventory()
            .iter()
            .find(|it| it.def.id == edge.loc_id)
        else {
            return TeleportSend::Wait;
        };
        let mut ix = Interactions::new(snapshot, d);
        return match ix.interact(OpTarget::Item(item), ActionSpec::Operation(edge.option)) {
            SendResult::Sent { .. } => TeleportSend::Sent,
            SendResult::Refused { reason, .. } => TeleportSend::Refused(reason),
        };
    }
    let Some(com_id) = spell_button(snapshot, edge) else {
        return TeleportSend::Blocked(format!(
            "packed spell teleport to ({}, {}, {}) is not one of the seven standard spellbook teleports",
            edge.to.x, edge.to.z, edge.to.level
        ));
    };
    match find_component(snapshot, com_id) {
        Some(widget) => {
            let mut ix = Interactions::new(snapshot, d);
            match ix.press(widget) {
                SendResult::Sent { .. } => TeleportSend::Sent,
                SendResult::Refused { reason, .. } => TeleportSend::Refused(reason),
            }
        }
        None => {
            if press(d, com_id) {
                TeleportSend::Sent
            } else {
                TeleportSend::Wait
            }
        }
    }
}

