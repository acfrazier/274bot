//! Legal outbound send table: every `ClientProt` from `client_prot.rs` has
//! a `LEGAL_SEND` row (opcode id + fixed length, `-1` variable). The typed
//! `Send` constructors are the builders the kernel and host use. The internal
//! sink writes an ISAAC-encrypted opcode and plaintext payload. Anticheat/event
//! packets are `pub` in the table but
//! unused by the kernel.

use std::sync::LazyLock;

use client::io::{map_client_prot, ClientProt, ClientRevision, Packet};

/// One legal outbound packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegalSend {
    /// The selected opcode.
    pub id: i32,
    /// The fixed body size, or -1 for a one-byte variable prefix.
    pub length: i32,
}

macro_rules! legal_send {
    ($($name:ident),* $(,)?) => {
        const NAMED_CLIENT_PROTS: &[ClientProt] = &[
            $(ClientProt::$name),*
        ];
        /// Every legal named outbound row in revision 274.
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

/// Legal outbound rows selected by named packet identity for `revision`.
/// The existing [`LEGAL_SEND`] constant remains the revision-274 table.
pub fn legal_sends_for(revision: ClientRevision) -> Vec<LegalSend> {
    NAMED_CLIENT_PROTS
        .iter()
        .map(|named| {
            let selected = map_client_prot(revision, *named);
            LegalSend {
                id: selected.id,
                length: selected.length,
            }
        })
        .collect()
}

/// Category charged by the engine's per-tick decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketCategory {
    /// Normal game actions, including movement and chat input.
    UserEvent,
    /// Client telemetry, focus and idle events.
    ClientEvent,
    /// An engine-restricted event category; no current named row uses it.
    RestrictedEvent,
    /// Client housekeeping excluded from input accounting.
    Housekeeping,
}

/// Selected metadata for one named packet opcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketMetadata {
    /// The selected revision's opcode byte.
    pub opcode: u8,
    /// The named protocol identity before revision mapping.
    pub named: ClientProt,
    /// The fixed body size, `-1` for VAR_BYTE or `-2` for VAR_SHORT.
    pub length: i32,
    /// The engine input category or housekeeping classification.
    pub category: PacketCategory,
}

const EMPTY_METADATA: [Option<PacketMetadata>; 256] = [None; 256];
static PACKET_METADATA: LazyLock<[[Option<PacketMetadata>; 256]; 2]> = LazyLock::new(|| {
    let mut tables = [EMPTY_METADATA; 2];
    for &named in NAMED_CLIENT_PROTS {
        let category = match named {
            ClientProt::NO_TIMEOUT
            | ClientProt::MAP_BUILD_COMPLETE
            | ClientProt::ANTICHEAT_OPLOGIC1
            | ClientProt::ANTICHEAT_OPLOGIC2
            | ClientProt::ANTICHEAT_OPLOGIC3
            | ClientProt::ANTICHEAT_OPLOGIC4
            | ClientProt::ANTICHEAT_OPLOGIC5
            | ClientProt::ANTICHEAT_OPLOGIC6
            | ClientProt::ANTICHEAT_OPLOGIC7
            | ClientProt::ANTICHEAT_OPLOGIC8
            | ClientProt::ANTICHEAT_OPLOGIC9
            | ClientProt::ANTICHEAT_CYCLELOGIC1
            | ClientProt::ANTICHEAT_CYCLELOGIC2
            | ClientProt::ANTICHEAT_CYCLELOGIC3
            | ClientProt::ANTICHEAT_CYCLELOGIC4
            | ClientProt::ANTICHEAT_CYCLELOGIC5
            | ClientProt::ANTICHEAT_CYCLELOGIC6
            | ClientProt::ANTICHEAT_CYCLELOGIC7 => PacketCategory::Housekeeping,
            ClientProt::IDLE_TIMER
            | ClientProt::EVENT_MOUSE_CLICK
            | ClientProt::EVENT_MOUSE_MOVE
            | ClientProt::EVENT_APPLET_FOCUS
            | ClientProt::EVENT_CAMERA_POSITION => PacketCategory::ClientEvent,
            _ => PacketCategory::UserEvent,
        };
        for (index, revision) in [ClientRevision::R274, ClientRevision::R289]
            .into_iter()
            .enumerate()
        {
            let selected = map_client_prot(revision, named);
            if let Ok(opcode) = u8::try_from(selected.id) {
                tables[index][usize::from(opcode)] = Some(PacketMetadata {
                    opcode,
                    named,
                    length: selected.length,
                    category,
                });
            }
        }
    }
    tables
});

/// Return metadata for an observed opcode without allocating or scanning tables.
pub fn packet_metadata(revision: ClientRevision, opcode: u8) -> Option<PacketMetadata> {
    PACKET_METADATA[usize::from(revision.is_289())][usize::from(opcode)]
}

