//! Per-row effects for the legal named outbound packets.
//!
//! [`effects`] accepts a validated [`Send`](super::Send), not a selected
//! [`ClientProt`](client::io::ClientProt). `Send` retains the named packet identity while
//! `write_for_revision` maps that identity to the selected wire row. This keeps revision-
//! specific opcode and length collisions out of the effects lookup.
//!
//! [`Effect`] distinguishes proven effects, proven absence, and outcomes that depend on
//! content scripts. A content-dependent effect is never reported as [`Effect::No`].
//! [`PacketEffects`] describes engine-reachable effects, not effects guaranteed by every
//! send. The two punishment marks below also include wire-valid but protocol-invalid
//! inputs.
//!
//! The table includes two server-side punishment paths that typed builders refuse:
//! malformed `REPORT_ABUSE` reasons and invalid-base-37 `MESSAGE_PRIVATE` names request
//! a 48-hour sender ban. These remain relevant when describing the engine handler,
//! although validated `Send` values cannot encode those malformed inputs.
//!
//! Citations use paths below `/Users/acfrazier/experiments/lostcity-289/engine/src`.
//! Handler files are in `network/game/client/handler`. The registry is
//! `network/game/client/ClientGameProtRepository.ts`. `Player.ts` and
//! `NetworkPlayer.ts` are in `engine/entity`, `InputTracking.ts` is in
//! `engine/entity/tracking`, and `World.ts` is in `engine`.

use super::Send;
use client::io::ClientProt;

/// Evidence status for one possible server-side effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// The engine handler can produce the effect for some input.
    ///
    /// The `REPORT_ABUSE` and `MESSAGE_PRIVATE` punishment paths also include documented
    /// wire-valid but protocol-invalid values. This does not mean every send causes the
    /// effect.
    Yes,
    /// The engine handler path establishes that this effect cannot occur.
    No,
    /// A content script may or may not produce this effect.
    ///
    /// Consumers must not treat this as absence or as a guaranteed effect.
    DependsOnContent,
}

/// What the engine can do with one named outbound packet.
///
/// Each field classifies what the engine can do with this packet. `Effect::Yes` means
/// the outcome is possible, not guaranteed for every send. The two punishment marks for
/// malformed values are wire-valid but protocol-invalid. Content-dispatched rows use
/// [`Effect::DependsOnContent`] for effects not proven directly by their handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketEffects {
    /// Whether this packet can cause an account-level punishment.
    ///
    /// The punishment may target the sender or another account.
    pub account_punishment: Effect,
    /// Whether this packet can mutate account state that survives the session.
    pub persistent: Effect,
    /// Whether this packet can act on or deliver an effect to another account.
    pub cross_player: Effect,
    /// Whether this packet can end the sender's or another account's session.
    pub logout: Effect,
}

const NO_EFFECTS: PacketEffects = PacketEffects {
    account_punishment: Effect::No,
    persistent: Effect::No,
    cross_player: Effect::No,
    logout: Effect::No,
};

const CONTENT_DEPENDENT: PacketEffects = PacketEffects {
    account_punishment: Effect::DependsOnContent,
    persistent: Effect::DependsOnContent,
    cross_player: Effect::DependsOnContent,
    logout: Effect::DependsOnContent,
};

const PLAYER_CONTENT_DEPENDENT: PacketEffects = PacketEffects {
    account_punishment: Effect::DependsOnContent,
    persistent: Effect::DependsOnContent,
    cross_player: Effect::Yes,
    logout: Effect::DependsOnContent,
};

const LOGOUT_ONLY: PacketEffects = PacketEffects {
    logout: Effect::Yes,
    ..NO_EFFECTS
};

const PERSISTENT_AND_CROSS_PLAYER: PacketEffects = PacketEffects {
    persistent: Effect::Yes,
    cross_player: Effect::Yes,
    ..NO_EFFECTS
};

const PUBLIC_MESSAGE: PacketEffects = PacketEffects {
    cross_player: Effect::Yes,
    ..NO_EFFECTS
};

const ALL_EFFECTS: PacketEffects = PacketEffects {
    account_punishment: Effect::Yes,
    persistent: Effect::Yes,
    cross_player: Effect::Yes,
    logout: Effect::Yes,
};

