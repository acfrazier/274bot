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

/// The default 1-based chat choice used by one-option NPC rides and by
/// jewellery/spirit hops when no packed sibling can identify a destination.
/// NPC fare pages do not blindly use this fallback: [`npc_hop_dialog_choice`]
/// recognizes the content's affirmative branch and returns `None` for an
/// unknown page so a new option cannot silently select Crandor or another
/// destination.
pub(super) const NPC_RIDE_CHOICE: i32 = 1;

/// Captain Shanks' two packed Boat edges share one NPC tile. Unlike a fare
/// page, his final page names both destinations; keep the route-specific
/// identity here so following Port Sarim cannot accidentally answer the
/// first Khazard option.
const SHANKS_NPC_ID: i32 = 518;
const SHANKS_KHAZARD_DEST: WorldTile = WorldTile {
    x: 2680,
    z: 3150,
    level: 0,
};
const SHANKS_PORT_SARIM_DEST: WorldTile = WorldTile {
    x: 3047,
    z: 3235,
    level: 0,
};

/// The dialog choice a jewellery rub hop answers: the 1-based index of the
/// edge's `to` among the packed same-`loc_id` rub edges — the
/// `switch_int($choice)` case order the bake emitted (the dueling ring's
/// only sibling answers 1). Jewellery hops without a teleport list fall back
/// to the modal's first choice. Spirit-tree dest pages use the same rule
/// among same-`loc_id`/`at` packed siblings.
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
///
/// Jewellery destinations retain their packed sibling-index behavior. Adult
/// spirit trees (`spirit_tree.rs2` ent / stronghold_ent) put "No thanks, old
/// tree." first on a 2-option page; dest-index 1 there silently drops the
/// hop. The 2-option page answers 2 ("Where can I go?"); the 3-option dest
/// list then uses packed sibling order. The young tree (loc 1317) is a single
/// "Yes please." / "No thank you." — choice 1 rides.
///
/// NPC-backed rides are content-driven: the recognized affirmative option is
/// selected even when Dragon Slayer inserts Crandor before the normal fare.
/// An unrecognized NPC page returns `None` rather than guessing a destination.
/// Dialogue Door hops (Al Kharid toll, Shantay disclaimer) use the same
/// fail-closed content labels.
pub(super) fn hop_dialog_choice(
    leg: &Leg,
    teleports: Option<&[TransportEdge]>,
    packed: Option<&[TransportEdge]>,
    chat_options: &[api::snapshot::ChatOptionView],
) -> Option<i32> {
    let Leg::Transport { edge } = leg else {
        return Some(NPC_RIDE_CHOICE);
    };
    if edge.kind == TransportKind::SpiritTree {
        return Some(spirit_tree_choice(leg, edge, packed, chat_options));
    }
    if matches!(
        edge.kind,
        TransportKind::Npc | TransportKind::Boat | TransportKind::Glider
    ) {
        return npc_hop_dialog_choice(edge, packed, chat_options);
    }
    if edge.kind == TransportKind::Door {
        return door_hop_dialog_choice(edge.loc_id, chat_options);
    }
    Some(dest_dialog_choice(leg, teleports, packed))
}

/// Content-driven Door chat: the Al Kharid border-guard pay option and the
/// Shantay first-crossing disclaimer. Unknown or duplicate labels fail
/// closed — choice 1 on the toll page is "walk around", not pay.
pub(super) fn door_hop_dialog_choice(
    loc_id: i32,
    chat_options: &[api::snapshot::ChatOptionView],
) -> Option<i32> {
    let matchers: &[fn(&str) -> bool] = if loc_id == SHANTAY_HENGE_LOC_ID {
        &[is_shantay_disclaimer_choice]
    } else if loc_id == AL_KHARID_TOLL_LEFT_LOC_ID || loc_id == AL_KHARID_TOLL_RIGHT_LOC_ID {
        &[is_alkharid_pay_choice]
    } else {
        return None;
    };
    for matcher in matchers {
        match unique_option_choice(chat_options, *matcher) {
            Err(()) => return None,
            Ok(Some(choice)) => return Some(choice),
            Ok(None) => {}
        }
    }
    None
}

/// Select the affirmative branch of a packed NPC-backed transport dialog.
///
/// These labels are the actual 289 content branches:
/// * sailors and cart/Elkoy fares use a "Yes please" answer;
/// * Entrana monks use one of the explicit "ready to go" answers;
/// * customs first asks to journey, then asks to search, then asks "Ok.";
/// * a glider pilot asks "Can you take me on the glider?";
/// * Captain Shanks' final page names Khazard and Port Sarim. Its target is
///   selected only when the matching two-edge Boat family is present in the
///   packed graph; an incomplete family fails closed.
///
/// The returned number is the live modal's 1-based option position, not the
/// script's `switch_int` value.
pub(super) fn npc_hop_dialog_choice(
    edge: &TransportEdge,
    packed: Option<&[TransportEdge]>,
    chat_options: &[api::snapshot::ChatOptionView],
) -> Option<i32> {
    if is_shanks_destination_page(chat_options) {
        return shanks_destination_choice(edge, packed, chat_options);
    }
    // Prefer the most specific page labels before the generic "Yes please"
    // fare branch. Each matcher is unique on the content page; a duplicate
    // match is treated as unknown rather than choosing one arbitrarily.
    for matcher in [
        is_glider_ride_choice as fn(&str) -> bool,
        is_customs_journey_choice,
        is_customs_search_choice,
        is_customs_ok_choice,
        is_affirmative_ride_choice,
    ] {
        match unique_option_choice(chat_options, matcher) {
            Err(()) => return None,
            Ok(Some(choice)) => return Some(choice),
            Ok(None) => {}
        }
    }
    None
}