mod sink {
    /// Raw encrypted sink for API internals and opt-in test recorders only.
    pub trait Out {
        /// Append one ISAAC-encrypted opcode.
        fn p1_enc(&mut self, opcode: i32);
        /// Append one plaintext byte.
        fn p1(&mut self, value: i32);
        /// Append one big-endian two-byte value.
        fn p2(&mut self, value: i32);
        /// Append one big-endian four-byte value.
        fn p4(&mut self, value: i32);
        /// Append a client string with its newline terminator.
        #[cfg(any(test, feature = "test-support"))]
        fn pjstr(&mut self, s: &str);
        /// Complete a validated packet for an optional real-sink observer.
        fn finish_packet(&mut self) {}
        /// Append an already validated body without a second buffer.
        fn pbytes(&mut self, bytes: &[u8]) {
            for &byte in bytes {
                self.p1(i32::from(byte));
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub use sink::Out;
#[cfg(not(any(test, feature = "test-support")))]
pub(crate) use sink::Out;

impl Out for Packet {
    fn p1_enc(&mut self, opcode: i32) {
        Packet::p1_enc(self, opcode);
    }
    fn p1(&mut self, value: i32) {
        Packet::p1(self, value);
    }
    fn p2(&mut self, value: i32) {
        Packet::p2(self, value);
    }
    fn p4(&mut self, value: i32) {
        Packet::p4(self, value);
    }
    #[cfg(any(test, feature = "test-support"))]
    fn pjstr(&mut self, s: &str) {
        Packet::pjstr(self, s);
    }
    fn pbytes(&mut self, bytes: &[u8]) {
        self.pdata(bytes, 0, bytes.len());
    }
    fn finish_packet(&mut self) {
        Packet::finish_packet(self);
    }
}

/// Per-packet effects derived from the engine handlers.
pub mod effects;

/// A refusal before any output cursor or cipher state changes.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PacketError {
    /// An integer cannot be represented by its wire field or protocol domain.
    #[error("{field} value {value} is outside {min}..={max}")]
    IntegerOutOfRange {
        /// The packet field.
        field: &'static str,
        /// The rejected value.
        value: i64,
        /// The inclusive minimum.
        min: i64,
        /// The inclusive maximum.
        max: i64,
    },
    /// The engine cannot decode this account name.
    #[error("invalid base-37 account name {0}")]
    InvalidName(i64),
    /// The report reason is not one of the twelve engine reasons.
    #[error("invalid abuse reason {0}")]
    InvalidAbuseReason(i32),
    /// A body exceeds its framing or decoder limit.
    #[error("packet body has {length} bytes, maximum {max}")]
    BodyTooLong {
        /// The rejected body length.
        length: usize,
        /// The applicable maximum.
        max: usize,
    },
    /// A packet has an incomplete or unsupported internal shape.
    #[error("invalid packet shape: {0}")]
    InvalidShape(&'static str),
    /// Text cannot pass through the client codec without loss or truncation.
    #[error("chat text is not representable within the client codec's 80-character limit")]
    InvalidText,
    /// A cheat is absent from the selected catalog or has invalid arguments.
    #[error("invalid catalog cheat: {0}")]
    InvalidCheat(String),
    /// Catalog validation and the live session select different revisions.
    #[error("packet catalog revision differs from the session revision")]
    RevisionMismatch,
}

/// The engine's framing limit, separate from each decoder's internal shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketLength {
    /// An exact fixed body size.
    Fixed(usize),
    /// An unsigned one-byte prefix.
    VarByte,
    /// A two-byte prefix, with the engine's 1600-byte ceiling.
    VarShort,
}

impl PacketLength {
    /// Validate a body before writing its opcode or its length prefix.
    pub fn validate(self, length: usize) -> Result<(), PacketError> {
        match self {
            Self::Fixed(expected) if length != expected => {
                Err(PacketError::InvalidShape("fixed body length"))
            }
            Self::VarByte if length > 255 => Err(PacketError::BodyTooLong { length, max: 255 }),
            Self::VarShort if length > 1600 => Err(PacketError::BodyTooLong { length, max: 1600 }),
            _ => Ok(()),
        }
    }
}

/// An account name accepted by `JString.fromBase37` in the selected engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Base37Name(i64);

impl Base37Name {
    /// Reject zero, negative names, trailing base-37 zeroes and names of 13 digits.
    pub fn new(value: i64) -> Result<Self, PacketError> {
        if value <= 0 || value >= 6_582_952_005_840_035_281 || value % 37 == 0 {
            return Err(PacketError::InvalidName(value));
        }
        Ok(Self(value))
    }

    /// The validated eight-byte wire value.
    pub fn value(self) -> i64 {
        self.0
    }
}

/// The twelve reasons accepted by `ReportAbuseHandler`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AbuseReason {
    /// Offensive language.
    OffensiveLanguage = 0,
    /// Item scamming.
    ItemScamming = 1,
    /// Password scamming.
    PasswordScamming = 2,
    /// Bug abuse.
    BugAbuse = 3,
    /// Staff impersonation.
    StaffImpersonation = 4,
    /// Account sharing or trading.
    AccountSharing = 5,
    /// Macroing.
    Macroing = 6,
    /// Multiple logging in.
    MultipleLoggingIn = 7,
    /// Encouraging rule breaking.
    EncouragingRuleBreaking = 8,
    /// Misuse of customer support.
    CustomerSupportMisuse = 9,
    /// Advertising websites.
    WebsiteAdvertising = 10,
    /// Real-world trading.
    RealWorldTrading = 11,
}

impl AbuseReason {
    /// The validated one-byte wire reason.
    pub fn value(self) -> u8 {
        self as u8
    }
}

impl TryFrom<i32> for AbuseReason {
    type Error = PacketError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        use AbuseReason::*;
        match value {
            0 => Ok(OffensiveLanguage),
            1 => Ok(ItemScamming),
            2 => Ok(PasswordScamming),
            3 => Ok(BugAbuse),
            4 => Ok(StaffImpersonation),
            5 => Ok(AccountSharing),
            6 => Ok(Macroing),
            7 => Ok(MultipleLoggingIn),
            8 => Ok(EncouragingRuleBreaking),
            9 => Ok(CustomerSupportMisuse),
            10 => Ok(WebsiteAdvertising),
            11 => Ok(RealWorldTrading),
            _ => Err(PacketError::InvalidAbuseReason(value)),
        }
    }
}

/// One complete mouse telemetry record. Callers cannot provide encoded bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseMoveRecord {
    /// Two-byte relative movement, with fewer than eight duplicate samples.
    Delta {
        /// Duplicate samples, 0 through 7.
        duplicates: u8,
        /// Horizontal delta, -32 through 31.
        dx: i8,
        /// Vertical delta, -32 through 31.
        dy: i8,
    },
    /// Three-byte absolute movement, with fewer than eight duplicate samples.
    Absolute {
        /// Duplicate samples, 0 through 7.
        duplicates: u8,
        /// Viewport x, 0 through 764.
        x: u16,
        /// Viewport y, 0 through 502.
        y: u16,
    },
    /// Four-byte absolute movement after 8 through 2047 duplicate samples.
    Repeated {
        /// Duplicate samples, 8 through 2047.
        duplicates: u16,
        /// Viewport x, 0 through 764.
        x: u16,
        /// Viewport y, 0 through 502.
        y: u16,
    },
    /// An off-viewport sample, encoded with the client's coordinate sentinel.
    Outside {
        /// Duplicate samples, 0 through 2047.
        duplicates: u16,
    },
}

