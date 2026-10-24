//! Native script runner and shared gameplay state.
//!
//! The slot owns compiled scripts and drives external runtimes through
//! the `surface` contract.

/// Inventory-first acquisition shared by native consumers.
pub mod acquire;
pub mod api_combat;
pub mod api_gather;
pub mod api_progress;

/// Shared native bank selector and transfer machines.
pub mod bank;
/// The persisted per-account bank hint file (design-bank-snapshot §1.4).
pub mod bank_hints;
/// Native boost-potion descriptors, planning and sip selection.
pub mod boost_potions;
pub mod canvas;
pub mod card;
pub mod channel;
pub mod combat;
mod combat_session;
/// Curated script site configuration and the hostile-attacker predicate.
pub mod content;
pub mod ctx;

pub mod duel_arena;
/// Script-side Ent tile lookup over caller-supplied rows.
pub mod ent;
pub mod escape_runes;

pub mod food_policy;
/// Borrowed preset-location helpers for compiled and compatibility callers.
pub mod gather_locations;
/// Native gather-tool selection over the posted `api::gather_tools` rows.
pub mod gather_tools;
/// Native woodcutting and mining gatherer card.
pub mod gatherer;
/// Native anchored holiday-item collector.
pub mod grinch;
pub mod ground_take;

pub mod identity;
pub mod interact;

pub mod isolated_env;

pub mod keep_list;

pub mod loadout_plan;
pub mod loadouts_store;
pub mod local_state;

pub mod native;
pub mod native_bank;
pub mod native_equipment;
pub mod native_production;
pub mod native_shop;
pub mod paint;
pub mod preparation;
pub mod quester;
/// Pure ranged supply predicate (`rangeSupplyEmpty`).
pub mod ranged;
/// Walking-depth helpers over the posted reach view.
pub mod reach_route;
pub mod registry;

pub mod settings_store;
pub mod shim;
pub mod shop_open;
pub mod slot;
/// Sealed external-script authority and shared server-ordered host input.
pub mod surface;
/// Pure condition derivation and survival arithmetic.
pub mod survival;
pub mod trade_screen;
pub mod watchdog;
pub mod work_admission;

pub use card::{
    ItemOptionCandidate, ItemOptionSpec, RegistryCard, ScriptKind, ScriptSel, ScriptSource,
    SettingDef,
};
pub use ctx::{CompiledTick, DetectedRandom, FindOptions, RandomClaim, ScriptCtx};
pub use duel_arena::{duel_name, duel_partner_name};
pub use identity::{
    card_assignment, card_identity_id, card_identity_key, claim_legacy_overrides,
    combine_fingerprints, compiled_assignment, compiled_identity_key, file_identity,
    migrate_legacy_setting_value, migrate_overrides, missing_file_assignment, parse_source_kind,
    paths_match, raw_sha, source_kind, NOTHING_CHANGED_CATALOG, NOTHING_CHANGED_RELOAD,
};
pub use isolated_env::{bot_file, bot_home, rs2b0t_env, IsolatedEnv};

pub use loadouts_store::{
    copy_equipment_preserving_supplies, resolve_setting_options,
    resolve_setting_options_with_labels, unique_loadout_name, worn_slot_label, LoadoutsBacking,
    LoadoutsBaseline, LoadoutsSaveError, LoadoutsStore, ResolvedSettingOptions, WORN_SLOT_LAYOUT,
};
pub use registry::{compiled_card, compiled_cards, compiled_id, is_whale, CompiledId};

pub use settings_store::{
    card_key, coerce_setting_value, default_script_settings_path, format_setting_value, merge_bag,
    parameter_rows, setting_visible, ScriptSettingsStore, CLUE_DUEL_PARTNER,
};
pub use slot::StartLoadError;
pub use slot::{
    CompiledDelivery, RunState, ScriptLifecycleReceipt, ScriptTerminalState, SlotScript,
    StartOutcome, StartPoll,
};
pub use watchdog::{
    ProgressWatchdog, RestartReason, ScriptProgress, Tile as WatchdogTile, WatchdogAction,
    WatchdogState, SKILL_SLOTS,
};

pub mod cake_stall;
pub mod clue;

pub mod dialogue_outcome;

pub mod melee_weapons;

/// Shared Make-X selection and count sequencing for runtime adapters.
pub mod production;
pub mod quest_journal;
