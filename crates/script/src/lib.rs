//! Compiled script runner: the `Script` trait, per-tick context, and the
//! per-uid `SlotScript` state machine. Load (out-of-tree JS) lives behind
//! the `load` feature: a picker library of JS cards plus a rustyscript/V8
//! isolate spawned only on Start.

/// Native boost-potion descriptors, planning and sip selection.
pub mod boost_potions;
#[cfg(feature = "load")]
pub mod canvas;
/// Curated script site configuration and the hostile-attacker predicate.
pub mod content;
pub mod ctx;
pub mod declared_abi;
/// Script-side Ent tile lookup over caller-supplied rows.
pub mod ent;
pub mod escape_runes;
#[cfg(feature = "load")]
mod events;
pub mod food_policy;
/// Native gather-tool selection over the posted `api::gather_tools` rows.
pub mod gather_tools;
pub mod host_js;
pub mod identity;
#[cfg(feature = "load")]
pub mod isolate_fb;
pub mod isolated_env;
#[cfg(feature = "load")]
pub mod js_cache;
mod keep_list;
pub mod load;
pub mod loadout_plan;
pub mod loadouts_store;
mod module_imports;
pub mod params;
/// Pure ranged supply predicate (`rangeSupplyEmpty`).
pub mod ranged;
pub mod registry;
pub mod rs2b0t_registry;
pub mod settings_store;
#[cfg(feature = "load")]
pub mod shim;
pub mod slot;
#[cfg(feature = "load")]
pub mod watchdog;

pub use ctx::{CompiledTick, DetectedRandom, FindOptions, RandomClaim, Script, ScriptCtx};
pub use identity::{
    card_assignment, card_identity_id, card_identity_key, claim_legacy_overrides,
    combine_fingerprints, compiled_assignment, compiled_identity_key, file_identity,
    migrate_legacy_setting_value, migrate_overrides, missing_file_assignment, parse_source_kind,
    paths_match, raw_sha, source_kind, NOTHING_CHANGED_CATALOG, NOTHING_CHANGED_RELOAD,
};
pub use isolated_env::{bot_file, bot_home, rs2b0t_env, IsolatedEnv};
#[cfg(feature = "load")]
pub use js_cache::{default_js_cache_root, CacheMeta, CachedJs, JsCache};
pub use load::{
    default_js_store, detect_shape, first_unloadable_specifier, is_catalog_dim, is_reserved,
    live_example_path, live_file_fixture_path, live_file_fixture_stem, parse_declared_api_version,
    raw_content_fingerprint, resolve_api_family, scan_import_specifiers,
    scan_same_folder_js_imports, sibling_module_url, ApiFamily, CatalogApplyReport, CatalogDiff,
    JsCard, LoadFailure, LoadShape, LoadStage, PreparedCard, ScriptSel, VersionDiag,
};
/// Cache/transpile-backed library and sibling resolution (isolate feature).
#[cfg(feature = "load")]
pub use load::{resolve_sibling_modules, JsLibrary};
pub use loadouts_store::{
    copy_equipment_preserving_supplies, default_loadouts_path, resolve_setting_options,
    resolve_setting_options_with_labels, unique_loadout_name, worn_slot_label, CarryEntry, Loadout,
    LoadoutsStore, ResolvedSettingOptions, WORN_SLOTS, WORN_SLOT_LAYOUT,
};
pub use params::defaults;
pub use registry::{compiled_id, compiled_ids, factory, is_whale, CompiledId};
pub use rs2b0t_registry::{
    clear_rs2b0t_import_at, default_rs2b0t_import_file, default_rs2b0t_path_file, parse_registry,
    parse_registry_with_sources, persist_rs2b0t_root, persist_rs2b0t_root_at, registry_index_path,
    rs2b0t_import_deferred, rs2b0t_import_deferred_at, rs2b0t_root, rs2b0t_root_at,
    script_file_path, set_rs2b0t_import_deferred_at, settings_schema_from_source,
    ItemOptionCandidate, ItemOptionSpec, RegistryCard, ScriptKind, ScriptSource, SettingDef,
};
pub use settings_store::{
    card_key, coerce_setting_value, default_script_settings_path, format_setting_value, merge_bag,
    parameter_rows, setting_visible, ScriptSettingsStore,
};
pub use slot::{RunState, ScriptLifecycleReceipt, ScriptTerminalState, SlotScript};
#[cfg(feature = "load")]
pub use slot::{StartLoadError, StartOutcome, StartPoll};
#[cfg(feature = "load")]
pub use watchdog::{
    ProgressWatchdog, RestartReason, Tile as WatchdogTile, WatchdogAction, WatchdogState,
};

#[cfg(feature = "load")]
pub use load::{transpile_ts, LoadIsolate, Ready};

#[cfg(feature = "load")]
mod autocast;
#[cfg(feature = "load")]
mod bank_access;
#[cfg(feature = "load")]
mod bank_deposit;
#[cfg(feature = "load")]
mod bank_op;
#[cfg(feature = "load")]
mod bank_open;
#[cfg(feature = "load")]
mod bank_select;
#[cfg(feature = "load")]
mod bank_withdraw;
#[cfg(feature = "load")]
mod banking_open;
pub mod cake_stall;
#[cfg(feature = "load")]
mod clue;
#[cfg(feature = "load")]
mod death_recovery;
#[cfg(feature = "load")]
mod dialog;
#[cfg(feature = "load")]
mod drive_partner_trade;
#[cfg(feature = "load")]
mod fire;
/// The hunt step machines on the machine host (fence F09).
#[cfg(feature = "load")]
mod hunt;
#[cfg(feature = "load")]
pub mod hunt_bank;
#[cfg(feature = "load")]
mod hunt_catalog;
#[cfg(feature = "load")]
pub mod hunt_cell;
#[cfg(feature = "load")]
pub mod hunt_fight;
#[cfg(feature = "load")]
pub mod hunt_key;
#[cfg(feature = "load")]
pub mod hunt_lair;
#[cfg(feature = "load")]
pub mod hunt_leave;
#[cfg(feature = "load")]
mod inspect_wait;
#[cfg(feature = "load")]
pub mod line_of_sight;
/// Per-isolate step-machine host: families Rust begins, steps and aborts.
#[cfg(feature = "load")]
mod machine;
/// Frozen market catalog facts over the selected revision's items.
#[cfg(feature = "load")]
mod market_catalog;
#[cfg(feature = "load")]
mod melee_weapons;
#[cfg(feature = "load")]
mod modals;
/// One decoded snapshot scene per isolate thread, read by the step machines.
#[cfg(feature = "load")]
pub mod observed;
#[cfg(feature = "load")]
mod partner_trade;
#[cfg(feature = "load")]
mod periodic_bank;
#[cfg(feature = "load")]
mod prayer;
#[cfg(feature = "load")]
mod production;
#[cfg(feature = "load")]
mod quest_journal;
#[cfg(feature = "load")]
mod reach;
#[cfg(feature = "load")]
mod reach_entity;
#[cfg(feature = "load")]
mod scene_query;
#[cfg(feature = "load")]
mod sherlock;
#[cfg(feature = "load")]
mod shop;
#[cfg(feature = "load")]
mod special;
#[cfg(feature = "load")]
mod supply_v2;
#[cfg(feature = "load")]
mod task_clock;
#[cfg(feature = "load")]
mod teleport;
#[cfg(feature = "load")]
mod trade;
#[cfg(feature = "load")]
mod walk;
#[cfg(feature = "load")]
mod walk_wait;

#[cfg(feature = "memory-profile")]
pub mod memory_profile;