/// The complete fourteen-byte minimap trailer, including fixed client markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MinimapTrailer {
    /// Signed click x relative to the minimap centre.
    pub click_x: i8,
    /// Signed click y relative to the minimap centre.
    pub click_y: i8,
    /// Camera yaw.
    pub camera_yaw: u16,
    /// The client's camera anti-cheat angle byte.
    pub camera_anticheat_angle: u8,
    /// The client's minimap zoom byte.
    pub minimap_zoom: u8,
    /// Fine-coordinate player x.
    pub player_x: u16,
    /// Fine-coordinate player z.
    pub player_z: u16,
    /// Whether the client selected a nearest reachable tile.
    pub nearest: bool,
}

/// Structured `ANTICHEAT_CYCLELOGIC1` optional markers and random byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleLogic1 {
    /// Include the leading 11499 word.
    pub prefix: bool,
    /// Include marker 139.
    pub marker139: bool,
    /// Include marker 94.
    pub marker94: bool,
    /// Include marker 65.
    pub marker65: bool,
    /// The random byte.
    pub random: u8,
}

/// Structured `ANTICHEAT_CYCLELOGIC2` optional markers and random fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleLogic2 {
    /// Include the leading 16791 word.
    pub prefix: bool,
    /// Include the 35025 word before the final constant.
    pub suffix: bool,
    /// The first random word.
    pub random_word_a: u16,
    /// The second random word.
    pub random_word_b: u16,
    /// The final random byte.
    pub random_tail: u8,
}

// Fixed bodies and all movement packets remain allocation-free. Larger variable
// bodies own their allocation; Send deliberately has no Clone implementation.
#[derive(Debug, PartialEq, Eq)]
enum Body {
    Inline { bytes: [u8; 67], length: u8 },
    Variable(Vec<u8>),
}

/// A complete, validated named packet. Construction grants no emission authority.
///
/// ```compile_fail
/// use api::prot::Send;
/// use client::io::ClientProt;
/// let forged = Send { prot: ClientProt::OPNPC1, value: 99 };
/// ```
#[derive(Debug, PartialEq, Eq)]
pub struct Send {
    named: ClientProt,
    body: Body,
    catalog_revision: Option<ClientRevision>,
}

fn integer(field: &'static str, value: i32, min: i32, max: i32) -> Result<i32, PacketError> {
    if !(min..=max).contains(&value) {
        return Err(PacketError::IntegerOutOfRange {
            field,
            value: i64::from(value),
            min: i64::from(min),
            max: i64::from(max),
        });
    }
    Ok(value)
}

macro_rules! words {
    ($($method:ident => $named:ident ($($field:ident),*);)*) => {
        impl Send {
            $(#[doc = concat!("Build `", stringify!($named), "` with unsigned two-byte fields in decoder order.")]
            pub fn $method($($field: i32),*) -> Result<Self, PacketError> {
                $(integer(stringify!($field), $field, 0, 65535)?;)*
                let mut bytes = [0; 67];
                let mut length = 0;
                $(bytes[length..length + 2].copy_from_slice(&($field as u16).to_be_bytes());
                  length += 2;)*
                Ok(Self::inline(ClientProt::$named, bytes, length))
            })*
        }
    };
}