/// Per-row [`PacketEffects`] for all 82 legal named packets, in
/// [`LEGAL_SEND`](super::LEGAL_SEND) order.
///
/// Each row's note records why its `No` values are supported or why its outcomes depend
/// on content. The cited engine files are read-only source evidence.
const KNOWN: [(ClientProt, PacketEffects); 82] = [
    // No: no decoder is bound for NO_TIMEOUT (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` consumes a packet only through a registered decoder.
    (ClientProt::NO_TIMEOUT, NO_EFFECTS),
    // No for punishment, persistence, and cross-player effects: the handler only sets
    // `requestIdleLogout` (`IdleTimerHandler.ts:7-13`).
    (ClientProt::IDLE_TIMER, LOGOUT_ONLY),
    // No for all four effects: this only queues session input telemetry
    // (`EventMouseClickHandler.ts:6-9`, `InputTracking.ts:75-86`).
    (ClientProt::EVENT_MOUSE_CLICK, NO_EFFECTS),
    // No for all four effects: this only queues session input telemetry
    // (`EventMouseMoveHandler.ts:6-9`, `InputTracking.ts:88-100`).
    (ClientProt::EVENT_MOUSE_MOVE, NO_EFFECTS),
    // No for all four effects: this only queues session input telemetry
    // (`EventAppletFocusHandler.ts:6-9`, `InputTracking.ts:62-73`).
    (ClientProt::EVENT_APPLET_FOCUS, NO_EFFECTS),
    // No for all four effects: this only queues session input telemetry
    // (`EventCameraPositionHandler.ts:6-9`, `InputTracking.ts:48-60`).
    (ClientProt::EVENT_CAMERA_POSITION, NO_EFFECTS),
    // No: no decoder is bound for ANTICHEAT_OPLOGIC1 (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::ANTICHEAT_OPLOGIC1, NO_EFFECTS),
    // No: no decoder is bound for ANTICHEAT_OPLOGIC2 (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::ANTICHEAT_OPLOGIC2, NO_EFFECTS),
    // No: no decoder is bound for ANTICHEAT_OPLOGIC3 (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::ANTICHEAT_OPLOGIC3, NO_EFFECTS),
    // No: no decoder is bound for ANTICHEAT_OPLOGIC4 (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::ANTICHEAT_OPLOGIC4, NO_EFFECTS),
    // No: no decoder is bound for ANTICHEAT_OPLOGIC5 (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::ANTICHEAT_OPLOGIC5, NO_EFFECTS),
    // No: no decoder is bound for ANTICHEAT_OPLOGIC6 (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::ANTICHEAT_OPLOGIC6, NO_EFFECTS),
    // No: no decoder is bound for ANTICHEAT_OPLOGIC7 (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::ANTICHEAT_OPLOGIC7, NO_EFFECTS),
    // No: no decoder is bound for ANTICHEAT_OPLOGIC8 (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::ANTICHEAT_OPLOGIC8, NO_EFFECTS),
    // No: no decoder is bound for ANTICHEAT_OPLOGIC9 (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::ANTICHEAT_OPLOGIC9, NO_EFFECTS),
    // No: no decoder is bound for ANTICHEAT_CYCLELOGIC1 (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::ANTICHEAT_CYCLELOGIC1, NO_EFFECTS),
    // No: no decoder is bound for ANTICHEAT_CYCLELOGIC2 (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::ANTICHEAT_CYCLELOGIC2, NO_EFFECTS),
    // No: no decoder is bound for ANTICHEAT_CYCLELOGIC3 (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::ANTICHEAT_CYCLELOGIC3, NO_EFFECTS),
    // No: no decoder is bound for ANTICHEAT_CYCLELOGIC4 (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::ANTICHEAT_CYCLELOGIC4, NO_EFFECTS),
    // No: no decoder is bound for ANTICHEAT_CYCLELOGIC5 (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::ANTICHEAT_CYCLELOGIC5, NO_EFFECTS),
    // No: no decoder is bound for ANTICHEAT_CYCLELOGIC6 (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::ANTICHEAT_CYCLELOGIC6, NO_EFFECTS),
    // No: no decoder is bound for ANTICHEAT_CYCLELOGIC7 (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::ANTICHEAT_CYCLELOGIC7, NO_EFFECTS),
    // Depends on content: the handler schedules the object option trigger
    // (`OpObjHandler.ts:44-48`), executed as a script by `Player.ts:1165-1176`.
    (ClientProt::OPOBJ1, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the object option trigger
    // (`OpObjHandler.ts:44-48`), executed as a script by `Player.ts:1165-1176`.
    (ClientProt::OPOBJ2, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the object option trigger
    // (`OpObjHandler.ts:44-48`), executed as a script by `Player.ts:1165-1176`.
    (ClientProt::OPOBJ3, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the object option trigger
    // (`OpObjHandler.ts:44-48`), executed as a script by `Player.ts:1165-1176`.
    (ClientProt::OPOBJ4, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the object option trigger
    // (`OpObjHandler.ts:44-48`), executed as a script by `Player.ts:1165-1176`.
    (ClientProt::OPOBJ5, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the spell-on-object trigger
    // (`OpObjTHandler.ts:48-51`), executed by `Player.ts:1165-1176`.
    (ClientProt::OPOBJT, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the item-on-object trigger
    // (`OpObjUHandler.ts:76-81`), executed by `Player.ts:1165-1176`.
    (ClientProt::OPOBJU, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the NPC option trigger
    // (`OpNpcHandler.ts:46-50`), executed as a script by `Player.ts:1165-1176`.
    (ClientProt::OPNPC1, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the NPC option trigger
    // (`OpNpcHandler.ts:46-50`), executed as a script by `Player.ts:1165-1176`.
    (ClientProt::OPNPC2, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the NPC option trigger
    // (`OpNpcHandler.ts:46-50`), executed as a script by `Player.ts:1165-1176`.
    (ClientProt::OPNPC3, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the NPC option trigger
    // (`OpNpcHandler.ts:46-50`), executed as a script by `Player.ts:1165-1176`.
    (ClientProt::OPNPC4, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the NPC option trigger
    // (`OpNpcHandler.ts:46-50`), executed as a script by `Player.ts:1165-1176`.
    (ClientProt::OPNPC5, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the spell-on-NPC trigger
    // (`OpNpcTHandler.ts:50-53`), executed by `Player.ts:1165-1176`.
    (ClientProt::OPNPCT, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the item-on-NPC trigger
    // (`OpNpcUHandler.ts:78-83`), executed by `Player.ts:1165-1176`.
    (ClientProt::OPNPCU, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the location option trigger
    // (`OpLocHandler.ts:52-56`), executed as a script by `Player.ts:1165-1176`.
    (ClientProt::OPLOC1, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the location option trigger
    // (`OpLocHandler.ts:52-56`), executed as a script by `Player.ts:1165-1176`.
    (ClientProt::OPLOC2, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the location option trigger
    // (`OpLocHandler.ts:52-56`), executed as a script by `Player.ts:1165-1176`.
    (ClientProt::OPLOC3, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the location option trigger
    // (`OpLocHandler.ts:52-56`), executed as a script by `Player.ts:1165-1176`.
    (ClientProt::OPLOC4, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the location option trigger
    // (`OpLocHandler.ts:52-56`), executed as a script by `Player.ts:1165-1176`.
    (ClientProt::OPLOC5, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the spell-on-location trigger
    // (`OpLocTHandler.ts:48-51`), executed by `Player.ts:1165-1176`.
    (ClientProt::OPLOCT, CONTENT_DEPENDENT),
    // Depends on content: the handler schedules the item-on-location trigger
    // (`OpLocUHandler.ts:76-81`), executed by `Player.ts:1165-1176`.
    (ClientProt::OPLOCU, CONTENT_DEPENDENT),
    // Cross-player is direct; the operation trigger is content-dependent
    // (`OpPlayerHandler.ts:34-40`, `Player.ts:1165-1176`).
    (ClientProt::OPPLAYER1, PLAYER_CONTENT_DEPENDENT),
    // Cross-player is direct; the operation trigger is content-dependent
    // (`OpPlayerHandler.ts:34-40`, `Player.ts:1165-1176`).
    (ClientProt::OPPLAYER2, PLAYER_CONTENT_DEPENDENT),
    // Cross-player is direct; the operation trigger is content-dependent
    // (`OpPlayerHandler.ts:34-40`, `Player.ts:1165-1176`).
    (ClientProt::OPPLAYER3, PLAYER_CONTENT_DEPENDENT),
    // Cross-player is direct; the operation trigger is content-dependent
    // (`OpPlayerHandler.ts:34-40`, `Player.ts:1165-1176`).
    (ClientProt::OPPLAYER4, PLAYER_CONTENT_DEPENDENT),
    // Cross-player is direct; the operation trigger is content-dependent
    // (`OpPlayerHandler.ts:34-40`, `Player.ts:1165-1176`).
    (ClientProt::OPPLAYER5, PLAYER_CONTENT_DEPENDENT),
    // Cross-player is direct; the spell-on-player trigger is content-dependent
    // (`OpPlayerTHandler.ts:46-49`, `Player.ts:1165-1176`).
    (ClientProt::OPPLAYERT, PLAYER_CONTENT_DEPENDENT),
    // Cross-player is direct; the item-on-player trigger is content-dependent
    // (`OpPlayerUHandler.ts:74-79`, `Player.ts:1165-1176`).
    (ClientProt::OPPLAYERU, PLAYER_CONTENT_DEPENDENT),
    // Depends on content: inventory operations resolve their configured script
    // (`OpHeldHandler.ts:65-73`).
    (ClientProt::OPHELD1, CONTENT_DEPENDENT),
    // Depends on content: inventory operations resolve their configured script
    // (`OpHeldHandler.ts:65-73`).
    (ClientProt::OPHELD2, CONTENT_DEPENDENT),
    // Depends on content: inventory operations resolve their configured script
    // (`OpHeldHandler.ts:65-73`).
    (ClientProt::OPHELD3, CONTENT_DEPENDENT),
    // Depends on content: inventory operations resolve their configured script
    // (`OpHeldHandler.ts:65-73`).
    (ClientProt::OPHELD4, CONTENT_DEPENDENT),
    // Depends on content: inventory operations resolve their configured script
    // (`OpHeldHandler.ts:65-73`).
    (ClientProt::OPHELD5, CONTENT_DEPENDENT),
    // Depends on content: the spell-on-held handler resolves and executes a script
    // (`OpHeldTHandler.ts:61-73`).
    (ClientProt::OPHELDT, CONTENT_DEPENDENT),
    // Depends on content: the item-on-held handler resolves and executes a script
    // (`OpHeldUHandler.ts:93-118`).
    (ClientProt::OPHELDU, CONTENT_DEPENDENT),
    // Depends on content: each inventory option resolves a content script
    // (`InvButtonHandler.ts:51-60`).
    (ClientProt::INV_BUTTON1, CONTENT_DEPENDENT),
    // Depends on content: each inventory option resolves a content script
    // (`InvButtonHandler.ts:51-60`).
    (ClientProt::INV_BUTTON2, CONTENT_DEPENDENT),
    // Depends on content: each inventory option resolves a content script
    // (`InvButtonHandler.ts:51-60`).
    (ClientProt::INV_BUTTON3, CONTENT_DEPENDENT),
    // Depends on content: each inventory option resolves a content script
    // (`InvButtonHandler.ts:51-60`).
    (ClientProt::INV_BUTTON4, CONTENT_DEPENDENT),
    // Depends on content: each inventory option resolves a content script
    // (`InvButtonHandler.ts:51-60`).
    (ClientProt::INV_BUTTON5, CONTENT_DEPENDENT),
    // Depends on content: the visible button resumes or selects an interface script
    // (`IfButtonHandler.ts:24-37`).
    (ClientProt::IF_BUTTON, CONTENT_DEPENDENT),
    // Depends on content: this resumes a paused script
    // (`ResumePauseButtonHandler.ts:7-13`).
    (ClientProt::RESUME_PAUSEBUTTON, CONTENT_DEPENDENT),
    // Depends on content: deferred modal close can run IF_CLOSE scripts and cancel
    // suspended scripts (`CloseModalHandler.ts:6-14`, `Player.ts:760-807,874-885`).
    (ClientProt::CLOSE_MODAL, CONTENT_DEPENDENT),
    // Depends on content: the count value is assigned to and resumes the active script
    // (`ResumePCountDialogHandler.ts:7-16`).
    (ClientProt::RESUME_P_COUNTDIALOG, CONTENT_DEPENDENT),
    // Depends on content: the tutorial tab handler dispatches the tutorial script
    // (`TutClickSideHandler.ts:16-21`).
    (ClientProt::TUT_CLICKSIDE, CONTENT_DEPENDENT),
    // No: no decoder is bound for MAP_BUILD_COMPLETE (`ClientGameProtRepository.ts:108-172`);
    // `NetworkPlayer.ts:132-149` skips unregistered packet dispatch.
    (ClientProt::MAP_BUILD_COMPLETE, NO_EFFECTS),
    // Depends on content: movement can run walk triggers
    // (`MoveClickHandler.ts:41-54`, `Player.ts:1108-1116`).
    (ClientProt::MOVE_OPCLICK, CONTENT_DEPENDENT),
    // Yes: invalid reasons ban and log out the sender; accepted reports can persist a
    // report or mute and affect the named offender (`ReportAbuseHandler.ts:14-27`).
    (ClientProt::REPORT_ABUSE, ALL_EFFECTS),
    // Depends on content: movement can run walk triggers
    // (`MoveClickHandler.ts:41-54`, `Player.ts:1108-1116`).
    (ClientProt::MOVE_MINIMAPCLICK, CONTENT_DEPENDENT),
    // Depends on content: drag options dispatch an inventory script
    // (`InvButtonDHandler.ts:46-57`).
    (ClientProt::INV_BUTTOND, CONTENT_DEPENDENT),
    // No for punishment and logout: the handler only updates the persistent ignore list
    // (`IgnoreListDelHandler.ts:8-15`, `World.ts:1576-1583`).
    (ClientProt::IGNORELIST_DEL, PERSISTENT_AND_CROSS_PLAYER),
    // No for punishment and logout: the handler only updates the persistent ignore list
    // (`IgnoreListAddHandler.ts:8-15`, `World.ts:1567-1574`).
    (ClientProt::IGNORELIST_ADD, PERSISTENT_AND_CROSS_PLAYER),
    // No for punishment and logout: this saves the sender's appearance
    // (`IdkSaveDesignHandler.ts:52-56`) and broadcasts it through the appearance mask
    // (`Player.ts:1917-1920`).
    (ClientProt::IDK_SAVEDESIGN, PERSISTENT_AND_CROSS_PLAYER),
    // No for punishment and logout: this stores chat modes and notifies the friend
    // service (`ChatSetModeHandler.ts:7-13`); save persists those modes (`Player.ts:266-267`).
    (ClientProt::CHAT_SETMODE, PERSISTENT_AND_CROSS_PLAYER),
    // Yes: malformed names ban the sender (`MessagePrivateHandler.ts:22-24`),
    // persisting `account.banned_until` (`server/login/LoginServer.ts:492-503`).
    // Valid messages reach another player through the friend service (`World.ts:1647-1659`).
    (ClientProt::MESSAGE_PRIVATE, ALL_EFFECTS),
    // No for punishment and logout: this updates the persistent friend list
    // (`FriendListDelHandler.ts:8-15`, `World.ts:1558-1565`).
    (ClientProt::FRIENDLIST_DEL, PERSISTENT_AND_CROSS_PLAYER),
    // No for punishment and logout: this updates the persistent friend list
    // (`FriendListAddHandler.ts:8-15`, `World.ts:1549-1556`).
    (ClientProt::FRIENDLIST_ADD, PERSISTENT_AND_CROSS_PLAYER),
    // Yes: staff-gated commands can save account changes, target other users, punish,
    // and terminate sessions (`ClientCheatHandler.ts:39-55,445-448,499-502,666-698`).
    (ClientProt::CLIENT_CHEAT, ALL_EFFECTS),
    // No for punishment, persistence, and logout: this updates transient chat state and
    // publishes the message to nearby players (`MessagePublicHandler.ts:14-43`).
    (ClientProt::MESSAGE_PUBLIC, PUBLIC_MESSAGE),
    // Depends on content: movement can run walk triggers
    // (`MoveClickHandler.ts:41-54`, `Player.ts:1108-1116`).
    (ClientProt::MOVE_GAMECLICK, CONTENT_DEPENDENT),
];

/// Server-side effects for one validated named packet.
///
/// Looks up the identity retained by `send`, before revision selection. The selected
/// revision's raw opcode and length are never accepted by this API. Returns `None` only
/// if a future `Send` identity is absent from the closed metadata table.
///
/// ```compile_fail
/// use api::prot::effects::effects;
/// use client::io::{map_client_prot, ClientProt, ClientRevision};
///
/// let selected = map_client_prot(ClientRevision::R289, ClientProt::OPPLAYER4);
/// let _ = effects(selected);
/// ```
pub fn effects(send: &Send) -> Option<PacketEffects> {
    let named = send.named_packet();
    KNOWN
        .iter()
        .find(|(prot, _)| *prot == named)
        .map(|(_, row)| *row)
}
#[cfg(test)]
mod tests {
    use super::super::{Body, Send, LEGAL_SEND};
    use super::{effects, Effect, PacketEffects, KNOWN};
    use client::io::{map_client_prot, ClientProt, ClientRevision};

    fn named_send(named: ClientProt) -> Send {
        Send {
            named,
            body: Body::Inline {
                bytes: [0; 67],
                length: 0,
            },
            catalog_revision: None,
        }
    }

    fn effect_values(row: PacketEffects) -> [Effect; 4] {
        [
            row.account_punishment,
            row.persistent,
            row.cross_player,
            row.logout,
        ]
    }

    #[test]
    fn covers_every_legal_send_row_in_order() {
        assert_eq!(KNOWN.len(), LEGAL_SEND.len());
        for ((named, _), legal) in KNOWN.iter().zip(LEGAL_SEND) {
            assert_eq!(named.id, legal.id, "opcode for {named:?}");
            assert_eq!(named.length, legal.length, "length for {named:?}");
        }
    }

    #[test]
    fn revision_289_opplayer4_keeps_its_named_effects() {
        let send = Send::opplayer4(42).expect("valid player index");
        let selected = map_client_prot(ClientRevision::R289, send.named_packet());
        assert_eq!(selected, ClientProt::OPNPC5);
        assert_eq!((selected.id, selected.length), (189, 2));

        let row = effects(&send).expect("OPPLAYER4 is known");
        assert_eq!(row.cross_player, Effect::Yes);
        assert_eq!(row.account_punishment, Effect::DependsOnContent);

        let npc = Send::opnpc5(42).expect("valid NPC index");
        assert_eq!(
            effects(&npc).expect("OPNPC5 is known").cross_player,
            Effect::DependsOnContent
        );
    }

    #[test]
    fn revision_289_resume_pausebutton_keeps_its_named_effects() {
        let send = Send::resume_pausebutton(42).expect("valid component");
        let selected = map_client_prot(ClientRevision::R289, send.named_packet());
        assert_eq!(selected, ClientProt::OPPLAYER2);
        assert_eq!((selected.id, selected.length), (166, 2));

        assert_eq!(
            effects(&send)
                .expect("RESUME_PAUSEBUTTON is known")
                .cross_player,
            Effect::DependsOnContent
        );
        let player = Send::opplayer2(42).expect("valid player index");
        assert_eq!(
            effects(&player).expect("OPPLAYER2 is known").cross_player,
            Effect::Yes
        );
    }

    #[test]
    fn every_revision_mapping_keeps_all_effects_bound_to_the_named_send() {
        let mut cross_revision_aliases = 0;

        for (named, expected) in KNOWN {
            let send = named_send(named);
            for revision in [ClientRevision::R274, ClientRevision::R289] {
                let selected = map_client_prot(revision, named);
                assert_eq!(send.named_packet(), named);
                assert_eq!(effects(&send), Some(expected), "{named:?} at {revision:?}");

                if revision == ClientRevision::R289
                    && selected != named
                    && KNOWN
                        .iter()
                        .any(|(other, _)| *other == selected && *other != named)
                {
                    cross_revision_aliases += 1;
                    assert_eq!(
                        effects(&send),
                        Some(expected),
                        "selected row {selected:?} must not alias {named:?}"
                    );
                }
            }
        }

        assert!(
            cross_revision_aliases >= 2,
            "exercise the known revision-289 wire aliases"
        );
    }

    #[test]
    fn report_abuse_marks_each_proven_effect() {
        let send = named_send(ClientProt::REPORT_ABUSE);
        assert_eq!(
            effect_values(effects(&send).expect("REPORT_ABUSE is known")),
            [Effect::Yes; 4]
        );
    }

    #[test]
    fn private_message_marks_punishment_persistence_delivery_and_logout() {
        let send = named_send(ClientProt::MESSAGE_PRIVATE);
        let row = effects(&send).expect("MESSAGE_PRIVATE is known");
        assert_eq!(row.account_punishment, Effect::Yes);
        assert_eq!(row.persistent, Effect::Yes);
        assert_eq!(row.cross_player, Effect::Yes);
        assert_eq!(row.logout, Effect::Yes);
    }

    #[test]
    fn design_save_marks_persistent_appearance_broadcast() {
        let send = named_send(ClientProt::IDK_SAVEDESIGN);
        let row = effects(&send).expect("IDK_SAVEDESIGN is known");
        assert_eq!(row.account_punishment, Effect::No);
        assert_eq!(row.persistent, Effect::Yes);
        assert_eq!(row.cross_player, Effect::Yes);
        assert_eq!(row.logout, Effect::No);
    }

    #[test]
    fn cheat_marks_each_proven_effect() {
        let send = named_send(ClientProt::CLIENT_CHEAT);
        assert_eq!(
            effect_values(effects(&send).expect("CLIENT_CHEAT is known")),
            [Effect::Yes; 4]
        );
    }

    #[test]
    fn player_operations_mark_targeting_and_content_effects() {
        for named in [
            ClientProt::OPPLAYER1,
            ClientProt::OPPLAYER2,
            ClientProt::OPPLAYER3,
            ClientProt::OPPLAYER4,
            ClientProt::OPPLAYER5,
            ClientProt::OPPLAYERT,
            ClientProt::OPPLAYERU,
        ] {
            let row = effects(&named_send(named)).expect("player operation is known");
            assert_eq!(row.cross_player, Effect::Yes, "{named:?}");
            assert_eq!(
                row.account_punishment,
                Effect::DependsOnContent,
                "{named:?}"
            );
            assert_eq!(row.persistent, Effect::DependsOnContent, "{named:?}");
            assert_eq!(row.logout, Effect::DependsOnContent, "{named:?}");
        }
    }

    #[test]
    fn content_dispatched_rows_never_silently_report_no() {
        for named in [
            ClientProt::OPOBJ1,
            ClientProt::OPOBJ2,
            ClientProt::OPOBJ3,
            ClientProt::OPOBJ4,
            ClientProt::OPOBJ5,
            ClientProt::OPOBJT,
            ClientProt::OPOBJU,
            ClientProt::OPNPC1,
            ClientProt::OPNPC2,
            ClientProt::OPNPC3,
            ClientProt::OPNPC4,
            ClientProt::OPNPC5,
            ClientProt::OPNPCT,
            ClientProt::OPNPCU,
            ClientProt::OPLOC1,
            ClientProt::OPLOC2,
            ClientProt::OPLOC3,
            ClientProt::OPLOC4,
            ClientProt::OPLOC5,
            ClientProt::OPLOCT,
            ClientProt::OPLOCU,
            ClientProt::OPPLAYER1,
            ClientProt::OPPLAYER2,
            ClientProt::OPPLAYER3,
            ClientProt::OPPLAYER4,
            ClientProt::OPPLAYER5,
            ClientProt::OPPLAYERT,
            ClientProt::OPPLAYERU,
            ClientProt::OPHELD1,
            ClientProt::OPHELD2,
            ClientProt::OPHELD3,
            ClientProt::OPHELD4,
            ClientProt::OPHELD5,
            ClientProt::OPHELDT,
            ClientProt::OPHELDU,
            ClientProt::INV_BUTTON1,
            ClientProt::INV_BUTTON2,
            ClientProt::INV_BUTTON3,
            ClientProt::INV_BUTTON4,
            ClientProt::INV_BUTTON5,
            ClientProt::INV_BUTTOND,
            ClientProt::IF_BUTTON,
            ClientProt::RESUME_PAUSEBUTTON,
            ClientProt::CLOSE_MODAL,
            ClientProt::RESUME_P_COUNTDIALOG,
            ClientProt::TUT_CLICKSIDE,
            ClientProt::MOVE_GAMECLICK,
            ClientProt::MOVE_OPCLICK,
            ClientProt::MOVE_MINIMAPCLICK,
        ] {
            let row = effects(&named_send(named)).expect("content row is known");
            let fields = effect_values(row);
            assert!(
                fields.iter().all(|field| *field != Effect::No),
                "{named:?} contains an unproven No: {fields:?}"
            );
            assert!(
                fields.contains(&Effect::DependsOnContent),
                "{named:?} has no content-dependent classification: {fields:?}"
            );
        }
    }

    #[test]
    fn idle_timer_has_only_a_proven_logout_effect() {
        let send = named_send(ClientProt::IDLE_TIMER);
        let row = effects(&send).expect("IDLE_TIMER is known");
        assert_eq!(row.account_punishment, Effect::No);
        assert_eq!(row.persistent, Effect::No);
        assert_eq!(row.cross_player, Effect::No);
        assert_eq!(row.logout, Effect::Yes);
    }

    #[test]
    fn unlisted_named_send_returns_none() {
        let unknown = named_send(ClientProt { id: -1, length: -1 });
        assert_eq!(effects(&unknown), None);
    }
}
