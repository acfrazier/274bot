use super::*;
pub const CATALOG_COMMIT_A: &str = "100adccc037d9f6898080e1cad58fcfc43364775";
pub const CATALOG_COMMIT_B: &str = "8e7d965be2071d6ec65c3265e12af797082d720a";

pub const NATURECRAFTER: &str = "NatureCrafter";
pub const MULECRAFTER: &str = "MuleCrafter";
pub const FLAXRUNNER: &str = "FlaxRunner";
pub const DUEL_ARENA: &str = "Duel Arena Combat Trainer";
pub const NATURECRAFTER_SHA256: &str =
    "025ac395b25d64ef818cc0321478f0a2c84a051b79f99decbbfec5a9a2f0812a";
pub const MULECRAFTER_SHA256: &str =
    "bf745db4c0a3df22406b49c8a8b716b10e0594302853d80ca6f38862d05dc1f9";
pub const FLAXRUNNER_SHA256: &str =
    "6edae2ae773b73b907b5a3d4c020052075f7b32f9c2872ca78246eead9dfff34";
pub const DUEL_ARENA_SHA256: &str =
    "5656dabb30a47aac590fa1afadba19e689dd792d70da8dc4851e18d62e52d090";
pub const NATURE_RUNNER_LOGIC_SHA256: &str =
    "7a81b75a4cc4fde41d12565f5fe999de7931d88f2529da6d2d5e6a31f82d82f0";
pub const MULECRAFTER_LOGIC_SHA256: &str =
    "d9cc408c1857a02332e3338e1ae1e956dea51c7c191d4a66af81be6e5108251a";
pub const FLAXRUNNER_LOGIC_SHA256: &str =
    "05a07311383d8ab8121b45e0dcf56a3eefef27d94d8c23565d80fec2462cc32e";
pub const DUEL_ARENA_LOGIC_SHA256: &str =
    "325ce631a7a3f246ab0bc51e9b09945aaa018d7c8971b334994384f62cd1d8f2";
pub const DUEL_INTERFACE_SHA256: &str =
    "658e20f50117f0d23b8524e0ca389399d6fd52de2de1585313081324434634f9";

pub const ESSENCE_UNNOTED_ID: i32 = 1436;
pub const ESSENCE_NOTED_ID: i32 = 1437;
pub const AIR_RUNE_ID: i32 = 556;
pub const AIR_TALISMAN_ID: i32 = 1438;
pub const TRADE_CAP: i32 = 25;
pub const MULE_TRADE_CAP: i32 = 27;
pub const BANK_SEED_ESSENCE: i32 = 200;
pub const TEMPLE_Z: i32 = 4000;
pub const AIR_RUINS: (i32, i32, i32) = (2983, 3288, 0);
pub const FALADOR_EAST: (i32, i32, i32) = (3013, 3355, 0);
pub const FLAX_ID: i32 = 1779;
pub const BOW_STRING_ID: i32 = 1777;
pub const FLAX_MIN_CAPACITY: i32 = 24;
pub const FLAX_FIELD: (i32, i32, i32) = (2741, 3444, 0);
pub const FLAX_MEET: (i32, i32, i32) = (2719, 3471, 0);
pub const FLAX_BANK: (i32, i32, i32) = (2725, 3493, 0);
pub const FLAX_WHEEL: (i32, i32, i32) = (2711, 3471, 1);
pub const DUEL_CHALLENGE_ANCHOR: (i32, i32, i32) = (3368, 3274, 0);
pub const DUEL_WEAPON_ALIAS: &str = "bronze_scimitar";
pub const SCRIPT_GOLD_DEADLINE_SECS: u64 = 180;
pub const SCRIPT_GOLD_WATCH_TICKS: u32 = 150;
pub const PREP_DEADLINE_SECS: u64 = 180;

pub const DUEL_SELECT_MODAL: i32 = 6575;
pub const DUEL_CONFIRM_MODAL: i32 = 6412;
pub const DUEL_WIN_MODAL: i32 = 6733;
pub const DUEL_SELECT_ACCEPT: i32 = 6674;
pub const DUEL_CONFIRM_ACCEPT: i32 = 6520;
pub const DUEL_SELECT_PARTNER: i32 = 6671;
pub const DUEL_SELECT_STATUS: i32 = 6684;
pub const DUEL_CONFIRM_STATUS: i32 = 6571;