fn is_shanks_destination_page(chat_options: &[api::snapshot::ChatOptionView]) -> bool {
    chat_options.len() == 3
        && option_eq(&chat_options[0].text, "Khazard Port please.")
        && option_eq(&chat_options[1].text, "Port Sarim please.")
        && option_eq(&chat_options[2].text, "Nowhere just at the moment thanks.")
}

fn shanks_destination_choice(
    edge: &TransportEdge,
    packed: Option<&[TransportEdge]>,
    chat_options: &[api::snapshot::ChatOptionView],
) -> Option<i32> {
    if edge.kind != TransportKind::Boat || edge.loc_id != SHANKS_NPC_ID {
        return None;
    }
    let expected_position = match edge.to {
        SHANKS_KHAZARD_DEST => 0,
        SHANKS_PORT_SARIM_DEST => 1,
        _ => return None,
    };
    let list = packed?;
    let mut sibling_count = 0;
    let mut target_position = None;
    for sibling in list.iter().filter(|sibling| {
        sibling.kind == TransportKind::Boat
            && sibling.loc_id == SHANKS_NPC_ID
            && sibling.at == edge.at
    }) {
        if sibling.to == edge.to {
            if target_position.is_some() {
                return None;
            }
            target_position = Some(sibling_count);
        }
        sibling_count += 1;
    }
    if sibling_count != 2 || target_position != Some(expected_position) {
        return None;
    }
    let choice = expected_position + 1;
    (choice <= chat_options.len()).then_some(choice as i32)
}

fn unique_option_choice(
    chat_options: &[api::snapshot::ChatOptionView],
    matcher: fn(&str) -> bool,
) -> Result<Option<i32>, ()> {
    let mut found = None;
    for (index, option) in chat_options.iter().enumerate() {
        if matcher(option.text.trim()) {
            let choice = index as i32 + 1;
            if found.is_some() {
                return Err(());
            }
            found = Some(choice);
        }
    }
    Ok(found)
}

fn option_eq(text: &str, expected: &str) -> bool {
    text.trim()
        .trim_end_matches('.')
        .eq_ignore_ascii_case(expected.trim_end_matches('.'))
}

fn option_starts_with(text: &str, prefix: &str) -> bool {
    text.trim()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

fn is_glider_ride_choice(text: &str) -> bool {
    option_eq(text, "Can you take me on the glider?")
}

fn is_customs_journey_choice(text: &str) -> bool {
    option_eq(text, "Can I journey on this ship?")
}

fn is_customs_search_choice(text: &str) -> bool {
    option_eq(text, "Search away, I have nothing to hide.")
}

fn is_customs_ok_choice(text: &str) -> bool {
    option_eq(text, "Ok.")
}

fn is_affirmative_ride_choice(text: &str) -> bool {
    option_eq(text, "Yes please")
        || option_starts_with(text, "Yes please, I'd like to go")
        || option_eq(text, "Can you show me out of the village?")
        || option_starts_with(text, "Yes, I'll buy a ticket for the ship")
        || option_eq(text, "Yes, I'm ready to go")
        || option_eq(text, "Yes, okay, I'm ready to go")
}

/// `border_gate.rs2` `~p_choice3(..., "Yes, ok.", 3)`: the only branch that
/// pays and calls `@pass_toll_gate`. The first option walks around.
fn is_alkharid_pay_choice(text: &str) -> bool {
    option_eq(text, "Yes, ok")
}

/// `shantay_pass.rs2` first-crossing `~p_choice2_header(..., "Go into Desert?")`.
fn is_shantay_disclaimer_choice(text: &str) -> bool {
    option_eq(text, "Yeah, that poster doesn't scare me!")
}

/// Honest refusal for the Al Kharid pay page when the inventory cannot
/// cover `inv_total(inv, coins) < 10` in `border_gate.rs2`. Other Door
/// pages (Shantay disclaimer) are not a coin charge.
pub(super) fn door_hop_choice_blocked(
    edge: &TransportEdge,
    snapshot: &GameSnapshot,
    choice: i32,
) -> Option<String> {
    if edge.kind != TransportKind::Door {
        return None;
    }
    let index = usize::try_from(choice.checked_sub(1)?).ok()?;
    let text = snapshot.chat_options().get(index)?.text.as_str();
    if !is_alkharid_pay_choice(text) {
        return None;
    }
    for &(id, count) in &edge.consumed_req {
        if snapshot.inv_count(id) < count {
            return Some(format!("need {count} coins to pay the Al Kharid toll"));
        }
    }
    None
}

pub(super) fn spirit_tree_choice(
    leg: &Leg,
    edge: &TransportEdge,
    packed: Option<&[TransportEdge]>,
    chat_options: &[api::snapshot::ChatOptionView],
) -> i32 {
    let n_dests = spirit_tree_dest_count(edge, packed);
    if n_dests == 1 {
        return NPC_RIDE_CHOICE;
    }
    if chat_options.len() >= 3 {
        return dest_dialog_choice(leg, None, packed);
    }
    2
}

pub(super) fn spirit_tree_dest_count(
    edge: &TransportEdge,
    packed: Option<&[TransportEdge]>,
) -> usize {
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
#[cfg(test)]
#[path = "dialog_tests.rs"]
mod tests;
