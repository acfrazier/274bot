//! Legal outbound send table: every `ClientProt` from `client_prot.rs` has
//! a `LEGAL_SEND` row (opcode id + fixed length, `-1` variable). The typed
//! `Send` constructors are the builders the kernel and host use; writes go
//! through [`Out`] (ISAAC-encrypted opcode + plaintext payload), never a
//! raw opcode inject. Anticheat/event packets are `pub` in the table but
//! unused by the kernel.

use client::io::{ClientProt, Packet};

/// One legal outbound packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegalSend {
    pub id: i32,
    pub length: i32,
}

macro_rules! legal_send {
    ($($name:ident),* $(,)?) => {
        pub const LEGAL_SEND: &[LegalSend] = &[
            $(LegalSend {
                id: ClientProt::$name.id,
                length: ClientProt::$name.length,
            }),*
        ];
    };
}

legal_send!(
    NO_TIMEOUT,
    IDLE_TIMER,
    EVENT_MOUSE_CLICK,
    EVENT_MOUSE_MOVE,
    EVENT_APPLET_FOCUS,
    EVENT_CAMERA_POSITION,
    ANTICHEAT_OPLOGIC1,
    ANTICHEAT_OPLOGIC2,
    ANTICHEAT_OPLOGIC3,
    ANTICHEAT_OPLOGIC4,
    ANTICHEAT_OPLOGIC5,
    ANTICHEAT_OPLOGIC6,
    ANTICHEAT_OPLOGIC7,
    ANTICHEAT_OPLOGIC8,
    ANTICHEAT_OPLOGIC9,
    ANTICHEAT_CYCLELOGIC1,
    ANTICHEAT_CYCLELOGIC2,
    ANTICHEAT_CYCLELOGIC3,
    ANTICHEAT_CYCLELOGIC4,
    ANTICHEAT_CYCLELOGIC5,
    ANTICHEAT_CYCLELOGIC6,
    ANTICHEAT_CYCLELOGIC7,
    OPOBJ1,
    OPOBJ2,
    OPOBJ3,
    OPOBJ4,
    OPOBJ5,
    OPOBJT,
    OPOBJU,
    OPNPC1,
    OPNPC2,
    OPNPC3,
    OPNPC4,
    OPNPC5,
    OPNPCT,
    OPNPCU,
    OPLOC1,
    OPLOC2,
    OPLOC3,
    OPLOC4,
    OPLOC5,
    OPLOCT,
    OPLOCU,
    OPPLAYER1,
    OPPLAYER2,
    OPPLAYER3,
    OPPLAYER4,
    OPPLAYER5,
    OPPLAYERT,
    OPPLAYERU,
    OPHELD1,
    OPHELD2,
    OPHELD3,
    OPHELD4,
    OPHELD5,
    OPHELDT,
    OPHELDU,
    INV_BUTTON1,
    INV_BUTTON2,
    INV_BUTTON3,
    INV_BUTTON4,
    INV_BUTTON5,
    IF_BUTTON,
    RESUME_PAUSEBUTTON,
    CLOSE_MODAL,
    RESUME_P_COUNTDIALOG,
    TUT_CLICKSIDE,
    MAP_BUILD_COMPLETE,
    MOVE_OPCLICK,
    REPORT_ABUSE,
    MOVE_MINIMAPCLICK,
    INV_BUTTOND,
    IGNORELIST_DEL,
    IGNORELIST_ADD,
    IDK_SAVEDESIGN,
    CHAT_SETMODE,
    MESSAGE_PRIVATE,
    FRIENDLIST_DEL,
    FRIENDLIST_ADD,
    CLIENT_CHEAT,
    MESSAGE_PUBLIC,
    MOVE_GAMECLICK,
);

/// Outbound packet sink: ISAAC-encrypted opcode write plus plaintext
/// payload. The client's `Packet` implements it; the kernel never writes a
/// bare opcode outside this path.
pub trait Out {
    fn p1_enc(&mut self, opcode: i32);
    fn p2(&mut self, value: i32);
    fn p4(&mut self, value: i32);
}

impl Out for Packet {
    fn p1_enc(&mut self, opcode: i32) {
        Packet::p1_enc(self, opcode);
    }
    fn p2(&mut self, value: i32) {
        Packet::p2(self, value);
    }
    fn p4(&mut self, value: i32) {
        Packet::p4(self, value);
    }
}

/// A typed outbound send: opcode + payload, written through [`Out`]. Each
/// constructor is the builder for its `LEGAL_SEND` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Send {
    pub prot: ClientProt,
    /// The packet's payload word: child id / count amount.
    pub value: i32,
}

impl Send {
    /// Interface button press (`IF_BUTTON` child id).
    pub fn if_button(child: i32) -> Self {
        Send {
            prot: ClientProt::IF_BUTTON,
            value: child,
        }
    }

    /// Close the open modal (`CLOSE_MODAL`, no payload).
    pub fn close_modal() -> Self {
        Send {
            prot: ClientProt::CLOSE_MODAL,
            value: 0,
        }
    }

    /// Answer a count dialog (`RESUME_P_COUNTDIALOG` amount).
    pub fn count_dialog(amount: i32) -> Self {
        Send {
            prot: ClientProt::RESUME_P_COUNTDIALOG,
            value: amount,
        }
    }

    /// Append the opcode and payload to `out`.
    pub fn write(self, out: &mut dyn Out) {
        out.p1_enc(self.prot.id);
        if self.prot.id == ClientProt::IF_BUTTON.id {
            out.p2(self.value);
        } else if self.prot.id == ClientProt::RESUME_P_COUNTDIALOG.id {
            out.p4(self.value);
        }
    }
}