pub const EXPECTED_DUEL_CONTROLS: DuelControls = DuelControls {
    select_modal: DUEL_SELECT_MODAL,
    confirm_modal: DUEL_CONFIRM_MODAL,
    win_modal: DUEL_WIN_MODAL,
    select_accept: DUEL_SELECT_ACCEPT,
    confirm_accept: DUEL_CONFIRM_ACCEPT,
    select_partner: DUEL_SELECT_PARTNER,
    select_status: DUEL_SELECT_STATUS,
    confirm_status: DUEL_CONFIRM_STATUS,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PairCase {
    Air,
    Mule,
    Flax,
    Duel,
}

impl PairCase {
    pub fn headed_cells() -> [Self; 4] {
        [Self::Air, Self::Mule, Self::Flax, Self::Duel]
    }

    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "nature_crafter_air" | "air_pair" => Ok(Self::Air),
            "mule_crafter_air" | "mule_pair" => Ok(Self::Mule),
            "flax_runner" | "flax_pair" => Ok(Self::Flax),
            "duel_arena" | "duel_pair" => Ok(Self::Duel),
            other => Err(format!(
                "pair core watch does not accept {other:?}; headed cells are nature_crafter_air, mule_crafter_air, flax_runner, duel_arena"
            )),
        }
    }

    pub fn scenario_name(self) -> &'static str {
        match self {
            Self::Air => "nature_crafter_air",
            Self::Mule => "mule_crafter_air",
            Self::Flax => "flax_runner",
            Self::Duel => "duel_arena",
        }
    }

    pub fn card_name(self) -> &'static str {
        match self {
            Self::Air => NATURECRAFTER,
            Self::Mule => MULECRAFTER,
            Self::Flax => FLAXRUNNER,
            Self::Duel => DUEL_ARENA,
        }
    }

    pub fn source_sha256(self) -> &'static str {
        match self {
            Self::Air => NATURECRAFTER_SHA256,
            Self::Mule => MULECRAFTER_SHA256,
            Self::Flax => FLAXRUNNER_SHA256,
            Self::Duel => DUEL_ARENA_SHA256,
        }
    }

    pub fn is_headed_cell(self) -> bool {
        matches!(self, Self::Air | Self::Mule | Self::Flax | Self::Duel)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    Mapped,
    ArityLimited,
    UnusedByCase,
    CatalogLiteralMatchesGenerated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OperationGate {
    pub source: &'static str,
    pub call: &'static str,
    pub host_shape: &'static str,
    pub kind: GateKind,
    pub owner: &'static str,
}
pub fn count_id(items: &[ItemView], id: i32) -> i32 {
    items
        .iter()
        .filter(|item| item.def.id == id && item.count > 0)
        .map(|item| item.count)
        .sum()
}

pub fn near(tile: Option<(i32, i32, i32)>, target: (i32, i32, i32), radius: i32) -> bool {
    tile.is_some_and(|tile| {
        tile.2 == target.2 && (tile.0 - target.0).abs().max((tile.1 - target.1).abs()) <= radius
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelogAdmission {
    WaitLogout,
    LoggedOut,
    WaitLogin,
    Ready,
}

pub fn relog_admission(
    saw_logout: bool,
    ingame: bool,
    scene_state: i32,
    inventory_tab_available: bool,
) -> RelogAdmission {
    if !saw_logout {
        if !ingame || scene_state != 2 {
            RelogAdmission::LoggedOut
        } else {
            RelogAdmission::WaitLogout
        }
    } else if ingame && scene_state == 2 && inventory_tab_available {
        RelogAdmission::Ready
    } else {
        RelogAdmission::WaitLogin
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartBarrier {
    Wait,
    StartBoth,
    RejectStartedWhileUnready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartBarrierInput {
    pub a_prepared: bool,
    pub b_prepared: bool,
    pub a_started: bool,
    pub b_started: bool,
    pub a_wait_ack: bool,
    pub b_wait_ack: bool,
    pub a_current_ok: bool,
    pub b_current_ok: bool,
}

pub fn shared_start_barrier(input: StartBarrierInput) -> StartBarrier {
    if input.a_started || input.b_started {
        if input.a_started && input.b_started {
            return StartBarrier::Wait;
        }
        return StartBarrier::RejectStartedWhileUnready;
    }
    if input.a_wait_ack || input.b_wait_ack {
        return StartBarrier::Wait;
    }
    if input.a_prepared && input.b_prepared && input.a_current_ok && input.b_current_ok {
        StartBarrier::StartBoth
    } else {
        StartBarrier::Wait
    }
}
pub(super) fn account_identity_eq(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    if left.is_empty() || right.is_empty() {
        return false;
    }
    JString::to_userhash(left) == JString::to_userhash(right)
}
pub fn in_temple(tile: Option<(i32, i32, i32)>) -> bool {
    tile.is_some_and(|tile| tile.1 > TEMPLE_Z)
}
pub(super) fn stat<'a>(snapshot: &'a GameSnapshot, name: &str) -> Option<&'a api::snapshot::StatView> {
    snapshot
        .stats()
        .iter()
        .find(|row| row.name.eq_ignore_ascii_case(name))
}