words! {
    if_button => IF_BUTTON(child);
    resume_pausebutton => RESUME_PAUSEBUTTON(child);
    event_camera_position => EVENT_CAMERA_POSITION(pitch,yaw);
    anticheat_oplogic2 => ANTICHEAT_OPLOGIC2(value);
    anticheat_oplogic6 => ANTICHEAT_OPLOGIC6(value);
    opobj1 => OPOBJ1(x,z,obj); opobj2 => OPOBJ2(x,z,obj);
    opobj3 => OPOBJ3(x,z,obj); opobj4 => OPOBJ4(x,z,obj); opobj5 => OPOBJ5(x,z,obj);
    opobjt => OPOBJT(x,z,obj,spell); opobju => OPOBJU(x,z,obj,use_obj,use_slot,use_com);
    oploc1 => OPLOC1(x,z,loc); oploc2 => OPLOC2(x,z,loc);
    oploc3 => OPLOC3(x,z,loc); oploc4 => OPLOC4(x,z,loc); oploc5 => OPLOC5(x,z,loc);
    oploct => OPLOCT(x,z,loc,spell); oplocu => OPLOCU(x,z,loc,use_obj,use_slot,use_com);
    opnpc1 => OPNPC1(npc); opnpc2 => OPNPC2(npc); opnpc3 => OPNPC3(npc);
    opnpc4 => OPNPC4(npc); opnpc5 => OPNPC5(npc); opnpct => OPNPCT(npc,spell);
    opnpcu => OPNPCU(npc,use_obj,use_slot,use_com);
    opplayer1 => OPPLAYER1(player); opplayer2 => OPPLAYER2(player); opplayer3 => OPPLAYER3(player);
    opplayer4 => OPPLAYER4(player); opplayer5 => OPPLAYER5(player); opplayert => OPPLAYERT(player,spell);
    opplayeru => OPPLAYERU(player,use_obj,use_slot,use_com);
    opheld1 => OPHELD1(obj,slot,com); opheld2 => OPHELD2(obj,slot,com);
    opheld3 => OPHELD3(obj,slot,com); opheld4 => OPHELD4(obj,slot,com); opheld5 => OPHELD5(obj,slot,com);
    opheldt => OPHELDT(obj,slot,com,spell); opheldu => OPHELDU(obj,slot,com,use_obj,use_slot,use_com);
    inv_button1 => INV_BUTTON1(obj,slot,com); inv_button2 => INV_BUTTON2(obj,slot,com);
    inv_button3 => INV_BUTTON3(obj,slot,com); inv_button4 => INV_BUTTON4(obj,slot,com);
    inv_button5 => INV_BUTTON5(obj,slot,com);
}

macro_rules! empty {
    ($($method:ident => $named:ident),* $(,)?) => {
        impl Send {
            $(#[doc = concat!("Build `", stringify!($named), "` with no body.")]
            pub fn $method() -> Self {
                Self::inline(ClientProt::$named, [0; 67], 0)
            })*
        }
    };
}
empty! {
    no_timeout => NO_TIMEOUT, idle_timer => IDLE_TIMER, close_modal => CLOSE_MODAL,
    map_build_complete => MAP_BUILD_COMPLETE, anticheat_cyclelogic5 => ANTICHEAT_CYCLELOGIC5,
    anticheat_cyclelogic7 => ANTICHEAT_CYCLELOGIC7,
}

macro_rules! bytes {
    ($($method:ident => $named:ident),* $(,)?) => {
        impl Send {
            $(#[doc = concat!("Build `", stringify!($named), "` with an unsigned byte.")]
            pub fn $method(value: i32) -> Result<Self, PacketError> {
                integer("value", value, 0, 255)?;
                let mut body = [0; 67];
                body[0] = value as u8;
                Ok(Self::inline(ClientProt::$named, body, 1))
            })*
        }
    };
}
bytes! {
    anticheat_oplogic4 => ANTICHEAT_OPLOGIC4, anticheat_oplogic5 => ANTICHEAT_OPLOGIC5,
    anticheat_oplogic8 => ANTICHEAT_OPLOGIC8, anticheat_cyclelogic3 => ANTICHEAT_CYCLELOGIC3,
    anticheat_cyclelogic4 => ANTICHEAT_CYCLELOGIC4, anticheat_cyclelogic6 => ANTICHEAT_CYCLELOGIC6,
}

macro_rules! signed_words {
    ($($method:ident => $named:ident),* $(,)?) => {
        impl Send {
            $(#[doc = concat!("Build `", stringify!($named), "` with its full signed 32-bit word.")]
            pub fn $method(value: i32) -> Self {
                let mut body = [0; 67];
                body[..4].copy_from_slice(&value.to_be_bytes());
                Self::inline(ClientProt::$named, body, 4)
            })*
        }
    };
}
signed_words! {
    count_dialog => RESUME_P_COUNTDIALOG, anticheat_oplogic1 => ANTICHEAT_OPLOGIC1,
    anticheat_oplogic3 => ANTICHEAT_OPLOGIC3, anticheat_oplogic7 => ANTICHEAT_OPLOGIC7,
}

