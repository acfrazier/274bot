//! JavaScript script surface: source loading, the picker library and V8 isolates.
//!
//! Loading and validation use the process ValidationSeat. Gameplay gets a
//! separate isolate only on Start, through the native slot's surface factory.
//! Nothing here embeds a catalog script tree; catalog sources come from the
//! operator's rs2b0t checkout.

#[path = "compat/anchor_return.rs"]
mod anchor_return;
#[path = "api_v2/api_session.rs"]
mod api_session;
#[path = "materialize/area.rs"]
mod area;
#[path = "scene/attack_clock.rs"]
mod attack_clock;
#[path = "compat/autocast.rs"]
mod autocast;
#[path = "materialize/bank_locations_v8.rs"]
mod bank_locations_v8;
#[path = "compat/bank_rows.rs"]
mod bank_rows;
#[path = "scene/bank_select.rs"]
mod bank_select;
#[path = "materialize/bank_tasks_v8.rs"]
mod bank_tasks_v8;
#[path = "compat/bank_withdraw.rs"]
mod bank_withdraw;
#[path = "runtime/bindings.rs"]
mod bindings;
#[path = "compat/boat_fare.rs"]
mod boat_fare;
#[path = "materialize/boost_potions_v8.rs"]
mod boost_potions_v8;
#[path = "materialize/buyout_plan.rs"]
mod buyout_plan;
#[path = "compat/cake_stall.rs"]
pub mod cake_stall;
#[path = "materialize/cake_stall_v8.rs"]
mod cake_stall_v8;
#[path = "materialize/callback_v8.rs"]
pub(crate) mod callback_v8;
#[path = "materialize/canvas_tape.rs"]
mod canvas_tape;
#[path = "materialize/chatbox_v8.rs"]
mod chatbox_v8;
#[path = "compat/clue.rs"]
pub mod clue;
#[path = "materialize/clue_facts_v8.rs"]
mod clue_facts_v8;
#[path = "materialize/clue_logic_v8.rs"]
mod clue_logic_v8;
#[path = "materialize/clue_pack_v8.rs"]
mod clue_pack_v8;
#[path = "materialize/combat_style_v8.rs"]
mod combat_style_v8;
#[path = "tooling/compat_dts.rs"]
pub mod compat_dts;
#[path = "compat/cook_locations.rs"]
mod cook_locations;
#[path = "runtime/crypto_stub.rs"]
mod crypto_stub;
#[path = "compat/death_recovery.rs"]
mod death_recovery;
#[path = "tooling/declared_abi.rs"]
pub mod declared_abi;
#[path = "compat/dialog.rs"]
mod dialog;
#[path = "materialize/dialog_v8.rs"]
mod dialog_v8;
#[path = "materialize/distance.rs"]
mod distance;
#[path = "compat/drive_partner_trade.rs"]
mod drive_partner_trade;
#[path = "compat/duel.rs"]
mod duel;
#[path = "materialize/entities_v8.rs"]
mod entities_v8;
#[path = "materialize/entity_pages.rs"]
mod entity_pages;
#[path = "scene/event_signal.rs"]
mod event_signal;
#[path = "scene/events.rs"]
mod events;
#[path = "compat/fight_upkeep.rs"]
mod fight_upkeep;
#[path = "compat/fire.rs"]
mod fire;
#[path = "materialize/fire_v8.rs"]
mod fire_v8;
#[path = "materialize/gather_methods_v8.rs"]
mod gather_methods_v8;
#[path = "materialize/gathering_v8.rs"]
mod gathering_v8;
#[path = "tooling/host_js.rs"]
pub mod host_js;
/// The hunt step machines on the machine host (fence F09).
#[path = "compat/hunt.rs"]
mod hunt;
#[path = "compat/hunt_bank.rs"]
pub mod hunt_bank;
#[path = "compat/hunt_catalog.rs"]
mod hunt_catalog;
#[path = "compat/hunt_cell.rs"]
pub mod hunt_cell;
#[path = "compat/hunt_fight.rs"]
pub mod hunt_fight;
#[path = "compat/hunt_key.rs"]
pub mod hunt_key;
#[path = "compat/hunt_lair.rs"]
pub mod hunt_lair;
#[path = "compat/hunt_leave.rs"]
pub mod hunt_leave;
#[path = "materialize/hunt_v8.rs"]
mod hunt_v8;
#[path = "scene/inspect_wait.rs"]
mod inspect_wait;
#[path = "materialize/inventory_v8.rs"]
mod inventory_v8;
#[path = "runtime/isolate.rs"]
mod isolate;
#[path = "library/js_cache.rs"]
pub mod js_cache;
#[path = "compat/keep_list.rs"]
pub mod keep_list;
#[path = "library/library.rs"]
mod library;
#[path = "scene/line_of_sight.rs"]
pub mod line_of_sight;
#[path = "materialize/line_of_sight.rs"]
mod line_of_sight_v8;
#[path = "materialize/lineage_v8.rs"]
mod lineage_v8;
#[path = "materialize/loadout_v8.rs"]
mod loadout_v8;
/// Per-isolate step-machine host: families Rust begins, steps and aborts.
mod machine;
#[path = "materialize/machine_v8.rs"]
mod machine_v8;
/// Frozen market catalog facts over the selected revision's items.
#[path = "compat/market_catalog.rs"]
mod market_catalog;
#[path = "materialize/melee_weapons_v8.rs"]
mod melee_weapons_v8;
#[cfg(feature = "memory-profile")]
#[path = "runtime/memory_profile.rs"]
pub mod memory_profile;
#[path = "compat/modals.rs"]
mod modals;
#[path = "library/module_imports.rs"]
pub mod module_imports;
/// One decoded snapshot scene per isolate thread, read by the step machines.
#[path = "scene/observed.rs"]
pub mod observed;
#[path = "materialize/paint_chrome.rs"]
mod paint_chrome;
#[path = "materialize/paint_jive.rs"]
mod paint_jive;
#[path = "compat/partner_trade.rs"]
mod partner_trade;
#[path = "materialize/partner_trade_v8.rs"]
mod partner_trade_v8;
#[path = "compat/periodic_bank.rs"]
mod periodic_bank;
#[path = "runtime/platform.rs"]
mod platform;
#[path = "compat/prayer.rs"]
mod prayer;
#[path = "compat/production.rs"]
mod production;
#[path = "materialize/progress_methods_v8.rs"]
mod progress_methods_v8;
#[path = "materialize/quest_facts_v8.rs"]
mod quest_facts_v8;
#[path = "compat/quest_journal/isolate.rs"]
pub mod quest_journal;
#[path = "compat/reach.rs"]
mod reach;
#[path = "compat/reach_entity.rs"]
mod reach_entity;
#[path = "materialize/reach_query.rs"]
pub(crate) mod reach_query;
#[path = "runtime/recovery_hints_v8.rs"]
mod recovery_hints_v8;
#[path = "library/rs2b0t_registry.rs"]
pub mod rs2b0t_registry;
#[path = "materialize/run_policy_v8.rs"]
mod run_policy_v8;
#[path = "materialize/rune_routes_v8.rs"]
mod rune_routes_v8;
#[path = "runtime/runtime_bridge.rs"]
mod runtime_bridge;
#[path = "scene/scene_query.rs"]
mod scene_query;
#[path = "materialize/scene_v8.rs"]
mod scene_v8;
#[path = "materialize/selected_facts_v8.rs"]
mod selected_facts_v8;
#[path = "library/shape.rs"]
mod shape;
#[path = "compat/sherlock.rs"]
pub mod sherlock;
pub mod shim;
#[path = "compat/shop.rs"]
mod shop;
#[path = "materialize/shop_ops_v8.rs"]
mod shop_ops_v8;
#[path = "materialize/skills_v8.rs"]
mod skills_v8;
#[path = "runtime/snapshot.rs"]
mod snapshot;
#[path = "compat/special.rs"]
mod special;
#[path = "scene/supply_v2.rs"]
mod supply_v2;
#[path = "materialize/supply_v8.rs"]
mod supply_v8;
pub mod surface;
#[path = "materialize/targets_v8.rs"]
mod targets_v8;
#[path = "scene/task_clock.rs"]
mod task_clock;
#[path = "compat/teleport.rs"]
mod teleport;
#[path = "materialize/tools_v8.rs"]
mod tools_v8;
#[path = "compat/trade.rs"]
mod trade;
#[path = "runtime/wait_clock.rs"]
mod wait_clock;
#[path = "compat/walk.rs"]
mod walk;
#[path = "materialize/walk_opening_v8.rs"]
mod walk_opening_v8;
#[path = "scene/walk_wait.rs"]
mod walk_wait;
#[path = "wire/isolate_fb.rs"]
pub mod wire;

