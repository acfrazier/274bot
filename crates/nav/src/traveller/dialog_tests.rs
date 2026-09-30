use super::*;

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn edge(kind: TransportKind) -> TransportEdge {
    TransportEdge {
        kind,
        at: tile(10, 10),
        to: tile(20, 20),
        loc_id: 1,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: Vec::new(),
        item_req: Vec::new(),
        quest_req: Vec::new(),
        varp_req: Vec::new(),
        worn_req: Vec::new(),
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

fn options(texts: &[&str]) -> Vec<api::snapshot::ChatOptionView> {
    texts
        .iter()
        .enumerate()
        .map(|(index, text)| api::snapshot::ChatOptionView {
            component_id: index as i32 + 1,
            text: (*text).to_string(),
        })
        .collect()
}

fn choice(kind: TransportKind, texts: &[&str]) -> Option<i32> {
    let leg = Leg::Transport { edge: edge(kind) };
    let chat_options = options(texts);
    hop_dialog_choice(&leg, None, None, &chat_options)
}

#[test]
fn npc_dialog_sailor_crandor_branch_selects_ride_option() {
    assert_eq!(
        choice(
            TransportKind::Boat,
            &[
                "I'd rather go to Crandor Isle.",
                "Yes please.",
                "No, thank you.",
            ],
        ),
        Some(2)
    );
}

#[test]
fn npc_dialog_sailor_without_crandor_selects_first_fare_option() {
    assert_eq!(
        choice(TransportKind::Boat, &["Yes please.", "No, thank you."]),
        Some(1)
    );
}

#[test]
fn npc_dialog_entrana_boats_select_ready_branch() {
    assert_eq!(
        choice(
            TransportKind::Boat,
            &["Yes, I'm ready to go.", "Not just yet."],
        ),
        Some(1)
    );
    assert_eq!(
        choice(
            TransportKind::Boat,
            &["No, not right now.", "Yes, okay, I'm ready to go."],
        ),
        Some(2)
    );
}

#[test]
fn npc_dialog_customs_selects_journey_search_and_fare_branches() {
    assert_eq!(
        choice(
            TransportKind::Boat,
            &[
                "Can I journey on this ship?",
                "Does Karamja have unusual customs then?",
            ],
        ),
        Some(1)
    );
    assert_eq!(
        choice(
            TransportKind::Boat,
            &[
                "Why?",
                "Search away, I have nothing to hide.",
                "You're not putting your hands on my things!",
            ],
        ),
        Some(2)
    );
    assert_eq!(
        choice(TransportKind::Boat, &["Ok.", "Oh, I'll not bother then."]),
        Some(1)
    );
    assert_eq!(
        choice(
            TransportKind::Boat,
            &[
                "Search away, I have nothing to hide.",
                "You're not putting your hands on my things!",
            ],
        ),
        Some(1)
    );
}

#[test]
fn npc_dialog_cart_and_elkoy_select_affirmative_branches() {
    assert_eq!(
        choice(
            TransportKind::Npc,
            &["Yes please, I'd like to go to Shilo Village.", "No thanks.",],
        ),
        Some(1)
    );
    assert_eq!(
        choice(
            TransportKind::Npc,
            &["Can you show me out of the village?", "Okay."],
        ),
        Some(1)
    );
}

#[test]
fn npc_dialog_glider_selects_ride_request() {
    assert_eq!(
        choice(
            TransportKind::Glider,
            &[
                "Can you take me on the glider?",
                "Why are gliders better than other transport?",
                "Sorry, I don't want anything now.",
            ],
        ),
        Some(1)
    );
}

#[test]
fn npc_dialog_unknown_destination_page_fails_closed() {
    assert_eq!(
        choice(
            TransportKind::Boat,
            &[
                "Khazard Port please.",
                "Port Sarim please.",
                "Nowhere just at the moment thanks.",
            ],
        ),
        None
    );
    assert_eq!(
        choice(
            TransportKind::Boat,
            &["Yes please.", "Yes please.", "No, thank you."],
        ),
        None
    );
    assert_eq!(
        choice(TransportKind::Boat, &["An unexpected route.", "No thanks."]),
        None
    );
}

#[test]
fn npc_dialog_shanks_uses_packed_destination_edge() {
    let khazard_to = WorldTile {
        x: 2680,
        z: 3150,
        level: 0,
    };
    let sarim_to = WorldTile {
        x: 3047,
        z: 3235,
        level: 0,
    };
    let mut khazard = edge(TransportKind::Boat);
    khazard.loc_id = 518;
    khazard.at = WorldTile {
        x: 2763,
        z: 2961,
        level: 1,
    };
    khazard.to = khazard_to;
    let mut sarim = khazard.clone();
    sarim.to = sarim_to;
    let packed = vec![khazard.clone(), sarim.clone()];
    let chat_options = options(&[
        "Khazard Port please.",
        "Port Sarim please.",
        "Nowhere just at the moment thanks.",
    ]);

    assert_eq!(
        hop_dialog_choice(
            &Leg::Transport {
                edge: khazard.clone(),
            },
            None,
            Some(&packed),
            &chat_options,
        ),
        Some(1)
    );
    assert_eq!(
        hop_dialog_choice(
            &Leg::Transport { edge: sarim },
            None,
            Some(&packed),
            &chat_options,
        ),
        Some(2)
    );
    assert_eq!(
        hop_dialog_choice(
            &Leg::Transport { edge: khazard },
            None,
            Some(&packed),
            &options(&[
                "Yes, I'll buy a ticket for the ship.",
                "No thanks, not just at the moment.",
            ]),
        ),
        Some(1)
    );
}

#[test]
fn hop_dialog_choice_preserves_jewellery_destination_index() {
    let first = edge(TransportKind::Teleport);
    let mut second = edge(TransportKind::Teleport);
    second.to = tile(30, 30);
    let leg = Leg::Transport {
        edge: second.clone(),
    };
    let teleports = vec![first, second];
    let chat_options = options(&["Edgeville.", "Karamja."]);

    assert_eq!(
        hop_dialog_choice(&leg, Some(&teleports), None, &chat_options),
        Some(2)
    );
}

#[test]
fn hop_dialog_choice_preserves_spirit_tree_gate_choice() {
    let mut edge_to = edge(TransportKind::SpiritTree);
    edge_to.to = tile(30, 30);
    let mut other = edge(TransportKind::SpiritTree);
    other.to = tile(40, 40);
    let leg = Leg::Transport { edge: edge_to };
    let packed = vec![edge(TransportKind::SpiritTree), other];
    let chat_options = options(&["No thanks, old tree.", "Where can I go?"]);

    assert_eq!(
        hop_dialog_choice(&leg, None, Some(&packed), &chat_options),
        Some(2)
    );
}

#[test]
fn hop_dialog_choice_rides_the_young_spirit_tree_single_destination() {
    // The young tree (loc 1317) has one destination, so its dialogue is a
    // yes/no page and the ride is choice 1, never "No thank you".
    let mut young = edge(TransportKind::SpiritTree);
    young.loc_id = 1317;
    let leg = Leg::Transport {
        edge: young.clone(),
    };
    let packed = vec![young];
    let chat_options = options(&["Yes please.", "No thank you."]);

    assert_eq!(
        hop_dialog_choice(&leg, None, Some(&packed), &chat_options),
        Some(1)
    );
}

#[test]
fn door_dialog_alkharid_pay_selects_yes_ok_not_the_refuse_branch() {
    assert_eq!(
        choice(
            TransportKind::Door,
            &[
                "No thank you, I'll walk around.",
                "Who does my money go to?",
                "Yes, ok.",
            ],
        ),
        Some(3)
    );
}

#[test]
fn door_dialog_shantay_disclaimer_selects_enter_regardless_of_order() {
    assert_eq!(
        choice(
            TransportKind::Door,
            &[
                "No, I'm having serious second thoughts now.",
                "Yeah, that poster doesn't scare me!",
            ],
        ),
        Some(2)
    );
}

#[test]
fn door_dialog_unknown_or_duplicate_page_fails_closed() {
    assert_eq!(choice(TransportKind::Door, &["Hello.", "Goodbye."]), None);
    assert_eq!(choice(TransportKind::Door, &["Yes, ok.", "Yes, ok."]), None);
}