impl Send {
    fn inline(named: ClientProt, bytes: [u8; 67], length: usize) -> Self {
        Self {
            named,
            body: Body::Inline {
                bytes,
                length: length as u8,
            },
            catalog_revision: None,
        }
    }

    fn variable(named: ClientProt, bytes: Vec<u8>) -> Result<Self, PacketError> {
        PacketLength::VarByte.validate(bytes.len())?;
        Ok(Self {
            named,
            body: Body::Variable(bytes),
            catalog_revision: None,
        })
    }

    /// The named identity, before revision mapping. It cannot change the packet.
    pub fn named_packet(&self) -> ClientProt {
        self.named
    }

    /// The complete validated body, without opcode or framing prefix.
    pub fn body(&self) -> &[u8] {
        match &self.body {
            Body::Inline { bytes, length } => &bytes[..usize::from(*length)],
            Body::Variable(bytes) => bytes,
        }
    }

    /// Validate the complete packet for one selected revision without writing
    /// bytes or advancing the cipher.
    pub fn validate_for_revision(
        &self,
        revision: ClientRevision,
    ) -> Result<PacketMetadata, PacketError> {
        if self
            .catalog_revision
            .is_some_and(|selected| selected != revision)
        {
            return Err(PacketError::RevisionMismatch);
        }
        let selected = map_client_prot(revision, self.named);
        let framing = match selected.length {
            -1 => PacketLength::VarByte,
            -2 => PacketLength::VarShort,
            length if length >= 0 => PacketLength::Fixed(length as usize),
            _ => return Err(PacketError::InvalidShape("unsupported framing")),
        };
        framing.validate(self.body().len())?;
        let opcode = u8::try_from(selected.id)
            .map_err(|_| PacketError::InvalidShape("opcode is outside one byte"))?;
        packet_metadata(revision, opcode)
            .ok_or(PacketError::InvalidShape("unsupported selected packet"))
    }

    /// Validate the selected row and append through the normal encrypted sink.
    /// Refusals precede every output write, including opcode encryption.
    /// Product builds accept only the concrete client `Packet`. The `test-support`
    /// feature also admits raw recorders for tests.
    pub fn write_for_revision(
        &self,
        revision: ClientRevision,
        #[cfg(any(test, feature = "test-support"))] out: &mut dyn Out,
        #[cfg(not(any(test, feature = "test-support")))] out: &mut Packet,
    ) -> Result<(), PacketError> {
        self.write_to(revision, out)
    }

    fn write_to(&self, revision: ClientRevision, out: &mut dyn Out) -> Result<(), PacketError> {
        let metadata = self.validate_for_revision(revision)?;
        let framing = match metadata.length {
            -1 => PacketLength::VarByte,
            -2 => PacketLength::VarShort,
            length if length >= 0 => PacketLength::Fixed(length as usize),
            _ => return Err(PacketError::InvalidShape("unsupported framing")),
        };
        let body = self.body();
        out.p1_enc(i32::from(metadata.opcode));
        match framing {
            PacketLength::VarByte => out.p1(body.len() as i32),
            PacketLength::VarShort => out.p2(body.len() as i32),
            PacketLength::Fixed(_) => {}
        }
        // Keep the existing scalar sink operations for count/button recorders.
        if matches!(framing, PacketLength::Fixed(2)) {
            out.p2(i32::from(u16::from_be_bytes([body[0], body[1]])));
        } else if matches!(framing, PacketLength::Fixed(4)) {
            out.p4(i32::from_be_bytes([body[0], body[1], body[2], body[3]]));
        } else if [
            ClientProt::MOVE_GAMECLICK,
            ClientProt::MOVE_OPCLICK,
            ClientProt::MOVE_MINIMAPCLICK,
        ]
        .contains(&self.named)
        {
            out.p1(i32::from(body[0]));
            out.p2(i32::from(u16::from_be_bytes([body[1], body[2]])));
            out.p2(i32::from(u16::from_be_bytes([body[3], body[4]])));
            out.pbytes(&body[5..]);
        } else {
            out.pbytes(body);
        }
        out.finish_packet();
        Ok(())
    }

    /// Build an unsigned 24-bit anti-cheat word without truncation.
    pub fn anticheat_oplogic9(value: i32) -> Result<Self, PacketError> {
        integer("value", value, 0, 0xff_ffff)?;
        let mut body = [0; 67];
        body[..3].copy_from_slice(&value.to_be_bytes()[1..]);
        Ok(Self::inline(ClientProt::ANTICHEAT_OPLOGIC9, body, 3))
    }

    /// Build the client's packed click time, button and viewport coordinate.
    pub fn event_mouse_click(
        elapsed_50ms: i32,
        right: bool,
        x: i32,
        y: i32,
    ) -> Result<Self, PacketError> {
        integer("elapsed_50ms", elapsed_50ms, 0, 4095)?;
        integer("x", x, 0, 764)?;
        integer("y", y, 0, 502)?;
        let value = (elapsed_50ms << 20) | (i32::from(right) << 19) | (y * 765 + x);
        let mut body = [0; 67];
        body[..4].copy_from_slice(&value.to_be_bytes());
        Ok(Self::inline(ClientProt::EVENT_MOUSE_CLICK, body, 4))
    }