pub use shape::{
    collect_raw_sibling_hashes, detect_shape, first_unloadable_for_card,
    first_unloadable_specifier, is_catalog_dim, is_reserved, live_example_path,
    live_file_fixture_path, live_file_fixture_stem, parse_declared_api_version,
    raw_content_fingerprint, resolve_api_family, resolve_sibling_path, scan_import_specifiers,
    scan_same_folder_js_imports, scan_scripts_sibling_js_imports, sibling_module_url, ApiFamily,
    LoadShape, ScriptSel, VersionDiag, CATALOG_DIM,
};

pub use shape::{resolve_sibling_modules, transpile_ts};

pub use library::JsLibrary;
pub use library::{
    default_js_store, format_load_failures, CatalogApplyReport, CatalogDiff, JsCard, LoadFailure,
    LoadStage, PreparedCard,
};

pub use library::{
    CardPreparationOutcome, CardPreparationSnapshot, CatalogPreparationResult,
    CatalogPreparationSnapshot, CatalogSnapshotError, CatalogWorkerError,
};

pub use isolate::{
    isolate_capacity_status, HeldWaitFacts, IsolateCapacityStatus, LoadIsolate, LoadIsolateError,
    QueuedInteract, QueuedSeatBatch, Ready, ScriptStopReceipt, TeardownProof, ValidationBatch,
    ValidationError,
};

pub use platform::bootstrap_v8_platform;

pub(crate) use recovery_hints_v8::{
    post as post_recovery_hints, startup_complete as recovery_startup_complete,
};

pub use recovery_hints_v8::{HintTile, RecoveryHintsCell};

pub use js_cache::{default_js_cache_root, CacheMeta, CachedJs, JsCache};
pub use rs2b0t_registry::{
    clear_rs2b0t_import_at, default_rs2b0t_import_file, default_rs2b0t_path_file, parse_registry,
    parse_registry_with_sources, persist_rs2b0t_root, persist_rs2b0t_root_at, registry_index_path,
    rs2b0t_import_deferred, rs2b0t_import_deferred_at, rs2b0t_root, rs2b0t_root_at,
    rs2b0t_root_checked, rs2b0t_root_checked_at, script_file_path, set_rs2b0t_import_deferred_at,
    settings_schema_from_source, try_settings_schema_from_source,
};

/// Canonical JS-backed cards installed by [`install_cards`].
pub use sherlock::CARDS;

/// Install the canonical JS-backed cards into the native registry.
pub fn install_cards() -> Result<(), script::registry::CardInstallError> {
    script::registry::install_cards(&CARDS)
}