    /// Report applet focus using the client's boolean byte.
    pub fn event_applet_focus(focused: bool) -> Self {
        let mut body = [0; 67];
        body[0] = u8::from(focused);
        Self::inline(ClientProt::EVENT_APPLET_FOCUS, body, 1)
    }

    /// Build a complete mouse-record stream. Validate all records before allocation.
    pub fn event_mouse_move(records: &[MouseMoveRecord]) -> Result<Self, PacketError> {
        let mut body = [0; 255];
        let mut length = 0;
        for &record in records {
            let (value, width) = mouse_record(record)?;
            let next = length + width;
            PacketLength::VarByte.validate(next)?;
            body[length..next].copy_from_slice(&value.to_be_bytes()[4 - width..]);
            length = next;
        }
        Self::variable(ClientProt::EVENT_MOUSE_MOVE, body[..length].to_vec())
    }

    /// Build the typed client anti-cheat marker sequence, not an encoded blob.
    pub fn anticheat_cyclelogic1(record: CycleLogic1) -> Result<Self, PacketError> {
        let mut body = [0; 67];
        let mut length = 0;
        let mut append = |bytes: &[u8]| {
            body[length..length + bytes.len()].copy_from_slice(bytes);
            length += bytes.len();
        };
        if record.prefix {
            append(&11499u16.to_be_bytes());
        }
        append(&10548u16.to_be_bytes());
        if record.marker139 {
            append(&[139]);
        }
        if record.marker94 {
            append(&[94]);
        }
        append(&51693u16.to_be_bytes());
        append(&[16]);
        append(&15036u16.to_be_bytes());
        if record.marker65 {
            append(&[65]);
        }
        append(&[record.random]);
        append(&22990u16.to_be_bytes());
        Ok(Self::inline(
            ClientProt::ANTICHEAT_CYCLELOGIC1,
            body,
            length,
        ))
    }

    /// Build the second typed anti-cheat marker sequence.
    pub fn anticheat_cyclelogic2(record: CycleLogic2) -> Result<Self, PacketError> {
        let mut body = [0; 67];
        let mut length = 0;
        let mut append = |bytes: &[u8]| {
            body[length..length + bytes.len()].copy_from_slice(bytes);
            length += bytes.len();
        };
        if record.prefix {
            append(&16791u16.to_be_bytes());
        }
        append(&[254]);
        for word in [
            record.random_word_a,
            16128,
            52610,
            record.random_word_b,
            55420,
        ] {
            append(&word.to_be_bytes());
        }
        if record.suffix {
            append(&35025u16.to_be_bytes());
        }
        append(&46628u16.to_be_bytes());
        append(&[record.random_tail]);
        Ok(Self::inline(
            ClientProt::ANTICHEAT_CYCLELOGIC2,
            body,
            length,
        ))
    }

    /// Move with a game-view click and at most 24 complete signed offset pairs.
    pub fn move_gameclick(
        control: bool,
        x: i32,
        z: i32,
        offsets: &[(i8, i8)],
    ) -> Result<Self, PacketError> {
        Self::movement(ClientProt::MOVE_GAMECLICK, control, x, z, offsets, None)
    }

    /// Move as a predecessor to an operation click.
    pub fn move_opclick(
        control: bool,
        x: i32,
        z: i32,
        offsets: &[(i8, i8)],
    ) -> Result<Self, PacketError> {
        Self::movement(ClientProt::MOVE_OPCLICK, control, x, z, offsets, None)
    }

    /// Move with the complete typed fourteen-byte minimap trailer.
    pub fn move_minimapclick(
        control: bool,
        x: i32,
        z: i32,
        offsets: &[(i8, i8)],
        trailer: MinimapTrailer,
    ) -> Result<Self, PacketError> {
        Self::movement(
            ClientProt::MOVE_MINIMAPCLICK,
            control,
            x,
            z,
            offsets,
            Some(trailer),
        )
    }

    fn movement(
        named: ClientProt,
        control: bool,
        x: i32,
        z: i32,
        offsets: &[(i8, i8)],
        trailer: Option<MinimapTrailer>,
    ) -> Result<Self, PacketError> {
        integer("x", x, 0, 65535)?;
        integer("z", z, 0, 65535)?;
        if offsets.len() > 24 {
            return Err(PacketError::InvalidShape(
                "movement exceeds 24 offset pairs",
            ));
        }
        let mut body = [0; 67];
        body[0] = u8::from(control);
        body[1..3].copy_from_slice(&(x as u16).to_be_bytes());
        body[3..5].copy_from_slice(&(z as u16).to_be_bytes());
        let mut length = 5;
        for &(dx, dz) in offsets {
            body[length] = dx as u8;
            body[length + 1] = dz as u8;
            length += 2;
        }
        if let Some(t) = trailer {
            let yaw = t.camera_yaw.to_be_bytes();
            let px = t.player_x.to_be_bytes();
            let pz = t.player_z.to_be_bytes();
            body[length..length + 14].copy_from_slice(&[
                t.click_x as u8,
                t.click_y as u8,
                yaw[0],
                yaw[1],
                57,
                t.camera_anticheat_angle,
                t.minimap_zoom,
                89,
                px[0],
                px[1],
                pz[0],
                pz[1],
                u8::from(t.nearest),
                63,
            ]);
            length += 14;
        }
        Ok(Self::inline(named, body, length))
    }

    /// Drag an inventory row. The final byte is the client's insertion mode.
    pub fn inv_buttond(
        com: i32,
        slot: i32,
        target_slot: i32,
        insert: bool,
    ) -> Result<Self, PacketError> {
        integer("com", com, 0, 65535)?;
        integer("slot", slot, 0, 65535)?;
        integer("target_slot", target_slot, 0, 65535)?;
        let mut body = [0; 67];
        body[..2].copy_from_slice(&(com as u16).to_be_bytes());
        body[2..4].copy_from_slice(&(slot as u16).to_be_bytes());
        body[4..6].copy_from_slice(&(target_slot as u16).to_be_bytes());
        body[6] = u8::from(insert);
        Ok(Self::inline(ClientProt::INV_BUTTOND, body, 7))
    }

    /// Report a tutorial tab click, using the handler's 0 through 13 domain.
    pub fn tut_clickside(tab: i32) -> Result<Self, PacketError> {
        integer("tab", tab, 0, 13)?;
        let mut body = [0; 67];
        body[0] = tab as u8;
        Ok(Self::inline(ClientProt::TUT_CLICKSIDE, body, 1))
    }

    /// Save a design. Identity-kit ownership/type checks remain engine content checks.
    /// An absent kit uses the decoder's 255 sentinel. `Some(255)` is invalid.
    pub fn idk_savedesign(
        gender: i32,
        kits: [Option<u8>; 7],
        colours: [i32; 5],
    ) -> Result<Self, PacketError> {
        integer("gender", gender, 0, 1)?;
        for kit in kits.into_iter().flatten() {
            integer("kit", i32::from(kit), 0, 254)?;
        }
        // Player.DESIGN_BODY_COLORS, engine/entity/Player.ts:101-107.
        for (colour, max) in colours.into_iter().zip([11, 15, 15, 5, 7]) {
            integer("colour", colour, 0, max)?;
        }
        let mut body = [0; 67];
        body[0] = gender as u8;
        for (index, kit) in kits.into_iter().enumerate() {
            body[1 + index] = kit.unwrap_or(255);
        }
        for (index, colour) in colours.into_iter().enumerate() {
            body[8 + index] = colour as u8;
        }
        Ok(Self::inline(ClientProt::IDK_SAVEDESIGN, body, 13))
    }

    /// Set chat modes using the real client's public/private/trade domains.
    pub fn chat_setmode(public: i32, private: i32, trade: i32) -> Result<Self, PacketError> {
        integer("public", public, 0, 3)?;
        integer("private", private, 0, 2)?;
        integer("trade", trade, 0, 2)?;
        let mut body = [0; 67];
        body[..3].copy_from_slice(&[public as u8, private as u8, trade as u8]);
        Ok(Self::inline(ClientProt::CHAT_SETMODE, body, 3))
    }

    fn name(named: ClientProt, name: Base37Name) -> Self {
        let mut body = [0; 67];
        body[..8].copy_from_slice(&name.value().to_be_bytes());
        Self::inline(named, body, 8)
    }

    /// Add a validated account to the friend list.
    pub fn friendlist_add(name: Base37Name) -> Self {
        Self::name(ClientProt::FRIENDLIST_ADD, name)
    }
    /// Remove a validated account from the friend list.
    pub fn friendlist_del(name: Base37Name) -> Self {
        Self::name(ClientProt::FRIENDLIST_DEL, name)
    }
    /// Add a validated account to the ignore list.
    pub fn ignorelist_add(name: Base37Name) -> Self {
        Self::name(ClientProt::IGNORELIST_ADD, name)
    }
    /// Remove a validated account from the ignore list.
    pub fn ignorelist_del(name: Base37Name) -> Self {
        Self::name(ClientProt::IGNORELIST_DEL, name)
    }

    /// Report a validated account with one of the twelve legal reasons.
    pub fn report_abuse(name: Base37Name, reason: AbuseReason, moderator_mute: bool) -> Self {
        let mut body = [0; 67];
        body[..8].copy_from_slice(&name.value().to_be_bytes());
        body[8] = reason.value();
        body[9] = u8::from(moderator_mute);
        Self::inline(ClientProt::REPORT_ABUSE, body, 10)
    }

    /// Pack a private message with the real client codec and a validated recipient.
    pub fn message_private(name: Base37Name, text: &str) -> Result<Self, PacketError> {
        Self::chat(
            ClientProt::MESSAGE_PRIVATE,
            &name.value().to_be_bytes(),
            text,
        )
    }

    /// Pack a public message with validated colour (0..11) and effect (0..5).
    pub fn message_public(colour: i32, effect: i32, text: &str) -> Result<Self, PacketError> {
        integer("colour", colour, 0, 11)?;
        integer("effect", effect, 0, 5)?;
        Self::chat(
            ClientProt::MESSAGE_PUBLIC,
            &[colour as u8, effect as u8],
            text,
        )
    }

    fn chat(named: ClientProt, prefix: &[u8], text: &str) -> Result<Self, PacketError> {
        // WordPack itself truncates after 80 characters and substitutes unknowns.
        // Refuse those inputs before invoking that shared codec, never duplicate it.
        const ALPHABET: &str = " etaoihnsrdlumwcyfgpbvkxjqz0123456789!?.,:;()-&*\\'@#+=£$%\"[]";
        let mut count = 0;
        for character in text.chars() {
            count += 1;
            if count > 80 || !ALPHABET.contains(character.to_ascii_lowercase()) {
                return Err(PacketError::InvalidText);
            }
        }
        let mut packed = Packet::new(vec![0; prefix.len() + count]);
        packed.pdata(prefix, 0, prefix.len());
        client::wordfilter::WordPack::pack(&mut packed, text);
        let packed_length = packed.pos - prefix.len();
        if packed_length > 100 {
            return Err(PacketError::BodyTooLong {
                length: packed_length,
                max: 100,
            });
        }
        // Packet intentionally exposes no owned-buffer extraction. This copy is
        // required to reuse its authoritative WordPack codec without retaining ISAAC.
        Self::variable(named, packed.data()[..packed.pos].to_vec())
    }

    /// Resolve and format a command through the selected debug catalog.
    /// Profile/session admission is still mandatory at the dispatch boundary.
    pub fn client_cheat(
        catalog: &crate::debug_commands::DebugCatalog,
        command: &str,
    ) -> Result<Self, PacketError> {
        if !command.is_ascii() || command.bytes().any(|byte| byte.is_ascii_control()) {
            return Err(PacketError::InvalidCheat(
                "command must be printable ASCII".into(),
            ));
        }
        if command.len() > 80 {
            return Err(PacketError::BodyTooLong {
                length: command.len() + 1,
                max: 81,
            });
        }
        let mut tokens = command.split(' ');
        let name = tokens.next().unwrap_or_default();
        let entry = catalog
            .commands()
            .iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| PacketError::InvalidCheat("unknown command".into()))?;
        let values: Vec<String> = tokens.map(str::to_owned).collect();
        let formatted = entry
            .format_command(&values)
            .map_err(PacketError::InvalidCheat)?;
        if formatted != command {
            return Err(PacketError::InvalidCheat("noncanonical command".into()));
        }
        let mut body = Vec::with_capacity(command.len() + 1);
        body.extend_from_slice(command.as_bytes());
        body.push(10);
        let mut packet = Self::variable(ClientProt::CLIENT_CHEAT, body)?;
        packet.catalog_revision = Some(catalog.revision());
        Ok(packet)
    }
}

fn mouse_record(record: MouseMoveRecord) -> Result<(u32, usize), PacketError> {
    let coordinate = |x: u16, y: u16| -> Result<u32, PacketError> {
        integer("x", i32::from(x), 0, 764)?;
        integer("y", i32::from(y), 0, 502)?;
        Ok(u32::from(y) * 765 + u32::from(x))
    };
    match record {
        MouseMoveRecord::Delta { duplicates, dx, dy } => {
            integer("duplicates", i32::from(duplicates), 0, 7)?;
            integer("dx", i32::from(dx), -32, 31)?;
            integer("dy", i32::from(dy), -32, 31)?;
            Ok((
                (u32::from(duplicates) << 12)
                    | (((i32::from(dx) + 32) as u32) << 6)
                    | (i32::from(dy) + 32) as u32,
                2,
            ))
        }
        MouseMoveRecord::Absolute { duplicates, x, y } => {
            integer("duplicates", i32::from(duplicates), 0, 7)?;
            Ok((
                0x80_0000 | (u32::from(duplicates) << 19) | coordinate(x, y)?,
                3,
            ))
        }
        MouseMoveRecord::Repeated { duplicates, x, y } => {
            integer("duplicates", i32::from(duplicates), 8, 2047)?;
            Ok((
                0xc000_0000 | (u32::from(duplicates) << 19) | coordinate(x, y)?,
                4,
            ))
        }
        MouseMoveRecord::Outside { duplicates } => {
            integer("duplicates", i32::from(duplicates), 0, 2047)?;
            if duplicates < 8 {
                Ok((0x80_0000 | (u32::from(duplicates) << 19) | 524287, 3))
            } else {
                Ok((0xc000_0000 | (u32::from(duplicates) << 19) | 524287, 4))
            }
        }
    }
}

#[cfg(test)]
mod packet_tests {
    use super::*;

    #[test]
    fn framing_limits_are_engine_limits_not_prefix_integer_limits() {
        assert_eq!(PacketLength::VarByte.validate(255), Ok(()));
        assert_eq!(
            PacketLength::VarByte.validate(256),
            Err(PacketError::BodyTooLong {
                length: 256,
                max: 255
            })
        );
        assert_eq!(PacketLength::VarShort.validate(1600), Ok(()));
        assert_eq!(
            PacketLength::VarShort.validate(1601),
            Err(PacketError::BodyTooLong {
                length: 1601,
                max: 1600
            })
        );
        assert_eq!(
            PacketLength::Fixed(2).validate(1),
            Err(PacketError::InvalidShape("fixed body length"))
        );
        assert_eq!(
            PacketLength::Fixed(2).validate(3),
            Err(PacketError::InvalidShape("fixed body length"))
        );
    }
}
